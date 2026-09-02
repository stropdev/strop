//! Which-key overlay (0003 §3): shows while a leader prefix pends.
//! v1 covers the Space namespace statically; the coverage-tested,
//! binding-table-driven version lands with the keymap tables (0003 §5.7).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;

use super::{ACCENT, BASE, MUTED, TEXT};

/// The Space namespace, as it will exist: available now vs. planned.
const SPACE_HINTS: &[(&str, &str, bool)] = &[
    ("f", "file finder", true),
    ("b", "buffers (MRU)", true),
    ("/", "live grep", true),
    ("j", "jumplist", false),
    ("s", "symbols", false),
    ("d", "diagnostics", true),
    ("k", "hover", true),
    ("u", "undo tree", false),
    ("g", "git…", true),
    ("?", "keybindings", true),
];

const GIT_HINTS: &[(&str, &str, bool)] = &[
    ("u", "undo hunk (reset to HEAD)", true),
    ("s", "stage hunk", true),
    ("p", "preview hunk", true),
    ("l", "commit browser", true),
    ("h", "file history", true),
    ("b", "blame", true),
    ("y", "copy permalink", true),
    ("o", "open permalink", true),
];

const MARK_HINTS: &[(&str, &str, bool)] = &[
    ("a–z", "set mark at cursor", true),
    ("'a", "jump to mark", true),
];

const G_HINTS: &[(&str, &str, bool)] =
    &[("g", "top of file", true), ("d", "goto definition", true)];

const BRACKET_HINTS: &[(&str, &str, bool)] = &[("c", "hunk", true)];

pub fn render_which_key(editor: &Editor, frame: &mut Frame) {
    if editor.picker_open() || editor.keybinds_open {
        return;
    }
    match editor.pending.as_str() {
        " g" => render_hints(frame, " space g ", GIT_HINTS),
        "m" => render_hints(frame, " mark ", MARK_HINTS),
        "'" | "`" => render_hints(frame, " mark jump ", &MARK_HINTS[1..]),
        "g" => render_hints(frame, " g ", G_HINTS),
        "]" => render_hints(frame, " ] ", BRACKET_HINTS),
        "[" => render_hints(frame, " [ ", BRACKET_HINTS),
        " " => render_hints(frame, " space ", SPACE_HINTS),
        _ => {}
    }
}

fn render_hints(frame: &mut Frame, title: &str, hints: &[(&str, &str, bool)]) {
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
        .map(|(key, desc, available)| {
            let key_style = if *available {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            };
            let desc_style = if *available {
                Style::default().fg(TEXT)
            } else {
                Style::default().fg(MUTED)
            };
            let suffix = if *available { "" } else { "  (soon)" };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(format!(" {key} "), key_style.bg(super::SELECT_BG)),
                Span::styled(format!("  {desc}{suffix}"), desc_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
