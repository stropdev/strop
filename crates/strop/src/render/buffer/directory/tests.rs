use super::*;
use crate::editor::Editor;
use ratatui::{backend::TestBackend, buffer::Buffer as Grid, Terminal};
use strop_workspace::{DirectoryEntry, DirectorySnapshot, EntryName, FileTime, ResourceLocation};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

fn entry(
    name: &str,
    kind: EntryKind,
    size: Option<u64>,
    age: Option<i64>,
    now: i64,
) -> DirectoryEntry {
    let mut observation = Observation::unknown(kind);
    observation.size = size;
    observation.modified = age.map(|age| FileTime {
        seconds: now - age,
        nanos: 0,
    });
    DirectoryEntry {
        name: EntryName::new(name.into()).unwrap(),
        observation,
        error: None,
    }
}
/// A folder buffer the way opening one makes it: editable, its names
/// the text, the cursor on the first row.
fn folder(entries: impl FnOnce(i64) -> Vec<DirectoryEntry>, state: ListingState) -> Editor {
    let mut editor = strop_engine::editor::test_support::remote::snapshot_editor("");
    let now = editor.tape().now().unix_seconds;
    let directory = Directory::new(DirectorySnapshot {
        location: ResourceLocation::local("/workspace/project".into()),
        entries: entries(now).into(),
        state,
    });
    let document = directory
        .into_document(strop_core::worker::WorkerId::new(1))
        .unwrap();
    let id = editor.admit_document(document).unwrap();
    editor.switch_to(id);
    editor.set_head(0);
    editor
}
fn project() -> Editor {
    folder(
        |now| {
            vec![
                entry("src", EntryKind::Directory, Some(4096), Some(DAY), now),
                entry("README.md", EntryKind::File, Some(8704), Some(3 * DAY), now),
                entry("zero.txt", EntryKind::File, Some(0), Some(2 * HOUR), now),
            ]
        },
        ListingState::Complete,
    )
}
fn frame(editor: &mut Editor, width: u16, height: u16) -> Grid {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::render::frame_capture::draw(editor, frame, false))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn row_text(grid: &Grid, y: u16) -> String {
    (0..grid.area.width)
        .map(|x| grid[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn rows_are_names_with_size_and_age_in_one_column() {
    let mut editor = project();
    assert_eq!(editor.buf().text(), "src/\nREADME.md\nzero.txt\n");
    let grid = frame(&mut editor, 80, 7);
    assert_eq!(
        row_text(&grid, 0),
        "▌  1 src/                              1d"
    );
    assert_eq!(
        row_text(&grid, 1),
        "   2 README.md                  8.5K   3d"
    );
    assert_eq!(
        row_text(&grid, 2),
        "   3 zero.txt                      0   2h"
    );
    let t = crate::render::tokens();
    // the cursor-row band spans the pane, minus the reserved track column
    assert!((0..79).all(|x| grid[(x, 0)].bg == t.cursorline));
    assert_eq!(grid[(5, 0)].fg, t.accent, "a folder is the accent");
    assert!(grid[(5, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(grid[(5, 1)].fg, t.text);
    assert_eq!(grid[(33, 1)].fg, t.muted, "metadata is quiet");
}

#[test]
fn an_edited_row_says_what_write_will_do_with_it() {
    let mut editor = project();
    editor.feed_text("jccREADME.txt<esc>yyponew.txt<esc>");
    let grid = frame(&mut editor, 80, 8);
    assert_eq!(
        row_text(&grid, 1),
        "   2 README.txt                was README.md"
    );
    assert_eq!(
        row_text(&grid, 2),
        "   3 README.txt                copy of README.md"
    );
    assert_eq!(row_text(&grid, 3), "▌  4 new.txt                   new");
    assert_eq!(
        row_text(&grid, 4),
        "   5 zero.txt                      0   2h"
    );
    assert_eq!(grid[(31, 1)].fg, crate::render::tokens().accent);
    let mut editor = project();
    editor.feed_text("Gyyp");
    let grid = frame(&mut editor, 80, 8);
    assert_eq!(
        row_text(&grid, 3),
        "▌  4 zero.txt                  copy of zero.txt"
    );
}

#[test]
fn a_name_past_the_column_keeps_its_name_and_drops_its_metadata() {
    let mut editor = folder(
        |now| {
            vec![
                entry(&"n".repeat(60), EntryKind::File, Some(1), Some(DAY), now),
                entry("short", EntryKind::File, Some(1), Some(DAY), now),
            ]
        },
        ListingState::Complete,
    );
    let grid = frame(&mut editor, 100, 5);
    assert_eq!(row_text(&grid, 0), format!("▌  1 {}", "n".repeat(60)));
    assert_eq!(
        row_text(&grid, 1),
        format!("   2 short{}1   1d", " ".repeat(49))
    );
}

#[test]
fn narrow_round_trip_keeps_native_names_and_cursor_mapping() {
    let mut editor = folder(
        |now| {
            vec![
                entry("日本語", EntryKind::Directory, Some(64), Some(DAY), now),
                entry("two  spaces.txt", EntryKind::File, None, None, now),
            ]
        },
        ListingState::Complete,
    );
    let expected = editor.buf().snapshot();
    let grid = frame(&mut editor, 24, 7);
    assert_eq!(grid[(5, 0)].symbol(), "日");
    assert_eq!(grid[(7, 0)].symbol(), "本");
    assert_eq!(grid[(9, 0)].symbol(), "語");
    frame(&mut editor, 2, 3);
    let restored = frame(&mut editor, 80, 9);
    assert_eq!(editor.buf().snapshot(), expected);
    assert_eq!(restored[(0, 0)].symbol(), "▌");
    assert_eq!(restored[(5, 0)].symbol(), "日");
    editor.feed_text("j");
    let source = editor
        .directory()
        .unwrap()
        .entry_location(LineIndex::new(editor.buf().line_of(editor.head())))
        .unwrap();
    assert_eq!(
        source.path,
        std::path::Path::new("/workspace/project/two  spaces.txt")
    );
}

#[test]
fn an_empty_folder_says_how_to_add_a_name() {
    let mut editor = folder(|_| Vec::new(), ListingState::Complete);
    let grid = frame(&mut editor, 60, 6);
    assert_eq!(
        row_text(&grid, 0),
        "▌  1 empty folder  ·  o adds a name, :w creates it"
    );
    assert!((1..5).all(|row| row_text(&grid, row)
        .chars()
        .all(|c| c.is_whitespace() || c == '\u{2502}')));
}

#[test]
fn sizes_and_ages_read_at_a_glance() {
    for (size, text) in [
        (0, "0"),
        (96, "96"),
        (999, "999"),
        (1024, "1.0K"),
        (8704, "8.5K"),
        (194_560, "190K"),
        (2_936_013, "2.8M"),
    ] {
        assert_eq!(human_size(size), text, "{size}");
    }
    for (seconds, text) in [
        (5, "now"),
        (300, "5m"),
        (5 * HOUR, "5h"),
        (3 * DAY, "3d"),
        (15 * DAY, "2w"),
        (400 * DAY, "1y"),
    ] {
        assert_eq!(age(seconds), text, "{seconds}");
    }
}
