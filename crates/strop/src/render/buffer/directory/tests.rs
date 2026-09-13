use super::*;
use crate::editor::{Document, Editor};
use ratatui::{backend::TestBackend, buffer::Buffer as Grid, Terminal};
use strop_core::Buffer;
use strop_workspace::{
    DirectoryEntry, DirectorySnapshot, EntryName, Observation, Permissions, ResourceLocation,
};

fn entry(name: &str, kind: EntryKind, size: Option<u64>) -> DirectoryEntry {
    let mut observation = Observation::unknown(kind);
    observation.size = size;
    observation.permissions = Some(
        Permissions::new(if kind == EntryKind::Directory {
            0o755
        } else {
            0o644
        })
        .unwrap(),
    );
    DirectoryEntry {
        name: EntryName::new(name.into()).unwrap(),
        observation,
        error: None,
    }
}
fn folder(entries: Vec<DirectoryEntry>, state: ListingState) -> Editor {
    let directory = Directory::new(DirectorySnapshot {
        location: ResourceLocation::local("/workspace/project".into()),
        entries: entries.into(),
        state,
    });
    let mut editor = strop_engine::editor::test_support::remote::snapshot_editor("");
    let document = Document::directory(Buffer::from_text(&directory.text()), directory);
    let id = editor.docs.insert(document);
    editor.switch_to(id);
    editor.set_head(
        editor
            .buf()
            .line_start(2.min(editor.buf().last_content_line())),
    );
    editor
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
fn folder_rows_align_metadata_and_keep_selection_and_marks_visible() {
    let mut editor = folder(
        vec![
            entry("src", EntryKind::Directory, Some(4096)),
            entry("README.md", EntryKind::File, Some(8192)),
            entry("zero.txt", EntryKind::File, Some(0)),
        ],
        ListingState::Complete,
    );
    let grid = frame(&mut editor, 80, 9);
    assert_eq!(
        row_text(&grid, 2),
        "▸  3 src/                         4.0 KiB  drwxr-xr-x"
    );
    assert_eq!(
        row_text(&grid, 3),
        "   4 README.md                    8.0 KiB  -rw-r--r--"
    );
    assert_eq!(
        row_text(&grid, 4),
        "   5 zero.txt                         0 B  -rw-r--r--"
    );
    assert!((0..80).all(|x| grid[(x, 2)].bg == diff::CURSOR_ROW_BG));
    assert_eq!(grid[(5, 2)].fg, ACCENT);
    assert_eq!(grid[(5, 3)].fg, TEXT);
    assert_eq!(grid[(34, 3)].fg, MUTED);
    assert_eq!(grid[(5, 0)].fg, MUTED);
    assert!(grid[(16, 0)].modifier.contains(Modifier::BOLD));
    editor.feed_text(":fs mark\rj");
    let marked = frame(&mut editor, 80, 9);
    assert_eq!(marked[(0, 2)].symbol(), "●");
    assert_eq!(marked[(0, 3)].symbol(), "▸");
    assert_eq!(marked[(79, 2)].bg, diff::CURSOR_ROW_BG);
    assert_eq!(marked[(79, 3)].bg, diff::CURSOR_ROW_BG);
}

#[test]
fn narrow_round_trip_keeps_native_names_and_cursor_mapping() {
    let mut editor = folder(
        vec![
            entry("日本語.txt", EntryKind::Directory, Some(64)),
            entry("two  spaces.txt", EntryKind::File, None),
        ],
        ListingState::Complete,
    );
    let expected = editor.buf().snapshot();
    let grid = frame(&mut editor, 24, 7);
    assert_eq!(grid[(5, 2)].symbol(), "日");
    assert_eq!(grid[(7, 2)].symbol(), "本");
    assert_eq!(grid[(9, 2)].symbol(), "語");
    frame(&mut editor, 2, 3);
    let restored = frame(&mut editor, 80, 9);
    assert_eq!(editor.buf().snapshot(), expected);
    let selected_row = (0..restored.area.height)
        .find(|&row| restored[(0, row)].symbol() == "▸")
        .unwrap();
    assert_eq!(restored[(5, selected_row)].symbol(), "日");
    assert_eq!(editor.head(), editor.buf().line_start(2));
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
fn empty_folder_is_not_rendered_as_missing_file_rows() {
    let mut editor = folder(Vec::new(), ListingState::Complete);
    let grid = frame(&mut editor, 60, 8);
    assert_eq!(row_text(&grid, 2), "     Empty folder");
    assert!((3..7).all(|row| row_text(&grid, row).is_empty()));
}
