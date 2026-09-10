//! The immutable commit-file sidebar tree (0032 §3): built once by the
//! Git worker alongside the file list; the binary's renderer borrows
//! visible rows. Native file indices preserve identity.
use std::path::Path;

use strop_core::layout::printable_grapheme;
use strop_git::memory::ChangedFile;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Sidebar interior clamps (0032 §3): a two-file commit shouldn't pay
/// a wide pane; a huge name clips instead of stealing one.
const SIDEBAR_MIN: usize = 12;
const SIDEBAR_MAX: usize = 24;

/// One sidebar row in tree form (zed-lite: always-expanded, directory
/// rows dim and shallow, files indented by depth).
#[derive(Debug)]
pub enum SidebarRow {
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
pub struct Sidebar {
    rows: Vec<SidebarRow>,
    /// Interior width in display cells (excludes the dividing rule).
    width: usize,
}

impl Sidebar {
    /// Flat file list → sorted tree rows: a directory row appears the
    /// first time its component prefix shows up. Sorting and grouping
    /// walk native path components, so distinct non-UTF-8 paths never
    /// collapse into one lossy label.
    pub fn build(files: &[ChangedFile]) -> Sidebar {
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
    pub fn measured_width(files: &[ChangedFile]) -> usize {
        let longest = files
            .iter()
            .flat_map(|file| file.path.components().enumerate())
            .map(|(depth, component)| {
                1 + 2 * depth + width(&component.as_os_str().to_string_lossy())
            })
            .max()
            .unwrap_or(0);
        (longest + 2).clamp(SIDEBAR_MIN, SIDEBAR_MAX) + 1
    }

    /// Interior + the dividing rule — the exact cell count the pane's
    /// `left_inset` (and through it, the caret geometry) reserves.
    pub fn outer_width(&self) -> usize {
        self.width + 1
    }

    /// Interior width in display cells — the renderer's padding math.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The tree rows in paint order.
    pub fn rows(&self) -> &[SidebarRow] {
        &self.rows
    }
}

/// One cluster's display cells: the printable form's terminal width — the
/// same measure the renderer's `Span::width` applies at emission.
fn grapheme_width(grapheme: &str) -> usize {
    UnicodeWidthStr::width(printable_grapheme(grapheme))
}

fn width(text: &str) -> usize {
    text.graphemes(true).map(grapheme_width).sum()
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
            .rows()
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
    fn narrow_commits_get_the_minimum_column() {
        let files = [file("ab.rs")];
        let sidebar = Sidebar::build(&files);
        assert_eq!(sidebar.outer_width(), SIDEBAR_MIN + 1);
    }

    #[test]
    fn wide_names_clamp_the_interior_at_the_maximum() {
        // 12 CJK glyphs = 24 display cells: the widest possible label,
        // clamping the interior at the maximum
        let files = vec![ChangedFile {
            path: "漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字.rs".into(),
            added: 1,
            deleted: 0,
        }];
        let sidebar = Sidebar::build(&files);
        assert_eq!(sidebar.width(), SIDEBAR_MAX);
    }
}
