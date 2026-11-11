//! Location rows (0093 §5): `src/main.rs:15  let n = hone(&mut blade, 4);`
//! — the place in the file-name idiom (0088 §2), then what is written
//! there. The place yields its directory first, so the name, the line and
//! some source text survive any width a row can have.
use std::path::Path;

use ratatui::style::Style;
use ratatui::text::Line;

use super::{emphasis, finish, generic_row, plain, split_match_cols};
use crate::render::card::row_marker;
use crate::render::clip;
use crate::render::file_name::{self, Label, Role};
use crate::render::style::Tokens;

/// Cells a place keeps however narrow the row: enough for `…/name.rs`.
const PLACE_FLOOR: usize = 12;

pub(super) fn location_row(
    t: &Tokens,
    item: &strop_picker::Item,
    match_cols: &[u32],
    root: &Path,
    width: u16,
    active: bool,
) -> Line<'static> {
    let (place, line, source) = match &item.payload {
        strop_picker::Payload::Grep {
            location,
            line,
            line_text,
            ..
        } => (
            Label::from_location(location, root),
            *line,
            line_text.as_ref(),
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
            "",
        ),
        _ => return generic_row(t, item, match_cols, width, active),
    };
    let number = format!(":{line}");
    let budget = usize::from(width).saturating_sub(2);
    // With source text to show, the place takes at most two thirds.
    let place_room = if source.is_empty() {
        budget
    } else {
        budget * 2 / 3
    }
    .saturating_sub(clip::width(&number))
    .max(PLACE_FLOOR);
    let whole = place.width() <= place_room;
    let place = place.clipped(place_room);
    let mut spans = vec![row_marker(active, t), plain(" ", Style::default())];
    // The row text the query ranked is `{path}:{line}  {source}`: while
    // the path is drawn whole its matched cells line up with it.
    if whole && place.authority.is_none() && item.text.starts_with(&place.path) {
        let (dir, name) = file_name::split(&place.path);
        let dir_chars = dir.chars().count();
        let (dir_cols, name_cols) = split_match_cols(match_cols, dir_chars, dir_chars);
        spans.extend(emphasis(t, dir, &dir_cols, file_name::dir_style(t)));
        spans.extend(emphasis(
            t,
            name,
            &name_cols,
            file_name::name_style(Role::Row, t),
        ));
    } else {
        spans.extend(place.spans(Role::Row, t));
    }
    spans.push(plain(number.clone(), Style::default().fg(t.secondary)));
    if !source.is_empty() {
        let used = place.width() + clip::width(&number) + 2;
        let shown = clip::clip_end(source, budget.saturating_sub(used));
        let source_at = (item.text.chars().count()).saturating_sub(source.chars().count());
        let source_cols: Vec<u32> = match_cols
            .iter()
            .filter_map(|column| column.checked_sub(source_at as u32))
            .collect();
        spans.push(plain("  ", Style::default()));
        spans.extend(emphasis(
            t,
            &shown,
            &source_cols,
            Style::default().fg(t.text),
        ));
    }
    finish(t, spans, width, active)
}
