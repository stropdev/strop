//! Git memory surfaces (M3, reworked 0010): commit browser, changed-files
//! dive, diff view, blame card, permalinks. Every surface is a real
//! readonly buffer (0001 §3: motions, /, yank work); jobs post onto the
//! event loop (0001 §5.6: no blocking the input path on shell git).
//! R9/R6: every job owns a ticket; results land through
//! `git_memory::jobs` handlers which validate ownership first.

mod jobs;
mod types;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{CancelReason, FailureKind, Outcome, Ticket};
use strop_git::memory::{self, BlameLine};
use strop_git::{Hunk, LineOrigin};

use super::document::{ReturnPoint, Surface};
use super::{trace, Editor, Key};

pub(crate) use jobs::{git_failure, repo_or_unavailable};
pub use types::{
    BlameKey, CardKey, ContextKey, DiveData, DiveKey, DiveTarget, GitJob, GitMutation, HunkData,
    HunkKey, LogKey, MutationKey, MutationKind, MutationOp,
};

/// The commit a Diff surface's file belongs to, with the commit's full
/// changed-file list — the sidebar's data (typed numstat rows, the same
/// ones the changed-files surface renders from; 0011 §4).
#[derive(Debug, Clone)]
pub struct CommitFiles {
    pub sha: String,
    pub files: Vec<memory::ChangedFile>,
    /// Selected file identity; display labels are not reversible native paths.
    pub current: PathBuf,
}

/// Where a hunk preview came from: the buffer it undoes/stages in, at
/// the revision it was captured. Edits since then invalidate it —
/// applying a stale region would cut the wrong lines.
#[derive(Debug, Clone)]
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
    pub(crate) fn push_surface(&mut self, name: Option<&str>, text: &str, mut surface: Surface) {
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
        let mut buf = strop_core::Buffer::from_text(text);
        buf.name = name.map(|n| n.to_string());
        // surfaces render via delta/plain rules: no tree-sitter;
        // readonly derives from the source (0021 §4)
        let id = self.docs.insert(super::Document::surface(buf, surface));
        self.drop_stale_scratch(id);
        self.push_jump(); // opening a surface is a jumplist entry
        self.generation += 1; // document set changed: old jobs are stale (0011 §2)
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
    }

    /// A diff surface from structured hunks (0010 §2). `label` heads the
    /// stats row; `origin` is set only for working-tree hunk previews.
    pub(crate) fn open_diff_surface(
        &mut self,
        name: &str,
        label: &str,
        hunks: Vec<Hunk>,
        origin: Option<HunkOrigin>,
    ) {
        self.open_delta(name, label, hunks, origin, None);
    }

    /// The diff-surface builder: `commit` rides along when the delta
    /// came from the dive chain (sidebar + `]f`/`[f`, 0011 §4).
    pub(crate) fn open_delta(
        &mut self,
        name: &str,
        label: &str,
        hunks: Vec<Hunk>,
        origin: Option<HunkOrigin>,
        commit: Option<CommitFiles>,
    ) {
        let (added, deleted) = hunk_stats(&hunks);
        let text = diff_surface_text(label, &hunks);
        self.push_surface(
            Some(name),
            &text,
            Surface::Diff {
                label: label.to_string(),
                hunks,
                added,
                deleted,
                origin,
                commit,
                sidebar_focus: false,
                return_to: None,
            },
        );
        // syntax highlighting under the origin tint (delta's look):
        // the label is the file path for commit deltas; "hunk" and
        // friends resolve to None and keep origin colors. The pure
        // detector reads the surface's own rope — no extra build.
        let hl =
            strop_syntax::Highlighter::for_path(std::path::Path::new(label), self.buf().text());
        if hl.is_some() {
            self.cur_mut().highlighter = hl;
        }
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
        let Some(context) = self.git.clone() else {
            self.message = "not a git repo".into();
            return;
        };
        let workdir = context.workdir().to_path_buf();
        let file = if file_scoped {
            self.buf().path.as_deref().and_then(|p| {
                let abs = if Path::new(p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    workdir.join(p)
                };
                abs.strip_prefix(&workdir).ok().map(|r| r.to_path_buf())
            })
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
            "loading log…",
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
        };
        let Some(ticket) = self.git_ticket(key) else {
            return;
        };
        self.log_requests.insert(doc, ticket.clone());
        let revision = self.buf().revision().get();
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"log",
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "revision":revision,
                "path":file.as_ref().map(|p|p.to_string_lossy()),
            })
        });
        let args = (
            ticket.clone(),
            trace::services::NativePath(workdir.clone()),
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
                match memory::log_graph_range(&workdir, 200, file.as_deref(), range) {
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
            Key::Enter => self.dive(),
            _ => self.feed_command(key),
        }
    }

    /// Yank the plan's target ranges (shared with normal mode's
    /// dispatch: one implementation, one behavior).
    pub(crate) fn yank_only(&mut self, command: &strop_grammar::Command) {
        let plan = match strop_grammar::plan(self.buf(), &self.all_cursors(), command) {
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
        let doc = self.current();
        if let Some(pane) = self.panes.get_mut(self.active_pane) {
            pane.doc = doc; // the pane follows the successor
        }
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

#[cfg(test)]
mod tests;
