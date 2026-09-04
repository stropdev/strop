//! Which-key overlay (0003 §3): shows while a leader prefix pends.
//! Generated from keymap::BINDINGS (0003 §5.7) — no hand-maintained
//! hint lists to drift against dispatch.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;
use crate::keymap;

use super::{ACCENT, BASE, MUTED, SELECT_BG, TEXT};

/// Pending prefixes that show a card: (pending keys, card title).
const PREFIXES: &[(&str, &str)] = &[
    (" ", " space "),
    (" g", " space g "),
    ("g", " g "),
    ("]", " ] "),
    ("[", " [ "),
    ("m", " mark "),
    ("'", " mark jump "),
    ("`", " mark jump "),
];

pub fn render_which_key(editor: &Editor, frame: &mut Frame) {
    if editor.picker_open() {
        return;
    }
    let Some(&(_, title)) = PREFIXES.iter().find(|(p, _)| *p == editor.pending) else {
        return;
    };
    let hints = keymap::children_of(&editor.pending, editor.mode);
    if hints.is_empty() {
        return;
    }
    render_hints(frame, title, &hints);
}

fn render_hints(frame: &mut Frame, title: &str, hints: &[keymap::Hint]) {
    let area = frame.area();
    let width = 40u16.min(area.width.saturating_sub(4));
    let height = (hints.len() as u16 + 2).min(area.height.saturating_sub(2));
    let card = Rect {
        x: area.width.saturating_sub(width + 2),
        y: area.height.saturating_sub(height + 2),
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            title,
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let lines: Vec<Line> = hints
        .iter()
        .map(|h| {
            let key_style = if h.live {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            };
            let desc_style = if h.live {
                Style::default().fg(TEXT)
            } else {
                Style::default().fg(MUTED)
            };
            let suffix = if h.live { "" } else { "  (soon)" };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(format!(" {} ", h.key), key_style.bg(SELECT_BG)),
                Span::styled(format!("  {}{}", h.desc, suffix), desc_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use strop_core::Buffer;

    /// The space card is generated from keymap::BINDINGS: live leader
    /// rows, the git prefix row, soon rows muted with the suffix.
    #[test]
    fn space_card_lists_table_children() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ");
        let frame = crate::headless::frame_string(&mut e, 80, 24);
        for present in [
            "file finder",
            "buffers (MRU)",
            "live grep",
            "global search & replace",
            "this popup",
            "diagnostics picker",
            "hover docs",
            "paste clipboard before",
            "git…",
            "jumplist picker  (soon)",
            "undo-tree browser",
        ] {
            assert!(frame.contains(present), "space card missing {present:?}");
        }
    }

    /// The `space g` card carries the git verbs from the table.
    #[test]
    fn git_card_lists_verbs() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" g");
        let frame = crate::headless::frame_string(&mut e, 80, 24);
        for verb in [
            "commit browser",
            "file history",
            "blame",
            "permalink: copy",
            "permalink: open",
            "hunk: undo",
            "hunk: stage",
            "hunk: preview",
        ] {
            assert!(frame.contains(verb), "git card missing {verb:?}");
        }
    }

    /// Prefix cards without table children render nothing (visual mode
    /// has no leader verbs beyond `space y`).
    #[test]
    fn visual_space_card_only_y() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text("v ");
        let frame = crate::headless::frame_string(&mut e, 80, 24);
        assert!(frame.contains("yank selection → clipboard"));
        assert!(!frame.contains("file finder"));
    }
}
