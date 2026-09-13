//! Save admission, optional formatting and owned source writes.
use super::*;

impl Editor {
    pub(crate) fn local_write_pending(&self) -> bool {
        !self.io.saves.is_empty()
            || matches!(self.lsp_state.after_format.as_ref(),
                Some(crate::editor::lsp::state::AfterFormat::Save { document, .. })
                    if self.docs.get(*document).is_some_and(|doc| matches!(doc.source, crate::editor::document::DocumentSource::File)))
    }

    pub fn request_save(&mut self, target: Option<PathBuf>, force: bool, close: bool) {
        if self.directory().is_some() {
            self.request_save_document(self.current(), target, force, close);
            return;
        }
        // auto_format (helix parity): a plain `:w` formats through the
        // language server first; the save chains on the reply. A
        // formatter failure or refusal never holds the save hostage.
        if self.config.auto_format && target.is_none() && self.lsp_format_available() {
            let before = self.lsp_state.navigation;
            self.lsp_format();
            if let Some(request) = self
                .lsp_state
                .navigation
                .filter(|request| Some(*request) != before)
            {
                self.lsp_state.after_format = Some(crate::editor::lsp::state::AfterFormat::Save {
                    document: self.current(),
                    close,
                    force,
                    request,
                });
                return;
            }
        }
        self.request_save_document(self.current(), target, force, close);
    }

    pub(crate) fn request_save_document(
        &mut self,
        document: DocumentId,
        target: Option<PathBuf>,
        force: bool,
        close: bool,
    ) -> bool {
        let blocked = if let Some(target) = target.as_ref() {
            self.filesystem
                .blocks(&strop_workspace::ResourceLocation::local(
                    self.cwd.join(target),
                ))
        } else {
            self.filesystem_blocks_document(document)
        };
        if blocked {
            self.message =
                "filesystem operation pending or unconfirmed; verify before saving this binding"
                    .into();
            return false;
        }
        if self
            .docs
            .get(document)
            .is_some_and(|doc| doc.directory_metadata_ref().is_some())
        {
            if self.filename_draft(document).is_some() {
                if target.is_some() || close {
                    self.message = "use :w without a target to review filename changes; filesystem application is explicit".into();
                    return false;
                }
                return match self.prepare_filename_draft(document) {
                    Ok(()) => true,
                    Err(error) => {
                        self.message = error;
                        false
                    }
                };
            }
            self.message = "Directory buffers have no file write binding; use :fs edit".into();
            return false;
        }
        if self.docs.get(document).is_some_and(|doc| {
            matches!(
                doc.source,
                crate::editor::document::DocumentSource::Remote(_)
            )
        }) {
            return self.request_remote_save(document, target, force, close);
        }
        if target
            .as_ref()
            .and_then(|path| path.to_str())
            .is_some_and(|path| path.starts_with("ssh://"))
        {
            self.message = "remote save-as is unsupported; no local fallback".into();
            return false;
        }
        if self
            .filesystem
            .blocks_namespace(&strop_workspace::Filesystem::Local)
        {
            self.message = "local filesystem mutation is pending or unconfirmed; settle or verify its receipt before saving".into();
            return false;
        }
        if self.io.saves.contains_key(&document) {
            self.message = "write already in progress".into();
            return false;
        }
        let Some(buffer) = self.docs.get(document).map(|doc| &doc.buf) else {
            self.message = "write refused: source buffer closed".into();
            return false;
        };
        let revision = buffer.revision();
        let target = target.map(|path| self.cwd.join(path));
        let work = match buffer.prepare_save(target.clone(), force) {
            Ok(work) => work,
            Err(error) => {
                self.message = format!("write failed: {error}");
                return false;
            }
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return false;
            }
        };
        let ticket = Ticket {
            request,
            key: SaveKey {
                document,
                revision,
                focus: self.focus_epoch,
                close,
                target,
                force,
            },
        };
        self.io.saves.insert(document, ticket.clone());
        self.message = "saving".into();
        match self.tape.request("io.save", &ticket) {
            Ok(false) => return true,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Save(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return false;
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-save",
            move |outcome| {
                let _ = tx.send(IoEvent::Save(Box::new(Completion { ticket, outcome })));
            },
            move |_| match work.execute() {
                Ok(receipt) => Outcome::Success(receipt),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
        true
    }

    /// Preserve the formatting stage alongside the actual persistence outcome.
    pub(crate) fn finish_save_feedback(&mut self, document: DocumentId) {
        if let Some(warning) = self.io.format_warnings.remove(&document) {
            self.message
                .push_str(&format!(" — format warning: {warning}"));
        }
        self.finish_change_save(document);
    }
}
