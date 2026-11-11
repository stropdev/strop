//! Where the cursor was in a status buffer, in terms that survive the
//! buffer being rebuilt from a new snapshot. Pure.

use super::{StatusRow, StatusView};

/// Where the cursor was, in terms that survive a rebuild.
pub(super) struct Anchor {
    line: usize,
    entry: Option<strop_git::StatusEntry>,
    /// The entry's position inside its section.
    ordinal: usize,
    section: Option<strop_git::Section>,
    /// Hunk index, its header text, and the cursor's offset below it.
    hunk: Option<(usize, String, usize)>,
}

impl Anchor {
    pub(super) fn capture(view: &StatusView, line: usize) -> Self {
        let row = view.row(line);
        let entry = row
            .and_then(StatusRow::entry)
            .and_then(|index| view.entry(index))
            .cloned();
        let ordinal = entry
            .as_ref()
            .and_then(|entry| {
                view.tree
                    .section(entry.section)
                    .position(|member| member == entry)
            })
            .unwrap_or(0);
        let hunk = row.and_then(StatusRow::hunk).and_then(|(entry, hunk)| {
            let header = view.line_of(StatusRow::HunkHeader { entry, hunk })?;
            Some((hunk, view.hunk(entry, hunk)?.header(), line - header))
        });
        Anchor {
            line,
            entry,
            ordinal,
            section: match row {
                Some(StatusRow::Section(section)) => Some(section),
                _ => None,
            },
            hunk,
        }
    }

    /// The line to land on in the rebuilt view: the same hunk, else the
    /// same file, else whatever now sits where the file was.
    pub(super) fn land(&self, view: &StatusView) -> usize {
        let first_file = || {
            view.rows
                .iter()
                .position(|row| matches!(row, StatusRow::File(_)))
                .unwrap_or(0)
        };
        if let Some(section) = self.section {
            return view
                .line_of(StatusRow::Section(section))
                .unwrap_or_else(first_file);
        }
        let Some(entry) = &self.entry else {
            return if view.rows.is_empty() {
                0
            } else {
                self.line.min(view.rows.len() - 1)
            };
        };
        if let Some(index) = view.tree.entries.iter().position(|member| member == entry) {
            if let Some((hunk, header, within)) = &self.hunk {
                // the same hunk keeps the cursor's place inside it; a
                // different hunk in that slot (the old one was staged)
                // lands on its header
                for candidate in [*hunk, hunk.saturating_sub(1)] {
                    let Some(found) = view.hunk(index, candidate) else {
                        continue;
                    };
                    let Some(line) = view.line_of(StatusRow::HunkHeader {
                        entry: index,
                        hunk: candidate,
                    }) else {
                        continue;
                    };
                    let same = candidate == *hunk && found.header() == *header;
                    return if same { line + within } else { line };
                }
            }
            return view
                .line_of(StatusRow::File(index))
                .unwrap_or_else(first_file);
        }
        // the entry left its section: the entry now in its place
        let members: Vec<usize> = (0..view.tree.entries.len())
            .filter(|index| view.tree.entries[*index].section == entry.section)
            .collect();
        members
            .get(self.ordinal.min(members.len().saturating_sub(1)))
            .and_then(|index| view.line_of(StatusRow::File(*index)))
            .unwrap_or_else(first_file)
    }
}
