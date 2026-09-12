//! Passive delimiter highlighting through the same pure matcher as `%`.
//! Jobs are cancellable and revision/caret-owned. Collection probes and
//! endpoints use their real source; no renderer-specific lexical semantics.

use super::analysis::{worker, AnalysisTarget};
use super::Editor;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{Completion, FailureKind, Outcome, Ticket};
use strop_core::Buffer;
use strop_grammar::{delimiter_pair, matching_delimiter_at, MatchCancelled};

/// The cheap UI-side gate: the caret (or, in Insert, the just-typed
/// byte) must sit on a delimiter byte before any job is worth owning.
/// The lexical check is the worker's; this only keeps non-delimiter
/// carets from allocating work — it never scans.
fn near_delimiter(buf: &Buffer, caret: usize, insert: bool) -> bool {
    let on = buf
        .byte_at(caret)
        .is_some_and(|byte| delimiter_pair(byte).is_some());
    let typed = insert
        && caret
            .checked_sub(1)
            .and_then(|before| buf.byte_at(before))
            .is_some_and(|byte| delimiter_pair(byte).is_some());
    on || typed
}

/// A match job's identity: the source document at one revision, the
/// probe caret in SOURCE bytes and the Insert-mode just-typed probe.
/// The overlay paints exact-key cache hits only.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MatchKey {
    pub target: AnalysisTarget,
    pub revision: BufferRevision,
    pub caret: usize,
    pub insert: bool,
}

/// Both delimiter byte offsets of a pair, in the target source document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PairMatch {
    pub first: usize,
    pub second: usize,
}

struct PendingMatch {
    ticket: Ticket<MatchKey>,
    cancel: Arc<AtomicBool>,
}

/// Per-source match jobs and their exact-key results. Owned by
/// `AnalysisState` so the edit/forget/stop lifecycle covers it.
#[derive(Default)]
pub(crate) struct PairState {
    pending: HashMap<AnalysisTarget, PendingMatch>,
    cache: HashMap<AnalysisTarget, Vec<(MatchKey, Option<PairMatch>)>>,
}

impl PairState {
    pub(crate) fn pending_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The owned job already computes this frame's key: a cache miss
    /// while the worker is busy must not resubmit per frame.
    fn pending_covers(&self, key: &MatchKey) -> bool {
        self.pending
            .get(&key.target)
            .is_some_and(|pending| pending.ticket.key == *key)
    }

    /// A queued scan for a buffer that just changed is unwanted work;
    /// its delivery would fail the revision recheck anyway.
    pub(crate) fn cancel_target(&mut self, target: &AnalysisTarget) {
        if let Some(pending) = self.pending.remove(target) {
            pending.cancel.store(true, Ordering::Release);
        }
    }

    pub(crate) fn cancel_all(&mut self) {
        for pending in self.pending.drain().map(|(_, pending)| pending) {
            pending.cancel.store(true, Ordering::Release);
        }
    }

    pub(crate) fn forget(&mut self, target: &AnalysisTarget) {
        self.cancel_target(target);
        self.cache.remove(target);
    }

    fn lookup(&self, key: &MatchKey) -> Option<Option<PairMatch>> {
        self.cache
            .get(&key.target)?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, value)| *value)
    }

    fn store(&mut self, key: MatchKey, value: Option<PairMatch>) {
        let entries = self.cache.entry(key.target.clone()).or_default();
        entries.retain(|(k, _)| *k != key);
        // a small ring: walking the caret across a line's delimiters
        // revisits recent keys; old revisions age out unused
        const CACHED_MATCHES: usize = 8;
        while entries.len() >= CACHED_MATCHES {
            entries.remove(0);
        }
        entries.push((key, value));
    }
}

