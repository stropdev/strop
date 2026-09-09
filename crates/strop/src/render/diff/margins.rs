//! Left-margin chrome (0011, 0032 §3): the blame column and the
//! commit file sidebar. Its immutable tree is built by the Git worker;
//! rendering borrows only visible rows. Native file indices preserve identity.

use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use strop_git::memory::ChangedFile;

use super::super::text;
use super::super::{ACCENT, MUTED, SELECT_BG, TEXT};

/// Blame gutter width: `sha˟7 author˟9 age˟3` + separators.
pub(crate) const BLAME_W: usize = 22;
/// Pane-divider color — the sidebar's rule matches it.
const RULE: Color = Color::Rgb(0x3a, 0x3d, 0x4d);
/// Younger than this counts as "recent" → accent (0011 §3).
const RECENT_SECS: i64 = 30 * 86400;
/// Sidebar interior clamps (0032 §3): a two-file commit shouldn't pay
/// a wide pane; a huge name clips instead of stealing one.
const SIDEBAR_MIN: usize = 12;
const SIDEBAR_MAX: usize = 24;

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

/// One sidebar row in tree form (zed-lite: always-expanded, directory
/// rows dim and shallow, files indented by depth).
#[derive(Debug)]
enum SidebarRow {
    Dir {
        name: String,
        depth: usize,
    },
    File {
        index: usize,
        name: String,
        depth: usize,
    },
}

/// The immutable commit tree; native filenames stay in its owning file list.
#[derive(Debug)]
pub(crate) struct Sidebar {
    rows: Vec<SidebarRow>,
    /// Interior width in display cells (excludes the dividing rule).
    width: usize,
}

impl Sidebar {
    /// Flat file list → sorted tree rows: a directory row appears the
    /// first time its component prefix shows up. Sorting and grouping
    /// walk native path components, so distinct non-UTF-8 paths never
    /// collapse into one lossy label.
    pub(crate) fn build(files: &[ChangedFile]) -> Sidebar {
        let mut order: Vec<usize> = (0..files.len()).collect();
        // byte order, not `Path`'s component order: `src/a.rs` must
        // stay ahead of `src/a/b.rs`, or the sibling file would render
        // beneath its directory's children
        order.sort_by(|&a, &b| {
            files[a]
                .path
                .as_os_str()
                .as_encoded_bytes()
                .cmp(files[b].path.as_os_str().as_encoded_bytes())
        });
        let mut rows: Vec<SidebarRow> = Vec::with_capacity(files.len() + 1);
        let mut previous_parent = Path::new("");
        for &index in &order {
            let path = &files[index].path;
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            let shared = parent
                .components()
                .zip(previous_parent.components())
                .take_while(|(left, right)| left == right)
                .count();
            for (depth, component) in parent.components().enumerate().skip(shared) {
                rows.push(SidebarRow::Dir {
                    name: printable(component.as_os_str().to_string_lossy()),
                    depth,
                });
            }
            rows.push(SidebarRow::File {
                index,
                name: path
                    .file_name()
                    .map_or_else(String::new, |name| printable(name.to_string_lossy())),
                depth: parent.components().count(),
            });
            previous_parent = parent;
        }
        Sidebar {
            width: Self::measured_width(files) - 1,
            rows,
        }
    }

    /// Used only while preparing the immutable tree, never during painting.
    pub(crate) fn measured_width(files: &[ChangedFile]) -> usize {
        let longest = files
            .iter()
            .flat_map(|file| file.path.components().enumerate())
            .map(|(depth, component)| {
                1 + 2 * depth + text::width(&component.as_os_str().to_string_lossy())
            })
            .max()
            .unwrap_or(0);
        (longest + 2).clamp(SIDEBAR_MIN, SIDEBAR_MAX) + 1
    }

    /// Interior + the dividing rule — the exact cell count the pane's
    /// `left_inset` (and through it, the caret geometry) reserves.
    pub(crate) fn outer_width(&self) -> usize {
        self.width + 1
    }

