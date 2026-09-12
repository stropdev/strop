//! Picker result rows (0050): per-kind composers over structured
//! payload fields — symbol chips, file basename-first, grep's real
//! match window. The selected LOGICAL row paints one full band
//! (marker, fields, trailing blanks), and query/match evidence is
//! amber+bold on top of it. Nothing here re-searches text or reparses
//! display strings.
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::diff::{ADD_FG, DEL_FG};
use super::super::{text, ACCENT, BASE, MUTED, SECONDARY, SELECT_BG, TEXT};
use super::window::{match_window, replacement_window};
use unicode_segmentation::UnicodeSegmentation;

/// The main results list: one or two display lines per logical row
/// (grep), the selection band covering all of a row's lines.
pub(super) fn render_results(
    frame: &mut Frame,
    area: Rect,
    picker: &strop_picker::Picker,
    selected: usize,
    tab_for_path: &impl Fn(&std::path::Path) -> usize,
) {
    let two_line = picker.kind == strop_picker::Kind::Grep;
    let per_row = if two_line { 2 } else { 1 };
    let visible_rows = (area.height as usize).div_ceil(per_row);
    let start_row = if selected >= visible_rows {
        selected + 1 - visible_rows
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    if picker.rows.is_empty()
        && picker.input.text.is_empty()
        && matches!(
            picker.kind,
            strop_picker::Kind::Files | strop_picker::Kind::Grep
        )
    {
        // 0051 R02: the empty field teaches the vocabulary — never an
        // obsolete flag hint
        lines.push(Line::from(Span::styled(
            " language:rust path:src/ glob:\"**/*.rs\" hidden:include case:smart",
            Style::default().fg(MUTED),
        )));
        lines.push(Line::from(Span::styled(
            if picker.kind == strop_picker::Kind::Files {
                " bare words are fuzzy paths · ctrl-space suggests"
            } else {
                " bare words are literal content · ctrl-space suggests"
            },
            Style::default().fg(MUTED),
        )));
    }
    if picker.rows.is_empty()
        && picker.input.text.is_empty()
        && !matches!(
            picker.kind,
            strop_picker::Kind::Files | strop_picker::Kind::Grep
        )
    {
        let message = if picker.streaming {
            " loading…"
        } else if !picker.items.is_empty() {
            " preparing results…"
        } else {
            " no entries available"
        };
        lines.push(Line::from(Span::styled(
            message,
            Style::default().fg(MUTED),
        )));
    }
    if picker.rows.is_empty() && !picker.input.text.is_empty() {
        // 0050 §8: an empty filtered set is explained, never a blank
        // card with a nonzero count
        let noun = match picker.kind {
            strop_picker::Kind::Grep | strop_picker::Kind::Replace => "matches".to_string(),
            _ => format!("{} match", picker.kind.title().trim()),
        };
        lines.push(Line::from(Span::styled(
            format!(" No {noun} “{}”", picker.input.text),
            Style::default().fg(SECONDARY),
        )));
        lines.push(Line::from(Span::styled(
            " edit query · esc normal/close",
            Style::default().fg(MUTED),
        )));
    }
    for (vi, row) in picker
        .rows
        .iter()
        .enumerate()
        .skip(start_row)
        .take(visible_rows)
    {
        let active = vi == selected;
        let Some(item) = picker.items.get(row.item) else {
            continue;
        };
        let match_cols = picker.match_columns(row);
        let tab = match &item.payload {
            strop_picker::Payload::Grep { path, .. } => tab_for_path(path),
            _ => 4,
        };
        for line in compose_row(item, picker.kind, match_cols, area.width, active, tab) {
            lines.push(line);
        }
    }
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(BASE)), area);
}

