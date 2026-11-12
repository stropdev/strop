//! Search (grep) rows: one header per file group (U7), one row per hit
//! with its match emphasized, the replacement delta beneath it when a
//! replacement is visible, and excluded hits struck through (rule 5).
use std::collections::HashSet;
use std::path::Path;

use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use super::super::window::{match_window, replacement_window};
use super::{emphasis, finish, generic_row, plain};
use crate::render::block::heading;
use crate::render::card::row_marker;
use crate::render::clip;
use crate::render::file_name;
use crate::render::style::Tokens;

/// Distinct files among the included Search hits.
pub(in crate::render::picker) fn file_count(picker: &strop_picker::Picker) -> usize {
    let mut files: HashSet<&Path> = HashSet::new();
    for (index, item) in picker.items.iter().enumerate() {
        if picker.is_excluded(index) {
            continue;
        }
        if let strop_picker::Payload::Grep { location, .. } = &item.payload {
            files.insert(location.path.as_path());
        }
    }
    files.len()
}

// ---- Search rows ------------------------------------------------------------

/// One header per file group (U7): the ruled heading every block draws
/// (0100 §3.2) with a one-cell blank gutter — the file name in the
/// heading idiom, a faint leader, the file's hit count in secondary at
/// the right edge, and one cell of padding after it.
pub(super) fn file_header(
    t: &Tokens,
    location: &strop_workspace::ResourceLocation,
    root: &Path,
    hits: Option<usize>,
    width: u16,
) -> Line<'static> {
    let label = file_name::Label::from_location(location, root);
    let heading = heading::Heading {
        gutter: 1,
        ruled: false,
        name: heading::Name::Path(&label),
        marks: Vec::new(),
        fact: hits.map_or_else(Vec::new, |hits| heading::fact(t, hits.to_string())),
        focus: heading::Focus::Plain,
    };
    let drawn = heading.draw(t, usize::from(width).saturating_sub(1));
    let mut spans = drawn.gutter;
    spans.extend(drawn.text);
    spans.push(plain(" ", Style::default()));
    finish(t, spans, width, false)
}

/// Match columns for the window's own char coordinates.
fn match_cols_in_window((start, end): (usize, usize)) -> Vec<u32> {
    (start as u32..end as u32).collect()
}

/// One hit: `▌  22  …ot file identity. The modeline…` — line number
/// right-aligned in secondary, the window with its match emphasized;
/// an excluded hit renders struck through in muted (no `[x]`, rule 5).
/// With a replacement visible, the `+` delta line follows.
#[allow(clippy::too_many_arguments)]
pub(super) fn search_rows(
    t: &Tokens,
    item: &strop_picker::Item,
    replacer: Option<&strop_picker::Replacer>,
    excluded: bool,
    width: u16,
    active: bool,
    tab: usize,
) -> Vec<Line<'static>> {
    let strop_picker::Payload::Grep {
        line,
        col,
        match_len,
        line_text,
        ..
    } = &item.payload
    else {
        return vec![generic_row(t, item, &[], width, active)];
    };
    // The same checked byte span drives source evidence and the optional delta.
    let (s, e) = strop_picker::replace_span(line_text, *col, *match_len);
    let numbers = format!(" {line:>4}  ");
    let numbers_w = clip::width(&numbers);
    let code_budget = usize::from(width).saturating_sub(1 + numbers_w + 2);
    // The hit's own replacement (captures expand per hit); a hit the term
    // no longer matches shows no delta, as the review will refuse it.
    let delta = replacer
        .filter(|_| !excluded)
        .and_then(|replacer| replacer.text(&line_text[s..e]));
    let delta = delta.as_deref();
    let ink = if excluded {
        Style::default()
            .fg(t.muted)
            .add_modifier(Modifier::CROSSED_OUT)
    } else if delta.is_some() {
        Style::default().fg(t.diff_del)
    } else {
        Style::default().fg(t.text)
    };
    let (old_window, old_match) = match_window(line_text, s, e, code_budget, tab);
    let evidence = if excluded {
        Vec::new()
    } else {
        match_cols_in_window(old_match)
    };
    let mut old = vec![row_marker(active, t)];
    old.push(plain(&numbers, Style::default().fg(t.secondary)));
    old.push(plain(if delta.is_some() { "- " } else { "" }, ink));
    old.extend(emphasis(t, &old_window, &evidence, ink));
    let hit = finish(t, old, width, active);
    let Some(delta) = delta else {
        let mut lines = vec![hit];
        if replacer.is_some() {
            lines.push(finish(t, Vec::new(), width, active));
        }
        return lines;
    };
    let (new_window, new_match) = replacement_window(line_text, s, e, delta, code_budget, tab);
    let add = Style::default().fg(t.diff_add);
    let mut new = vec![plain(" ", Style::default())];
    new.push(plain(" ".repeat(numbers_w), Style::default()));
    new.push(plain("+ ", add));
    new.extend(emphasis(
        t,
        &new_window,
        &match_cols_in_window(new_match),
        add,
    ));
    vec![hit, finish(t, new, width, active)]
}
