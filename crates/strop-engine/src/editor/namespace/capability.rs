//! Admission and per-document authority decisions shared by menus and execution.

use super::{CapabilityDecision, NamespaceCapabilities, WorkspaceCapability};
use crate::editor::Editor;
use strop_core::id::DocumentId;
use strop_workspace::{Filesystem, ResourceLocation};

fn unavailable(reason: impl Into<String>, next_step: impl Into<String>) -> CapabilityDecision {
    CapabilityDecision::Unavailable {
        reason: reason.into(),
        next_step: next_step.into(),
    }
}

impl Editor {
    pub fn current_namespace_capabilities(&self) -> NamespaceCapabilities {
        self.namespace_capabilities_for(&self.directory_context(), Some(self.current()))
    }

    pub fn namespace_capability_for(
        &self,
        root: &ResourceLocation,
        capability: WorkspaceCapability,
        document: Option<DocumentId>,
    ) -> CapabilityDecision {
        self.namespace_capabilities_for(root, document)
            .get(capability)
            .clone()
    }

    pub fn namespace_capabilities_for(
        &self,
        root: &ResourceLocation,
        document: Option<DocumentId>,
    ) -> NamespaceCapabilities {
        use CapabilityDecision::Available;
        let (read, tools, search, write, terminal) = match &root.filesystem {
            Filesystem::Local => (Available, Available, Available, Available, Available),
            Filesystem::Remote(endpoint) => {
                let admitted = self.remote.workers.get(endpoint).is_some();
                let worker_step = ":remote worker enables tools and terminal here";
                let tools = if admitted {
                    Available
                } else {
                    unavailable("no admitted SSH worker", worker_step)
                };
                let terminal = if admitted {
                    Available
                } else {
                    unavailable(
                        "an interactive terminal needs the admitted SSH worker",
                        worker_step,
                    )
                };
                // Search is the explicit SSH admission action; it never uses
                // SFTP or a host path as an execution fallback.
                let write = match document.and_then(|id| self.docs.get(id)) {
                    Some(document)
                        if document.remote_metadata().is_some_and(|source| {
                            source.write.is_some()
                                && source.file.path().starts_with(&root.path)
                                && source.file.endpoint() == endpoint
                        }) =>
                    {
                        Available
                    }
                    _ => unavailable(
                        "this SSH document has no verified write permit",
                        "open the file and use :remote edit",
                    ),
                };
                (Available, tools, Available, write, terminal)
            }
            Filesystem::Container(id) => {
                let identity = self.containers.attached.get(id.as_str());
                let Some(identity) = identity else {
                    let decision = unavailable(
                        "this container incarnation is not attached",
                        "attach the running container first",
                    );
                    return NamespaceCapabilities {
                        root: root.clone(),
                        read: decision.clone(),
                        tools: decision.clone(),
                        search: decision.clone(),
                        write: decision.clone(),
                        terminal: decision,
                    };
                };
                let admitted = self.containers.workers.get(identity).is_some();
                let worker_step = ":container-worker enables tools, Search and terminal here";
                let tools = if admitted {
                    Available
                } else {
                    unavailable(
                        "no admitted worker for this container incarnation",
                        worker_step,
                    )
                };
                let search = if admitted {
                    Available
                } else {
                    unavailable("container Search needs its admitted worker", worker_step)
                };
                let terminal = if admitted {
                    Available
                } else {
                    unavailable("container terminal needs its admitted worker", worker_step)
                };
                let permitted = admitted
                    && document.is_some_and(|document| {
                        self.io.container_writes.permit_matches(
                            document,
                            root,
                            &identity.started_at,
                        )
                    });
                let write = if permitted {
                    Available
                } else if admitted {
                    unavailable(
                        "this container document has no verified edit permit",
                        ":container-edit verifies this snapshot and enables editing",
                    )
                } else {
                    unavailable(
                        "container editing needs both worker admission and document authority",
                        ":container-worker, then :container-edit on the file",
                    )
                };
                (Available, tools, search, write, terminal)
            }
        };
        NamespaceCapabilities {
            root: root.clone(),
            read,
            tools,
            search,
            write,
            terminal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn local_and_unadmitted_ssh_decisions_match_execution_owners() {
        let editor = Editor::new(Buffer::from_text(""));
        let local = editor.namespace_capabilities_for(
            &ResourceLocation::local("/workspace".into()),
            Some(editor.current()),
        );
        assert!(local.read.available());
        assert!(local.tools.available());
        assert!(local.search.available());
        assert!(local.write.available());
        assert!(local.terminal.available());

        let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://fixture.example").unwrap();
        let remote = editor.namespace_capabilities_for(
            &ResourceLocation::remote(endpoint, "/workspace".into()),
            Some(editor.current()),
        );
        assert!(remote.read.available());
        assert!(remote.search.available(), "Search is the admission action");
        assert!(!remote.tools.available());
        assert!(!remote.write.available());
        assert!(!remote.terminal.available());
        assert_eq!(
            remote.write.next_step(),
            Some("open the file and use :remote edit")
        );
    }
}
