//! The dive chain (0010 §3, 0011, R6): commit browser → changed files
//! → file delta, blame card → browser at that commit, sidebar focus.
//! Enter dives and `]f`/`[f` fetch on workers with owned tickets; q
//! unwinds through ReturnPoints (by document id).

use std::path::Path;

use strop_core::worker::{CancelReason, Outcome};

use super::document::Surface;
use super::git_memory::{
    CommitFiles, DiveData, DiveKey, DiveTarget, GitJob, PreparedDiff, PreparedFiles,
};
use super::Editor;

impl Editor {
    /// Enter with the blame gutter on: dive into the cursor line's
    /// commit, positioned at its sha (0011 §3). An unloaded or edited-
    /// stale gutter falls back to the single-line card; with the gutter
    /// off, Enter stays inert in normal mode.
    pub(crate) fn dive_from_blame(&mut self) -> bool {
        let key = self.current();
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
    /// worker; the surface appears when its data lands (R6). The fetch
    /// runs against the repository the *origin buffer* lives in: a
    /// surface opened from a remote file dives on that endpoint, never
    /// the local cwd (0036 RW8).
    pub(crate) fn dive(&mut self) {
        let doc = self.current();
        let line = self.buf().line_of(self.head());
        let target = match self.surface() {
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
                    sha: sha.clone(),
                    path: file.path.clone(),
                }
            }
            _ => return,
        };
        let Some(context) = self.git_context().cloned() else {
            self.message = "not a git repo".into();
            return;
        };
        self.message = "loading…".into();
        self.register_dive(DiveKey {
            document: doc,
            repo: context.repo.clone(),
            target,
        });
    }

    /// `]f` / `[f`: next/previous file of the same commit (0011 §4).
    /// The delta fetch is a worker request; the surface rewrites in
    /// place when the data lands, superseding any earlier dive request
    /// for the same surface. Provenance comes from the delta's own
    /// [`CommitFiles`] — the repository the commit was listed in, not
    /// whatever buffer is currently active.
    pub(crate) fn commit_file_step(&mut self, forward: bool) {
        let Some(Surface::Diff {
            commit: Some(cf), ..
        }) = self.surface().cloned()
        else {
            self.message = "]f/[f: file navigation needs a commit diff".into();
            return;
        };
        if cf.files.len() < 2 {
            self.message = "single-file commit".into();
            return;
        }
        let Some(cur) = cf.files.index_of(&cf.current) else {
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
        self.message = format!("loading {}…", file.path.display());
        self.register_dive(DiveKey {
            document: self.current(),
            repo: cf.repo.clone(),
            target: DiveTarget::FileDelta {
                sha: cf.sha.clone(),
                path: file.path.clone(),
            },
        });
    }

    /// Register one dive request for a surface document, superseding
    /// (and cancelling) any earlier one, then launch the native fetch.
    pub(super) fn register_dive(&mut self, key: DiveKey) {
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
                "repository":if ticket.key.repo.is_remote() { "remote" } else { "local" },
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "target":match &ticket.key.target {
                    DiveTarget::HunkPreview { .. } => "hunk".into(),
                    DiveTarget::CommitFiles { sha } => format!("files@{sha}"),
                    DiveTarget::FileDelta { sha, path } => {
                        format!("delta@{sha}:{}", path.display())
                    }
                },
            })
        });
        let args = (ticket.clone(),);
        let repo = ticket.key.repo.clone();
        let target = ticket.key.target.clone();
        let captured_hunks = self.hunks.clone();
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
                    DiveTarget::HunkPreview { index, .. } => match captured_hunks.get(*index) {
                        Some(hunk) => Outcome::Success(DiveData::Delta(PreparedDiff::new(
                            "hunk".into(),
                            vec![hunk.as_ref().clone()],
                        ))),
                        None => Outcome::failed(
                            strop_core::worker::FailureKind::InvalidInput,
                            "hunk snapshot no longer contains the selected hunk",
                        ),
                    },
                    DiveTarget::CommitFiles { sha } => {
                        // numstat is one bounded run on either backend
                        let exec = strop_git::GitExec::for_target(&repo);
                        match strop_git::memory::show_stat(&exec, &cancel, sha) {
                            Ok(files) => Outcome::Success(DiveData::Files(PreparedFiles::new(
                                sha.clone(),
                                files,
                            ))),
                            Err(message) => {
                                Outcome::failed(strop_core::worker::FailureKind::Exit, message)
                            }
                        }
                    }
                    DiveTarget::FileDelta { sha, path } => match &repo {
                        strop_git::RepoTarget::Local { .. } => {
                            let native =
                                match super::git_memory::repo_or_unavailable(repo.workdir()) {
                                    Ok(native) => native,
                                    Err(failure) => {
                                        return Outcome::Failed {
                                            failure,
                                            partial: None,
                                        }
                                    }
                                };
                            match native.commit_file_diff(sha, path) {
                                Ok(diff) => Outcome::Success(DiveData::Delta(PreparedDiff::new(
                                    strop_core::layout::printable_text(path.to_string_lossy())
                                        .into_owned(),
                                    diff.hunks,
                                ))),
                                Err(message) => {
                                    Outcome::failed(strop_core::worker::FailureKind::Exit, message)
                                }
                            }
                        }
                        strop_git::RepoTarget::Remote { endpoint, workdir } => {
                            match strop_git::remote::commit_file_diff(
                                endpoint, workdir, sha, path, &cancel,
                            ) {
                                Ok(diff) => Outcome::Success(DiveData::Delta(PreparedDiff::new(
                                    strop_core::layout::printable_text(path.to_string_lossy())
                                        .into_owned(),
                                    diff.hunks,
                                ))),
                                Err(error) => Outcome::Failed {
                                    failure: strop_core::worker::Failure::new(
                                        strop_core::worker::FailureKind::Exit,
                                        error.to_string(),
                                    ),
                                    partial: None,
                                },
                            }
                        }
                    },
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
    pub(crate) fn load_commit_delta(&mut self, cf: &CommitFiles, path: &Path, hunks: PreparedDiff) {
        let label = hunks.label().to_owned();
        let idx = self.current();
        let replaced = self
            .doc_mut(idx)
            .buf
            .system_edit()
            .replace_rope(hunks.text());
        if let Err(error) = replaced {
            super::trace::services::rejected("git", "delta rewrite failed");
            self.message = format!("delta rewrite failed: {error}");
            return;
        }
        if let Some(Some(Surface::Diff {
            hunks: hunk_slot,
            commit: Some(commit),
            ..
        })) = self.docs.get_mut(idx).map(|d| d.surface_payload_mut())
        {
            *hunk_slot = hunks;
            commit.current = path.to_path_buf();
        }
        self.set_head(0);
        self.view_mut().view_top = 0;
        let pos = cf.files.index_of(path).map_or(0, |i| i + 1);
        self.message = format!("{label} · {pos}/{}", cf.files.len());
    }
}
