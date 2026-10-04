//! UI-side analysis ownership. Frozen ropes go to one actor; frames consume only
//! immutable, revision-matched viewport results. Native parser state stays there.
pub(crate) mod layouts;
mod presentation;
mod search;
pub(crate) mod worker;
use super::matching::{MatchKey, PairMatch, PairState};
use super::{document::DocumentSource, Editor};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{Completion, Outcome, Ticket};

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AnalysisTarget {
    Document(DocumentId),
    Preview(strop_workspace::ResourceLocation),
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnalysisKey {
    pub target: AnalysisTarget,
    pub revision: BufferRevision,
    pub first: usize,
    pub last: usize,
    pub tab: usize,
    pub guides: bool,
    pub left: usize,
    pub right: usize,
    #[serde(with = "strop_core::path_serde::option")]
    pub syntax_path: Option<PathBuf>,
    pub search: Option<strop_grammar::CompiledQuery>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FrameAnalysis {
    pub spans: Vec<strop_syntax::Span>,
    pub guides: strop_syntax::GuideFrame,
    pub search: Option<Result<SearchSummary, String>>,
    pub layouts: Vec<strop_core::layout::PreparedLineLayout>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchSummary {
    pub count: usize,
    pub hits: Vec<strop_grammar::SearchMatch>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub enum AnalysisEvent {
    Completed(Box<Completion<AnalysisKey, FrameAnalysis>>),
    /// A matching-delimiter job landed (0051 §7 R09).
    Matched(Box<Completion<MatchKey, Option<PairMatch>>>),
    Stopped,
}
struct Pending {
    ticket: Ticket<AnalysisKey>,
    cancel: Arc<AtomicBool>,
}
struct Cached {
    key: AnalysisKey,
    value: Option<Arc<FrameAnalysis>>,
}
pub(crate) struct AnalysisState {
    pub tx: mpsc::Sender<AnalysisEvent>,
    pub rx: Option<mpsc::Receiver<AnalysisEvent>>,
    worker: Option<worker::Worker>,
    registered: HashSet<AnalysisTarget>,
    pending: HashMap<AnalysisTarget, Pending>,
    cache: HashMap<AnalysisTarget, Vec<Cached>>,
    /// Edit-mapped syntax presentations (0066): the interim display a
    /// pane keeps while its fresh exact analysis is in flight.
    presentations: HashMap<AnalysisTarget, presentation::Presentation>,
    /// Matching-delimiter jobs/results (0051 §7 R09) — same worker,
    /// same lifecycle.
    pub(crate) pair: PairState,
    started: bool,
    stopping: bool,
}
impl Default for AnalysisState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Some(rx),
            worker: None,
            registered: HashSet::new(),
            pending: HashMap::new(),
            cache: HashMap::new(),
            presentations: HashMap::new(),
            pair: PairState::default(),
            started: false,
            stopping: false,
        }
    }
}
impl AnalysisState {
    pub fn pending(&self) -> bool {
        !self.pending.is_empty() || !self.pair.pending_empty() || self.stopping
    }
    pub(crate) fn start(&mut self, tape: &strop_trace::replay::Tape) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        let result: Result<(), String> = tape
            .call("analysis.start", &(), || {
                worker::Worker::start(self.tx.clone())
                    .map(|worker| self.worker = Some(worker))
                    .map_err(|error| error.to_string())
            })
            .map_err(|error| error.to_string())?;
        result?;
        self.started = true;
        Ok(())
    }

    /// Match jobs share the worker and its edit journal (0051 §7 R09).
    pub(crate) fn register(&mut self, target: AnalysisTarget) {
        self.registered.insert(target);
    }

    pub(crate) fn worker(&self) -> Option<&worker::Worker> {
        self.worker.as_ref()
    }
    pub fn edits(&mut self, document: DocumentId, changes: &[strop_core::Change]) {
        let target = AnalysisTarget::Document(document);
        // a queued match scan for text that just changed is unwanted
        // work (0051 §7 R09): its delivery would fail the revision
        // recheck anyway — cancel the scan itself, not only the guard
        self.pair.cancel_target(&target);
        if !self.registered.contains(&target) {
            return;
        }
        if let Some(pending) = self.pending.get(&target) {
            pending.cancel.store(true, Ordering::Release);
        }
        // 0066: fold the same journal through the retained syntax
        // presentation, so interim frames carry mapped coordinates.
        if let Some(presentation) = self.presentations.get_mut(&target) {
            presentation.apply(changes);
        }
        if let Some(worker) = &self.worker {
            if !worker.edits(target, changes.to_vec()) {
                self.started = false;
            }
        }
    }

    pub fn forget(&mut self, target: AnalysisTarget) {
        if let Some(pending) = self.pending.remove(&target) {
            pending.cancel.store(true, Ordering::Release);
        }
        self.pair.forget(&target);
        self.cache.remove(&target);
        self.presentations.remove(&target);
        self.registered.remove(&target);
        if let Some(worker) = &self.worker {
            if !worker.forget(target) {
                self.started = false;
            }
        }
    }

    pub fn stop(&mut self) {
        for pending in self.pending.values() {
            pending.cancel.store(true, Ordering::Release);
        }
        self.pair.cancel_all();
        self.stopping = self.started;
        self.worker = None;
    }
}

