//! Textual remote entry and ordinary navigation cancellation share the I/O owner.
use super::{Document, Editor, OpenIntent};
use crate::files::FileTarget;
use strop_core::id::DocumentId;
use strop_core::worker::CancelReason;

impl Editor {
    pub(crate) fn request_user_open(&mut self, path: &str, intent: OpenIntent) {
        match FileTarget::parse(path.into()) {
            Ok(target) => self.request_target(target, intent),
            Err(error) => self.message = error.to_string(),
        }
    }
    pub(crate) fn refresh_remote(&mut self) -> bool {
        let Some(file) = self.remote_file().cloned() else {
            return false;
        };
        self.request_target(FileTarget::Remote(file.into()), OpenIntent::Refresh);
        true
    }
    /// Revoke before signalling; queued success can no longer own navigation.
    pub(crate) fn cancel_open(&mut self, reason: CancelReason) -> bool {
        let Some(request) = self.io.navigation.take() else {
            return false;
        };
        self.io.open.remove(&request);
        if let Some(handle) = self.worker_handles.remove(&request) {
            handle.cancel(reason);
        }
        true
    }
    pub(super) fn finish_remote_refresh(&mut self, document: DocumentId, replacement: Document) {
        self.cancel_pending();
        if self.docs.get(document).is_none() {
            return;
        }
        match self.publish_remote_snapshot(document, replacement, false) {
            Ok(()) => self.message = "remote snapshot refreshed".into(),
            Err(error) => self.message = format!("remote refresh failed: {error}"),
        }
    }
}
