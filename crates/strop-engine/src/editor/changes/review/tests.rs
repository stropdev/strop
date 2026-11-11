use super::super::ChangeProducer;
use super::*;
use std::path::PathBuf;
use strop_core::id::LineIndex;
use strop_core::Buffer;
use strop_lsp::{PositionEncoding, ServerColumn, ServerEdit, ServerId, ServerPosition};
use strop_workspace::ResourceLocation;

fn at(line: usize, col: usize) -> ServerPosition {
    ServerPosition {
        line: LineIndex::new(line),
        column: ServerColumn::new(col),
    }
}

fn edit(sl: usize, sc: usize, el: usize, ec: usize, text: &str) -> ServerEdit {
    ServerEdit {
        start: at(sl, sc),
        end: at(el, ec),
        new_text: text.into(),
    }
}

fn file_editor(dir: &tempfile::TempDir, name: &str, text: &str) -> (Editor, DocumentId) {
    std::fs::write(dir.path().join(name), text).unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    e.open_fixture(&dir.path().join(name)).unwrap();
    let document = e.current();
    (e, document)
}

fn open_second(e: &mut Editor, dir: &tempfile::TempDir, name: &str, text: &str) -> DocumentId {
    std::fs::write(dir.path().join(name), text).unwrap();
    e.open_fixture(&dir.path().join(name)).unwrap();
    e.current()
}

/// Bind the document to a server so `build_change_plan` resolves its
/// location — the inverse map the live LSP path maintains.
fn bind(e: &mut Editor, document: DocumentId) {
    let (revision, path) = {
        let doc = e.docs.get(document).unwrap();
        (doc.buf.revision(), doc.buf.path.clone().unwrap())
    };
    e.lsp_state.bindings.insert(
        document,
        crate::editor::lsp::state::Binding {
            server: ServerId::new(7),
            revision,
            path,
            root: PathBuf::from("/workspace"),
            language: "rust".into(),
            target: strop_workspace::Filesystem::Local,
        },
    );
}

fn text_of(e: &Editor, document: DocumentId) -> String {
    e.docs.get(document).unwrap().buf.text().to_string()
}

/// A two-document rename proposal with one unopened (refused) target.
fn two_file_editor() -> (Editor, tempfile::TempDir, DocumentId, DocumentId) {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, first) = file_editor(&dir, "a.txt", "alpha here\n");
    let second = open_second(&mut e, &dir, "b.txt", "alpha there\n");
    bind(&mut e, first);
    bind(&mut e, second);
    let plan = e.build_change_plan(
        ChangeProducer::Rename,
        vec![
            (
                ResourceLocation::local(dir.path().join("a.txt")),
                vec![edit(0, 0, 0, 5, "omega")],
            ),
            (
                ResourceLocation::local(dir.path().join("b.txt")),
                vec![edit(0, 0, 0, 5, "omega")],
            ),
            (
                ResourceLocation::local(dir.path().join("ghost.txt")),
                vec![edit(0, 0, 0, 3, "x")],
            ),
        ],
        PositionEncoding::Utf8,
    );
    e.present_change_plan(plan);
    (e, dir, first, second)
}

fn disk(dir: &tempfile::TempDir, name: &str) -> String {
    std::fs::read_to_string(dir.path().join(name)).unwrap()
}

#[test]
fn multi_target_plan_opens_a_diff_review_without_mutating() {
    let (e, _dir, first, _second) = two_file_editor();
    assert_eq!(e.buf().name.as_deref(), Some("review: rename"));
    let review = e.current();
    assert_ne!(review, first, "the review buffer takes focus");
    assert!(e.buf().readonly, "the review is a real read-only buffer");
    let text = e.buf().text().to_string();
    assert!(
        text.starts_with("rename: 2 file(s) to change, 1 refused\nEnter applies and writes"),
        "{text}"
    );
    // Per-file unified diffs against the pinned bases.
    assert!(text.contains("--- a/"), "{text}");
    assert!(text.contains("+++ b/"), "{text}");
    assert!(text.contains("@@ -1,1 +1,1 @@"), "{text}");
    assert!(text.contains("-alpha here"), "{text}");
    assert!(text.contains("+omega here"), "{text}");
    assert!(text.contains("-alpha there"), "{text}");
    assert!(text.contains("+omega there"), "{text}");
    // The unopened target is named with its reason, never dropped.
    assert!(text.contains("ghost.txt"), "{text}");
    assert!(text.contains("not open on the server"), "{text}");
    // Nothing has mutated yet.
    assert_eq!(text_of(&e, first), "alpha here\n");
    assert!(
        e.message.contains("Enter applies and writes"),
        "{}",
        e.message
    );
}

