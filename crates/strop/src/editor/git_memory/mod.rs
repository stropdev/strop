//! Git memory surfaces (M3, reworked 0010): commit browser, changed-files
//! dive, diff view, blame card, permalinks. Every surface is a real
//! readonly buffer (0001 §3: motions, /, yank work); jobs post onto the
//! event loop (0001 §5.6: no blocking the input path on shell git).
//! R9/R6: every job owns a ticket; results land through
//! `git_memory::jobs` handlers which validate ownership first.

mod file_list;
mod hunk_set;
mod jobs;
mod presentation;
pub(crate) use file_list::PreparedFiles;
pub(crate) use hunk_set::HunkSet;
pub(crate) use presentation::PreparedDiff;
mod types;
pub(crate) use jobs::{git_failure, repo_or_unavailable};
pub use types::GitJob;
pub(crate) use types::{
    BlameKey, CardKey, ContextKey, DiveData, DiveKey, DiveTarget, GitMutation, HunkData, HunkKey,
    LogKey, MutationKey, MutationKind, MutationOp,
};

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{CancelReason, FailureKind, Outcome, Ticket};
use strop_git::exec::GitExec;
use strop_git::memory::{self, BlameLine};
use strop_git::{Hunk, LineOrigin, RepoTarget};

use super::document::{ReturnPoint, Surface};
use super::{trace, Editor, Key};
/// The commit a Diff surface's file belongs to, with the commit's full
/// changed-file list — the sidebar's data (typed numstat rows, the same
/// ones the changed-files surface renders from; 0011 §4). `repo` is
/// the provenance the whole delta chain replays: `]f` steps and dives
/// from this surface run against that repository — remote surfaces
/// never answer from the local cwd (0036 RW8).
#[derive(Debug, Clone)]
pub struct CommitFiles {
    pub repo: RepoTarget,
    pub sha: String,
    pub files: PreparedFiles,
    /// Selected file identity; display labels are not reversible native paths.
    pub current: PathBuf,
}

/// Where a hunk preview came from: the buffer it undoes/stages in, at
/// the revision it was captured. Edits since then invalidate it —
/// applying a stale region would cut the wrong lines.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HunkOrigin {
    pub buffer: DocumentId,
    pub revision: BufferRevision,
    /// The origin was untracked when captured: undo refuses (there is
    /// no HEAD content to restore from).
    pub untracked: bool,
}

/// Per-buffer blame gutter state (0011 §3), keyed by canonical path —
/// no parallel vector to keep aligned, and index churn can never pair
/// one buffer with another's blame. Valid only while the buffer's
/// revision and line count still match the capture.
#[derive(Debug, Clone)]
pub struct BlameGutter {
    pub lines: Vec<BlameLine>,
    /// Buffer revision when the blame was captured; any edit since
    /// invalidates the line↔buffer-line pairing.
    pub revision: BufferRevision,
    /// The pending request while the gutter loads; `None` once loaded.
    /// A late result for a removed request is rejected by ticket, so
    /// toggle-off/on at the same path can never cross-pollinate.
    pub request: Option<Ticket<BlameKey>>,
}

impl Editor {
    pub fn surface(&self) -> Option<&Surface> {
        self.cur().surface_payload()
    }
    // ---- surface lifecycle --------------------------------------------
    pub(crate) fn push_surface(
        &mut self,
        name: Option<&str>,
        text: ropey::Rope,
        mut surface: Surface,
    ) {
        let Some(context) = self.git_context().cloned() else {
            self.message = "not a git repository".into();
            return;
        };
        // rebind-after-insert happens in Document::surface insertion
        // below — dropping before the new id exists strands panes

        // surfaces stack: only the first one opened from a plain buffer
        // carries a return point (closing the deepest unwinds the chain)
        if self.surface().is_none() {
            surface.set_return_point(ReturnPoint {
                buffer: self.current(),
                cursor: self.head(),
                view_top: self.view_top(),
                hscroll: self.view().hscroll,
            });
        }
        let mut buf = strop_core::Buffer::from_snapshot(text);
        buf.name = name.map(|n| n.to_string());
        // surfaces render via delta/plain rules: no tree-sitter;
        // readonly derives from the source (0021 §4)
        let id = self
            .docs
            .insert(super::Document::surface(buf, surface, context));
        self.drop_stale_scratch(id);
        self.push_jump(); // opening a surface is a jumplist entry
        self.generation += 1; // document set changed: old jobs are stale (0011 §2)
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
    }

