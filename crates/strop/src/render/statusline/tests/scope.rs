//! The breadcrumb on the row (0107 §10.3): its cut, its inks, and that
//! moving the cursor or typing a key moves no other cell.

use strop_git::GitContext;

use super::super::scope::fit;
use super::super::Modeline;
use super::{editor, grid, row};
use crate::editor::Editor;
use crate::render::clip::width;
use crate::render::tokens;

/// A small Rust file: an impl with two methods, a free function, and a
/// method three levels deep (`tests › impl Fixture ›
/// a_long_test_function`, 43 cells).
const RUST: &str = "\
//! The modeline's width plan.
pub(super) struct Modeline {
    chip: String,
}

impl Modeline {
    pub(super) fn fit(&self, cells: usize) -> usize {
        let sizes = self.sizes();
        cells.min(sizes)
    }

    fn hints_shown(&self) -> usize {
        0
    }
}

pub(super) fn allot(sizes: usize, cells: usize) -> usize {
    sizes.min(cells)
}

mod tests {
    struct Fixture;

    impl Fixture {
        fn a_long_test_function() {
            let value = 1;
        }
    }
}
";

/// Lines of [`RUST`] where the cursor is in `fit`, in `allot`, at top
/// level, and deepest.
const IN_FIT: usize = 7;
const IN_ALLOT: usize = 17;
const TOP: usize = 4;
const DEEP: usize = 25;

fn outlined(branch: &str) -> Editor {
    let mut e = editor(
        RUST,
        Some("/w/crates/strop/src/render/statusline/layout.rs"),
    );
    e.fixture_set_git(Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: None,
        head_branch: Some(branch.into()),
        remotes: Vec::new(),
    }));
    e.wait_symbols();
    e
}

fn at_line(e: &mut Editor, line: usize) {
    e.feed_text(&format!("{}G", line + 1));
}

fn cells(row: &str) -> Vec<char> {
    row.chars().collect()
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn shown(names_: &[&str], cells: usize) -> String {
    let names = names(names_);
    let shown = fit(&names, cells);
    let mut text = String::new();
    if shown.elided {
        text.push_str("… › ");
    }
    let segments: Vec<&str> = shown
        .segments
        .iter()
        .map(|segment| segment.as_ref())
        .collect();
    text.push_str(&segments.join(" › "));
    assert_eq!(width(&text), shown.cells(), "{text:?}");
    text
}

#[test]
fn the_cut_drops_outer_segments_then_cuts_the_innermost() {
    let deep = ["mod tests", "impl Fixture", "a_long_test_name"];
    assert_eq!(width(&shown(&deep, 100)), 43);
    assert_eq!(
        shown(&deep, 43),
        "mod tests › impl Fixture › a_long_test_name"
    );
    assert_eq!(shown(&deep, 40), "… › impl Fixture › a_long_test_name");
    assert_eq!(shown(&deep, 20), "… › a_long_test_name");
    assert_eq!(shown(&deep, 10), "… › a_lon…");
    assert_eq!(shown(&["impl Modeline", "fit"], 10), "… › fit");
    assert_eq!(shown(&["allot"], 3), "al…");
    assert_eq!(shown(&[], 10), "");
    for cells in 0..60 {
        let text = shown(&deep, cells);
        assert!(width(&text) <= cells.max(1), "{text:?} in {cells}");
        // the innermost is the last to lose a cell: while it is cut,
        // nothing outer is on the row
        if !text.ends_with("a_long_test_name") {
            assert!(!text.contains("Fixture"), "{text:?} in {cells}");
        }
    }
    let long = "n".repeat(33);
    assert_eq!(
        shown(&[&long], 100),
        format!("{}…", "n".repeat(31)),
        "a 33-cell name is cut to 32"
    );
}

#[test]
fn the_innermost_is_secondary_the_outer_muted_the_separator_faint() {
    let mut e = outlined("main");
    at_line(&mut e, IN_FIT);
    let t = tokens();
    let grid = grid(&e, 120, 4);
    let text = cells(&row(&e, 120, 4));
    let column = |needle: &str| {
        let needle = cells(needle);
        text.windows(needle.len())
            .position(|window| window == needle.as_slice())
            .unwrap_or_else(|| panic!("{needle:?} on the row"))
    };
    let ink = |x: usize| grid[(x as u16, 3)].fg;
    assert_eq!(ink(column("impl Modeline")), t.muted);
    assert_eq!(ink(column("›")), t.faint);
    assert_eq!(ink(column("fit")), t.secondary);
}

/// Every column where two rows differ.
fn differing(a: &[char], b: &[char]) -> Vec<usize> {
    (0..a.len().min(b.len()))
        .filter(|&x| a[x] != b[x])
        .collect()
}

#[test]
fn moving_the_cursor_moves_nothing_outside_the_slot() {
    for columns in [48u16, 80, 120, 200] {
        let mut e = outlined("feature/modeline-design");
        let mut rows = Vec::new();
        for line in [IN_FIT, IN_ALLOT, TOP, DEEP] {
            at_line(&mut e, line);
            rows.push(cells(&row(&e, columns, 4)));
        }
        // the percentage and the position change with the line: they
        // are the row's last fields
        let tail = rows[0].iter().position(|ch| *ch == '%').unwrap() - 3;
        let mut moved: Vec<usize> = Vec::new();
        for pair in rows.windows(2) {
            moved.extend(
                differing(&pair[0], &pair[1])
                    .into_iter()
                    .filter(|&x| x < tail),
            );
        }
        if let (Some(first), Some(last)) = (moved.iter().min(), moved.iter().max()) {
            assert!(
                last - first < crate::editor::SCOPE_CAP,
                "{columns}: only the slot changes, {first}..={last}: {:?}",
                rows.iter()
                    .map(|row| row.iter().collect::<String>())
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn typed_keys_leave_the_breadcrumb_where_it_was() {
    let mut e = outlined("main");
    at_line(&mut e, IN_FIT);
    let start = |e: &Editor| {
        let text = cells(&row(e, 80, 4));
        let needle = cells("impl Modeline");
        text.windows(needle.len())
            .position(|window| window == needle.as_slice())
            .expect("the breadcrumb on the row")
    };
    let idle = start(&e);
    e.feed_text("d");
    assert_eq!(start(&e), idle);
    e.feed_text("2");
    assert_eq!(start(&e), idle);
}

#[test]
fn the_row_fits_every_width_with_a_43_cell_breadcrumb() {
    let mut e = outlined("feature/modeline-design");
    at_line(&mut e, DEEP);
    let t = tokens();
    let line = Modeline::gather(&e, &t);
    assert_eq!(
        line.scope
            .as_ref()
            .map(|(names, slot)| (names.len(), *slot)),
        Some((3, crate::editor::SCOPE_CAP))
    );
    for cells in 1..=200 {
        let spans = match line.fit(cells) {
            Some(grants) => line.spans(&t, &grants),
            None => {
                let (chip, chip_cells) = line.bare(cells);
                line.bare_spans(&t, &chip, chip_cells, cells)
            }
        };
        let used: usize = spans.iter().map(|span| width(&span.content)).sum();
        assert!(used <= cells, "{used} cells in a {cells}-cell row");
        if line.fit(cells).is_some() {
            assert_eq!(used, cells, "a fitted row fills {cells}");
        }
    }
}

#[test]
fn the_engine_measures_names_as_the_row_does() {
    for name in [
        "impl Modeline",
        "a_long_test_name",
        "日本語の名前",
        "e\u{301}clair",
        "tab\there",
        "",
    ] {
        assert_eq!(crate::editor::scope_cells(name), width(name), "{name:?}");
    }
}
