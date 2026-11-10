//! Picker result rows (0050, 0088 §2): per-kind composers over structured
//! payload fields — symbol chips, file basename-first, grep hits grouped
//! under one header per file (U7), diagnostics as severity + message +
//! location (U8). The selected LOGICAL row paints one full band (marker,
//! fields, trailing blanks) and query/match evidence is accent+bold on
//! top of it. Nothing here re-searches text or reparses display strings
//! beyond the fields the engine wrote.
use std::path::Path;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::card::{band, row_marker};
use super::super::clip;
use super::super::file_name::{self, Label, Role};
use super::super::style::Tokens;

mod location;
mod search;
#[cfg(test)]
mod tests;

pub(in crate::render::picker) use search::file_count;
use search::{file_header, search_rows};

/// The main results list: one display line per logical row, plus a
/// header line wherever a Search hit opens a new file group; replace
/// mode adds the delta line under each hit.
pub(super) fn render_results(
    frame: &mut Frame,
    t: &Tokens,
    area: Rect,
    picker: &strop_picker::Picker,
    root: &Path,
    tab_for_path: &impl Fn(&strop_workspace::ResourceLocation) -> usize,
) {
    let selected = picker.selected;
    let height = usize::from(area.height);
    let search = picker.kind == strop_picker::Kind::Search;
    let mut lines: Vec<Line> = Vec::with_capacity(height);
    if picker.rows.is_empty() && picker.input.text().is_empty() {
        if matches!(
            picker.kind,
            strop_picker::Kind::Files | strop_picker::Kind::Search
        ) {
            // 0051 R02: the empty field teaches the vocabulary — never an
            // obsolete flag hint
            lines.push(quiet(
                t,
                " language:rust path:src/ glob:\"**/*.rs\" hidden:include case:smart",
            ));
            lines.push(quiet(
                t,
                if picker.kind == strop_picker::Kind::Files {
                    " bare words are fuzzy paths · ctrl-space suggests"
                } else {
                    " bare words are literal content · ctrl-space suggests"
                },
            ));
        } else {
            lines.push(quiet(
                t,
                if picker.streaming {
                    " loading…"
                } else if !picker.items.is_empty() {
                    " preparing results…"
                } else {
                    " no entries available"
                },
            ));
        }
    }
    if picker.rows.is_empty() && !picker.input.text().is_empty() {
        // 0050 §8: an empty filtered set is explained, never a blank
        // card with a nonzero count
        let noun = match picker.kind {
            strop_picker::Kind::Search => "matches".to_string(),
            _ => format!("{} match", picker.kind.title().trim()),
        };
        lines.push(Line::from(Span::styled(
            format!(" No {noun} “{}”", picker.input.text()),
            Style::default().fg(t.secondary),
        )));
        lines.push(quiet(t, " edit query · esc close"));
    }
    let start_row = if search {
        picker.scroll_top
    } else {
        selected.saturating_sub(height.saturating_sub(1))
    };
    let name_column = if picker.kind == strop_picker::Kind::Files {
        file_name_column(picker, start_row, height, area.width)
    } else {
        0
    };
    let mut last_file: Option<&Path> = None;
    for (vi, row) in picker.rows.iter().enumerate().skip(start_row) {
        if lines.len() >= height {
            break;
        }
        let active = vi == selected;
        let Some(item) = picker.items.get(row.item) else {
            continue;
        };
        let match_cols = picker.match_columns(row);
        if search {
            let strop_picker::Payload::Grep { location, .. } = &item.payload else {
                lines.push(generic_row(t, item, match_cols, area.width, active));
                continue;
            };
            if last_file != Some(location.path.as_path()) {
                last_file = Some(location.path.as_path());
                lines.push(file_header(
                    t,
                    location,
                    root,
                    picker.file_match_count(row.item),
                    area.width,
                ));
                if lines.len() >= height {
                    break;
                }
            }
            let tab = tab_for_path(location);
            lines.extend(search_rows(
                t,
                item,
                picker
                    .replacement_visible
                    .then_some(picker.replace_input.text()),
                picker.is_excluded(row.item),
                area.width,
                active,
                tab,
            ));
            continue;
        }
        lines.push(match picker.kind {
            strop_picker::Kind::Symbols | strop_picker::Kind::WorkspaceSymbols => {
                symbol_row(t, item, match_cols, area.width, active)
            }
            strop_picker::Kind::Files => {
                file_row(t, &item.text, match_cols, name_column, area.width, active)
            }
            strop_picker::Kind::Buffers => buffer_row(t, item, match_cols, area.width, active),
            strop_picker::Kind::Diagnostics => diagnostic_row(t, item, root, area.width, active),
            strop_picker::Kind::Locations => {
                location::location_row(t, item, match_cols, root, area.width, active)
            }
            strop_picker::Kind::Theme => match &item.payload {
                strop_picker::Payload::ThemeChoice(id) => {
                    theme_row(t, *id, match_cols, area.width, active)
                }
                _ => generic_row(t, item, match_cols, area.width, active),
            },
            _ => generic_row(t, item, match_cols, area.width, active),
        });
    }
    lines.truncate(height);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        area,
    );
}

