//! A folder row: the buffer text is the entry's name, styled by kind;
//! size and age (or, for an edited row, what `:w` will do with it) are
//! virtual text in a column measured once per listing. The pending-change
//! words come from the engine's `Draft::row_change`, the same function the
//! batch is compiled with, so the row never says something `:w` won't do.
use crate::editor::{Directory, RowChange};
use crate::render::block::Painted;
use crate::render::style::Tokens;
use ratatui::{
    style::{Modifier, Style},
    text::Span,
};
use strop_core::id::LineIndex;
use strop_workspace::{EntryKind, ListingState, Observation};

#[cfg(test)]
mod tests;

pub(super) fn row(
    t: &Tokens,
    directory: &Directory,
    now: i64,
    line: usize,
    text: ropey::RopeSlice<'_>,
    focused: bool,
) -> Option<Painted<'static>> {
    let name = text.to_string();
    let quiet = Style::default().fg(t.muted);
    let pending = Style::default().fg(t.accent);
    let row_bg = focused.then_some(t.cursorline);
    if name.is_empty() {
        let quiet = match row_bg {
            Some(bg) => quiet.bg(bg),
            None => quiet,
        };
        let note = (line == 0 && directory.visible.is_empty())
            .then(|| (placeholder(directory).to_string(), quiet));
        return Some(Painted {
            row_bg,
            note,
            ..Default::default()
        });
    }
    let (kind, note) = match &directory.draft {
        Some(draft) => match draft.row_change(line, &name) {
            RowChange::Unchanged(source) => (
                source.observation.kind,
                metadata(&source.observation, now).map(|text| (text, quiet)),
            ),
            RowChange::Renamed(source) => (
                source.observation.kind,
                Some((format!("was {}", source.text), pending)),
            ),
            RowChange::Copy(source) => (
                source.observation.kind,
                Some((
                    format!(
                        "copy of {}",
                        Directory::relative_label(&directory.location, &source.location)
                    ),
                    pending,
                )),
            ),
            RowChange::New | RowChange::Blank => (
                if name.ends_with('/') {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                Some(("new".to_string(), pending)),
            ),
        },
        None => {
            let entry = directory.entry(LineIndex::new(line))?;
            (
                entry.observation.kind,
                metadata(&entry.observation, now).map(|text| (text, quiet)),
            )
        }
    };
    let width = crate::render::clip::width(&name);
    let note = note.and_then(|(text, style)| {
        let pad = directory.name_width.checked_sub(width)?;
        let style = match row_bg {
            Some(bg) => style.bg(bg),
            None => style,
        };
        (pad >= 2).then(|| (format!("{}{text}", " ".repeat(pad)), style))
    });
    let color = match kind {
        EntryKind::Directory => t.accent,
        EntryKind::SymbolicLink => t.syn.function,
        EntryKind::Unknown => t.muted,
        _ => t.text,
    };
    let style = if focused || kind == EntryKind::Directory {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    Some(Painted {
        text: vec![Span::styled(name, style)],
        row_bg,
        note,
        ..Default::default()
    })
}

/// What an empty folder, or a filter that matches nothing, says on row 1.
fn placeholder(directory: &Directory) -> &'static str {
    match directory.state {
        ListingState::Complete if directory.entries.is_empty() => {
            "empty folder  ·  o adds a name, :w creates it"
        }
        ListingState::Complete if !directory.filter.is_empty() => {
            "nothing matches  ·  :filter clears"
        }
        ListingState::Complete => "only dotfiles here  ·  g. shows them",
        ListingState::Limited { .. } => "nothing matches in this cut-short listing",
        ListingState::Failed { .. } => "listing incomplete  ·  :e lists it again",
    }
}

/// `   8.5K   1d`: a size column (blank for a folder) and an age column.
fn metadata(observation: &Observation, now: i64) -> Option<String> {
    let size = match (observation.kind, observation.size) {
        (EntryKind::Directory, _) | (_, None) => String::new(),
        (_, Some(size)) => human_size(size),
    };
    let age = observation
        .modified
        .map(|modified| age(now.saturating_sub(modified.seconds)))
        .unwrap_or_default();
    Some(format!("{size:>5}  {age:>3}"))
}

/// `96`, `8.5K`, `190K`, `2.8M`: three significant characters at most.
pub(super) fn human_size(size: u64) -> String {
    if size < 1000 {
        return size.to_string();
    }
    let units = ['K', 'M', 'G', 'T', 'P', 'E'];
    let mut value = size as f64 / 1024.0;
    let mut unit = 0;
    while value >= 999.5 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value < 9.95 {
        format!("{value:.1}{}", units[unit])
    } else {
        format!("{value:.0}{}", units[unit])
    }
}

/// `now`, `5m`, `5h`, `3d`, `2w`, `1y`.
pub(super) fn age(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const YEAR: i64 = 365 * DAY;
    match seconds {
        ..MINUTE => "now".into(),
        MINUTE..HOUR => format!("{}m", seconds / MINUTE),
        HOUR..DAY => format!("{}h", seconds / HOUR),
        DAY..WEEK => format!("{}d", seconds / DAY),
        WEEK..YEAR => format!("{}w", seconds / WEEK),
        YEAR.. => format!("{}y", seconds / YEAR),
    }
}
