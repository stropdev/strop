//! Read the declaration only for the pinned local workspace. At most one
//! physical read runs; changes during it revoke its result and owe one reread.
use std::sync::Arc;
use strop_core::worker::{self, FailureKind, Outcome, WorkerId};
use strop_picker::{DeclaredBoundary, InventoryRoot};

use super::super::{io::IoEvent, Editor};

#[derive(Default)]
pub(crate) struct Manifest {
    active: Option<(WorkerId, Option<InventoryRoot>)>,
    reread: bool,
    refresh_owed: bool,
    entries: Arc<[DeclaredBoundary]>,
}
impl Manifest {
    pub(super) fn pending(&self) -> bool {
        self.active.is_some()
    }
}

impl Editor {
    pub(crate) fn manifest_loading(&self) -> bool {
        self.notify.manifest.active.is_some()
    }

    pub(crate) fn workspace_declarations(&self) -> &Arc<[DeclaredBoundary]> {
        &self.notify.manifest.entries
    }

    pub(crate) fn reload_workspace_manifest(&mut self) {
        if self.finishing || !self.resolution.enabled {
            return;
        }
        let root = self.cwd.clone();
        let mut ticket = if self.tape.is_replay() {
            None
        } else {
            match self.source_worker() {
                Ok(source) => source.inventory_root(&root),
                Err(error) => {
                    self.message = error.message;
                    return;
                }
            }
        };
        if !self.notify.manifest.entries.is_empty() {
            self.notify.manifest.entries = Arc::default();
            if let (Some(source), Some(previous)) = (&self.picker_source, &ticket) {
                source.set_declarations(previous, Arc::default());
                ticket = source.inventory_root(&root);
            }
            self.retire_manifest_attachments();
            self.notify.manifest.refresh_owed = true;
            self.invalidate_manifest_picker();
        }
        if self.notify.manifest.active.is_some() {
            self.notify.manifest.reread = true;
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(id) => id,
            Err(e) => {
                self.message = e.message;
                return;
            }
        };
        self.notify.manifest.active = Some((request, ticket));
        let capture = super::super::trace::services::NativePath(root.clone());
        match self
            .tape
            .request("workspace.declarations", &(request, capture))
        {
            Ok(false) => return,
            Ok(true) => (),
            Err(e) => {
                self.notify.manifest.active = None;
                self.message = e.to_string();
                return;
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "workspace-declarations",
            move |outcome| {
                let _ = tx.send(IoEvent::WorkspaceDeclarations { request, outcome });
            },
            move |_| match strop_picker::read_workspace_declarations(&root) {
                Ok(entries) => Outcome::Success(entries),
                Err(e) => Outcome::failed(FailureKind::Io, e.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }

    pub(crate) fn workspace_manifest_done(
        &mut self,
        request: WorkerId,
        outcome: Outcome<Arc<[DeclaredBoundary]>>,
    ) {
        self.worker_handles.remove(&request);
        if self.notify.manifest.active.as_ref().map(|(id, _)| *id) != Some(request) {
            return;
        }
        let ticket = self
            .notify
            .manifest
            .active
            .take()
            .and_then(|(_, root)| root);
        if std::mem::take(&mut self.notify.manifest.reread) {
            self.reload_workspace_manifest();
            return;
        }
        let (entries, diagnostic) = match outcome {
            Outcome::Success(entries) => (entries, None),
            Outcome::Failed { failure, .. } => (Arc::default(), Some(failure.message)),
            Outcome::Cancelled(_) => return,
        };
        if !entries.is_empty() {
            let accepted = self
                .tape
                .observe_owned("workspace.bind_declarations", &request, || {
                    self.picker_source
                        .as_ref()
                        .zip(ticket.as_ref())
                        .is_some_and(|(source, ticket)| {
                            source.set_declarations(ticket, entries.clone())
                        })
                });
            match accepted {
                Ok(true) => {}
                Ok(false) => {
                    if !self.docs.is_empty() {
                        self.lsp_attach_document(self.current());
                    }
                    self.message =
                        "workspace declaration no longer applies; using discovery".into();
                    return;
                }
                Err(error) => {
                    self.message = error.to_string();
                    return;
                }
            }
        }
        let has_entries = !entries.is_empty();
        self.notify.manifest.entries = entries;
        if has_entries {
            self.retire_manifest_attachments();
        }
        if std::mem::take(&mut self.notify.manifest.refresh_owed) || has_entries {
            self.refresh_manifest_picker();
        }
        if !self.docs.is_empty() {
            self.lsp_attach_document(self.current());
        }
        if let Some(message) = diagnostic {
            self.message = message;
        }
    }
}
