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
        if self
            .docs
            .get(document)
            .is_some_and(|doc| matches!(doc.source, crate::editor::DocumentSource::Terminal(_)))
        {
            self.message = "terminal buffers have no file write binding; yank text to an ordinary buffer to export it".into();
            return false;
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
            self.message =
                "filesystem operation pending or unconfirmed; verify before saving this binding"
                    .into();
            return false;
        }
        if self
            .docs
            .get(document)
            .is_some_and(|doc| doc.directory_metadata_ref().is_some())
        {
            if self.filename_draft(document).is_some() {
                if target.is_some() || close {
                    self.message = "use :w without a target to review filename changes; filesystem application is explicit".into();
                    return false;
                }
                return match self.prepare_filename_draft(document) {
                    Ok(()) => true,
                    Err(error) => {
                        self.message = error;
                        false
                    }
                };
            }
            self.message = "Directory buffers have no file write binding; use :fs edit".into();
            return false;
        }
        if self.docs.get(document).is_some_and(|doc| {
            matches!(
                doc.source,
                crate::editor::document::DocumentSource::Remote(_)
            )
        }) {
            return self.request_remote_save(document, target, force, close);
        }
        if target
            .as_ref()
            .and_then(|path| path.to_str())
            .is_some_and(|path| path.starts_with("ssh://"))
        {
            self.message = "remote save-as is unsupported; no local fallback".into();
            return false;
        }
        if self
            .filesystem
            .blocks_namespace(&strop_workspace::Filesystem::Local)
        {
            self.message = "local filesystem mutation is pending or unconfirmed; settle or verify its receipt before saving".into();
            return false;
        }
        if self.io.saves.contains_key(&document) {
            self.message = "write already in progress".into();
            return false;
        }
        // An unconfirmed store blocks rewrites: `:w` now verifies the
        // frozen attempt against fresh evidence, never a blind rewrite
        // (0058 WK09 — the write may have committed).
        if self.io.store_attempts.contains_key(&document) {
            return self.request_store_verification(document);
        }
        let Some(buffer) = self.docs.get(document).map(|doc| &doc.buf) else {
            self.message = "write refused: source buffer closed".into();
            return false;
        };
        let revision = buffer.revision();
        let target = target.map(|path| self.cwd.join(path));
        let work = match buffer.prepare_save(target.clone(), force) {
            Ok(work) => work,
            Err(error) => {
                let detail = format!("write failed: {error}");
                self.message = detail.clone();
                self.note_save_failure(
                    document, false, revision, target, force, close, true, detail,
                );
                return false;
            }
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message.clone();
                self.note_save_failure(
                    document,
                    false,
                    revision,
                    target,
                    force,
                    close,
                    true,
                    error.message,
                );
                return false;
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
        self.io.saves.insert(document, ticket.clone());
        self.message = "saving".into();
        match self.tape.request("io.save", &ticket) {
            Ok(false) => return true,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Save(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return false;
            }
        }
        let tx = self.io.tx.clone();
        // The write is a Store intent through the checked kernel.
        let local = self.filesystem.local().clone();
        let plan = {
            let mut plan = work.into_plan();
            // A plain `:w` names the buffer's own path; a CLI-opened
            // missing file keeps the relative name it was given, and the
            // store guard requires an absolute target. Resolve against
            // the editor's cwd (launch-time — no :cd exists) at this
            // admission site: the job reads no process state.
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
        true
    }

    /// Verify a frozen unconfirmed store instead of rewriting: the next `:w` after an unconfirmed outcome reconciles
    /// first. Verification never writes; after `Unchanged` an explicit
    /// fresh `:w` is the user's retry.
    pub(crate) fn request_store_verification(&mut self, document: DocumentId) -> bool {
        let Some(attempt) = self.io.store_attempts.get(&document).cloned() else {
            return false;
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return false;
            }
        };
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
        self.io.saves.insert(document, ticket.clone());
        self.message = "verifying the unconfirmed write; nothing rewrites".into();
        match self.tape.request("io.save", &ticket) {
            Ok(false) => return true,
            Ok(true) => {}
            Err(error) => {
                self.handle_io(IoEvent::Save(Box::new(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                })));
                return false;
            }
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
        true
    }

    /// A committed store receipt — direct or proven by verification —
    /// retires the saved revision.
    pub(super) fn accept_store_receipt(&mut self, key: &SaveKey, receipt: SaveReceipt) {
        self.resolve_save_feedback(key.document, false);
        let Some(document) = self.docs.get_mut(key.document) else {
            self.message = "snapshot written; source buffer closed".into();
            self.finish_save_feedback(key.document);
            return;
        };
        let previous_path = document.buf.path.clone();
        let saved = document.buf.accept_save(receipt);
        if saved {
            // A confirmed save IS the fresh observation: external-change
            // state clears only here or on a guarded reload, never on a
            // stale one.
            document.external_change = false;
        }
        let renamed = previous_path != document.buf.path;
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
    ) {
        use strop_workspace::operation::VerifiedOutcome;
        match verified {
            VerifiedOutcome::Committed(change) => {
                // The kernel proved the intended bytes landed; adopt the
                // observed baseline exactly as a direct commit's receipt.
                let stamp = change
                    .destination_after
                    .and_then(|after| after.modified)
                    .map(super::open::filetime_to_systemtime);
                let canonical = std::fs::canonicalize(&attempt.write_target)
                    .unwrap_or_else(|_| attempt.write_target.clone());
                let receipt = SaveReceipt::from_store(
                    attempt.origin.clone(),
                    attempt.target.clone(),
                    canonical,
                    attempt.revision,
                    stamp,
                );
                self.io.store_attempts.remove(&key.document);
                self.accept_store_receipt(key, receipt);
            }
            VerifiedOutcome::Unchanged => {
                self.io.store_attempts.remove(&key.document);
                self.resolve_save_feedback(key.document, false);
                self.message = "the original is unchanged on disk; :w writes again".into();
            }
            VerifiedOutcome::Unknown { detail } => {
                self.message =
                    format!("write outcome still unconfirmed: {detail}; :w verifies again");
                self.note_unconfirmed_save(
                    key.document,
                    attempt.revision,
                    format!("write outcome still unconfirmed: {detail}; :w verifies again"),
                );
            }
        }
    }

    /// Preserve the formatting stage alongside the actual persistence outcome.
    pub(crate) fn finish_save_feedback(&mut self, document: DocumentId) {
        if let Some(warning) = self.io.format_warnings.remove(&document) {
            self.message
                .push_str(&format!(" — format warning: {warning}"));
        }
        self.finish_batch_write(document);
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
            let canonical =
                std::fs::canonicalize(&write_target).unwrap_or_else(|_| write_target.clone());
            Outcome::Success(SaveOutcome::Written(SaveReceipt::from_store(
                plan.origin.clone(),
                plan.target.clone(),
                canonical,
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
