//! Left-margin chrome (0011, 0032 §3): the blame column and the
//! commit file sidebar. Its immutable tree is built by the engine's Git
//! worker; rendering borrows only visible rows. Native file indices
//! preserve identity.

use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use strop_engine::editor::{Sidebar, SidebarRow};
use strop_git::memory::ChangedFile;

use super::super::text;
use super::super::{ACCENT, MUTED, SELECT_BG, TEXT};

/// Blame gutter width: `sha˟7 author˟9 age˟3` + separators.
pub(crate) const BLAME_W: usize = 22;
/// Pane-divider color — the sidebar's rule matches it.
const RULE: Color = Color::Rgb(0x3a, 0x3d, 0x4d);
/// Younger than this counts as "recent" → accent (0011 §3).
const RECENT_SECS: i64 = 30 * 86400;
/// The blame cell for one buffer line: `sha7 author9 age3`, muted for
/// old commits, accent for recent ones and uncommitted lines
/// (`0000000 you now`). Author clipping and padding are display-cell
/// work, so a wide name cannot change the column's width.
pub(crate) fn blame_spans(line: &strop_git::memory::BlameLine, now: i64) -> Span<'static> {
    let uncommitted = line.is_uncommitted();
    let recent = line.ts > 0 && now.saturating_sub(line.ts) < RECENT_SECS;
    let fg = if uncommitted || recent { ACCENT } else { MUTED };
    let sha: String = if uncommitted {
        "0".repeat(7)
    } else {
        line.sha.chars().take(7).collect()
    };
    let author = text::clip_end(&line.author, 9);
    let author_pad = " ".repeat(9usize.saturating_sub(text::width(&author)));
    let age: String = line.age.chars().take(3).collect();
    Span::styled(
        format!("{sha} {author}{author_pad} {age:>3} "),
        Style::default().fg(fg),
    )
}

/// A blank blame cell (filler rows past the buffer end).
pub(crate) fn blame_blank() -> Span<'static> {
    Span::styled(" ".repeat(BLAME_W), Style::default())
}

/// One sidebar row: directories dim, files indented by depth, the
/// current native path marked `▸` (sidebar focused) / `▌` (not) on
/// the selection background; the dividing rule closes the column.
/// Rows past the tree stay blank so the column reads as one
/// surface. Emission is exactly `outer_width` cells: labels clip
/// at grapheme/cell boundaries, then pad in cells.
pub(crate) fn sidebar_row_spans(
    sidebar: &Sidebar,
    files: &[ChangedFile],
    current: &Path,
    row: usize,
    focused: bool,
) -> Vec<Span<'static>> {
    let w = sidebar.width();
    let mut spans = match sidebar.rows().get(row) {
        Some(SidebarRow::Dir { name, depth }) => {
            let label = format!("{}{}/", " ".repeat(2 * depth), name);
            let shown = text::clip_end(&label, w.saturating_sub(1));
            let pad = (w - 1).saturating_sub(text::width(&shown));
            vec![
                Span::styled(format!(" {shown}"), Style::default().fg(MUTED)),
                Span::raw(" ".repeat(pad)),
            ]
        }
        Some(SidebarRow::File { index, name, depth }) => {
            let is_current = files[*index].path == current;
            let marker = match (is_current, focused) {
                (true, true) => "▸",
                (true, false) => "▌",
                (false, _) => " ",
            };
            let label = format!("{}{name}", " ".repeat(2 * depth));
            let shown = text::clip_end(&label, w.saturating_sub(2));
            let pad = (w - 1).saturating_sub(text::width(&shown));
            let style = if is_current {
                Style::default()
                    .fg(ACCENT)
                    .bg(SELECT_BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TEXT)
            };
            let fill = if is_current {
                Style::default().bg(SELECT_BG)
            } else {
                Style::default()
            };
            vec![
                Span::styled(format!("{marker}{shown}"), style),
                Span::styled(" ".repeat(pad), fill),
            ]
        }
        None => vec![Span::styled(" ".repeat(w), Style::default())],
    };
    spans.push(Span::styled(
        "│",
        Style::default().fg(if focused { ACCENT } else { RULE }),
    ));
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            added: 1,
            deleted: 0,
        }
    }

    #[test]
    fn current_file_is_native_path_identity() {
        let files = vec![file("a.rs"), file("b.rs")];
        let sidebar = Sidebar::build(&files);
        // focused: the current file points at you
        let b = sidebar_row_spans(&sidebar, &files, Path::new("b.rs"), 1, true);
        assert!(b.iter().any(|s| s.content.starts_with('▸')));
        // unfocused: the same file keeps a quiet marker
        let b_quiet = sidebar_row_spans(&sidebar, &files, Path::new("b.rs"), 1, false);
        assert!(b_quiet.iter().any(|s| s.content.starts_with('▌')));
        // a different row never borrows the marker
        let a = sidebar_row_spans(&sidebar, &files, Path::new("b.rs"), 0, true);
        assert!(a.iter().all(|s| !s.content.starts_with('▸')));
        // rows past the tree stay blank (plus the rule)
        let blank = sidebar_row_spans(&sidebar, &files, Path::new("b.rs"), 5, true);
        assert!(blank
            .iter()
            .all(|s| s.content == "│" || s.content.chars().all(|c| c == ' ')));
    }

    #[test]
    fn every_row_fits_the_reserved_width_in_cells() {
        // 12 CJK glyphs = 24 display cells: the widest possible label,
        // clamping the interior at the maximum
        let files = vec![ChangedFile {
            path: "漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字.rs".into(),
            added: 1,
            deleted: 0,
        }];
        let sidebar = Sidebar::build(&files);
        for row in 0..sidebar.rows().len() + 3 {
            let spans = sidebar_row_spans(&sidebar, &files, Path::new("none.rs"), row, false);
            let cells: usize = spans.iter().map(|s| text::width(&s.content)).sum();
            assert_eq!(cells, sidebar.outer_width(), "row {row}");
        }
        // a wide name clips at a grapheme boundary with an ellipsis
        let file_row = sidebar_row_spans(&sidebar, &files, Path::new("none.rs"), 0, false);
        let label: String = file_row
            .iter()
            .flat_map(|s| s.content.chars())
            .skip(1) // marker
            .collect();
        assert!(label.contains('…'));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_distinct_identity() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let a: std::path::PathBuf = OsString::from_vec(b"a/\xff.rs".to_vec()).into();
        let b: std::path::PathBuf = OsString::from_vec(b"a/\xfe.rs".to_vec()).into();
        let files = vec![
            ChangedFile {
                path: a.clone(),
                added: 1,
                deleted: 0,
            },
            ChangedFile {
                path: b.clone(),
                added: 1,
                deleted: 0,
            },
        ];
        let sidebar = Sidebar::build(&files);
        // both lossy labels are "a/\u{fffd}.rs" — two distinct rows anyway
        assert_eq!(sidebar.rows().len(), 3);
        // native-byte order: \xfe sorts before \xff
        let marks_current = |row: usize, current: &Path| {
            sidebar_row_spans(&sidebar, &files, current, row, true)
                .iter()
                .any(|s| s.content.starts_with('▸'))
        };
        assert!(marks_current(2, &a) && !marks_current(1, &a));
        assert!(marks_current(1, &b) && !marks_current(2, &b));
    }
}