fn quiet(t: &Tokens, text: &'static str) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(t.muted)))
}

/// Pad a row to the full width and print every cell; the band covers
/// the blanks (0050 §4). Content past the width clips with an ellipsis.
fn finish(t: &Tokens, mut spans: Vec<Span<'static>>, width: u16, active: bool) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    for span in &mut spans {
        if span.content.chars().any(char::is_control) {
            span.content = strop_core::layout::printable_text(span.content.as_ref())
                .into_owned()
                .into();
        }
    }
    let total: usize = spans.iter().map(Span::width).sum();
    let clipped = total > usize::from(width);
    let budget = usize::from(width) - usize::from(clipped);
    let mut used = 0;
    let mut output = Vec::new();
    for mut span in spans {
        if used >= budget {
            break;
        }
        if span.width() > budget - used {
            let mut prefix = String::new();
            for grapheme in
                unicode_segmentation::UnicodeSegmentation::graphemes(span.content.as_ref(), true)
            {
                let cells = clip::width(grapheme);
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
        output.push(Span::styled("…", Style::default().fg(t.muted)));
    }
    band(Line::from(output), width, active, t)
}

/// Emphasize the query match inside a field's text: accent+bold chars,
/// never background blocks (0001 §4). `cols` are char indices.
fn emphasis(t: &Tokens, value: &str, cols: &[u32], base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut character = 0;
    let mut run = String::new();
    let mut run_matched = false;
    let flush = |spans: &mut Vec<Span<'static>>, run: &mut String, matched: bool| {
        if run.is_empty() {
            return;
        }
        let style = if matched {
            base.fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            base
        };
        spans.push(Span::styled(std::mem::take(run), style));
    };
    for (glyph, grapheme) in strop_core::layout::RopeGraphemes::new(value.into(), 4) {
        let end = character + grapheme.chars().count();
        let matched = cols
            .iter()
            .any(|&column| character <= column as usize && (column as usize) < end);
        character = end;
        if glyph.width == 0 {
            continue;
        }
        if matched != run_matched {
            flush(&mut spans, &mut run, run_matched);
            run_matched = matched;
        }
        run.push_str(strop_core::layout::printable_grapheme(&grapheme));
    }
    flush(&mut spans, &mut run, run_matched);
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

fn chip(t: &Tokens, badge: Option<&str>) -> Span<'static> {
    let label = badge.unwrap_or("");
    Span::styled(
        format!(" {:<7} ", label),
        Style::default().fg(t.kind(label)),
    )
}

