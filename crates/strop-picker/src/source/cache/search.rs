//! Capture native folder identity for both inventory and streaming walks.
use super::{
    Arc, AtomicBool, Caches, Directories, Directory, InventoryError, InventoryRoot, Mutex,
    Ordering, Path, PathBuf,
};

impl Directories {
    pub(super) fn open(cache: &Mutex<Caches>, root: &Path) -> Result<Self, InventoryError> {
        let pinned = {
            let owner = cache.lock();
            owner
                .pinned
                .as_ref()
                .filter(|(p, _)| root.starts_with(p.as_path()))
                .map(|(p, _)| InventoryRoot(p.clone(), owner.declaration_ticket.clone()))
        };
        let folder = Directory::open(root)?;
        let pinned = pinned
            .map(|ticket| {
                let directory = if ticket.0.as_path() == root {
                    folder.clone()
                } else {
                    Directory::open(&ticket.0)?
                };
                Ok::<_, InventoryError>((ticket, directory))
            })
            .transpose()?;
        Ok(Self { pinned, folder })
    }

    pub(super) fn current(&self, cache: &Mutex<Caches>, root: &Path) -> bool {
        let mut current = true;
        if !self.folder.still_at(root) {
            cache.lock().replaced_directory(root, &self.folder);
            current = false;
        }
        if let Some((ticket, dir)) = &self.pinned {
            if ticket.0.as_path() != root && !dir.still_at(&ticket.0) {
                cache.lock().replaced_directory(&ticket.0, dir);
                current = false;
            }
        }
        current
    }
}

/// A priority hint is one native name, never a query, match or membership list.
/// Bound payloads independently of the walk (at most four retained scopes).
const HINT_BYTES: usize = 4096;

pub(in crate::source) struct SearchScan {
    directories: Directories,
    folder: Arc<PathBuf>,
    pub epoch: Arc<AtomicBool>,
    pub hint: Option<Arc<Path>>,
    first_hit: std::sync::OnceLock<Arc<Path>>,
}

impl SearchScan {
    pub(in crate::source) fn begin(
        cache: &Mutex<Caches>,
        root: &Path,
    ) -> Result<Self, InventoryError> {
        let directories = Directories::open(cache, root)?;
        let mut owner = cache.lock();
        let (slot, evicted) = owner.scope_for(root, &directories);
        let scope = &owner.scopes[slot?];
        let scan = Self {
            directories,
            folder: scope.folder.clone(),
            epoch: scope.current.clone(),
            hint: scope.search_hint.clone(),
            first_hit: Default::default(),
        };
        drop(owner);
        drop(evicted);
        Ok(scan)
    }

    pub(in crate::source) fn note_hit(&self, path: &Path) {
        if path.as_os_str().as_encoded_bytes().len() <= HINT_BYTES {
            self.first_hit.get_or_init(|| match &self.hint {
                Some(prior) if prior.as_ref() == path => prior.clone(),
                _ => Arc::from(path),
            });
        }
    }

    pub(in crate::source) fn current(&self, cache: &Mutex<Caches>) -> bool {
        self.epoch.load(Ordering::Acquire) && self.directories.current(cache, &self.folder)
    }

    pub(in crate::source) fn finish(self, cache: &Mutex<Caches>) -> bool {
        let mut owner = cache.lock();
        let Some(scope) = owner
            .scopes
            .iter_mut()
            .find(|s| !s.retired && Arc::ptr_eq(&s.current, &self.epoch))
        else {
            return false;
        };
        if !self.epoch.load(Ordering::Acquire) {
            return false;
        }
        scope.search_hint = self.first_hit.into_inner();
        true
    }
}
