use super::*;
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryGuard {
    pub expected: LocatedObservation,
}
impl RecoveryGuard {
    pub(super) fn accepts(&self, batch: &strop_fs::batch::PreparedBatch) -> bool {
        batch
            .steps
            .iter()
            .filter_map(|step| step.source.as_ref())
            .any(|source| {
                source.location == self.expected.location
                    && source
                        .value
                        .as_ref()
                        .zip(self.expected.value.as_ref())
                        .is_some_and(|(now, before)| now.same_metadata(before))
            })
    }
}
impl Editor {
    pub(super) fn filesystem_receipt_key(
        &self,
        argument: &str,
    ) -> Result<(WorkerId, usize), String> {
        let (operation, step) = argument
            .split_once(' ')
            .ok_or("specify OPERATION STEP from :fs operations")?;
        let id: u64 = operation
            .parse()
            .map_err(|_| "invalid filesystem operation ID")?;
        let step = step
            .parse::<usize>()
            .ok()
            .and_then(|step| step.checked_sub(1))
            .ok_or("step numbers start at 1")?;
        let operation = self
            .filesystem
            .history
            .iter()
            .find(|attempt| attempt.ticket.request.get() == id)
            .map(|attempt| attempt.ticket.request)
            .ok_or("filesystem operation not found")?;
        Ok((operation, step))
    }
    pub(super) fn recover_filesystem_step(
        &mut self,
        operation: WorkerId,
        step: usize,
    ) -> Result<(), String> {
        let attempt = self
            .filesystem
            .history
            .iter()
            .find(|attempt| attempt.ticket.request == operation)
            .ok_or("filesystem operation not found")?;
        let receipt = attempt
            .receipts
            .get(step)
            .ok_or("filesystem step not found")?;
        let StepOutcome::Committed {
            recovery,
            publication,
            destination_after,
            ..
        } = &receipt.outcome
        else {
            return Err("only a confirmed committed step can prepare recovery; verify unconfirmed outcomes first".into());
        };
        let source = receipt.operation.source.as_ref();
        let destination = receipt.operation.destination.as_ref();
        if receipt.operation.intent.kind == OperationKind::Remove {
            return Err(
                "permanent removal has no filesystem undo; detached drafts remain in memory".into(),
            );
        }
        let owned = publication
            .as_ref()
            .ok_or("receipt has no owned publication witness")?;
        let expected_kind = match receipt.operation.intent.kind {
            OperationKind::CreateDirectory => Some(strop_workspace::EntryKind::Directory),
            OperationKind::CreateFile | OperationKind::Copy => {
                Some(strop_workspace::EntryKind::File)
            }
            _ => source
                .and_then(|value| value.value.as_ref())
                .map(|value| value.kind),
        };
        let matches = |value: &&strop_workspace::Observation| {
            owned.matches_metadata(value) && Some(value.kind) == expected_kind
        };
        let expected = attempt
            .publication
            .get(step)
            .and_then(|value| value.as_ref())
            .filter(matches)
            .or_else(|| destination_after.as_ref().filter(matches))
            .cloned();
        let expected_content = owned.content;
        let (kind, from, to) = match receipt.operation.intent.kind {
            OperationKind::Rename => (
                OperationKind::Rename,
                destination.map(|value| value.location.clone()),
                source.map(|value| value.location.clone()),
            ),
            OperationKind::Trash => (
                OperationKind::Restore,
                recovery.clone(),
                source.map(|value| value.location.clone()),
            ),
            OperationKind::Restore => (
                OperationKind::Trash,
                destination.map(|value| value.location.clone()),
                None,
            ),
            OperationKind::CreateFile | OperationKind::CreateDirectory | OperationKind::Copy => {
                if receipt.operation.intent.kind != OperationKind::CreateDirectory
                    && expected_content.is_none()
                {
                    return Err("receipt lacks the intended content witness; recovery cannot infer ownership".into());
                }
                (
                    OperationKind::Remove,
                    destination.map(|value| value.location.clone()),
                    None,
                )
            }
            OperationKind::Remove => return Err("permanent removal has no filesystem undo".into()),
        };
        let from = from.ok_or("receipt has no recoverable source location")?;
        let expected = expected.ok_or(
            "receipt lacks the original publication witness; recovery cannot infer ownership",
        )?;
        let guard = RecoveryGuard {
            expected: LocatedObservation {
                location: from.clone(),
                value: Some(expected),
            },
        };
        self.prepare_filesystem(
            vec![OperationIntent {
                kind,
                source: Some(from),
                destination: to,
                copy_version: CopyVersion::Stored,
                expected_content,
            }],
            Some(guard),
        )
    }
}
