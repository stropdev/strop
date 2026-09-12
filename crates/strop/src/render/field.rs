//! One printable cell projection for modal input fields, independent of grammar.
use super::{ACCENT, MUTED, TEXT};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use strop_core::id::DisplayColumn;
use strop_core::layout::{clip, printable_grapheme, LineLayout, RopeGraphemes};
use strop_picker::query::{HighlightSpan, Role};

pub(super) struct FieldLine {
    pub line: Line<'static>,
    pub cursor: Option<u16>,
}

pub(super) fn project(
    label: &str,
    value: &str,
    caret: usize,
    active: bool,
    width: u16,
    highlights: &[HighlightSpan],
) -> FieldLine {
    if width == 0 {
        return FieldLine {
            line: Line::default(),
            cursor: None,
        };
    }
    let label = format!("{label} ");
    let label_budget = usize::from(width.saturating_sub(1));
    let mut spans = Vec::new();
    let mut prefix = 0;
    for (glyph, grapheme) in RopeGraphemes::new(label.as_str().into(), 4) {
        let Some(visible) = clip(glyph, DisplayColumn::new(0), label_budget) else {
            continue;
        };
        spans.push(Span::styled(
            if visible.complete {
                printable_grapheme(&grapheme).to_string()
            } else {
                " ".repeat(visible.width)
            },
            Style::default().fg(if active { ACCENT } else { MUTED }),
        ));
        prefix = visible.x + visible.width;
    }
    let available = usize::from(width).saturating_sub(prefix);
    let layout = LineLayout::build(value, 4);
    let caret_cell = layout.cell_at_byte(caret.min(value.len())).get();
    let origin = DisplayColumn::new(caret_cell.saturating_sub(available.saturating_sub(1)));
    let mut used = 0;
    for (glyph, grapheme) in RopeGraphemes::new(value.into(), 4) {
        if glyph.cell.get() >= origin.get() + available {
            break;
        }
        let Some(visible) = clip(glyph, origin, available) else {
            continue;
        };
        let style = highlights
            .iter()
            .rev()
            .find(|span| span.range.contains(&glyph.byte))
            .map_or_else(|| Style::default().fg(TEXT), |span| role_style(span.role));
        let text = if visible.complete && grapheme != "\t" {
            printable_grapheme(&grapheme).to_string()
        } else {
            " ".repeat(visible.width)
        };
        spans.push(Span::styled(text, style));
        used = visible.x + visible.width;
    }
    if active && caret == value.len() && used < available {
        spans.push(Span::styled("▏", Style::default().fg(ACCENT)));
    }
    FieldLine {
        line: Line::from(spans),
        cursor: active.then_some((prefix + caret_cell.saturating_sub(origin.get())) as u16),
    }
}

fn role_style(role: Role) -> Style {
    match role {
        Role::QualifierKey => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        Role::Punctuation => Style::default().fg(MUTED),
        Role::Value | Role::Literal => Style::default().fg(TEXT),
        Role::Regex => Style::default().fg(Color::Rgb(0xcb, 0xa6, 0xf7)),
        Role::Negation | Role::Error => Style::default()
            .fg(Color::Rgb(0xf3, 0x8b, 0xa8))
            .add_modifier(Modifier::UNDERLINED),
        Role::Incomplete => Style::default().fg(MUTED).add_modifier(Modifier::ITALIC),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caret_and_text_share_unicode_tab_and_resize_geometry() {
        let text = "界e\u{301}\talpha界omega";
        for width in [1, 4, 12, 40] {
            for caret in [0, 3, 6, text.len()] {
                let projection = project("find", text, caret, true, width, &[]);
                assert!(projection.line.width() <= usize::from(width));
                assert!(projection.cursor.is_some_and(|column| column < width));
                assert!(projection
                    .line
                    .spans
                    .iter()
                    .all(|span| !span.content.chars().any(char::is_control)));
            }
        }
    }
}