#[test]
fn enter_applies_and_writes_and_u_reverts_buffers_and_files() {
    let (mut e, dir, first, second) = two_file_editor();
    let review = e.current();
    e.feed(strop_core::frontend_input::Key::Enter);
    assert_eq!(text_of(&e, first), "omega here\n");
    assert_eq!(text_of(&e, second), "omega there\n");
    e.wait_io().unwrap();
    assert_eq!(disk(&dir, "a.txt"), "omega here\n");
    assert_eq!(disk(&dir, "b.txt"), "omega there\n");
    assert!(!e.docs.get(first).unwrap().buf.is_dirty());
    assert!(!e.docs.get(second).unwrap().buf.is_dirty());
    assert_eq!(e.message, "rename: wrote 2 file(s)");
    // The same buffer keeps the diff under a heading that says what happened.
    let after = text_of(&e, review);
    assert!(
        after.starts_with("rename: changed and wrote 2 file(s), 1 refused\nu reverts"),
        "{after}"
    );
    assert!(after.contains("-alpha here"), "{after}");
    assert!(after.contains("ghost.txt"), "{after}");

    e.feed_text("u");
    assert_eq!(text_of(&e, first), "alpha here\n");
    assert_eq!(text_of(&e, second), "alpha there\n");
    e.wait_io().unwrap();
    assert_eq!(disk(&dir, "a.txt"), "alpha here\n");
    assert_eq!(disk(&dir, "b.txt"), "alpha there\n");
    assert!(!e.docs.get(first).unwrap().buf.is_dirty());
    assert!(!e.docs.get(second).unwrap().buf.is_dirty());
    let after = text_of(&e, review);
    assert!(
        after.starts_with("rename: reverted and wrote 2 file(s), 1 refused\nEnter applies again"),
        "{after}"
    );

    // The reverted text is the reviewed text: Enter applies it again.
    e.feed(strop_core::frontend_input::Key::Enter);
    e.wait_io().unwrap();
    assert_eq!(disk(&dir, "a.txt"), "omega here\n");
    assert_eq!(disk(&dir, "b.txt"), "omega there\n");
}

#[test]
fn u_names_the_files_it_could_not_revert() {
    let (mut e, dir, first, second) = two_file_editor();
    let review = e.current();
    e.review_apply_pub();
    e.wait_io().unwrap();
    e.switch_to(first);
    e.feed_text("0rx"); // a.txt moves past what the apply left
    e.switch_to(review);
    e.feed_text("u");
    e.wait_io().unwrap();
    assert_eq!(text_of(&e, first), "xmega here\n", "the user's edit stands");
    assert_eq!(text_of(&e, second), "alpha there\n");
    assert_eq!(disk(&dir, "b.txt"), "alpha there\n");
    let after = text_of(&e, review);
    assert!(after.contains("a.txt (edited since)"), "{after}");
}

#[test]
fn closing_an_unapplied_review_cancels_it() {
    let (mut e, dir, first, second) = two_file_editor();
    let review = e.current();
    e.feed_text("q");
    assert!(e.docs.get(review).is_none(), "q closes the review");
    assert_eq!(text_of(&e, first), "alpha here\n");
    assert_eq!(text_of(&e, second), "alpha there\n");
    assert_eq!(disk(&dir, "a.txt"), "alpha here\n");
    assert!(e.message.contains("cancelled"), "{}", e.message);
    e.review_apply_pub();
    assert_eq!(e.message, "no review to apply");
}

#[test]
fn a_source_edited_since_the_review_is_refused_by_name() {
    let (mut e, _dir, first, second) = two_file_editor();
    let review = e.current();
    e.switch_to(first);
    e.feed_text("0rx"); // revision moves past the review's base
    e.switch_to(review);
    e.review_apply_pub();
    // The user's edit stands; the review never recomputes onto it.
    assert_eq!(text_of(&e, first), "xlpha here\n");
    assert_eq!(text_of(&e, second), "omega there\n");
    let after = text_of(&e, review);
    assert!(
        after.starts_with("rename: changed 1 file(s), 2 refused"),
        "{after}"
    );
}

#[test]
fn a_source_closed_since_the_review_is_refused_by_name() {
    let (mut e, _dir, first, second) = two_file_editor();
    let review = e.current();
    e.switch_to(second);
    e.close_buffer(true); // close b.txt after the review
    assert_eq!(text_of(&e, first), "alpha here\n");
    e.switch_to(review);
    e.review_apply_pub();
    assert_eq!(text_of(&e, first), "omega here\n");
    let after = text_of(&e, review);
    assert!(
        after.starts_with("rename: changed 1 file(s), 2 refused"),
        "{after}"
    );
}