/// Unlike `%`, the passive overlay never searches ahead from a plain byte.
pub(crate) fn match_delimiters(
    buf: &Buffer,
    caret: usize,
    insert: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Option<(usize, usize)>, MatchCancelled> {
    if cancelled() {
        return Err(MatchCancelled);
    }
    let probe = if buf.byte_at(caret).and_then(delimiter_pair).is_some() {
        Some(caret)
    } else {
        caret
            .checked_sub(1)
            .filter(|_| insert)
            .filter(|&at| buf.byte_at(at).and_then(delimiter_pair).is_some())
    };
    let Some(probe) = probe else { return Ok(None) };
    matching_delimiter_at(buf, probe, cancelled)
        .map(|mate| mate.map(|mate| (probe.min(mate), probe.max(mate))))
}

impl Editor {
    /// The matching-delimiter overlay for the active pane (0051 §7
    /// R09): both pair endpoints in VIEW bytes of `doc`, each only when
    /// it maps into the viewed document — for a collection, through an
    /// excerpt of the SAME source; never across files, gaps or chrome.
    /// Passive: a miss owns a cancellable worker job and this frame
    /// paints nothing; nothing here scrolls, blocks or interprets text.
    pub fn pair_highlight(
        &mut self,
        doc: DocumentId,
        caret: usize,
        insert: bool,
    ) -> [Option<usize>; 2] {
        const NONE: [Option<usize>; 2] = [None, None];
        if self.finishing {
            return NONE;
        }
        // Pairing happens in the underlying source (0051 §7): a
        // collection body row projects the caret into its source.
        let Some((source, source_caret)) = self.source_position(doc, caret) else {
            return NONE;
        };
        let near = match self.docs.get(source) {
            Some(document) => near_delimiter(&document.buf, source_caret, insert),
            None => return NONE,
        };
        if !near {
            // the caret moved off delimiters: a queued scan is unwanted
            self.analysis
                .pair
                .cancel_target(&AnalysisTarget::Document(source));
            return NONE;
        }
        let Some(document) = self.docs.get(source) else {
            return NONE;
        };
        let key = MatchKey {
            target: AnalysisTarget::Document(source),
            revision: document.buf.revision(),
            caret: source_caret,
            insert,
        };
        let rope = document.buf.snapshot();
        match self.analysis.pair.lookup(&key) {
            Some(Some(pair)) => [
                self.view_byte_for_source(doc, source, pair.first),
                self.view_byte_for_source(doc, source, pair.second),
            ],
            Some(None) => NONE,
            None => {
                self.request_match(key, rope);
                NONE
            }
        }
    }

    /// A source byte's view byte in `doc`: identity for ordinary
    /// buffers; for a collection, the excerpt of THAT SAME source
    /// covering the byte's line (0051 §7 — never another file's card,
    /// never a gap, never a generated header). An unexcerpted partner
    /// maps nowhere and simply paints nothing.
    fn view_byte_for_source(
        &self,
        doc: DocumentId,
        source: DocumentId,
        byte: usize,
    ) -> Option<usize> {
        if !self.collections.contains_key(&doc) {
            return (source == doc).then_some(byte);
        }
        let collection = self.collections.get(&doc)?;
        let source_buf = &self.docs.get(source)?.buf;
        let view_buf = &self.docs.get(doc)?.buf;
        let target_line = source_buf.line_of(byte.min(source_buf.len_bytes()));
        for excerpt in &collection.excerpts {
            if excerpt.source != source {
                continue;
            }
            let first_line = source_buf.line_of(excerpt.start);
            if target_line < first_line || target_line >= first_line + excerpt.view_lines {
                continue;
            }
            let view_line = excerpt.view_line + 1 + (target_line - first_line);
            if view_line > view_buf.last_content_line() {
                return None;
            }
            let col = byte.saturating_sub(source_buf.line_start(target_line));
            let start = view_buf.line_start(view_line);
            return Some((start + col).min(view_buf.line_end(view_line)));
        }
        None
    }

    /// Own one cancellable match job (0051 §7): a superseded scan is
    /// cancelled in place — never joined, never awaited on input.
    fn request_match(&mut self, key: MatchKey, rope: ropey::Rope) {
        if self.analysis.pair.pending_covers(&key) {
            return; // the owned job already computes this key
        }
        if let Err(error) = self.analysis.start(&self.tape) {
            self.message = format!("analysis: {error}");
            return;
        }
        self.analysis.pair.cancel_target(&key.target);
        let request = match self.worker_ids.allocate() {
            Ok(id) => id,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: key.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.analysis.register(key.target.clone());
        self.analysis.pair.pending.insert(
            key.target.clone(),
            PendingMatch {
                ticket: ticket.clone(),
                cancel: cancel.clone(),
            },
        );
        match self.tape.request("analysis.match", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_match(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let work = worker::MatchWork {
            ticket,
            rope,
            cancel,
        };
        let failed = match self.analysis.worker() {
            Some(worker) => worker.match_work(work).err(),
            None => Some(Box::new(work)),
        };
        if let Some(work) = failed {
            self.handle_match(Completion {
                ticket: work.ticket,
                outcome: Outcome::failed(
                    FailureKind::Disconnected,
                    "display analysis worker stopped",
                ),
            });
        }
    }

    /// Land a match completion (0051 §7): only the live request counts;
    /// the source revision is rechecked before caching, so a result
    /// computed against edited text is dropped, and the exact-key
    /// lookup rechecks caret/mode/view on every frame. Unknown and
    /// unmatched states cache None — they must not resubmit per frame
    /// and must never paint a stale previous pair.
    pub(crate) fn handle_match(&mut self, completion: Completion<MatchKey, Option<PairMatch>>) {
        let key = completion.ticket.key;
        if !self
            .analysis
            .pair
            .pending
            .get(&key.target)
            .is_some_and(|pending| pending.ticket.request == completion.ticket.request)
        {
            return;
        }
        self.analysis.pair.pending.remove(&key.target);
        let current = match &key.target {
            AnalysisTarget::Document(document) => self
                .docs
                .get(*document)
                .is_some_and(|document| document.buf.revision() == key.revision),
            AnalysisTarget::Preview(path) => self.previews.contains_key(path),
        };
        if !current {
            return;
        }
        match completion.outcome {
            Outcome::Success(value) => self.analysis.pair.store(key, value),
            Outcome::Failed { .. } => self.analysis.pair.store(key, None),
            Outcome::Cancelled(_) => {}
        }
    }
}

#[cfg(test)]
mod tests;
