//! UI-side analysis ownership. Frozen ropes go to one actor; frames consume only
//! immutable, revision-matched viewport results. Native parser state stays there.
pub(crate) mod layouts;
mod presentation;
mod search;
pub(crate) mod worker;
use super::matching::{MatchKey, PairMatch, PairState};
use super::{document::DocumentSource, Editor};
use std::collections::{HashMap, HashSet};
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
    /// Syntax identity: the detected language, not the path — a rename
    /// within a language keeps the parser, a re-detection on write
    /// changes it (0111 decision 7).
    pub language: Option<strop_syntax::languages::LanguageId>,
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
    /// One-based global match numbers, aligned with `hits`.
    #[serde(default)]
    pub ordinals: Vec<usize>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub enum AnalysisEvent {
    Completed(Box<Completion<AnalysisKey, FrameAnalysis>>),
    /// A matching-delimiter job landed (0051 §7 R09).
    Matched(Box<Completion<MatchKey, Option<PairMatch>>>),
    /// A symbol-outline job landed (0099 §4.1).
    Outlined(
        Box<Completion<crate::editor::symbols::outline::OutlineKey, Vec<strop_lsp::ProtoSymbol>>>,
    ),
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
    /// Document symbols and their builds (0099 §4.1): the edit, close
    /// and stop lifecycle below covers them.
    pub(crate) symbols: crate::editor::symbols::SymbolState,
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
            symbols: Default::default(),
            started: false,
            stopping: false,
        }
    }
}
impl AnalysisState {
    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
            || !self.pair.pending_empty()
            || self.symbols.pending()
            || self.stopping
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

    /// Record a decided window. Only the key's own revision and tab are
    /// kept: every probe compares the full key against the live revision
    /// and revisions are never reused, so older frames could never be
    /// served again. The interim display lives in the presentation (0066).
    fn remember(&mut self, key: AnalysisKey, value: Option<Arc<FrameAnalysis>>) {
        const CACHED_WINDOWS: usize = 8;
        let entries = self.cache.entry(key.target.clone()).or_default();
        entries.retain(|entry| entry.key.revision == key.revision && entry.key.tab == key.tab);
        if entries.len() == CACHED_WINDOWS {
            entries.remove(0);
        }
        entries.push(Cached { key, value });
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
        self.symbols.edited(document, changes);
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
        if let AnalysisTarget::Document(document) = &target {
            self.symbols.closed(*document);
        }
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
        self.symbols.stop();
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
        let language = doc.syntax_language();
        if !guides && language.is_none() && search.is_none() {
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
            language,
            search,
        })
    }

    /// The decided entry for a key: a frame, or a remembered failure
    /// (`value: None`). A failure is decided too — it is not re-requested
    /// until the key changes (0111 decision 1).
    fn exact_entry(&self, key: &AnalysisKey) -> Option<&Cached> {
        self.analysis
            .cache
            .get(&key.target)
            .and_then(|entries| entries.iter().find(|entry| entry.key == *key))
    }

    /// Exact cache hit for a key, if a successful result is cached.
    fn exact_analysis(&self, key: &AnalysisKey) -> Option<Arc<FrameAnalysis>> {
        self.exact_entry(key).and_then(|entry| entry.value.clone())
    }

    /// The declared pane bounds probe (0066): whether the window needs
    /// analysis that is not decided yet. An edit-mapped interim frame is
    /// displayed, but it is not a decided result, so a window whose fresh
    /// analysis is in flight still reports `WindowBounds::Loading`; a
    /// window whose analysis failed is decided and does not.
    pub(crate) fn window_analysis_pending_for(
        &self,
        document: DocumentId,
        first: usize,
        last: usize,
        left: usize,
        width: usize,
    ) -> bool {
        let Some(key) = self.analysis_key(document, first, last, left, width) else {
            return false;
        };
        self.finishing || self.exact_entry(&key).is_none()
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
        if let Some(entry) = self.exact_entry(&key) {
            return entry.value.clone().or_else(|| self.interim_analysis(&key));
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
        let entry = self.previews.get(path)?;
        let key = self.preview_key(path, entry, first, last, width);
        self.exact_analysis(&key)
    }

    fn preview_key(
        &self,
        path: &strop_workspace::ResourceLocation,
        entry: &crate::editor::picker::PreviewEntry,
        first: usize,
        last: usize,
        width: usize,
    ) -> AnalysisKey {
        AnalysisKey {
            target: AnalysisTarget::Preview(path.clone()),
            revision: BufferRevision::new(0),
            first,
            last,
            tab: self.tab_width_for_location(path).max(1),
            guides: false,
            left: 0,
            right: width,
            language: entry.language,
            search: None,
        }
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
        let key = self.preview_key(path, entry, first, last, width);
        if let Some(cached) = self.exact_entry(&key) {
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

    /// One-based current match and total, once the asynchronous search
    /// summary covering the caret has landed. Navigation never scans here.
    pub fn search_position(&self) -> Option<(usize, usize)> {
        let query = self.current_search_query().ok().flatten()?;
        let summary = self.search_summary(&query)?.as_ref().ok()?;
        let head = self.head();
        let line_end = self.buf().line_end(self.buf().line_of(head));
        let before_line_end = head < line_end && self.buf().ceil_boundary(head + 1) == line_end;
        summary
            .hits
            .iter()
            .zip(&summary.ordinals)
            .find(|(hit, _)| {
                (hit.start.get() <= head && head < hit.end.get())
                    || (hit.is_empty() && hit.start.get() == head)
                    || (before_line_end && hit.start.get() == line_end)
            })
            .map(|(_, ordinal)| (*ordinal, summary.count))
    }

    fn request_analysis(&mut self, key: AnalysisKey, rope: ropey::Rope) {
        if let Err(error) = self.analysis.start(&self.tape) {
            self.message = format!("analysis: {error}");
            self.analysis.remember(key, None);
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
            AnalysisEvent::Outlined(completion) => {
                crate::editor::symbols::outline_done(self, *completion);
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
        self.analysis.remember(key, value);
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
mod tests;
