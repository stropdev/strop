//! Save admission, optional formatting and owned source writes.
use super::*;
use strop_core::worker::{self, FailureKind};

impl Editor {
    pub(crate) fn local_write_pending(&self) -> bool {
        !self.io.saves.is_empty()
            // An unconfirmed store holds its binding until verified.
            || !self.io.store_attempts.is_empty()
            || matches!(self.lsp_state.after_format.as_ref(),
                Some(crate::editor::lsp::state::AfterFormat::Save { document, .. })
                    if self.docs.get(*document).is_some_and(|doc| matches!(doc.source, crate::editor::document::DocumentSource::File)))
    }

    pub fn request_save(&mut self, target: Option<PathBuf>, force: bool, close: bool) {
        if self.directory().is_some() {
            self.request_save_document(self.current(), target, force, close);
            return;
        }
        // auto_format (helix parity): a plain `:w` formats through the
        // language server first; the save chains on the reply. A
        // formatter failure or refusal never holds the save hostage.
        if self.config.auto_format && target.is_none() && self.lsp_format_available() {
            let before = self.lsp_state.navigation;
            self.lsp_format();
            if let Some(request) = self
                .lsp_state
                .navigation
                .filter(|request| Some(*request) != before)
            {
                self.lsp_state.after_format = Some(crate::editor::lsp::state::AfterFormat::Save {
                    document: self.current(),
                    close,
                    force,
                    request,
                });
                return;
            }
        }
        self.request_save_document(self.current(), target, force, close);
    }

    pub(crate) fn request_save_document(
        &mut self,
        document: DocumentId,
        target: Option<PathBuf>,
        force: bool,
        close: bool,
    ) -> bool {
        match self.admit_document_save(document, target, force, close) {
            Ok(_) => true,
            Err(reason) => {
                self.message = reason;
                false
            }
        }
    }

