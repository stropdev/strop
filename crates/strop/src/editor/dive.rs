//! The dive chain (0010 §3, 0011, R6): commit browser → changed files
//! → file delta, blame card → browser at that commit, sidebar focus.
//! Enter dives and `]f`/`[f` fetch on workers with owned tickets; q
//! unwinds through ReturnPoints (by document id).

use std::path::Path;

use strop_core::worker::{CancelReason, Outcome};
use strop_git::Hunk;

use super::document::Surface;
use super::git_memory::{
    diff_surface_text, hunk_stats, CommitFiles, DiveData, DiveKey, DiveTarget, GitJob,
};
use super::Editor;

impl Editor {
    /// Enter with the blame gutter on: dive into the cursor line's
    /// commit, positioned at its sha (0011 §3). An unloaded or edited-
    /// stale gutter falls back to the single-line card; with the gutter
    /// off, Enter stays inert in normal mode.
    pub(crate) fn dive_from_blame(&mut self) -> bool {
        if self.buf().readonly || self.buf().path.is_none() {
            return false;
        }
        let key = self.blame_key();
        match self.blame_gutters.get(&key) {
            None => false,
            Some(_) if self.blame_gutter_for(self.current()).is_some() => {
                let line = self.buf().line_of(self.head());
                match self.blame_gutters.get(&key).and_then(|g| g.lines.get(line)) {
                    Some(bl) if bl.is_uncommitted() => {
                        self.message = "uncommitted line".into();
                        true
                    }
                    Some(bl) => {
                        let sha = bl.sha.clone();
                        self.open_log_at(&sha);
                        true
                    }
                    None => false,
                }
            }
            Some(_) => {
                self.blame_line(); // still loading (or stale): card
                true
            }
        }
    }

    /// Enter on a surface line dives deeper (0001 pillar 3.2) — the
    /// native fetch (`git show --numstat`, the commit delta) runs on a
    /// worker; the surface appears when its data lands (R6).
    pub(crate) fn dive(&mut self) {
        let doc = self.current();
        let line = self.buf().line_of(self.head());
        let target = match self.surface().cloned() {
            Some(Surface::CommitLog { rows, .. }) => {
                let Some(sha) = rows.get(line).and_then(|r| r.sha.clone()) else {
                    return;
                };
                DiveTarget::CommitFiles { sha }
            }
            Some(Surface::ChangedFiles { sha, files, .. }) => {
                // row 0/1 are the header
                let Some(file) = line.checked_sub(2).and_then(|i| files.get(i)) else {
                    return;
                };
                DiveTarget::FileDelta {
                    sha,
                    path: file.path.clone(),
                }
            }
            _ => return,
        };
        let Some(context) = self.git.clone() else {
            self.message = "not a git repo".into();
            return;
        };
        self.register_dive(DiveKey {
            document: doc,
            workdir: context.workdir().to_path_buf(),
            target,
        });
        self.message = "loading…".into();
    }

    /// `]f` / `[f`: next/previous file of the same commit (0011 §4).
    /// The delta fetch is a worker request; the surface rewrites in
    /// place when the data lands, superseding any earlier dive request
    /// for the same surface.
    pub(crate) fn commit_file_step(&mut self, forward: bool) {
        let Some(Surface::Diff {
            commit: Some(cf),
            label,
            ..
        }) = self.surface().cloned()
        else {
            self.message = "]f/[f: file navigation needs a commit diff".into();
            return;
        };
        if cf.files.len() < 2 {
            self.message = "single-file commit".into();
            return;
        }
        let Some(cur) = cf
            .files
            .iter()
            .position(|f| f.path.display().to_string() == label)
        else {
            self.message = "current file not in commit".into();
            return;
        };
        let n = cf.files.len();
        let next = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        let file = cf.files[next].clone();
        let Some(context) = self.git.clone() else {
            self.message = "not a git repo".into();
            return;
        };
        self.register_dive(DiveKey {
            document: self.current(),
            workdir: context.workdir().to_path_buf(),
            target: DiveTarget::FileDelta {
                sha: cf.sha.clone(),
                path: file.path.clone(),
            },
        });
        self.message = format!("loading {}…", file.path.display());
    }

