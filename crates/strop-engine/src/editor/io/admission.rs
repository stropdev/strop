//! Open-request admission, freshness checks, and publication into the editor.
use super::{Editor, FileTarget, IoEvent, OpenIntent, OpenKey};
use std::path::PathBuf;
use strop_core::id::DocumentId;
use strop_core::worker::{self, Completion, FailureKind, Outcome, Ticket};

impl Editor {
    pub fn request_open(&mut self, path: PathBuf, intent: OpenIntent) {
        self.request_target(FileTarget::Local(path), intent);
    }

    pub fn request_target(&mut self, target: FileTarget, intent: OpenIntent) {
        self.remember_directory_view();
        if target
            .resource_location()
            .is_some_and(|location| self.filesystem.blocks(&location))
        {
            self.message = "filesystem operation pending or unconfirmed; verify before reopening this resource".into();
            return;
        }
        if matches!(intent, OpenIntent::Refresh) && self.directory().is_some() {
            if let Err(error) = self.start_directory_task(
                self.current(),
                super::super::directory::DirectoryTask::Reload,
                None,
            ) {
                self.message = error;
            }
            return;
        }
        if matches!(intent, OpenIntent::Refresh) && self.remote_write_blocks_refresh(self.current())
        {
            self.message =
                "save pending or unconfirmed; settle or verify before refreshing this snapshot"
                    .into();
            return;
        }
        let requires_file = intent.requires_file();
        let browse = matches!(
            intent,
            OpenIntent::Browse | OpenIntent::DirectoryParent { .. }
        ) || (matches!(intent, OpenIntent::Refresh) && self.directory().is_some());
        if !matches!(target, FileTarget::Remote(_))
            && matches!(intent, OpenIntent::RemoteDestination)
        {
            self.message = "a remote destination requires a remote target".into();
            return;
        }
        let path = match target {
            FileTarget::Local(path) => FileTarget::Local(self.cwd.join(path)),
            remote => remote,
        };
        self.cancel_open(worker::CancelReason::Superseded);
        let existing = self
            .docs
            .iter()
            .find_map(|(id, document)| document.matches_target(&path).then_some(id));
        if let Some(id) = existing.filter(|&id| {
            !matches!(intent, OpenIntent::Refresh)
                && (!matches!(
                    intent,
                    OpenIntent::Browse
                        | OpenIntent::DirectoryParent { .. }
                        | OpenIntent::RemoteDestination
                ) || self.doc(id).directory_metadata_ref().is_none())
        }) {
            if (requires_file && self.doc(id).directory_metadata_ref().is_some())
                || (browse && self.doc(id).directory_metadata_ref().is_none())
            {
                self.message = if browse {
                    "browse requires a directory"
                } else {
                    "this view requires a regular file"
                }
                .into();
                return;
            }
            self.finish_open(id, intent);
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let key = OpenKey {
            path: path.clone(),
            origin: self.current(),
            revision: self.buf().revision(),
            focus: self.focus_epoch,
            intent,
        };
        if !matches!(key.intent, OpenIntent::CollectionSource { .. }) {
            self.io.navigation = Some(request);
        }
        self.io.open.insert(request, key.clone());
        self.message = format!("loading {path}");
        let tx = self.io.tx.clone();
        let ticket = Ticket { request, key };
        match self.tape.request("io.open", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Open(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return;
            }
        }
        let work = super::open::OpenRead {
            local: self.filesystem.local().clone(),
            previous_directories: {
                let mut previous: Vec<_> = self
                    .docs
                    .iter()
                    .filter_map(|(_, doc)| doc.directory_metadata_ref().cloned())
                    .collect();
                for saved in self.directories.views.values() {
                    if !previous
                        .iter()
                        .any(|source| source.location == saved.directory.location)
                    {
                        previous.push(saved.directory.clone());
                    }
                }
                previous
            },
            target: path,
            browse,
            requires_file,
            client: self.remote_client(),
            reveal: match &ticket.key.intent {
                OpenIntent::DirectoryParent { child } => Some(child.clone()),
                _ => None,
            },
        };
        let handle = worker::spawn(
            "strop-open",
            move |outcome| {
                let _ = tx.send(IoEvent::Open(Box::new(Completion { ticket, outcome })));
            },
            move |cancel| work.run(&cancel),
        );
        self.worker_handles.insert(request, handle);
    }

    fn open_fresh(&self, key: &OpenKey) -> bool {
        if self.finishing {
            return false;
        }
        // Background work owns its operation generation, not the current focus.
        if let OpenIntent::CollectionSource { owner } = key.intent {
            return !self.docs.is_empty()
                && self
                    .collection_build
                    .as_ref()
                    .is_some_and(|build| build.owner == owner);
        }
        if let OpenIntent::LspLocation { context, .. } = &key.intent {
            if !self.lsp_context_fresh(context) {
                return false;
            }
        }
        !self.docs.is_empty()
            && self.current() == key.origin
            && self.focus_epoch == key.focus
            && self.buf().revision() == key.revision
    }

    fn finish_open(&mut self, document: DocumentId, intent: OpenIntent) {
        self.resolve_indent_for(document);
        match intent {
            OpenIntent::SearchHit(hit) => {
                let Some(range) =
                    super::super::picker::checked_hit_range(self.doc(document).buf.text(), &hit)
                else {
                    self.message = format!(
                        "{}: search hit changed; refresh Search before opening it",
                        self.doc(document).label(&self.cwd)
                    );
                    return;
                };
                self.jump_land(document, range.start.get());
                self.discover_git();
                self.lsp_attach_document(self.current());
            }
            OpenIntent::LspLocation { context, position } => {
                self.finish_lsp_jump(document, position, context)
            }
            OpenIntent::Split { vertical } => self.split_document(vertical, document),
            // Background opens never move focus; a pending collection
            // build counts down and assembles when its sources land.
            OpenIntent::CollectionSource { owner } => self.collection_source_ready(owner),
            intent => {
                self.switch_to(document);
                if self.doc(document).directory_metadata_ref().is_some()
                    && self.filename_draft(document).is_none()
                {
                    self.restore_directory_view(document);
                } else if self.doc(document).directory_metadata_ref().is_none() {
                    self.set_head(0);
                    self.view_mut().view_top = 0;
                }
                match intent {
                    OpenIntent::Switch { readonly: true } => self
                        .buf_mut()
                        .set_readonly(strop_core::ReadonlyReason::Command),
                    OpenIntent::DirectoryParent { child } => {
                        if let Some(line) = self
                            .directory()
                            .and_then(|directory| directory.line_for(&child))
                        {
                            self.set_head(self.buf().line_start(line));
                            self.place_jump_target();
                        }
                    }
                    OpenIntent::AtLine { line } => {
                        self.set_head(
                            self.buf()
                                .line_start(line.get().min(self.buf().last_content_line())),
                        );
                        self.run_motion("^");
                    }
                    OpenIntent::Grep { line, column } => {
                        let line = line.get().min(self.buf().last_content_line());
                        let offset = self
                            .buf()
                            .line_start(line)
                            .saturating_add(column.get())
                            .min(self.buf().line_end(line));
                        self.set_head(self.buf().clamp_boundary(offset));
                        // grep/symbol landings use the 0051 §7
                        // placement: center unless comfortably visible
                        self.place_jump_target();
                    }
                    _ => {}
                }
                self.remember_directory_view();
                self.remember_remote_destination();
                self.discover_git();
                self.lsp_attach_document(self.current());
                self.recovery_offer_for(document);
            }
        }
    }

    pub(super) fn open_done(&mut self, completion: Completion<OpenKey, super::Opened>) {
        let request = completion.ticket.request;
        if self.io.open.get(&request) != Some(&completion.ticket.key) {
            return;
        }
        let Some(key) = self.io.open.remove(&request) else {
            return;
        };
        self.worker_handles.remove(&request);
        if self.io.navigation == Some(request) {
            self.io.navigation = None;
        }
        if !self.open_fresh(&key) {
            return;
        }
        match completion.outcome {
            Outcome::Success(mut opened) => {
                if matches!(key.intent, OpenIntent::Refresh) {
                    self.finish_refresh(key.origin, opened.document);
                    return;
                }
                if !self.cur().matches_target(&opened.canonical) {
                    opened.document.set_return_point(self.jump_record());
                }
                let existing = self.docs.iter().find_map(|(id, document)| {
                    document.matches_target(&opened.canonical).then_some(id)
                });
                let id = if let Some(id) = existing {
                    if self.doc(id).directory_metadata_ref().is_some() {
                        if let Err(error) = self.publish_source_snapshot(id, opened.document) {
                            self.message = error.to_string();
                            return;
                        }
                    }
                    id
                } else {
                    if let OpenIntent::SearchHit(hit) = &key.intent {
                        if super::super::picker::checked_hit_range(opened.document.buf.text(), hit)
                            .is_none()
                        {
                            self.message = format!(
                                "{}: search hit changed; refresh Search before opening it",
                                key.path
                            );
                            return;
                        }
                    }
                    // Background loads never steal the view: the
                    // pristine scratch stays until a foreground
                    // open or the built collection replaces it.
                    let takes_focus = !matches!(key.intent, OpenIntent::CollectionSource { .. });
                    // A first remote open selects its exact directory (or
                    // file parent), never one mutable root per endpoint.
                    let root = opened
                        .document
                        .remote_metadata()
                        .map(|source| {
                            strop_workspace::ResourceLocation::remote(
                                source.file.endpoint().clone(),
                                source
                                    .file
                                    .path()
                                    .parent()
                                    .unwrap_or_else(|| std::path::Path::new("/"))
                                    .to_path_buf(),
                            )
                        })
                        .or_else(|| {
                            opened
                                .document
                                .directory_metadata_ref()
                                .map(|directory| directory.location.clone())
                        });
                    if let Some(root) = root.filter(|root| self.workspaces.resolve(root).is_none())
                    {
                        if self.select_root(root).is_err() {
                            if let OpenIntent::CollectionSource { owner } = key.intent {
                                self.collection_source_ready(owner);
                            }
                            self.message = format!("open {}: identity space exhausted", key.path);
                            return;
                        }
                    }
                    // Identity admission is the last fallible
                    // step (0056 AR13): no view/mru state moves early.
                    let admitted = self.docs.try_insert(opened.document);
                    let Ok(id) = admitted else {
                        if let OpenIntent::CollectionSource { owner } = key.intent {
                            self.collection_source_ready(owner);
                        }
                        self.message = format!("open {}: identity space exhausted", key.path);
                        return;
                    };
                    if takes_focus {
                        self.drop_stale_scratch(id);
                    }
                    self.generation += 1;
                    self.mru.push(id);
                    id
                };
                self.message.clear();
                let user_open = matches!(
                    key.intent,
                    OpenIntent::Switch { .. } | OpenIntent::Split { .. }
                );
                self.finish_open(id, key.intent);
                if self.current() == id
                    && matches!(&key.path, FileTarget::Local(_))
                    && user_open
                    && matches!(self.cur().source, super::super::DocumentSource::File)
                    && self.buf().disk_stamp().is_none()
                {
                    // Vim's `"name" [New]`: the path did not exist, so an
                    // empty buffer was opened and only `:w` creates it.
                    self.message = format!("\"{}\" [new file]", self.cur().label(&self.cwd));
                }
            }
            Outcome::Failed { failure, .. } => {
                if let OpenIntent::CollectionSource { owner } = key.intent {
                    self.collection_source_ready(owner);
                }
                if let Some(source) = self
                    .docs
                    .get_mut(key.origin)
                    .and_then(|doc| doc.directory_metadata_mut())
                {
                    source.stale = Some(failure.message.clone());
                }
                self.message = format!("open {}: {}", key.path, failure.message);
            }
            Outcome::Cancelled(_) => {}
        }
    }
}
