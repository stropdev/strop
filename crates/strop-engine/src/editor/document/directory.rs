//! One real Directory buffer for every supported filesystem namespace.
use super::{Document, DocumentSource, JumpRecord};
use imbl::OrdMap;
use std::fmt::Write;
use std::sync::Arc;
use strop_core::{id::LineIndex, Buffer};
use strop_workspace::{
    DirectoryEntry, EntryKind, EntryName, ListingState, Observation, ResourceLocation,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Directory {
    pub location: ResourceLocation,
    pub entries: Arc<[DirectoryEntry]>,
    pub visible: Arc<[usize]>,
    pub filter: String,
    pub summary: String,
    pub state: ListingState,
    pub stale: Option<String>,
    pub view_revision: strop_core::id::BufferRevision,
    #[serde(with = "entry_map")]
    pub marked: OrdMap<EntryName, Observation>,
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
    pub fn from_listing(listed: strop_fs::ListedDirectory) -> Self {
        let mut source = Self::new(listed.snapshot);
        source.connection = listed.connection;
        source
    }
    pub fn new(snapshot: strop_workspace::DirectorySnapshot) -> Self {
        Self {
            location: snapshot.location,
            visible: (0..snapshot.entries.len()).collect(),
            entries: snapshot.entries,
            state: snapshot.state,
            filter: String::new(),
            stale: None,
            marked: OrdMap::new(),
            restoration: OrdMap::new(),
            connection: None,
            return_to: None,
            draft: None,
            view_revision: strop_core::id::BufferRevision::new(0),
            summary: "folder · hidden on · no ignores".into(),
        }
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
    /// back into resources by navigation, marks or operation admission.
    pub fn text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(
            text,
            "{} — {} / {} entries{}",
            self.location.label(),
            self.visible.len(),
            self.entries.len(),
            match self.state {
                ListingState::Complete => "",
                ListingState::Limited { .. } => " — LIMITED",
                ListingState::Failed { .. } => " — INCOMPLETE",
            }
        );
        text.push_str("../\n");
        for &index in self.visible.iter() {
            let entry = &self.entries[index];
            text.push_str(&entry.name.display());
            match entry.observation.kind {
                EntryKind::Directory => text.push('/'),
                EntryKind::SymbolicLink => text.push('@'),
                _ => {}
            }
            text.push_str("  ");
            text.push(entry.observation.kind.marker());
            match entry.observation.permissions {
                Some(permissions) => {
                    let _ = write!(text, "{permissions}");
                }
                None => text.push_str("?????????"),
            }
            match entry.observation.size {
                Some(size) => {
                    let _ = write!(text, "  {size}");
                }
                None => text.push_str("  ?"),
            }
            if entry.error.is_some() {
                text.push_str("  unavailable");
            }
            text.push('\n');
        }
        text
    }
    pub fn parent(&self) -> Option<ResourceLocation> {
        strop_fs::parent(&self.location)
    }
    pub fn entry(&self, line: LineIndex) -> Option<&DirectoryEntry> {
        line.get()
            .checked_sub(2)
            .and_then(|row| self.visible.get(row))
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
            .map(|row| LineIndex::new(row + 2))
    }
    pub(crate) fn entry_index(&self, location: &ResourceLocation) -> Option<usize> {
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
    pub fn toggle_mark(&mut self, line: LineIndex) -> bool {
        let Some(entry) = self.entry(line) else {
            return false;
        };
        let name = entry.name.clone();
        let observation = entry.observation.clone();
        if self.marked.remove(&name).is_none() {
            self.marked.insert(name, observation);
        }
        true
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
    pub fn directory(mut buffer: Buffer, mut source: Directory) -> Self {
        debug_assert!(
            source.validate(),
            "directory rows have one native identity each"
        );
        source.view_revision = buffer.revision();
        buffer.path = None;
        buffer.name = Some(format!("directory {}", source.location.label()));
        buffer.readonly = source.draft.as_ref().is_none_or(|draft| !draft.editable());
        Self {
            buf: buffer,
            syntax_hint: None,
            indent: super::Indent::default(),
            indent_override: super::IndentOverride::default(),
            detection: None,
            source: DocumentSource::Directory(Box::new(source)),
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
