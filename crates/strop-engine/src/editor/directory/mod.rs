//! Directory navigation, view history and owned current-folder filtering.
//! Native effects belong to strop-fs; rows never become path or command text.
mod filter;
mod jobs;
pub(crate) use filter::apply_filter;
#[cfg(test)]
mod tests;
use super::io::{IoEvent, OpenIntent, Opened};
use super::{Directory, Editor, Key};
use crate::files::FileTarget;
use std::collections::HashMap;
use strop_core::id::{BufferRevision, DocumentId, LineIndex};
use strop_core::worker::{self, CancelReason, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_workspace::{Filesystem, ResourceLocation};

#[derive(Default)]
pub(crate) struct DirectoryState {
    pub filters: HashMap<WorkerId, FilterKey>,
    pub views: HashMap<ResourceLocation, SavedDirectoryView>,
    order: std::collections::VecDeque<ResourceLocation>,
}
#[derive(Clone)]
pub(crate) struct SavedDirectoryView {
    pub record: super::jumps::JumpRecord,
    pub directory: Directory,
    row: usize,
    top: usize,
    column: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum DirectoryTask {
    Filter,
    Reload,
    EditNames,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilterKey {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub location: ResourceLocation,
    pub query: String,
    pub focus: u64,
    pub(crate) task: DirectoryTask,
    pub(crate) draft: Option<WorkerId>,
}

impl Editor {
    pub(crate) fn relocate_directory_history(
        &mut self,
        source: Option<&ResourceLocation>,
        destination: Option<&ResourceLocation>,
        removed: bool,
    ) {
        let map = |location: &ResourceLocation| -> Option<ResourceLocation> {
            if let Some(source) = source.filter(|source| {
                source.filesystem == location.filesystem && location.path.starts_with(&source.path)
            }) {
                if removed {
                    return None;
                }
                if let Some(destination) = destination {
                    return location.relocated(source, destination);
                }
            }
            Some(location.clone())
        };
        let previous = std::mem::take(&mut self.directories.views);
        let mut moved = Vec::new();
        for (location, mut view) in previous {
            let Some(target) = map(&location) else {
                continue;
            };
            if removed || destination.is_some() {
                if let Some(source) = source {
                    view.directory.record_relocation(source, destination);
                }
            }
            view.directory.location = target.clone();
            if source.into_iter().chain(destination).any(|resource| {
                resource.filesystem == target.filesystem
                    && (resource.path.parent() == Some(target.path.as_path())
                        || target.path.starts_with(&resource.path))
            }) {
                view.directory.stale = Some("filesystem changed; refresh required".into());
            }
            if target != location {
                moved.push((target, view));
            } else {
                self.directories.views.insert(target, view);
            }
        }
        self.directories.views.extend(moved);
        let mut seen = std::collections::HashSet::new();
        self.directories.order = std::mem::take(&mut self.directories.order)
            .into_iter()
            .filter_map(|location| map(&location))
            .filter(|location| seen.insert(location.clone()))
            .collect();
    }
    pub fn directory(&self) -> Option<&Directory> {
        if self.docs.is_empty() {
            return None;
        }
        self.docs
            .get(self.current())
            .and_then(|doc| doc.directory_metadata_ref())
    }
    pub fn directory_visibility_summary(&self) -> Option<&str> {
        let source = self.directory()?;
        Some(if source.stale.is_some() {
            "stale directory — refresh required"
        } else if !source.state.is_complete() {
            "incomplete directory listing"
        } else {
            &source.summary
        })
    }
    pub(crate) fn remember_directory_view(&mut self) {
        let Some(source) = self.directory() else {
            return;
        };
        if source.draft.is_some() || source.view_revision != self.buf().revision() {
            return;
        }
        let location = source.location.clone();
        let saved = SavedDirectoryView {
            directory: source.clone(),
            record: self.jump_record(),
            row: self.buf().line_of(self.head()),
            top: self.view_top(),
            column: self.buf().col_of(self.head()),
        };
        self.directories.views.insert(location.clone(), saved);
        self.directories.order.retain(|old| old != &location);
        self.directories.order.push_back(location);
        while self.directories.order.len() > 16
            || self
                .directories
                .views
                .values()
                .map(|view| view.directory.entries.len())
                .sum::<usize>()
                > 200_000
        {
            let Some(old) = self.directories.order.pop_front() else {
                break;
            };
            self.directories.views.remove(&old);
        }
    }
    pub(crate) fn restore_directory_view(&mut self, document: DocumentId) {
        let Some(source) = self
            .docs
            .get(document)
            .and_then(|doc| doc.directory_metadata_ref())
        else {
            return;
        };
        if let Some(saved) = self.directories.views.get(&source.location).cloned() {
            let mut view = saved.record;
            if view.document != document {
                let locate = |line| {
                    saved
                        .directory
                        .restoration_location(LineIndex::new(line))
                        .and_then(|location| source.line_for(&location))
                        .map(|line| line.get())
                        .unwrap_or(line.min(1))
                };
                let row = locate(saved.row);
                let top = locate(saved.top);
                let buffer = &self.doc(document).buf;
                view.document = document;
                view.offset = buffer.clamp_boundary(
                    (buffer.line_start(row) + saved.column).min(buffer.line_end(row)),
                );
                view.anchor = view.offset;
                view.extras.clear();
                view.view_top = buffer.line_start(top);
            }
            self.jump_to(view);
        } else if source.entry(LineIndex::new(2)).is_some() {
            self.set_head(self.buf().line_start(2));
        } else {
            self.set_head(self.buf().line_start(1));
        }
    }
    /// Textual open operands inherit a Directory scope, not an ordinary remote
    /// file's namespace. Outside Directory, native paths retain local Ex semantics.
    pub(crate) fn open_context(&self) -> ResourceLocation {
        self.directory().map_or_else(
            || ResourceLocation::local(self.cwd.clone()),
            |directory| directory.location.clone(),
        )
    }
    /// Operand context is captured in the source namespace; no ambient chdir.
    pub(crate) fn directory_context(&self) -> ResourceLocation {
        if let Some(source) = self.directory() {
            return source.location.clone();
        }
        let source = self
            .navigation_source()
            .map_or(self.current(), |(source, _)| source);
        if let Some(target) = self.doc(source).file_target(&self.cwd) {
            if let Some(location) = target.resource_location() {
                if let Some(parent) = strop_fs::parent(&location) {
                    return parent;
                }
            }
        }
        ResourceLocation::local(self.cwd.clone())
    }
    pub(crate) fn directory_operand(&self, value: &str) -> Result<FileTarget, String> {
        Self::directory_operand_from(self.directory_context(), value)
    }
    pub(crate) fn directory_operand_from(
        mut context: ResourceLocation,
        value: &str,
    ) -> Result<FileTarget, String> {
        if value.starts_with("ssh://")
            || value.starts_with("file://")
            || value.starts_with("container:")
        {
            return FileTarget::parse(value.into()).map_err(|error| error.to_string());
        }
        context.path = context.path.join(value);
        FileTarget::from_location(&context).map_err(|error| error.to_string())
    }
    pub(crate) fn browse_directory(&mut self, operand: &str) -> Result<(), String> {
        let target = if !operand.is_empty() {
            Self::directory_operand_from(self.open_context(), operand)?
        } else if let Some(source) = self.directory() {
            FileTarget::from_location(&source.location).map_err(|error| error.to_string())?
        } else {
            let source = self
                .navigation_source()
                .map_or(self.current(), |(source, _)| source);
            let parent = self
                .doc(source)
                .file_target(&self.cwd)
                .and_then(|target| target.resource_location())
                .and_then(|location| strop_fs::parent(&location))
                .unwrap_or_else(|| ResourceLocation::local(self.cwd.clone()));
            FileTarget::from_location(&parent).map_err(|error| error.to_string())?
        };
        self.push_jump();
        self.request_target(target, OpenIntent::Browse);
        Ok(())
    }
    pub fn reveal_source(&mut self) {
        if self.directory().is_some() {
            return;
        }
        let source = self
            .navigation_source()
            .map_or(self.current(), |(source, _)| source);
        let child = self
            .doc(source)
            .file_target(&self.cwd)
            .and_then(|target| target.resource_location());
        let Some(child) = child else {
            if let Err(error) = self.browse_directory("") {
                self.message = error;
            }
            return;
        };
        let Some(parent) = strop_fs::parent(&child) else {
            self.message = "resource has no parent directory".into();
            return;
        };
        match FileTarget::from_location(&parent) {
            Ok(target) => {
                self.push_jump();
                self.request_target(target, OpenIntent::DirectoryParent { child });
            }
            Err(error) => self.message = error.to_string(),
        }
    }
    pub(crate) fn refresh_directory(&mut self) -> bool {
        let Some(location) = self.directory().map(|source| source.location.clone()) else {
            return false;
        };
        match FileTarget::from_location(&location) {
            Ok(target) => self.request_target(target, OpenIntent::Refresh),
            Err(error) => self.message = error.to_string(),
        }
        true
    }
    pub(crate) fn directory_key(&mut self, key: Key) -> bool {
        if !matches!(key, Key::Enter | Key::Backspace | Key::Char('-')) {
            return false;
        }
        let Some(source) = self.directory() else {
            return false;
        };
        if source.draft.is_some() || !self.buf().readonly {
            return false;
        }
        if source.view_revision != self.buf().revision() {
            self.message = "directory presentation changed; refresh before navigating".into();
            return true;
        }
        let line = self.buf().line_of(self.head());
        let parent = matches!(key, Key::Backspace | Key::Char('-')) || line == 1;
        let (location, intent) = if parent {
            (
                source.parent(),
                OpenIntent::DirectoryParent {
                    child: source.location.clone(),
                },
            )
        } else {
            let entry = source.entry(LineIndex::new(line));
            (
                entry.map(|entry| source.location_of(entry)),
                if entry.is_some_and(|entry| {
                    entry.observation.kind == strop_workspace::EntryKind::Directory
                }) {
                    OpenIntent::Browse
                } else {
                    OpenIntent::Switch { readonly: false }
                },
            )
        };
        match location.map(|location| FileTarget::from_location(&location)) {
            Some(Ok(target)) => {
                self.push_jump();
                self.request_target(target, intent);
            }
            Some(Err(error)) => self.message = error.to_string(),
            None => self.message = "no directory entry at this position".into(),
        }
        true
    }
}
