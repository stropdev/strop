use super::*;
use std::fmt::Write;
struct View {
    text: String,
    rows: Vec<ReviewRow>,
}
impl View {
    fn new() -> Self {
        Self {
            text: String::new(),
            rows: Vec::new(),
        }
    }
    fn line(&mut self, text: &str, row: ReviewRow) {
        self.text.push_str(text);
        self.text.push('\n');
        self.rows.push(row);
    }
    fn finish(self) -> (String, Vec<ReviewRow>) {
        (self.text, self.rows)
    }
}
fn description(operation: &OperationIntent) -> String {
    let source = operation.source.as_ref().map(ResourceLocation::label);
    let destination = operation.destination.as_ref().map(ResourceLocation::label);
    match (source, destination) {
        (Some(source), Some(destination)) => {
            format!("{}: {source} → {destination}", operation.kind.label())
        }
        (Some(source), None) => format!("{}: {source}", operation.kind.label()),
        (None, Some(destination)) => format!("{}: {destination}", operation.kind.label()),
        _ => format!("{}: missing resource", operation.kind.label()),
    }
}
fn inventory(view: &mut View, batch: &strop_fs::batch::PreparedBatch) {
    for (index, step) in batch.steps.iter().enumerate() {
        view.line(
            &format!("{}. {}", index + 1, description(&step.intent)),
            if step.intent.kind.destructive() {
                ReviewRow::Removed
            } else {
                ReviewRow::Added
            },
        );
        if step.intent.kind == OperationKind::Copy {
            view.line(
                match step.intent.copy_version {
                    CopyVersion::Stored => "   copy stored bytes",
                    CopyVersion::Buffer => {
                        "   copy pinned current-buffer contents; original remains unsaved"
                    }
                },
                ReviewRow::Context,
            );
        }
        if !step.dependencies.is_empty() {
            let mut dependencies = String::from("   requires committed steps:");
            for dependency in &step.dependencies {
                let _ = write!(dependencies, " {}", dependency + 1);
            }
            view.line(&dependencies, ReviewRow::Context);
        }
    }
    for refusal in &batch.refused {
        view.line(
            &format!(
                "REFUSED {} — {}",
                description(&refusal.intent),
                refusal.failure
            ),
            ReviewRow::Warning,
        );
    }
    let mut policies = std::collections::BTreeSet::new();
    for step in &batch.steps {
        for policy in [
            &step.capability.metadata_policy,
            &step.capability.concurrency_policy,
        ] {
            if policies.insert(policy) {
                view.line(policy, ReviewRow::Context);
            }
        }
    }
}
pub(super) fn proposal(
    id: WorkerId,
    batch: &strop_fs::batch::PreparedBatch,
) -> (String, Vec<ReviewRow>) {
    let mut view = View::new();
    view.line(
        &format!("filesystem change review {}", id.get()),
        ReviewRow::Heading,
    );
    view.line(
        &format!(
            "{} exact steps; {} refused; nothing applied",
            batch.steps.len(),
            batch.refused.len()
        ),
        ReviewRow::Context,
    );
    view.line(
        "Apply filesystem changes: :apply-change   Cancel: :cancel-change",
        ReviewRow::Heading,
    );
    if batch
        .steps
        .iter()
        .any(|step| step.intent.kind == OperationKind::Remove)
    {
        view.line(
            "PERMANENT DELETION: these Remove steps have no Trash recovery.",
            ReviewRow::Warning,
        );
    }
    view.line(
        "Missing parents appear as separate creation steps. Apply approves every listed step.",
        ReviewRow::Context,
    );
    inventory(&mut view, batch);
    view.finish()
}
pub(super) fn receipt(
    id: WorkerId,
    batch: &strop_fs::batch::PreparedBatch,
    receipts: &[StepReceipt],
    warning: Option<&str>,
) -> (String, Vec<ReviewRow>) {
    let mut view = View::new();
    view.line(
        &format!("filesystem operation {} — receipt", id.get()),
        ReviewRow::Heading,
    );
    view.line(
        "A batch is not atomic. Verify never retries a mutation.",
        ReviewRow::Context,
    );
    if let Some(warning) = warning {
        view.line(
            &format!("Worker cleanup warning: {warning}"),
            ReviewRow::Warning,
        );
    }
    for receipt in receipts {
        view.line(
            &format!(
                "{}. {}",
                receipt.step + 1,
                description(&receipt.operation.intent)
            ),
            ReviewRow::File,
        );
        match &receipt.outcome {
            StepOutcome::Committed {
                recovery, warnings, ..
            } => {
                view.line("   COMMITTED", ReviewRow::Added);
                if receipt.operation.intent.kind != OperationKind::Remove {
                    view.line(
                        &format!(
                            "   checked recovery: :fs undo {} {}",
                            id.get(),
                            receipt.step + 1
                        ),
                        ReviewRow::Context,
                    );
                }
                if let Some(recovery) = recovery {
                    view.line(
                        &format!("   recovery: {}", recovery.label()),
                        ReviewRow::Context,
                    );
                }
                for warning in warnings {
                    view.line(&format!("   warning: {warning}"), ReviewRow::Warning);
                }
            }
            StepOutcome::Refused(error) => {
                view.line(&format!("   REFUSED: {error}"), ReviewRow::Warning)
            }
            StepOutcome::Cancelled { detail } => {
                view.line(&format!("   CANCELLED: {detail}"), ReviewRow::Context)
            }
            StepOutcome::Unconfirmed {
                detail, recovery, ..
            } => {
                view.line(&format!("   UNCONFIRMED: {detail}"), ReviewRow::Warning);
                view.line(
                    &format!("   :fs verify {} {}", id.get(), receipt.step + 1),
                    ReviewRow::Context,
                );
                if let Some(recovery) = recovery {
                    view.line(
                        &format!("   possible recovery: {}", recovery.label()),
                        ReviewRow::Warning,
                    );
                }
            }
        }
    }
    for refusal in &batch.refused {
        view.line(
            &format!(
                "REFUSED {} — {}",
                description(&refusal.intent),
                refusal.failure
            ),
            ReviewRow::Warning,
        );
    }
    view.finish()
}
