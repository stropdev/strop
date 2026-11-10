//! The command/search card (noice.nvim lineage): `:` and `/` float as a
//! top-center card instead of squatting in the statusline — what you're
//! typing deserves focus. Match count rides along on search.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::Editor;

use super::card::Card;
use super::style::Tokens;

pub fn render_cmd_card(editor: &Editor, t: &Tokens, frame: &mut Frame) {
    // `?` is a first-class search sigil here (the old detection missed
    // it: backward searches never got the card)
    let Some(kind) = editor.pending_sigil() else {
        return;
    };
    if !matches!(kind, ':' | '/' | '?' | '|') {
        return;
    }
    if editor.picker_open() {
        return;
    }
    let pending = editor.pending().text();
    let substitution = if kind == ':' {
        editor.substitute_preview()
    } else {
        None
    };
    let changed_row_count = match &substitution {
        Some(crate::editor::SubstitutePreviewState::Ready(preview)) => preview
            .diff_rows()
            .iter()
            .filter(|row| {
                matches!(
                    row,
                    crate::editor::ReviewRow::Removed | crate::editor::ReviewRow::Added
                )
            })
            .count(),
        _ => 0,
    };
    let changed_rows = if changed_row_count > 8 {
        9 // first four, an elision, and the last four (offscreen included)
    } else {
        changed_row_count
    };
    let substitution_height = usize::from(substitution.is_some()) + changed_rows;

    let area = frame.area();
    // ex completion rides along: candidates under the input (0003 §1)
    let candidates = if kind == ':' {
        editor.ex_candidates()
    } else {
        Vec::new()
    };
    // A typed field must not resize under the caret, so the card takes
    // rule 9's cap (60 % of the columns, ≥ 32) rather than its content.
    let width = ((u32::from(area.width) * 60 / 100) as u16)
        .max(32)
        .min(area.width.saturating_sub(4));
    // The card lives between the top row and the statusline and never
    // covers either (0091 B2). With fewer than three rows there is no room
    // for a frame: the field alone takes the top row (0091 B6).
    if area.height == 0 || area.width == 0 {
        return;
    }
    let height = (3 + candidates.len().min(6) as u16 + substitution_height as u16)
        .min(area.height.saturating_sub(2));
    let card = Rect {
        x: (area.width.saturating_sub(width)) / 2,
        y: 1,
        width,
        height,
    };
    let title = match kind {
        ':' => "command".to_owned(),
        '|' => "pipe".to_owned(),
        _ => "search".to_owned(),
    };
    let title = editor
        .pending_field_state()
        .filter(|state| {
            state.mode == crate::editor::PendingFieldMode::Normal && state.outer_operator.is_some()
        })
        .map_or(title, |state| {
            format!(
                "{} · field NORMAL · Esc cancels",
                state.outer_operator.expect("filtered operator").key()
            )
        });
    let inner = if height >= 3 {
        Card::new(&title).render(frame, card, t)
    } else {
        let row = Rect {
            y: 0,
            height: 1,
            ..card
        };
        frame.render_widget(ratatui::widgets::Clear, row);
        frame.render_widget(
            ratatui::widgets::Block::default().style(Style::default().bg(t.surface)),
            row,
        );
        row
    };

    // body: the payload, not the prefix — ":w" shows "w", "/foo" shows "foo"
    let body = pending.strip_prefix(kind).unwrap_or(pending);
    let text_area = Rect {
        height: inner.height.min(1),
        ..inner
    };
    let caret_byte = editor
        .pending()
        .cursor()
        .saturating_sub(kind.len_utf8())
        .min(body.len());
    let highlights = if matches!(kind, '/' | '?') {
        strop_picker::query::highlight::buffer_search(body)
    } else {
        Vec::new()
    };
    let field = super::field::project(
        t,
        &kind.to_string(),
        body,
        caret_byte,
        true,
        text_area.width,
        &highlights,
    );
    let caret = field.cursor;
    let mut spans = field.line.spans;

    // search rides with a live match count — right-aligned inside the
    // field's width instead of appended past it (0068 C2: inline
    // append truncated hard)
    if matches!(kind, '/' | '?') {
        let label = match editor.current_search_query() {
            Ok(Some(query)) => match editor.search_summary(&query) {
                Some(Ok(summary)) => format!(
                    "{} match{}",
                    summary.count,
                    if summary.count == 1 { "" } else { "es" }
                ),
                Some(Err(error)) => error.to_string(),
                None => "searching…".into(),
            },
            Ok(None) => "0 matches".into(),
            Err(error) => error.to_string(),
        };
        let label = super::clip::clip_end(&label, usize::from(text_area.width) / 3);
        let label_w = super::clip::width(&label);
        let used: usize = spans.iter().map(|span| span.width()).sum();
        let pad = usize::from(text_area.width).saturating_sub(used + label_w + 1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(
            label.into_owned(),
            Style::default().fg(t.muted),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), text_area);

    // completion rows: first candidate accent (Tab cycles to it), the
    // rest muted with their doc strings. The name column sizes to the
    // shown candidates (0068 C2: a fixed 10 overflowed into the doc
    // column), names and docs ellipsize — nothing touches the border.
    let body_str = pending.strip_prefix(':').unwrap_or("");
    let shown: Vec<_> = candidates.iter().take(6).collect();
    let name_w = shown
        .iter()
        .map(|(name, _)| super::clip::width(name))
        .max()
        .unwrap_or(4)
        .clamp(4, 16);
    for (i, (name, doc)) in shown.iter().enumerate() {
        let y = text_area.y + 1 + i as u16;
        if y >= inner.y + inner.height {
            break;
        }
        let row = Rect {
            y,
            height: 1,
            ..text_area
        };
        let (name_fg, doc_fg) = if i == 0 {
            (t.accent, t.text)
        } else {
            (t.text, t.muted)
        };
        let marker = if name == &body_str { "▌" } else { " " };
        let doc_budget = usize::from(text_area.width).saturating_sub(1 + name_w + 2 + 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(t.accent)),
                Span::styled(
                    format!("{:<name_w$}  ", super::clip::clip_end(name, name_w)),
                    Style::default().fg(name_fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    super::clip::clip_end(doc, doc_budget).into_owned(),
                    Style::default().fg(doc_fg),
                ),
            ])),
            row,
        );
    }

    if let Some(state) = &substitution {
        let y = text_area.y + 1 + shown.len() as u16;
        if y < inner.y + inner.height {
            let (label, color) = match state {
                crate::editor::SubstitutePreviewState::Ready(preview) => {
                    (preview.summary(), t.muted)
                }
                crate::editor::SubstitutePreviewState::Resolving => {
                    ("resolving substitution…".into(), t.muted)
                }
                crate::editor::SubstitutePreviewState::NoTarget => {
                    ("0 substitutions · pattern not found".into(), t.muted)
                }
                crate::editor::SubstitutePreviewState::Incomplete(error) => {
                    (error.clone(), t.muted)
                }
                crate::editor::SubstitutePreviewState::Invalid(error) => {
                    (format!("invalid · {error}"), t.error)
                }
                crate::editor::SubstitutePreviewState::Unavailable(error) => {
                    (format!("unavailable · {error}"), t.error)
                }
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(label, Style::default().fg(color)))),
                Rect {
                    y,
                    height: 1,
                    ..text_area
                },
            );
        }
        if let crate::editor::SubstitutePreviewState::Ready(preview) = state {
            let lines: Vec<(&str, &crate::editor::ReviewRow)> = preview
                .diff_text()
                .lines()
                .zip(preview.diff_rows())
                .filter(|(_, row)| {
                    matches!(
                        row,
                        crate::editor::ReviewRow::Removed | crate::editor::ReviewRow::Added
                    )
                })
                .collect();
            let windows = changed_windows(&lines, usize::from(text_area.width));
            let changed = windows
                .into_iter()
                .zip(lines.iter().map(|(_, row)| *row))
                .enumerate()
                .filter(|(index, _)| {
                    changed_row_count <= 8
                        || *index < 4
                        || index.saturating_add(4) >= changed_row_count
                })
                .take(8);
            for (index, (_, (line, row))) in changed.enumerate() {
                let elided = changed_row_count > 8 && index >= 4;
                let y = y + 1 + index as u16 + u16::from(elided);
                if changed_row_count > 8 && index == 4 && y - 1 < inner.y + inner.height {
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled("…", Style::default().fg(t.muted)))),
                        Rect {
                            y: y - 1,
                            height: 1,
                            ..text_area
                        },
                    );
                }
                if y >= inner.y + inner.height {
                    break;
                }
                let color = match row {
                    crate::editor::ReviewRow::Removed => t.diff_del,
                    crate::editor::ReviewRow::Added => t.diff_add,
                    _ => unreachable!("filtered changed rows"),
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(line, Style::default().fg(color)))),
                    Rect {
                        y,
                        height: 1,
                        ..text_area
                    },
                );
            }
        }
    }

    if let Some(column) = caret {
        if column < text_area.width && text_area.height != 0 {
            crate::render::frame_capture::place_cursor(frame, (text_area.x + column, text_area.y));
        }
    }
}

