//! Which-key overlay (0003 §3, 0088 §2): shows while a leader prefix
//! pends. Generated from keymap::BINDINGS (0003 §5.7) — no hand-
//! maintained hint lists to drift against dispatch. Rows are grouped in
//! the keymap's curated section order (the same order `Space ?` uses),
//! key ranges collapse (`1-9`, `a-z`), and ten or more rows flow into
//! two columns (U9).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::Editor;
use crate::keymap;

use super::card::{card_width, keycap, Card};
use super::clip::{clip_end, width};
use super::style::Tokens;

/// Pending prefixes that show a card: (pending keys, card title).
const PREFIXES: &[(&str, &str)] = &[
    (" ", "space"),
    (" w", "space w"),
    (" g", "space g"),
    ("g", "g"),
    ("]", "]"),
    ("[", "["),
    ("m", "mark"),
    ("'", "mark jump"),
    ("`", "mark jump"),
];

/// Ten rows or more flow into two columns.
const TWO_COLUMNS_AT: usize = 10;
const DESC_CAP: usize = 48;
/// Empty rows between the card and the echo row / statusline: both are
/// on the surface plane and would read as one block if they touched.
const MARGIN: u16 = 1;

pub fn render_which_key(editor: &Editor, t: &Tokens, frame: &mut Frame) {
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
    let rows = rows(&hints, &marks);
    let echo_rows = super::echo::rows(editor, t, frame.area().width);
    render_rows(frame, t, title, &rows, echo_rows);
}

struct Row {
    key: String,
    live: bool,
    desc: String,
}

/// Rows in curated section order with ranges collapsed: hints whose
/// keys are consecutive single digits (or letters) with one description
/// become one `1-9` row, the way `<a>` already reads as `a-z`.
fn rows(hints: &[keymap::Hint], marks: &[(char, usize, String)]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::with_capacity(hints.len());
    for section in keymap::SECTIONS {
        let mut range: Option<(char, char, usize)> = None; // (first, last, row index)
        for hint in hints.iter().filter(|hint| hint.section == section.id) {
            let single = hint
                .key
                .chars()
                .next()
                .filter(|_| hint.key.chars().count() == 1);
            if let (Some(ch), Some((first, last, at))) = (single, range) {
                if ch.is_ascii_digit() == first.is_ascii_digit()
                    && ch as u32 == last as u32 + 1
                    && rows[at].desc == hint.desc
                {
                    range = Some((first, ch, at));
                    rows[at].key = format!("{first}-{ch}");
                    continue;
                }
            }
            range = single
                .filter(|ch| ch.is_ascii_digit())
                .map(|ch| (ch, ch, rows.len()));
            rows.push(Row {
                key: if hint.key == "<a>" {
                    "a-z".into()
                } else {
                    hint.key.clone()
                },
                live: hint.live,
                desc: hint.desc.to_owned(),
            });
        }
    }
    if !marks.is_empty() {
        for (name, line, text) in marks {
            rows.push(Row {
                key: name.to_string(),
                live: true,
                desc: format!(":{line}  {text}"),
            });
        }
    }
    rows
}

/// A column's natural width: keycap(key + 2) + gap(1) + description.
fn column_width(rows: &[Row]) -> (usize, usize) {
    let key_w = rows.iter().map(|row| width(&row.key)).max().unwrap_or(1);
    let desc_w = rows
        .iter()
        .map(|row| width(&row.desc) + if row.live { 0 } else { 7 })
        .max()
        .unwrap_or(1)
        .min(DESC_CAP);
    (key_w, key_w + 2 + 1 + desc_w)
}

