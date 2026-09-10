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
    let prefix = editor.walker.prefix_display();
    let pending = prefix.as_str();
    let Some(&(_, title)) = PREFIXES.iter().find(|(p, _)| *p == pending) else {
        return;
    };
    let hints = keymap::children_of(pending, editor.mode);
    if hints.is_empty() {
        return;
    }
    // Mark prefixes list the live marks below the static hint (0047 §3):
    // for `'`/` the list IS the menu; for `m` it shows what a letter
    // would overwrite.
    let marks = if matches!(pending, "m" | "'" | "`") {
        editor.mark_rows()
    } else {
        Vec::new()
    };
    render_hints(frame, title, &hints, &marks);
}

fn render_hints(
    frame: &mut Frame,
    title: &str,
    hints: &[keymap::Hint],
    marks: &[(char, usize, String)],
) {
    let area = frame.area();
    let width = 40u16.min(area.width.saturating_sub(4));
    let extra = if marks.is_empty() { 1 } else { marks.len() } as u16;
    let extra = if title.contains("mark") { extra } else { 0 };
    let height = (hints.len() as u16 + extra + 2).min(area.height.saturating_sub(2));
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

    let mut lines: Vec<Line> = hints
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
            // Absorb tokens read as jargon (`<a>`); the card speaks
            // the range the user can type.
            let key = if h.key == "<a>" { "a-z" } else { &h.key };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(format!(" {} ", key), key_style.bg(SELECT_BG)),
                Span::styled(format!("  {}{}", h.desc, suffix), desc_style),
            ])
        })
        .collect();
    if title.contains("mark") {
        if marks.is_empty() {
            lines.push(Line::from(Span::styled(
                "  no marks set",
                Style::default().fg(MUTED),
            )));
        } else {
            for (name, line, text) in marks {
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        format!(" {} ", name),
                        Style::default()
                            .fg(ACCENT)
                            .add_modifier(Modifier::BOLD)
                            .bg(SELECT_BG),
                    ),
                    Span::styled(format!("  :{}  {}", line, text), Style::default().fg(MUTED)),
                ]));
            }
        }
    }
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
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
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
            "jumplist picker",
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
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
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

    /// The `m` card speaks plainly: `a-z` (never `<a>`), plus the live
    /// marks with their landing lines — for `'` the list is the menu.
    #[test]
    fn mark_card_lists_live_marks() {
        let mut e = Editor::new(Buffer::from_text(
            "fn main() {}\nlet x = 1;\nfn helper() {}\n",
        ));
        e.feed_text("jma"); // mark a on line 2
        e.feed_text("jmb"); // mark b on line 3
        e.feed_text("ggm"); // back to top, pend m
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
        assert!(
            frame.contains("a-z"),
            "absorb label reads as a range: {frame}"
        );
        assert!(!frame.contains("<a>"), "no table token leaks: {frame}");
        assert!(frame.contains("set mark at cursor"), "{frame}");
        assert!(frame.contains(":2  let x = 1;"), "mark a row: {frame}");
        assert!(frame.contains(":3  fn helper() {}"), "mark b row: {frame}");
        e.feed(crate::editor::Key::Esc);
        e.feed_text("'");
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
        assert!(frame.contains("jump to mark"), "{frame}");
        assert!(frame.contains(":2  let x = 1;"), "{frame}");
    }

    /// No marks: the card says so instead of showing an empty list.
    #[test]
    fn mark_card_without_marks_says_so() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text("m");
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
        assert!(frame.contains("no marks set"), "{frame}");
        assert!(frame.contains("a-z"), "{frame}");
    }

    /// Prefix cards without table children render nothing (visual mode
    /// has no leader verbs beyond `space y`).
    #[test]
    fn visual_space_card_only_y() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text("v ");
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
        assert!(frame.contains("yank selection → clipboard"));
        assert!(!frame.contains("file finder"));
    }
}
