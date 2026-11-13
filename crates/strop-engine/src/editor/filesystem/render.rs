//! The confirm buffer `:w` opens in a folder, and the per-change result
//! it turns into when not every change was made. Rows render the prepared
//! batch itself: what is shown is what `y` executes.
use super::*;
use std::path::Path;

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

fn relative(root: Option<&ResourceLocation>, location: &ResourceLocation) -> String {
    match root {
        Some(root) => crate::editor::Directory::relative_label(root, location),
        None => location.label(),
    }
}

fn verb(intent: &OperationIntent) -> &'static str {
    match intent.kind {
        OperationKind::CreateFile | OperationKind::CreateDirectory => "create",
        OperationKind::Rename => {
            let parent = |location: &Option<ResourceLocation>| {
                location
                    .as_ref()
                    .map(|location| location.path.parent().map(Path::to_path_buf))
            };
            if parent(&intent.source) == parent(&intent.destination) {
                "rename"
            } else {
                "move"
            }
        }
        OperationKind::Copy => "copy",
        OperationKind::Trash => "trash",
        OperationKind::Remove => "delete",
        OperationKind::Restore => "restore",
        OperationKind::Store => "save",
    }
}

/// One change: `  rename  main.rs → main.rx`; a folder ends in `/`.
fn change(root: Option<&ResourceLocation>, intent: &OperationIntent, folder: bool) -> String {
    let name = |location: &ResourceLocation| {
        let mut text = relative(root, location);
        if folder || intent.kind == OperationKind::CreateDirectory {
            text.push('/');
        }
        text
    };
    let what = match (&intent.source, &intent.destination) {
        (Some(source), Some(destination)) => format!("{} → {}", name(source), name(destination)),
        (Some(source), None) => name(source),
        (None, Some(destination)) => name(destination),
        (None, None) => String::new(),
    };
    format!("  {:<7} {what}", verb(intent))
}

/// The step acts on a folder (its observed source is one).
fn folder(step: &PreparedOperation) -> bool {
    step.source
        .as_ref()
        .and_then(|source| source.value.as_ref())
        .is_some_and(|observed| observed.kind == strop_workspace::EntryKind::Directory)
}

fn row(intent: &OperationIntent) -> ReviewRow {
    if intent.kind.destructive() {
        ReviewRow::Removed
    } else {
        ReviewRow::Added
    }
}

fn refusals(
    view: &mut View,
    root: Option<&ResourceLocation>,
    batch: &strop_fs::batch::PreparedBatch,
) {
    for refusal in &batch.refused {
        view.line(
            &format!(
                "{}  refused: {}",
                change(root, &refusal.intent, false),
                refusal.failure
            ),
            ReviewRow::Warning,
        );
    }
}

/// The confirm buffer: a heading, one row per change, the two keys.
pub(super) fn proposal(
    label: &str,
    root: Option<&ResourceLocation>,
    batch: &strop_fs::batch::PreparedBatch,
) -> (String, Vec<ReviewRow>) {
    let mut view = View::new();
    view.line(
        &format!("{label} — {}", changes(batch.steps.len())),
        ReviewRow::Heading,
    );
    view.line("", ReviewRow::Context);
    for step in &batch.steps {
        view.line(&change(root, &step.intent, folder(step)), row(&step.intent));
    }
    refusals(&mut view, root, batch);
    view.line("", ReviewRow::Context);
    view.line("y apply   q cancel", ReviewRow::Heading);
    view.finish()
}

/// What happened to each change when not all of them were made.
pub(super) fn outcome(
    label: &str,
    root: Option<&ResourceLocation>,
    batch: &strop_fs::batch::PreparedBatch,
    receipts: &[StepReceipt],
) -> (String, Vec<ReviewRow>) {
    let mut view = View::new();
    let done = receipts
        .iter()
        .filter(|receipt| receipt.outcome.is_committed())
        .count();
    view.line(
        &format!("{label} — {done} of {} applied", changes(receipts.len())),
        ReviewRow::Heading,
    );
    view.line("", ReviewRow::Context);
    for receipt in receipts {
        let line = change(root, &receipt.operation.intent, folder(&receipt.operation));
        let (result, kind) = match &receipt.outcome {
            StepOutcome::Committed { .. } => ("done".to_string(), ReviewRow::Added),
            StepOutcome::Refused(error) => (format!("failed: {error}"), ReviewRow::Warning),
            StepOutcome::Cancelled { .. } => ("not attempted".to_string(), ReviewRow::Context),
            StepOutcome::Unconfirmed { detail, .. } => {
                (format!("unconfirmed: {detail}"), ReviewRow::Warning)
            }
        };
        view.line(&format!("{line}  {result}"), kind);
    }
    refusals(&mut view, root, batch);
    view.line("", ReviewRow::Context);
    view.line("q returns to the folder", ReviewRow::Heading);
    view.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(path: &str) -> ResourceLocation {
        ResourceLocation::local(std::path::PathBuf::from(path))
    }
    fn intent(
        kind: OperationKind,
        source: Option<&str>,
        destination: Option<&str>,
    ) -> OperationIntent {
        OperationIntent {
            kind,
            source: source.map(local),
            destination: destination.map(local),
            copy_version: CopyVersion::Stored,
            expected_content: None,
            store: None,
        }
    }

    #[test]
    fn each_change_reads_as_one_verb_and_paths_relative_to_the_folder() {
        let root = local("/w/demo/src");
        let root = Some(&root);
        let cases = [
            (
                intent(
                    OperationKind::Rename,
                    Some("/w/demo/src/main.rs"),
                    Some("/w/demo/src/main.rx"),
                ),
                "  rename  main.rs → main.rx",
            ),
            (
                intent(
                    OperationKind::CreateFile,
                    None,
                    Some("/w/demo/src/fresh.txt"),
                ),
                "  create  fresh.txt",
            ),
            (
                intent(
                    OperationKind::CreateDirectory,
                    None,
                    Some("/w/demo/src/sub"),
                ),
                "  create  sub/",
            ),
            (
                intent(
                    OperationKind::Rename,
                    Some("/w/demo/docs/notes.txt"),
                    Some("/w/demo/src/notes.txt"),
                ),
                "  move    ../docs/notes.txt → notes.txt",
            ),
            (
                intent(
                    OperationKind::Copy,
                    Some("/w/demo/src/lib.rs"),
                    Some("/w/demo/src/lib copy.rs"),
                ),
                "  copy    lib.rs → lib copy.rs",
            ),
            (
                intent(OperationKind::Trash, Some("/w/demo/src/new.rs"), None),
                "  trash   new.rs",
            ),
            (
                intent(OperationKind::Remove, Some("/w/demo/src/old.rs"), None),
                "  delete  old.rs",
            ),
        ];
        for (intent, expected) in cases {
            assert_eq!(change(root, &intent, false), expected);
        }
    }
}
