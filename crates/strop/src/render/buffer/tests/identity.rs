use super::*;

fn draw(editor: &mut Editor, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = viewport_terminal(width, height);
    terminal.draw(|f| crate::render::paint(editor, f)).unwrap();
    terminal.backend().buffer().clone()
}

fn grid_text(grid: &ratatui::buffer::Buffer) -> String {
    (0..grid.area.height)
        .map(|y| row_symbols(grid, 0, y, grid.area.width).join(""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Clone, Copy, Debug)]
enum OpenRoute {
    Cli,
    Picker,
    Edit,
}

fn open_through(route: OpenRoute, root: &std::path::Path, path: &std::path::Path) -> Editor {
    let editor = match route {
        OpenRoute::Cli => Editor::new_in(Buffer::open(path).unwrap(), root.to_path_buf()),
        OpenRoute::Edit => {
            let mut editor = Editor::new_in(Buffer::from_text(""), root.to_path_buf());
            editor.feed_text(&format!(":e {}\r", path.display()));
            editor.wait_io().unwrap();
            editor
        }
        OpenRoute::Picker => {
            let mut editor = Editor::new_in(Buffer::from_text(""), root.to_path_buf());
            editor.open_picker(strop_picker::Kind::Files);
            editor.picker_items_fixture(vec![strop_picker::Item {
                badge: None,
                text: "src/sample.rs".into(),
                payload: strop_picker::Payload::File(path.to_path_buf()),
            }]);
            editor.feed(strop_core::frontend_input::Key::Enter);
            editor.wait_io().unwrap();
            editor
        }
    };
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(path),
        "route {route:?}, message: {}",
        editor.message()
    );
    editor
}

#[test]
fn cli_picker_and_edit_project_the_same_workspace_relative_title() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("src/sample.rs");
    std::fs::create_dir(source.parent().unwrap()).unwrap();
    std::fs::write(&source, "fn main() {}\n").unwrap();

    for route in [OpenRoute::Cli, OpenRoute::Picker, OpenRoute::Edit] {
        let mut editor = open_through(route, root.path(), &source);
        editor.split(true, None);
        let titles = row_symbols(&draw(&mut editor, 100, 6), 0, 0, 100).join("");
        assert_eq!(titles.matches("src/sample.rs").count(), 2, "{titles}");
        assert!(titles.contains("╭ 1 src/sample.rs"), "{titles}");
        assert!(titles.contains("╭ 2 src/sample.rs"), "{titles}");
        assert!(!titles.contains(root.path().to_string_lossy().as_ref()));
    }
}

#[test]
fn duplicate_outside_dirty_and_readonly_titles_keep_their_context() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let first_path = root.path().join("one/shared.rs");
    let second_path = root.path().join("two/shared.rs");
    let outside_path = outside.path().join("outside.rs");
    for path in [&first_path, &second_path] {
        std::fs::create_dir(path.parent().unwrap()).unwrap();
        std::fs::write(path, "\n").unwrap();
    }
    std::fs::write(&outside_path, "\n").unwrap();

    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    let first = editor.open_fixture(&first_path).unwrap();
    let second = editor.open_fixture(&second_path).unwrap();
    editor.open_fixture(&outside_path).unwrap();
    editor.split_document(true, first);
    editor.split_document(true, second);
    {
        let mut buffer = editor.fixture_buf_mut();
        buffer.dirty = true;
        buffer.readonly = true;
    }

    let titles = row_symbols(&draw(&mut editor, 180, 6), 0, 0, 180).join("");
    assert!(titles.contains("one/shared.rs"), "{titles}");
    assert!(titles.contains("╭ 2 one/shared.rs"), "{titles}");
    assert!(titles.contains("╭ 3 two/shared.rs"), "{titles}");
    assert!(titles.contains(outside_path.to_string_lossy().as_ref()));
    assert!(titles.contains('●'), "{titles}");
    assert!(titles.contains("[RO]"), "{titles}");

    let titles = row_symbols(&draw(&mut editor, 48, 16), 0, 0, 48).join("");
    assert!(
        titles.contains("╭ 1 /"),
        "outside authority remains absolute: {titles}"
    );
    assert!(
        titles.contains("one/shared.rs") || titles.contains("o/shared.rs"),
        "first duplicate keeps context: {titles}"
    );
    assert!(
        titles.contains("two/shared.rs") || titles.contains("t/shared.rs") || titles.contains("t/"),
        "second duplicate keeps distinct context: {titles}"
    );
}