impl Editor {
    /// The analysis key for one document window, or None when the
    /// document needs no analysis (no syntax, guides or search).
    fn analysis_key(
        &self,
        document: DocumentId,
        first: usize,
        last: usize,
        left: usize,
        width: usize,
    ) -> Option<AnalysisKey> {
        let doc = self.docs.get(document)?;
        let guides = self.config.indent_guides
            && matches!(
                doc.source,
                DocumentSource::File | DocumentSource::Scratch | DocumentSource::Remote(_)
            );
        let search = if document == self.current() {
            self.current_search_query().ok().flatten()
        } else {
            None
        };
        if !guides && doc.syntax_path().is_none() && search.is_none() {
            return None;
        }
        Some(AnalysisKey {
            target: AnalysisTarget::Document(document),
            revision: doc.buf.revision(),
            first,
            last,
            tab: doc.indent.width.max(1),
            guides,
            left,
            right: left.saturating_add(width),
            syntax_path: doc.syntax_path().map(std::path::Path::to_path_buf),
            search,
        })
    }

    /// Exact cache hit for a key, if a successful result is cached.
    fn exact_analysis(&self, key: &AnalysisKey) -> Option<Arc<FrameAnalysis>> {
        self.analysis
            .cache
            .get(&key.target)
            .and_then(|entries| entries.iter().find(|entry| entry.key == *key))
            .and_then(|entry| entry.value.clone())
    }

    /// Exact cache hit only. The declared pane bounds probe (0066): an
    /// edit-mapped interim frame is displayed, but it is not a cached
    /// exact result, so a window whose fresh analysis is in flight
    /// still reports `WindowBounds::Loading`.
    pub(crate) fn exact_analysis_cached(
        &self,
        document: DocumentId,
        first: usize,
        last: usize,
        left: usize,
        width: usize,
    ) -> Option<Arc<FrameAnalysis>> {
        if self.finishing {
            return None;
        }
        let key = self.analysis_key(document, first, last, left, width)?;
        self.exact_analysis(&key)
    }

    /// The 0066 interim frame: previously accepted syntax, mapped
    /// through the mutation journal to the coordinates it still
    /// vouches for. Never a ticket, cancel or request.
    fn interim_analysis(&self, key: &AnalysisKey) -> Option<Arc<FrameAnalysis>> {
        let document = match &key.target {
            AnalysisTarget::Document(document) => *document,
            AnalysisTarget::Preview(_) => return None,
        };
        let doc = self.docs.get(document)?;
        let presentation = self.analysis.presentations.get(&key.target)?;
        if presentation.mapped_revision() != doc.buf.revision() {
            return None; // an edit this presentation never saw
        }
        let len = doc.buf.len_bytes();
        let first_line = doc.buf.line_of(key.first.min(len));
        let last_line = doc.buf.line_of(key.last.min(len)).saturating_add(1);
        presentation.interim_frame(key, first_line, last_line, len)
    }

