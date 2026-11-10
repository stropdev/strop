//! The language-service badge (0088 §2 rule 4, U3): a 4-cell `LSP!` only
//! when a service has failed, `LSP?` when one was refused, a count when
//! several did — and nothing at all while services start or run. The
//! complete scoped list and reasons live in `:explain`.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::editor::{Editor, LspPhase};
use crate::render::style::Tokens;

pub(super) struct Badge {
    text: String,
    phase: LspPhase,
}

impl Badge {
    pub(super) fn gather(editor: &Editor) -> Option<Self> {
        let mut failed = 0usize;
        let mut refused = 0usize;
        for status in editor.lsp_statuses() {
            match status.phase {
                LspPhase::Failed => failed += 1,
                LspPhase::Refused => refused += 1,
                LspPhase::Starting | LspPhase::Ready => {}
            }
        }
        let (mark, count, phase) = if failed > 0 {
            ("LSP!", failed, LspPhase::Failed)
        } else if refused > 0 {
            ("LSP?", refused, LspPhase::Refused)
        } else {
            return None;
        };
        Some(Self {
            text: if count == 1 {
                mark.to_owned()
            } else {
                format!("{mark}{count}")
            },
            phase,
        })
    }

    pub(super) fn width(&self) -> usize {
        self.text.chars().count()
    }

    pub(super) fn span(&self, t: &Tokens) -> Span<'_> {
        let color = match self.phase {
            LspPhase::Failed => t.error,
            LspPhase::Refused | LspPhase::Starting | LspPhase::Ready => t.warning,
        };
        Span::styled(
            self.text.as_str(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )
    }
}
