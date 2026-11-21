//! Writing a group of files as one act (0094): a collection's `:w`, a
//! review's apply and its revert. Each file is an ordinary owned save; the
//! group reports once, when the last one lands, and names what failed.
use std::collections::HashMap;

use strop_core::id::DocumentId;
use strop_core::worker::WorkerId;

use crate::editor::Editor;

pub(crate) struct BatchWrite {
    label: String,
    /// Worker identity and the file name at admission, even if later closed.
    pending: HashMap<WorkerId, String>,
    wrote: usize,
    failed: Vec<String>,
    /// The review whose heading reports the result.
    review: Option<DocumentId>,
}

impl Editor {
    /// Save `documents` together. Dirty buffers only reach here; a refused
    /// save is named in the result like a failed one.
    pub(crate) fn write_documents(
        &mut self,
        label: &str,
        documents: Vec<DocumentId>,
        review: Option<DocumentId>,
    ) {
        let count = documents.len();
        let mut batch = BatchWrite {
            label: label.to_owned(),
            pending: HashMap::with_capacity(count),
            wrote: 0,
            failed: Vec::new(),
            review,
        };
        for document in documents {
            let name = self
                .docs
                .get(document)
                .map_or_else(|| "a closed buffer".to_owned(), |doc| doc.label(&self.cwd));
            match self.admit_document_save(document, None, false, false) {
                Ok(Some(request)) => {
                    batch.pending.insert(request, name);
                }
                Ok(None) => batch
                    .failed
                    .push(format!("{name} (filename changes need review)")),
                Err(reason) => batch.failed.push(format!("{name} ({reason})")),
            }
        }
        if batch.pending.is_empty() {
            self.report_batch_write(batch);
        } else {
            self.changes.writes.push(batch);
            self.message = format!("{label}: writing {count} file(s)");
        }
    }

    /// A worker result belongs only to the batch that admitted this request.
    /// Returns whether its final report was published.
    pub(crate) fn finish_batch_write(&mut self, request: WorkerId, failure: Option<&str>) -> bool {
        let Some((index, name)) = self
            .changes
            .writes
            .iter_mut()
            .enumerate()
            .find_map(|(i, batch)| batch.pending.remove(&request).map(|name| (i, name)))
        else {
            return false;
        };
        let batch = &mut self.changes.writes[index];
        match failure {
            None => batch.wrote += 1,
            Some(reason) => batch.failed.push(format!("{name} ({reason})")),
        }
        if !batch.pending.is_empty() {
            return false;
        }
        let batch = self.changes.writes.remove(index);
        self.report_batch_write(batch);
        true
    }

    /// A review's files are still on their way to disk.
    pub(crate) fn review_write_pending(&self, review: DocumentId) -> bool {
        self.changes
            .writes
            .iter()
            .any(|batch| batch.review == Some(review))
    }

    fn report_batch_write(&mut self, batch: BatchWrite) {
        self.message = match batch.failed.as_slice() {
            [] => format!("{}: wrote {} file(s)", batch.label, batch.wrote),
            failed => format!(
                "{}: wrote {} file(s); could not write {}",
                batch.label,
                batch.wrote,
                failed.join(", ")
            ),
        };
        if let Some(review) = batch.review {
            self.review_written(review, batch.wrote, &batch.failed);
        }
    }
}
