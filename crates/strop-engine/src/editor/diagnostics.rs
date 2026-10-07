//! Per-document diagnostic queries (0009): gutter signs, the cursor
//! line's end-of-line note, modeline chips. Data arrives via LSP events
//! already resolved to byte-domain columns; these read it.

use strop_lsp::{ResolvedDiag, Severity};

use super::{Editor, Mode};

pub struct DocumentDiagnostics {
    pub revision: strop_core::id::BufferRevision,
    pub items: Vec<ResolvedDiag>,
}

impl Editor {
    /// Publish one diagnostic set for a document. Insert-mode freeze
    /// (Neovim's `update_in_insert = false` default — the modal answer
    /// to the flashing-diagnostics complaint, helix #13596 asks for the
    /// same): while the edited document is in Insert, the publish is
    /// stashed instead of displayed. The frozen set's revision stops
    /// matching on the first edit, so typing never paints partial-word
    /// underlines; publishes for other documents stay live, and the
    /// latest stash applies at InsertLeave (see
    /// [`Self::apply_pending_diagnostics`]).
    pub(crate) fn record_diagnostics(
        &mut self,
        document: strop_core::id::DocumentId,
        set: DocumentDiagnostics,
    ) {
        if matches!(self.mode, Mode::Insert) && document == self.current() {
            self.pending_diags.insert(document, set);
        } else {
            self.pending_diags.remove(&document);
            self.diags.insert(document, set);
        }
    }

    /// InsertLeave: apply the stash when it still names the buffer's
    /// revision (the last publish covered the last edit — the common
    /// case, so underlines return the moment Esc lands). An older stash
    /// is dropped: the next publish for the unedited buffer arrives on
    /// its own.
    pub(crate) fn apply_pending_diagnostics(&mut self) {
        let current = self.current();
        let Some(set) = self.pending_diags.remove(&current) else {
            return;
        };
        if self
            .docs
            .get(current)
            .is_some_and(|doc| doc.buf.revision() == set.revision)
        {
            self.diags.insert(current, set);
        }
    }
}

impl Editor {
    /// (errors, warnings) on the buffer — the modeline's diag chips.
    pub fn diag_counts(&self, idx: strop_core::id::DocumentId) -> (usize, usize) {
        let mut e = 0;
        let mut w = 0;
        for d in self.diags_for(idx).into_iter().flatten() {
            match d.severity {
                Severity::Error => e += 1,
                Severity::Warning => w += 1,
                _ => {}
            }
        }
        (e, w)
    }

    /// Cached diagnostics belong to a document incarnation and text revision,
    /// not a pathname shared by several full/range/tail views.
    pub(super) fn diags_for(&self, idx: strop_core::id::DocumentId) -> Option<&[ResolvedDiag]> {
        let cached = self.diags.get(&idx)?;
        (self.docs.get(idx)?.buf.revision() == cached.revision).then_some(cached.items.as_slice())
    }

    /// The worst diagnostic's (severity, message) on a 1-based line —
    /// the cursor line's end-of-line note (0009 UX).
    pub fn diag_message_at(
        &self,
        idx: strop_core::id::DocumentId,
        line_1based: usize,
    ) -> Option<(Severity, &str)> {
        self.diags_for(idx)?
            .iter()
            .filter(|d| d.line.get() + 1 == line_1based)
            .min_by_key(|d| d.severity)
            .map(|d| (d.severity, d.message.as_str()))
    }

