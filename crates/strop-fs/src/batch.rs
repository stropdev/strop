//! Owned namespace dispatch and dependency ordering. Every generated parent is a
//! visible prepared step; nothing executes until its complete plan is approved.
use crate::Environment;
use strop_core::worker::CancelToken;
use strop_workspace::operation::*;
use strop_workspace::{Filesystem, ResourceLocation};

pub const STEP_LIMIT: usize = 512;
const PARENT_LIMIT: usize = 128;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreparedBatch {
    pub steps: Vec<PreparedOperation>,
    pub refused: Vec<OperationRefusal>,
}

pub fn prepare_one(
    intent: &OperationIntent,
    allow_occupied: bool,
    environment: &Environment,
    token: &CancelToken,
) -> Result<PreparedOperation, FsFailure> {
    let location = intent
        .location()
        .ok_or_else(|| FsFailure::new(FsFailureKind::InvalidPath, "operation has no resource"))?;
    if intent
        .source
        .iter()
        .chain(intent.destination.iter())
        .any(|other| other.filesystem != location.filesystem)
    {
        return Err(FsFailure::new(
            FsFailureKind::Unsupported,
            "cross-namespace transfers require a separate transfer contract",
        ));
    }
    match location.filesystem {
        Filesystem::Local => crate::local::prepare(intent, allow_occupied, environment, token),
        Filesystem::Remote(_) => strop_remote::filesystem::prepare(intent, allow_occupied, token),
        Filesystem::Container(_) => Err(FsFailure::new(
            FsFailureKind::Unsupported,
            "container filesystem operations are read-only by policy",
        )),
    }
}

pub fn prepare(
    intents: &[OperationIntent],
    environment: &Environment,
    token: &CancelToken,
) -> Result<PreparedBatch, FsFailure> {
    let mut result = PreparedBatch {
        steps: Vec::new(),
        refused: Vec::new(),
    };
    if intents.len() > STEP_LIMIT {
        return Err(FsFailure::new(
            FsFailureKind::Incomplete,
            "operation batch exceeds the 512-step bound",
        ));
    }
    for intent in intents {
        match prepare_one(intent, true, environment, token) {
            Ok(operation) => result.steps.push(operation),
            Err(failure) => result.refused.push(OperationRefusal {
                intent: intent.clone(),
                failure,
            }),
        }
    }
    let mut index = 0;
    while index < result.steps.len() {
        let missing: Vec<_> = result.steps[index]
            .parents
            .iter()
            .filter(|parent| parent.value.is_none())
            .map(|parent| parent.location.clone())
            .collect();
        for parent in missing {
            if result.steps.iter().any(|step| {
                step.intent.kind == OperationKind::CreateDirectory
                    && step
                        .destination
                        .as_ref()
                        .is_some_and(|destination| destination.location == parent)
            }) {
                continue;
            }
            if result.steps.len() >= STEP_LIMIT || parent.path.components().count() > PARENT_LIMIT {
                refuse_all(
                    &mut result,
                    FsFailure::new(
                        FsFailureKind::Incomplete,
                        "reviewed parent chain exceeds the operation bound",
                    ),
                );
                return Ok(result);
            }
            let intent = OperationIntent {
                kind: OperationKind::CreateDirectory,
                source: None,
                destination: Some(parent),
                copy_version: CopyVersion::Stored,
                expected_content: None,
            };
            match prepare_one(&intent, false, environment, token) {
                Ok(parent) => result.steps.push(parent),
                Err(failure) => {
                    refuse_all(&mut result, failure);
                    return Ok(result);
                }
            }
        }
        index += 1;
    }
    if let Err(failure) = dependencies(&mut result.steps) {
        refuse_all(&mut result, failure);
    }
    Ok(result)
}

fn refuse_all(batch: &mut PreparedBatch, failure: FsFailure) {
    batch
        .refused
        .extend(batch.steps.drain(..).map(|operation| OperationRefusal {
            intent: operation.intent,
            failure: failure.clone(),
        }));
}
fn source(operation: &PreparedOperation) -> Option<&ResourceLocation> {
    operation.source.as_ref().map(|value| &value.location)
}
fn destination(operation: &PreparedOperation) -> Option<&ResourceLocation> {
    operation.destination.as_ref().map(|value| &value.location)
}
fn vacates(operation: &PreparedOperation) -> bool {
    matches!(
        operation.intent.kind,
        OperationKind::Rename
            | OperationKind::Trash
            | OperationKind::Remove
            | OperationKind::Restore
    )
}
fn conflict(detail: &str) -> FsFailure {
    FsFailure::new(FsFailureKind::Conflict, detail)
}