/// Each changed line cut to `cells`, with its change in view (0091 B2). A
/// removed line and the added line after it are one pair and share one
/// window, placed on the first character where they differ, so the eye
/// compares the same columns. `…` marks text cut on either side.
fn changed_windows(lines: &[(&str, &crate::editor::ReviewRow)], cells: usize) -> Vec<String> {
    let mut windows = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let pair = matches!(lines[index].1, crate::editor::ReviewRow::Removed)
            && lines
                .get(index + 1)
                .is_some_and(|next| matches!(next.1, crate::editor::ReviewRow::Added));
        let members = if pair { 2 } else { 1 };
        let widest = lines[index..index + members]
            .iter()
            .map(|(line, _)| super::clip::width(line))
            .max()
            .unwrap_or(0);
        // characters after the sign column the pair has in common
        let common = if pair {
            lines[index]
                .0
                .chars()
                .skip(1)
                .zip(lines[index + 1].0.chars().skip(1))
                .take_while(|(old, new)| old == new)
                .count()
        } else {
            0
        };
        // keep a third of the row as context before the change
        let skip = if widest <= cells {
            0
        } else {
            common.saturating_sub(cells / 3)
        };
        for (line, _) in &lines[index..index + members] {
            windows.push(window(line, skip, cells));
        }
        index += members;
    }
    windows
}

