use super::{picker_split, CellRect, PreparedPicker, ViewGeometry};
use crate::editor::Editor;
use strop_core::Buffer;
use strop_picker::{Item, Kind, Payload};

fn prepare(editor: &mut Editor, columns: u16, rows: u16) -> PreparedPicker {
    editor.prepare_view(ViewGeometry { columns, rows });
    editor.prepared_view().picker.expect("prepared picker")
}

fn append(editor: &mut Editor, labels: impl IntoIterator<Item = String>) {
    let document = editor.current();
    editor.picker_items_fixture(
        labels
            .into_iter()
            .map(|text| Item {
                text,
                payload: Payload::Buffer(document),
                badge: None,
            })
            .collect(),
    );
}

fn picker(kind: Kind, labels: &[&str]) -> Editor {
    let mut editor = Editor::new(Buffer::from_text("selected source identity\n"));
    editor.set_picker(crate::editor::picker::PickerGlue::diagnostics(
        strop_picker::Picker::new(kind, Vec::new(), false),
    ));
    append(&mut editor, labels.iter().map(|label| (*label).to_string()));
    editor
}

fn split(editor: &mut Editor, columns: u16, rows: u16) -> (CellRect, CellRect) {
    let prepared = prepare(editor, columns, rows);
    (
        prepared.results.expect("results"),
        prepared.preview.expect("preview"),
    )
}

#[test]
fn short_labels_reclaim_list_width_but_long_paths_keep_their_budget() {
    let mut editor = picker(Kind::Files, &["sample.rs"]);
    let (short_list, short_preview) = split(&mut editor, 100, 30);
    assert!(
        short_preview.width > short_list.width,
        "short list {short_list:?}, preview {short_preview:?}"
    );
    assert_eq!(short_list.width, 28);

    editor.picker.as_mut().unwrap().picker.clear_items();
    append(
        &mut editor,
        ["very/long/disambiguating/path/to/a/file/with/a/meaningful/basename.rs".to_string()],
    );
    let (long_list, long_preview) = split(&mut editor, 100, 30);
    let (historical, _) = picker_split(CellRect::new(0, 0, 100, 30), Kind::Files, false).unwrap();
    assert_eq!(long_list, historical);
    assert!(long_preview.width > 24);
}

#[test]
fn streaming_selection_and_repeated_resizes_keep_one_committed_policy() {
    let mut editor = picker(Kind::Files, &["sample.rs", "second.rs"]);
    let initial = split(&mut editor, 100, 30);
    editor.feed(strop_core::frontend_input::Key::Down);
    assert_eq!(editor.picker().unwrap().picker.selected, 1);
    assert_eq!(split(&mut editor, 100, 30), initial);

    append(
        &mut editor,
        ["other.rs".to_string(), "third.rs".to_string()],
    );
    assert_eq!(
        split(&mut editor, 100, 30),
        initial,
        "short streaming batches do not oscillate the columns"
    );

    for columns in [120, 80, 100, 120, 100] {
        let (list, preview) = split(&mut editor, columns, 30);
        // card margin (2 or 0) + border + padding on each side
        let chrome = if columns >= 100 { 8 } else { 4 };
        assert_eq!(list.width + preview.width, columns - chrome);
        assert_eq!(list.x + list.width, preview.x);
        if columns == 100 {
            assert_eq!((list, preview), initial);
        }
    }

    let stacked = prepare(&mut editor, 60, 30);
    let list = stacked.results.unwrap();
    let preview = stacked.preview.unwrap();
    assert_eq!(preview.height, 8);
    assert_eq!(preview.y, list.y + list.height);
    assert!(prepare(&mut editor, 60, 10).preview.is_none());
    assert!(prepare(&mut editor, 100, 5).results.is_none());
    assert_eq!(split(&mut editor, 100, 30), initial);
}

#[test]
fn measurement_stops_at_the_admitted_metadata_bound() {
    let mut editor = picker(Kind::Files, &[]);
    append(
        &mut editor,
        (0..256).map(|index| format!("short-{index}.rs")),
    );
    let bounded = split(&mut editor, 100, 30);
    append(
        &mut editor,
        [
            "a/pathological/late/path/that/is/outside/the/bounded/measurement/sample.rs"
                .to_string(),
        ],
    );
    assert_eq!(split(&mut editor, 100, 30), bounded);
}

#[test]
fn symbol_status_rows_share_the_bounded_allocation() {
    let mut editor = picker(Kind::WorkspaceSymbols, &["dispatch  · :1"]);
    let before = split(&mut editor, 100, 30).0;
    editor
        .picker
        .as_mut()
        .unwrap()
        .picker
        .set_pinned(vec![Item {
            text: "workspace-with-a-long-name  path/to/project · indexing declarations".into(),
            payload: Payload::ProjectStatus(std::path::PathBuf::from("/project")),
            badge: None,
        }]);
    editor.picker_items_fixture(Vec::new());
    let after = split(&mut editor, 100, 30).0;
    assert!(
        after.width > before.width,
        "before {before:?}, after {after:?}"
    );
    assert_eq!(
        after.width, 51,
        "status text remains bounded by the list cap"
    );
}

#[test]
fn empty_error_and_replacement_states_preserve_usable_geometry() {
    let mut empty = Editor::new(Buffer::from_text(""));
    empty.open_picker(Kind::Files);
    let initial = prepare(&mut empty, 100, 30);
    assert!(initial.results.is_some());
    assert!(initial.preview.is_some());
    empty.picker.as_mut().unwrap().picker.streaming = false;
    empty.picker.as_mut().unwrap().picker.error = Some("source failed".into());
    assert_eq!(prepare(&mut empty, 100, 30).results, initial.results);

    let mut search = picker(Kind::Search, &["sample.rs:1 · selected source identity"]);
    let (list, preview) = split(&mut search, 100, 30);
    assert!(preview.width > list.width);
    search.picker.as_mut().unwrap().picker.toggle_replacement();
    let replaced = prepare(&mut search, 100, 30);
    assert_eq!(replaced.results.unwrap().width, list.width);
    assert_eq!(replaced.preview.unwrap().x, preview.x);
    assert_eq!(replaced.results.unwrap().height + 1, list.height);
}
