//! LSP-type → strop-type conversions (hover text, diagnostics):
//! pure functions over async_lsp::lsp_types.

use async_lsp::lsp_types::Diagnostic;

use crate::protocol::Diag;

pub(crate) fn hover_text(hover: &async_lsp::lsp_types::Hover) -> String {
    use async_lsp::lsp_types::HoverContents;
    match &hover.contents {
        HoverContents::Markup(m) => m.value.clone(),
        HoverContents::Array(a) => a
            .iter()
            .map(|s| match s {
                async_lsp::lsp_types::MarkedString::String(s) => s.clone(),
                async_lsp::lsp_types::MarkedString::LanguageString(l) => l.value.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// LSP diagnostic → UTF-8-friendly Diag (positions are UTF-16 line/col;
/// the editor maps them through the rope at render time).
pub(crate) fn diag_from_lsp(d: &Diagnostic) -> Diag {
    Diag {
        line: d.range.start.line as usize,
        col: d.range.start.character as usize,
        end_line: d.range.end.line as usize,
        end_col: d.range.end.character as usize,
        severity: d
            .severity
            .map(|s| match s {
                async_lsp::lsp_types::DiagnosticSeverity::ERROR => 1,
                async_lsp::lsp_types::DiagnosticSeverity::WARNING => 2,
                async_lsp::lsp_types::DiagnosticSeverity::INFORMATION => 3,
                _ => 4,
            })
            .unwrap_or(3),
        message: d.message.clone(),
    }
}
