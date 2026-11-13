//! One real Directory buffer for every supported filesystem namespace.
use super::{Document, DocumentSource, JumpRecord};
use imbl::OrdMap;
use std::sync::Arc;
use strop_core::{id::LineIndex, Buffer};
use strop_workspace::{DirectoryEntry, EntryKind, EntryName, ListingState, ResourceLocation};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Directory {
    pub location: ResourceLocation,
    pub entries: Arc<[DirectoryEntry]>,
    pub visible: Arc<[usize]>,
    pub filter: String,
    /// Dotfiles are listed (`g.` flips it; remembered per folder).
    pub show_hidden: bool,
    /// Where the metadata column starts, measured once per listing so it
    /// does not move while scrolling.
    pub name_width: usize,
    pub state: ListingState,
    pub stale: Option<String>,
    /// Verified name changes used only to restore positions on the next snapshot.
    /// Navigation continues to describe the displayed (possibly stale) listing.
    #[serde(with = "entry_map")]
    restoration: OrdMap<EntryName, Option<ResourceLocation>>,
    #[serde(skip)]
    pub connection: Option<strop_remote::ConnectionLease>,
    pub return_to: Option<JumpRecord>,
    pub draft: Option<crate::editor::filesystem::draft::Draft>,
}

impl Directory {
    pub fn from_listing(
        listed: strop_fs::ListedDirectory,
        connection: Option<strop_remote::ConnectionLease>,
    ) -> Self {
        let mut source = Self::new(listed.snapshot);
        source.connection = connection;
        source
    }
    pub fn new(snapshot: strop_workspace::DirectorySnapshot) -> Self {
        let mut directory = Self {
            location: snapshot.location,
            visible: (0..snapshot.entries.len()).collect(),
            entries: snapshot.entries,
            state: snapshot.state,
            filter: String::new(),
            show_hidden: true,
            name_width: 0,
            stale: None,
            restoration: OrdMap::new(),
            connection: None,
            return_to: None,
            draft: None,
        };
        directory.measure();
        directory
    }

    /// The metadata column: two cells past the longest visible name,
    /// between 26 and 50 cells.
    pub(crate) fn measure(&mut self) {
        let longest = self
            .visible
            .iter()
            .map(|&index| {
                let entry = &self.entries[index];
                UnicodeWidthStr::width(entry.name.display().as_str())
                    + usize::from(entry.observation.kind == EntryKind::Directory)
            })
            .max()
            .unwrap_or(0);
        self.name_width = (longest + 2).clamp(26, 50);
    }

    /// One buffer line per visible entry: its name, a folder with a
    /// trailing `/`. The filename draft projects exactly this text.
    pub fn row_text(entry: &DirectoryEntry) -> String {
        let mut name = entry.name.display();
        if entry.observation.kind == EntryKind::Directory {
            name.push('/');
        }
        name
    }

    /// `location` as seen from the folder `root`: `notes.txt`,
    /// `sub/inner.txt`, `../docs/notes.txt`; its full label when the
    /// namespaces differ.
    pub fn relative_label(root: &ResourceLocation, location: &ResourceLocation) -> String {
        use std::path::{Component, PathBuf};
        if root.filesystem != location.filesystem {
            return location.label();
        }
        let from: Vec<Component> = root.path.components().collect();
        let to: Vec<Component> = location.path.components().collect();
        let common = from
            .iter()
            .zip(&to)
            .take_while(|(left, right)| left == right)
            .count();
        let mut path = PathBuf::new();
        for _ in common..from.len() {
            path.push("..");
        }
        for part in &to[common..] {
            path.push(part);
        }
        if path.as_os_str().is_empty() {
            path.push(".");
        }
        strop_workspace::directory::display_path(&path)
    }

    /// The buffer for this listing: editable (with its filename draft)
    /// when the visible rows fit one batch, read-only otherwise.
    pub fn into_document(mut self, id: strop_core::worker::WorkerId) -> Result<Document, String> {
        let text = if self.visible.len() <= crate::editor::filesystem::draft::ROW_LIMIT {
            let (draft, rope) = crate::editor::filesystem::draft::Draft::new(id, &self)?;
            self.draft = Some(draft);
            rope
        } else {
            self.draft = None;
            ropey::Rope::from_str(&self.text())
        };
        Ok(Document::directory(Buffer::from_snapshot(text), self))
    }

    pub(crate) fn record_relocation(
        &mut self,
        source: &ResourceLocation,
        destination: Option<&ResourceLocation>,
    ) {
        let updates: Vec<_> = self
            .restoration
            .iter()
            .filter_map(|(name, location)| {
                let location = location.as_ref()?;
                (location.filesystem == source.filesystem
                    && location.path.starts_with(&source.path))
                .then(|| {
                    (
                        name.clone(),
                        destination.and_then(|target| location.relocated(source, target)),
                    )
                })
            })
            .collect();
        for (name, location) in updates {
            self.restoration.insert(name, location);
        }
        if let Some(index) = self.entry_index(source) {
            let name = self.entries[index].name.clone();
            if !self.restoration.contains_key(&name) {
                self.restoration.insert(name, destination.cloned());
            }
        }
        debug_assert!(self.restoration.len() <= self.entries.len());
    }

