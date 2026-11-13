//! Filename edits are ordinary buffer edits over immutable native source tokens.
//! Text supplies destinations only; provenance supplies every source authority.
mod compile;
mod editing;
mod geometry;
mod names;
mod registers;
#[cfg(test)]
mod tests;
use super::*;
use imbl::{OrdMap, Vector};
use std::sync::Arc;
use strop_workspace::{DirectorySnapshot, Observation};
pub const ROW_LIMIT: usize = strop_fs::batch::STEP_LIMIT;
const TEXT_LIMIT: usize = 1024 * 1024;
pub const OVER_LIMIT: &str = "more than 512 entries; :filter first to edit names";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileReference {
    pub draft: WorkerId,
    pub entry: usize,
    /// The row text the listing projected for this entry.
    pub text: String,
    pub location: ResourceLocation,
    pub observation: Observation,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegisterEntry {
    pub source: Arc<FileReference>,
    pub cut: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileRegister {
    pub rows: Vec<Option<RegisterEntry>>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Origin {
    Original(usize),
    Copy(Arc<FileReference>),
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Row {
    id: u64,
    origin: Option<Origin>,
    bytes: usize,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Geometry {
    rows: Vector<Row>,
    error: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    Editing,
    Applying(WorkerId),
    Reloading,
    Unconfirmed(WorkerId),
}
/// What one draft row will do when written; the renderer labels rows
/// with it and the compiler classifies rows with it, so the preview and
/// the batch cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowChange<'a> {
    /// An existing entry, name unchanged.
    Unchanged(&'a FileReference),
    /// An existing entry under a new name.
    Renamed(&'a FileReference),
    /// A copy of a file, from this folder or another.
    Copy(&'a FileReference),
    /// A name typed fresh: created on write.
    New,
    /// A blank row: no effect.
    Blank,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stamp {
    pub id: WorkerId,
    pub intent: u64,
    pub root: ResourceLocation,
}
#[derive(Debug, Clone)]
struct Hint {
    revision: BufferRevision,
    at: usize,
    removed: usize,
    inserted: usize,
    delete_last_row: bool,
    ambiguous: bool,
    paste: Option<(usize, Arc<FileRegister>, usize)>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    pub id: WorkerId,
    pub root: ResourceLocation,
    pub revision: BufferRevision,
    pub phase: Phase,
    pub intent_epoch: u64,
    base: Arc<[Arc<FileReference>]>,
    pub(crate) targets: HashMap<u64, ResourceLocation>,
    geometry: Geometry,
    snapshots: OrdMap<usize, Geometry>,
    next_row: u64,
    #[serde(skip)]
    hint: Option<Hint>,
    pub latest: Option<DirectorySnapshot>,
}
impl Draft {
    pub(crate) fn new(
        id: WorkerId,
        source: &super::super::Directory,
    ) -> Result<(Self, ropey::Rope), String> {
        if source.visible.len() > ROW_LIMIT {
            return Err(OVER_LIMIT.into());
        }
        let mut text = String::new();
        let mut rows = Vector::new();
        let mut base = Vec::with_capacity(source.visible.len());
        for (index, visible) in source.visible.iter().enumerate() {
            let entry = source
                .entries
                .get(*visible)
                .ok_or("directory row identity is invalid")?;
            let name = super::super::Directory::row_text(entry);
            if text.len().saturating_add(name.len()).saturating_add(1) > TEXT_LIMIT {
                return Err("filename draft exceeds its 1 MiB projection bound".into());
            }
            rows.push_back(Row {
                id: index as u64,
                origin: Some(Origin::Original(index)),
                bytes: name.len(),
            });
            text.push_str(&name);
            text.push('\n');
            base.push(Arc::new(FileReference {
                draft: id,
                entry: index,
                text: name,
                location: source.location_of(entry),
                observation: entry.observation.clone(),
            }));
        }
        let next_row = rows.len() as u64 + 1;
        rows.push_back(Row {
            id: next_row - 1,
            origin: None,
            bytes: 0,
        });
        let geometry = Geometry { rows, error: None };
        let mut snapshots = OrdMap::new();
        snapshots.insert(0, geometry.clone());
        Ok((
            Self {
                id,
                root: source.location.clone(),
                revision: BufferRevision::new(0),
                phase: Phase::Editing,
                base: base.into(),
                geometry,
                snapshots,
                next_row,
                hint: None,
                latest: None,
                intent_epoch: 0,
                targets: HashMap::new(),
            },
            ropey::Rope::from_str(&text),
        ))
    }
    pub fn source(&self, line: usize) -> Option<&FileReference> {
        if self.geometry.error.is_some() {
            return None;
        }
        match self.geometry.rows.get(line)?.origin.as_ref()? {
            Origin::Original(index) => self.base.get(*index).map(AsRef::as_ref),
            Origin::Copy(source) => Some(source.as_ref()),
        }
    }
    /// What row `line`, now reading `name`, does when written.
    pub fn row_change(&self, line: usize, name: &str) -> RowChange<'_> {
        if name.is_empty() {
            return RowChange::Blank;
        }
        let Some(row) = self.geometry.rows.get(line) else {
            return RowChange::New;
        };
        match &row.origin {
            None => RowChange::New,
            Some(Origin::Copy(source)) => RowChange::Copy(source),
            Some(Origin::Original(index)) => match self.base.get(*index) {
                Some(source)
                    if source.text == name
                        || self
                            .destination(name)
                            .is_ok_and(|(destination, _)| destination == source.location) =>
                {
                    RowChange::Unchanged(source)
                }
                Some(source) => RowChange::Renamed(source),
                None => RowChange::New,
            },
        }
    }
    /// The location a row's text names, and whether it names a folder.
    pub(super) fn destination(&self, name: &str) -> Result<(ResourceLocation, bool), String> {
        let decoded = names::decode(name)?;
        let directory = decoded.as_os_str().as_encoded_bytes().last() == Some(&b'/');
        let relative: std::path::PathBuf = decoded.components().collect();
        Ok((
            ResourceLocation {
                filesystem: self.root.filesystem.clone(),
                path: self.root.path.join(relative),
            },
            directory,
        ))
    }
    pub fn location(&self, line: usize) -> Option<ResourceLocation> {
        if self.phase == Phase::Reloading {
            if let Some(target) = self.row_id(line).and_then(|row| self.targets.get(&row)) {
                return Some(target.clone());
            }
        }
        self.source(line).map(|source| source.location.clone())
    }
    pub fn error(&self) -> Option<&str> {
        self.geometry.error.as_deref()
    }
    pub fn row_id(&self, line: usize) -> Option<u64> {
        self.geometry.rows.get(line).map(|row| row.id)
    }
    pub fn editable(&self) -> bool {
        self.phase == Phase::Editing
    }
    fn allocate_row(&mut self, origin: Option<Origin>, bytes: usize) -> Result<Row, String> {
        let id = self.next_row;
        self.next_row = id.checked_add(1).ok_or("filename row identity exhausted")?;
        Ok(Row { id, origin, bytes })
    }
    pub(crate) fn valid(&self) -> bool {
        self.root.path.is_absolute()
            && self.base.len() <= ROW_LIMIT
            && self.geometry.rows.len() <= ROW_LIMIT + 1
            && self.geometry.rows.iter().all(|row| {
                row.id < self.next_row
                    && match &row.origin {
                        Some(Origin::Original(index)) => *index < self.base.len(),
                        Some(Origin::Copy(source)) => {
                            source.location.filesystem == self.root.filesystem
                        }
                        None => true,
                    }
            })
    }
}
