//! 0066: syntax colors survive edits and scrolls on interim frames.
//! The worker completion is held in the channel — tests never pump it
//! until `wait_analysis` — so the draw right after an edit renders
//! exactly the edit-mapped interim frame a fast typist sees.
use crate::editor::Editor;
use ratatui::style::Color;
use strop_core::Buffer;

const SOURCE: &str = "fn main() {\n    let message = \"hello\";\n    let count = 42;\n    println!(\"{} {}\", message, count);\n}\n\nfn untouched(value: usize) -> usize {\n    // This function is not being edited.\n    let doubled = value * 2;\n    doubled + 1\n}\n";

fn editor_at(path: &std::path::Path) -> Editor {
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.open_fixture(path).unwrap();
    editor
}

type Grid = Vec<Vec<(String, Color)>>;

/// One draw through the real paint path, returning every cell's
/// (symbol, foreground).
fn draw(
    editor: &mut Editor,
    terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
) -> Grid {
    terminal
        .draw(|frame| crate::render::paint(editor, frame))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| {
                    let cell = &buffer[(x, y)];
                    (cell.symbol().to_string(), cell.fg)
                })
                .collect()
        })
        .collect()
}

fn text(row: &[(String, Color)]) -> String {
    row.iter().map(|(symbol, _)| symbol.as_str()).collect()
}

/// Cells past the gutter whose symbol did not move but whose foreground
/// changed — the 0066 flash metric (98 cells on the reported build).
/// The gutter is skipped by content offset: the current-line number's
/// accent legitimately follows the cursor.
fn foreground_changes(
    before: &Grid,
    after: &Grid,
    rows: std::ops::Range<usize>,
    row_shift: isize,
) -> usize {
    let mut changed = 0;
    for y in rows {
        let at = (y as isize + row_shift).max(0) as usize;
        let Some(moved) = after.get(at) else {
            continue;
        };
        let gutter = content_start(&before[y]);
        for (x, (was, now)) in before[y].iter().zip(moved).enumerate() {
            if x >= gutter && was.0 == now.0 && !was.0.trim().is_empty() && was.1 != now.1 {
                changed += 1;
            }
        }
    }
    changed
}

fn content_start(row: &[(String, Color)]) -> usize {
    row.iter()
        .position(|(symbol, _)| {
            !symbol
                .chars()
                .all(|c| c.is_whitespace() || c.is_ascii_digit() || c == '▎')
        })
        .unwrap_or(row.len())
}
fn keyword_color() -> Color {
    crate::render::class_color(strop_syntax::Class::Keyword)
}

fn keyword_fg_at(row: &[(String, Color)], needle: &str) -> Color {
    let at = text(row)
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} not in row"));
    row[at].1
}

#[test]
fn typing_one_space_does_not_flash_unrelated_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typing.rs");
    std::fs::write(&path, SOURCE).unwrap();
    let mut editor = editor_at(&path);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();

    // warm the exact analysis, then hold every later completion
    let _ = draw(&mut editor, &mut terminal);
    editor.wait_analysis();
    let warm = draw(&mut editor, &mut terminal);
    let keyword = keyword_color();
    let untouched = warm
        .iter()
        .find(|row| text(row).contains("fn untouched"))
        .unwrap();
    assert_eq!(
        keyword_fg_at(untouched, "fn"),
        keyword,
        "warm keyword color"
    );

    // one space inserted at byte zero; the fresh analysis is in flight
    editor.feed_text("gg0i <esc>");
    let interim = draw(&mut editor, &mut terminal);
    assert_eq!(
        foreground_changes(&warm, &interim, 1..warm.len(), 0),
        0,
        "the interim frame keeps every unrelated cell's color"
    );
    let untouched = interim
        .iter()
        .find(|row| text(row).contains("fn untouched"))
        .unwrap();
    assert_eq!(
        keyword_fg_at(untouched, "fn"),
        keyword,
        "the untouched function stays keyword-colored"
    );
    let shifted = keyword_fg_at(&interim[0], "fn");
    assert_eq!(shifted, keyword, "the edited line's `fn` keeps its color");

    // the exact result lands and agrees
    editor.wait_analysis();
    let settled = draw(&mut editor, &mut terminal);
    assert_eq!(
        foreground_changes(&warm, &settled, 1..warm.len(), 0),
        0,
        "the settled frame reintroduces no changes"
    );
}

#[test]
fn scrolling_keeps_the_covered_overlap_colored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scroll.rs");
    let mut source = String::new();
    for index in 0..8 {
        source.push_str(&format!(
            "fn function_{index}(value: usize) -> usize {{\n    let doubled = value * 2;\n    doubled + {index}\n}}\n\n"
        ));
    }
    std::fs::write(&path, source).unwrap();
    let mut editor = editor_at(&path);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 10)).unwrap();

    let _ = draw(&mut editor, &mut terminal);
    editor.wait_analysis();
    let warm = draw(&mut editor, &mut terminal);

    // move the cursor past the bottom of the view: the viewport window
    // changes without an edit, so the fresh analysis is keyed to a new
    // window while the pane still shows mostly-covered lines
    editor.feed_text("10j");
    let interim = draw(&mut editor, &mut terminal);
    let shared = shared_line_numbers(&warm, &interim);
    assert!(shared.len() >= 3, "the view actually scrolled: {shared:?}");
    assert_eq!(
        line_color_changes(&warm, &interim, &shared),
        0,
        "the covered overlap keeps its colors while the new window is in flight"
    );

    editor.wait_analysis();
    let settled = draw(&mut editor, &mut terminal);
    assert_eq!(
        line_color_changes(&warm, &settled, &shared),
        0,
        "the settled scroll reintroduces no changes"
    );
}

/// Gutter line numbers visible in both grids, with their row indexes.
fn shared_line_numbers(before: &Grid, after: &Grid) -> Vec<(String, usize, usize)> {
    let index = |grid: &Grid| {
        grid.iter()
            .enumerate()
            .filter_map(|(y, row)| line_number(row).map(|number| (number, y)))
            .collect::<Vec<_>>()
    };
    let after_index = index(after);
    index(before)
        .into_iter()
        .filter_map(|(number, y)| {
            after_index
                .iter()
                .find(|(other, _)| *other == number)
                .map(|(_, at)| (number, y, *at))
        })
        .collect()
}

fn line_number(row: &[(String, Color)]) -> Option<String> {
    let joined = text(row);
    let head = joined.trim_start();
    let digits: String = head.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// Cells of shared lines whose symbol did not move but whose foreground
/// changed — gutter excluded by content offset, border column by width.
fn line_color_changes(before: &Grid, after: &Grid, shared: &[(String, usize, usize)]) -> usize {
    let mut changed = 0;
    for (_, was_row, now_row) in shared {
        let (was, now) = (&before[*was_row], &after[*now_row]);
        let gutter = content_start(was);
        for (x, (left, right)) in was.iter().zip(now).enumerate() {
            if x >= gutter
                && x + 1 < was.len()
                && left.0 == right.0
                && !left.0.trim().is_empty()
                && left.1 != right.1
            {
                changed += 1;
            }
        }
    }
    changed
}
