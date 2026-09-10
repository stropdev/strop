//! UI-side analysis ownership. Frozen ropes go to one actor; frames consume only
//! immutable, revision-matched viewport results. Native parser state stays there.
pub(crate) mod layouts;
mod search;
mod worker;
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
    Preview(#[serde(with = "strop_core::path_serde")] PathBuf),
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
            started: false,
            stopping: false,
        }
    }
}
impl AnalysisState {
    pub fn pending(&self) -> bool {
        !self.pending.is_empty() || self.stopping
    }
    fn start(&mut self, tape: &strop_trace::replay::Tape) -> Result<(), String> {
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
    pub fn edits(&mut self, document: DocumentId, changes: &[strop_core::Change]) {
        let target = AnalysisTarget::Document(document);
        if !self.registered.contains(&target) {
            return;
        }
        if let Some(pending) = self.pending.get(&target) {
            pending.cancel.store(true, Ordering::Release);
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
        self.cache.remove(&target);
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
        self.stopping = self.started;
        self.worker = None;
    }
}

/// A stale frame served for an interim frame: spans clipped to the live
/// text length so a shrink can never hand an out-of-range range to the
/// renderer. Guides/search ride along unclipped — they are approximate
/// for one frame by design.
fn clip_stale_frame(frame: Arc<FrameAnalysis>, len: usize) -> Arc<FrameAnalysis> {
    let mut clipped = (*frame).clone();
    clipped.spans.retain(|span| span.start < len);
    for span in &mut clipped.spans {
        span.end = span.end.min(len);
    }
    Arc::new(clipped)
}
impl Editor {
    pub(crate) fn document_analysis(
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
        let doc = self.docs.get(document)?;
        let guides = self.config.indent_guides
            && matches!(
                doc.source,
                DocumentSource::File | DocumentSource::Scratch | DocumentSource::Remote(_)
            );
        let target = AnalysisTarget::Document(document);
        let search = if document == self.current() {
            self.current_search_query().ok().flatten()
        } else {
            None
        };
        if !guides && doc.syntax_path().is_none() && search.is_none() {
            return None;
        }
        let key = AnalysisKey {
            target,
            revision: doc.buf.revision(),
            first,
            last,
            tab: self.config.tab_size.max(1),
            guides,
            left,
            right: left.saturating_add(width),
            syntax_path: doc.syntax_path().map(std::path::Path::to_path_buf),
            search,
        };
        if let Some(cached) = self
            .analysis
            .cache
            .get(&key.target)
            .and_then(|entries| entries.iter().find(|entry| entry.key == key))
        {
            return cached.value.clone();
        }
        // An edit changes the revision before the worker's fresh frame
        // lands. Serve the newest older frame for the same window and
        // signature — highlights track one frame behind (spans clipped
        // to the live length) instead of blanking for a frame.
        let stale = self
            .analysis
            .cache
            .get(&key.target)
            .and_then(|entries| {
                entries
                    .iter()
                    .rev()
                    .find(|entry| {
                        entry.key.first == key.first
                            && entry.key.last == key.last
                            && entry.key.tab == key.tab
                            && entry.key.search == key.search
                            && entry.key.revision.get() < key.revision.get()
                            && entry.value.is_some()
                    })
                    .and_then(|entry| entry.value.clone())
            })
            .map(|frame| clip_stale_frame(frame, doc.buf.len_bytes()));
        if let Some(pending) = self.analysis.pending.get(&key.target) {
            if pending.ticket.key.revision != key.revision
                || pending.ticket.key.search != key.search
            {
                pending.cancel.store(true, Ordering::Release);
            }
            return stale;
        }
        let rope = doc.buf.snapshot();
        self.request_analysis(key, rope);
        stale
    }

    pub(crate) fn preview_analysis(
        &mut self,
        path: &std::path::Path,
        first: usize,
        last: usize,
        width: usize,
    ) -> Option<Arc<FrameAnalysis>> {
        if self.finishing {
            return None;
        }
        let entry = self.previews.get(path)?;
        let key = AnalysisKey {
            target: AnalysisTarget::Preview(path.to_path_buf()),
            revision: BufferRevision::new(0),
            first,
            last,
            tab: self.config.tab_size.max(1),
            guides: false,
            left: 0,
            right: width,
            syntax_path: Some(path.to_path_buf()),
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

    pub(crate) fn search_summary(
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
        let AnalysisEvent::Completed(completion) = event else {
            self.analysis.stopping = false;
            self.analysis.started = false;
            return;
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
                Some(Arc::new(frame))
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!("analysis: {}", failure.message);
                None
            }
            Outcome::Cancelled(_) => return,
        };
        let entries = self.analysis.cache.entry(key.target.clone()).or_default();
        // Keep the previous revision's frames too: a miss on the current
        // revision serves the newest older frame for the interim
        // (document_analysis' stale serve — the no-flicker path).
        let previous = key.revision.get().saturating_sub(1);
        entries.retain(|entry| entry.key.revision.get() >= previous && entry.key.tab == key.tab);
        const CACHED_WINDOWS: usize = 8;
        if entries.len() == CACHED_WINDOWS {
            entries.remove(0);
        }
        entries.push(Cached { key, value });
    }
}

#[cfg(test)]
impl Editor {
    pub(crate) fn analysis_fixture(&mut self) -> Arc<FrameAnalysis> {
        loop {
            if let Some(frame) =
                self.document_analysis(self.current(), 0, self.buf().len_bytes(), 0, 80)
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
    use strop_syntax::{Class, Emphasis, Span};

    /// The interim frame (field report: highlights blank for a frame
    /// after each edit): with a cached frame at revision 0, an edit to
    /// revision 1 must still serve spans — clipped to the live text.
    #[test]
    fn an_edit_serves_the_previous_frame_instead_of_blanking() {
        let mut e = Editor::new(Buffer::from_text("fn main() {}\n"));
        e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
        let doc = e.current();
        let target = AnalysisTarget::Document(doc);
        let revision = e.buf().revision();
        let key = AnalysisKey {
            target: target.clone(),
            revision,
            first: 0,
            last: 0,
            tab: 4,
            guides: true,
            left: 0,
            right: 100,
            syntax_path: Some(PathBuf::from("/workspace/a.rs")),
            search: None,
        };
        let frame = FrameAnalysis {
            spans: vec![Span {
                start: 0,
                end: 2,
                class: Class::Keyword,
                emphasis: Emphasis::default(),
            }],
            ..Default::default()
        };
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
            outcome: Outcome::Success(frame),
        })));
        assert!(e.document_analysis(doc, 0, 0, 0, 100).is_some());
        e.feed_text("x"); // revision moves; no fresh frame exists yet
        let served = e.document_analysis(doc, 0, 0, 0, 100);
        assert!(
            served.is_some(),
            "an interim frame serves the previous analysis"
        );
        // shrink the text past the span: clipping keeps it in range
        e.feed_text("0wD");
        let _ = e.document_analysis(doc, 0, 0, 0, 100);
    }
}
