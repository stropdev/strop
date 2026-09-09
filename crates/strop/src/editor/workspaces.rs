//! Workspace registry (0042 slice 2): the editor's bound workspace
//! contexts — one per filesystem namespace in use — with stable
//! generational identity and an incarnation counter that bumps when an
//! endpoint disconnects and later reconnects, so results from before a
//! break are never mistaken for the new session's.
//!
//! Jobs keep capturing the concrete targets they already capture; this
//! registry is the identity hub the explain surface reads and later
//! capability scoping (execution bindings, container contexts) builds on.

use std::collections::HashMap;
use std::path::PathBuf;

use strop_core::id::{Arena, WorkspaceId, WorkspaceKind};
use strop_workspace::Filesystem;

/// One bound context: which filesystem, anchored where, which incarnation.
pub(crate) struct WorkspaceContext {
    pub filesystem: Filesystem,
    /// Local: the process cwd. Remote: no project-root concept yet — the
    /// endpoint is the context; real roots arrive with container and
    /// worktree bindings (0037).
    pub root: Option<PathBuf>,
    /// Bumps on disconnect; a reconnect binds a fresh incarnation.
    pub incarnation: u64,
}

#[derive(Default)]
pub(crate) struct WorkspaceRegistry {
    arena: Arena<WorkspaceKind, WorkspaceContext>,
    by_filesystem: HashMap<Filesystem, WorkspaceId>,
}

impl WorkspaceRegistry {
    /// The context for a filesystem, binding it on first use. Idempotent:
    /// an already-bound filesystem keeps its identity and incarnation.
    pub fn bind(&mut self, filesystem: Filesystem, root: Option<PathBuf>) -> WorkspaceId {
        if let Some(id) = self.by_filesystem.get(&filesystem) {
            return *id;
        }
        let id = self.arena.insert(WorkspaceContext {
            filesystem: filesystem.clone(),
            root,
            incarnation: 0,
        });
        self.by_filesystem.insert(filesystem, id);
        id
    }

    /// A disconnect ends the incarnation: the next bind/reconnect observes
    /// a bumped counter rather than silently continuing the old session.
    pub fn note_disconnect(&mut self, filesystem: &Filesystem) {
        if let Some(id) = self.by_filesystem.get(filesystem) {
            if let Some(context) = self.arena.get_mut(*id) {
                context.incarnation += 1;
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (WorkspaceId, &WorkspaceContext)> {
        self.arena.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_workspace::RemoteEndpoint;

    #[test]
    fn rebinding_keeps_identity_and_disconnect_bumps_incarnation() {
        let mut registry = WorkspaceRegistry::default();
        let local = registry.bind(Filesystem::Local, Some(PathBuf::from("/work")));
        assert_eq!(registry.bind(Filesystem::Local, None), local, "idempotent");
        let endpoint = RemoteEndpoint::parse("ssh://dev@example.com:2222").unwrap();
        let remote = registry.bind(Filesystem::Remote(endpoint.clone()), None);
        assert_ne!(local, remote, "namespaces never share a slot");
        let remote_fs = Filesystem::Remote(endpoint.clone());
        let incarnation = |registry: &WorkspaceRegistry| {
            registry
                .iter()
                .find(|(_, context)| context.filesystem == remote_fs)
                .map(|(_, context)| context.incarnation)
        };
        assert_eq!(incarnation(&registry), Some(0));
        registry.note_disconnect(&remote_fs);
        assert_eq!(incarnation(&registry), Some(1));
        assert_eq!(
            registry.bind(Filesystem::Remote(endpoint), None),
            remote,
            "reconnect reuses the slot with the bumped incarnation"
        );
        let other = RemoteEndpoint::parse("ssh://other.example.com").unwrap();
        registry.note_disconnect(&Filesystem::Remote(other));
    }
}
