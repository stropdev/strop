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
    /// A declaration was read before initial watch registration completed.
    /// Confirm it after registration before allowing the first member launch.
    awaiting_watch: bool,
    refresh_owed: bool,
    entries: Arc<[DeclaredBoundary]>,
}
impl Manifest {
    pub(super) fn pending(&self) -> bool {
        self.active.is_some() || self.awaiting_watch
    }

    pub(super) fn awaiting_registration(&self) -> bool {
        self.awaiting_watch
    }
}

impl Editor {
    pub(crate) fn manifest_loading(&self) -> bool {
        self.notify.manifest.pending()
    }

    pub(crate) fn workspace_declarations(&self) -> &Arc<[DeclaredBoundary]> {
        &self.notify.manifest.entries
    }

    pub(crate) fn reload_workspace_manifest(&mut self) {
        if self.finishing || !self.resolution.enabled {
            return;
        }
        self.notify.manifest.awaiting_watch = false;
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
            // A recursive subscription can settle after this read. Installing
            // now would start a server, then retire and restart it merely
            // because the startup watch obtained coverage. Ordinary directories
            // without declarations need not wait for the registration walk.
            match self
                .tape
                .observe_owned("workspace.registration_pending", &request, || {
                    self.notify.subscribing()
                }) {
                Ok(true) => {
                    self.notify.manifest.awaiting_watch = true;
                    return;
                }
                Ok(false) => {}
                Err(error) => {
                    self.message = error.to_string();
                    return;
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::notify::Record;
    use strop_core::Buffer;

    #[test]
    fn file_open_does_not_launch_a_declared_server_twice_during_subscription() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("child");
        std::fs::create_dir_all(&child).unwrap();
        let file = child.join("lib.rs");
        std::fs::write(&file, "fn symbol() {}\n").unwrap();
        std::fs::create_dir(dir.path().join(".strop")).unwrap();
        std::fs::write(
            dir.path().join(".strop/workspace.toml"),
            "version=1\n[[project]]\npath='child'\nlanguages=['rust']\n",
        )
        .unwrap();
        let entries = strop_picker::read_workspace_declarations(dir.path()).unwrap();
        let mut editor = Editor::new_in(Buffer::open(file).unwrap(), dir.path().to_owned());
        editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
            Err(std::io::Error::other("unexpected native access"))
        }));
        editor.resolution.enabled = true;
        editor.notify.subscribing = true;
        editor.reload_workspace_manifest();
        editor.lsp_start_services();
        let request = editor.notify.manifest.active.as_ref().unwrap().0;
        editor.workspace_manifest_done(request, Outcome::Success(entries.clone()));
        assert!(
            editor.lsp_state.attach.pending.is_empty(),
            "registration can still race this read"
        );
        editor
            .notify
            .queue
            .push_record(Record::Settled(Outcome::Success(
                super::super::SubscribedScope {
                    subscription: strop_fs::notify::Subscription {
                        id: 1,
                        generation: 1,
                    },
                    root: dir.path().to_owned(),
                    owned_trace: None,
                },
            )));
        editor.handle_notify();
        let confirmed = editor.notify.manifest.active.as_ref().unwrap().0;
        editor.workspace_manifest_done(confirmed, Outcome::Success(entries));
        assert_eq!(editor.workspace_declarations().len(), 1);
        assert_eq!(editor.lsp_state.attach.pending.len(), 1);
    }
}
