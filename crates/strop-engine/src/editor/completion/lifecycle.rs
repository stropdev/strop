use super::mailbox::CompletionMailbox;
use super::session::{InsertPrefix, RequestIntent, WorkerLifecycle};
use super::worker::{WorkRefusal, Worker};
use crate::editor::Editor;
use std::sync::Arc;
use strop_core::worker::{CancelReason, Failure, FailureKind};

impl Editor {
    pub(super) fn completion_worker(&mut self) -> Result<(), Failure> {
        match &self.completion.worker {
            WorkerLifecycle::Running { .. } => return Ok(()),
            WorkerLifecycle::Stopping { .. } => {
                return Err(Failure::new(
                    FailureKind::Unavailable,
                    "completion worker is retiring; the newest request remains pending",
                ))
            }
            WorkerLifecycle::Idle => {}
            WorkerLifecycle::Failed(failure) => return Err(failure.clone()),
        }
        let mailbox = Arc::new(CompletionMailbox::default());
        if let Some(sender) = &self.app_tx {
            mailbox.connect(sender.clone());
        }
        let mut native = None;
        let started = self
            .tape
            .call("completion.start", &(), || {
                Worker::start(mailbox.clone())
                    .map(|worker| native = Some(worker))
                    .map_err(|error| error.to_string())
            })
            .map_err(|error| Failure::new(FailureKind::Protocol, error.to_string()))?;
        if let Err(error) = started {
            let failure = Failure::new(FailureKind::ThreadStart, error);
            self.completion.worker = WorkerLifecycle::Failed(failure.clone());
            return Err(failure);
        }
        let weak = Arc::downgrade(&mailbox);
        let wake = Arc::new(move || {
            if let Some(mailbox) = weak.upgrade() {
                mailbox.capacity();
            }
        });
        self.completion.worker = WorkerLifecycle::Running {
            native,
            mailbox,
            wake,
        };
        Ok(())
    }

    pub(super) fn completion_cancel_request(
        &mut self,
        stamp: strop_lsp::RequestStamp,
        reason: CancelReason,
    ) {
        match self.tape.request("completion.cancel", &(stamp, reason)) {
            Ok(true) => {
                if let Some(client) = self.lsp_live_client(stamp.server) {
                    client.cancel_completion(stamp, reason);
                }
            }
            Ok(false) => {}
            Err(error) => self.message = format!("completion cancellation diverged: {error}"),
        }
    }

    pub(in crate::editor) fn completion_close(&mut self, reason: CancelReason) {
        // Speculative edits never outlive their session (0059 §6.1).
        self.completion_revert_preview();
        if let Some(mailbox) = self.completion.worker.mailbox() {
            mailbox.query(None);
        }
        let session = self.completion.session.take();
        if let Some(session) = &session {
            for stamp in [session.request, session.resolve.request()]
                .into_iter()
                .flatten()
            {
                self.completion_cancel_request(stamp, reason);
            }
        }
        self.completion.intent = None;
        self.completion.prefix = InsertPrefix::None;
        self.completion.prefix_growth = None;
        drop(session);
        self.completion.released();
    }

    pub(crate) fn stop_completion(&mut self) {
        self.completion_close(CancelReason::Shutdown);
        self.completion.source = None;
        let lifecycle = std::mem::replace(&mut self.completion.worker, WorkerLifecycle::Idle);
        self.completion.worker = match lifecycle {
            WorkerLifecycle::Running {
                native, mailbox, ..
            } => {
                match self.tape.request("completion.stop", &()) {
                    Ok(true) => {
                        if let Some(worker) = &native {
                            worker.stop();
                        }
                    }
                    Ok(false) => {}
                    Err(error) => self.message = format!("completion shutdown diverged: {error}"),
                }
                WorkerLifecycle::Stopping {
                    native,
                    mailbox,
                    stage: super::session::RetirementStage::Record,
                }
            }
            WorkerLifecycle::Failed(_) => WorkerLifecycle::Idle,
            other => other,
        };
    }