    /// Readonly presentation query for the paint path: exact cache hit,
    /// or the 0066 edit-mapped interim frame — never a ticket, cancel
    /// or request. Admission lives in [`Self::document_analysis`],
    /// called by preparation.
    pub fn document_analysis_cached(
        &self,
        document: DocumentId,
        first: usize,
        last: usize,
        left: usize,
        width: usize,
    ) -> Option<Arc<FrameAnalysis>> {
        if self.finishing {
            return None;
        }
        let key = self.analysis_key(document, first, last, left, width)?;
        if let Some(cached) = self.exact_analysis(&key) {
            return Some(cached);
        }
        self.interim_analysis(&key)
    }

    pub fn document_analysis(
        &mut self,
        document: DocumentId,
        first: usize,
        last: usize,
        left: usize,
        width: usize,
    ) -> Option<Arc<FrameAnalysis>> {
        if self.finishing {
            return None;
        }
        let key = self.analysis_key(document, first, last, left, width)?;
        if let Some(cached) = self.exact_analysis(&key) {
            return Some(cached);
        }
        // An edit changes the revision before the worker's fresh frame
        // lands. The 0066 interim frame keeps the accepted colors on
        // screen at journal-mapped coordinates instead of blanking.
        let interim = self.interim_analysis(&key);
        if let Some(pending) = self.analysis.pending.get(&key.target) {
            if pending.ticket.key.revision != key.revision
                || pending.ticket.key.search != key.search
            {
                pending.cancel.store(true, Ordering::Release);
            }
            return interim;
        }
        let rope = self.docs.get(document)?.buf.snapshot();
        self.request_analysis(key, rope);
        interim
    }

    /// Readonly paint query for preview windows: cache hit only.
    pub fn preview_analysis_cached(
        &self,
        path: &strop_workspace::ResourceLocation,
        first: usize,
        last: usize,
        width: usize,
    ) -> Option<Arc<FrameAnalysis>> {
        if self.finishing {
            return None;
        }
        self.previews.get(path)?;
        let key = AnalysisKey {
            target: AnalysisTarget::Preview(path.clone()),
            revision: BufferRevision::new(0),
            first,
            last,
            tab: self.tab_width_for_location(path).max(1),
            guides: false,
            left: 0,
            right: width,
            syntax_path: Some(path.path.clone()),
            search: None,
        };
        self.analysis
            .cache
            .get(&key.target)
            .and_then(|entries| entries.iter().find(|entry| entry.key == key))
            .and_then(|entry| entry.value.clone())
    }

    pub fn preview_analysis(
        &mut self,
        path: &strop_workspace::ResourceLocation,
        first: usize,
        last: usize,
        width: usize,
    ) -> Option<Arc<FrameAnalysis>> {
        if self.finishing {
            return None;
        }
        let entry = self.previews.get(path)?;
        let key = AnalysisKey {
            target: AnalysisTarget::Preview(path.clone()),
            revision: BufferRevision::new(0),
            first,
            last,
            tab: self.tab_width_for_location(path).max(1),
            guides: false,
            left: 0,
            right: width,
            syntax_path: Some(path.path.clone()),
            search: None,
        };
        if let Some(cached) = self
            .analysis
            .cache
            .get(&key.target)
            .and_then(|entries| entries.iter().find(|entry| entry.key == key))
        {
            return cached.value.clone();
        }
        if self.analysis.pending.contains_key(&key.target) {
            return None;
        }
        let rope = entry.rope.clone();
        self.request_analysis(key, rope);
        None
    }

    pub fn search_summary(
        &self,
        query: &strop_grammar::CompiledQuery,
    ) -> Option<&Result<SearchSummary, String>> {
        self.analysis
            .cache
            .get(&AnalysisTarget::Document(self.current()))?
            .iter()
            .rev()
            .find(|entry| {
                entry.key.revision == self.buf().revision()
                    && entry.key.search.as_ref() == Some(query)
            })?
            .value
            .as_ref()?
            .search
            .as_ref()
    }

