//! Blame (0011 §3, R9/R6): the left-margin gutter (rootle's shape),
//! the card, and the dive into the blamed commit. Requests own
//! tickets; a stale revision, a superseded request or an edited buffer
//! drops the pairing honestly — and every failure is terminal, so the
//! next explicit toggle genuinely starts a new request.

use std::path::{Path, PathBuf};

use strop_core::worker::{CancelReason, Outcome};

use super::git_memory::{BlameGutter, BlameKey, CardKey, GitJob};
use super::Editor;

impl Editor {
    /// `Space g b`: on a file buffer, toggle the blame gutter; anywhere
    /// else (or as feedback while the gutter loads) the single-line
    /// card (0011 §3).
    pub(crate) fn toggle_blame_gutter(&mut self) {
        if self.buf().readonly || self.buf().path.is_none() {
            return self.blame_line();
        }
        let key = self.blame_key();
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
        let workdir = self.git.as_ref().map(|c| c.workdir().to_path_buf());
        // the loading marker owns its ticket BEFORE any launch decision
        // (replay contract); pure-validation failures settle through
        // the same terminal path as worker failures
        let Some(ticket) = self.git_ticket(BlameKey {
            document: doc,
            revision,
            path: key.clone(),
            workdir: workdir.clone().unwrap_or_default(),
        }) else {
            return;
        };
        self.blame_gutters.insert(
            key.clone(),
            BlameGutter {
                lines: Vec::new(),
                revision,
                request: Some(ticket.clone()),
            },
        );
        self.spawn_blame_file(&key, ticket);
        self.blame_line(); // the card covers the line until data lands
    }

    /// Canonical path key for the current buffer's gutter entry — the
    /// same normalization every lookup uses, so `f.rs` and an absolute
    /// path for one file share one entry.
    pub(crate) fn blame_key(&self) -> PathBuf {
        self.buf()
            .file_identity()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                self.cwd
                    .join(self.buf().path.as_deref().unwrap_or_else(|| Path::new("")))
            })
    }

    pub(crate) fn blame_key_of(&self, path: &Path) -> PathBuf {
        self.docs
            .iter()
            .find_map(|(_, document)| {
                (document.buf.path.as_deref() == Some(path))
                    .then(|| document.buf.file_identity())
                    .flatten()
            })
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.cwd.join(path))
    }

    fn spawn_blame_file(&mut self, key: &Path, ticket: strop_core::worker::Ticket<BlameKey>) {
        let Some(workdir) = self.git.as_ref().map(|c| c.workdir().to_path_buf()) else {
            self.send_git_failure(
                ticket,
                strop_core::worker::FailureKind::Unavailable,
                "blame failed: not a git repo",
                GitJob::Gutter,
            );
            return;
        };
        let Ok(rel) = key.strip_prefix(&workdir).map(|r| r.to_path_buf()) else {
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
                "path":key.to_string_lossy(),
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
                match strop_git::memory::blame_file(&workdir, &rel) {
                    Ok(lines) => Outcome::Success(lines),
                    Err(message) => Outcome::failed(strop_core::worker::FailureKind::Exit, message),
                }
            },
        );
    }
    /// The buffer's blame gutter, if its data is still trustworthy:
    /// same revision, same line count. Any edit since the capture
    /// voids the line↔buffer-line pairing.
    pub fn blame_gutter_for(&self, buffer: strop_core::id::DocumentId) -> Option<&BlameGutter> {
        let buf = self.docs.get(buffer).map(|d| &d.buf)?;
        let path = buf.path.as_deref()?;
        let gutter = if let Some(identity) = buf.file_identity() {
            self.blame_gutters.get(identity)?
        } else {
            self.blame_gutters.get(&self.cwd.join(path))?
        };
        // len_lines counts the trailing newline's phantom line — the
        // content count is what blame rows pair with
        let content_lines = buf.last_content_line() + 1;
        (gutter.revision == buf.revision() && gutter.lines.len() == content_lines).then_some(gutter)
    }

    /// `Space g b` fallback / surface blame: the card for the cursor
    /// line. One card request at a time — a new request supersedes (and
    /// cancels) the old one.
    pub(crate) fn blame_line(&mut self) {
        let Some(path) = self.buf().path.clone() else {
            self.message = "blame works on file buffers".into();
            return;
        };
        if let Some(ticket) = self.card_request.take() {
            self.cancel_git_worker(ticket.request, CancelReason::Superseded);
        }
        let doc = self.current();
        let revision = self.buf().revision();
        let line = self.buf().line_of(self.head()) + 1;
        let workdir = self.git.as_ref().map(|c| c.workdir().to_path_buf());
        let Some(ticket) = self.git_ticket(CardKey {
            origin: BlameKey {
                document: doc,
                revision,
                path: self.blame_key_of(&path),
                workdir: workdir.clone().unwrap_or_default(),
            },
            line,
        }) else {
            return;
        };
        self.card_request = Some(ticket.clone());
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"blame_line",
                "path":path.to_string_lossy(),"line":line,
                "document":{"slot":doc.index(),"generation":doc.generation()},
                "revision":revision.get(),
            })
        });
        let Some(workdir) = workdir else {
            self.send_git_failure(
                ticket,
                strop_core::worker::FailureKind::Unavailable,
                "blame failed: not a git repo",
                GitJob::Card,
            );
            return;
        };
        let abs = if path.is_absolute() {
            path.clone()
        } else {
            workdir.join(&path)
        };
        let Ok(rel) = abs.strip_prefix(&workdir).map(|r| r.to_path_buf()) else {
            self.send_git_failure(
                ticket,
                strop_core::worker::FailureKind::InvalidInput,
                "blame failed: not under workdir",
                GitJob::Card,
            );
            return;
        };
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
                match strop_git::memory::blame_line(&workdir, &rel, line) {
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
