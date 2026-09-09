//! Blame (0011 §3, R9/R6): the left-margin gutter (rootle's shape),
//! the card, and the dive into the blamed commit. Requests own
//! tickets; a stale revision, a superseded request or an edited buffer
//! drops the pairing honestly — and every failure is terminal, so the
//! next explicit toggle genuinely starts a new request.
//!
//! Remote buffers (0036 RW8) blame through the same seam: bounded
//! remote `git blame` on the endpoint, keyed by an endpoint-qualified
//! identity so a remote entry can never pair with a local buffer — or
//! the reverse. Blame speaks file line numbers, so a partial remote
//! window refuses instead of attributing window-relative lines.

use std::path::PathBuf;

use strop_core::worker::{CancelReason, Outcome};

use super::git_memory::{BlameGutter, BlameKey, CardKey, GitJob};
use super::Editor;

impl Editor {
    /// `Space g b`: on a file buffer (local, or a remote buffer with a
    /// complete window), toggle the blame gutter; anywhere else (or as
    /// feedback while the gutter loads) the single-line card (0011 §3).
    pub(crate) fn toggle_blame_gutter(&mut self) {
        let remote = self.remote_file().cloned();
        let local_file = remote.is_none() && !self.buf().readonly && self.buf().path.is_some();
        if !local_file {
            if remote.is_some() {
                if !self.remote_window_complete() {
                    self.message = "partial remote snapshot — blame needs a full window".into();
                    return;
                }
            } else {
                return self.blame_line();
            }
        }
        let Some(context) = self.git_context().cloned() else {
            self.message = "not a git repo".into();
            return;
        };
        // the context must be this buffer's repository: a remote file
        // blames on its own endpoint, never any local workdir
        if let Some(file) = &remote {
            if context.repo.endpoint() != Some(file.endpoint()) {
                self.message = "buffer's repository is not the current context".into();
                return;
            }
        }
        let key = self.current();
        if let Some(mut gutter) = self.blame_gutters.remove(&key) {
            // toggle off: revoke the pending request — its late result
            // cannot repopulate a newer incarnation of this entry
            if let Some(ticket) = gutter.request.take() {
                self.cancel_git_worker(ticket.request, CancelReason::Dismissed);
            }
            return;
        }
        let doc = self.current();
        let revision = self.buf().revision();
        let Some(file) = self.doc(doc).file_target(&self.cwd) else {
            return;
        };
        // the loading marker owns its ticket BEFORE any launch decision
        // (replay contract); pure-validation failures settle through
        // the same terminal path as worker failures
        let Some(ticket) = self.git_ticket(BlameKey {
            document: doc,
            revision,
            file,
            repo: context.repo.clone(),
        }) else {
            return;
        };
        self.blame_gutters.insert(
            key,
            BlameGutter {
                lines: Vec::new(),
                revision,
                request: Some(ticket.clone()),
            },
        );
        self.spawn_blame_file(ticket);
        self.blame_line(); // the card covers the line until data lands
    }

