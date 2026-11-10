//! Writing a group of files as one act (0094): a collection's `:w`, a
//! review's apply and its revert. Each file is an ordinary owned save; the
//! group reports once, when the last one lands, and names what failed.
use std::collections::HashSet;

use strop_core::id::DocumentId;

use crate::editor::Editor;

pub(crate) struct BatchWrite {
    id: u64,
    label: String,
    pending: HashSet<DocumentId>,
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
        // Registered before the first request: a save can complete inside
        // its own admission (replay, protocol failure).
        let id = self.changes.write_seq;
        self.changes.write_seq = id.wrapping_add(1);
        self.changes.writes.push(BatchWrite {
            id,
            label: label.to_owned(),
            pending: documents.iter().copied().collect(),
            wrote: 0,
            failed: Vec::new(),
            review,
        });
        let count = documents.len();
        for document in documents {
            if self.request_save_document(document, None, false, false) {
                continue;
            }
            let reason = self.message.clone();
            self.settle_batch_write(document, Some(reason));
        }
        if self.changes.writes.iter().any(|batch| batch.id == id) {
            self.message = format!("{label}: writing {count} file(s)");
        }
    }

    /// One save finished, successfully or not.
    pub(crate) fn finish_batch_write(&mut self, document: DocumentId) {
        let failure = self
            .docs
            .get(document)
            .is_none_or(|doc| doc.buf.is_dirty())
            .then(|| self.message.clone());
        self.settle_batch_write(document, failure);
    }

    /// A review's files are still on their way to disk.
    pub(crate) fn review_write_pending(&self, review: DocumentId) -> bool {
        self.changes
            .writes
            .iter()
            .any(|batch| batch.review == Some(review))
    }

    fn settle_batch_write(&mut self, document: DocumentId, failure: Option<String>) {
        let Some(index) = self
            .changes
            .writes
            .iter()
            .position(|batch| batch.pending.contains(&document))
        else {
            return;
        };
        let batch = &mut self.changes.writes[index];
        batch.pending.remove(&document);
        match failure {
            None => batch.wrote += 1,
            Some(reason) => {
                let name = self
                    .docs
                    .get(document)
                    .map_or_else(|| "a closed buffer".to_owned(), |doc| doc.label(&self.cwd));
                self.changes.writes[index]
                    .failed
                    .push(format!("{name} ({reason})"));
            }
        }
        if !self.changes.writes[index].pending.is_empty() {
            return;
        }
        let batch = self.changes.writes.remove(index);
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