    /// One sidebar row: directories dim, files indented by depth, the
    /// current native path marked `▸` (sidebar focused) / `▌` (not) on
    /// the selection background; the dividing rule closes the column.
    /// Rows past the tree stay blank so the column reads as one
    /// surface. Emission is exactly `outer_width` cells: labels clip
    /// at grapheme/cell boundaries, then pad in cells.
    pub(crate) fn row_spans(
        &self,
        files: &[ChangedFile],
        current: &Path,
        row: usize,
        focused: bool,
    ) -> Vec<Span<'static>> {
        let w = self.width;
        let mut spans = match self.rows.get(row) {
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
}

/// One emission-side label: the printable form of a (possibly lossy)
/// component name — the same policy the buffer's path rows use, so a
/// newline/tab in a native name becomes a one-cell replacement here
/// and in the width math alike, never a raw byte or a tab stop.
fn printable(name: std::borrow::Cow<'_, str>) -> String {
    strop_core::layout::printable_text(name).into_owned()
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
    fn tree_groups_by_directory_with_native_indices() {
        let files = vec![
            file("src/api/handlers.rs"),
            file("README.md"),
            file("src/main.rs"),
            file("src/api/mod.rs"),
        ];
        let sidebar = Sidebar::build(&files);
        let shape: Vec<String> = sidebar
            .rows
            .iter()
            .map(|r| match r {
                SidebarRow::Dir { name, depth } => format!("{}{}/", "  ".repeat(*depth), name),
                SidebarRow::File { index, name, depth } => {
                    format!("{}{}#{}", "  ".repeat(*depth), name, index)
                }
            })
            .collect();
        assert_eq!(
            shape,
            [
                "README.md#1", // root file, no dir row
                "src/",
                "  api/",
                "    handlers.rs#0",
                "    mod.rs#3",
                "  main.rs#2",
            ]
        );
    }

    #[test]
    fn current_file_is_native_path_identity() {
        let files = vec![file("a.rs"), file("b.rs")];
        let sidebar = Sidebar::build(&files);
        // focused: the current file points at you
        let b = sidebar.row_spans(&files, Path::new("b.rs"), 1, true);
        assert!(b.iter().any(|s| s.content.starts_with('▸')));
        // unfocused: the same file keeps a quiet marker
        let b_quiet = sidebar.row_spans(&files, Path::new("b.rs"), 1, false);
        assert!(b_quiet.iter().any(|s| s.content.starts_with('▌')));
        // a different row never borrows the marker
        let a = sidebar.row_spans(&files, Path::new("b.rs"), 0, true);
        assert!(a.iter().all(|s| !s.content.starts_with('▸')));
        // rows past the tree stay blank (plus the rule)
        let blank = sidebar.row_spans(&files, Path::new("b.rs"), 5, true);
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
        assert_eq!(sidebar.width, SIDEBAR_MAX);
        for row in 0..sidebar.rows.len() + 3 {
            let spans = sidebar.row_spans(&files, Path::new("none.rs"), row, false);
            let cells: usize = spans.iter().map(|s| text::width(&s.content)).sum();
            assert_eq!(cells, sidebar.outer_width(), "row {row}");
        }
        // a wide name clips at a grapheme boundary with an ellipsis
        let file_row = sidebar.row_spans(&files, Path::new("none.rs"), 0, false);
        let label: String = file_row
            .iter()
            .flat_map(|s| s.content.chars())
            .skip(1) // marker
            .collect();
        assert!(label.contains('…'));
    }

    #[test]
    fn narrow_commits_get_the_minimum_column() {
        let files = [file("ab.rs")];
        let sidebar = Sidebar::build(&files);
        assert_eq!(sidebar.outer_width(), SIDEBAR_MIN + 1);
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
        assert_eq!(sidebar.rows.len(), 3);
        // native-byte order: \xfe sorts before \xff
        let marks_current = |row: usize, current: &Path| {
            sidebar
                .row_spans(&files, current, row, true)
                .iter()
                .any(|s| s.content.starts_with('▸'))
        };
        assert!(marks_current(2, &a) && !marks_current(1, &a));
        assert!(marks_current(1, &b) && !marks_current(2, &b));
    }
}
