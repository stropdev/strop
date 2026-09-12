//! Explicit persistence receipts. Every admitted write owns its report target;
//! later edits, focus changes and newer change plans cannot redirect its result.
use super::*;

pub(super) struct PendingChangeSave {
    pub(super) report: DocumentId,
    label: String,
}

impl Editor {
    /// Save the newest applied operation, never merely the current buffer.
    pub(crate) fn save_changed_files_pub(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let targets: Vec<_> = self
            .changes
            .receipts
            .back()
            .into_iter()
            .flat_map(|receipt| receipt.applied.iter().map(|(document, _, _)| *document))
            .filter(|document| seen.insert(*document))
            .filter(|document| self.docs.get(*document).is_none_or(|doc| doc.buf.dirty))
            .collect();
        if targets.is_empty() {
            self.message = "no changed files to save".into();
            return;
        }
        let mut buffer = Buffer::from_text("strop save receipt — per-file persistence results\nApply and Save are separate; newer edits remain unsaved.\n\n");
        buffer.name = Some("change save receipt".into());
        let report = self.open_temporary_output(buffer);
        self.review.rows.insert(
            report,
            vec![ReviewRow::Heading, ReviewRow::Context, ReviewRow::Context],
        );
        let mut admitted = 0;
        for document in targets {
            let label = self
                .docs
                .get(document)
                .map(|doc| doc.label(&self.cwd))
                .unwrap_or_else(|| format!("closed source {document:?}"));
            if self.request_save_document(document, None, false, false) {
                self.append_save_result(report, &label, "saving");
                self.review
                    .saves
                    .insert(document, PendingChangeSave { report, label });
                admitted += 1;
            } else {
                let reason = self.message.clone();
                self.append_save_result(report, &label, &reason);
            }
        }
        self.message =
            format!("saving {admitted} changed file(s); per-file results in this receipt");
    }

    pub(crate) fn finish_change_save(&mut self, document: DocumentId) {
        let Some(pending) = self.review.saves.remove(&document) else {
            return;
        };
        let result = self.message.clone();
        self.append_save_result(pending.report, &pending.label, &result);
    }

    fn append_save_result(&mut self, report: DocumentId, label: &str, result: &str) {
        let Some(doc) = self.docs.get(report) else {
            return;
        };
        let end = doc.buf.len_bytes();
        let row = doc.buf.line_of(end);
        // Paths and server diagnostics are metadata, not executable terminal
        // controls or extra receipt rows. Keep their full printable spelling.
        let mut line =
            strop_core::layout::printable_text(format!("{label}: {result}")).into_owned();
        line.push('\n');
        let appended = self
            .doc_mut(report)
            .buf
            .system_edit()
            .replace(strop_core::Range::charwise(end, end), &line);
        if let Err(error) = appended {
            self.message = format!("save receipt update failed: {error}");
            return;
        }
        let rows = self.review.rows.entry(report).or_default();
        rows.resize(row, ReviewRow::Context);
        rows.push(ReviewRow::Context);
    }
}