    /// Register one dive request for a surface document, superseding
    /// (and cancelling) any earlier one, then launch the native fetch.
    fn register_dive(&mut self, key: DiveKey) {
        let doc = key.document;
        if let Some(old) = self.dive_requests.remove(&doc) {
            self.cancel_git_worker(old.request, CancelReason::Superseded);
        }
        let Some(ticket) = self.git_ticket(key) else {
            return;
        };
        self.dive_requests.insert(doc, ticket.clone());
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"dive",
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "target":match &ticket.key.target {
                    DiveTarget::CommitFiles { sha } => format!("files@{sha}"),
                    DiveTarget::FileDelta { sha, path } => {
                        format!("delta@{sha}:{}", path.display())
                    }
                },
            })
        });
        let args = (ticket.clone(),);
        let workdir = ticket.key.workdir.clone();
        let target = ticket.key.target.clone();
        self.launch_git_job(
            "git-dive",
            "git.dive",
            ticket,
            &args,
            GitJob::Dive,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                match &target {
                    DiveTarget::CommitFiles { sha } => {
                        match strop_git::memory::show_stat(&workdir, sha) {
                            Ok(files) => Outcome::Success(DiveData::Files(files)),
                            Err(message) => {
                                Outcome::failed(strop_core::worker::FailureKind::Exit, message)
                            }
                        }
                    }
                    DiveTarget::FileDelta { sha, path } => {
                        let repo = match super::git_memory::repo_or_unavailable(&workdir) {
                            Ok(repo) => repo,
                            Err(failure) => {
                                return Outcome::Failed {
                                    failure,
                                    partial: None,
                                }
                            }
                        };
                        match repo.commit_file_diff(sha, path) {
                            Ok(diff) => Outcome::Success(DiveData::Delta(diff)),
                            Err(message) => {
                                Outcome::failed(strop_core::worker::FailureKind::Exit, message)
                            }
                        }
                    }
                }
            },
        );
    }

    /// Tab on a commit diff: hop focus between the file sidebar and
    /// the diff content (tuicr's model, 0011 §4).
    pub(crate) fn toggle_sidebar_focus(&mut self) {
        let Some(Some(Surface::Diff {
            commit: Some(_),
            sidebar_focus,
            ..
        })) = self
            .docs
            .get_mut(self.current())
            .map(|d| d.surface_payload_mut())
        else {
            self.message = "tab: no file sidebar here".into();
            return;
        };
        *sidebar_focus = !*sidebar_focus;
    }

    pub(crate) fn sidebar_focused(&self) -> bool {
        matches!(
            self.surface(),
            Some(Surface::Diff {
                sidebar_focus: true,
                ..
            })
        )
    }

    /// Swap the current diff surface to another file of the same
    /// commit: surface data and buffer text in place, cursor to top.
    pub(crate) fn load_commit_delta(&mut self, cf: &CommitFiles, path: &Path, hunks: Vec<Hunk>) {
        let (added, deleted) = hunk_stats(&hunks);
        let label = path.display().to_string();
        let text = diff_surface_text(&label, &hunks);
        let idx = self.current();
        if let Err(error) = self.replace_system(idx, &text) {
            super::trace::services::rejected("git", "delta rewrite failed");
            self.message = format!("delta rewrite failed: {error}");
            return;
        }
        if let Some(Some(Surface::Diff {
            label: slot,
            hunks: hunk_slot,
            added: add_slot,
            deleted: del_slot,
            ..
        })) = self.docs.get_mut(idx).map(|d| d.surface_payload_mut())
        {
            *slot = label.clone();
            *hunk_slot = hunks;
            *add_slot = added;
            *del_slot = deleted;
        }
        // the highlighter follows the file the surface now shows (the
        // pure detector reads the rewritten surface's own rope)
        let hl = strop_syntax::Highlighter::for_path(
            std::path::Path::new(&label),
            self.doc(idx).buf.text(),
        );
        self.doc_mut(idx).highlighter = hl;
        self.set_head(0);
        self.view_mut().view_top = 0;
        let pos = cf
            .files
            .iter()
            .position(|f| f.path == path)
            .map_or(0, |i| i + 1);
        self.message = format!("{label} · {pos}/{}", cf.files.len());
    }
}