/// One logical row as display lines, by kind.
fn compose_row(
    item: &strop_picker::Item,
    kind: strop_picker::Kind,
    match_cols: &[u32],
    width: u16,
    active: bool,
    tab: usize,
) -> Vec<Line<'static>> {
    match kind {
        strop_picker::Kind::Symbols => vec![symbol_row(item, match_cols, width, active)],
        strop_picker::Kind::Files => vec![file_row(&item.text, match_cols, width, active)],
        strop_picker::Kind::Grep => grep_rows(item, width, active, tab),
        _ => vec![generic_row(item, match_cols, width, active)],
    }
}

/// Selection band and the marker share one style decision.
fn marker(active: bool) -> Span<'static> {
    let style = if active {
        Style::default().fg(ACCENT).bg(SELECT_BG)
    } else {
        Style::default().fg(MUTED)
    };
    Span::styled(if active { "▌" } else { " " }, style)
}

/// Pad a row to the full width; the band covers the blanks (0050 §4).
fn finish(mut spans: Vec<Span<'static>>, width: u16, active: bool) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    for span in &mut spans {
        if span.content.chars().any(char::is_control) {
            span.content = strop_core::layout::printable_text(span.content.as_ref())
                .into_owned()
                .into();
        }
        if active {
            span.style = span.style.bg(SELECT_BG);
        }
    }
    let clipped = spans.iter().map(Span::width).sum::<usize>() > width as usize;
    let budget = width as usize - usize::from(clipped);
    let mut used = 0;
    let mut output = Vec::new();
    for mut span in spans {
        if used >= budget {
            break;
        }
        if span.width() > budget - used {
            let mut prefix = String::new();
            for grapheme in span.content.graphemes(true) {
                let cells = text::width(grapheme);
                if used + cells > budget {
                    break;
                }
                prefix.push_str(grapheme);
                used += cells;
            }
            span.content = prefix.into();
            output.push(span);
            break;
        }
        used += span.width();
        output.push(span);
    }
    if clipped {
        output.push(Span::styled(
            "…",
            if active {
                Style::default().fg(MUTED).bg(SELECT_BG)
            } else {
                Style::default().fg(MUTED)
            },
        ));
        used += 1;
    }
    if active && used < width as usize {
        output.push(Span::styled(
            " ".repeat(width as usize - used),
            Style::default().bg(SELECT_BG),
        ));
    }
    Line::from(output)
}

/// Emphasize the query match inside a field's text: accent+bold chars,
/// never background blocks (0001 §4). `cols` are char indices.
fn emphasis(value: &str, cols: &[u32], base: Style, active: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut character = 0;
    for (glyph, grapheme) in strop_core::layout::RopeGraphemes::new(value.into(), 4) {
        let end = character + grapheme.chars().count();
        let matched = cols
            .iter()
            .any(|&column| character <= column as usize && (column as usize) < end);
        character = end;
        if glyph.width == 0 {
            continue;
        }
        let mut style = if matched {
            base.fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            base
        };
        if active {
            style = style.bg(SELECT_BG);
        }
        let shown = strop_core::layout::printable_grapheme(&grapheme).to_string();
        spans.push(Span::styled(shown, style));
    }
    spans
}

fn plain(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

// ---- symbols --------------------------------------------------------------

/// The kind chip slot: 9 cells including padding, so names share one
/// left edge across `fn` and `variant` rows (0050 §5). Quiet semantic
/// foreground — never a saturated badge background.
const CHIP_W: usize = 9;

fn chip(badge: Option<&str>, active: bool) -> Span<'static> {
    let label = badge.unwrap_or("");
    let text = format!(" {:<7} ", label);
    let mut style = Style::default().fg(kind_color(label));
    if active {
        style = style.bg(SELECT_BG);
    }
    Span::styled(text, style)
}

/// Four quiet kind families (0050 §4): callables, types, data, modules.
fn kind_color(label: &str) -> Color {
    match label {
        "fn" | "meth" | "new" => Color::Rgb(0x89, 0xb4, 0xfa),
        "struct" | "class" | "iface" | "enum" | "variant" | "T" => Color::Rgb(0xcb, 0xa6, 0xf7),
        "const" | "var" | "field" | "prop" => Color::Rgb(0xa6, 0xe3, 0xa1),
        "mod" | "ns" | "pkg" => Color::Rgb(0x94, 0xe2, 0xd5),
        _ => SECONDARY,
    }
}