    fn request_analysis(&mut self, key: AnalysisKey, rope: ropey::Rope) {
        if let Err(error) = self.analysis.start(&self.tape) {
            self.message = format!("analysis: {error}");
            self.analysis
                .cache
                .entry(key.target.clone())
                .or_default()
                .push(Cached { key, value: None });
            return;
        }
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
        self.admissions.analysis += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.analysis.registered.insert(key.target.clone());
        self.analysis.pending.insert(
            key.target.clone(),
            Pending {
                ticket: ticket.clone(),
                cancel: cancel.clone(),
            },
        );
        match self.tape.request("analysis.viewport", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_analysis(AnalysisEvent::Completed(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(
                        strop_core::worker::FailureKind::Protocol,
                        error.to_string(),
                    ),
                })));
                return;
            }
        }
        let work = worker::Work {
            ticket,
            rope,
            cancel,
        };
        let failed = match &self.analysis.worker {
            Some(worker) => worker.analyze(work).err(),
            None => Some(Box::new(work)),
        };
        if let Some(work) = failed {
            self.handle_analysis(AnalysisEvent::Completed(Box::new(Completion {
                ticket: work.ticket,
                outcome: Outcome::failed(
                    strop_core::worker::FailureKind::Disconnected,
                    "display analysis worker stopped",
                ),
            })));
        }
    }

    pub(crate) fn handle_analysis(&mut self, event: AnalysisEvent) {
        let completion = match event {
            AnalysisEvent::Stopped => {
                self.analysis.stopping = false;
                self.analysis.started = false;
                return;
            }
            AnalysisEvent::Matched(completion) => {
                self.handle_match(*completion);
                return;
            }
            AnalysisEvent::Completed(completion) => completion,
        };
        let key = completion.ticket.key;
        if !self
            .analysis
            .pending
            .get(&key.target)
            .is_some_and(|pending| pending.ticket.request == completion.ticket.request)
        {
            return;
        }
        self.analysis.pending.remove(&key.target);
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
        let value = match completion.outcome {
            Outcome::Success(frame) => {
                if let AnalysisTarget::Document(document) = key.target {
                    if let Some(document) = self.docs.get_mut(document) {
                        if !document
                            .buf
                            .install_line_layouts(key.revision, &frame.layouts)
                        {
                            self.message = "invalid layout publication".into();
                            return;
                        }
                    }
                }
                if let AnalysisTarget::Document(_) = key.target {
                    self.analysis
                        .presentations
                        .entry(key.target.clone())
                        .or_default()
                        .accept(&key, &frame);
                }
                Some(Arc::new(frame))
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!("analysis: {}", failure.message);
                if let AnalysisTarget::Document(_) = key.target {
                    if let Some(presentation) = self.analysis.presentations.get_mut(&key.target) {
                        presentation.fail(&key);
                    }
                }
                None
            }
            Outcome::Cancelled(_) => return,
        };
        let entries = self.analysis.cache.entry(key.target.clone()).or_default();
        // Keep the previous revision's frames for the newest-first exact
        // probes; the interim display itself lives in the presentation
        // (0066), which accept() just refreshed.
        let previous = key.revision.get().saturating_sub(1);
        entries.retain(|entry| entry.key.revision.get() >= previous && entry.key.tab == key.tab);
        const CACHED_WINDOWS: usize = 8;
        if entries.len() == CACHED_WINDOWS {
            entries.remove(0);
        }
        entries.push(Cached { key, value });
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Editor {
    /// Whether any analysis/match work is in flight (test/inspection probe).
    pub fn analysis_pending_probe(&self) -> bool {
        self.analysis.pending()
    }

    pub fn analysis_fixture(&mut self) -> Arc<FrameAnalysis> {
        self.analysis_fixture_width(80)
    }

    /// The fixture at an explicit text budget: the pane's reserved
    /// scrollbar column (0064 §1) narrows what paint requests, so a
    /// test seeding spans for an N-wide terminal seeds at N-1.
    pub fn analysis_fixture_width(&mut self, width: usize) -> Arc<FrameAnalysis> {
        // 0066: document_analysis serves the interim frame before the
        // exact result lands, so the fixture must await the exact cache
        // explicitly — a projected frame must never satisfy a consumer
        // asking for fresh analysis.
        let document = self.current();
        let len = self.buf().len_bytes();
        self.analysis_key(document, 0, len, 0, width)
            .expect("analysis fixture requires a syntax, guides or search surface");
        loop {
            let _interim = self.document_analysis(document, 0, len, 0, width);
            if let Some(frame) = self
                .analysis_key(document, 0, len, 0, width)
                .and_then(|key| self.exact_analysis(&key))
            {
                return frame;
            }
            let event = self
                .analysis
                .rx
                .as_ref()
                .expect("unforwarded analysis")
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("analysis completion");
            self.handle_analysis(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::worker::WorkerId;
    use strop_core::Buffer;
    use strop_syntax::Span;

    /// 0066 S1: after an edit, the readonly paint query serves the
    /// accepted colors at journal-mapped coordinates — not a blank
    /// pane — while the fresh exact analysis is still in flight. The
    /// worker completion is deliberately left unpumped.
    #[test]
    fn an_edit_serves_edit_mapped_spans_instead_of_blanking() {
        let mut e = Editor::new(Buffer::from_text("fn main() {}\nfn other() {}\n"));
        e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
        let doc = e.current();
        let warm = e.analysis_fixture().spans.clone();
        assert!(!warm.is_empty(), "the rust grammar analyzes this source");
        // one space inserted at byte zero: every warm span shifts by one
        e.feed_text("i <esc>");
        let served = e
            .document_analysis_cached(doc, 0, e.buf().len_bytes(), 0, 100)
            .expect("the interim frame serves the accepted colors");
        let expected: Vec<Span> = warm
            .iter()
            .map(|span| Span {
                start: span.start + 1,
                end: span.end + 1,
                ..*span
            })
            .collect();
        assert_eq!(served.spans, expected, "spans follow the edit");
        // preparation admits the fresh window; it is still in flight
        let _ = e.document_analysis(doc, 0, e.buf().len_bytes(), 0, 100);
        assert!(
            e.analysis_pending_probe(),
            "this is the interim frame, not the fresh result"
        );
        e.wait_analysis(); // drain the cancelled pre-edit request
        let exact = e.analysis_fixture_width(100);
        assert_eq!(exact.spans, expected, "the settled parse agrees");
    }

    /// 0066: a viewport that moved without an edit keeps its covered
    /// overlap colored while the new window's analysis is in flight —
    /// scrolling must not blank the pane either.
    #[test]
    fn a_scrolled_viewport_serves_the_covered_overlap() {
        let mut e = Editor::new(Buffer::from_text(
            "fn one() {}\nfn two() {}\nfn three() {}\n",
        ));
        e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
        let doc = e.current();
        let warm = e.analysis_fixture_width(80).spans.clone();
        let len = e.buf().len_bytes();
        let line1 = e.buf().line_end(0);
        let served = e
            .document_analysis_cached(doc, line1, len, 0, 100)
            .expect("the covered overlap stays colored");
        let expected: Vec<Span> = warm
            .iter()
            .filter(|span| span.start.max(line1) < span.end.min(len))
            .map(|span| Span {
                start: span.start.max(line1),
                end: span.end.min(len),
                ..*span
            })
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(served.spans, expected);
    }

    /// 0066 §3C: a real analysis failure invalidates the window's
    /// provisional coverage instead of leaving it looking settled.
    #[test]
    fn a_failed_window_loses_its_provisional_coverage() {
        let mut e = Editor::new(Buffer::from_text("fn main() {}\n"));
        e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
        let doc = e.current();
        e.analysis_fixture();
        e.feed_text("i <esc>");
        let key = e
            .analysis_key(doc, 0, e.buf().len_bytes(), 0, 100)
            .expect("syntax surface");
        let target = key.target.clone();
        e.analysis.pending.insert(
            target.clone(),
            Pending {
                ticket: Ticket {
                    request: WorkerId::new(1),
                    key: key.clone(),
                },
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        e.handle_analysis(AnalysisEvent::Completed(Box::new(Completion {
            ticket: Ticket {
                request: WorkerId::new(1),
                key,
            },
            outcome: Outcome::failed(strop_core::worker::FailureKind::Io, "boom".to_string()),
        })));
        assert!(
            e.document_analysis_cached(doc, 0, e.buf().len_bytes(), 0, 100)
                .is_none(),
            "failed coverage must not keep serving projected colors"
        );
    }
}