fn dependencies(steps: &mut Vec<PreparedOperation>) -> Result<(), FsFailure> {
    for (index, step) in steps.iter().enumerate() {
        for other in &steps[..index] {
            if destination(step).is_some() && destination(step) == destination(other) {
                return Err(conflict("multiple steps own one destination"));
            }
            if source(step).is_some()
                && source(step) == source(other)
                && (vacates(step) || vacates(other))
            {
                return Err(conflict("multiple conflicting operations own one source"));
            }
            for (parent, child) in [(step, other), (other, step)] {
                if let Some(parent_source) = parent.source.as_ref().filter(|source| {
                    source
                        .value
                        .as_ref()
                        .is_some_and(|value| value.kind == strop_workspace::EntryKind::Directory)
                }) {
                    if child
                        .source
                        .iter()
                        .chain(child.destination.iter())
                        .any(|child| {
                            child.location.filesystem == parent_source.location.filesystem
                                && child.location.path != parent_source.location.path
                                && child
                                    .location
                                    .path
                                    .starts_with(&parent_source.location.path)
                        })
                    {
                        return Err(conflict(
                            "parent/child operation overlap requires separate reviews",
                        ));
                    }
                }
            }
        }
    }
    let mut edges = vec![Vec::new(); steps.len()];
    for (index, step) in steps.iter().enumerate() {
        for parent in step.parents.iter().filter(|parent| parent.value.is_none()) {
            let dependency = steps
                .iter()
                .position(|other| {
                    other.intent.kind == OperationKind::CreateDirectory
                        && destination(other) == Some(&parent.location)
                })
                .ok_or_else(|| conflict("missing parent has no reviewed creation step"))?;
            edges[index].push(dependency);
        }
        if let Some(target) = step
            .destination
            .as_ref()
            .filter(|target| target.value.is_some())
        {
            let dependency = steps
                .iter()
                .position(|other| vacates(other) && source(other) == Some(&target.location))
                .ok_or_else(|| {
                    conflict("occupied destination is not vacated by this reviewed plan")
                })?;
            edges[index].push(dependency);
        }
    }
    let mut order = Vec::with_capacity(steps.len());
    let mut included = vec![false; steps.len()];
    while order.len() < steps.len() {
        let next = (0..steps.len()).find(|index| {
            !included[*index] && edges[*index].iter().all(|dependency| included[*dependency])
        });
        let Some(next) = next else {
            return Err(FsFailure::new(
                FsFailureKind::Unsupported,
                "cyclic rename graph requires a private recovery strategy; no steps applied",
            ));
        };
        included[next] = true;
        order.push(next);
    }
    let mut new_index = vec![0; steps.len()];
    for (index, old) in order.iter().enumerate() {
        new_index[*old] = index;
    }
    let mut old: Vec<_> = std::mem::take(steps).into_iter().map(Some).collect();
    for index in order {
        let Some(mut operation) = old[index].take() else {
            return Err(FsFailure::new(
                FsFailureKind::Protocol,
                "duplicate dependency order entry",
            ));
        };
        operation.dependencies = edges[index]
            .iter()
            .map(|dependency| new_index[*dependency])
            .collect();
        steps.push(operation);
    }
    Ok(())
}

pub fn execute(
    plan: &PreparedBatch,
    contents: &std::collections::HashMap<usize, ropey::Rope>,
    token: &CancelToken,
) -> Vec<StepReceipt> {
    let mut receipts: Vec<StepReceipt> = Vec::with_capacity(plan.steps.len());
    for (step, operation) in plan.steps.iter().enumerate() {
        let outcome = if token.is_cancelled() {
            StepOutcome::Cancelled {
                detail: "cancelled before this step started".into(),
            }
        } else if operation.dependencies.iter().any(|dependency| {
            receipts
                .get(*dependency)
                .is_none_or(|receipt| !receipt.outcome.is_committed())
        }) {
            StepOutcome::Refused(conflict("a required earlier step did not commit"))
        } else {
            match operation
                .intent
                .location()
                .map(|location| &location.filesystem)
            {
                Some(Filesystem::Local) => {
                    crate::local::execute(operation, contents.get(&step), &receipts, token)
                }
                Some(Filesystem::Remote(_)) => strop_remote::filesystem::execute(
                    operation,
                    contents.get(&step),
                    &receipts,
                    token,
                ),
                _ => StepOutcome::Refused(FsFailure::new(
                    FsFailureKind::Unsupported,
                    "namespace has no mutation capability",
                )),
            }
        };
        let uncertain = outcome.is_unconfirmed();
        receipts.push(StepReceipt {
            step,
            operation: operation.clone(),
            outcome,
        });
        if uncertain {
            receipts.extend(plan.steps.iter().enumerate().skip(step + 1).map(
                |(step, operation)| StepReceipt {
                    step,
                    operation: operation.clone(),
                    outcome: StepOutcome::Refused(conflict(
                        "an earlier outcome is unconfirmed; verify before further mutations",
                    )),
                },
            ));
            break;
        }
    }
    receipts
}

pub fn verify(receipt: &StepReceipt, token: &CancelToken) -> Result<VerifiedOutcome, FsFailure> {
    match receipt
        .operation
        .intent
        .location()
        .map(|location| &location.filesystem)
    {
        Some(Filesystem::Local) => crate::local::verify(receipt, token),
        Some(Filesystem::Remote(_)) => strop_remote::filesystem::verify(receipt, token),
        _ => Err(FsFailure::new(
            FsFailureKind::Unsupported,
            "namespace has no mutation verification capability",
        )),
    }
}