fn render_rows(frame: &mut Frame, t: &Tokens, title: &str, rows: &[Row], echo_rows: u16) {
    let area = frame.area();
    if area.width < 12 || area.height < 6 || rows.is_empty() {
        // nothing legible fits; a hard-clipped card would lie (0068 C1)
        return;
    }
    let (_, single_w) = column_width(rows);
    let columns = if rows.len() >= TWO_COLUMNS_AT && area.width as usize >= 2 * single_w + 9 {
        2
    } else {
        1
    };
    // borders (2) + the statusline + the empty row that parts the card
    // from it (0098 §3.4) + echo rows
    let reserved = 2 + 1 + MARGIN + echo_rows;
    let per_column = rows.len().div_ceil(columns);
    let body_budget = usize::from(area.height.saturating_sub(reserved)).max(1);
    let shown_per_column = if per_column > body_budget {
        body_budget.saturating_sub(1).max(1) // the footer takes a row
    } else {
        per_column
    };
    let overflow = rows.len().saturating_sub(shown_per_column * columns);
    let body_rows = shown_per_column + usize::from(overflow > 0);
    // each column is as wide as its own rows, not the widest overall
    let slices: Vec<&[Row]> = (0..columns)
        .map(|column| {
            let start = (column * shown_per_column).min(rows.len());
            let end = ((column + 1) * shown_per_column).min(rows.len());
            &rows[start..end]
        })
        .collect();
    let widths: Vec<(usize, usize)> = slices.iter().map(|rows| column_width(rows)).collect();
    let wanted = 4 + widths.iter().map(|(_, w)| w).sum::<usize>() + (columns - 1) * 3;
    let card_w = card_width(wanted, area.width).max((wanted as u16).min(area.width - 2));
    let height = body_rows as u16 + 2;
    let card = Rect {
        x: area.width.saturating_sub(card_w) / 2, // bottom-center
        y: area.height.saturating_sub(height + MARGIN + 1 + echo_rows),
        width: card_w,
        height,
    };
    let inner = Card::new(title).render(frame, card, t);
    let inner_w = usize::from(inner.width);
    let mut lines: Vec<Line> = Vec::with_capacity(body_rows);
    for index in 0..shown_per_column {
        let mut spans = Vec::new();
        let mut offset = 0usize;
        for (column, slice) in slices.iter().enumerate() {
            let Some(row) = slice.get(index) else {
                break;
            };
            let (key_w, column_w) = widths[column];
            if column > 0 {
                offset += widths[column - 1].1 + 3;
                let used: usize = spans.iter().map(|span: &Span| span.width()).sum();
                spans.push(Span::raw(" ".repeat(offset.saturating_sub(used))));
            }
            let desc_budget = column_w
                .min(inner_w.saturating_sub(offset))
                .saturating_sub(key_w + 2 + 1);
            let padded = format!("{:>key_w$}", row.key);
            if row.live {
                spans.push(keycap(&padded, t));
            } else {
                spans.push(Span::styled(
                    format!(" {padded} "),
                    Style::default().fg(t.muted).bg(t.raised),
                ));
            }
            let desc = if row.live {
                row.desc.clone()
            } else {
                format!("{} (soon)", row.desc)
            };
            spans.push(Span::styled(
                format!(" {}", clip_end(&desc, desc_budget)),
                if row.live {
                    Style::default().fg(t.text)
                } else {
                    Style::default().fg(t.muted).add_modifier(Modifier::ITALIC)
                },
            ));
        }
        lines.push(Line::from(spans));
    }
    if overflow > 0 {
        lines.push(Line::from(Span::styled(
            // in the key column, under the keys it continues
            format!(" +{overflow} more"),
            Style::default().fg(t.muted),
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
            "status: changed files",
            "commit (:wq commits",
            "branches: switch / create",
            "log",
            "file history",
            "blame gutter / card",
            "hunk: stage",
            "hunk: unstage",
            "hunk: discard",
            "hunk: preview",
            "permalink: copy",
            "permalink: open",
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

    /// No marks: the card shows the range row alone.
    #[test]
    fn mark_card_without_marks_shows_the_range() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text("m");
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
        assert!(frame.contains("a-z"), "{frame}");
        assert!(frame.contains("set mark at cursor"), "{frame}");
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

    /// U9: `space w` collapses the nine pane-focus digits into one row.
    #[test]
    fn pane_card_collapses_the_digit_range() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" w");
        let frame = crate::headless::frame_string(&mut e, 100, 24).unwrap();
        assert!(frame.contains("1-9"), "{frame}");
        for digit in 2..=9 {
            assert!(
                !frame.contains(&format!(" {digit}  pane")),
                "digit {digit} folded into the range: {frame}"
            );
        }
    }

    /// U9: the leader card at ten or more rows flows into two columns and
    /// orders its rows by keymap section — the leader verbs before git.
    #[test]
    fn leader_card_flows_into_two_columns_in_section_order() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ");
        let frame = crate::headless::frame_string(&mut e, 140, 40).unwrap();
        let hints = crate::keymap::children_of(" ", crate::editor::Mode::Normal).len();
        let body = frame
            .lines()
            .filter(|line| line.contains("│") && !line.contains("╭"))
            .count();
        assert!(
            body < hints,
            "two columns: {body} rows for {hints} hints\n{frame}"
        );
        let files = frame
            .lines()
            .position(|line| line.contains("file finder"))
            .unwrap();
        let git = frame
            .lines()
            .position(|line| line.contains("git…"))
            .unwrap();
        assert!(files <= git, "leader rows precede the git prefix: {frame}");
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
            wide.contains("commit (:wq commits, :q! aborts)"),
            "the full description fits: {wide}"
        );
        assert!(
            wide.contains("file history (visual: selected lines)"),
            "{wide}"
        );
        let narrow = crate::headless::frame_string(&mut e, 36, 16).unwrap();
        assert!(narrow.contains('…'), "narrow card ellipsizes: {narrow}");
        // the core invariant: a description is either complete or
        // ellipsized — never a mid-word cut against the border
        for line in narrow.lines() {
            if line.contains("commit (:wq") {
                assert!(
                    line.contains(":q! aborts)") || line.contains('…'),
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
        let frame = crate::headless::frame_string(&mut e, 60, 8).unwrap();
        assert!(frame.contains("more"), "overflow footer: {frame}");
    }

    /// 0098 §3.4: the card and the statusline share a plane, so one
    /// empty row parts them — above the echo rows when a message is live.
    #[test]
    fn the_card_hangs_one_row_above_the_statusline() {
        let border = |line: &str| line.contains('╰') || line.contains('│');
        for (columns, rows) in [(80, 24), (140, 40), (60, 8), (44, 12)] {
            let mut e = Editor::new(Buffer::from_text("x\n"));
            e.feed_text(" ");
            let frame = crate::headless::frame_string(&mut e, columns, rows).unwrap();
            let lines: Vec<&str> = frame.lines().collect();
            let last = lines.len() - 1;
            assert!(lines[last].contains("PENDING"), "{frame}");
            assert!(!border(lines[last - 1]), "{columns}x{rows}: {frame}");
            assert!(lines[last - 2].contains('╰'), "{columns}x{rows}: {frame}");
        }
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.set_message("a live message");
        e.feed_text(" ");
        let frame = crate::headless::frame_string(&mut e, 80, 24).unwrap();
        if let Some(echo) = frame
            .lines()
            .position(|line| line.contains("a live message"))
        {
            let lines: Vec<&str> = frame.lines().collect();
            assert!(!border(lines[echo - 1]), "{frame}");
            assert!(lines[echo - 2].contains('╰'), "{frame}");
        }
    }

    /// 0098 §3.4: the overflow footer starts in the key column.
    #[test]
    fn the_overflow_footer_lines_up_with_the_keys() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ");
        let frame = crate::headless::frame_string(&mut e, 60, 10).unwrap();
        let lines: Vec<&str> = frame.lines().collect();
        let footer = lines.iter().position(|line| line.contains("more")).unwrap();
        let cells = |line: &str, at: usize| line[..at].chars().count();
        let plus = cells(lines[footer], lines[footer].find('+').unwrap());
        let key = lines[footer - 1]
            .char_indices()
            .skip_while(|(_, ch)| *ch != '│')
            .skip(1)
            .find(|(_, ch)| *ch != ' ')
            .map(|(at, _)| cells(lines[footer - 1], at))
            .unwrap();
        assert_eq!(plus, key, "{frame}");
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
