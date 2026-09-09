//! Git working surface (M2, reworked 0010/R6/R9): hunks between HEAD
//! and the live buffer, refreshed through owned worker requests; hunk
//! nav and the hunk verbs. No native git work runs on the input or
//! render path — discovery, diffs and index mutations are worker jobs
//! with terminal, ticket-owned results.
mod context;

use std::sync::Arc;
use strop_core::worker::{CancelReason, Load, Outcome};
use strop_git::Hunk;

use crate::files::FileTarget;

use super::git_memory::{
    git_failure, repo_or_unavailable, GitJob, GitMutation, HunkData, HunkKey, HunkSet, MutationKey,
    MutationKind, MutationOp,
};
use super::transact::ChangeSet;
use super::Editor;

/// What a hunk verb (`Space g u`/`g s`) targets from the current view.
enum HunkTarget {
    /// Not on a hunk surface: act on the cursor's own buffer.
    NotASurface,
    /// A hunk preview whose origin buffer still matches the revision
    /// it was captured at.
    Fresh {
        buffer: strop_core::id::DocumentId,
        hunk: Arc<Hunk>,
        /// The origin buffer was untracked when captured: undo refuses.
        untracked: bool,
    },
    /// The origin buffer changed since the preview opened — applying
    /// the stored region would cut the wrong lines.
    Stale,
}