    /// A diff surface from structured hunks (0010 §2). `label` heads the
    /// stats row; `origin` is set only for working-tree hunk previews.
    pub(crate) fn open_delta(
        &mut self,
        name: &str,
        hunks: PreparedDiff,
        origin: Option<HunkOrigin>,
        commit: Option<CommitFiles>,
    ) {
        let text = hunks.text();
        self.push_surface(
            Some(name),
            text,
            Surface::Diff {
                hunks,
                origin,
                commit,
                sidebar_focus: false,
                return_to: None,
            },
        );
    }

    /// `Space g l`: commit browser. `Space g h`: log scoped to the file.
    pub(crate) fn open_log(&mut self, file_scoped: bool) {
        self.open_log_inner(file_scoped, None, None);
    }

    /// Open the commit browser *at* a commit — the blame dive lands on
    /// the row it was asked about (0011 §3), not the newest entry.
    pub(crate) fn open_log_at(&mut self, sha: &str) {
        self.open_log_inner(false, Some(sha.to_string()), None);
    }

    /// `Space g h` in visual mode: the history of the selected lines
    /// (git log -L) — selection archaeology (0014 wave 4).
    pub(crate) fn open_line_history(&mut self, start: usize, end: usize) {
        self.open_log_inner(true, None, Some((start, end)));
    }

    fn open_log_inner(
        &mut self,
        file_scoped: bool,
        focus: Option<String>,
        range: Option<(usize, usize)>,
    ) {
        let Some(context) = self.git_context().cloned() else {
            self.message = "not a git repo".into();
            return;
        };
        // `git log -L` speaks file line numbers: a partial remote
        // window's lines are window-relative, and pretending they are
        // file coordinates would show the history of the WRONG lines
        // (0036: partial windows never masquerade as full-file Git
        // inputs).
        if range.is_some() && self.remote_file().is_some() && !self.remote_window_complete() {
            self.message = "partial remote snapshot — line history needs a full window".into();
            return;
        }
        let repo = context.repo.clone();
        let file = if file_scoped {
            self.current_buffer_rel(&repo)
        } else {
            None
        };
        self.push_surface(
            Some(if range.is_some() {
                "git log ·lines"
            } else if file_scoped {
                "git log ·file"
            } else {
                "git log"
            }),
            ropey::Rope::from_str("loading log…"),
            Surface::CommitLog {
                rows: vec![],
                focus,
                return_to: None,
            },
        );
        // the new surface document owns its request; registration
        // happens before launch (replay contract)
        let doc = self.current();
        let key = LogKey {
            document: doc,
            revision: self.buf().revision(),
            repo: repo.clone(),
        };
        let Some(ticket) = self.git_ticket(key) else {
            return;
        };
        self.log_requests.insert(doc, ticket.clone());
        let revision = self.buf().revision().get();
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"log",
                "target":if repo.is_remote() { "remote" } else { "local" },
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "revision":revision,
                "path":file.as_ref().map(|p|p.to_string_lossy()),
            })
        });
        let args = (
            ticket.clone(),
            file.clone().map(trace::services::NativePath),
            range,
        );
        self.launch_git_job(
            "git-log",
            "git.log",
            ticket,
            &args,
            GitJob::Log,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let exec = GitExec::for_target(&repo);
                match memory::log_graph_range(&exec, &cancel, 200, file.as_deref(), range) {
                    Ok(rows) => Outcome::Success(rows),
                    Err(message) => Outcome::failed(FailureKind::Exit, message),
                }
            },
        );
    }

    // ---- surface interaction -------------------------------------------

    /// Keys for readonly surface buffers (0001 §3): q closes, Enter
    /// dives, and everything else flows through the shared Walker
    /// command path — motions and yank resolve, mutations refuse
    /// (0010 §6). The Walker owns the pending state, so `: / ?` and
    /// multi-key sequences behave exactly as in normal mode.
    pub(crate) fn feed_readonly(&mut self, key: Key) {
        if key == Key::Esc {
            self.cancel_remote_write(self.current());
            self.stop_remote_follow(self.current());
            self.cancel_remote_filter(self.current());
            self.walker.clear();
            return;
        }
        if self.walker.at_prefix(&["ctrl-w"]) && key == Key::Char('q') {
            self.walker.clear();
            self.close_surface();
            return;
        }
        if key == Key::Char('f') && (self.walker.at_prefix(&["]"]) || self.walker.at_prefix(&["["]))
        {
            let forward = self.walker.at_prefix(&["]"]);
            let n = self.walker.state.count().unwrap_or(1);
            self.walker.clear();
            for _ in 0..n {
                self.commit_file_step(forward);
            }
            return;
        }
        if !self.walker.is_ground() {
            return self.feed_command(key);
        }
        if self.remote_directory_key(key) {
            return;
        }
        match key {
            Key::Char('q') => self.close_surface(),
            Key::CtrlL => self.needs_repaint = true,
            Key::CtrlO => self.jump_back(),
            Key::Tab | Key::Backtab => {
                if matches!(
                    self.surface(),
                    Some(Surface::Diff {
                        commit: Some(_),
                        ..
                    })
                ) {
                    self.toggle_sidebar_focus();
                } else {
                    self.jump_forward();
                }
            }
            Key::Char('j') | Key::Down if self.sidebar_focused() => self.commit_file_step(true),
            Key::Char('k') | Key::Up if self.sidebar_focused() => self.commit_file_step(false),
            Key::Enter if self.sidebar_focused() => self.toggle_sidebar_focus(),
            Key::Enter if self.surface().is_some() => self.dive(),
            _ => self.feed_command(key),
        }
    }

    /// Yank the plan's target ranges (shared with normal mode's
    /// dispatch: one implementation, one behavior).
    pub(crate) fn yank_only(&mut self, command: &strop_grammar::Command) {
        if self.defer_resolution(
            command,
            self.all_cursors(),
            super::resolution::ResolutionPurpose::Execute,
        ) {
            return;
        }
        let plan = match self.resolved_plan(command, &self.all_cursors()) {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                self.message = "no target".into();
                return;
            }
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        let Some(first) = plan.targets.first() else {
            return;
        };
        let range = first.range;
        let text = plan
            .targets
            .iter()
            .map(|target| self.buf().slice_string(target.range))
            .collect::<Vec<_>>()
            .join("\n");
        self.set_register(
            command.register,
            if range.is_linewise() {
                super::Register::linewise(text)
            } else {
                super::Register::characterwise(text)
            },
        );
        self.note_search(command);
        self.flash(range);
    }

    /// `q`: pop one surface (0011 §1). In a split the *pane* closes —
    /// the buffer stays, vim `:q` semantics — and only the last pane's
    /// close closes the buffer, running the guaranteed return-point
    /// restore.
    fn close_surface(&mut self) {
        self.close_pane_or_buffer(true);
    }
}