/// ` chip  name  container                  :line` — location holds the
/// right edge; the container drops before the name truncates.
fn symbol_row(
    t: &Tokens,
    item: &strop_picker::Item,
    match_cols: &[u32],
    width: u16,
    active: bool,
) -> Line<'static> {
    let (name, container, location) = symbol_fields(item);
    let budget = (width as usize).saturating_sub(1 + CHIP_W);
    let loc_w = clip::width(&location);
    let name_w = if location.is_empty() {
        budget
    } else {
        budget.saturating_sub(loc_w + 1)
    };
    let name_budget = name_w.saturating_sub(if container.is_empty() { 0 } else { 1 });
    let mut spans = vec![row_marker(active, t), chip(t, item.badge.as_deref())];
    let name_chars = clip::width(&name);
    if name_chars <= name_budget {
        spans.extend(emphasis(t, &name, match_cols, Style::default().fg(t.text)));
        let used = 1 + CHIP_W + name_chars;
        // container sits after the name; pad between the fields
        if !container.is_empty() && used + 1 + clip::width(&container) + loc_w < width as usize {
            spans.push(plain(" ", Style::default()));
            spans.extend(emphasis(
                t,
                &container,
                &[],
                Style::default().fg(t.secondary),
            ));
        } else if used < width as usize {
            spans.push(plain(" ", Style::default()));
        }
    } else {
        let clipped = clip::clip_end(&name, name_budget);
        spans.extend(emphasis(
            t,
            &clipped,
            match_cols,
            Style::default().fg(t.text),
        ));
    }
    if !location.is_empty() {
        let used: usize = spans.iter().map(Span::width).sum();
        let pad = (width as usize).saturating_sub(used + loc_w + 1);
        spans.push(plain(" ".repeat(pad.max(1)), Style::default()));
        spans.push(plain(&location, Style::default().fg(t.secondary)));
    }
    finish(t, spans, width, active)
}

/// (name, container, location) from the symbol row's own construction
/// fields — the text is `name  container · :line` from the Symbols arm.
fn symbol_fields(item: &strop_picker::Item) -> (String, String, String) {
    // the Symbols arm writes "name  container · :line" or "name  · :line";
    // status rows write "name  path · reason" (0063 §2)
    let text = item.text.trim_end();
    let (head, tail) = text.rsplit_once('·').unwrap_or((text, ""));
    let location = match &item.payload {
        strop_picker::Payload::Grep { line, .. } => format!(":{line}"),
        strop_picker::Payload::Remote { line, .. } => format!(":{line}"),
        // A project status row's right-edge field is its reason.
        strop_picker::Payload::ProjectStatus(_) => tail.trim().to_string(),
        _ => String::new(),
    };
    let mut parts = head.splitn(2, "  ");
    let name = parts.next().unwrap_or("").trim().to_string();
    let container = parts
        .next()
        .map(|c| c.trim().to_string())
        .unwrap_or_default();
    (name, container, location)
}

// ---- files ----------------------------------------------------------------

/// The directory column: the widest visible basename plus a gap, capped
/// at half the row so a long name cannot push every directory off.
fn file_name_column(
    picker: &strop_picker::Picker,
    start: usize,
    height: usize,
    width: u16,
) -> usize {
    let widest = picker
        .rows
        .iter()
        .skip(start)
        .take(height)
        .filter_map(|row| picker.items.get(row.item))
        .map(|item| {
            let name = item.text.rsplit('/').next().unwrap_or(&item.text);
            clip::width(name)
        })
        .max()
        .unwrap_or(0);
    (widest + 2).min(usize::from(width) / 2)
}

/// ` STATUS.md      plans/` — basename bright (query matches on it
/// emphasized), the directory in `fg.secondary` after it at one shared
/// column, elided from the left so the parent nearest the file survives.
fn file_row(
    t: &Tokens,
    value: &str,
    match_cols: &[u32],
    name_column: usize,
    width: u16,
    active: bool,
) -> Line<'static> {
    let (dir, name) = match value.rfind('/') {
        Some(i) => (&value[..i], &value[i + 1..]),
        None => ("", value),
    };
    let name_off = dir.chars().count() + usize::from(!dir.is_empty());
    let (dir_cols, name_cols) = split_match_cols(match_cols, dir.chars().count(), name_off);
    let budget = (width as usize).saturating_sub(2);
    let mut spans = vec![row_marker(active, t), plain(" ", Style::default())];
    let name_w = clip::width(name);
    let column = name_column.max(name_w + 2).min(budget);
    let shown_name = clip::clip_end(name, column.saturating_sub(2).max(1));
    spans.extend(emphasis(
        t,
        &shown_name,
        &name_cols,
        file_name::name_style(Role::Row, t),
    ));
    let room = budget.saturating_sub(column);
    if !dir.is_empty() && room >= 4 {
        let pad = column.saturating_sub(clip::width(&shown_name));
        spans.push(plain(" ".repeat(pad), Style::default()));
        let shown_dir = format!("{dir}/");
        let shown_dir = if clip::width(&shown_dir) > room {
            clip::clip_start(&shown_dir, room).into_owned()
        } else {
            shown_dir
        };
        // columns only survive while the directory is whole
        let cols: &[u32] = if shown_dir.starts_with('…') {
            &[]
        } else {
            &dir_cols
        };
        spans.extend(emphasis(t, &shown_dir, cols, file_name::dir_style(t)));
    }
    finish(t, spans, width, active)
}

