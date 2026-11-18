//! The tab line (plans/0113 §8.4): what fits at every width, the inks,
//! and the row it takes from the panes.

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::Buffer;

use super::clip::width;
use super::tabline::{fit, runs, Tab};
use crate::editor::Editor;

fn tab(number: usize, name: &str, more: usize, unsaved: bool) -> Tab {
    Tab {
        number: number.to_string(),
        name: name.to_owned(),
        more,
        unsaved,
    }
}

/// The row as text, padded with the band to `columns` cells.
fn row(tabs: &[Tab], current: usize, columns: usize) -> String {
    let text: String = runs(tabs, current, columns)
        .into_iter()
        .map(|(text, _, _)| text)
        .collect();
    let pad = columns.saturating_sub(width(&text));
    format!("{text}{}", " ".repeat(pad))
}

fn six() -> Vec<Tab> {
    vec![
        tab(1, "layout.rs", 2, false),
        tab(2, "statusline.rs", 0, true),
        tab(3, "golden_modeline_tests.rs", 1, false),
        tab(4, "1:fish", 0, false),
        tab(5, "Cargo.toml", 0, false),
        tab(6, "breadcrumb.md", 0, false),
    ]
}

fn twenty() -> Vec<Tab> {
    (1..=20)
        .map(|k| tab(k, &format!("file_{k}.rs"), 0, k == 3))
        .collect()
}

/// §5.2's targets, cell for cell.
#[test]
fn the_rows_of_the_plan() {
    let three = [
        tab(1, "layout.rs", 1, true),
        tab(2, "statusline.rs", 0, true),
        tab(3, "[scratch]", 0, false),
    ];
    for (tabs, current, columns, expected) in [
        (
            &three[..],
            0,
            80,
            " 1 layout.rs +1 ●  2 statusline.rs ●  3 [scratch]                               ",
        ),
        (
            &six()[..],
            2,
            120,
            " 1 layout.rs +2  2 statusline.rs ●  3 golden_modeline_tests.rs +1  4 1:fish  5 Cargo.toml  6 breadcrumb.md              ",
        ),
        (
            &six()[..],
            2,
            80,
            " 1 layout.… +2  2 statusl… ●  3 golden_… +1  4 1:fish  5 Cargo.t…  6 breadcr…   ",
        ),
        (
            &six()[..],
            2,
            64,
            " 1 +2  2 ●  3 golden_modeline_tests.rs +1  4  5  6              ",
        ),
        (
            &six()[..],
            2,
            48,
            " 1 +2  2 ●  3 golden_modeline_test… +1  4  5  6 ",
        ),
        (
            &twenty()[..],
            0,
            48,
            " 1 file_…  2  3 ●  4  5  6  7  8  9  10  11 ›   ",
        ),
        (
            &twenty()[..],
            10,
            48,
            "‹ 7  8  9  10  11 file_…  12  13  14  15  16 ›  ",
        ),
        (
            &twenty()[..],
            19,
            48,
            "‹ 11  12  13  14  15  16  17  18  19  20 file_… ",
        ),
    ] {
        assert_eq!(row(tabs, current, columns), expected, "{columns} columns");
    }
}

/// Every width and every count: the current label is always there, whole
/// or cut, and the row never runs past the width unless that label alone
/// is wider (the paint clips it then).
#[test]
fn the_current_label_always_shows_and_the_row_fits() {
    for count in 1..=30 {
        let tabs: Vec<Tab> = (1..=count)
            .map(|k| tab(k, &"name".repeat(k % 9), k % 3, k % 4 == 0))
            .collect();
        for current in [0, count / 2, count - 1] {
            for columns in 1..=200 {
                let runs = runs(&tabs, current, columns);
                let here: String = runs
                    .iter()
                    .filter(|(_, _, here)| *here)
                    .map(|(text, _, _)| text.as_str())
                    .collect();
                assert!(
                    here.starts_with(&format!(" {} ", current + 1)),
                    "{count} tabs, current {current}, {columns} columns: {here:?}"
                );
                let total: usize = runs.iter().map(|(text, _, _)| width(text)).sum();
                let fit = fit(&tabs, current, columns);
                assert!(
                    total <= columns || (fit.first, fit.end) == (current, current + 1),
                    "{count} tabs, current {current}, {columns} columns: {total}"
                );
            }
        }
    }
}

fn paint(editor: &mut Editor, columns: u16, rows: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(columns, rows)).unwrap();
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The current label is the only thing on the row on the text
/// background; `●` is the warning ink; nothing has the accent.
#[test]
fn the_current_label_sits_on_the_text_background() {
    let t = crate::render::tokens();
    let mut editor = Editor::new(Buffer::from_text("one\n"));
    editor.feed_text(":tab split<cr>x:tabnew<cr>");
    let grid = paint(&mut editor, 60, 8);
    let row: String = (0..60).map(|x| grid[(x, 0)].symbol().to_owned()).collect();
    assert!(
        row.starts_with(" 1 [scratch] ●  2 [scratch] ●  3 [scratch] "),
        "{row:?}"
    );
    let current = row[..row.find(" 3 ").unwrap()].chars().count();
    for x in 0..60u16 {
        let cell = &grid[(x, 0)];
        let on_current = (current..current + 13).contains(&usize::from(x));
        assert_eq!(
            cell.bg,
            if on_current { t.base } else { t.surface },
            "cell {x}"
        );
        assert_ne!(cell.fg, t.accent, "cell {x}");
        if cell.symbol() == "●" {
            assert_eq!(cell.fg, t.warning, "cell {x}");
        }
    }
}

/// With two tab pages the panes start on row 1 and the modeline stays the
/// last row; with one, nothing moves.
#[test]
fn the_tab_line_takes_the_top_row_from_the_panes() {
    let mut editor = Editor::new(Buffer::from_text("one\ntwo\n"));
    paint(&mut editor, 40, 10);
    let pane = editor.prepared_view().panes[0].rect;
    assert_eq!((pane.y, pane.height), (0, 9));
    editor.feed_text(":vs<cr>:tab split<cr>");
    paint(&mut editor, 40, 10);
    let pane = editor.prepared_view().panes[0].rect;
    assert_eq!((pane.y, pane.height), (1, 8));
    editor.feed_text("gt");
    paint(&mut editor, 40, 10);
    let panes = &editor.prepared_view().panes;
    assert_eq!(panes.len(), 2);
    assert!(
        panes.iter().all(|pane| pane.rect.y == 2),
        "a title row under the tab line"
    );
    assert!(panes.iter().all(|pane| pane.rect.y + pane.rect.height == 9));
    editor.feed_text(":tabo<cr>");
    paint(&mut editor, 40, 10);
    assert!(editor
        .prepared_view()
        .panes
        .iter()
        .all(|pane| pane.rect.y == 1));
}