impl Editor {
    /// Register the next gutter diff. Render-safe (R6): pure checks and
    /// registration only — the diff itself runs on a worker against an
    /// immutable text snapshot.
    pub fn refresh_hunks(&mut self) {
        if self.docs.is_empty() || self.finishing {
            return;
        }
        let Some(context) = self.git.clone() else {
            return; // discovery pending (or honestly not a repo)
        };
        let doc = self.current();
        let Some(document) = self.docs.get(doc) else {
            return;
        };
        let revision = document.buf.revision();
        // the buffer's file identity: a local path or a remote file —
        // a remote buffer never contributes a local path spelling
        let file = match &document.source {
            super::document::DocumentSource::Remote(remote) => {
                FileTarget::Remote(remote.file.clone().into())
            }
            super::document::DocumentSource::File => match document.buf.path.as_deref() {
                Some(path) => FileTarget::Local(std::path::PathBuf::from(path)),
                // a scratch buffer has no git identity: no owner, no vectors
                None => {
                    self.clear_hunk_view();
                    return;
                }
            },
            // surfaces and output buffers have no git identity either
            _ => {
                self.clear_hunk_view();
                return;
            }
        };
        // the context must be the buffer's own repository: a remote
        // file under any other target (local repo, or a different
        // endpoint) has no gutter here — clear honestly, never diff
        // against the wrong machine
        let remote_file = match (&file, &context.repo) {
            (FileTarget::Remote(remote), strop_git::RepoTarget::Remote { endpoint, .. })
                if endpoint == remote.endpoint() =>
            {
                remote.absolute_file()
            }
            (FileTarget::Local(_), strop_git::RepoTarget::Local { .. }) => None,
            _ => {
                self.clear_hunk_view();
                return;
            }
        };
        if remote_file.is_some() && !self.remote_window_complete() {
            // a partial window's line numbers are window-relative: no
            // full-file hunk coordinates exist to show (0036)
            self.clear_hunk_view();
            return;
        }
        let key = HunkKey {
            document: doc,
            revision,
            file: file.clone(),
            repo: context.repo.clone(),
            git_view: self.git_view,
        };
        if self.hunk_load.covers(&key) {
            return; // running for this key, or a settled snapshot for it
        }
        let snapshot = document.buf.snapshot();
        // a different key supersedes the old owner synchronously; its
        // late result is rejected by ticket
        self.cancel_hunk_owner();
        let Some(ticket) = self.git_ticket(key) else {
            return;
        };
        self.hunk_load = Load::Running(ticket.clone());
        // stale signs paint WRONG lines after an edit — clear honestly
        // for the frames the diff takes, never lie
        self.hunks = HunkSet::default();
        self.staged_hunks = HunkSet::default();
        self.hunks_untracked = false;
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"hunks",
                "target":if context.repo.is_remote() { "remote" } else { "local" },
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "revision":revision.get(),"file":file.to_string(),
            })
        });
        let args = ticket.clone();
        if let Some(remote) = remote_file {
            // the remote gutter: bounded HEAD/index blob fetches plus
            // the shared hunk semantics — never a local repository
            let endpoint = remote.endpoint().clone();
            let workdir = context.repo.workdir().to_path_buf();
            let head_sha = context.head_sha.clone();
            let rel = context.repo.rel_of(remote.path());
            self.launch_git_job(
                "git-hunks-remote",
                "git.hunks",
                ticket,
                &args,
                GitJob::Hunks,
                move |cancel| {
                    if cancel.is_cancelled() {
                        return Outcome::Cancelled(CancelReason::Superseded);
                    }
                    let Some(rel) = rel else {
                        return Outcome::failed(
                            strop_core::worker::FailureKind::Exit,
                            "buffer is outside the remote repository",
                        );
                    };
                    let text = snapshot.to_string();
                    match strop_git::remote::gutter(
                        &endpoint,
                        &workdir,
                        head_sha.as_deref(),
                        &rel,
                        &text,
                        &cancel,
                    ) {
                        Ok((unstaged, staged, untracked)) => Outcome::Success(HunkData {
                            unstaged: HunkSet::new(unstaged, snapshot.len_lines()),
                            staged: HunkSet::new(staged, snapshot.len_lines()),
                            untracked,
                        }),
                        Err(error) => Outcome::Failed {
                            failure: strop_core::worker::Failure::new(
                                strop_core::worker::FailureKind::Exit,
                                error.to_string(),
                            ),
                            partial: None,
                        },
                    }
                },
            );
            return;
        }
        let workdir = context.repo.workdir().to_path_buf();
        let path = match &file {
            FileTarget::Local(path) => path.clone(),
            FileTarget::Remote(_) => unreachable!("remote handled above"),
        };
        self.launch_git_job(
            "git-hunks",
            "git.hunks",
            ticket,
            &args,
            GitJob::Hunks,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let text = snapshot.to_string();
                let repo = match repo_or_unavailable(&workdir) {
                    Ok(repo) => repo,
                    Err(failure) => {
                        return Outcome::Failed {
                            failure,
                            partial: None,
                        }
                    }
                };
                let unstaged = match repo.unstaged_hunks(&path, &text) {
                    Ok(hunks) => hunks,
                    Err(error) => {
                        return Outcome::Failed {
                            failure: git_failure("diff index↔buffer", error),
                            partial: None,
                        }
                    }
                };
                let staged = match repo.staged_hunks(&path) {
                    Ok(hunks) => hunks,
                    Err(error) => {
                        return Outcome::Failed {
                            failure: git_failure("diff HEAD↔index", error),
                            partial: None,
                        }
                    }
                };
                let untracked = match repo.is_untracked(&path) {
                    Ok(untracked) => untracked,
                    Err(error) => {
                        return Outcome::Failed {
                            failure: git_failure("index lookup", error),
                            partial: None,
                        }
                    }
                };
                Outcome::Success(HunkData {
                    unstaged: HunkSet::new(unstaged, snapshot.len_lines()),
                    staged: HunkSet::new(staged, snapshot.len_lines()),
                    untracked,
                })
            },
        );
    }

    /// Revoke the hunk owner and clear both cached vectors — the
    /// honest state whenever the current buffer cannot take part in a
    /// gutter diff (scratch, surface, mismatched repository or partial
    /// remote window).
    fn clear_hunk_view(&mut self) {
        self.cancel_hunk_owner();
        self.hunks = HunkSet::default();
        self.staged_hunks = HunkSet::default();
        self.hunks_untracked = false;
    }

    /// Revoke the running hunk owner (if any) and return to Idle. The
    /// worker's late result — success, failure or the synthetic
    /// `Cancelled` — is rejected: it no longer owns the view.
    fn cancel_hunk_owner(&mut self) {
        let running = match &self.hunk_load {
            Load::Running(ticket) => Some(ticket.request),
            _ => None,
        };
        if let Some(request) = running {
            self.cancel_git_worker(request, CancelReason::Superseded);
        }
        self.hunk_load = Load::Idle;
    }

    /// Gutter sign for a 1-based buffer line: `+` add, `~` change,
    /// `-` deletion below (0001 pillar 3.1).
    pub fn sign_at(&self, line_1based: usize) -> Option<char> {
        self.hunks.sign(line_1based)
    }

    /// Staged sign (HEAD↔index edge, 0014 wave 4): the line sits inside
    /// a staged hunk's new side. Line alignment between index and live
    /// text is approximate when both sets exist — the gutter's rule:
    /// unstaged wins, staged marks what's already in the index.
    pub fn sign_at_staged(&self, line_1based: usize) -> bool {
        self.staged_hunks.addition(line_1based)
    }

    /// `]c` / `[c`: jump to the next/previous changed line. An explicit
    /// command retries a previously failed diff; render never does.
    pub(crate) fn jump_hunk(&mut self, forward: bool) {
        // a partial remote window has no full-file coordinates to jump
        // by — the honest refusal, not "no more hunks"
        if self.remote_file().is_some() && !self.remote_window_complete() {
            self.message = "partial remote snapshot — hunks need a full window".into();
            return;
        }
        self.hunk_load.retry_failed();
        self.refresh_hunks();
        let cur = self.buf().line_of(self.head()) + 1;
        let target = self.hunks.next_line(cur, forward);
        match target {
            Some(l) => {
                self.set_head(self.buf().line_start(l - 1));
                self.clamp_cursor();
            }
            None => self.message = "no more hunks".into(),
        }
    }

    /// The hunk under the cursor, if any.
    fn hunk_under_cursor(&mut self) -> Option<Arc<Hunk>> {
        self.hunk_load.retry_failed();
        self.refresh_hunks();
        let line = self.buf().line_of(self.head()) + 1;
        self.hunks
            .at_line(line)
            .map(|index| self.hunks[index].clone())
    }

    /// Apply `hunk`'s reverse to buffer `idx`: pure deletions reinsert,
    /// pure additions drop, changes swap old content back — one
    /// pre-edit Replacement through the gateway. `untracked` names an
    /// origin with no HEAD content to restore from.
    fn restore_hunk_in(
        &mut self,
        idx: strop_core::id::DocumentId,
        hunk: &Hunk,
        untracked: bool,
    ) -> bool {
        if self.doc(idx).buf.path.is_none() || untracked {
            return false; // nothing in HEAD to restore from
        }
        let new_first = hunk.changed_region().0;
        // byte-precise restore text (0020 §7): the hunk's own old-side
        // lines carry CRLF and missing-final-newline exactly — the
        // str::lines + LF join it replaces could not
        let old: String = {
            let mut bytes = Vec::new();
            for l in hunk
                .lines
                .iter()
                .filter(|l| l.origin != strop_git::LineOrigin::Addition)
            {
                bytes.extend_from_slice(&l.bytes_with_terminator());
            }
            match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => {
                    self.message = format!("hunk contains non-UTF-8 text: {error}");
                    return false;
                }
            }
        };

        // one validated replacement through the gateway (0024) — the
        // base-revision check refuses a drifted hunk
        let base = self.doc(idx).buf.revision();
        // Header extents include context; the restored old side includes the
        // same context. Mixing changed-only bounds with full text duplicates it.
        let first = if hunk.new_count == 0 {
            hunk.new_start
        } else {
            hunk.new_start.saturating_sub(1)
        };
        let start = self
            .doc(idx)
            .buf
            .line_start(first)
            .min(self.doc(idx).buf.len_bytes());
        let end = if hunk.new_count == 0 {
            start
        } else {
            self.doc(idx)
                .buf
                .line_start(first.saturating_add(hunk.new_count))
                .min(self.doc(idx).buf.len_bytes())
        };
        let replacement =
            strop_core::Replacement::new(strop_core::Range::charwise(start, end), old);
        if let Err(error) = self.apply(
            idx,
            base,
            ChangeSet {
                edits: vec![replacement],
                undo_open: false,
            },
        ) {
            self.message = match error {
                super::transact::ApplyError::Edit(strop_core::EditError::StaleRevision {
                    ..
                }) => "buffer changed — reopen the hunk preview".into(),
                other => format!("hunk reset failed: {other}"),
            };
            return false;
        }
        // cursor placement: the target's own pane moves to the
        // restored region
        let land = self
            .doc(idx)
            .buf
            .line_start((new_first - 1).min(self.doc(idx).buf.len_lines().saturating_sub(1)));
        if self.current() == idx {
            self.set_head(land);
            self.clamp_cursor();
            self.flash(strop_core::Range::charwise(self.head(), self.head()));
        } else if let Some(pane) = self.panes.iter_mut().find(|p| p.doc == idx) {
            pane.sels.collapse_primary(land);
        }
        true
    }

    /// `Space g u`: reset a hunk to HEAD's content. From the hunk
    /// surface it restores the origin buffer's hunk (0010 §2). A
    /// remote origin refuses: the snapshot is readonly and the remote
    /// repository has no writable index to restore from (RW4).
    pub(crate) fn undo_hunk(&mut self) {
        if self.remote_endpoint().is_some() {
            self.message = "remote Git mutations are not supported".into();
            return;
        }
        let remote_refusal = |editor: &Self, idx| {
            matches!(
                editor.docs.get(idx).map(|d| &d.source),
                Some(super::document::DocumentSource::Remote(_))
            )
            .then(|| "remote snapshots are read-only — hunk reset refused".to_string())
        };
        match self.hunk_surface_target() {
            HunkTarget::Fresh {
                buffer,
                hunk,
                untracked,
            } => {
                if let Some(message) = remote_refusal(self, buffer) {
                    self.message = message;
                } else if self.restore_hunk_in(buffer, &hunk, untracked) {
                    self.message = "hunk reset".into();
                }
            }
            HunkTarget::Stale => self.message = "buffer changed — reopen the hunk preview".into(),
            HunkTarget::NotASurface => {
                let Some(hunk) = self.hunk_under_cursor() else {
                    self.message = "no hunk here".into();
                    return;
                };
                if let Some(message) = remote_refusal(self, self.current()) {
                    self.message = message;
                    return;
                }
                let untracked = self.hunks_untracked;
                if self.restore_hunk_in(self.current(), &hunk, untracked) {
                    self.message = "hunk reset".into();
                }
            }
        }
    }

    /// `Space g s`: stage a hunk (index ← worktree edge). The index
    /// write runs on a worker, serialized FIFO with every other
    /// mutation — the input path only validates and queues. A remote
    /// repository refuses by capability (RW4): there is no remote
    /// index write, and nothing ever falls back to a local one.
    pub(crate) fn stage_hunk(&mut self) {
        if self.remote_endpoint().is_some() {
            self.message = "remote Git mutations are not supported".into();
            return;
        }
        match self.hunk_surface_target() {
            HunkTarget::Fresh { buffer, hunk, .. } => self.stage_hunk_in(buffer, &hunk),
            HunkTarget::Stale => self.message = "buffer changed — reopen the hunk preview".into(),
            HunkTarget::NotASurface => {
                let Some(hunk) = self.hunk_under_cursor() else {
                    self.message = "no hunk here".into();
                    return;
                };
                self.stage_hunk_in(self.current(), &hunk);
            }
        }
    }

    fn stage_hunk_in(&mut self, idx: strop_core::id::DocumentId, hunk: &Arc<Hunk>) {
        let Some(path) = self.doc(idx).buf.path.clone() else {
            return;
        };
        // staging reads the *disk* file's hunk: a dirty buffer means the
        // two disagree, and auto-saving would silently write every
        // unrelated unsaved edit to the worktree (0014). Refuse loudly.
        if self.doc(idx).buf.dirty {
            self.message = "unsaved changes — :w first, then stage".into();
            return;
        }
        let Some(context) = self.git.clone() else {
            self.message = "not a git repo".into();
            return;
        };
        if context.repo.is_remote() {
            self.message = "remote repositories are read-only — staging is refused (RW4)".into();
            return;
        }
        let Ok(rel) = std::path::Path::new(&path)
            .strip_prefix(context.workdir())
            .map(|p| p.to_path_buf())
        else {
            self.message = "buffer not under workdir".into();
            return;
        };
        let key = MutationKey {
            document: idx,
            revision: self.doc(idx).buf.revision(),
            kind: MutationKind::Stage,
            rel,
            repo: context.repo.clone(),
            git_view: self.git_view,
        };
        self.git_mutations.push_back(GitMutation {
            key,
            op: MutationOp::Stage { hunk: hunk.clone() },
        });
        self.pump_git_mutations();
    }

    /// `Space g S`: unstage the hunk under the cursor — the index→HEAD
    /// edge. Queued like staging; the index write never blocks input.
    /// A remote repository refuses by capability (RW4) — no remote
    /// index exists to write, and the local one is never touched.
    pub(crate) fn unstage_hunk(&mut self) {
        if self.remote_endpoint().is_some() {
            self.message = "remote Git mutations are not supported".into();
            return;
        }
        if self.buf().dirty {
            self.message = "unsaved changes — :w first".into();
            return;
        }
        let line = self.buf().line_of(self.head()) + 1;
        let Some(hunk) = self
            .staged_hunks
            .at_line(line)
            .map(|index| self.staged_hunks[index].clone())
        else {
            self.message = "no staged hunk here".into();
            return;
        };
        let Some(path) = self.buf().path.clone() else {
            return;
        };
        let Some(context) = self.git.clone() else {
            self.message = "not a git repo".into();
            return;
        };
        if context.repo.is_remote() {
            self.message = "remote repositories are read-only — unstaging is refused (RW4)".into();
            return;
        }
        let Ok(rel) = std::path::Path::new(&path)
            .strip_prefix(context.workdir())
            .map(|p| p.to_path_buf())
        else {
            self.message = "buffer not under workdir".into();
            return;
        };
        let key = MutationKey {
            document: self.current(),
            revision: self.buf().revision(),
            kind: MutationKind::Unstage,
            rel,
            repo: context.repo.clone(),
            git_view: self.git_view,
        };
        self.git_mutations.push_back(GitMutation {
            key,
            op: MutationOp::Unstage { hunk },
        });
        self.pump_git_mutations();
    }

    /// `Space g p`: preview the hunk under the cursor as a diff surface
    /// (0010 §2) — a readonly buffer you can move in; `q` closes,
    /// `Space g u`/`g s` still act on the file.
    pub(crate) fn preview_hunk(&mut self) {
        self.hunk_load.retry_failed();
        self.refresh_hunks();
        let line = self.buf().line_of(self.head()) + 1;
        let Some(index) = self.hunks.at_line(line) else {
            self.message = "no hunk here".into();
            return;
        };
        let origin = super::git_memory::HunkOrigin {
            buffer: self.current(),
            revision: self.cur().buf.revision(),
            untracked: self.hunks_untracked,
        };
        let Some(context) = self.git_context() else {
            return;
        };
        let key = super::git_memory::DiveKey {
            document: self.current(),
            repo: context.repo.clone(),
            target: super::git_memory::DiveTarget::HunkPreview { origin, index },
        };
        self.message = "loading hunk…".into();
        self.register_dive(key);
    }

    /// What a `Space g u`/`g s` from the current buffer should act on:
    /// the hunk surface's origin when fresh, a refusal when the origin
    /// buffer has moved on, and the cursor's own hunk otherwise.
    fn hunk_surface_target(&self) -> HunkTarget {
        let Some(super::Surface::Diff { hunks, origin, .. }) = self.surface() else {
            return HunkTarget::NotASurface;
        };
        let Some(origin) = origin else {
            return HunkTarget::NotASurface; // commit delta: nothing to undo
        };
        let Some(hunk) = hunks.first() else {
            return HunkTarget::NotASurface;
        };
        match self.docs.get(origin.buffer) {
            Some(d) if d.buf.revision() == origin.revision => HunkTarget::Fresh {
                buffer: origin.buffer,
                hunk: hunk.clone(),
                untracked: origin.untracked,
            },
            _ => HunkTarget::Stale,
        }
    }
}

#[cfg(test)]
mod tests;
