//! The picker card (0003 §2, 0088 §2): find-file/symbols/grep own one
//! stable near-full-frame workspace over a dimmed backdrop — input top,
//! results left, the file preview right with width reclaimed from short
//! labels. Transient pickers keep the smaller centered floating card.
//! House style: the card primitive, `▌` selection marker, accent+bold
//! matched chars, keycap hints in the bottom border, an honest count
//! top-right, a thumb-only results scrollbar and one rule between the
//! list and the preview.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::editor::Editor;

mod hints;
mod preview;
mod rows;
mod window;
#[cfg(test)]
mod workspace_tests;

use rows::render_results;

use super::card::Card;
use super::style::Tokens;

/// Dim the backdrop: the editor stays readable under the card (0003 §2.1
/// live backdrop), with fg colors pulled toward the base.
pub fn dim_backdrop(t: &Tokens, frame: &mut Frame, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut frame.buffer_mut()[(x, y)];
            cell.set_fg(t.dim(cell.fg));
            if cell.bg != t.base {
                cell.set_bg(t.dim(cell.bg));
            }
        }
    }
}

/// `2,311` — thousands grouped so a total reads at a glance.
pub(super) fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The honest count (U6): `38 of 2,311` while filtered, the total when
/// not, `…` while a source still streams; a failed source headlines
/// instead (0014: rg's bad-flag stderr reads like an empty project
/// otherwise), and a warning takes the slot only when there is no error.
fn count_note(picker: &strop_picker::Picker, streaming: bool) -> String {
    if let Some(notice) = picker.error.as_ref().or(picker.warning.as_ref()) {
        return notice.clone();
    }
    let total = picker.items.len();
    let shown = picker.rows.len();
    let tail = if streaming { "…" } else { "" };
    if picker.kind == strop_picker::Kind::Search {
        let included = total.saturating_sub(picker.excluded_count());
        let files = rows::file_count(picker);
        return format!(
            "{} hit{} · {} file{}{tail}",
            grouped(included),
            if included == 1 { "" } else { "s" },
            grouped(files),
            if files == 1 { "" } else { "s" },
        );
    }
    if shown != total {
        format!("{} of {}{tail}", grouped(shown), grouped(total))
    } else {
        format!("{}{tail}", grouped(total))
    }
}

/// The engine's query summary reads `namespace · mode · hidden X (origin)
/// · ignored Y (origin)`. It is chrome only when something in it is not
/// the default: the namespace is dropped when it is the working
/// directory, and the whole row when the rest matches the config.
fn non_default_scope(editor: &Editor, summary: &str) -> Option<String> {
    let config = editor.config();
    let on_off = |on: bool| if on { "on" } else { "off" };
    let default_tail = format!(
        "hidden {} (session) · ignored {} (session)",
        on_off(config.search_show_hidden),
        on_off(!config.search_respect_ignore)
    );
    let cwd = editor.cwd().to_string_lossy();
    let (namespace, rest) = summary.split_once(" · ")?;
    let local = namespace == cwd || namespace == "local";
    if local && rest.ends_with(&default_tail) && !rest.contains("(query)") {
        return None;
    }
    Some(if local {
        rest.to_owned()
    } else {
        summary.to_owned()
    })
}

