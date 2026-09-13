use super::*;
use crate::editor::directory::DirectoryTask;
impl Editor {
    pub fn filename_draft(&self, document: DocumentId) -> Option<&Draft> {
        self.docs
            .get(document)?
            .directory_metadata_ref()?
            .draft
            .as_ref()
    }
    pub(crate) fn sync_filename_draft(&mut self, document: DocumentId) {
        let Some(doc) = self.docs.get_mut(document) else {
            return;
        };
        let super::super::super::DocumentSource::Directory(source) = &mut doc.source else {
            return;
        };
        let Some(draft) = source.draft.as_mut() else {
            return;
        };
        draft.sync(&doc.buf);
    }
    pub(crate) fn begin_filename_draft(&mut self) -> Result<(), String> {
        let source = self
            .directory()
            .ok_or("Edit names requires a Directory; use Space e or :browse first")?;
        if matches!(
            source.location.filesystem,
            strop_workspace::Filesystem::Container(_)
        ) {
            return Err("container filename drafts are unavailable: container mutations are read-only by policy".into());
        }
        if source.stale.is_some() {
            return Err("refresh the stale Directory before editing names".into());
        }
        if self.filesystem.blocks(&source.location) {
            return Err("filesystem operations must settle or verify before editing names".into());
        }
        self.start_directory_task(self.current(), DirectoryTask::EditNames, None)
    }
    pub(crate) fn discard_filename_draft(&mut self) -> Result<(), String> {
        let document = self.current();
        let phase = self
            .filename_draft(document)
            .ok_or("no filename draft is active")?
            .phase;
        if matches!(phase, Phase::Applying(_) | Phase::Unconfirmed(_)) {
            return Err(
                "admitted filesystem outcomes must settle or verify before discarding the draft"
                    .into(),
            );
        }
        if self.filesystem.pending.as_ref().is_some_and(|proposal| {
            proposal.ticket.key.origin == document && proposal.ticket.key.draft.is_some()
        }) {
            self.retire_filesystem_review("filename draft discarded");
        }
        if self
            .filesystem
            .preparing
            .as_ref()
            .is_some_and(|ticket| ticket.key.origin == document && ticket.key.draft.is_some())
        {
            if let Some(ticket) = self.filesystem.preparing.take() {
                if let Some(handle) = self.worker_handles.remove(&ticket.request) {
                    handle.cancel(worker::CancelReason::Dismissed);
                }
            }
            self.filesystem.preparing_copies.clear();
        }
        if let Some(doc) = self.docs.get_mut(document) {
            if let Some(draft) = doc
                .directory_metadata_mut()
                .and_then(|source| source.draft.as_mut())
            {
                draft.phase = Phase::Reloading;
            }
            doc.buf.readonly = true;
            doc.buf.dirty = false;
        }
        self.start_directory_task(document, DirectoryTask::Reload, None)
    }
    pub(crate) fn filename_copy_policy(&mut self, argument: &str) -> Result<(), String> {
        let policy = match argument {
            "stored" => CopyVersion::Stored,
            "buffer" => CopyVersion::Buffer,
            _ => return Err("use :fs copies stored or :fs copies buffer".into()),
        };
        let document = self.current();
        let draft = self
            .docs
            .get_mut(document)
            .and_then(|doc| doc.directory_metadata_mut())
            .and_then(|source| source.draft.as_mut())
            .ok_or("copy policy requires a filename draft")?;
        if !draft.editable() {
            return Err("filename draft is waiting for an admitted operation".into());
        }
        draft.intent_epoch = draft
            .intent_epoch
            .checked_add(1)
            .ok_or("filename intent identity exhausted")?;
        draft.copy_version = Some(policy);
        self.message = format!("filename draft copies use {policy:?} contents");
        Ok(())
    }
    pub(crate) fn filename_removal_policy(&mut self, argument: &str) -> Result<(), String> {
        let policy = match argument {
            "trash" => Removal::Trash,
            "permanent" => Removal::Permanent,
            _ => return Err("use :fs deletes trash or :fs deletes permanent".into()),
        };
        let document = self.current();
        let draft = self
            .docs
            .get_mut(document)
            .and_then(|doc| doc.directory_metadata_mut())
            .and_then(|source| source.draft.as_mut())
            .ok_or("deletion policy requires a filename draft")?;
        if !draft.editable() {
            return Err("filename draft is waiting for an admitted operation".into());
        }
        if policy == Removal::Trash && draft.root.filesystem != strop_workspace::Filesystem::Local {
            return Err(
                "remote Trash is unavailable; permanent removal must be chosen explicitly".into(),
            );
        }
        draft.intent_epoch = draft
            .intent_epoch
            .checked_add(1)
            .ok_or("filename intent identity exhausted")?;
        draft.removal = policy;
        self.message = format!("filename draft deletions use {policy:?}");
        Ok(())
    }
    pub(crate) fn prepare_filename_draft(&mut self, document: DocumentId) -> Result<(), String> {
        let draft = self
            .filename_draft(document)
            .cloned()
            .ok_or("no filename draft is active")?;
        if !draft.editable() {
            return Err("filename draft is waiting for its admitted operation".into());
        }
        let mut copies = HashMap::new();
        let mut seen = std::collections::HashSet::new();
        for row in &draft.geometry.rows {
            let Some(Origin::Copy(source)) = &row.origin else {
                continue;
            };
            if !seen.insert(source.location.clone()) {
                continue;
            }
            let target = crate::files::FileTarget::from_location(&source.location)
                .map_err(|error| error.to_string())?;
            let open = self
                .docs
                .iter()
                .find_map(|(_, doc)| doc.matches_target(&target).then_some(doc));
            if draft.copy_version.is_none() && open.is_some_and(|doc| doc.buf.dirty) {
                return Err("draft copies include unsaved sources; choose :fs copies stored or :fs copies buffer before :w".into());
            }
            if draft.copy_version == Some(CopyVersion::Buffer) {
                let open = open.ok_or(
                    "copy current buffer requires that source to be open; choose stored otherwise",
                )?;
                copies.insert(source.location.clone(), open.buf.snapshot());
            }
        }
        let key = FsKey {
            origin: document,
            revision: self.doc(document).buf.revision(),
            focus: self.focus_epoch,
            open_created: false,
            intents: Arc::new(Vec::new()),
            recovery: None,
            draft: Some(Stamp {
                id: draft.id,
                intent: draft.intent_epoch,
                root: draft.root.clone(),
            }),
        };
        let text = self.doc(document).buf.snapshot();
        let environment = self.filesystem.environment.clone();
        self.start_filesystem_preparation(key, copies, move |token| {
            let work = || -> Result<PreparedFilesystem, String> {
                let compiled = draft.compile(&text, &token)?;
                let batch = strop_fs::batch::prepare(&compiled.intents, &environment, &token).map_err(|error| error.to_string())?;
                for step in &batch.steps {
                    let Some(source) = step.source.as_ref() else { continue };
                    if let Some(expected) = compiled.sources.iter().find(|expected| step.intent.source.as_ref() == Some(&expected.location)) {
                        if !expected.value.as_ref().zip(source.value.as_ref()).is_some_and(|(before, now)| compile::matches_observed(before, now)) {
                            return Err(format!("filename source changed since the captured listing: {}; draft retained", expected.location.label()));
                        }
                    }
                }
                let draft_targets = compiled.targets.into_iter().map(|(row, target)| {
                    let resolved = batch.steps.iter().find(|step| step.intent.destination.as_ref() == Some(&target))
                        .and_then(|step| step.destination.as_ref()).map(|destination| destination.location.clone()).unwrap_or(target);
                    (row, resolved)
                }).collect();
                Ok(PreparedFilesystem { batch, draft_targets })
            };
            match work() {
                Ok(batch) => Outcome::Success(batch),
                Err(_) if token.is_cancelled() => Outcome::Cancelled(worker::CancelReason::Superseded),
                Err(error) => Outcome::failed(FailureKind::InvalidInput, error),
            }
        })
    }
    pub(crate) fn filename_draft_fresh(
        &self,
        document: DocumentId,
        revision: BufferRevision,
        stamp: &Stamp,
    ) -> bool {
        self.docs.get(document).is_some_and(|doc| {
            doc.buf.revision() == revision
                && doc.directory_metadata_ref().is_some_and(|source| {
                    source.location == stamp.root
                        && source.draft.as_ref().is_some_and(|draft| {
                            draft.id == stamp.id
                                && draft.intent_epoch == stamp.intent
                                && draft.editable()
                        })
                })
        })
    }
    pub(crate) fn freeze_filename_draft(
        &mut self,
        document: DocumentId,
        stamp: &Stamp,
        operation: WorkerId,
    ) {
        if let Some(doc) = self.docs.get_mut(document) {
            if let Some(draft) = doc
                .directory_metadata_mut()
                .and_then(|source| source.draft.as_mut())
                .filter(|draft| draft.id == stamp.id)
            {
                draft.phase = Phase::Applying(operation);
                doc.buf.readonly = true;
            }
        }
    }
    pub(crate) fn finish_filename_draft(
        &mut self,
        document: DocumentId,
        stamp: &Stamp,
        operation: WorkerId,
        committed: bool,
        unconfirmed: bool,
    ) {
        let published: std::collections::HashSet<_> = self
            .filesystem
            .history
            .iter()
            .find(|attempt| attempt.ticket.request == operation)
            .into_iter()
            .flat_map(|attempt| &attempt.receipts)
            .filter(|receipt| receipt.outcome.is_committed())
            .filter_map(|receipt| {
                receipt
                    .operation
                    .destination
                    .as_ref()
                    .map(|destination| destination.location.clone())
            })
            .collect();
        let Some(doc) = self.docs.get_mut(document) else {
            return;
        };
        let Some(draft) = doc
            .directory_metadata_mut()
            .and_then(|source| source.draft.as_mut())
            .filter(|draft| draft.id == stamp.id)
        else {
            return;
        };
        draft.phase = if unconfirmed {
            Phase::Unconfirmed(operation)
        } else if committed {
            Phase::Reloading
        } else {
            Phase::Editing
        };
        if committed && !unconfirmed {
            let originals: HashMap<_, _> = draft
                .geometry
                .rows
                .iter()
                .filter_map(|row| {
                    if let Some(Origin::Original(index)) = &row.origin {
                        draft
                            .base
                            .get(*index)
                            .map(|source| (row.id, source.location.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            draft.targets.retain(|row, location| {
                published.contains(location) || originals.get(row) == Some(location)
            });
        }
        doc.buf.readonly = unconfirmed || committed;
        if committed && !unconfirmed {
            doc.buf.dirty = false;
            if let Err(error) = self.start_directory_task(document, DirectoryTask::Reload, None) {
                self.message = error;
            }
        }
    }
}
