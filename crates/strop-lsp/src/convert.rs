//! LSP-type → strop-type conversions (hover text, diagnostics):
//! pure functions over async_lsp::lsp_types.

use async_lsp::lsp_types::Diagnostic;
use strop_core::id::LineIndex;

use crate::protocol::{Diag, ServerColumn, Severity};

pub(crate) fn hover_text(hover: &async_lsp::lsp_types::Hover) -> String {
    use async_lsp::lsp_types::HoverContents;
    let marked = |s: &async_lsp::lsp_types::MarkedString| match s {
        async_lsp::lsp_types::MarkedString::String(s) => s.clone(),
        async_lsp::lsp_types::MarkedString::LanguageString(l) => l.value.clone(),
    };
    match &hover.contents {
        HoverContents::Markup(m) => m.value.clone(),
        HoverContents::Scalar(s) => marked(s),
        HoverContents::Array(a) => a.iter().map(marked).collect::<Vec<_>>().join("\n"),
    }
}

/// LSP diagnostic → wire-domain `Diag` (server columns; the editor
/// resolves them through the rope at receipt).
pub(crate) fn diag_from_lsp(d: &Diagnostic) -> Diag {
    Diag {
        line: LineIndex::new(d.range.start.line as usize),
        col: ServerColumn::new(d.range.start.character as usize),
        end_line: LineIndex::new(d.range.end.line as usize),
        end_col: ServerColumn::new(d.range.end.character as usize),
        severity: d
            .severity
            .map(|s| match s {
                async_lsp::lsp_types::DiagnosticSeverity::ERROR => Severity::Error,
                async_lsp::lsp_types::DiagnosticSeverity::WARNING => Severity::Warning,
                async_lsp::lsp_types::DiagnosticSeverity::INFORMATION => Severity::Information,
                _ => Severity::Hint,
            })
            .unwrap_or(Severity::Information),
        message: d.message.clone(),
    }
}
