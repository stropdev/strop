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

use super::super::{dim_color, text, ACCENT, BASE, MUTED, SECONDARY, SELECT_BG, TEXT};

/// The main results list: one or two display lines per logical row
/// (grep), the selection band covering all of a row's lines.
pub(super) fn render_results(
    frame: &mut Frame,
    area: Rect,
    picker: &strop_picker::Picker,
    selected: usize,
) {
    let two_line = picker.kind == strop_picker::Kind::Grep;
    let per_row = if two_line { 2 } else { 1 };
    let visible_rows = (area.height as usize) / per_row;
    let start_row = if selected >= visible_rows {
        selected + 1 - visible_rows
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
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
            " esc clears the query",
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
        for line in compose_row(item, picker.kind, match_cols, area.width, active) {
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
) -> Vec<Line<'static>> {
    match kind {
        strop_picker::Kind::Symbols => vec![symbol_row(item, match_cols, width, active)],
        strop_picker::Kind::Files => vec![file_row(item, match_cols, width, active)],
        strop_picker::Kind::Grep => grep_rows(item, width, active),
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
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if active {
        spans = spans
            .into_iter()
            .map(|span| span.patch_style(Style::default().bg(SELECT_BG)))
            .collect();
        if used < width as usize {
            spans.push(Span::styled(
                " ".repeat(width as usize - used),
                Style::default().bg(SELECT_BG),
            ));
        }
    }
    Line::from(spans)
}

/// Emphasize the query match inside a field's text: accent+bold chars,
/// never background blocks (0001 §4). `cols` are char indices.
fn emphasis(text: &str, cols: &[u32], base: Style, active: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(text.chars().count());
    for (ci, ch) in text.chars().enumerate() {
        let mut st = base;
        if cols.contains(&(ci as u32)) {
            st = st.fg(ACCENT).add_modifier(Modifier::BOLD);
        }
        if active {
            st = st.bg(SELECT_BG);
        }
        spans.push(Span::styled(ch.to_string(), st));
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
    let budget = width as usize - 1 - CHIP_W; // marker + chip
    let loc_w = location.chars().count();
    let (name_w, _cont_w) = if !location.is_empty() {
        (budget.saturating_sub(loc_w + 1), loc_w)
    } else {
        (budget, 0)
    };
    let name_budget = name_w.saturating_sub(if container.is_empty() { 0 } else { 1 });
    let mut spans = vec![marker(active), chip(item.badge.as_deref(), active)];
    let name_chars = name.chars().count();
    if name_chars <= name_budget {
        spans.extend(emphasis(
            &name,
            match_cols,
            Style::default().fg(TEXT),
            active,
        ));
        let used = 1 + CHIP_W + name_chars;
        // container sits after the name; pad between the fields
        if !container.is_empty() && used + 1 + container.chars().count() + loc_w < width as usize {
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
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
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
fn file_row(
    item: &strop_picker::Item,
    match_cols: &[u32],
    width: u16,
    active: bool,
) -> Line<'static> {
    let (dir, name) = match item.text.rfind('/') {
        Some(i) => (&item.text[..i], &item.text[i + 1..]),
        None => ("", item.text.as_str()),
    };
    let name_off = dir.chars().count() + usize::from(!dir.is_empty());
    // matches inside the basename shift into the name's own coordinates;
    // matches in the directory emphasize the context field (0050 §6)
    let name_cols: Vec<u32> = match_cols
        .iter()
        .filter_map(|c| c.checked_sub(name_off as u32))
        .collect();
    let dir_cols: Vec<u32> = match_cols
        .iter()
        .copied()
        .filter(|c| (*c as usize) < dir.chars().count())
        .collect();
    let budget = width as usize - 1;
    let dir_w = dir.chars().count();
    let name_w = name.chars().count();
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
        // tight: name first, then the context tail (never elide the
        // only distinguishing components — elide the middle)
        let room = budget.saturating_sub(name_w + 2);
        spans.extend(emphasis(
            name,
            &name_cols,
            Style::default().fg(TEXT),
            active,
        ));
        if room > 4 {
            let elided = elide_middle(dir, room);
            let pad = budget - name_w - elided.chars().count();
            spans.push(plain(" ".repeat(pad.max(1)), Style::default()));
            spans.extend(emphasis(
                &elided,
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
    let total = path.chars().count();
    if total <= room {
        return path.to_string();
    }
    let keep_tail = room.saturating_sub(2) / 2;
    let keep_head = room - keep_tail - 1;
    let head: String = path.chars().take(keep_head).collect();
    let tail: String = path.chars().skip(total.saturating_sub(keep_tail)).collect();
    format!("{head}…{tail}")
}

// ---- grep -----------------------------------------------------------------

/// Two display lines per hit (0050 §7): filename + directory, then the
/// code window around the REAL submatch with line:char numbers. A wide
/// list may collapse to one row when everything fits.
fn grep_rows(item: &strop_picker::Item, width: u16, active: bool) -> Vec<Line<'static>> {
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
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let budget = width as usize - 1;
    // line 1: filename bright, directory secondary at the right edge
    let mut top = vec![marker(active)];
    let name_w = name.chars().count();
    let dir_w = dir.chars().count();
    top.extend(emphasis(&name, &[], Style::default().fg(TEXT), active));
    if dir_w > 0 && name_w + dir_w + 2 <= budget {
        top.push(plain(" ".repeat(budget - name_w - dir_w), Style::default()));
        top.extend(emphasis(&dir, &[], Style::default().fg(SECONDARY), active));
    }
    let line1 = finish(top, width, active);
    // line 2: numbers secondary, then the code window around the match
    let match_start = col.saturating_sub(1);
    let match_end = match_start + match_len;
    let before_chars = line_text[..match_start.min(line_text.len())]
        .chars()
        .count();
    let char_col = before_chars + 1;
    let numbers = format!(" {line}:{char_col}  ");
    let code_budget = budget.saturating_sub(numbers.chars().count());
    let (window, win_match) = match_window(line_text, match_start, match_end, code_budget);
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

/// The display window around the submatch: match-centered when the line
/// is long, ellipses mark clipped sides. Returns (text, (start,end) of
/// the match within the window, in CHARS).
fn match_window(line: &str, start: usize, end: usize, budget: usize) -> (String, (usize, usize)) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let total = chars.len();
    // char index of the match bounds
    let s = chars.iter().position(|(b, _)| *b >= start).unwrap_or(total);
    let e = chars.iter().position(|(b, _)| *b >= end).unwrap_or(total);
    if total <= budget {
        return (line.trim_start().to_string(), (s.saturating_sub(0), e));
    }
    let match_w = e.saturating_sub(s);
    if match_w + 2 >= budget {
        // the match alone overflows: show its head, no fake context
        let w: String = chars[s..s + budget.saturating_sub(1).max(1)]
            .iter()
            .map(|(_, c)| c)
            .collect();
        return (format!("{w}…"), (0, budget.saturating_sub(1)));
    }
    // center the match with balanced context
    let ctx = (budget - match_w - 2) / 2;
    let mut from = s.saturating_sub(ctx);
    if from + budget > total {
        from = total - budget;
    }
    let lead_clipped = from > 0;
    let take = budget - usize::from(lead_clipped);
    let tail_clipped = from + take < total;
    let take = take - usize::from(tail_clipped);
    let body: String = chars[from..from + take].iter().map(|(_, c)| c).collect();
    let text = format!(
        "{}{}{}",
        if lead_clipped { "…" } else { "" },
        body,
        if tail_clipped { "…" } else { "" }
    );
    let ws = s - from + usize::from(lead_clipped);
    let we = e - from + usize::from(lead_clipped);
    (text, (ws, we))
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
    for (ci, ch) in text.chars().enumerate() {
        let mut st = base;
        if ci < dim_prefix {
            st = st.fg(MUTED);
        }
        if match_cols.contains(&(ci as u32)) {
            st = st.fg(ACCENT).add_modifier(Modifier::BOLD);
        }
        if active {
            st = st.bg(SELECT_BG);
        }
        spans.push(Span::styled(ch.to_string(), st));
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
            text.rfind('/').map_or(0, |i| i + 1)
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
                i + 1
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

pub(super) fn render_replace_results(frame: &mut Frame, area: Rect, p: &strop_picker::Picker) {
    let visible = area.height as usize;
    let selected = p.selected;
    let start = if selected >= visible {
        selected + 1 - visible
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::with_capacity(visible);
    for (vi, row) in p.rows.iter().enumerate().skip(start).take(visible) {
        let active = vi == selected;
        let marker = if active { "▌" } else { " " };
        let excluded = p.is_excluded(row.item);
        let text_fg = if excluded {
            dim_color(MUTED)
        } else if active {
            TEXT
        } else {
            Color::Rgb(0xb8, 0xb4, 0xa9)
        };
        let mut spans = vec![
            Span::styled(
                marker,
                Style::default().fg(if active { ACCENT } else { MUTED }),
            ),
            Span::styled(
                if excluded { " ✗ " } else { "   " },
                Style::default().fg(if excluded {
                    Color::Rgb(0xf3, 0x8b, 0xa8)
                } else {
                    MUTED
                }),
            ),
        ];
        let Some(item) = p.items.get(row.item) else {
            continue;
        };
        if let strop_picker::Payload::Grep {
            line,
            col,
            match_len,
            line_text,
            ..
        } = &item.payload
        {
            spans.push(Span::styled(
                format!("{:>4}:{:<4}", line, col),
                Style::default().fg(MUTED),
            ));
            let (s, e) = strop_picker::replace_span(line_text, *col, *match_len);
            spans.push(Span::styled(
                text::clip_end(&line_text[..s], area.width as usize).into_owned(),
                Style::default().fg(text_fg),
            ));
            spans.push(Span::styled(
                text::clip_end(&line_text[s..e], area.width as usize).into_owned(),
                Style::default()
                    .fg(dim_color(TEXT))
                    .add_modifier(Modifier::CROSSED_OUT),
            ));
            if !p.replace_input.text.is_empty() {
                spans.push(Span::styled(
                    text::clip_end(&p.replace_input.text, area.width as usize).into_owned(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::styled(
                text::clip_end(&line_text[e..], area.width as usize).into_owned(),
                Style::default().fg(text_fg),
            ));
        } else {
            spans.push(Span::styled(
                text::clip_end(&item.text, area.width as usize).into_owned(),
                Style::default().fg(text_fg),
            ));
        }
        if active {
            spans = spans
                .into_iter()
                .map(|sp| sp.patch_style(Style::default().bg(SELECT_BG)))
                .collect();
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(BASE)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use strop_picker::{Item, Payload};

    fn sym(name: &str, container: &str, line: usize, kind: &str) -> Item {
        Item {
            badge: Some(kind.into()),
            text: format!("{name}  {container} · :{line}"),
            payload: Payload::Grep {
                path: PathBuf::from("/p/net.cpp"),
                line,
                col: 1,
                match_len: 1,
                line_text: String::new(),
            },
        }
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn symbol_row_aligns_names_and_keeps_container_when_it_fits() {
        let item = sym("retry_request", "net :: RequestDispatcher", 5, "meth");
        let line = symbol_row(&item, &[], 80, false);
        let text = line_text(&line);
        assert!(text.contains(" meth   "), "fixed chip slot: {text:?}");
        assert!(text.contains("retry_request"), "{text:?}");
        assert!(
            text.contains("net :: RequestDispatcher"),
            "container kept at width: {text:?}"
        );
        assert!(
            text.trim_end().ends_with(":5"),
            "location at the right: {text:?}"
        );
        // name starts at one column across kinds
        let a = symbol_row(&sym("a", "", 1, "fn"), &[], 80, false);
        let b = symbol_row(&sym("b", "", 1, "variant"), &[], 80, false);
        assert_eq!(line_text(&a).find('a'), line_text(&b).find('b'));
    }
}
