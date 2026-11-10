//! Durable project bindings (0081 §6.2): a workspace is an exact
//! filesystem-namespace/absolute-root pair, not one mutable root per endpoint.
//! Generational IDs keep stale owners from aliasing a later binding, while the
//! incarnation counter advances for every root on a disconnected endpoint.

use std::collections::HashMap;
use std::path::PathBuf;

use strop_core::id::{Arena, WorkspaceId, WorkspaceKind};
use strop_workspace::{Filesystem, ResourceLocation};

/// The durable identity of one selected project root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceKey {
    pub filesystem: Filesystem,
    #[serde(with = "strop_core::path_serde")]
    pub root: PathBuf,
}

impl WorkspaceKey {
    pub fn new(location: ResourceLocation) -> Result<Self, WorkspaceRootError> {
        if !location.path.is_absolute() {
            return Err(WorkspaceRootError::RelativePath);
        }
        Ok(Self {
            filesystem: location.filesystem,
            root: location.path,
        })
    }

    pub fn location(&self) -> ResourceLocation {
        ResourceLocation {
            filesystem: self.filesystem.clone(),
            path: self.root.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceRootError {
    #[error("workspace root must be an absolute native path")]
    RelativePath,
    #[error(transparent)]
    Exhausted(#[from] strop_core::id::ArenaExhausted),
}

/// One bound project: which filesystem, anchored where, which incarnation.
pub struct WorkspaceContext {
    pub filesystem: Filesystem,
    /// Always absolute. Directory browsing does not mutate this selected
    /// identity.
    pub root: PathBuf,
    /// Bumps on disconnect; reconnecting the same exact root reuses its ID.
    pub incarnation: u64,
}

impl WorkspaceContext {
    pub fn key(&self) -> WorkspaceKey {
        WorkspaceKey {
            filesystem: self.filesystem.clone(),
            root: self.root.clone(),
        }
    }

    pub fn location(&self) -> ResourceLocation {
        ResourceLocation {
            filesystem: self.filesystem.clone(),
            path: self.root.clone(),
        }
    }
}

#[derive(Default)]
pub struct WorkspaceRegistry {
    arena: Arena<WorkspaceKind, WorkspaceContext>,
    by_root: HashMap<WorkspaceKey, WorkspaceId>,
}

impl WorkspaceRegistry {
    /// Deliberately select/open an exact project root. Selection is idempotent:
    /// another root on the same endpoint gets an independent durable ID.
    pub fn select_root(
        &mut self,
        root: ResourceLocation,
    ) -> Result<WorkspaceId, WorkspaceRootError> {
        let key = WorkspaceKey::new(root)?;
        if let Some(id) = self.by_root.get(&key) {
            return Ok(*id);
        }
        let id = self.arena.try_insert(WorkspaceContext {
            filesystem: key.filesystem.clone(),
            root: key.root.clone(),
            incarnation: 0,
        })?;
        self.by_root.insert(key, id);
        Ok(id)
    }

    /// Resolve a resource to the longest selected root that covers it in the
    /// same namespace. In particular, `/work/a` does not cover `/work/ab`.
    pub fn resolve(&self, resource: &ResourceLocation) -> Option<(WorkspaceId, &WorkspaceContext)> {
        self.resolve_path(&resource.filesystem, &resource.path)
    }

    /// Borrowed hot-path form: no path/location allocation for labels.
    pub fn resolve_path(
        &self,
        filesystem: &Filesystem,
        path: &std::path::Path,
    ) -> Option<(WorkspaceId, &WorkspaceContext)> {
        if !path.is_absolute() {
            return None;
        }
        self.arena
            .iter()
            .filter(|(_, context)| {
                context.filesystem == *filesystem && path.starts_with(&context.root)
            })
            .max_by_key(|(_, context)| context.root.components().count())
    }

    pub fn get(&self, id: WorkspaceId) -> Option<&WorkspaceContext> {
        self.arena.get(id)
    }

    /// A disconnect ends every project incarnation on that endpoint. If one
    /// counter is exhausted only that exact binding is retired; reconnect binds
    /// a fresh generational ID without disturbing sibling projects.
    pub fn note_disconnect(&mut self, filesystem: &Filesystem) {
        let affected: Vec<_> = self
            .arena
            .iter()
            .filter(|(_, context)| &context.filesystem == filesystem)
            .map(|(id, _)| id)
            .collect();
        for id in affected {
            let retire = match self.arena.get_mut(id) {
                Some(context) => match context.incarnation.checked_add(1) {
                    Some(next) => {
                        context.incarnation = next;
                        false
                    }
                    None => true,
                },
                None => continue,
            };
            if retire {
                if let Some(context) = self.arena.remove(id) {
                    self.by_root.remove(&context.key());
                }
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (WorkspaceId, &WorkspaceContext)> {
        self.arena.iter()
    }
}

impl super::Editor {
    /// Explicit project-open/root-selection boundary for UI and terminal
    /// integration. This never changes process cwd or another root.
    pub fn select_root(
        &mut self,
        root: ResourceLocation,
    ) -> Result<WorkspaceId, WorkspaceRootError> {
        self.workspaces.select_root(root)
    }

    pub fn workspace_root_for(&self, resource: &ResourceLocation) -> Option<ResourceLocation> {
        self.workspaces
            .resolve(resource)
            .map(|(_, context)| context.location())
    }

    pub fn workspace_root_path(
        &self,
        filesystem: &Filesystem,
        path: &std::path::Path,
    ) -> Option<&std::path::Path> {
        self.workspaces
            .resolve_path(filesystem, path)
            .map(|(_, context)| context.root.as_path())
    }

    /// Default scope for Files/Search/terminals: the focused resource's
    /// selected longest covering project. The project is pinned (plan 0092
    /// §2): a local FILE outside every selected root — a library source
    /// reached by go-to-definition, a loose file — keeps the launch project
    /// as its scope; visiting it never re-roots the editor. Only a
    /// deliberately browsed Directory, a terminal's launch location or an
    /// unbound remote resource scopes to its own exact location.
    pub fn current_workspace_root(&self) -> ResourceLocation {
        if let Some(launch) = self.terminal_launch_location(self.current()) {
            return self
                .workspace_root_for(launch)
                .unwrap_or_else(|| launch.clone());
        }
        let context = self.directory_context();
        if let Some(root) = self.workspace_root_for(&context) {
            return root;
        }
        if self.directory().is_none() && context.filesystem == Filesystem::Local {
            return ResourceLocation::local(self.cwd.clone());
        }
        context
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_workspace::RemoteEndpoint;

    fn remote(endpoint: &RemoteEndpoint, root: &str) -> ResourceLocation {
        ResourceLocation::remote(endpoint.clone(), PathBuf::from(root))
    }

    #[test]
    fn exact_roots_coexist_on_one_endpoint_and_resolve_longest_covering_root() {
        let mut registry = WorkspaceRegistry::default();
        let endpoint = RemoteEndpoint::parse("ssh://dev@example.com:2222").unwrap();
        let first = registry
            .select_root(remote(&endpoint, "/work/alpha"))
            .unwrap();
        let second = registry
            .select_root(remote(&endpoint, "/work/beta"))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            registry
                .select_root(remote(&endpoint, "/work/alpha"))
                .unwrap(),
            first
        );
        assert_eq!(registry.iter().count(), 2);

        let nested = registry
            .select_root(remote(&endpoint, "/work/alpha/nested"))
            .unwrap();
        let (_, resolved) = registry
            .resolve(&remote(&endpoint, "/work/alpha/nested/src/lib.rs"))
            .unwrap();
        assert_eq!(resolved.root, PathBuf::from("/work/alpha/nested"));
        let (resolved_id, _) = registry
            .resolve(&remote(&endpoint, "/work/alpha/src/lib.rs"))
            .unwrap();
        assert_eq!(resolved_id, first);
        assert!(registry
            .resolve(&remote(&endpoint, "/work/alphabet/src/lib.rs"))
            .is_none());
        let (_, resolved) = registry
            .resolve(&remote(&endpoint, "/work/beta/src/lib.rs"))
            .unwrap();
        assert_eq!(resolved.root, PathBuf::from("/work/beta"));
        assert_eq!(registry.get(second).unwrap().incarnation, 0);
        assert_eq!(registry.get(nested).unwrap().incarnation, 0);
    }

    #[test]
    fn same_basename_across_namespaces_has_distinct_identity_and_labels() {
        let mut registry = WorkspaceRegistry::default();
        let local = registry
            .select_root(ResourceLocation::local("/work/app".into()))
            .unwrap();
        let endpoint = RemoteEndpoint::parse("ssh://example.com").unwrap();
        let remote_id = registry.select_root(remote(&endpoint, "/srv/app")).unwrap();
        assert_ne!(local, remote_id);
        assert_eq!(
            registry
                .resolve(&ResourceLocation::local("/work/app/src/lib.rs".into()))
                .unwrap()
                .0,
            local
        );
        let (_, context) = registry
            .resolve(&remote(&endpoint, "/srv/app/src/lib.rs"))
            .unwrap();
        assert_eq!(context.location().label(), "ssh://example.com/srv/app");
        assert!(registry
            .resolve(&ResourceLocation::local("/srv/app/src/lib.rs".into()))
            .is_none());
    }

    #[test]
    fn selection_requires_absolute_native_paths_without_mutating_registry() {
        let mut registry = WorkspaceRegistry::default();
        assert_eq!(
            registry.select_root(ResourceLocation::local("relative/app".into())),
            Err(WorkspaceRootError::RelativePath)
        );
        assert_eq!(registry.iter().count(), 0);
    }

    #[test]
    fn disconnect_bumps_all_endpoint_roots_and_reconnect_keeps_exact_ids() {
        let mut registry = WorkspaceRegistry::default();
        let endpoint = RemoteEndpoint::parse("ssh://dev@example.com").unwrap();
        let filesystem = Filesystem::Remote(endpoint.clone());
        let first = registry
            .select_root(remote(&endpoint, "/work/alpha"))
            .unwrap();
        let second = registry
            .select_root(remote(&endpoint, "/work/beta"))
            .unwrap();
        let local = registry
            .select_root(ResourceLocation::local("/work/alpha".into()))
            .unwrap();
        registry.note_disconnect(&filesystem);
        assert_eq!(registry.get(first).unwrap().incarnation, 1);
        assert_eq!(registry.get(second).unwrap().incarnation, 1);
        assert_eq!(registry.get(local).unwrap().incarnation, 0);
        assert_eq!(
            registry
                .select_root(remote(&endpoint, "/work/alpha"))
                .unwrap(),
            first
        );
        assert_eq!(
            registry
                .select_root(remote(&endpoint, "/work/beta"))
                .unwrap(),
            second
        );
        let other = RemoteEndpoint::parse("ssh://other.example.com").unwrap();
        registry.note_disconnect(&Filesystem::Remote(other));
        assert_eq!(registry.get(first).unwrap().incarnation, 1);
    }

    #[test]
    fn exhausted_incarnation_retires_only_that_root_and_stale_id_stays_dead() {
        let mut registry = WorkspaceRegistry::default();
        let endpoint = RemoteEndpoint::parse("ssh://dev@example.com").unwrap();
        let filesystem = Filesystem::Remote(endpoint.clone());
        let retired = registry
            .select_root(remote(&endpoint, "/work/alpha"))
            .unwrap();
        let sibling = registry
            .select_root(remote(&endpoint, "/work/beta"))
            .unwrap();
        registry.arena.get_mut(retired).unwrap().incarnation = u64::MAX;
        registry.note_disconnect(&filesystem);
        assert!(registry.get(retired).is_none());
        assert_eq!(registry.get(sibling).unwrap().incarnation, 1);
        let fresh = registry
            .select_root(remote(&endpoint, "/work/alpha"))
            .unwrap();
        assert_ne!(fresh, retired);
        assert_eq!(registry.get(fresh).unwrap().incarnation, 0);
        assert_eq!(registry.iter().count(), 2);
    }

    /// Plan 0092 §2, the reported drift: launch in the project, follow
    /// go-to-definition into the Rust stdlib, open Files — the scope must
    /// still be the project, not the library's directory.
    #[test]
    fn a_file_outside_the_project_never_re_roots_files_search_or_terminals() {
        use strop_core::Buffer;
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("workspace");
        let library = fixture.path().join("rustup/library/core/src");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(library.join("option.rs"), "pub enum Option<T> {}\n").unwrap();
        let mut editor = super::super::Editor::new_in(
            Buffer::open(project.join("src/main.rs")).unwrap(),
            project.clone(),
        );
        let pinned = ResourceLocation::local(project.clone());
        assert_eq!(editor.current_workspace_root(), pinned);

        let outside = editor
            .docs
            .try_insert(super::super::document::Document::new(
                Buffer::open(library.join("option.rs")).unwrap(),
            ))
            .unwrap();
        editor.switch_to(outside);
        assert_eq!(editor.current_workspace_root(), pinned);
        editor.open_picker(strop_picker::Kind::Files);
        assert_eq!(
            editor.picker.as_ref().unwrap().file_root.as_ref(),
            Some(&pinned)
        );
        editor.close_picker();
        editor.open_search(false);
        assert_eq!(editor.search_scope().unwrap().root, pinned);
        editor.close_picker();
        editor.feed_text(":pwd<cr>");
        assert_eq!(editor.message, project.display().to_string());

        // Deliberately browsing a directory is the explicit way out.
        editor.browse_directory(&library.to_string_lossy()).unwrap();
        editor.wait_io().unwrap();
        assert_eq!(
            editor.current_workspace_root(),
            ResourceLocation::local(library)
        );
    }
}
