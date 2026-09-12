//! Bounded input work: large grammar reads run on one CPU owner. Input after an
//! accepted command stays ordered; Ctrl-C revokes it. Preview and execution use
//! the same revision/cursor/command-stamped pure resolver result.
mod worker;
use super::{Editor, Key};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use strop_core::{
    id::{BufferRevision, DocumentId},
    worker::{Completion, FailureKind, Outcome, Ticket},
};
use strop_grammar::{ActionPlan, Command, Resolved};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolutionKey {
    document: DocumentId,
    revision: BufferRevision,
    pane: usize,
    command: Command,
    cursors: Vec<usize>,
    tab: usize,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolutionData {
    resolved: Vec<Option<Resolved>>,
    plan: Option<ActionPlan>,
    layouts: Vec<strop_core::layout::PreparedLineLayout>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ResolutionEvent {
    Completed(Box<Completion<ResolutionKey, ResolutionData>>),
    Stopped,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolutionPurpose {
    Motion,
    Execute,
    IncSearch,
    Preview,
    VisualObject,
    RepeatSearch(bool),
    DotRepeat,
}
impl ResolutionPurpose {
    fn blocks_input(self) -> bool {
        matches!(
            self,
            Self::Motion
                | Self::Execute
                | Self::VisualObject
                | Self::RepeatSearch(_)
                | Self::DotRepeat
        )
    }
}
struct Pending {
    ticket: Ticket<ResolutionKey>,
    purpose: ResolutionPurpose,
    cancel: Arc<AtomicBool>,
    macro_depth: usize,
}
pub enum DeferredInput {
    Key(Key),
    GeneratedKey {
        key: Key,
        depth: usize,
    },
    Paste(String),
    Leaf {
        handler: fn(&mut Editor, char),
        key: char,
        remaining: usize,
        depth: usize,
    },
    Macro {
        keys: Arc<[Key]>,
        position: usize,
        repetitions: usize,
        depth: usize,
    },
}
pub struct ResolutionState {
    pub tx: mpsc::Sender<ResolutionEvent>,
    pub rx: Option<mpsc::Receiver<ResolutionEvent>>,
    worker: Option<worker::ResolutionWorker>,
    pending: Option<Pending>,
    ready: Option<(ResolutionKey, Result<ResolutionData, String>)>,
    pub queue: VecDeque<DeferredInput>,
    pub staged: VecDeque<DeferredInput>,
    pub in_action: bool,
    resume_scheduled: bool,
    started: bool,
    stopping: bool,
    pub enabled: bool,
}
impl Default for ResolutionState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Some(rx),
            worker: None,
            pending: None,
            ready: None,
            queue: VecDeque::new(),
            staged: VecDeque::new(),
            in_action: false,
            resume_scheduled: false,
            started: false,
            stopping: false,
            enabled: false,
        }
    }
}
impl ResolutionState {
    pub fn blocked(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.purpose.blocks_input())
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some() || self.stopping || !self.queue.is_empty()
    }
    pub fn cancel(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.cancel.store(true, Ordering::Release);
        }
    }
    pub fn cancel_preview(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.purpose.blocks_input())
        {
            self.cancel();
        }
    }
    pub fn stop(&mut self) {
        self.cancel();
        self.queue.clear();
        self.staged.clear();
        self.stopping = self.started;
        self.worker = None;
    }
}

impl Editor {
    /// Small pure reads have a fixed source bound. Larger scans and counts never
    /// run on input/render. This is a work bound, not a second grammar dialect.
    pub(crate) fn resolution_is_large(&self, command: &Command) -> bool {
        const INLINE_BYTES: usize = 4096;
        self.resolution.enabled
            && (self.buf().len_bytes() > INLINE_BYTES || command.count.unwrap_or(1) > INLINE_BYTES)
    }
    fn resolution_matches(
        &self,
        key: &ResolutionKey,
        command: &Command,
        cursors: &[usize],
    ) -> bool {
        !self.docs.is_empty()
            && key.document == self.current()
            && key.revision == self.buf().revision()
            && key.pane == self.active_pane
            && key.tab == self.cur_indent().width.max(1)
            && &key.command == command
            && key.cursors == cursors
    }
    fn prepared_resolution(
        &self,
        command: &Command,
        cursors: &[usize],
    ) -> Option<Result<&ResolutionData, &str>> {
        let (key, ready) = self.resolution.ready.as_ref()?;
        self.resolution_matches(key, command, cursors)
            .then(|| ready.as_ref().map_err(String::as_str))
    }
    pub(crate) fn resolved_many(
        &self,
        command: &Command,
        cursors: &[usize],
    ) -> Result<Vec<Option<Resolved>>, String> {
        if let Some(ready) = self.prepared_resolution(command, cursors) {
            return ready
                .map(|data| data.resolved.clone())
                .map_err(str::to_owned);
        }
        strop_grammar::resolve_many(self.buf(), cursors, command).map_err(|error| error.to_string())
    }
    pub(crate) fn resolved_plan(
        &self,
        command: &Command,
        cursors: &[usize],
    ) -> Result<Option<ActionPlan>, String> {
        if let Some(ready) = self.prepared_resolution(command, cursors) {
            return ready.map(|data| data.plan.clone()).map_err(str::to_owned);
        }
        strop_grammar::plan(self.buf(), cursors, command).map_err(|error| error.to_string())
    }
    pub(crate) fn resolution_ready(&self, command: &Command, cursors: &[usize]) -> bool {
        self.prepared_resolution(command, cursors).is_some()
    }