    /// Diagnostic spans on a 1-based line as (col, end_col, severity)
    /// — the undercurl layer (0009 UX). Same-line diags only; columns
    /// are byte offsets into the line.
    pub fn diag_ranges_at(
        &self,
        idx: strop_core::id::DocumentId,
        line_1based: usize,
    ) -> Vec<(usize, usize, Severity)> {
        self.diags_for(idx)
            .map(|ds| {
                ds.iter()
                    .filter(|d| d.line.get() + 1 == line_1based)
                    .map(|d| {
                        (
                            d.col.get(),
                            d.end_col.get().max(d.col.get() + 1),
                            d.severity,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Worst diagnostic severity for a 1-based line of buffer `idx`, if
    /// any (0001 pillar 4: merges with the git gutter). Per-buffer, so
    /// panes show their own diagnostics.
    pub fn diag_severity_at(
        &self,
        idx: strop_core::id::DocumentId,
        line_1based: usize,
    ) -> Option<Severity> {
        let diags = self.diags_for(idx)?;
        let mut best: Option<Severity> = None;
        for d in diags {
            if d.line.get() + 1 == line_1based {
                best = Some(best.map_or(d.severity, |b: Severity| b.min(d.severity)));
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::id::{BufferRevision, ByteColumn, LineIndex};
    use strop_core::Buffer;

    fn diag_set(revision: u64) -> DocumentDiagnostics {
        DocumentDiagnostics {
            revision: BufferRevision::new(revision),
            items: vec![ResolvedDiag {
                line: LineIndex::new(0),
                col: ByteColumn::new(0),
                end_line: LineIndex::new(0),
                end_col: ByteColumn::new(4),
                severity: Severity::Warning,
                message: "partial word".into(),
            }],
        }
    }

    /// Insert-mode freeze (Neovim's update_in_insert=false default):
    /// typing hides the frozen set on the first edit, a publish for the
    /// edited document is stashed rather than painted mid-word, and
    /// InsertLeave applies the stash while it is revision-current.
    #[test]
    fn insert_mode_freezes_the_edited_documents_diagnostics() {
        let mut e = Editor::new(Buffer::from_text("word here\n"));
        let doc = e.current();
        let rev0 = e.buf().revision();
        e.record_diagnostics(doc, diag_set(rev0.get()));
        assert_eq!(e.diag_counts(doc).1, 1, "normal mode shows the set");

        e.feed_text("i");
        assert!(matches!(e.mode(), Mode::Insert));
        e.feed_text("x");
        assert_eq!(
            e.diag_counts(doc),
            (0, 0),
            "the frozen set hides once the text moves"
        );
        // the server's publish for the partial text: stashed, not painted
        let rev1 = e.buf().revision();
        e.record_diagnostics(doc, diag_set(rev1.get()));
        assert_eq!(e.diag_counts(doc), (0, 0), "no partial-word underline");

        e.feed(strop_core::frontend_input::Key::Esc);
        assert_eq!(e.diag_counts(doc).1, 1, "InsertLeave applies the stash");
    }

    /// The freeze is scoped to the document being edited — other
    /// documents publish live even in Insert mode (split panes).
    #[test]
    fn publishes_for_other_documents_stay_live_in_insert_mode() {
        let mut e = Editor::new(Buffer::from_text("word here\n"));
        e.feed_text("i");
        assert!(matches!(e.mode(), Mode::Insert));
        let mut arena = strop_core::id::Arena::<strop_core::id::DocumentKind, ()>::default();
        let _ = arena.try_insert(());
        let other = arena.try_insert(()).unwrap(); // slot 1 — never current()
        e.record_diagnostics(other, diag_set(0));
        assert!(
            e.diags.contains_key(&other),
            "a non-current document's publish is not stashed"
        );
        assert!(!e.pending_diags.contains_key(&other));
    }

    /// A stash computed before the final edits never paints stale
    /// positions at InsertLeave; the next live publish owns the truth.
    #[test]
    fn a_stale_stash_is_dropped_at_insert_leave() {
        let mut e = Editor::new(Buffer::from_text("word here\n"));
        let doc = e.current();
        let rev0 = e.buf().revision();
        e.feed_text("i");
        e.record_diagnostics(doc, diag_set(rev0.get()));
        e.feed_text("xy"); // the buffer moved past the stash
        e.feed(strop_core::frontend_input::Key::Esc);
        assert_eq!(e.diag_counts(doc), (0, 0), "stale stash dropped");
        assert!(!e.pending_diags.contains_key(&doc));
    }
}