/// ` chip  name  container                  :line` — location holds the
/// right edge; the container drops before the name truncates.
fn symbol_row(
    item: &strop_picker::Item,
    match_cols: &[u32],
    width: u16,
    active: bool,
) -> Line<'static> {
    let (name, container, location) = symbol_fields(item);
    let budget = (width as usize).saturating_sub(1 + CHIP_W);
    let loc_w = text::width(&location);
    let (name_w, _cont_w) = if !location.is_empty() {
        (budget.saturating_sub(loc_w + 1), loc_w)
    } else {
        (budget, 0)
    };
    let name_budget = name_w.saturating_sub(if container.is_empty() { 0 } else { 1 });
    let mut spans = vec![marker(active), chip(item.badge.as_deref(), active)];
    let name_chars = text::width(&name);
    if name_chars <= name_budget {
        spans.extend(emphasis(
            &name,
            match_cols,
            Style::default().fg(TEXT),
            active,
        ));
        let used = 1 + CHIP_W + name_chars;
        // container sits after the name; pad between the fields
        if !container.is_empty() && used + 1 + text::width(&container) + loc_w < width as usize {
            spans.push(plain(" ", Style::default()));
            spans.extend(emphasis(
                &container,
                &[],
                Style::default().fg(SECONDARY),
                active,
            ));
        } else if used < width as usize {
            spans.push(plain(" ", Style::default()));
        }
    } else {
        let clipped = text::clip_end(&name, name_budget);
        spans.extend(emphasis(
            &clipped,
            match_cols,
            Style::default().fg(TEXT),
            active,
        ));
    }
    if !location.is_empty() {
        let used: usize = spans.iter().map(Span::width).sum();
        let pad = (width as usize).saturating_sub(used + loc_w + 1);
        spans.push(plain(" ".repeat(pad.max(1)), Style::default()));
        spans.push(plain(&location, Style::default().fg(SECONDARY)));
    }
    finish(spans, width, active)
}

/// (name, container, location) from the symbol row's own construction
/// fields — the text is `name  container · :line` from the Symbols arm.
fn symbol_fields(item: &strop_picker::Item) -> (String, String, String) {
    let location = match &item.payload {
        strop_picker::Payload::Grep { line, .. } => format!(":{line}"),
        strop_picker::Payload::Remote { line, .. } => format!(":{line}"),
        _ => String::new(),
    };
    // the Symbols arm writes "name  container · :line" or "name  · :line"
    let text = item.text.trim_end();
    let (head, _) = text.rsplit_once('·').unwrap_or((text, ""));
    let mut parts = head.splitn(2, "  ");
    let name = parts.next().unwrap_or("").trim().to_string();
    let container = parts
        .next()
        .map(|c| c.trim().to_string())
        .unwrap_or_default();
    (name, container, location)
}

// ---- files ----------------------------------------------------------------