    pub(crate) fn defer_resolution(
        &mut self,
        command: &Command,
        cursors: Vec<usize>,
        purpose: ResolutionPurpose,
    ) -> bool {
        if !self.resolution_is_large(command) || self.resolution_ready(command, &cursors) {
            return false;
        }
        if let Some(pending) = self.resolution.pending.as_ref() {
            if self.resolution_matches(&pending.ticket.key, command, &cursors) {
                if purpose.blocks_input() {
                    if let Some(pending) = self.resolution.pending.as_mut() {
                        pending.purpose = purpose;
                    }
                }
                return true;
            }
        }
        self.resolution.cancel();
        if !self.resolution.started {
            let started: Result<(), String> = match self.tape.call("grammar.start", &(), || {
                worker::ResolutionWorker::start(self.resolution.tx.clone())
                    .map(|worker| self.resolution.worker = Some(worker))
                    .map_err(|error| error.to_string())
            }) {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = started {
                self.message = format!("grammar: {error}");
                return true;
            }
            self.resolution.started = true;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return true;
            }
        };
        let key = ResolutionKey {
            document: self.current(),
            revision: self.buf().revision(),
            pane: self.active_pane,
            command: command.clone(),
            cursors,
            tab: self.cur_indent().width.max(1),
        };
        let ticket = Ticket { request, key };
        let cancel = Arc::new(AtomicBool::new(false));
        self.resolution.pending = Some(Pending {
            ticket: ticket.clone(),
            purpose,
            cancel: cancel.clone(),
            macro_depth: self.macro_depth,
        });
        match self.tape.request("grammar.resolve", &ticket) {
            Ok(false) => return true,
            Ok(true) => {}
            Err(error) => {
                self.handle_resolution(ResolutionEvent::Completed(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return true;
            }
        }
        let work = worker::Work {
            ticket,
            rope: self.buf().snapshot(),
            cancel,
        };
        let failed = match &self.resolution.worker {
            Some(worker) => worker.resolve(work).err(),
            None => Some(Box::new(work)),
        };
        if let Some(work) = failed {
            self.handle_resolution(ResolutionEvent::Completed(Box::new(Completion {
                ticket: work.ticket,
                outcome: Outcome::failed(FailureKind::Disconnected, "grammar worker stopped"),
            })));
        }
        true
    }

    pub(crate) fn handle_resolution(&mut self, event: ResolutionEvent) {
        let ResolutionEvent::Completed(completion) = event else {
            self.resolution.started = false;
            self.resolution.stopping = false;
            return;
        };
        if !self
            .resolution
            .pending
            .as_ref()
            .is_some_and(|pending| pending.ticket == completion.ticket)
        {
            return;
        }
        let Some(pending) = self.resolution.pending.take() else {
            return;
        };
        let key = completion.ticket.key;
        let current = !self.finishing
            && !self.docs.is_empty()
            && self.current() == key.document
            && self.active_pane == key.pane
            && self.buf().revision() == key.revision
            && (!pending.purpose.blocks_input() || self.all_cursors() == key.cursors);
        if !current {
            self.resolution.queue.clear();
            if !self.finishing {
                self.message = "resolution cancelled: document or cursor changed".into();
            }
            return;
        }
        let ready = match completion.outcome {
            Outcome::Success(data) => {
                if !self.docs.get_mut(key.document).is_some_and(|document| {
                    document
                        .buf
                        .install_line_layouts(key.revision, &data.layouts)
                }) {
                    self.message = "invalid layout publication".into();
                    self.resolution.queue.clear();
                    return;
                }
                Ok(data)
            }
            Outcome::Failed { failure, .. } => Err(failure.message),
            Outcome::Cancelled(_) => {
                self.resume_resolution_input();
                return;
            }
        };
        let succeeded = ready.is_ok();
        if let Err(error) = &ready {
            self.message = error.clone();
        }
        self.resolution.ready = Some((key.clone(), ready));
        if succeeded {
            let saved_depth = self.macro_depth;
            self.macro_depth = pending.macro_depth;
            self.run_input_action(|editor| match pending.purpose {
                ResolutionPurpose::Motion => editor.move_cursor(&key.command),
                ResolutionPurpose::Execute => editor.dispatch_grammar(&key.command),
                ResolutionPurpose::IncSearch => editor.incsearch_jump(),
                ResolutionPurpose::Preview => {}
                ResolutionPurpose::VisualObject => editor.select_visual_object(&key.command),
                ResolutionPurpose::RepeatSearch(invert) => editor.repeat_search(invert),
                ResolutionPurpose::DotRepeat => editor.dot_repeat_pub(),
            });
            self.macro_depth = saved_depth;
        }
        self.resume_resolution_input();
    }

    pub(crate) fn resume_resolution_input(&mut self) {
        self.resolution.resume_scheduled = false;
        // Exactly one queued input per delivery keeps replay independent of
        // host timing. The outer event loop owns the render/time budget.
        if !self.resolution.blocked() && !self.should_quit {
            if let Some(input) = self.resolution.queue.pop_front() {
                self.run_input_action(|editor| {
                    match input {
                        DeferredInput::Key(key) => editor.feed_inner(key),
                        DeferredInput::GeneratedKey { key, depth } => {
                            let saved = editor.macro_depth;
                            editor.macro_depth = depth;
                            editor.feed_inner(key);
                            editor.macro_depth = saved;
                        }
                        DeferredInput::Paste(text) => editor.paste_bracketed(&text),
                        DeferredInput::Leaf {
                            handler,
                            key,
                            remaining,
                            depth,
                        } => {
                            let saved = editor.macro_depth;
                            editor.macro_depth = depth;
                            editor.run_counted_leaf(handler, key, remaining);
                            editor.macro_depth = saved;
                        }
                        DeferredInput::Macro {
                            keys,
                            position,
                            repetitions,
                            depth,
                        } => {
                            editor.run_macro_step(keys, position, repetitions, depth);
                        }
                    }
                    editor.prepare_resolution_preview();
                });
            }
        }
        self.schedule_resolution_input();
    }
}

impl Editor {
    pub(crate) fn run_input_action(&mut self, action: impl FnOnce(&mut Editor)) {
        let nested = self.resolution.in_action;
        self.resolution.in_action = true;
        action(self);
        self.resolution.in_action = nested;
        if !nested {
            while let Some(input) = self.resolution.staged.pop_back() {
                self.resolution.queue.push_front(input);
            }
            self.schedule_resolution_input();
            // Collections write back at normal-mode action boundaries
            // (0044); the revision gate keeps this free for motions.
            if self.mode == super::Mode::Normal {
                self.commit_collection_sources();
            }
        }
    }
    pub(crate) fn run_counted_leaf(
        &mut self,
        handler: fn(&mut Editor, char),
        key: char,
        count: usize,
    ) {
        if !self.resolution.enabled {
            for _ in 0..count {
                handler(self, key);
            }
            return;
        }
        if count == 0 {
            return;
        }
        handler(self, key);
        if count > 1 {
            self.resolution.queue.push_front(DeferredInput::Leaf {
                handler,
                key,
                remaining: count - 1,
                depth: self.macro_depth,
            });
        }
        self.schedule_resolution_input();
    }
    fn schedule_resolution_input(&mut self) {
        if !self.resolution.blocked()
            && !self.resolution.queue.is_empty()
            && !self.resolution.resume_scheduled
        {
            self.resolution.resume_scheduled = true;
            if let Some(sender) = &self.app_tx {
                if sender.send(super::events::AppEvent::ResumeInput).is_err() {
                    self.message = "deferred input channel closed".into();
                }
            }
        }
    }
    pub(crate) fn queue_macro(&mut self, keys: Vec<Key>, repetitions: usize, depth: usize) {
        if keys.is_empty() || repetitions == 0 {
            return;
        }
        self.resolution.queue.push_front(DeferredInput::Macro {
            keys: keys.into(),
            position: 0,
            repetitions,
            depth,
        });
        self.schedule_resolution_input();
    }
    fn run_macro_step(
        &mut self,
        keys: Arc<[Key]>,
        position: usize,
        repetitions: usize,
        depth: usize,
    ) {
        let key = keys[position];
        let next = position + 1;
        if next < keys.len() {
            self.resolution.queue.push_front(DeferredInput::Macro {
                keys,
                position: next,
                repetitions,
                depth,
            });
        } else if repetitions > 1 {
            self.resolution.queue.push_front(DeferredInput::Macro {
                keys,
                position: 0,
                repetitions: repetitions - 1,
                depth,
            });
        }
        let saved = self.macro_depth;
        self.macro_depth = depth;
        self.feed_inner(key);
        self.macro_depth = saved;
    }
}
