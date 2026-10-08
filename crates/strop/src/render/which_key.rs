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

use super::{accent, base, muted, select_bg, text};

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
    let prefix = editor.walker().prefix_display();
    let pending = prefix.as_str();
    let Some(&(_, title)) = PREFIXES.iter().find(|(p, _)| *p == pending) else {
        return;
    };
    let hints = keymap::children_of(pending, editor.mode());
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
    if area.width < 12 || area.height < 6 {
        // nothing legible fits; a hard-clipped card would lie (0068 C1)
        return;
    }
    // which-key discipline (which-key.nvim: the card breathes): bottom-
    // center, fitted to content within bounds, one key column, 1-cell
    // inner padding, descriptions ellipsized — never a mid-word cut
    // against the border, and a "+N more" footer instead of a silent cut.
    struct Row {
        key: String,
        key_live: bool,
        desc: String,
        desc_live: bool,
    }
    let mut rows: Vec<Row> = hints
        .iter()
        .map(|h| Row {
            // Absorb tokens read as jargon (`<a>`); the card speaks the
            // range the user can type.
            key: if h.key == "<a>" {
                "a-z".into()
            } else {
                h.key.clone()
            },
            key_live: h.live,
            desc: format!("{}{}", h.desc, if h.live { "" } else { "  (soon)" }),
            desc_live: h.live,
        })
        .collect();
    if title.contains("mark") {
        if marks.is_empty() {
            rows.push(Row {
                key: String::new(),
                key_live: false,
                desc: "no marks set".into(),
                desc_live: false,
            });
        } else {
            for (name, line, text) in marks {
                rows.push(Row {
                    key: name.to_string(),
                    key_live: true,
                    desc: format!(":{line}  {text}"),
                    desc_live: false,
                });
            }
        }
    }
    let key_w = rows
        .iter()
        .map(|row| super::clip::width(&row.key))
        .max()
        .unwrap_or(1)
        .max(1);
    const DESC_CAP: usize = 60;
    let desc_w = rows
        .iter()
        .map(|row| super::clip::width(&row.desc))
        .max()
        .unwrap_or(1)
        .min(DESC_CAP);
    // borders(2) + inner pad(2) + chip(key_w+2) + gap(2) + description
    let wanted = 2 + 2 + (key_w + 2) + 2 + desc_w;
    let max_w = area.width.saturating_sub(2).max(4);
    let width = (wanted as u16).min(max_w).max(20.min(max_w));
    let height = (rows.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let card = Rect {
        x: area.width.saturating_sub(width) / 2, // bottom-center
        y: area.height.saturating_sub(height + 1),
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted()))
        .style(Style::default().bg(base()))
        .title(Span::styled(
            title,
            Style::default().fg(text()).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    // 1-cell left pad + chip + 2-cell gap + 1-cell right pad: no text
    // ever touches a border, ellipsis included
    let desc_budget = usize::from(inner.width).saturating_sub(1 + (key_w + 2) + 2 + 1);
    let visible = usize::from(inner.height);
    let overflow = rows.len().saturating_sub(visible);
    let shown = if overflow > 0 {
        visible.saturating_sub(1)
    } else {
        rows.len()
    };
    let mut lines: Vec<Line> = Vec::with_capacity(shown + 1);
    for row in rows.iter().take(shown) {
        if row.key.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(" {}", row.desc),
                Style::default().fg(muted()),
            )));
            continue;
        }
        let key_style = if row.key_live {
            Style::default().fg(accent()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(muted())
        };
        let desc_style = if row.desc_live {
            Style::default().fg(text())
        } else {
            Style::default().fg(muted())
        };
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(format!(" {:>key_w$} ", row.key), key_style.bg(select_bg())),
            Span::styled(
                format!("  {}", super::clip::clip_end(&row.desc, desc_budget)),
                desc_style,
            ),
        ]));
    }
    if overflow > 0 {
        lines.push(Line::from(Span::styled(
            format!("  +{overflow} more"),
            Style::default().fg(muted()),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use strop_core::Buffer;

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
        e.feed(strop_core::frontend_input::Key::Esc);
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

    /// 0068 C1 (the user complaint): descriptions never cut mid-word
    /// against the border — full text at a comfortable width, an
    /// ellipsis at a narrow one.
    #[test]
    fn descriptions_never_clip_mid_word() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" g");
        let wide = crate::headless::frame_string(&mut e, 100, 24).unwrap();
        assert!(
            wide.contains("hunk: undo unstaged (restore from index)"),
            "the full description fits: {wide}"
        );
        assert!(
            wide.contains("file history (visual: selected lines)"),
            "{wide}"
        );
        let narrow = crate::headless::frame_string(&mut e, 48, 16).unwrap();
        assert!(narrow.contains('…'), "narrow card ellipsizes: {narrow}");
        // the core invariant: a description is either complete or
        // ellipsized — never a mid-word cut against the border
        for line in narrow.lines() {
            if line.contains("undo unstaged") {
                assert!(
                    line.contains("restore from index") || line.contains('…'),
                    "no hard truncation: {line}"
                );
            }
        }
    }

    /// 0068 C1: more hints than rows read "+N more" — never a silent cut.
    #[test]
    fn overflow_reads_a_more_footer() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ");
        let frame = crate::headless::frame_string(&mut e, 100, 8).unwrap();
        assert!(frame.contains("more"), "overflow footer: {frame}");
    }

    /// 0068 C1: the card centers horizontally instead of hugging the
    /// right edge (which-key convention).
    #[test]
    fn the_card_centers_horizontally() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" g");
        let frame = crate::headless::frame_string(&mut e, 100, 24).unwrap();
        let title_row = frame
            .lines()
            .find(|line| line.contains("space g"))
            .expect("the card title row");
        let left = title_row.chars().position(|c| c == '╭').unwrap();
        let right = title_row
            .char_indices()
            .rev()
            .find(|(_, c)| *c == '╮')
            .map(|(index, _)| title_row[..index].chars().count())
            .unwrap();
        let right_margin = 100usize.saturating_sub(right + 1);
        assert!(
            left.abs_diff(right_margin) <= 2,
            "centered: left {left} vs right margin {right_margin}"
        );
    }
}