#[test]
fn a_single_clean_document_applies_directly() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, document) = file_editor(&dir, "a.txt", "alpha here\n");
    bind(&mut e, document);
    let plan = e.build_change_plan(
        ChangeProducer::Rename,
        vec![(
            ResourceLocation::local(dir.path().join("a.txt")),
            vec![edit(0, 0, 0, 5, "omega")],
        )],
        PositionEncoding::Utf8,
    );
    e.present_change_plan(plan);
    assert_eq!(e.current(), document, "focus stays on the edited file");
    assert_eq!(text_of(&e, document), "omega here\n");
    assert_eq!(e.message, "rename: applied to 1 buffer(s)");
    // One buffer: its own `u` is the undo.
    e.feed_text("u");
    assert_eq!(text_of(&e, document), "alpha here\n");
}

#[test]
fn a_newer_review_supersedes_the_older_buffer_visibly() {
    let (mut e, dir, first, _second) = two_file_editor();
    let stale = e.current();
    let plan = e.build_change_plan(
        ChangeProducer::CodeAction,
        vec![
            (
                ResourceLocation::local(dir.path().join("a.txt")),
                vec![edit(0, 0, 0, 5, "sigma")],
            ),
            (
                ResourceLocation::local(dir.path().join("b.txt")),
                vec![edit(0, 0, 0, 5, "sigma")],
            ),
        ],
        PositionEncoding::Utf8,
    );
    e.present_change_plan(plan);
    assert_ne!(e.current(), stale);
    let retired = text_of(&e, stale);
    assert!(
        retired.contains("superseded by a newer review"),
        "{retired}"
    );
    assert!(e
        .buf()
        .text()
        .to_string()
        .starts_with("code action: 2 file(s)"));
    e.review_apply_pub();
    assert_eq!(text_of(&e, first), "sigma here\n");
}

#[test]
fn nearby_edits_share_one_hunk_with_context_on_both_sides() {
    // Rendering is exercised through the public review buffer, so the
    // hunk layout a user sees is what is asserted.
    let dir = tempfile::tempdir().unwrap();
    let (mut e, document) = file_editor(
        &dir,
        "a.txt",
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    );
    bind(&mut e, document);
    let plan = e.build_change_plan(
        ChangeProducer::CodeAction,
        vec![
            (
                ResourceLocation::local(dir.path().join("a.txt")),
                vec![edit(1, 0, 1, 3, "TWO"), edit(2, 0, 2, 5, "THREE")],
            ),
            (
                ResourceLocation::local(dir.path().join("ghost.txt")),
                vec![edit(0, 0, 0, 1, "x")],
            ),
        ],
        PositionEncoding::Utf8,
    );
    e.present_change_plan(plan);
    let text = e.buf().text().to_string();
    // Adjacent edits merge into one hunk with context on both sides.
    assert!(text.contains("@@ -1,6 +1,6 @@"), "{text}");
    assert!(
        text.contains(" one\n-two\n+TWO\n-three\n+THREE\n four\n five\n six\n"),
        "{text}"
    );
    assert_eq!(
        text.lines().filter(|line| line.starts_with("@@")).count(),
        1,
        "{text}"
    );
    e.review_apply_pub();
    assert_eq!(
        text_of(&e, document),
        "one\nTWO\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nten\n"
    );
}

#[test]
fn editing_a_review_cannot_apply_the_hidden_original_plan() {
    let (mut editor, _dir, first, second) = two_file_editor();
    editor.feed_text(":set noro<cr>ggIchanged review<esc>");
    editor.review_apply_pub();
    assert_eq!(text_of(&editor, first), "alpha here\n");
    assert_eq!(text_of(&editor, second), "alpha there\n");
    assert!(
        editor.message.contains("review changed"),
        "{}",
        editor.message
    );
}

#[test]
fn a_file_that_could_not_be_written_is_named_and_stays_dirty() {
    let (mut editor, dir, _first, second) = two_file_editor();
    let review = editor.current();
    // b.txt becomes unwritable between the review and the apply.
    std::fs::remove_file(dir.path().join("b.txt")).unwrap();
    std::fs::create_dir(dir.path().join("b.txt")).unwrap();
    editor.review_apply_pub();
    editor.wait_io().unwrap();
    assert_eq!(disk(&dir, "a.txt"), "omega here\n");
    assert!(editor.docs.get(second).unwrap().buf.is_dirty());
    let after = text_of(&editor, review);
    assert!(after.contains("could not write b.txt"), "{after}");
    assert!(
        editor
            .message
            .contains("wrote 1 file(s); could not write b.txt"),
        "{}",
        editor.message
    );
}
