//! Folder decoration preserves the real buffer prefix byte-for-byte. Metadata
//! comes from the listing; selection/search/caret keep the shared text renderer.
use crate::editor::Directory;
use crate::render::{diff, ACCENT, MUTED, TEXT};
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use strop_core::id::LineIndex;
use strop_workspace::{EntryKind, ListingState};

#[cfg(test)]
mod tests;

pub(super) fn row(
    directory: &Directory,
    line: usize,
    text: ropey::RopeSlice<'_>,
    focused: bool,
) -> Option<diff::RowDecoration> {
    if directory.draft.is_some() {
        return None;
    }
    let text = text.to_string();
    let quiet = Style::default().fg(MUTED);
    if line == 0 {
        let boundary = directory.location.label().len();
        let (path, count) = text.split_at_checked(boundary)?;
        let leaf = path.rfind('/').map_or(0, |slash| {
            if slash + 1 == path.len() {
                slash
            } else {
                slash + 1
            }
        });
        return Some(diff::RowDecoration {
            spans: vec![
                Span::styled(path[..leaf].to_owned(), quiet),
                Span::styled(
                    path[leaf..].to_owned(),
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(count.to_owned(), quiet),
            ],
            row_bg: Some(diff::BAND_BG),
        });
    }
    if line == 1 {
        let mut spans = vec![Span::styled(
            text,
            Style::default().fg(if focused { ACCENT } else { MUTED }),
        )];
        spans.push(Span::styled(
            if directory.parent().is_some() {
                "  parent directory"
            } else {
                "  filesystem root"
            },
            quiet,
        ));
        spans.push(Span::styled("   ·   Enter open   Space a actions", quiet));
        return Some(diff::RowDecoration {
            spans,
            row_bg: focused.then_some(diff::CURSOR_ROW_BG),
        });
    }
    let entry = directory.entry(LineIndex::new(line))?;
    let name = entry.name.display();
    let suffix = usize::from(matches!(
        entry.observation.kind,
        EntryKind::Directory | EntryKind::SymbolicLink
    ));
    let (label, metadata) = text.split_at_checked(name.len() + suffix)?;
    if !label.starts_with(&name) {
        return None;
    }
    let color = match entry.observation.kind {
        EntryKind::Directory => ACCENT,
        EntryKind::SymbolicLink => Color::Rgb(0x89, 0xb4, 0xfa),
        EntryKind::Unknown => MUTED,
        _ => TEXT,
    };
    let marked = directory.marked.contains_key(&entry.name);
    let label_style = if focused || marked || entry.observation.kind == EntryKind::Directory {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    Some(diff::RowDecoration {
        spans: vec![
            Span::styled(label.to_owned(), label_style),
            Span::styled(metadata.to_owned(), quiet),
        ],
        row_bg: (focused || marked).then_some(diff::CURSOR_ROW_BG),
    })
}

pub(super) fn empty_message(directory: &Directory) -> &'static str {
    match directory.state {
        ListingState::Complete if directory.entries.is_empty() => "Empty folder",
        ListingState::Complete => "No entries match this filter",
        ListingState::Limited { .. } => "No matching entries in this limited listing",
        ListingState::Failed { .. } => "Listing incomplete — refresh to try again",
    }
}
