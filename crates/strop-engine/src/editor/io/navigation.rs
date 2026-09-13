//! Textual resource entry and reload share one typed navigation owner.
use super::{Document, Editor, OpenIntent};
use strop_core::id::DocumentId;
use strop_core::worker::CancelReason;

impl Editor {
    pub(crate) fn cancel_lsp_open_from(&mut self, document: DocumentId) {
        if self
            .io
            .navigation
            .and_then(|request| self.io.open.get(&request))
            .is_some_and(|key| {
                matches!(&key.intent, OpenIntent::LspLocation { context, .. }
                if context.stamp.document == document)
            })
        {
            self.cancel_open(CancelReason::Superseded);
        }
    }

    pub(crate) fn request_user_open(&mut self, path: &str, intent: OpenIntent) {
        match Self::directory_operand_from(self.open_context(), path) {
            Ok(target) => {
                self.push_jump();
                self.request_target(target, intent);
            }
            Err(error) => self.message = error,
        }
    }
    pub(crate) fn refresh_current_resource(&mut self) -> bool {
        if self.refresh_directory() {
            return true;
        }
        let Some(target) = self.cur().file_target(&self.cwd) else {
            return false;
        };
        self.request_target(target, OpenIntent::Refresh);
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
    pub(super) fn finish_refresh(&mut self, document: DocumentId, mut replacement: Document) {
        self.cancel_pending();
        if self.docs.get(document).is_none() {
            return;
        }
        if matches!(replacement.source, super::super::DocumentSource::File) {
            replacement.buf.readonly = self.doc(document).buf.readonly;
        }
        let label = if replacement.directory_metadata_ref().is_some() {
            "directory"
        } else if replacement.remote_metadata().is_some() {
            "remote snapshot"
        } else {
            "file"
        };
        match self.publish_source_snapshot(document, replacement, false) {
            Ok(()) => {
                self.message = if self.filename_draft(document).is_some() {
                    "filename draft retained; source observations refreshed".into()
                } else {
                    format!("{label} refreshed")
                }
            }
            Err(error) => self.message = format!("{label} refresh failed: {error}"),
        }
    }
}
