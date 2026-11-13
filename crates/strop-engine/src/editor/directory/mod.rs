//! Directory navigation, view history and owned current-folder filtering.
//! Native effects belong to strop-fs; rows never become path or command text.
mod filter;
mod jobs;
pub(crate) use filter::apply_filter;
#[cfg(test)]
mod tests;
use super::io::{IoEvent, OpenIntent, Opened};
use super::{Directory, Editor};
use crate::files::FileTarget;
use std::collections::HashMap;
use strop_core::frontend_input::Key;
use strop_core::id::{BufferRevision, DocumentId, LineIndex};
use strop_core::worker::{self, CancelReason, Completion, FailureKind, Outcome, Ticket, WorkerId};
use strop_workspace::ResourceLocation;

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
    /// The modeline's muted readout for the folder in view: `27 items`,
    /// `3 of 9 · filter rs`, `22 items · dotfiles hidden`.
    pub fn directory_summary(&self) -> Option<String> {
        let source = self.directory()?;
        let items = |count: usize| {
            if count == 1 {
                "1 item".to_string()
            } else {
                format!("{count} items")
            }
        };
        let mut text = if source.filter.is_empty() {
            items(source.visible.len())
        } else {
            format!(
                "{} of {} · filter {}",
                source.visible.len(),
                source.entries.len(),
                source.filter
            )
        };
        if !source.show_hidden {
            text.push_str(" · dotfiles hidden");
        }
        if source.stale.is_some() {
            text.push_str(" · out of date");
        }
        match source.state {
            strop_workspace::ListingState::Complete => {}
            strop_workspace::ListingState::Limited { .. } => text.push_str(" · listing cut short"),
            strop_workspace::ListingState::Failed { .. } => text.push_str(" · listing incomplete"),
        }
        Some(text)
    }
    /// A folder named the way files are: relative to the working
    /// directory (`crates/strop-engine/`, `strop/` for the root itself),
    /// absolute outside it or on another machine.
    pub fn folder_label(&self, location: &ResourceLocation) -> String {
        let mut label = match location.local_path() {
            Some(path) if path == self.cwd => path.file_name().map_or_else(
                || "/".into(),
                |name| strop_workspace::directory::display_path(name.as_ref()),
            ),
            Some(path) => match path.strip_prefix(&self.cwd) {
                Ok(relative) => strop_workspace::directory::display_path(relative),
                Err(_) => location.label(),
            },
            None => location.label(),
        };
        if !label.ends_with('/') {
            label.push('/');
        }
        label
    }
    pub(crate) fn remember_directory_view(&mut self) {
        let Some(source) = self.directory() else {
            return;
        };
        if self.buf().is_dirty() {
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
                        .unwrap_or(line)
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
        } else {
            self.set_head(0);
        }
    }
    /// The base a relative Ex operand (`:e`, `:sp`, their
    /// completion) resolves against. Local operands follow Vim: the editor's
    /// cwd, even from a local Directory buffer (0088 U2 — browsing never
    /// changes where `:e path` lands). A remote Directory has no cwd in
    /// its namespace, so its own location is the base there; an
    /// ordinary remote file never lends its namespace.
    pub(crate) fn open_context(&self) -> ResourceLocation {
        match self.directory() {
            Some(directory)
                if !matches!(
                    directory.location.filesystem,
                    strop_workspace::Filesystem::Local
                ) =>
            {
                directory.location.clone()
            }
            _ => ResourceLocation::local(self.cwd.clone()),
        }
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
    pub(crate) fn directory_operand_from(
        mut context: ResourceLocation,
        value: &str,
    ) -> Result<FileTarget, String> {
        if value.starts_with("ssh://") || value.starts_with("file://") {
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
    /// `-` and `Enter` in a folder: the parent (cursor on the folder you
    /// left), or the entry under the cursor. Pending edits stay with the
    /// folder's buffer; a row typed fresh is not on disk yet.
    pub(crate) fn directory_key(&mut self, key: Key) -> bool {
        if !matches!(key, Key::Enter | Key::Char('-')) {
            return false;
        }
        let Some(source) = self.directory() else {
            return false;
        };
        let line = self.buf().line_of(self.head());
        let (location, intent) = if key == Key::Char('-') {
            (
                source.parent(),
                OpenIntent::DirectoryParent {
                    child: source.location.clone(),
                },
            )
        } else {
            let entry = match &source.draft {
                // `location`, not the source: right after an apply the
                // row already names its new path while the folder
                // lists again.
                Some(draft) => draft
                    .source(line)
                    .zip(draft.location(line))
                    .map(|(source, location)| (location, source.observation.kind)),
                None => source
                    .entry(LineIndex::new(line))
                    .map(|entry| (source.location_of(entry), entry.observation.kind)),
            };
            let Some((location, kind)) = entry else {
                self.message = if self.buf().line_start(line) == self.buf().line_end(line) {
                    "no entry on this line".into()
                } else {
                    "not on disk yet; :w creates it".into()
                };
                return true;
            };
            let intent = if kind == strop_workspace::EntryKind::Directory {
                OpenIntent::Browse
            } else {
                OpenIntent::Switch { readonly: false }
            };
            (Some(location), intent)
        };
        match location.map(|location| FileTarget::from_location(&location)) {
            Some(Ok(target)) => {
                self.push_jump();
                self.request_target(target, intent);
            }
            Some(Err(error)) => self.message = error.to_string(),
            None => self.message = "this folder has no parent".into(),
        }
        true
    }
    /// `g.`: show or hide dotfiles in this folder.
    pub(crate) fn toggle_directory_hidden(&mut self) {
        if self.directory().is_none() {
            self.message = "g. shows or hides dotfiles in a folder".into();
            return;
        }
        if self.buf().is_dirty() {
            self.message = "write (:w) or drop (:e!) the pending changes first".into();
            return;
        }
        let document = self.current();
        if let Some(source) = self.doc_mut(document).directory_metadata_mut() {
            source.show_hidden = !source.show_hidden;
        }
        if let Err(error) = self.start_directory_task(document, DirectoryTask::Filter, None) {
            self.message = error;
        }
    }
}