    pub(crate) fn restoration_location(&self, line: LineIndex) -> Option<ResourceLocation> {
        if self.draft.is_some() {
            return self.entry_location(line);
        }
        let entry = self.entry(line)?;
        self.restoration
            .get(&entry.name)
            .cloned()
            .unwrap_or_else(|| Some(self.location_of(entry)))
    }
    /// Worker-side projection. Filenames are escaped for display, never decoded
    /// back into resources by navigation or operation admission.
    pub fn text(&self) -> String {
        let mut text = String::new();
        for &index in self.visible.iter() {
            text.push_str(&Self::row_text(&self.entries[index]));
            text.push('\n');
        }
        text
    }
    pub fn parent(&self) -> Option<ResourceLocation> {
        strop_fs::parent(&self.location)
    }
    pub fn entry(&self, line: LineIndex) -> Option<&DirectoryEntry> {
        self.visible
            .get(line.get())
            .and_then(|&index| self.entries.get(index))
    }
    pub fn entry_location(&self, line: LineIndex) -> Option<ResourceLocation> {
        if let Some(draft) = &self.draft {
            return draft.location(line.get());
        }
        self.entry(line).map(|entry| self.location_of(entry))
    }
    pub fn location_of(&self, entry: &DirectoryEntry) -> ResourceLocation {
        ResourceLocation {
            filesystem: self.location.filesystem.clone(),
            path: self.location.path.join(entry.name.as_path()),
        }
    }
    pub fn line_for(&self, location: &ResourceLocation) -> Option<LineIndex> {
        if let Some(draft) = &self.draft {
            return (0..=super::super::filesystem::draft::ROW_LIMIT)
                .find(|row| draft.location(*row).as_ref() == Some(location))
                .map(LineIndex::new);
        }
        self.visible
            .binary_search(&self.entry_index(location)?)
            .ok()
            .map(LineIndex::new)
    }
    pub fn entry_index(&self, location: &ResourceLocation) -> Option<usize> {
        if location.filesystem != self.location.filesystem
            || location.path.parent() != Some(self.location.path.as_path())
        {
            return None;
        }
        let name = location.path.file_name()?;
        let split = self
            .entries
            .partition_point(|entry| entry.observation.kind == EntryKind::Directory);
        for (offset, entries) in [(0, &self.entries[..split]), (split, &self.entries[split..])] {
            if let Ok(index) =
                entries.binary_search_by(|entry| entry.name.as_path().as_os_str().cmp(name))
            {
                return Some(offset + index);
            }
        }
        None
    }
    pub fn validate(&self) -> bool {
        self.location.path.is_absolute()
            && self
                .draft
                .as_ref()
                .is_none_or(|draft| draft.root == self.location && draft.valid())
            && !self
                .location
                .path
                .as_os_str()
                .as_encoded_bytes()
                .contains(&0)
            && self.visible.iter().all(|&index| index < self.entries.len())
            && self.visible.windows(2).all(|pair| pair[0] < pair[1])
            && self
                .entries
                .iter()
                .map(|entry| &entry.name)
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.entries.len()
            && self.entries.windows(2).all(|pair| {
                let group = |entry: &DirectoryEntry| {
                    usize::from(entry.observation.kind != EntryKind::Directory)
                };
                (group(&pair[0]), &pair[0].name) < (group(&pair[1]), &pair[1].name)
            })
    }
}

impl Document {
    pub fn directory(mut buffer: Buffer, source: Directory) -> Self {
        debug_assert!(
            source.validate(),
            "directory rows have one native identity each"
        );
        buffer.path = None;
        buffer.name = Some(source.location.label());
        if source.draft.as_ref().is_none_or(|draft| !draft.editable()) {
            buffer.set_readonly(strop_core::ReadonlyReason::DirectoryListing);
        }
        Self {
            buf: buffer,
            syntax_hint: None,
            indent: super::Indent::default(),
            indent_override: super::IndentOverride::default(),
            detection: None,
            source: DocumentSource::Directory(Box::new(source)),
            external_change: false,
            loan: None,
        }
    }
    pub fn directory_metadata_ref(&self) -> Option<&Directory> {
        match &self.source {
            DocumentSource::Directory(source) => Some(source),
            _ => None,
        }
    }
    pub(crate) fn directory_metadata_mut(&mut self) -> Option<&mut Directory> {
        match &mut self.source {
            DocumentSource::Directory(source) => Some(source),
            _ => None,
        }
    }
}

mod entry_map {
    use super::*;
    use serde::Deserialize;
    pub fn serialize<S: serde::Serializer, T: serde::Serialize + Clone>(
        value: &OrdMap<EntryName, T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(value.iter())
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de> + Clone>(
        deserializer: D,
    ) -> Result<OrdMap<EntryName, T>, D::Error> {
        let entries = Vec::<(EntryName, T)>::deserialize(deserializer)?;
        let count = entries.len();
        let result: OrdMap<_, _> = entries.into_iter().collect();
        if result.len() != count {
            return Err(serde::de::Error::custom(
                "duplicate native directory entry key",
            ));
        }
        Ok(result)
    }
}
