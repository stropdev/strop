//! `:cd`: the root moves, and what was keyed by the old one follows.
//! The session, recovery and LSP re-anchorings are tested beside their
//! owners (`session/tests.rs`, `recovery/tests.rs`, `lsp/tests/project.rs`).

use std::path::PathBuf;

use strop_core::Buffer;
use strop_workspace::ResourceLocation;

use crate::editor::Editor;

/// `one/` (with `a.txt`) and `two/` side by side; the editor in `one`
/// on `a.txt`, named relative to the root as a command-line launch does.
fn two_projects() -> (tempfile::TempDir, PathBuf, PathBuf, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (one, two) = (root.join("one"), root.join("two"));
    std::fs::create_dir(&one).unwrap();
    std::fs::create_dir(&two).unwrap();
    std::fs::write(one.join("a.txt"), "alpha\n").unwrap();
    let mut buffer = Buffer::open(one.join("a.txt")).unwrap();
    buffer.path = Some("a.txt".into());
    let editor = Editor::new_in(buffer, one.clone());
    (dir, one, two, editor)
}

fn cd(editor: &mut Editor, argument: &str) {
    editor.feed_text(&format!(":cd {argument}<cr>"));
    editor.wait_io().unwrap();
}

#[test]
fn cd_moves_the_root_and_pwd_says_where() {
    let (_dir, one, two, mut e) = two_projects();
    cd(&mut e, "../two");
    assert_eq!(e.cwd, two);
    assert_eq!(e.message(), two.display().to_string());
    e.feed_text(":pwd<cr>");
    assert_eq!(e.message(), two.display().to_string());
    cd(&mut e, &one.display().to_string());
    assert_eq!(e.cwd, one, "an absolute directory");
}

#[test]
fn cd_dash_goes_back() {
    let (_dir, one, two, mut e) = two_projects();
    cd(&mut e, "-");
    assert_eq!(e.message(), "no previous directory");
    cd(&mut e, "../two");
    cd(&mut e, "-");
    assert_eq!(e.cwd, one);
    cd(&mut e, "-");
    assert_eq!(e.cwd, two);
}

#[test]
fn cd_to_nothing_or_to_a_file_stays_and_says_why() {
    let (_dir, one, _two, mut e) = two_projects();
    cd(&mut e, "missing");
    assert_eq!(e.cwd, one);
    assert!(e.message().contains("missing"), "{}", e.message());
    cd(&mut e, "a.txt");
    assert_eq!(e.cwd, one);
    assert!(
        e.message().ends_with("a.txt is not a directory"),
        "{}",
        e.message()
    );
}

#[test]
fn a_buffer_named_relative_to_the_old_root_still_names_its_file() {
    let (_dir, one, _two, mut e) = two_projects();
    cd(&mut e, "../two");
    assert_eq!(e.buf().path.as_deref(), Some(one.join("a.txt").as_path()));
    // and it saves where it came from, not into the new root
    e.feed_text("x:w<cr>");
    e.wait_io().unwrap();
    assert_eq!(
        std::fs::read_to_string(one.join("a.txt")).unwrap(),
        "lpha\n"
    );
}

#[test]
fn buffer_names_are_shown_relative_to_the_new_root() {
    let (_dir, one, two, mut e) = two_projects();
    std::fs::write(two.join("b.txt"), "beta\n").unwrap();
    e.open_fixture(&two.join("b.txt")).unwrap();
    let label = |e: &Editor| e.doc(e.current()).label(&e.cwd);
    assert_eq!(label(&e), two.join("b.txt").display().to_string());
    cd(&mut e, "../two");
    assert_eq!(label(&e), "b.txt");
    let listing = super::super::buffer_listing(&e, false);
    assert!(listing.contains("\"b.txt\""), "{listing}");
    assert!(
        listing.contains(&format!("\"{}\"", one.join("a.txt").display())),
        "{listing}"
    );
}

#[test]
fn files_search_and_new_terminals_scope_to_the_new_root() {
    let (_dir, one, two, mut e) = two_projects();
    cd(&mut e, "../two");
    let root = ResourceLocation::local(two.clone());
    assert_eq!(e.current_workspace_root(), root);
    e.open_picker(strop_picker::Kind::Files);
    assert_eq!(e.picker.as_ref().unwrap().file_root.as_ref(), Some(&root));
    e.close_picker();
    e.open_search(false);
    assert_eq!(e.search_scope().unwrap().root, root);
    e.close_picker();
    // the old root is no longer a selected project
    assert!(e.workspaces.iter().all(|(_, context)| context.root != one));
}

#[test]
fn cd_waits_for_a_write_admitted_against_the_old_root() {
    let (_dir, one, two, mut e) = two_projects();
    e.feed_text("x:w<cr>");
    super::super::change_directory(&mut e, two);
    assert_eq!(e.cwd, one);
    assert_eq!(
        e.message(),
        "a file is still being written; :cd again in a moment"
    );
    e.wait_io().unwrap();
}