pub fn render_picker(editor: &Editor, t: &Tokens, frame: &mut Frame) {
    let Some(glue) = editor.picker() else {
        return;
    };
    let area = frame.area();
    dim_backdrop(t, frame, area);
    let p = &glue.picker;
    let kind = p.kind;
    let streaming = p.streaming || glue.rank_pending.is_some();
    let search_mode = kind == strop_picker::Kind::Search;
    let replace_mode = search_mode && p.replacement_visible;

    // Search, files and symbols own one stable near-full-frame workspace;
    // the smaller floating card remains for transient pickers.
    let card = super::from_cells(strop_engine::editor::prepare::picker_card(
        super::to_cells(area),
        kind,
    ));
    let note = count_note(p, streaming);
    let note = strop_core::layout::printable_text(&note);
    let hints = hints::for_picker(editor, glue, card.width);
    let title = kind.title().trim();
    Card::new(title)
        .note(Some(&note))
        .hints(&hints)
        .render(frame, card, t);
    // The engine's inner geometry — 1-cell border + 1-cell padding (0001
    // §4: floating panes breathe) — is the card primitive's too; paint
    // reads the engine's so it agrees with preparation to the cell.
    let inner = super::from_cells(strop_engine::editor::prepare::picker_inner(
        super::to_cells(card),
    ));

    // input row(s) + content split; replace mode adds a second field
    let input_h = strop_engine::editor::prepare::picker_input_height(kind, p.replacement_visible);
    // A tiny terminal spends its last row on the active editor, not results.
    // Do not rewrite the retained viewport while it cannot be displayed.
    if inner.height < input_h + 1 {
        let with = replace_mode && p.field == strop_picker::Field::Replace;
        let projected = super::field::project(
            t,
            if with { "With" } else { "Find" },
            if with {
                p.replace_input.text()
            } else {
                p.input.text()
            },
            if with {
                p.replace_input.cursor()
            } else {
                p.input.cursor()
            },
            true,
            inner.width,
            &[],
        );
        let cursor = projected.cursor;
        frame.render_widget(Paragraph::new(projected.line), inner);
        if let Some(column) = cursor.filter(|column| *column < inner.width && inner.height > 0) {
            crate::render::frame_capture::place_cursor(frame, (inner.x + column, inner.y));
        }
        return;
    }
    let (field_area, content_area) = {
        let (fields, content) =
            strop_engine::editor::prepare::picker_field_split(super::to_cells(inner), input_h);
        (super::from_cells(fields), super::from_cells(content))
    };
    let glyph = if p.input_normal() { "▮" } else { "❯" };
    // Every surface that speaks the query language (0051 §3) renders
    // its qualifier highlighting; the summary line describes source
    // visibility policy, which only the streaming surfaces have.
    let query_roles = matches!(
        kind,
        strop_picker::Kind::Files
            | strop_picker::Kind::Search
            | strop_picker::Kind::Symbols
            | strop_picker::Kind::WorkspaceSymbols
            | strop_picker::Kind::Jumps
    );
    let summary_roles = matches!(kind, strop_picker::Kind::Files | strop_picker::Kind::Search);
    let search_active = !replace_mode || p.field == strop_picker::Field::Search;
    let search_label = if search_mode {
        format!("{glyph} Find")
    } else {
        glyph.to_string()
    };
    let search = super::field::project(
        t,
        &search_label,
        p.input.text(),
        p.input.cursor(),
        search_active,
        field_area.width,
        if query_roles {
            glue.query_highlights.as_slice()
        } else {
            &[]
        },
    );
    let mut field_caret = search.cursor.map(|column| (column, 0u16));
    let mut fields = vec![search.line];
    if replace_mode {
        let replacement = super::field::project(
            t,
            &format!("{glyph} With"),
            p.replace_input.text(),
            p.replace_input.cursor(),
            !search_active,
            field_area.width,
            &[],
        );
        if let Some(column) = replacement.cursor {
            field_caret = Some((column, 1));
        }
        fields.push(replacement.line);
    }
    frame.render_widget(Paragraph::new(fields), field_area);
    // The scope row (rule 4): only when a scope differs from the default
    // — an explicit qualifier, a session toggle away from the config, or
    // a namespace other than the working directory.
    let scope_y = field_area.y + input_h - 1;
    if scope_y < content_area.y && summary_roles {
        if let Some(scope) = non_default_scope(editor, &glue.query_summary) {
            frame.render_widget(
                Paragraph::new(scope).style(Style::default().fg(t.muted)),
                Rect {
                    y: scope_y,
                    height: 1,
                    ..field_area
                },
            );
        }
    }

    // 0050 §8: preparation commits one content-aware split for the list,
    // reveal clamp, preview admission and this paint. Narrow and tiny
    // fallbacks are already expressed by that snapshot.
    let prepared = editor
        .prepared_view()
        .picker
        .expect("open picker has prepared geometry");
    let (results, preview_area) = match (prepared.results, prepared.preview) {
        (Some(results), preview) => (super::from_cells(results), preview.map(super::from_cells)),
        (None, _) => (content_area, None),
    };

    let selected_theme = if kind == strop_picker::Kind::Theme {
        p.rows.get(p.selected).and_then(|row| {
            p.items.get(row.item).and_then(|item| match &item.payload {
                strop_picker::Payload::ThemeChoice(id) => Some(*id),
                _ => None,
            })
        })
    } else {
        None
    };
    // The selected identity gets a bounded three-line detail: canonical name,
    // then the complete description wrapped at the normal compact width. Tiny
    // cards keep the input and scrolling list instead of sacrificing them.
    let (results, theme_detail) = match selected_theme {
        Some(id) if results.height >= 6 && results.width >= 32 => {
            let results = Rect {
                height: results.height - 3,
                ..results
            };
            let detail = Rect {
                y: results.y + results.height,
                height: 3,
                width: results.width.saturating_sub(1),
                ..results
            };
            (results, Some((detail, id)))
        }
        _ => (results, None),
    };

    if kind == strop_picker::Kind::RemoteAddress {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from("host or user@host[:port]"),
                Line::from("ssh://host/absolute/path  —  file or directory"),
                Line::from("New hosts open /; no mount or recursive workspace scan."),
            ])
            .style(Style::default().fg(t.muted)),
            results,
        );
    } else {
        // the scrollbar column is reserved BEFORE text budgets (0050 §8)
        let text_area = Rect {
            width: results.width.saturating_sub(1),
            ..results
        };
        render_results(frame, t, text_area, p, editor.cwd(), &|path| {
            editor.tab_width_for_location(path)
        });
    }
    // thumb-only results scrollbar (rule 4): nothing when the list fits
    let row_count = p.rows.len();
    if !results.is_empty() && row_count > results.height as usize {
        let track_x = results.x + results.width - 1;
        let track_h = results.height as usize;
        let frac = p.selected as f32 / row_count.max(1) as f32;
        let thumb = ((track_h - 1) as f32 * frac) as usize;
        let cell = &mut frame.buffer_mut()[(track_x, results.y + thumb as u16)];
        cell.set_symbol("▮");
        cell.set_fg(t.muted);
    }
    if let Some((area, id)) = theme_detail {
        let applied = id == editor.theme();
        let mut heading = vec![Span::styled(
            id.name(),
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )];
        if applied {
            heading.push(Span::styled(" ● applied", Style::default().fg(t.accent)));
        }
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(heading),
                Line::from(Span::styled(
                    id.description(),
                    Style::default().fg(t.secondary),
                )),
            ])
            .style(Style::default().bg(t.surface))
            .wrap(Wrap { trim: true }),
            area,
        );
    }
    if let Some(preview_area) = preview_area {
        preview::render_preview(editor, t, frame, preview_area);
    }
    // Manual suggestions own focus without resizing the underlying card.
    if let Some(list) = &glue.suggestions {
        let available = frame.area().bottom().saturating_sub(field_area.y + input_h);
        let show = (list.items.len().min(6) as u16).min(available.saturating_sub(2));
        let area = Rect {
            x: field_area.x,
            y: field_area.y + input_h,
            width: field_area.width.min(40),
            height: (show + 2).min(available),
        };
        let mut lines: Vec<Line> = Vec::with_capacity(show as usize);
        let first = list
            .selected
            .saturating_sub(show.saturating_sub(1) as usize);
        for (i, suggestion) in list
            .items
            .iter()
            .enumerate()
            .skip(first)
            .take(show as usize)
        {
            let active = i == list.selected;
            let style = if active {
                Style::default().fg(t.text)
            } else {
                Style::default().fg(t.secondary)
            };
            let line = Line::from(vec![
                super::card::row_marker(active, t),
                Span::styled(suggestion.insert.clone(), style),
                Span::styled(
                    format!("  {}", suggestion.detail),
                    Style::default().fg(t.muted),
                ),
            ]);
            lines.push(super::card::band(
                line,
                area.width.saturating_sub(2),
                active,
                t,
            ));
        }
        let inner = Card::new("suggest").render(frame, area, t);
        frame.render_widget(
            Paragraph::new(lines),
            Rect {
                x: inner.x.saturating_sub(1),
                width: inner.width.saturating_add(2),
                ..inner
            },
        );
    }
    if let Some((column, row)) = field_caret {
        if column < field_area.width && row < field_area.height {
            crate::render::frame_capture::place_cursor(
                frame,
                (field_area.x + column, field_area.y + row),
            );
        }
    }
}