    /// Drivers include a retirement wake while this is true. A stopped record
    /// can precede the final native thread epilogue; no input path joins it.
    pub fn completion_retiring(&self) -> bool {
        matches!(self.completion.worker, WorkerLifecycle::Stopping { .. })
    }

    pub(super) fn completion_collect_retired(&mut self) {
        let WorkerLifecycle::Stopping {
            native,
            stage: super::session::RetirementStage::ThreadExit,
            ..
        } = &self.completion.worker
        else {
            return;
        };
        match self.tape.call("completion.retired", &(), || {
            native.as_ref().is_none_or(Worker::is_finished)
        }) {
            Ok(true) => self.completion.worker = WorkerLifecycle::Idle,
            Ok(false) => {}
            Err(error) => self.message = format!("completion retirement diverged: {error}"),
        }
    }

    pub(crate) fn completion_settings_changed(
        &mut self,
        before: crate::config::CompletionSettings,
    ) {
        if before == self.config.completion {
            return;
        }
        if matches!(self.completion.worker, WorkerLifecycle::Failed(_)) {
            self.completion.worker = WorkerLifecycle::Idle;
        }
        self.completion.prefix = InsertPrefix::None;
        if matches!(self.completion.worker, WorkerLifecycle::Idle)
            && self.completion.session.is_none()
        {
            // Startup configuration has no callbacks to revoke and consumes no
            // pre-seed worker identity in a forensic recording.
            self.completion.intent = None;
            return;
        }
        let manual = self
            .completion
            .session
            .as_ref()
            .map(|session| session.query.context.invocation)
            .filter(|invocation| invocation.manual());
        match self.worker_ids.allocate() {
            Ok(generation) => self.completion.settings_generation = generation.get(),
            Err(error) => {
                self.stop_completion();
                self.message = error.message;
                return;
            }
        }
        if !self.config.completion.enabled {
            self.stop_completion();
        } else {
            self.completion_close(CancelReason::Superseded);
            if let Some(invocation) = manual {
                self.completion.intent = Some(RequestIntent {
                    invocation,
                    direction: None,
                    trigger: strop_lsp::completion::CompletionTrigger::Invoked,
                    cached_language: None,
                    selection: None,
                });
            }
        }
    }

    pub(super) fn completion_offer_source(&mut self) -> Result<(), WorkRefusal> {
        let Some(session) = &self.completion.session else {
            return Err(WorkRefusal::WrongSource);
        };
        let Some(binding) = session.query.context.primary_binding() else {
            return Err(WorkRefusal::WrongSource);
        };
        if self
            .completion
            .source
            .as_ref()
            .is_none_or(|source| source.binding != *binding)
        {
            let generation = self.worker_ids.allocate().map_err(|error| {
                self.message = error.message;
                WorkRefusal::Closed
            })?;
            let revision = session
                .query
                .context
                .primary()
                .map(|target| target.revision)
                .ok_or(WorkRefusal::WrongSource)?;
            self.completion.source = Some(super::session::SourceLease {
                generation,
                binding: binding.clone(),
                revision,
                snapshot_waiting: true,
                language_attachment_requested: false,
                language_context: None,
            });
        }
        let Some(source) = &mut self.completion.source else {
            return Err(WorkRefusal::WrongSource);
        };
        let Some(document) = self.docs.get(source.binding.document) else {
            return Err(WorkRefusal::WrongSource);
        };
        let words = session.query.context.invocation.words();
        let args = (
            source.generation,
            source.binding.document,
            document.buf.revision(),
            document.buf.len_bytes(),
            words,
        );
        let admitted = self
            .tape
            .call("completion.source", &args, || {
                self.completion
                    .worker
                    .native()
                    .ok_or(WorkRefusal::Closed)?
                    .source(
                        source.generation,
                        source.binding.document,
                        document.buf.revision(),
                        document.buf.text(),
                        words,
                    )
            })
            .map_err(|error| {
                self.message = format!("completion source admission diverged: {error}");
                WorkRefusal::Closed
            })?;
        source.revision = document.buf.revision();
        source.snapshot_waiting = admitted == Err(WorkRefusal::Busy);
        admitted
    }
}