#[test]
fn remote_pane_identity_is_document_scoped_and_keeps_authority_when_narrow() {
    let mut editor = strop_engine::editor::test_support::remote::snapshot_editor("remote\n");
    let scratch = editor.first_doc();
    editor.split_document(true, scratch);

    let grid = draw(&mut editor, 100, 6);
    let titles = row_symbols(&grid, 0, 0, 100).join("");
    assert!(
        titles.contains("╭ 1 ssh://fixture/repo/app.log"),
        "{titles}"
    );
    assert!(titles.contains("[RO]"), "{titles}");
    assert!(titles.contains("╭ 2 [scratch]"), "{titles}");

    let grid = draw(&mut editor, 48, 16);
    let titles = row_symbols(&grid, 0, 0, 48).join("");
    assert!(titles.contains("ssh://"), "{titles}");
    assert!(titles.contains("[RO]"), "{titles}");

    editor.focus_pane(0);
    let grid = draw(&mut editor, 100, 6);
    let titles = row_symbols(&grid, 0, 0, 100).join("");
    assert!(
        titles.contains("╭ 1 ssh://fixture/repo/app.log"),
        "{titles}"
    );
    assert!(titles.contains("╭ 2 [scratch]"), "{titles}");
}

#[test]
fn narrow_titles_preserve_focus_flags_geometry_and_single_vertical_rule() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sample.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();

    for vertical in [true, false] {
        let mut editor = Editor::new_in(Buffer::open(&path).unwrap(), root.path().to_path_buf());
        {
            let mut buffer = editor.fixture_buf_mut();
            buffer.dirty = true;
            buffer.readonly = true;
        }
        editor.split(vertical, None);
        let mut terminal = viewport_terminal(48, 16);
        terminal
            .draw(|f| crate::render::paint(&mut editor, f))
            .unwrap();

        if vertical {
            let grid = terminal.backend().buffer();
            let second_title = (0..48).find(|&x| grid[(x, 0)].symbol() == "2").unwrap();
            // `╭ 2 …`: the corner, a space, the number; the divider column
            // sits one cell left of the corner and carries the one rule
            assert_eq!(grid[(second_title - 1, 0)].symbol(), " ");
            assert_eq!(grid[(second_title - 2, 0)].symbol(), "╭");
            assert_eq!(
                grid[(second_title - 3, 1)].symbol(),
                "│",
                "the divider column"
            );
            assert_eq!(grid[(47, 1)].symbol(), " ", "reserved column, no track");
        } else {
            let grid = terminal.backend().buffer();
            assert!((0..grid.area.height).any(|y| grid[(0, y)].symbol() == "╰"));
        }

        for (width, height) in [(48, 16), (24, 10), (48, 16)] {
            terminal.backend_mut().resize(width, height);
            terminal
                .draw(|f| crate::render::paint(&mut editor, f))
                .unwrap();
            let text = grid_text(terminal.backend().buffer());
            assert!(text.contains("╭ 2 "), "{text}");
            assert!(text.contains('●'), "{text}");
            assert!(text.contains("[RO]"), "{text}");
        }
    }
}

#[test]
fn workspace_title_clipping_prints_controls_and_preserves_graphemes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("deep/tab\t界e\u{301}.rs");
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "\n").unwrap();
    let mut editor = Editor::new_in(Buffer::open(&path).unwrap(), root.path().to_path_buf());
    editor.split(true, None);

    let grid = draw(&mut editor, 24, 10);
    let titles = row_symbols(&grid, 0, 0, 24).join("");
    assert!(!titles.contains('\t'), "{titles:?}");
    assert!(titles.contains('界'), "{titles:?}");
    assert!(titles.contains("e\u{301}.rs"), "{titles:?}");
}

#[cfg(unix)]
#[test]
fn non_utf8_workspace_title_uses_lossy_display_without_changing_identity() {
    use std::os::unix::ffi::OsStrExt;

    let root = tempfile::tempdir().unwrap();
    let path = root
        .path()
        .join(std::ffi::OsStr::from_bytes(b"bad-\xff.rs"));
    std::fs::write(&path, "\n").unwrap();
    let mut editor = Editor::new_in(Buffer::open(&path).unwrap(), root.path().to_path_buf());
    editor.split(true, None);

    let grid = draw(&mut editor, 48, 16);
    let titles = row_symbols(&grid, 0, 0, 48).join("");
    assert!(titles.contains('\u{fffd}'), "{titles:?}");
    assert_eq!(editor.buf().path.as_deref(), Some(path.as_path()));
}