/// Added/deleted counts across hunks.
pub(crate) fn hunk_stats(hunks: &[Hunk]) -> (usize, usize) {
    hunks.iter().fold((0, 0), |(a, d), h| {
        let adds = h
            .lines
            .iter()
            .filter(|l| l.origin == LineOrigin::Addition)
            .count();
        let dels = h
            .lines
            .iter()
            .filter(|l| l.origin == LineOrigin::Deletion)
            .count();
        (a + adds, d + dels)
    })
}

/// The buffer text a diff surface shows: stats row, then per hunk a
/// header row and unprefixed content rows — exactly the rendered
/// layout (0010 §2).
pub(crate) fn diff_surface_text(label: &str, hunks: &[Hunk]) -> String {
    let (added, deleted) = hunk_stats(hunks);
    let mut text = format!("{label} +{added} -{deleted}\n");
    for hunk in hunks {
        text.push_str(&hunk.header());
        text.push('\n');
        for line in &hunk.lines {
            text.push_str(&line.text_str());
            text.push('\n');
        }
    }
    text
}

/// The git job channel ends (created once in `Editor::new`).
pub fn git_channel() -> (Sender<GitJob>, Receiver<GitJob>) {
    channel()
}

impl Editor {
    /// Table shim (0008 stage 2).
    pub(crate) fn open_log_pub(&mut self, file_scoped: bool) {
        self.open_log(file_scoped);
    }
}

impl Editor {
    /// The current buffer's repo-relative path for `repo` — a local
    /// buffer's path or a remote buffer's remote-file path, stripped
    /// by the repository that owns it. A buffer never borrows another
    /// machine's spelling: a remote file only resolves under its own
    /// endpoint's repository, a local file only under a local one.
    pub(crate) fn current_buffer_rel(&self, repo: &RepoTarget) -> Option<PathBuf> {
        match &self.cur().source {
            super::document::DocumentSource::Remote(file) => match repo {
                RepoTarget::Remote { endpoint, .. } if endpoint == file.file.endpoint() => {
                    repo.rel_of(file.file.path())
                }
                _ => None,
            },
            super::document::DocumentSource::File => {
                let path = self.cur().buf.path.as_deref()?;
                let abs = if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    repo.workdir().join(path)
                };
                repo.rel_of(&abs)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
