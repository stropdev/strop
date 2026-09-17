//! One lock orders initialize, open/change/close and request admission;
//! the ordered wire queue turns those admissions into frames in the
//! same order. Notification enqueueing is admission success, not server
//! acknowledgment. Text travels as owned rope snapshots (R6): the
//! editor never materializes a full-document String.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use async_lsp::lsp_types as lt;
use ropey::Rope;
use strop_core::id::{BufferRevision, DocumentId};

use super::queue::{Admission, WireEnv, WireJob};
use super::Client;
use crate::protocol::*;

#[derive(Default)]
pub(super) struct SyncState {
    pub(super) ready: bool,
    pub(super) documents: HashMap<PathBuf, OpenDocument>,
    /// Pre-init admitted requests, flushed in order by
    /// `finish_initialize`. Bounded (0056 AR06): a server stuck in
    /// handshake cannot accumulate requests without limit.
    pub(super) pending_requests: Vec<PendingRequest>,
    /// Allocated versions are monotonic across reopens, so a stale
    /// versioned diagnostic can never relabel itself.
    next_version: Option<WireVersion>,
    seen_paths: HashSet<PathBuf>,
}

/// Bound on pre-init admitted requests (AR06). Launch refusal is a
/// terminal Note, never a silent drop (R9).
pub(super) const MAX_PENDING_REQUESTS: usize = 64;

pub(super) struct OpenDocument {
    pub(super) document: DocumentId,
    pub(super) revision: BufferRevision,
    uri: lt::Url,
    language_id: String,
    pending_text: Option<Rope>,
    version: Option<WireVersion>,
    allow_unversioned: bool,
}

impl SyncState {
    fn next_version(&mut self) -> Option<WireVersion> {
        let current = self.next_version.unwrap_or(WireVersion::new(0));
        let next = current.next()?;
        self.next_version = Some(next);
        Some(next)
    }

    pub(super) fn diagnostic_context(
        &self,
        path: &Path,
        version: Option<WireVersion>,
        server: ServerId,
        encoding: PositionEncoding,
    ) -> Option<DiagnosticContext> {
        let open = self.documents.get(path)?;
        let sent = open.version?;
        match version {
            Some(v) if v != sent => return None,
            None if !open.allow_unversioned => return None,
            _ => {}
        }
        Some(DiagnosticContext {
            server,
            document: open.document,
            revision: open.revision,
            encoding,
            version,
        })
    }
}

/// Whether `stamp` still names the live open incarnation of its path.
pub(super) fn owns(env: &WireEnv, stamp: &RequestStamp, path: &Path) -> bool {
    stamp.server == env.id
        && env
            .sync
            .lock()
            .documents
            .get(path)
            .is_some_and(|open| open.document == stamp.document && open.revision == stamp.revision)
}

/// The wire version this connection last sent for `path` — the only
/// version a server may legitimately name in a versioned edit.
pub(super) fn sent_version(env: &WireEnv, path: &Path) -> Option<WireVersion> {
    env.sync
        .lock()
        .documents
        .get(path)
        .and_then(|open| open.version)
}

impl Client {
    pub fn did_open(
        &self,
        document: DocumentId,
        revision: BufferRevision,
        path: &Path,
        language_id: &str,
        text: Rope,
    ) -> bool {
        let Some(uri) = self.uri(path) else {
            return false;
        };
        let mut state = self.sync.lock();
        if let Some(open) = state.documents.get(path) {
            return open.document == document && open.revision == revision;
        }
        let allow_unversioned = !state.seen_paths.contains(path);
        if state.ready {
            let Some(version) = state.next_version() else {
                return false;
            };
            // A full wire queue refuses BEFORE admission: no binding is
            // recorded, the caller's `false` surfaces the refusal.
            if self.queue.send(WireJob::Open {
                uri: uri.clone(),
                language_id: language_id.to_owned(),
                version,
                text,
            }) == Admission::Refused
            {
                return false;
            }
            state.seen_paths.insert(path.to_owned());
            state.documents.insert(
                path.to_owned(),
                OpenDocument {
                    document,
                    revision,
                    uri,
                    language_id: language_id.to_owned(),
                    pending_text: None,
                    version: Some(version),
                    allow_unversioned,
                },
            );
        } else {
            state.documents.insert(
                path.to_owned(),
                OpenDocument {
                    document,
                    revision,
                    uri,
                    language_id: language_id.to_owned(),
                    pending_text: Some(text),
                    version: None,
                    allow_unversioned,
                },
            );
        }
        true
    }

