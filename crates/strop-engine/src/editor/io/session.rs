//! Session persistence glue: serialized captures, one in-flight
//! write, newest queued capture replacing an unwritten one.

use super::{Editor, IoEvent};
use strop_core::worker::{self, FailureKind, Outcome, WorkerId};

impl Editor {
    pub(crate) fn request_session_save(&mut self) {
        let Some(work) = crate::session::capture_save(self) else {
            return;
        };
        if self.io.session.is_some() {
            // Serialized writes; newest queued capture replaces an unwritten one.
            self.io.queued_session = Some(work);
        } else {
            self.start_session_save(work);
        }
    }

    fn start_session_save(&mut self, work: crate::session::SaveRequest) {
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.io.session = Some(request);
        match self.tape.request("io.session", &request) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Session {
                    request,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-session",
            move |outcome| {
                let _ = tx.send(IoEvent::Session { request, outcome });
            },
            move |_| match work.persist() {
                Ok(()) => Outcome::Success(()),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(super) fn handle_session(&mut self, request: WorkerId, outcome: Outcome<()>) {
        if self.io.session != Some(request) {
            return;
        }
        self.io.session = None;
        self.worker_handles.remove(&request);
        if let Outcome::Failed { failure, .. } = outcome {
            self.message = format!("session save failed: {}", failure.message);
            self.io.session_error = Some(self.message.clone());
        }
        if let Some(work) = self.io.queued_session.take() {
            self.start_session_save(work);
        }
    }
}
