//! Per-document diagnostic queries (0009): gutter signs, the cursor
//! line's end-of-line note, modeline chips. Data arrives via LSP events;
//! these read it.

use std::path::PathBuf;

use super::Editor;

impl Editor {
    /// (errors, warnings) on the buffer — the modeline's diag chips.
    pub fn diag_counts(&self, idx: strop_core::id::DocumentId) -> (usize, usize) {
        let mut e = 0;
        let mut w = 0;
        for d in self.diags_for(idx).into_iter().flatten() {
            match d.severity {
                1 => e += 1,
                2 => w += 1,
                _ => {}
            }
        }
        (e, w)
    }

    /// Diagnostics of buffer `idx`, resolved against cwd like
    /// diag_severity_at.
    fn diags_for(&self, idx: strop_core::id::DocumentId) -> Option<&Vec<strop_lsp::Diag>> {
        let path = self.docs.get(idx).map(|d| &d.buf)?.path.as_deref()?;
        let abs = if std::path::Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };
        self.diags.get(&abs)
    }

    /// The worst diagnostic's (severity, message) on a 1-based line —
    /// the cursor line's end-of-line note (0009 UX).
    pub fn diag_message_at(
        &self,
        idx: strop_core::id::DocumentId,
        line_1based: usize,
    ) -> Option<(u8, &str)> {
        self.diags_for(idx)?
            .iter()
            .filter(|d| d.line + 1 == line_1based)
            .min_by_key(|d| d.severity)
            .map(|d| (d.severity, d.message.as_str()))
    }

    /// Diagnostic spans on a 1-based line as (col, end_col, severity)
    /// — the undercurl layer (0009 UX). Same-line diags only.
    pub fn diag_ranges_at(
        &self,
        idx: strop_core::id::DocumentId,
        line_1based: usize,
    ) -> Vec<(usize, usize, u8)> {
        self.diags_for(idx)
            .map(|ds| {
                ds.iter()
                    .filter(|d| d.line + 1 == line_1based)
                    .map(|d| (d.col, d.end_col.max(d.col + 1), d.severity))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Worst diagnostic severity (1=error … 4=hint) for a 1-based line
    /// of buffer `idx`, if any (0001 pillar 4: merges with the git
    /// gutter). Per-buffer, so panes show their own diagnostics.
    pub fn diag_severity_at(
        &self,
        idx: strop_core::id::DocumentId,
        line_1based: usize,
    ) -> Option<u8> {
        let path = self.docs.get(idx).map(|d| &d.buf)?.path.as_deref()?;
        let abs = if std::path::Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.cwd.join(path)
        };
        let diags = self.diags.get(&abs)?;
        let mut best: Option<u8> = None;
        for d in diags {
            if d.line + 1 == line_1based {
                best = Some(best.map_or(d.severity, |b: u8| b.min(d.severity)));
            }
        }
        best
    }
}
