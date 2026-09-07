//! Remote reads share ordinary open admission; refresh publishes a new document
//! incarnation without rebuilding a large rope on the event loop.
use super::{Document, Editor, OpenIntent};
use crate::editor::document::DocumentSource;
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

    pub(crate) fn remote_file(&self) -> Option<&strop_remote::RemoteFile> {
        match &self.cur().source {
            DocumentSource::Remote(file) => Some(file),
            _ => None,
        }
    }

    pub(crate) fn refresh_remote(&mut self) -> bool {
        let Some(file) = self.remote_file().cloned() else {
            return false;
        };
        self.request_target(FileTarget::Remote(file), OpenIntent::Refresh);
        true
    }

    /// Revocation occurs before signalling, so even an already queued success
    /// cannot publish. The callback only signals; the worker owns all reaping.
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

    pub(super) fn finish_remote_refresh(&mut self, old: DocumentId, document: Document) {
        self.cancel_pending();
        let Some(old_document) = self.docs.get(old) else {
            return;
        };
        let before = &old_document.buf;
        let after = &document.buf;
        let position = |offset: usize| {
            let line = before.line_of(offset.min(before.len_bytes()));
            let column = offset.saturating_sub(before.line_start(line));
            let line = line.min(after.last_content_line());
            after.clamp_boundary((after.line_start(line) + column).min(after.line_end(line)))
        };
        // Retain every split's independent view, marks and jumplist locations.
        for pane in &mut self.panes {
            if pane.doc == old {
                pane.sels.map_positions(position);
                pane.view_top = pane.view_top.min(after.last_content_line());
            }
        }
        for (owner, offset) in self.marks.values_mut() {
            if *owner == old {
                *offset = position(*offset);
            }
        }
        for (owner, offset) in self
            .jumplist_past
            .iter_mut()
            .chain(&mut self.jumplist_future)
        {
            if *owner == old {
                *offset = position(*offset);
            }
        }
        let new = self.docs.insert(document);
        for pane in &mut self.panes {
            if pane.doc == old {
                pane.doc = new;
            }
        }
        for id in &mut self.mru {
            if *id == old {
                *id = new;
            }
        }
        for (owner, _) in self.marks.values_mut() {
            if *owner == old {
                *owner = new;
            }
        }
        for (owner, _) in self
            .jumplist_past
            .iter_mut()
            .chain(&mut self.jumplist_future)
        {
            if *owner == old {
                *owner = new;
            }
        }
        self.docs.remove(old);
        self.trace_documents.remove(&old);
        self.generation += 1;
        self.focus_epoch += 1;
        self.clamp_cursor();
        self.message = "remote snapshot refreshed".into();
    }
}