/// `line` with its sign kept and `skip` characters after it cut away.
fn window(line: &str, skip: usize, cells: usize) -> String {
    let mut chars = line.chars();
    let mut text: String = chars.next().into_iter().collect();
    if skip > 0 {
        text.push('…');
        text.extend(chars.skip(skip + 1));
    } else {
        text.extend(chars);
    }
    super::clip::clip_end(&text, cells).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    fn tokens() -> Tokens {
        Tokens::new(strop_core::theme::ThemeId::Hone)
    }

    fn draw_search(prompt: &str) -> ratatui::buffer::Buffer {
        let mut editor = Editor::new(Buffer::from_text("Needle\nneedle\n"));
        editor.feed_text(prompt);
        let backend = ratatui::backend::TestBackend::new(80, 8);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_cmd_card(&editor, &tokens(), frame))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn search_start(buffer: &ratatui::buffer::Buffer, text: &str) -> u16 {
        let start = (0..buffer.area.width)
            .find(|x| buffer[(*x, 2)].symbol() == text.chars().next().unwrap().to_string())
            .unwrap();
        let rendered: String = (start..start + text.len() as u16)
            .map(|x| buffer[(x, 2)].symbol())
            .collect();
        assert_eq!(rendered, text);
        start
    }

    #[test]
    fn buffer_case_and_picker_language_share_field_styles() {
        let t = tokens();
        let picker = strop_picker::query::SearchQuery::parse("language:cpp");
        let picker = super::super::field::project(
            &t,
            "/",
            "language:cpp",
            "language:cpp".len(),
            true,
            40,
            &picker.highlights,
        );
        let picker_style = |content: &str| {
            picker
                .line
                .spans
                .iter()
                .find(|span| span.content == content)
                .unwrap()
                .style
        };

        for sigil in ['/', '?'] {
            let grid = draw_search(&format!("{sigil}case:sensitiveNeedle"));
            let start = search_start(&grid, "case:sensitive");
            for x in start..start + 4 {
                assert_eq!(grid[(x, 2)].fg, picker_style("l").fg.unwrap());
                assert_eq!(grid[(x, 2)].fg, t.accent);
                assert!(grid[(x, 2)].modifier.contains(Modifier::BOLD));
            }
            assert_eq!(grid[(start + 4, 2)].fg, picker_style(":").fg.unwrap());
            assert_eq!(grid[(start + 4, 2)].fg, t.muted);
            for x in start + 5..start + 14 {
                assert_eq!(grid[(x, 2)].fg, picker_style("c").fg.unwrap());
                assert_eq!(grid[(x, 2)].fg, t.text);
            }
        }
    }

    #[test]
    fn mid_pattern_case_directive_renders_as_literal_text() {
        let grid = draw_search("/needlecase:sensitive");
        let start = search_start(&grid, "needlecase:sensitive");
        for x in start..start + "needlecase:sensitive".len() as u16 {
            assert_eq!(grid[(x, 2)].fg, tokens().text);
            assert!(!grid[(x, 2)].modifier.contains(Modifier::BOLD));
        }
    }

    fn draw_ex(editor: &Editor, rows: u16) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(100, rows);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_cmd_card(editor, &tokens(), frame))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &ratatui::buffer::Buffer, row: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol())
            .collect()
    }

    #[test]
    fn substitute_card_shows_real_before_after_cells_then_invalidates_them() {
        let original = "alpha one\nalpha two alpha\n";
        let mut editor = Editor::new(Buffer::from_text(original));
        editor.feed_text(":%s/alpha/omega/g");
        let grid = draw_ex(&editor, 14);
        assert!(row_text(&grid, 3).contains("3 substitutions"));
        assert!(row_text(&grid, 3).contains("2 changed lines"));
        assert!(row_text(&grid, 4).contains("-alpha one"));
        assert!(row_text(&grid, 5).contains("+omega one"));
        assert!(row_text(&grid, 6).contains("-alpha two alpha"));
        assert!(row_text(&grid, 7).contains("+omega two omega"));
        assert_eq!(editor.buf().text().to_string(), original);

        editor.feed_text("x");
        let grid = draw_ex(&editor, 14);
        assert!(row_text(&grid, 3).contains("invalid · trailing characters: x"));
        assert!(!row_text(&grid, 4).contains("+omega"));
        assert_eq!(editor.buf().text().to_string(), original);
    }

    /// 0091 B2: the change is in view however far right it is, the removed
    /// and added rows show the same columns, and the statusline row below
    /// the card is never drawn over.
    #[test]
    fn substitute_card_scrolls_to_the_change_and_leaves_the_statusline() {
        let long = format!(
            "{}needle = old_value; // tail",
            "let padding = 1; ".repeat(8)
        );
        let mut editor = Editor::new(Buffer::from_text(&format!("{long}\nshort old_value\n")));
        editor.feed_text(":%s/old_value/fresh/g");
        let backend = ratatui::backend::TestBackend::new(90, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_cmd_card(&editor, &tokens(), frame))
            .unwrap();
        let grid = terminal.backend().buffer().clone();
        let removed = row_text(&grid, 4);
        let added = row_text(&grid, 5);
        assert!(removed.contains("-…"), "cut on the left: {removed}");
        assert!(removed.contains("needle = old_value;"), "{removed}");
        assert!(added.contains("needle = fresh;"), "{added}");
        assert_eq!(
            removed.find("needle"),
            added.find("needle"),
            "one window for the pair"
        );
        // a line that fits is not cut
        assert!(row_text(&grid, 6).contains("-short old_value"));
        assert!(row_text(&grid, 7).contains("+short fresh"));
    }

    /// 0091 B6: no terminal is too small to type a command in. The card
    /// keeps off the statusline row, and with no room for a frame the
    /// field alone takes the top row.
    #[test]
    fn the_command_card_never_reaches_the_statusline_at_any_height() {
        let text = (0..30)
            .map(|line| format!("edge {line}\n"))
            .collect::<String>();
        for rows in 1..=12u16 {
            let mut editor = Editor::new(Buffer::from_text(&text));
            editor.feed_text(":%s/edge/rim/g");
            let grid = {
                let backend = ratatui::backend::TestBackend::new(90, rows);
                let mut terminal = ratatui::Terminal::new(backend).unwrap();
                terminal
                    .draw(|frame| render_cmd_card(&editor, &tokens(), frame))
                    .unwrap();
                terminal.backend().buffer().clone()
            };
            if rows >= 2 {
                let last = row_text(&grid, rows - 1);
                assert!(
                    last.trim().is_empty(),
                    "{rows} rows: statusline row drawn over: {last}"
                );
            }
            let typed = (0..rows).any(|row| row_text(&grid, row).contains("%s/edge/rim/g"));
            assert!(typed, "{rows} rows: the field is visible");
        }
    }

    #[test]
    fn substitute_card_reports_offscreen_extent_and_outer_field_mode() {
        let original = (0..30)
            .map(|line| format!("alpha {line}\n"))
            .collect::<String>();
        let mut editor = Editor::new(Buffer::from_text(&original));
        editor.feed_text(":%s/alpha/omega/g");
        let grid = draw_ex(&editor, 15);
        assert!(row_text(&grid, 3).contains("30 substitutions"));
        assert!(row_text(&grid, 3).contains("6 offscreen"));
        assert!(row_text(&grid, 8).contains('…'));
        assert!(row_text(&grid, 12).contains("+omega 29"));

        let mut editor = Editor::new(Buffer::from_text("alpha\n"));
        editor.feed_text("d/alpha");
        assert!(!editor.input_normal());
        editor.feed(strop_core::frontend_input::Key::Esc);
        assert_eq!(editor.pending_sigil(), Some('/'));
        assert!(editor.pending().is_active());
        assert!(!editor.picker_open());
        let grid = draw_ex(&editor, 10);
        let status = row_text(&grid, 1);
        assert!(
            status.contains("d · field NORMAL · Esc cancels"),
            "{status}"
        );
        editor.feed(strop_core::frontend_input::Key::Esc);
        assert_eq!(editor.buf().text().to_string(), "alpha\n");
    }
}
