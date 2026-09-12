use super::super::ChangeProducer;
use super::*;
use std::path::PathBuf;
use strop_core::id::LineIndex;
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

#[test]
fn multi_target_plan_opens_a_diff_review_without_mutating() {
    let (e, _dir, first, _second) = two_file_editor();
    assert_eq!(e.buf().name.as_deref(), Some("change proposal 1"));
    let review = e.current();
    assert_ne!(review, first, "the review buffer takes focus");
    assert!(e.buf().readonly, "the review is a real read-only buffer");
    let text = e.buf().text().to_string();
    assert!(text.contains("strop change proposal 1: rename"), "{text}");
    assert!(text.contains(APPLY_COMMAND), "{text}");
    assert!(text.contains(CANCEL_COMMAND), "{text}");
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
    assert!(e.message.contains("proposal 1"), "{}", e.message);
}

#[test]
fn apply_mutates_exactly_the_planned_sources_and_leaves_a_receipt() {
    let (mut e, _dir, first, second) = two_file_editor();
    let review = e.current();
    e.review_apply_pub();
    assert_eq!(text_of(&e, first), "omega here\n");
    assert_eq!(text_of(&e, second), "omega there\n");
    // The review buffer stays around as the receipt.
    let receipt = text_of(&e, review);
    assert!(receipt.contains("applied: "), "{receipt}");
    assert!(receipt.contains("a.txt"), "{receipt}");
    assert!(receipt.contains("b.txt"), "{receipt}");
    assert!(receipt.contains("refused: "), "{receipt}");
    assert!(receipt.contains("ghost.txt"), "{receipt}");
    assert!(receipt.contains("not open on the server"), "{receipt}");
    // The recorded receipt anchors grouped undo across both buffers.
    e.feed_text(":undo-change\r");
    assert_eq!(text_of(&e, first), "alpha here\n");
    assert_eq!(text_of(&e, second), "alpha there\n");
}

#[test]
fn cancel_mutates_nothing_and_leaves_a_cancelled_receipt() {
    let (mut e, _dir, first, second) = two_file_editor();
    let review = e.current();
    e.review_cancel_pub();
    assert_eq!(text_of(&e, first), "alpha here\n");
    assert_eq!(text_of(&e, second), "alpha there\n");
    let receipt = text_of(&e, review);
    assert!(receipt.contains("— CANCELLED"), "{receipt}");
    assert!(receipt.contains("nothing applied"), "{receipt}");
    assert!(receipt.contains("ghost.txt"), "{receipt}");
    assert!(e.message.contains("cancelled"), "{}", e.message);
    // No receipt was recorded: there is nothing to undo.
    e.feed_text(":undo-change\r");
    assert_eq!(e.message, "no change to undo");
    // A second cancel is a named no-op, not a panic or a stale apply.
    e.review_cancel_pub();
    assert_eq!(e.message, "no change proposal awaiting review");
}

#[test]
fn a_source_edited_since_the_proposal_is_refused_by_name() {
    let (mut e, _dir, first, second) = two_file_editor();
    let review = e.current();
    e.switch_to(first);
    e.feed_text("0rx"); // revision moves past the proposal's base
    e.review_apply_pub();
    // The user's edit stands; the proposal never recomputes onto it.
    assert_eq!(text_of(&e, first), "xlpha here\n");
    assert_eq!(text_of(&e, second), "omega there\n");
    assert_eq!(
        e.message,
        "rename: 1 buffer(s) applied, 2 target(s) refused"
    );
    let receipt = text_of(&e, review);
    assert!(receipt.contains("a.txt"), "{receipt}");
    assert!(receipt.contains("edited since the proposal"), "{receipt}");
    assert!(receipt.contains("re-run the rename"), "{receipt}");
}

#[test]
fn a_source_closed_since_the_proposal_is_refused_by_name() {
    let (mut e, _dir, first, second) = two_file_editor();
    let review = e.current();
    e.switch_to(second);
    e.close_buffer(true); // close b.txt after the proposal
    assert_eq!(text_of(&e, first), "alpha here\n");
    e.review_apply_pub();
    assert_eq!(text_of(&e, first), "omega here\n");
    let receipt = text_of(&e, review);
    assert!(receipt.contains("closed since the proposal"), "{receipt}");
    assert_eq!(
        e.message,
        "rename: 1 buffer(s) applied, 2 target(s) refused"
    );
}

#[test]
fn a_single_clean_document_applies_directly_with_the_existing_receipt() {
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
}

#[test]
fn a_newer_proposal_supersedes_the_older_buffer_visibly() {
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
    assert!(retired.contains("SUPERSEDED"), "{retired}");
    assert!(e.buf().text().to_string().contains("change proposal 2"));
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
    editor.feed_text(":set noro<cr>ggIchanged review<esc>:apply-change<cr>");
    assert_eq!(text_of(&editor, first), "alpha here\n");
    assert_eq!(text_of(&editor, second), "alpha there\n");
    assert!(
        editor.message.contains("review changed"),
        "{}",
        editor.message
    );
}

#[test]
fn save_change_retains_each_file_outcome_after_focus_moves() {
    let (mut editor, dir, first, second) = two_file_editor();
    editor.review_apply_pub();
    editor.doc_mut(second).buf.readonly = true;
    editor.save_changed_files_pub();
    let report = editor.current();
    editor.switch_to(first);
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), first, "completion must not steal focus");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "omega here\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "alpha there\n"
    );
    assert!(editor.docs.get(second).unwrap().buf.dirty);
    let results = text_of(&editor, report);
    assert!(
        results
            .lines()
            .any(|line| line.contains("a.txt") && line.contains("written")),
        "{results}"
    );
    assert!(
        results
            .lines()
            .any(|line| line.contains("b.txt") && line.contains("read")),
        "{results}"
    );
}
