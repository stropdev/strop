use super::*;
use strop_core::id::LineIndex;
use strop_workspace::Filesystem;
impl Editor {
    pub(crate) fn filesystem_completion_context(
        &self,
        command: &str,
        argument: &str,
    ) -> Result<ResourceLocation, String> {
        if command == "rename" && !argument.contains('/') {
            if let Some(directory) = self.directory() {
                return Ok(directory.location.clone());
            }
            let source = self
                .navigation_source()
                .map_or(self.current(), |(source, _)| source);
            return self
                .doc(source)
                .file_target(&self.cwd)
                .and_then(|target| target.resource_location())
                .and_then(|location| strop_fs::parent(&location))
                .ok_or_else(|| "rename requires a filesystem source".into());
        }
        Ok(self.directory_context())
    }
    pub(crate) fn run_filesystem_ex(&mut self, argument: &str, range: Option<(usize, usize)>) {
        if let Err(error) = self.filesystem_command(argument, range) {
            self.message = error;
        }
    }
    fn filesystem_sources(
        &self,
        range: Option<(usize, usize)>,
    ) -> Result<Vec<ResourceLocation>, String> {
        if let Some(directory) = self.directory() {
            if let Some(draft) = directory.draft.as_ref() {
                let line = self.buf().line_of(self.head());
                let (first, last) = range.unwrap_or((line, line));
                if last.saturating_sub(first) >= strop_fs::batch::STEP_LIMIT {
                    return Err("selection exceeds the 512-row bound".into());
                }
                return (first..=last)
                    .map(|line| {
                        draft
                            .source(line)
                            .map(|source| source.location.clone())
                            .ok_or_else(|| "this filename row has no stored source identity".into())
                    })
                    .collect();
            }
            if directory.stale.is_some() || directory.view_revision != self.buf().revision() {
                return Err(
                    "directory is stale or edited; refresh before preparing operations".into(),
                );
            }
            let mut sources = Vec::new();
            if !directory.marked.is_empty() {
                for (name, before) in &directory.marked {
                    let location = ResourceLocation {
                        filesystem: directory.location.filesystem.clone(),
                        path: directory.location.path.join(name.as_path()),
                    };
                    let entry = directory
                        .entry_index(&location)
                        .map(|index| &directory.entries[index])
                        .ok_or("a marked entry no longer exists; refresh marks")?;
                    if &entry.observation != before {
                        return Err("a marked entry changed; refresh marks before operating".into());
                    }
                    sources.push(location);
                    if sources.len() > strop_fs::batch::STEP_LIMIT {
                        return Err("marked operation exceeds the 512-step bound".into());
                    }
                }
            } else {
                let line = self.buf().line_of(self.head());
                let (first, last) = range.unwrap_or((line, line));
                if last.saturating_sub(first) >= strop_fs::batch::STEP_LIMIT {
                    return Err("selection exceeds the 512-step bound".into());
                }
                for line in first..=last {
                    let entry = directory.entry(LineIndex::new(line)).ok_or(
                        "selection includes a directory heading or parent control, not an entry",
                    )?;
                    sources.push(directory.location_of(entry));
                }
            }
            return Ok(sources);
        }
        let source = self
            .navigation_source()
            .map_or(self.current(), |(source, _)| source);
        self.doc(source)
            .file_target(&self.cwd)
            .and_then(|target| target.resource_location())
            .map(|source| vec![source])
            .ok_or_else(|| "current view has no filesystem source".into())
    }
    fn filesystem_command(
        &mut self,
        argument: &str,
        range: Option<(usize, usize)>,
    ) -> Result<(), String> {
        if argument.is_empty() {
            self.open_filesystem_actions();
            return Ok(());
        }
        let (command, argument) = argument.split_once(' ').unwrap_or((argument, ""));
        match command {
            "search" => {
                if !argument.is_empty() || range.is_some() {
                    return Err(
                        "fs search takes the current Directory scope, not an operand or range"
                            .into(),
                    );
                }
                let root = self
                    .directory()
                    .ok_or("fs search requires a Directory buffer")?
                    .location
                    .clone();
                self.open_search_in(super::super::picker::SearchScope { root }, false);
                return Ok(());
            }
            "edit" => return self.begin_filename_draft(),
            "discard" => return self.discard_filename_draft(),
            "copies" => return self.filename_copy_policy(argument),
            "deletes" => return self.filename_removal_policy(argument),
            "refresh" => {
                if !self.refresh_directory() {
                    return Err("filesystem refresh requires a Directory buffer".into());
                }
                return Ok(());
            }
            "mark" | "unmark" => {
                if self.filename_draft(self.current()).is_some() {
                    return Err("filename drafts review all changed rows; use listing marks outside the draft".into());
                }
                let line = LineIndex::new(self.buf().line_of(self.head()));
                let current = self.current();
                let directory = self
                    .docs
                    .get_mut(current)
                    .and_then(|document| document.directory_metadata_mut())
                    .ok_or("mark requires a Directory buffer")?;
                if command == "unmark" {
                    directory.marked.clear();
                } else if !directory.toggle_mark(line) {
                    return Err("no directory entry at this position".into());
                }
                self.message = format!("{} filesystem entries marked", directory.marked.len());
                return Ok(());
            }
            "path" => {
                let sources = self.filesystem_sources(range)?;
                let mut text = String::new();
                for source in sources {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&source.uri().map_err(|error| error.to_string())?);
                }
                self.set_register(Some('+'), super::super::Register::characterwise(text));
                self.message = "resource URI copied to clipboard register".into();
                return Ok(());
            }
            "operations" => {
                if self.filesystem.history.is_empty() {
                    return Err("no admitted filesystem operations in this session".into());
                }
                let mut text = String::new();
                let mut rows = Vec::new();
                for attempt in &self.filesystem.history {
                    let (body, kinds) = render::receipt(
                        attempt.ticket.request,
                        &attempt.batch,
                        &attempt.receipts,
                        attempt.warning.as_deref(),
                    );
                    text.push_str(&body);
                    rows.extend(kinds);
                }
                let mut buffer = strop_core::Buffer::from_text(&text);
                buffer.name = Some("filesystem operations".into());
                let document = self.open_temporary_output(buffer);
                self.set_filesystem_review_rows(document, rows);
                return Ok(());
            }
            "verify" | "undo" => {
                let (operation, step) = self.filesystem_receipt_key(argument)?;
                return if command == "verify" {
                    self.verify_filesystem_step(operation, step)
                } else {
                    self.recover_filesystem_step(operation, step)
                };
            }
            _ => {}
        }
        if self.filename_draft(self.current()).is_some() {
            return Err("edit filename fields and use :w for their review, or :fs discard before explicit operations".into());
        }
        if matches!(command, "create" | "mkdir") {
            if argument.is_empty() {
                return Err("creation requires an explicit destination".into());
            }
            let destination = self
                .directory_operand(argument)?
                .resource_location()
                .ok_or("creation requires a filesystem destination")?;
            return self.prepare_filesystem(
                vec![OperationIntent {
                    kind: if command == "mkdir" {
                        OperationKind::CreateDirectory
                    } else {
                        OperationKind::CreateFile
                    },
                    source: None,
                    destination: Some(destination),
                    copy_version: CopyVersion::Stored,
                    expected_content: None,
                }],
                None,
            );
        }
        let kind = match command {
            "rename" | "move" => OperationKind::Rename,
            "copy" => OperationKind::Copy,
            "trash" => OperationKind::Trash,
            "remove" => OperationKind::Remove,
            _ => return Err("filesystem actions: create, mkdir, rename, move, copy [stored|buffer], trash, remove, edit, discard, copies, deletes, mark, unmark, refresh, search, path, operations, verify, undo".into()),
        };
        let sources = self.filesystem_sources(range)?;
        let (version, explicit, argument) = if command == "copy" {
            if let Some(destination) = argument.strip_prefix("stored ") {
                (CopyVersion::Stored, true, destination)
            } else if let Some(destination) = argument.strip_prefix("buffer ") {
                (CopyVersion::Buffer, true, destination)
            } else {
                (CopyVersion::Stored, false, argument)
            }
        } else {
            (CopyVersion::Stored, true, argument)
        };
        if kind == OperationKind::Copy && !explicit {
            for source in &sources {
                let target = crate::files::FileTarget::from_location(source)
                    .map_err(|error| error.to_string())?;
                if self
                    .docs
                    .iter()
                    .any(|(_, document)| document.matches_target(&target) && document.buf.dirty)
                {
                    return Err("source has unsaved text; choose :fs copy stored DEST or :fs copy buffer DEST".into());
                }
            }
        }
        let needs_destination = matches!(kind, OperationKind::Rename | OperationKind::Copy);
        if needs_destination && argument.is_empty() {
            return Err("operation requires an explicit destination".into());
        }
        if !needs_destination && !argument.is_empty() {
            return Err("removal operates only on the explicitly selected entries".into());
        }
        let mut intents = Vec::with_capacity(sources.len());
        for source in &sources {
            if matches!(source.filesystem, Filesystem::Container(_)) {
                return Err("container filesystem operations are read-only by policy".into());
            }
            let destination = if needs_destination {
                let mut target = if command == "rename"
                    && !argument.contains('/')
                    && !argument.starts_with("file:")
                    && !argument.starts_with("container:")
                {
                    let parent = strop_fs::parent(source).ok_or("source has no parent")?;
                    ResourceLocation {
                        filesystem: parent.filesystem,
                        path: parent.path.join(argument),
                    }
                } else {
                    self.directory_operand(argument)?
                        .resource_location()
                        .ok_or("destination requires a filesystem namespace")?
                };
                if command == "move"
                    || (command == "copy" && (sources.len() > 1 || argument.ends_with('/')))
                {
                    target
                        .path
                        .push(source.path.file_name().ok_or("source has no basename")?);
                }
                Some(target)
            } else {
                None
            };
            intents.push(OperationIntent {
                kind,
                source: Some(source.clone()),
                destination,
                copy_version: version,
                expected_content: None,
            });
        }
        self.prepare_filesystem(intents, None)
    }
}