    /// An accepted file write returns its unique completion owner. A
    /// filename draft opens a review instead and has no save receipt.
    pub(crate) fn admit_document_save(
        &mut self,
        document: DocumentId,
        target: Option<PathBuf>,
        force: bool,
        close: bool,
    ) -> Result<Option<WorkerId>, String> {
        if self
            .docs
            .get(document)
            .is_some_and(|doc| matches!(doc.source, crate::editor::DocumentSource::Terminal(_)))
        {
            return Err("terminal buffers have no file write binding; yank text to an ordinary buffer to export it".into());
        }
        let blocked = if let Some(target) = target.as_ref() {
            self.filesystem
                .blocks(&strop_workspace::ResourceLocation::local(
                    self.cwd.join(target),
                ))
        } else {
            self.filesystem_blocks_document(document)
        };
        if blocked {
            return Err(
                "filesystem operation pending or unconfirmed; verify before saving this binding"
                    .into(),
            );
        }
        if self
            .docs
            .get(document)
            .is_some_and(|doc| doc.directory_metadata_ref().is_some())
        {
            if self.filename_draft(document).is_some() {
                if target.is_some() || close {
                    return Err("use :w without a target to review filename changes; filesystem application is explicit".into());
                }
                return self.prepare_filename_draft(document).map(|()| None);
            }
            return Err(crate::editor::filesystem::draft::OVER_LIMIT.into());
        }
        if self.docs.get(document).is_some_and(|doc| {
            matches!(
                doc.source,
                crate::editor::document::DocumentSource::Remote(_)
            )
        }) {
            return self.request_remote_save(document, target, force, close).map(Some);
        }
        if target
            .as_ref()
            .and_then(|path| path.to_str())
            .is_some_and(|path| path.starts_with("ssh://"))
        {
            return Err("remote save-as is unsupported; no local fallback".into());
        }
        if self
            .filesystem
            .blocks_namespace(&strop_workspace::Filesystem::Local)
        {
            return Err("local filesystem mutation is pending or unconfirmed; settle or verify its receipt before saving".into());
        }
        if self.io.saves.contains_key(&document) {
            return Err("write already in progress".into());
        }
        // An unconfirmed store blocks rewrites: `:w` now verifies the
        // frozen attempt against fresh evidence, never a blind rewrite
        // (0058 WK09 — the write may have committed).
        if self.io.store_attempts.contains_key(&document) {
            return self.request_store_verification(document).map(Some);
        }
        let Some(buffer) = self.docs.get(document).map(|doc| &doc.buf) else {
            return Err("write refused: source buffer closed".into());
        };
        let revision = buffer.revision();
        let target = target.map(|path| self.cwd.join(path));
        let work = match buffer.prepare_save(target.clone(), force) {
            Ok(work) => work,
            Err(error) => {
                let detail = format!("write failed: {error}");
                self.note_save_failure(
                    document, false, revision, target, force, close, true, detail.clone(),
                );
                return Err(detail);
            }
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.note_save_failure(
                    document,
                    false,
                    revision,
                    target,
                    force,
                    close,
                    true,
                    error.message.clone(),
                );
                return Err(error.message);
            }
        };
        let focus = self.focus_epoch;
        let ticket = Ticket {
            request,
            key: SaveKey {
                document,
                revision,
                focus,
                close,
                target,
                force,
            },
        };
        self.message = "saving".into();
        if !self.register_save_ticket(&ticket)? {
            return Ok(Some(request));
        }
        let tx = self.io.tx.clone();
        // The write is a Store intent through the checked kernel.
        let local = self.filesystem.local().clone();
        let plan = {
            let mut plan = work.into_plan();
            // A plain `:w` names the buffer's own path; a CLI-opened
            // missing file keeps the relative name it was given, and the
            // store guard requires an absolute target. Resolve against
            // the editor's cwd at this admission site: the job reads
            // no process state. (`:cd` makes such names absolute first.)
            // `origin` stays untouched — save acceptance keys on it.
            if !plan.target.is_absolute() {
                plan.target = self.cwd.join(&plan.target);
            }
            plan
        };
        let handle = worker::spawn(
            "strop-save",
            move |outcome| {
                let _ = tx.send(IoEvent::Save(Box::new(Completion { ticket, outcome })));
            },
            move |token| store_document(plan, &local, close, focus, &token),
        );
        self.worker_handles.insert(request, handle);
        Ok(Some(request))
    }

    /// Verify a frozen unconfirmed store instead of rewriting: the next `:w` after an unconfirmed outcome reconciles
    /// first. Verification never writes; after `Unchanged` an explicit
    /// fresh `:w` is the user's retry.
    pub(crate) fn request_store_verification(&mut self, document: DocumentId) -> Result<WorkerId, String> {
        let attempt = self.io.store_attempts.get(&document).cloned()
            .ok_or("no unconfirmed write to verify")?;
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let ticket = Ticket {
            request,
            key: SaveKey {
                document,
                revision: attempt.revision,
                focus: self.focus_epoch,
                close: attempt.close,
                target: Some(attempt.target.clone()),
                force: attempt.force,
            },
        };
        self.message = "verifying the unconfirmed write; nothing rewrites".into();
        if !self.register_save_ticket(&ticket)? {
            return Ok(request);
        }
        let tx = self.io.tx.clone();
        let local = self.filesystem.local().clone();
        let handle = worker::spawn(
            "strop-save-verify",
            move |outcome| {
                let _ = tx.send(IoEvent::Save(Box::new(Completion { ticket, outcome })));
            },
            move |token| match local.verify_recovered(&token, &attempt.receipt) {
                Ok(verified) => Outcome::Success(SaveOutcome::Verified {
                    attempt: Box::new(attempt),
                    verified,
                }),
                Err(error)
                    if error.kind == strop_workspace::operation::FsFailureKind::Cancelled =>
                {
                    Outcome::Cancelled(worker::CancelReason::Dismissed)
                }
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
        Ok(request)
    }

    /// Replay may suppress a launch or fail it synchronously. Only a
    /// request that still has a future receipt is admitted to a batch.
    fn register_save_ticket(&mut self, ticket: &Ticket<SaveKey>) -> Result<bool, String> {
        self.io.saves.insert(ticket.key.document, ticket.clone());
        self.tape.request("io.save", ticket).map_err(|error| {
            self.handle_io(IoEvent::Save(Box::new(Completion {
                ticket: ticket.clone(),
                outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
            })));
            format!("write failed: {error}")
        })
    }

    /// A committed store receipt — direct or proven by verification —
    /// retires the saved revision.
    pub(super) fn accept_store_receipt(&mut self, key: &SaveKey, receipt: SaveReceipt) {
        self.resolve_save_feedback(key.document, false);
        let Some(document) = self.docs.get_mut(key.document) else {
            self.message = "snapshot written; source buffer closed".into();
            return;
        };
        let previous_path = document.buf.path.clone();
        let saved = document.buf.accept_save(receipt);
        document.redetect_syntax();
        if saved {
            // A confirmed save IS the fresh observation: external-change
            // state clears only here or on a guarded reload, never on a
            // stale one.
            document.external_change = false;
        }
        let renamed = previous_path != document.buf.path;
        if saved {
            self.git_work_saved(key.document);
        }
        if renamed {
            self.lsp_close_document(key.document);
            if !self.docs.is_empty() && self.current() == key.document {
                self.lsp_attach_document(self.current());
            }
        }
        self.message = if saved {
            "written"
        } else {
            "snapshot written; newer edits remain unsaved"
        }
        .into();
        self.request_session_save();
        self.recovery_note_saved();
        if saved
            && key.close
            && !self.docs.is_empty()
            && self.current() == key.document
            && self.focus_epoch == key.focus
        {
            self.close_pane_or_buffer(false);
        }
    }

    /// Reconcile one verified store attempt: a committed
    /// outcome adopts the observed baseline and retires the revision once;
    /// unchanged preserves dirty text for an explicit retry; unknown stays
    /// frozen and keeps blocking rewrites.
    pub(super) fn store_verified(
        &mut self,
        key: &SaveKey,
        attempt: StoreAttempt,
        verified: strop_workspace::operation::VerifiedOutcome,
    ) -> Result<(), String> {
        use strop_workspace::operation::VerifiedOutcome;
        match verified {
            VerifiedOutcome::Committed(change) => {
                // The kernel proved the intended bytes landed; adopt the
                // observed baseline exactly as a direct commit's receipt.
                let stamp = change
                    .destination_after
                    .and_then(|after| after.modified)
                    .map(super::open::filetime_to_systemtime);
                let receipt = SaveReceipt::from_store(
                    attempt.origin.clone(),
                    attempt.target.clone(),
                    attempt.canonical.clone(),
                    attempt.revision,
                    stamp,
                );
                self.io.store_attempts.remove(&key.document);
                self.accept_store_receipt(key, receipt);
                Ok(())
            }
            VerifiedOutcome::Unchanged => {
                self.io.store_attempts.remove(&key.document);
                self.resolve_save_feedback(key.document, false);
                Err("the original is unchanged on disk; :w writes again".into())
            }
            VerifiedOutcome::Unknown { detail } => {
                let reason = format!("write outcome still unconfirmed: {detail}; :w verifies again");
                self.note_unconfirmed_save(key.document, attempt.revision, reason.clone());
                Err(reason)
            }
        }
    }

    /// Preserve the formatting stage alongside the actual persistence outcome.
    pub(crate) fn finish_save_feedback(
        &mut self,
        request: WorkerId,
        document: DocumentId,
        result: Result<(), String>,
    ) {
        let warning = self.io.format_warnings.remove(&document);
        if self.finish_batch_write(request, result.as_ref().err().map(String::as_str)) {
            return;
        }
        if let Err(reason) = result {
            self.message = reason;
        }
        if let Some(warning) = warning {
            self.message
                .push_str(&format!(" — format warning: {warning}"));
        }
    }
}

/// One local document save: prepare the Store intent, apply the frozen
/// content and classify the receipt — committed, refused, cancelled, or
/// unconfirmed with frozen evidence retained. The kernel does the
/// filesystem work.
fn store_document(
    plan: strop_core::SavePlan,
    local: &crate::editor::namespace::LocalFs,
    close: bool,
    focus: u64,
    token: &worker::CancelToken,
) -> Outcome<SaveOutcome> {
    use sha2::Digest;
    use strop_workspace::operation::{
        FsFailureKind, OperationIntent, OperationKind, StepOutcome, StorePolicy,
    };
    // Write-path parity with the in-process writer: a plain `:w` resolves
    // the target's canonical spelling; save-as writes the name as given.
    let write_target = if plan.new_name {
        plan.target.clone()
    } else {
        std::fs::canonicalize(&plan.target).unwrap_or_else(|_| plan.target.clone())
    };
    let mut bytes = Vec::with_capacity(plan.text.len_bytes());
    for chunk in plan.text.chunks() {
        bytes.extend_from_slice(chunk.as_bytes());
    }
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    let intent = OperationIntent {
        kind: OperationKind::Store,
        source: None,
        destination: Some(strop_workspace::ResourceLocation::local(
            write_target.clone(),
        )),
        copy_version: strop_workspace::operation::CopyVersion::Stored,
        expected_content: Some(digest),
        store: Some(StorePolicy {
            baseline: plan.baseline.map(super::open::systemtime_to_filetime),
            baseline_object: None,
            baseline_attributes: None,
            force: plan.force,
            expect_absent: plan.new_name,
            displayed: None,
        }),
    };
    let operation = match local.prepare_store(token, intent) {
        Ok(operation) => operation,
        Err(error) if error.kind == FsFailureKind::Cancelled => {
            return Outcome::Cancelled(worker::CancelReason::Dismissed);
        }
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    let attempt = |receipt: strop_workspace::operation::StepReceipt| {
        Box::new(StoreAttempt {
            revision: plan.revision,
            origin: plan.origin.clone(),
            target: plan.target.clone(),
            write_target: write_target.clone(),
            canonical: canonical_spelling(&write_target),
            close,
            force: plan.force,
            focus,
            receipt,
        })
    };
    let receipt = local.apply_store(token, operation, plan.text.clone());
    match &receipt.outcome {
        StepOutcome::Committed {
            destination_after,
            publication,
            ..
        } => {
            // The receipt must name the intended bytes (the helper's
            // receipt check), else the outcome is honestly unconfirmed.
            if (*publication).and_then(|witness| witness.content) != Some(digest) {
                return Outcome::Success(SaveOutcome::Unconfirmed {
                    detail: "receipt does not match the intended bytes and metadata".into(),
                    attempt: attempt(receipt),
                });
            }
            let stamp = destination_after
                .as_ref()
                .and_then(|after| after.modified)
                .map(super::open::filetime_to_systemtime);
            Outcome::Success(SaveOutcome::Written(SaveReceipt::from_store(
                plan.origin.clone(),
                plan.target.clone(),
                canonical_spelling(&write_target),
                plan.revision,
                stamp,
            )))
        }
        StepOutcome::Refused(failure) => Outcome::failed(FailureKind::Io, failure.to_string()),
        StepOutcome::Cancelled { .. } => Outcome::Cancelled(worker::CancelReason::Dismissed),
        StepOutcome::Unconfirmed { detail, .. } => Outcome::Success(SaveOutcome::Unconfirmed {
            detail: detail.clone(),
            attempt: attempt(receipt),
        }),
    }
}

/// `path` with its symlinks resolved: the file's own when it exists,
/// else its directory's with the name kept (a store that is not yet
/// confirmed), else `path` as given. Worker side only.
fn canonical_spelling(path: &std::path::Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}
