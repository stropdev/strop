use super::super::DocumentSource;
use super::*;
use strop_workspace::Filesystem;
fn intersects(left: &ResourceLocation, right: &ResourceLocation) -> bool {
    left.filesystem == right.filesystem
        && (left.path.starts_with(&right.path) || right.path.starts_with(&left.path))
}
impl Editor {
    pub(super) fn filesystem_admission(
        &self,
        intents: &[OperationIntent],
        own_draft: Option<DocumentId>,
    ) -> Result<(), String> {
        let local_write_pending = self.local_write_pending();
        for intent in intents {
            for location in intent.source.iter().chain(intent.destination.iter()) {
                if location.filesystem == Filesystem::Local && local_write_pending {
                    return Err("local save/save-as is still in progress; wait before preparing filesystem mutations".into());
                }
                if self.filesystem.blocks(location) {
                    return Err(format!("filesystem operation pending or unconfirmed for {}; verify its receipt first", location.label()));
                }
                if matches!(location.filesystem, Filesystem::Container(_)) {
                    return Err("container filesystem operations are read-only by policy".into());
                }
                for key in self.io.open.values() {
                    if key
                        .path
                        .resource_location()
                        .is_some_and(|opened| intersects(&opened, location))
                    {
                        return Err(format!(
                            "an open/refresh is still pending for {}",
                            location.label()
                        ));
                    }
                }
            }
            if let Some(source) = intent.source.as_ref().filter(|_| {
                matches!(
                    intent.kind,
                    OperationKind::Rename
                        | OperationKind::Trash
                        | OperationKind::Remove
                        | OperationKind::Restore
                )
            }) {
                if source.filesystem == Filesystem::Local
                    && (self.cwd.starts_with(&source.path)
                        || self
                            .state_dir
                            .as_ref()
                            .is_some_and(|state| state.starts_with(&source.path)))
                {
                    return Err(format!("{} contains an active workspace/cwd/state anchor; relocation is not authorized", source.label()));
                }
            }
            for (document, entry) in self.docs.iter() {
                let Some(location) = entry
                    .file_target(&self.cwd)
                    .and_then(|target| target.resource_location())
                else {
                    continue;
                };
                let affected = intent
                    .source
                    .iter()
                    .chain(intent.destination.iter())
                    .any(|resource| intersects(resource, &location));
                if affected
                    && own_draft != Some(document)
                    && entry
                        .directory_metadata_ref()
                        .is_some_and(|source| source.draft.is_some())
                {
                    return Err(format!(
                        "{} has an active filename draft; finish or discard it first",
                        location.label()
                    ));
                }
                if affected
                    && (self.io.save_pending_for(document)
                        || matches!(self.lsp_state.after_format.as_ref(),
                            Some(super::super::lsp::state::AfterFormat::Save { document: owner, .. }) if *owner == document)
                        || self.remote_write_blocks_refresh(document))
                {
                    return Err(format!(
                        "save/edit admission/verification is pending for {}",
                        location.label()
                    ));
                }
                if intent.destination.as_ref() == Some(&location) {
                    let vacated = intents.iter().any(|other| {
                        other.source.as_ref() == Some(&location)
                            && matches!(
                                other.kind,
                                OperationKind::Rename
                                    | OperationKind::Trash
                                    | OperationKind::Remove
                                    | OperationKind::Restore
                            )
                    });
                    if !vacated {
                        return Err(format!("destination {} is already owned by an open document, including its unsaved text", location.label()));
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn reconcile_filesystem_step(&mut self, receipt: &StepReceipt) {
        let StepOutcome::Committed {
            destination_after, ..
        } = &receipt.outcome
        else {
            return;
        };
        self.invalidate_filesystem_completions();
        let operation = &receipt.operation;
        let source = operation.source.as_ref().map(|source| &source.location);
        let destination = operation
            .destination
            .as_ref()
            .map(|destination| &destination.location);
        let moving = matches!(
            operation.intent.kind,
            OperationKind::Rename | OperationKind::Restore
        );
        let removing = matches!(
            operation.intent.kind,
            OperationKind::Trash | OperationKind::Remove
        );
        let mut affected = Vec::new();
        if moving || removing {
            if let Some(source) = source {
                for (document, entry) in self.docs.iter() {
                    let Some(location) = entry
                        .file_target(&self.cwd)
                        .and_then(|target| target.resource_location())
                    else {
                        continue;
                    };
                    let logical = operation.intent.source.as_ref().filter(|logical| {
                        logical.filesystem == location.filesystem
                            && location.path.starts_with(&logical.path)
                    });
                    let base = if location.filesystem == source.filesystem
                        && location.path.starts_with(&source.path)
                    {
                        Some(source)
                    } else {
                        logical
                    };
                    if let Some(base) = base {
                        let target = if moving {
                            destination.and_then(|target| location.relocated(base, target))
                        } else {
                            None
                        };
                        affected.push((document, location, target));
                    }
                }
            }
        }
        for (document, old, destination) in &affected {
            self.stop_remote_follow(*document);
            self.cancel_directory_filter(*document);
            self.lsp_close_document(*document);
            self.revoke_git_requests_for(*document);
            self.blame_gutters.remove(document);
            self.analysis
                .forget(super::super::analysis::AnalysisTarget::Document(*document));
            if let Some(destination) = destination {
                if let Filesystem::Remote(endpoint) = &destination.filesystem {
                    if self.doc(*document).remote_metadata().is_some() {
                        match strop_workspace::RemoteFile::from_path(
                            endpoint.clone(),
                            destination.path.clone(),
                        ) {
                            Ok(file) => self.relocate_remote_binding(*document, file),
                            Err(error) => {
                                self.message = format!("relocation binding refused: {error}");
                                continue;
                            }
                        }
                    }
                }
                let directory = self.doc(*document).directory_metadata_ref().is_some();
                if directory {
                    let header = format!("{}\n", destination.label());
                    let end = self.doc(*document).buf.line_start(1);
                    let result = self
                        .doc_mut(*document)
                        .buf
                        .system_edit()
                        .replace(strop_core::Range::charwise(0, end), &header);
                    if let Err(error) = result {
                        self.message = format!("directory header update failed: {error}");
                    }
                    if let Some(entry) = self.docs.get_mut(*document) {
                        if let Some(source) = entry.directory_metadata_mut() {
                            source.location = destination.clone();
                        }
                        let revision = entry.buf.revision();
                        if let Some(source) = entry.directory_metadata_mut() {
                            source.view_revision = revision;
                        }
                        entry.buf.name = Some(format!("directory {}", destination.label()));
                    }
                } else if destination.filesystem == Filesystem::Local {
                    let stamp = if source == Some(old) {
                        destination_after
                            .as_ref()
                            .and_then(|value| value.modified)
                            .and_then(system_time)
                    } else {
                        None
                    };
                    if let Some(entry) = self.docs.get_mut(*document) {
                        entry
                            .buf
                            .relocate_file_binding(destination.path.clone(), stamp);
                    }
                }
            } else {
                self.revoke_remote_write(*document);
                if let Some(entry) = self.docs.get_mut(*document) {
                    entry.buf.detach_file_binding();
                    entry.buf.name = Some(format!("detached {} — original removed", old.label()));
                    entry.buf.readonly = false;
                    entry.buf.dirty = true;
                    entry.syntax_hint = Some(old.path.clone());
                    entry.source = DocumentSource::Scratch;
                }
            }
        }
        self.lsp_retire_remote_servers();
        let documents: Vec<_> = affected.iter().map(|(document, _, _)| *document).collect();
        self.invalidate_filesystem_text_review(&documents);
        let collections: Vec<_> = self
            .collections
            .iter()
            .filter(|(_, collection)| {
                collection
                    .excerpts
                    .iter()
                    .any(|excerpt| documents.contains(&excerpt.source))
            })
            .map(|(id, _)| *id)
            .collect();
        for collection in collections {
            self.collection_render_view(collection);
        }
        let touched: Vec<_> = source.into_iter().chain(destination).collect();
        let previews: Vec<_> = self
            .previews
            .keys()
            .filter(|path| {
                touched.iter().any(|location| {
                    location.filesystem == path.filesystem
                        && (path.path.starts_with(&location.path)
                            || location.path.starts_with(&path.path))
                })
            })
            .cloned()
            .collect();
        for path in previews {
            self.previews.remove(&path);
            if let Some(worker::Load::Running(ticket)) = self.preview_loads.remove(&path) {
                if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                    handle.cancel(worker::CancelReason::Superseded);
                }
            }
            self.analysis
                .forget(super::super::analysis::AnalysisTarget::Preview(path));
        }
        let directories: Vec<_> = self
            .docs
            .iter()
            .filter_map(|(id, entry)| entry.directory_metadata_ref().map(|_| id))
            .collect();
        for id in directories {
            let Some(entry) = self.docs.get_mut(id) else {
                continue;
            };
            if let Some(directory) = entry.directory_metadata_mut() {
                if moving || removing {
                    if let Some(source) = source {
                        directory
                            .record_relocation(source, if moving { destination } else { None });
                    }
                    if operation.intent.source.as_ref() != source {
                        if let Some(logical) = &operation.intent.source {
                            directory.record_relocation(
                                logical,
                                if moving {
                                    operation.intent.destination.as_ref()
                                } else {
                                    None
                                },
                            );
                        }
                    }
                }
                if touched.iter().any(|location| {
                    location.filesystem == directory.location.filesystem
                        && (location.path.parent() == Some(directory.location.path.as_path())
                            || directory.location.path.starts_with(&location.path))
                }) {
                    directory.stale = Some("filesystem changed; refresh required".into());
                }
            }
        }
        self.relocate_directory_history(source, if moving { destination } else { None }, removing);
        if (moving || removing) && operation.intent.source.as_ref() != source {
            self.relocate_directory_history(
                operation.intent.source.as_ref(),
                if moving {
                    operation.intent.destination.as_ref()
                } else {
                    None
                },
                removing,
            );
        }
        for glue in self
            .picker
            .iter_mut()
            .chain(self.retained_search.iter_mut())
        {
            if let Some(context) = glue.search.as_mut() {
                context.refresh_requested = true;
            }
        }
        self.filesystem_picker_changed();
        if !self.docs.is_empty() && documents.contains(&self.current()) {
            self.discover_git();
            self.lsp_maybe_attach();
        }
    }
}
fn system_time(time: strop_workspace::FileTime) -> Option<std::time::SystemTime> {
    let base = if time.seconds >= 0 {
        std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(time.seconds as u64))?
    } else {
        std::time::UNIX_EPOCH
            .checked_sub(std::time::Duration::from_secs(time.seconds.unsigned_abs()))?
    };
    base.checked_add(std::time::Duration::from_nanos(u64::from(time.nanos)))
}
