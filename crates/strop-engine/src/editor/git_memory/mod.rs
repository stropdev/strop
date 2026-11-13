//! Git memory surfaces (M3, reworked 0010): commit browser, changed-files
//! dive, diff view, blame card, permalinks. Every surface is a real
//! readonly buffer (0001 §3: motions, /, yank work); jobs post onto the
//! event loop (0001 §5.6: no blocking the input path on shell git).
//! R9/R6: every job owns a ticket; results land through
//! `git_memory::jobs` handlers which validate ownership first.

mod file_list;
pub(crate) mod graph;
mod history;
pub mod hunk_set;
mod jobs;
pub mod log_text;
mod presentation;
mod sidebar;
pub use file_list::{file_row_text, FilesRow, PreparedFiles};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use history::publish_rows as publish_log_rows;
pub(crate) use hunk_set::HunkSet;
pub(crate) use jobs::{git_failure, repo_host, repo_or_unavailable};
pub(crate) use presentation::line_emphasis;
pub use presentation::PreparedDiff;
pub use sidebar::{Sidebar, SidebarRow};
mod types;
pub use types::GitJob;
#[cfg(test)]
pub(crate) use types::MutationKind;
pub(crate) use types::{
    BlameKey, BranchKey, CardKey, CommitDraft, ContextKey, DiveData, DiveKey, DiveTarget, DraftKey,
    GitMutation, HunkData, HunkKey, LogRequest, MutationKey, MutationOp, Porcelain, PorcelainDone,
    PorcelainKey, StatusCounts, StatusData, StatusKey,
};

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::Ticket;
use strop_git::memory::BlameLine;
use strop_git::{Hunk, LineOrigin, RepoTarget};

use super::document::Surface;
use super::Editor;
use strop_core::frontend_input::Key;
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
            surface.set_return_point(self.jump_record());
        }
        let mut buf = strop_core::Buffer::from_snapshot(text);
        buf.name = name.map(|n| n.to_string());
        // surfaces render via delta/plain rules: no tree-sitter;
        // readonly derives from the source (0021 §4)
        let Ok(id) = self
            .docs
            .try_insert(super::Document::surface(buf, surface, context))
        else {
            self.message = "document identity space exhausted".into();
            return;
        };
        self.drop_stale_scratch(id);
        self.push_jump(); // opening a surface is a jumplist entry
        self.generation += 1; // document set changed: old jobs are stale (0011 §2)
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
    }

    /// A diff surface from structured hunks (0010 §2). `label` heads the
    /// heading row; `origin` is set only for working-tree hunk previews.
    pub fn open_delta(
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

    /// The current log surface is still reading: more pages may come.
    pub fn log_loading(&self) -> bool {
        self.log_requests.contains_key(&self.current())
    }

    /// `Space g l`: commit browser. `Space g h`: log scoped to the file.
    pub fn open_log(&mut self, file_scoped: bool) {
        history::open(self, file_scoped, None, None);
    }

    /// Open the commit browser *at* a commit — the blame dive lands on
    /// the row it was asked about (0011 §3), not the newest entry.
    pub(crate) fn open_log_at(&mut self, sha: &str) {
        history::open(self, false, Some(sha.to_string()), None);
    }

    /// `Space g h` in visual mode: the history of the selected lines
    /// (git log -L) — selection archaeology (0014 wave 4).
    pub(crate) fn open_line_history(&mut self, start: usize, end: usize) {
        history::open(self, true, None, Some((start, end)));
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
            if self.review.preparing.is_some() {
                self.review_cancel_pub();
            }
            self.cancel_directory_filter(self.current());
            self.git_work.clear_pending_keys();
            self.walker.clear();
            return;
        }
        if self.walker.at_prefix(&["space", "w"]) && key == Key::Char('q') {
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
        if key == Key::Char('c')
            && self.status_view().is_some()
            && (self.walker.at_prefix(&["]"]) || self.walker.at_prefix(&["["]))
        {
            let forward = self.walker.at_prefix(&["]"]);
            let n = self.walker.state.count().unwrap_or(1);
            self.walker.clear();
            for _ in 0..n {
                self.status_hunk_step(forward);
            }
            return;
        }
        if !self.walker.is_ground() {
            return self.feed_command(key);
        }
        if self.status_key(key) || self.directory_key(key) || self.review_key(key) {
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
        self.yank_register(
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

/// The buffer text a diff surface shows: the heading row (its label, the
/// file; the `+N -M` beside it is drawn, 0100 §3.8), then per hunk a
/// header row and unprefixed content rows — exactly the rendered
/// layout (0010 §2).
pub(crate) fn diff_surface_text(label: &str, hunks: &[Hunk]) -> String {
    let mut text = format!("{label}\n");
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

#[cfg(test)]
mod tests;