/// ` client.cpp   tests/integration/transport/http` — basename bright
/// (query matches on it emphasized), the parent path secondary at the
/// right edge, middle-elided but never dropping the disambiguator.
fn file_row(value: &str, match_cols: &[u32], width: u16, active: bool) -> Line<'static> {
    let (dir, name) = match value.rfind('/') {
        Some(i) => (&value[..i], &value[i + 1..]),
        None => ("", value),
    };
    let name_off = dir.chars().count() + usize::from(!dir.is_empty());
    // matches inside the basename shift into the name's own coordinates;
    // matches in the directory emphasize the context field (0050 §6)
    let mut name_cols: Vec<u32> = match_cols
        .iter()
        .filter_map(|c| c.checked_sub(name_off as u32))
        .collect();
    let dir_cols: Vec<u32> = match_cols
        .iter()
        .copied()
        .filter(|c| (*c as usize) < dir.chars().count())
        .collect();
    let budget = (width as usize).saturating_sub(1);
    let dir_w = text::width(dir);
    let name_w = text::width(name);
    let mut spans = vec![marker(active)];
    if budget >= name_w + dir_w + 2 {
        spans.extend(emphasis(
            name,
            &name_cols,
            Style::default().fg(TEXT),
            active,
        ));
        let pad = budget - name_w - dir_w;
        spans.push(plain(" ".repeat(pad), Style::default()));
        spans.extend(emphasis(
            dir,
            &dir_cols,
            Style::default().fg(SECONDARY),
            active,
        ));
    } else {
        let room = if dir.is_empty() || budget < 8 {
            0
        } else {
            (budget / 3).max(4).min(budget / 2)
        };
        let name_budget = budget.saturating_sub(room + usize::from(room > 0));
        let shown = text::clip_end(name, name_budget);
        if name_w > name_budget {
            let prefix = shown.chars().count().saturating_sub(1);
            name_cols.retain(|column| (*column as usize) < prefix);
        }
        spans.extend(emphasis(
            &shown,
            &name_cols,
            Style::default().fg(TEXT),
            active,
        ));
        if room > 0 {
            let context = elide_middle(dir, room);
            let pad = budget.saturating_sub(text::width(&shown) + text::width(&context));
            spans.push(plain(" ".repeat(pad), Style::default()));
            spans.extend(emphasis(
                &context,
                &[],
                Style::default().fg(SECONDARY),
                active,
            ));
        }
    }
    finish(spans, width, active)
}

/// Middle-elide a path, keeping head and tail components (the tail is
/// where duplicate basenames disambiguate).
fn elide_middle(path: &str, room: usize) -> String {
    if room == 0 {
        return String::new();
    }
    if text::width(path) <= room {
        return path.to_string();
    }
    let tail_budget = (room - 1) * 2 / 3;
    let head_budget = room - 1 - tail_budget;
    let head = text::clip_end(path, head_budget + 1);
    let tail = text::clip_start(path, tail_budget + 1);
    format!(
        "{}…{}",
        head.strip_suffix('…').unwrap_or(&head),
        tail.strip_prefix('…').unwrap_or(&tail)
    )
}

// ---- grep -----------------------------------------------------------------

/// Two display lines per hit (0050 §7): filename + directory, then the
/// code window around the REAL submatch with line:char numbers. A wide
/// list may collapse to one row when everything fits.
fn grep_rows(
    item: &strop_picker::Item,
    width: u16,
    active: bool,
    tab: usize,
) -> Vec<Line<'static>> {
    let strop_picker::Payload::Grep {
        path,
        line,
        col,
        match_len,
        line_text,
    } = &item.payload
    else {
        return vec![generic_row(item, &[], width, active)];
    };
    let budget = (width as usize).saturating_sub(1);
    let line1 = file_row(&strop_picker::display_path(path), &[], width, active);
    // line 2: numbers secondary, then the code window around the match
    let (match_start, match_end) = strop_picker::replace_span(line_text, *col, *match_len);
    let numbers = format!(" {line}:{}  ", match_start + 1);
    let code_budget = budget.saturating_sub(text::width(&numbers));
    let (window, win_match) = match_window(line_text, match_start, match_end, code_budget, tab);
    let mut bottom = vec![plain(" ", Style::default())];
    bottom.push(plain(&numbers, Style::default().fg(SECONDARY)));
    bottom.extend(emphasis(
        &window,
        &match_cols_in_window(win_match, &window),
        Style::default().fg(TEXT),
        active,
    ));
    let line2 = finish(bottom, width, active);
    vec![line1, line2]
}

/// Match columns for the window's own char coordinates.
fn match_cols_in_window((start, end): (usize, usize), _window: &str) -> Vec<u32> {
    (start as u32..end as u32).collect()
}

// ---- generic rows (buffers, jumps, remote, diagnostics, locations) --------

