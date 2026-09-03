//! The keybinds popup (`Space ?`): sidebar of section chips with counts,
//! keycap rows for the active section, version in the title — the rootle
//! keybinds_popup shape. Renders from keymap::BINDINGS (0003 §5.7: the
//! popup is generated from the same table; no hand-maintained help).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::editor::Editor;
use crate::keymap::{BINDINGS, SECTIONS};

use super::{ACCENT, BASE, MUTED, SELECT_BG, TEXT};

pub fn render_keybinds(editor: &Editor, frame: &mut Frame) {
    if !editor.keybinds_open {
        return;
    }
    let area = frame.area();
    let width = (area.width * 78 / 100).clamp(56, area.width.saturating_sub(4));
    let height = (area.height * 72 / 100).clamp(14, area.height.saturating_sub(4));
    let card = Rect {
        x: (area.width - width) / 2,
        y: (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(BASE))
        .title(Span::styled(
            " keybindings ",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_top(
            Line::from(Span::styled(
                format!(" strop {} ", env!("CARGO_PKG_VERSION")),
                Style::default().fg(MUTED),
            ))
            .right_aligned(),
        )
        .title_bottom(Span::styled(
            " tab/h/l section · j/k scroll · esc close ",
            Style::default().fg(MUTED),
        ));
    frame.render_widget(&block, card);
    let inner = block.inner(card);
    let inner = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(16), Constraint::Min(1)])
        .split(inner);

    // sidebar: section chips, active filled (0003 §5.4)
    let mut side: Vec<Line> = vec![Line::from("")];
    for (i, s) in SECTIONS.iter().enumerate() {
        let active = i == editor.keybinds_section;
        let count = BINDINGS.iter().filter(|b| b.section == *s).count();
        let bg = if active { SELECT_BG } else { BASE };
        side.push(Line::from(vec![
            Span::styled(
                if active { "▸ " } else { "  " },
                Style::default().fg(ACCENT).bg(bg),
            ),
            Span::styled(
                format!("{s:<10}"),
                if active {
                    Style::default()
                        .fg(TEXT)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(MUTED).bg(bg)
                },
            ),
            Span::styled(format!("{count}"), Style::default().fg(MUTED).bg(bg)),
        ]));
    }
    frame.render_widget(Paragraph::new(side), cols[0]);

    // rows: keycap chips + descriptions, scrolled. Planned slots
    // (live:false) render in their own muted subsection at the bottom —
    // never styled as live bindings (0003 §5.7).
    let section = SECTIONS[editor.keybinds_section];
    let mut rows: Vec<Line> = Vec::new();
    let mut planned: Vec<Line> = Vec::new();
    for b in BINDINGS.iter().filter(|b| b.section == section) {
        let line = if b.live {
            Line::from(vec![
                Span::styled(
                    format!(" {:<18}", b.keys),
                    Style::default()
                        .fg(ACCENT)
                        .bg(SELECT_BG)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", b.desc), Style::default().fg(TEXT)),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!(" {:<18}", b.keys),
                    Style::default().fg(MUTED).bg(BASE),
                ),
                Span::styled(format!("  {}  (soon)", b.desc), Style::default().fg(MUTED)),
            ])
        };
        if b.live {
            rows.push(line);
        } else {
            planned.push(line);
        }
    }
    if !planned.is_empty() {
        rows.push(Line::from(Span::styled(
            " planned",
            Style::default().fg(MUTED),
        )));
        rows.extend(planned);
    }
    let rows: Vec<Line> = rows
        .into_iter()
        .skip(editor.keybinds_scroll)
        .take(cols[1].height as usize)
        .collect();
    frame.render_widget(Paragraph::new(rows), cols[1]);
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use strop_core::Buffer;

    /// The popup renders table rows; planned slots land in their own
    /// muted subsection with the (soon) suffix — never styled live.
    /// Normal is the default section (it owns the n/N soon row); tab
    /// walks to leader for its pair.
    #[test]
    fn popup_sections_and_planned_slots() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ?");
        let frame = crate::headless::frame_string(&mut e, 120, 30);
        assert!(frame.contains("keybindings"));
        assert!(frame.contains("word / WORD motions"));

        e.feed(crate::editor::Key::Tab);
        e.feed(crate::editor::Key::Tab);
        e.feed(crate::editor::Key::Tab); // normal → visual → insert → leader
        let frame = crate::headless::frame_string(&mut e, 120, 30);
        assert!(frame.contains("file finder"));
        assert!(frame.contains("planned"));
        assert!(frame.contains("jumplist picker  (soon)"));
        assert!(frame.contains("undo-tree browser"));
    }
}