    fn spawn_blame_file(&mut self, ticket: strop_core::worker::Ticket<BlameKey>) {
        let repo = ticket.key.repo.clone();
        let rel = match (&repo, &ticket.key.file) {
            (strop_git::RepoTarget::Local { .. }, crate::files::FileTarget::Local(path)) => {
                repo.rel_of(path)
            }
            (
                strop_git::RepoTarget::Remote { endpoint, .. },
                crate::files::FileTarget::Remote(location),
            ) if location.endpoint() == endpoint => location
                .absolute_file()
                .and_then(|file| repo.rel_of(file.path())),
            _ => None,
        };
        let Some(rel) = rel else {
            self.send_git_failure(
                ticket,
                strop_core::worker::FailureKind::InvalidInput,
                "blame failed: buffer not under workdir",
                GitJob::Gutter,
            );
            return;
        };
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"blame_gutter",
                "repository":if repo.is_remote() { "remote" } else { "local" },
                "file": &ticket.key.file,
                "document":{"slot":ticket.key.document.index(),"generation":ticket.key.document.generation()},
                "revision":ticket.key.revision.get(),
            })
        });
        let args = ticket.clone();
        self.launch_git_job(
            "git-blame-file",
            "git.blame_gutter",
            ticket,
            &args,
            GitJob::Gutter,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let exec = strop_git::GitExec::for_target(&repo);
                match strop_git::memory::blame_file(&exec, &cancel, &rel) {
                    Ok(lines) => Outcome::Success(lines),
                    Err(message) => Outcome::failed(strop_core::worker::FailureKind::Exit, message),
                }
            },
        );
    }

    /// The buffer's blame gutter, if its data is still trustworthy:
    /// same revision, same line count. Any edit since the capture
    /// voids the line↔buffer-line pairing. A remote buffer pairs only
    /// with its endpoint-qualified entry.
    pub fn blame_gutter_for(&self, buffer: strop_core::id::DocumentId) -> Option<&BlameGutter> {
        let document = self.docs.get(buffer)?;
        let buf = &document.buf;
        if document
            .remote_metadata()
            .is_some_and(|source| !source.window.is_complete())
            || self.remote_following(buffer)
        {
            return None;
        }
        let gutter = self.blame_gutters.get(&buffer)?;
        // len_lines counts the trailing newline's phantom line — the
        // content count is what blame rows pair with
        let content_lines = buf.last_content_line() + 1;
        (gutter.revision == buf.revision() && gutter.lines.len() == content_lines).then_some(gutter)
    }

    /// `Space g b` fallback / surface blame: the card for the cursor
    /// line. One card request at a time — a new request supersedes (and
    /// cancels) the old one. The card speaks file line numbers: a
    /// partial remote window refuses rather than blaming the wrong
    /// line of the file.
    pub(crate) fn blame_line(&mut self) {
        match self.remote_file().cloned() {
            Some(file) => {
                // the card names a file line: a partial window's line
                // numbers are window-relative — refuse, never blame
                // the wrong line of the file
                if !self.remote_window_complete() {
                    self.message = "partial remote snapshot — blame needs a full window".into();
                    return;
                }
                self.blame_line_remote(file);
            }
            None => match self.buf().path.clone() {
                Some(path) => self.blame_line_local(path),
                None => self.message = "blame works on file buffers".into(),
            },
        }
    }
    fn blame_line_local(&mut self, path: PathBuf) {
        let Some(context) = self.git_context().cloned() else {
            self.message = "not a git repo".into();
            return;
        };
        if context.repo.is_remote() {
            self.message = "buffer's repository is not the current context".into();
            return;
        }
        if let Some(ticket) = self.card_request.take() {
            self.cancel_git_worker(ticket.request, CancelReason::Superseded);
        }
        let doc = self.current();
        let revision = self.buf().revision();
        let line = self.buf().line_of(self.head()) + 1;
        let Some(file) = self.doc(doc).file_target(&self.cwd) else {
            return;
        };
        let Some(ticket) = self.git_ticket(CardKey {
            origin: BlameKey {
                document: doc,
                revision,
                file,
                repo: context.repo.clone(),
            },
            line,
        }) else {
            return;
        };
        self.card_request = Some(ticket.clone());
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"blame_line",
                "repository":"local",
                "path":path.to_string_lossy(),"line":line,
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "revision":revision.get(),
            })
        });
        let abs = if path.is_absolute() {
            path.clone()
        } else {
            context.workdir().join(&path)
        };
        let Some(rel) = context.repo.rel_of(&abs) else {
            self.send_git_failure(
                ticket,
                strop_core::worker::FailureKind::InvalidInput,
                "blame failed: not under workdir",
                GitJob::Card,
            );
            return;
        };
        self.launch_blame_line(ticket, context.repo, rel, line);
    }

    fn blame_line_remote(&mut self, file: strop_workspace::RemoteFile) {
        let Some(context) = self.git_context().cloned() else {
            self.message = "not a git repo".into();
            return;
        };
        if context.repo.endpoint() != Some(file.endpoint()) {
            self.message = "buffer's repository is not the current context".into();
            return;
        }
        if let Some(ticket) = self.card_request.take() {
            self.cancel_git_worker(ticket.request, CancelReason::Superseded);
        }
        let doc = self.current();
        let revision = self.buf().revision();
        let line = self.buf().line_of(self.head()) + 1;
        let Some(ticket) = self.git_ticket(CardKey {
            origin: BlameKey {
                document: doc,
                revision,
                file: crate::files::FileTarget::Remote(file.clone().into()),
                repo: context.repo.clone(),
            },
            line,
        }) else {
            return;
        };
        self.card_request = Some(ticket.clone());
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"blame_line",
                "repository":"remote",
                "path":file.path().to_string_lossy(),"line":line,
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "revision":revision.get(),
            })
        });
        let Some(rel) = context.repo.rel_of(file.path()) else {
            self.send_git_failure(
                ticket,
                strop_core::worker::FailureKind::InvalidInput,
                "blame failed: buffer not under the remote workdir",
                GitJob::Card,
            );
            return;
        };
        self.launch_blame_line(ticket, context.repo, rel, line);
    }

    /// The card's one bounded run: `git blame --line-porcelain -L` on
    /// whichever backend owns the repository.
    fn launch_blame_line(
        &mut self,
        ticket: strop_core::worker::Ticket<CardKey>,
        repo: strop_git::RepoTarget,
        rel: PathBuf,
        line: usize,
    ) {
        let args = ticket.clone();
        self.launch_git_job(
            "git-blame-line",
            "git.blame_line",
            ticket,
            &args,
            GitJob::Card,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let exec = strop_git::GitExec::for_target(&repo);
                match strop_git::memory::blame_line(&exec, &cancel, &rel, line) {
                    Ok(card) => Outcome::Success(Box::new(card)),
                    Err(message) => Outcome::failed(strop_core::worker::FailureKind::Exit, message),
                }
            },
        );
    }

    /// Card authority dismissal (the feed path calls this BEFORE the
    /// key is interpreted): a visible card is taken (the caller decides
    /// whether the key also acts), and any pending request is revoked —
    /// a late card can never reappear after its dismissal.
    pub(crate) fn dismiss_card_authority(&mut self) -> Option<strop_git::memory::BlameCard> {
        if let Some(ticket) = self.card_request.take() {
            self.cancel_git_worker(ticket.request, CancelReason::Dismissed);
        }
        self.blame_card.take()
    }
}