fn generic_row(
    item: &strop_picker::Item,
    match_cols: &[u32],
    width: u16,
    active: bool,
) -> Line<'static> {
    let mut spans = vec![marker(active)];
    let text = text::clip_end(&item.text, width.saturating_sub(1) as usize);
    let dim_prefix = locator_prefix_chars(item);
    let base = Style::default().fg(if active {
        TEXT
    } else {
        Color::Rgb(0xb8, 0xb4, 0xa9)
    });
    let mut character = 0;
    for (glyph, grapheme) in strop_core::layout::RopeGraphemes::new(text.as_ref().into(), 4) {
        let end = character + grapheme.chars().count();
        let mut style = if character < dim_prefix {
            base.fg(MUTED)
        } else {
            base
        };
        if match_cols
            .iter()
            .any(|&column| character <= column as usize && (column as usize) < end)
        {
            style = style.fg(ACCENT).add_modifier(Modifier::BOLD);
        }
        character = end;
        if glyph.width == 0 {
            continue;
        }
        if active {
            style = style.bg(SELECT_BG);
        }
        spans.push(Span::styled(
            strop_core::layout::printable_grapheme(&grapheme).to_string(),
            style,
        ));
    }
    finish(spans, width, active)
}

/// Chars of the row text that form the locator (dimmed): computed from
/// the payload's structured fields, never by parsing the text.
fn locator_prefix_chars(item: &strop_picker::Item) -> usize {
    use strop_picker::Payload;
    match &item.payload {
        // file rows: dim the directory portion, the name stays bright
        Payload::File(path) => {
            let text = path.display().to_string();
            text.rfind('/').map_or(0, |i| text[..i + 1].chars().count())
        }
        // grep hits render "{path}:{line} · …": dim through the number
        Payload::Grep { path, line, .. } => {
            path.display().to_string().chars().count() + 1 + digits(*line)
        }
        // jumplist rows carry a 2-char marker before "{name}:{line}"
        Payload::Jump { .. } => item
            .text
            .find(':')
            .map(|i| {
                item.text[..i + 1].chars().count()
                    + item.text[i + 1..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .count()
            })
            .unwrap_or(0),
        // remote hits render "{endpoint}{path}:{line}"
        Payload::Remote {
            endpoint,
            path,
            line,
            ..
        } => {
            endpoint.to_string().chars().count()
                + path.display().to_string().chars().count()
                + 1
                + digits(*line)
        }
        _ => 0,
    }
}

fn digits(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

// ---- replace ---------------------------------------------------------------

/// Three display lines per hit (0051 §6 R04): the grep source-hit
/// vocabulary — inclusion state, filename bright, directory secondary,
/// source numbers secondary, the code window around the REAL submatch
/// with the match amber/bold — plus the replacement delta in the
/// common diff roles. Exclusion is a neutral inclusion state, never
/// the failure red.
fn replace_rows(
    item: &strop_picker::Item,
    replacement: &str,
    excluded: bool,
    width: u16,
    active: bool,
    tab: usize,
) -> Vec<Line<'static>> {
    let strop_picker::Payload::Grep {
        path,
        line,
        col,
        match_len,
        line_text,
    } = &item.payload
    else {
        return vec![generic_row(item, &[], width, active)];
    };
    let secondary = if excluded { MUTED } else { SECONDARY };
    let budget = (width as usize).saturating_sub(1);
    let notice = if excluded && width >= 24 {
        " · excluded"
    } else {
        ""
    };
    let path_width = width.saturating_sub((4 + text::width(notice)) as u16);
    let heading = file_row(&strop_picker::display_path(path), &[], path_width, active);
    let mut top = vec![
        marker(active),
        plain(
            if excluded { "[ ] " } else { "[x] " },
            Style::default().fg(MUTED),
        ),
    ];
    for mut span in heading.spans.into_iter().skip(1) {
        if excluded {
            span.style = span.style.fg(MUTED);
        }
        top.push(span);
    }
    if !notice.is_empty() {
        top.push(plain(notice, Style::default().fg(MUTED)));
    }
    let line1 = finish(top, width, active);
    // lines 2/3: `-` old / `+` new over the same match-window machinery
    // as grep_rows; the match/replacement spans stay amber/bold on top
    // of the diff roles. Preview and apply share `replace_span`.
    let (s, e) = strop_picker::replace_span(line_text, *col, *match_len);
    let numbers = format!(" {line}:{}  ", s + 1);
    let numbers_w = text::width(&numbers);
    let code_budget = budget.saturating_sub(numbers_w + 2);
    let evidence = |window: &str, span: (usize, usize)| {
        if excluded {
            Vec::new()
        } else {
            match_cols_in_window(span, window)
        }
    };
    let (old_window, old_match) = match_window(line_text, s, e, code_budget, tab);
    let del = if excluded { secondary } else { DEL_FG };
    let mut old = vec![plain(" ", Style::default())];
    old.push(plain(&numbers, Style::default().fg(secondary)));
    old.push(plain("- ", Style::default().fg(del)));
    old.extend(emphasis(
        &old_window,
        &evidence(&old_window, old_match),
        Style::default().fg(del),
        active,
    ));
    let line2 = finish(old, width, active);
    let (new_window, new_match) =
        replacement_window(line_text, s, e, replacement, code_budget, tab);
    let add = if excluded { secondary } else { ADD_FG };
    let mut new = vec![plain(" ", Style::default())];
    new.push(plain(" ".repeat(numbers_w), Style::default()));
    new.push(plain("+ ", Style::default().fg(add)));
    new.extend(emphasis(
        &new_window,
        &evidence(&new_window, new_match),
        Style::default().fg(add),
        active,
    ));
    let line3 = finish(new, width, active);
    vec![line1, line2, line3]
}

/// The replace results list (0051 §6 R04): three display lines per
/// logical row — source-hit identity, `-` old, `+` new — with the
/// selection band covering the whole logical block, and the same named
/// empty states as the grep list.
pub(super) fn render_replace_results(
    frame: &mut Frame,
    area: Rect,
    p: &strop_picker::Picker,
    tab_for_path: &impl Fn(&std::path::Path) -> usize,
) {
    const PER_ROW: usize = 3;
    let visible_rows = (area.height as usize).div_ceil(PER_ROW);
    let selected = p.selected;
    let start = if selected >= visible_rows {
        selected + 1 - visible_rows
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    if p.rows.is_empty() && p.input.text.is_empty() {
        // 0051 R02: the empty field teaches the vocabulary — never an
        // obsolete flag hint
        lines.push(Line::from(Span::styled(
            " language:rust path:src/ glob:\"**/*.rs\" hidden:include case:smart",
            Style::default().fg(MUTED),
        )));
        lines.push(Line::from(Span::styled(
            " bare words are literal content · ctrl-space suggests",
            Style::default().fg(MUTED),
        )));
    }
    if p.rows.is_empty() && !p.input.text.is_empty() {
        // 0050 §8: an empty set is explained, never a blank card
        lines.push(Line::from(Span::styled(
            format!(" No matches “{}”", p.input.text),
            Style::default().fg(SECONDARY),
        )));
        lines.push(Line::from(Span::styled(
            " edit Find · esc normal/close",
            Style::default().fg(MUTED),
        )));
    }
    for (vi, row) in p.rows.iter().enumerate().skip(start).take(visible_rows) {
        let active = vi == selected;
        let Some(item) = p.items.get(row.item) else {
            continue;
        };
        lines.extend(replace_rows(
            item,
            &p.replace_input.text,
            p.is_excluded(row.item),
            area.width,
            active,
            match &item.payload {
                strop_picker::Payload::Grep { path, .. } => tab_for_path(path),
                _ => 4,
            },
        ));
    }
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(BASE)), area);
}

#[cfg(test)]
mod tests;
