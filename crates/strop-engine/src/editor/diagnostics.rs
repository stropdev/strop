//! Per-document diagnostic queries (0009): gutter signs, the cursor
//! line's end-of-line note, modeline chips. Data arrives via LSP events
//! already resolved to byte-domain columns; these read it.

use strop_lsp::{ResolvedDiag, Severity};

use super::Editor;

pub struct DocumentDiagnostics {
    pub revision: strop_core::id::BufferRevision,
    pub items: Vec<ResolvedDiag>,
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
