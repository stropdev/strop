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
    editor.feed(Key::Char('-'));
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
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    editor.feed_text(":filter path:child<cr>");
    editor.wait_io().unwrap();
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    editor.feed(Key::Char('-'));
    editor.wait_io().unwrap();
    assert_eq!(editor.directory().unwrap().filter, "path:child");
    editor.feed_text(":q<cr>");
    assert_eq!(
        editor.directory().unwrap().location.path,
        root.path().join("child")
    );
    editor.feed(Key::Char('-'));
    editor.wait_io().unwrap();
    assert_eq!(editor.directory().unwrap().filter, "path:child");
    assert_eq!(editor.buf().text(), "child/\n");
    assert_eq!(editor.buf().line_of(editor.head()), 0, "on the folder left");
}

#[test]
fn reveal_explicitly_clears_a_filter_that_hides_the_source() {
    let (root, mut editor) = fixture();
    editor.feed_text(":e .<cr>");
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
    assert_eq!(
        editor.directory_summary().as_deref(),
        Some("1 item · out of date")
    );
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

#[test]
fn line_n_is_entry_n_and_a_yank_is_the_name() {
    let (root, mut editor) = fixture();
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "child/\nother.txt\n");
    assert!(!editor.buf().readonly, "a folder is an editable buffer");
    let directory = editor.directory().unwrap();
    assert_eq!(
        directory.entry(LineIndex::new(1)).unwrap().name.as_path(),
        std::path::Path::new("other.txt")
    );
    assert_eq!(
        directory.line_for(&ResourceLocation::local(root.path().join("child"))),
        Some(LineIndex::new(0))
    );
    editor.feed_text("jyy");
    assert_eq!(editor.register(None).text, "other.txt\n");
}

#[test]
fn pending_edits_survive_entering_a_child_and_coming_back() {
    let (_root, mut editor) = fixture();
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    editor.feed_text("jccrenamed.txt<esc>gg");
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "- name.txt\n");
    editor.feed(Key::Char('-'));
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "child/\nrenamed.txt\n");
    assert!(editor.buf().is_dirty());
    assert_eq!(
        editor.buf().line_of(editor.head()),
        0,
        "cursor on the child"
    );
}

#[test]
fn enter_on_a_name_not_yet_written_says_so() {
    let (_root, mut editor) = fixture();
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    // No indentation job: a folder's names are not code.
    editor.feed_text("Gonew.txt<esc>");
    assert_eq!(editor.buf().text(), "child/\nother.txt\nnew.txt\n");
    editor.feed(Key::Enter);
    assert_eq!(editor.message(), "not on disk yet; :w creates it");
    assert!(editor.directory().is_some());
}

#[test]
fn bang_edit_drops_pending_edits_and_lists_the_folder_again() {
    let (_root, mut editor) = fixture();
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    editor.feed_text("ddu");
    editor.feed_text("dd");
    assert!(editor.buf().is_dirty());
    editor.feed_text(":e<cr>");
    assert_eq!(editor.message(), "unsaved changes — :e! to force");
    editor.feed_text(":e!<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "child/\nother.txt\n");
    assert!(!editor.buf().is_dirty());
    assert!(!editor.buf().readonly);
}

#[test]
fn dotfiles_toggle_with_g_dot_and_the_choice_is_remembered() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join(".env"), "").unwrap();
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "child/\n.env\nother.txt\n");
    editor.feed_text("Gg.");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "child/\nother.txt\n");
    assert_eq!(
        editor.directory_summary().as_deref(),
        Some("2 items · dotfiles hidden")
    );
    assert_eq!(
        editor.buf().line_of(editor.head()),
        1,
        "cursor stays on other.txt"
    );
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    editor.feed(Key::CtrlO);
    editor.wait_io().unwrap();
    editor.feed_text(":e<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "child/\nother.txt\n");
    editor.feed_text("g.");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "child/\n.env\nother.txt\n");
}

#[test]
fn filtering_keeps_the_cursor_on_its_entry_and_refuses_pending_edits() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("zeta.rs"), "").unwrap();
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    editor.feed_text("G:filter rs<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "zeta.rs\n");
    assert_eq!(
        editor.directory_summary().as_deref(),
        Some("1 of 3 · filter rs")
    );
    editor.feed_text(":filter<cr>");
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().line_of(editor.head()), 2, "still on zeta.rs");
    editor.feed_text("ccx<esc>:filter rs<cr>");
    assert_eq!(
        editor.message(),
        "write (:w) or drop (:e!) the pending changes first"
    );
}

#[test]
fn a_folder_over_the_row_limit_is_read_only_and_says_how_to_edit() {
    let (root, mut editor) = fixture();
    let many = root.path().join("many");
    std::fs::create_dir(&many).unwrap();
    for index in 0..=crate::editor::filesystem::draft::ROW_LIMIT {
        std::fs::write(many.join(format!("f{index:04}")), "").unwrap();
    }
    editor.feed_text(":e many<cr>");
    editor.wait_io().unwrap();
    assert!(editor.buf().readonly);
    editor.feed_text("dd");
    assert_eq!(
        editor.message(),
        "more than 512 entries; :filter first to edit names"
    );
    editor.feed_text(":filter f000<cr>");
    editor.wait_io().unwrap();
    assert!(
        !editor.buf().readonly,
        "a filtered view fits and is editable"
    );
}

#[test]
fn a_folder_reopens_with_the_cursor_where_it_was() {
    let (_root, mut editor) = fixture();
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    editor.feed_text("j");
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(editor.buf().text(), "outside\n");
    editor.feed_text(":e .<cr>");
    editor.wait_io().unwrap();
    assert!(editor.directory().is_some());
    assert_eq!(editor.buf().line_of(editor.head()), 1, "back on other.txt");
}