/// Query matches fall on the directory or on the name: matches inside
/// the basename shift into the name's own coordinates, matches in the
/// directory emphasize the context field (0050 §6).
fn split_match_cols(match_cols: &[u32], dir_chars: usize, name_off: usize) -> (Vec<u32>, Vec<u32>) {
    let dir_cols = match_cols
        .iter()
        .copied()
        .filter(|c| (*c as usize) < dir_chars)
        .collect();
    let name_cols = match_cols
        .iter()
        .filter_map(|c| c.checked_sub(name_off as u32))
        .collect();
    (dir_cols, name_cols)
}

// ---- buffers --------------------------------------------------------------

/// ` crates/strop/src/render/card.rs` — the open file in the row idiom
/// (directory secondary, name text; a remote file behind its `ssh:host`
/// authority), the rest of the label (a terminal's ` · phase`) in
/// muted. A terminal's `term://` name is one identity, never split.
fn buffer_row(
    t: &Tokens,
    item: &strop_picker::Item,
    match_cols: &[u32],
    width: u16,
    active: bool,
) -> Line<'static> {
    let text = clip::clip_end(&item.text, width.saturating_sub(2) as usize);
    let (path, rest) = text.split_once(" · ").unwrap_or((&text, ""));
    let label = if item.badge.is_some() {
        Label::local(path)
    } else {
        Label::parse(path)
    };
    let (dir, name) = if item.badge.is_some() {
        ("", path)
    } else {
        file_name::split(&label.path)
    };
    // query matches are char columns of the item text; a parsed remote
    // label is re-spelt (`ssh:host /path`), so its matches no longer map
    let match_cols: &[u32] = if label.authority.is_some() {
        &[]
    } else {
        match_cols
    };
    let dir_chars = dir.chars().count();
    let (dir_cols, name_cols) = split_match_cols(match_cols, dir_chars, dir_chars);
    let mut spans = vec![row_marker(active, t), plain(" ", Style::default())];
    if let Some(authority) = &label.authority {
        spans.push(plain(
            format!("{authority} "),
            file_name::authority_style(t),
        ));
    }
    spans.extend(emphasis(t, dir, &dir_cols, file_name::dir_style(t)));
    spans.extend(emphasis(
        t,
        name,
        &name_cols,
        file_name::name_style(Role::Row, t),
    ));
    if !rest.is_empty() {
        spans.push(plain(format!(" · {rest}"), Style::default().fg(t.muted)));
    }
    finish(t, spans, width, active)
}

// ---- themes ---------------------------------------------------------------

/// Selection and application are separate: the band marks the cursor and
/// `●` (rule 5) marks the live palette. Six swatches — base, surface,
/// text, accent, keyword, string — preview the palette's planes and ink.
fn theme_row(
    t: &Tokens,
    id: strop_core::theme::ThemeId,
    match_cols: &[u32],
    width: u16,
    active: bool,
) -> Line<'static> {
    let applied = id == t.id;
    let mut spans = vec![row_marker(active, t), plain(" ", Style::default())];
    spans.extend(emphasis(
        t,
        id.label(),
        match_cols,
        Style::default().fg(if active { t.text } else { t.secondary }),
    ));
    if applied {
        spans.push(plain(" ●", Style::default().fg(t.accent)));
    }
    // A complete strip is right-aligned only after the whole label fits.
    // Narrow rows retain the selection, application mark and name instead.
    let used: usize = spans.iter().map(Span::width).sum();
    let width = width as usize;
    if width >= 24 && width >= used + 8 {
        spans.push(plain(" ".repeat(width - used - 7), Style::default()));
        let palette = id.palette();
        for role in [
            palette.bg.base,
            palette.bg.surface,
            palette.fg.text,
            palette.accent,
            palette.syn.keyword,
            palette.syn.string,
        ] {
            spans.push(Span::styled(
                "■",
                Style::default().fg(crate::render::style::color(role)),
            ));
        }
    }
    finish(t, spans, width as u16, active)
}

