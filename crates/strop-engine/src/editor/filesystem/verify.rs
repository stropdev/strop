use super::*;

impl Editor {
    pub(super) fn verify_filesystem_step(
        &mut self,
        operation: WorkerId,
        step: usize,
    ) -> Result<(), String> {
        if self.filesystem.pending() {
            return Err("another filesystem request is still running".into());
        }
        let receipt = self
            .filesystem
            .history
            .iter()
            .find(|attempt| attempt.ticket.request == operation)
            .and_then(|attempt| attempt.receipts.get(step))
            .cloned()
            .ok_or("filesystem receipt not found")?;
        let request = self.worker_ids.allocate().map_err(|error| error.message)?;
        let ticket = Ticket {
            request,
            key: VerifyKey { operation, step },
        };
        self.filesystem.verifying = Some(ticket.clone());
        match self.tape.request("filesystem.verify", &ticket) {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(error) => {
                self.filesystem_verified(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return Ok(());
            }
        }
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-fs-verify",
            move |outcome| {
                let _ = tx.send(IoEvent::Filesystem(Box::new(FsEvent::Verified(Box::new(
                    Completion { ticket, outcome },
                )))));
            },
            move |token| match strop_fs::batch::verify(&receipt, &token) {
                Ok(result) => Outcome::Success(result),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
        self.message = "verifying filesystem outcome; no mutation will be retried".into();
        Ok(())
    }

    pub(super) fn filesystem_verified(
        &mut self,
        completion: Completion<VerifyKey, VerifiedOutcome>,
    ) {
        if self.filesystem.verifying.as_ref() != Some(&completion.ticket) {
            return;
        }
        self.filesystem.verifying = None;
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        let Some(index) = self
            .filesystem
            .history
            .iter()
            .position(|attempt| attempt.ticket.request == key.operation)
        else {
            return;
        };
        let Some(receipt) = self.filesystem.history[index].receipts.get_mut(key.step) else {
            return;
        };
        if !receipt.outcome.is_unconfirmed() {
            self.message = match completion.outcome {
                Outcome::Success(VerifiedOutcome::Committed(_)) => {
                    "filesystem outcome verified against current names".into()
                }
                Outcome::Success(VerifiedOutcome::Unchanged) => {
                    "current names match the before-state; historical receipt retained".into()
                }
                Outcome::Success(VerifiedOutcome::Unknown { detail }) => detail,
                Outcome::Failed { failure, .. } => failure.message,
                Outcome::Cancelled(_) => "filesystem verification cancelled".into(),
            };
            return;
        }
        match completion.outcome {
            Outcome::Success(VerifiedOutcome::Unchanged) => {
                let detail = match &receipt.outcome {
                    StepOutcome::Unconfirmed { detail, .. } => detail.as_str(),
                    _ => "",
                };
                receipt.outcome = StepOutcome::Refused(FsFailure::new(
                    FsFailureKind::Conflict,
                    format!(
                        "verification observed the unchanged before-state; prior details: {detail}"
                    ),
                ));
            }
            Outcome::Success(VerifiedOutcome::Committed(change)) => {
                let strop_workspace::operation::VerifiedChange {
                    source_after,
                    destination_after,
                } = *change;
                let (recovery, publication, warnings) = match &receipt.outcome {
                    StepOutcome::Unconfirmed {
                        recovery,
                        publication,
                        detail,
                        ..
                    } => (
                        recovery.clone(),
                        *publication,
                        vec![format!("verified after: {detail}")],
                    ),
                    _ => (None, None, Vec::new()),
                };
                receipt.outcome = StepOutcome::Committed {
                    source_after,
                    destination_after,
                    recovery,
                    warnings,
                    publication,
                };
                let receipt = receipt.clone();
                self.reconcile_filesystem_step(&receipt);
            }
            Outcome::Success(VerifiedOutcome::Unknown { detail }) => {
                self.message = detail;
                return;
            }
            Outcome::Failed { failure, .. } => {
                self.message = failure.message;
                return;
            }
            Outcome::Cancelled(_) => {
                self.message =
                    "filesystem verification cancelled; outcome remains unconfirmed".into();
                return;
            }
        }
        self.publish_filesystem_receipt(index);
        self.finish_filesystem_draft_attempt(index);
    }
}