    pub fn did_change(
        &self,
        document: DocumentId,
        revision: BufferRevision,
        path: &Path,
        text: Rope,
    ) -> bool {
        let mut state = self.sync.lock();
        let Some(open) = state.documents.get(path) else {
            return false;
        };
        if open.document != document {
            return false;
        }
        if open.revision == revision {
            return true;
        }
        let uri = open.uri.clone();
        if state.ready {
            let Some(version) = state.next_version() else {
                return false;
            };
            // Refusal before admission (queue full): the recorded
            // revision stays stale, so the next sync retries with a
            // newer full snapshot — full-text sync makes the skipped
            // version safe. `Coalesced` means an unsent snapshot was
            // replaced in place; the wire state below is identical.
            if self.queue.send(WireJob::Change { uri, version, text }) == Admission::Refused {
                return false;
            }
            if let Some(open) = state.documents.get_mut(path) {
                open.version = Some(version);
                open.revision = revision;
            }
        } else {
            // Pre-init changes coalesce into the open snapshot that
            // `finish_initialize` will send — never a wire didChange
            // before `initialized`.
            if let Some(open) = state.documents.get_mut(path) {
                open.pending_text = Some(text);
                open.revision = revision;
            }
        }
        true
    }

    pub fn did_close(&self, document: DocumentId, path: &Path) {
        let mut state = self.sync.lock();
        if !state
            .documents
            .get(path)
            .is_some_and(|open| open.document == document)
        {
            return;
        }
        let Some(open) = state.documents.remove(path) else {
            return;
        };
        // Requests queued pre-init for this incarnation are cancelled
        // with an explicit terminal note (R9); dispatched ones keep
        // their stamp and are refused by the editor's freshness check.
        let encoding = self.caps.encoding();
        state.pending_requests.retain(|request| {
            if request.stamp.document == document {
                let context = ReplyContext {
                    stamp: request.stamp,
                    encoding,
                    kind: request.input.kind,
                };
                let _ = self.tx.send(LspEvent::Note {
                    context,
                    text: "cancelled — the document closed".into(),
                });
                false
            } else {
                true
            }
        });
        // A close is framed only for an incarnation whose open was
        // admitted to the wire; a pre-init close sends nothing.
        if open.version.is_some() {
            self.queue.send(WireJob::Close { uri: open.uri });
        }
    }

    /// Called only after capabilities and Initialized have been
    /// published. Queues every pending open (latest snapshot wins) and
    /// then every still-owned request, in that order.
    pub(super) fn finish_initialize(&self) -> Result<(), FlushError> {
        let mut state = self.sync.lock();
        // Sorted: full replay must see the same open sequence regardless
        // of hash seed (R11).
        let mut paths: Vec<PathBuf> = state.documents.keys().cloned().collect();
        paths.sort();
        for path in paths {
            let Some(version) = state.next_version() else {
                return Err(FlushError::VersionExhausted);
            };
            let Some(open) = state.documents.get_mut(&path) else {
                continue;
            };
            let Some(text) = open.pending_text.take() else {
                continue;
            };
            // Rope clones are cheap; keeping the snapshot until the
            // queue admits it lets a refusal leave state untouched.
            let admitted = self.queue.send(WireJob::Open {
                uri: open.uri.clone(),
                language_id: open.language_id.clone(),
                version,
                text: text.clone(),
            });
            if admitted == Admission::Refused {
                // The wire is not draining: keep the snapshot pending
                // and fail the flush rather than half-publish it.
                open.pending_text = Some(text);
                return Err(FlushError::QueueFull);
            }
            open.version = Some(version);
            state.seen_paths.insert(path);
        }
        // Lock stays held: dispatch happens in queue order, after every
        // open frame above it. Every admitted request ends in exactly
        // one terminal event (R9): dispatched, or an explicit cancel.
        for request in std::mem::take(&mut state.pending_requests) {
            let context = ReplyContext {
                stamp: request.stamp,
                encoding: self.caps.encoding(),
                kind: request.input.kind,
            };
            if !owns_state(&state, self.id, &request) {
                let _ = self.tx.send(LspEvent::Note {
                    context,
                    text: "cancelled — the document changed or closed during startup".into(),
                });
            } else if self.queue.send(WireJob::Request(request)) == Admission::Refused {
                let _ = self.tx.send(LspEvent::Note {
                    context,
                    text: "cancelled — the server's wire queue is full".into(),
                });
            }
        }
        state.ready = true;
        Ok(())
    }
}

fn owns_state(state: &SyncState, server: ServerId, request: &PendingRequest) -> bool {
    request.stamp.server == server
        && state
            .documents
            .get(&request.input.path)
            .is_some_and(|open| {
                open.document == request.stamp.document && open.revision == request.stamp.revision
            })
}

/// Why the post-initialize flush could not complete.
#[derive(Debug)]
pub(super) enum FlushError {
    VersionExhausted,
    /// The bounded wire queue refused a pending open: the connection
    /// is not draining, so initialization cannot honestly complete.
    QueueFull,
}
