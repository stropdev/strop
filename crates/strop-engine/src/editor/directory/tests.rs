use super::*;
use strop_core::Buffer;

fn fixture() -> (tempfile::TempDir, Editor) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("child")).unwrap();
    std::fs::write(root.path().join("child/- name.txt"), "inside\n").unwrap();
    std::fs::write(root.path().join("other.txt"), "outside\n").unwrap();
    let editor = Editor::new_in(Buffer::from_text("origin\n"), root.path().to_path_buf());
    (root, editor)
}

#[test]
fn directory_file_and_parent_roundtrips_preserve_scope_and_selection() {
    let (root, mut editor) = fixture();
    editor.feed_text(":e child<cr>");
    editor.wait_io().unwrap();
    assert_eq!(
        editor.directory().unwrap().location.path,
        root.path().join("child")
    );
    assert_eq!(editor.cwd, root.path());
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "inside\n");
    assert!(
        !editor.buf().readonly,
        "a local file opened from Directory remains editable"
    );
    editor.feed(Key::CtrlO);
    assert_eq!(
        editor.directory().unwrap().location.path,
        root.path().join("child")
    );
    editor.feed(Key::Backspace);
    editor.wait_io().unwrap();
    assert_eq!(editor.directory().unwrap().location.path, root.path());
    assert_eq!(
        editor
            .directory()
            .unwrap()
            .entry(LineIndex::new(editor.buf().line_of(editor.head())))
            .unwrap()
            .name
            .as_path(),
        std::path::Path::new("child")
    );
    assert_eq!(editor.cwd, root.path());
}

#[test]
fn closed_directory_restores_filter_and_source_entry_from_native_identity() {
    let (root, mut editor) = fixture();
    editor.feed_text(":browse<cr>");
    editor.wait_io().unwrap();
    editor.feed_text(":filter path:child<cr>");
    editor.wait_io().unwrap();
    editor.view_mut().hscroll = strop_core::id::DisplayColumn::new(3);
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    editor.feed(Key::Backspace);
    editor.wait_io().unwrap();
    assert_eq!(editor.directory().unwrap().filter, "path:child");
    editor.feed_text(":q<cr>");
    assert_eq!(
        editor.directory().unwrap().location.path,
        root.path().join("child")
    );
    editor.feed(Key::Backspace);
    editor.wait_io().unwrap();
    assert_eq!(editor.directory().unwrap().filter, "path:child");
    assert_eq!(editor.view().hscroll.get(), 3);
}

#[test]
fn reveal_explicitly_clears_a_filter_that_hides_the_source() {
    let (root, mut editor) = fixture();
    editor.feed_text(":browse<cr>");
    editor.wait_io().unwrap();
    editor.feed_text(":filter path:child<cr>");
    editor.wait_io().unwrap();
    editor.request_open(
        root.path().join("other.txt"),
        OpenIntent::Switch { readonly: false },
    );
    editor.wait_io().unwrap();
    editor.reveal_source();
    editor.wait_io().unwrap();
    let directory = editor.directory().unwrap();
    assert!(directory.filter.is_empty());
    assert_eq!(
        directory
            .entry_location(LineIndex::new(editor.buf().line_of(editor.head())))
            .unwrap()
            .path,
        root.path().join("other.txt")
    );
}

#[test]
fn local_reload_preserves_identity_readonly_and_adopts_the_observed_save_baseline() {
    let (root, mut editor) = fixture();
    let path = root.path().join("other.txt");
    let source = editor.open_fixture(&path).unwrap();
    editor.buf_mut().readonly = true;
    std::fs::write(&path, "changed\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(42)),
        )
        .unwrap();
    editor.feed_text(":e<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), source);
    assert_eq!(editor.buf().text(), "changed\n");
    assert!(editor.buf().readonly);
    editor.feed_text(":set noro<cr>Iedited <esc>:w<cr>");
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "edited changed\n");
    assert!(!editor.buf().is_dirty());
}

#[test]
fn failed_refresh_retains_the_directory_instead_of_publishing_empty_success() {
    let (root, mut editor) = fixture();
    editor.feed_text(":e child<cr>");
    editor.wait_io().unwrap();
    let before = editor.buf().text().to_string();
    std::fs::remove_file(root.path().join("child/- name.txt")).unwrap();
    std::fs::remove_dir(root.path().join("child")).unwrap();
    editor.feed_text(":e!<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), before.as_str());
    assert!(editor.directory().unwrap().stale.is_some());
}

#[test]
fn ex_edit_from_a_directory_resolves_against_cwd_like_vim() {
    // 0088 U2: from `child/`, `:e other.txt` is the cwd's file — not a new
    // `child/other.txt` built from the Directory buffer's path.
    let (root, mut editor) = fixture();
    editor.feed_text(":e child<cr>");
    editor.wait_io().unwrap();
    assert!(editor.directory().is_some());
    editor.feed_text(":e other.txt<cr>");
    editor.wait_io().unwrap();
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(root.path().join("other.txt").as_path())
    );
    assert_eq!(editor.buf().text(), "outside\n");
    assert!(
        !editor.message().contains("[new file]"),
        "an existing file is not announced as new: {}",
        editor.message()
    );
}

#[test]
fn ex_edit_of_a_missing_path_says_new_file() {
    let (root, mut editor) = fixture();
    editor.feed_text(":e child<cr>");
    editor.wait_io().unwrap();
    editor.feed_text(":e fresh.txt<cr>");
    editor.wait_io().unwrap();
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(root.path().join("fresh.txt").as_path())
    );
    assert!(
        editor.message().contains("[new file]") && editor.message().contains("fresh.txt"),
        "{}",
        editor.message()
    );
}