// ---- diagnostics ------------------------------------------------------------

/// `▌ ● cannot find function …      editmap.rs:25` (U8): severity glyph
/// in its colour, the message in text, the location in secondary at
/// the right edge. The engine writes `{path}:{line} {E|W|I|H} {message}`
/// and the message into the payload's `line_text`; the severity letter
/// is read from the row text at the offset those fields pin.
fn diagnostic_row(
    t: &Tokens,
    item: &strop_picker::Item,
    root: &Path,
    width: u16,
    active: bool,
) -> Line<'static> {
    let (location, line, message) = match &item.payload {
        strop_picker::Payload::Grep {
            location,
            line,
            line_text,
            ..
        } => (
            Label::from_location(location, root),
            *line,
            line_text.to_string(),
        ),
        strop_picker::Payload::Remote {
            endpoint,
            path,
            line,
            ..
        } => (
            Label::from_location(
                &strop_workspace::ResourceLocation::remote(endpoint.clone(), path.clone()),
                root,
            ),
            *line,
            item.text.clone(),
        ),
        _ => return generic_row(t, item, &[], width, active),
    };
    let severity = item
        .text
        .split(' ')
        .nth(1)
        .and_then(|word| word.chars().next());
    let (glyph, color) = match severity {
        Some('E') => ("●", t.error),
        Some('W') => ("▲", t.warning),
        Some('I') => ("ℹ", t.info),
        _ => ("·", t.hint),
    };
    let line = format!(":{line}");
    // marker + ` ● ` = 4 cells before the message
    let budget = usize::from(width).saturating_sub(4);
    let loc_w = location.width() + clip::width(&line);
    let message_budget = budget.saturating_sub(loc_w + 2);
    let message = message.replace('\n', " · ");
    let message = clip::clip_end(&message, message_budget.max(4));
    let used = 4 + clip::width(&message);
    let pad = usize::from(width).saturating_sub(used + loc_w).max(1);
    let mut spans = vec![
        row_marker(active, t),
        plain(format!(" {glyph} "), Style::default().fg(color)),
        plain(message.into_owned(), Style::default().fg(t.text)),
        plain(" ".repeat(pad), Style::default()),
    ];
    spans.extend(location.spans(Role::Row, t));
    spans.push(plain(line, Style::default().fg(t.secondary)));
    finish(t, spans, width, active)
}

// ---- generic rows (buffers, jumps, remote, locations) -------------------------

fn generic_row(
    t: &Tokens,
    item: &strop_picker::Item,
    match_cols: &[u32],
    width: u16,
    active: bool,
) -> Line<'static> {
    let text = clip::clip_end(&item.text, width.saturating_sub(2) as usize);
    let dim_prefix = locator_prefix_chars(item);
    let base = Style::default().fg(if active { t.text } else { t.secondary });
    let mut spans = vec![row_marker(active, t), plain(" ", Style::default())];
    let total = text.chars().count();
    let (locator, rest) = split_chars(&text, dim_prefix.min(total));
    spans.extend(emphasis(t, locator, match_cols, base.fg(t.muted)));
    let rest_cols: Vec<u32> = match_cols
        .iter()
        .filter_map(|c| c.checked_sub(dim_prefix as u32))
        .collect();
    spans.extend(emphasis(t, rest, &rest_cols, base));
    finish(t, spans, width, active)
}

fn split_chars(text: &str, chars: usize) -> (&str, &str) {
    let byte = text
        .char_indices()
        .nth(chars)
        .map_or(text.len(), |(byte, _)| byte);
    text.split_at(byte)
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
        Payload::Grep { location, line, .. } => {
            location.label().chars().count() + 1 + digits(*line)
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
