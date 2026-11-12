//! Editing through a collection (0094 §3.1): excerpt edits are source
//! edits, chrome is not text, `:w` writes the touched files and `u` is the
//! collection's own history.
use super::*;
use strop_core::frontend_input::Key;
use strop_core::id::DocumentId;

fn source(e: &Editor, path: &std::path::Path) -> DocumentId {
    e.docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_deref() == Some(path)).then_some(id))
        .unwrap()
}

fn text(e: &Editor, document: DocumentId) -> String {
    e.doc(document).buf.text().to_string()
}

fn at(e: &mut Editor, needle: &str) {
    let position = current_text(e).find(needle).unwrap();
    e.set_head(position);
}

/// The view is what a fresh rendering of its sources would be.
pub(super) fn assert_view_is_its_sources(e: &Editor, view: DocumentId) {
    let mut fresh = e.collections[&view].clone();
    let rendered = super::super::projection::render(&e.docs, &e.cwd, &mut fresh);
    assert_eq!(text(e, view), rendered, "the view drifted from its sources");
    let live = &e.collections[&view];
    for (now, expected) in live.excerpts.iter().zip(&fresh.excerpts) {
        assert_eq!(
            (now.view_start, now.view_end, now.view_line, now.view_lines),
            (
                expected.view_start,
                expected.view_end,
                expected.view_line,
                expected.view_lines
            ),
            "excerpt geometry drifted"
        );
    }
    assert_eq!(live.rows, fresh.rows, "row roles drifted");
}

#[test]
fn an_edit_inside_an_excerpt_changes_its_source() {
    let (mut e, a, b) = fixture();
    e.feed(Key::CtrlO);
    let view = e.current();
    let (a, b) = (source(&e, &a), source(&e, &b));
    at(&mut e, "alpha one");
    type_keys(&mut e, "cwomega<esc>");
    assert_eq!(text(&e, a), "omega one\nalpha two\n");
    assert!(e.doc(a).buf.is_dirty(), "the source owns the unsaved edit");
    assert!(!e.buf().is_dirty(), "the view's dirty bit is not the story");
    assert!(current_text(&e).contains("omega one"));
    at(&mut e, "beta two");
    type_keys(&mut e, "x");
    assert_eq!(text(&e, b), "beta one\neta two\n");
    assert_view_is_its_sources(&e, view);
}

#[test]
fn lines_open_delete_and_paste_inside_an_excerpt() {
    let (mut e, a, _) = fixture();
    e.feed(Key::CtrlO);
    let view = e.current();
    let a = source(&e, &a);
    at(&mut e, "alpha two");
    type_keys(&mut e, "onew line<esc>");
    assert_eq!(text(&e, a), "alpha one\nalpha two\nnew line\n");
    assert_view_is_its_sources(&e, view);
    type_keys(&mut e, "Oabove<esc>");
    assert_eq!(text(&e, a), "alpha one\nalpha two\nabove\nnew line\n");
    type_keys(&mut e, "ddp");
    assert_eq!(text(&e, a), "alpha one\nalpha two\nnew line\nabove\n");
    assert_view_is_its_sources(&e, view);
    at(&mut e, "alpha one");
    type_keys(&mut e, "dd");
    assert_eq!(text(&e, a), "alpha two\nnew line\nabove\n");
    assert_view_is_its_sources(&e, view);
}

#[test]
fn borders_gaps_and_the_title_are_not_text() {
    let (mut e, a, b) = fixture();
    e.feed(Key::CtrlO);
    let view = e.current();
    let (a, b) = (source(&e, &a), source(&e, &b));
    let before = (current_text(&e), text(&e, a), text(&e, b));
    let rows = current_text(&e).lines().count();
    for row in 0..rows {
        if e.collections[&view].rows[row] == crate::editor::CollectionRow::Body {
            continue;
        }
        for keys in ["x", "ix<esc>", "rZ", "D", "~"] {
            e.set_head(e.buf().line_start(row));
            type_keys(&mut e, keys);
            assert_eq!(
                (current_text(&e), text(&e, a), text(&e, b)),
                before,
                "{keys} on row {row} changed something"
            );
        }
    }
    // Joining the last body line with the border below it, and a delete
    // running from one file's excerpt into the next.
    at(&mut e, "alpha two");
    type_keys(&mut e, "J");
    assert!(e.message.contains("not text"), "{}", e.message);
    type_keys(&mut e, "dG");
    assert_eq!((current_text(&e), text(&e, a), text(&e, b)), before);
    type_keys(&mut e, "Vjjjd");
    assert_eq!((current_text(&e), text(&e, a), text(&e, b)), before);
    assert_view_is_its_sources(&e, view);
}

#[test]
fn w_writes_every_changed_file_the_collection_shows() {
    let (mut e, a_path, b_path) = fixture();
    e.feed(Key::CtrlO);
    let view = e.current();
    let (a, b) = (source(&e, &a_path), source(&e, &b_path));
    at(&mut e, "alpha one");
    type_keys(&mut e, "cwomega<esc>");
    at(&mut e, "beta two");
    type_keys(&mut e, "cwgamma<esc>");
    type_keys(&mut e, ":w<cr>");
    e.wait_io().unwrap();
    assert_eq!(e.current(), view, ":w keeps the collection open");
    assert_eq!(
        std::fs::read_to_string(&a_path).unwrap(),
        "omega one\nalpha two\n"
    );
    assert_eq!(
        std::fs::read_to_string(&b_path).unwrap(),
        "beta one\ngamma two\n"
    );
    assert!(!e.doc(a).buf.is_dirty());
    assert!(!e.doc(b).buf.is_dirty());
    assert_eq!(e.message, "collection: wrote 2 file(s)");
    type_keys(&mut e, ":w<cr>");
    assert_eq!(e.message, "collection: no changed files to write");
}

#[test]
fn u_is_the_collections_own_history_and_reverts_the_source() {
    let (mut e, a, b) = fixture();
    e.feed(Key::CtrlO);
    let view = e.current();
    let (a, b) = (source(&e, &a), source(&e, &b));
    at(&mut e, "alpha one");
    type_keys(&mut e, "cwomega<esc>");
    at(&mut e, "beta two");
    type_keys(&mut e, "cwgamma<esc>");
    type_keys(&mut e, "u");
    assert_eq!(text(&e, b), "beta one\nbeta two\n", "one u, one edit");
    assert_eq!(text(&e, a), "omega one\nalpha two\n");
    assert_view_is_its_sources(&e, view);
    type_keys(&mut e, "u");
    assert_eq!(text(&e, a), "alpha one\nalpha two\n");
    e.feed(Key::CtrlR);
    assert_eq!(text(&e, a), "omega one\nalpha two\n");
    assert_view_is_its_sources(&e, view);
    // The source's own history holds the same steps: its `u` works there.
    e.switch_to(a);
    type_keys(&mut e, "u");
    assert_eq!(text(&e, a), "alpha one\nalpha two\n");
    assert_view_is_its_sources(&e, view);
}

#[test]
fn an_insert_session_is_one_undo_step_in_the_source_too() {
    let (mut e, a, _) = fixture();
    e.feed(Key::CtrlO);
    let a = source(&e, &a);
    at(&mut e, "alpha one");
    type_keys(&mut e, "Ihello <esc>");
    assert_eq!(text(&e, a), "hello alpha one\nalpha two\n");
    e.switch_to(a);
    type_keys(&mut e, "u");
    assert_eq!(text(&e, a), "alpha one\nalpha two\n");
}

#[test]
fn a_source_edit_made_elsewhere_restarts_the_collections_history() {
    let (mut e, a, _) = fixture();
    e.feed(Key::CtrlO);
    let view = e.current();
    let a = source(&e, &a);
    at(&mut e, "alpha one");
    type_keys(&mut e, "x");
    e.switch_to(a);
    type_keys(&mut e, "Gx");
    e.switch_to(view);
    assert_view_is_its_sources(&e, view);
    type_keys(&mut e, "u");
    assert_eq!(e.message, "already at oldest change");
    assert_eq!(text(&e, a), "lpha one\nlpha two\n");
}

#[test]
fn two_collections_of_one_source_stay_in_step() {
    let (mut e, a, _) = fixture();
    e.feed(Key::CtrlO);
    let first = e.current();
    // a second collection, of another search, over one of the same lines
    e.open_picker(Kind::Search);
    e.picker.as_mut().unwrap().picker.paste("two");
    e.picker_items_fixture(vec![Item {
        badge: None,
        text: "a.txt:2".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(a.clone()),
            line: 2,
            col: 1,
            match_len: 5,
            line_text: "alpha two".into(),
        },
    }]);
    e.feed(Key::CtrlO);
    let second = e.current();
    assert_ne!(first, second);
    at(&mut e, "alpha two");
    type_keys(&mut e, "cwdelta<esc>");
    assert_view_is_its_sources(&e, first);
    assert_view_is_its_sources(&e, second);
    assert!(text(&e, first).contains("delta two"));
}

#[test]
fn a_source_without_a_final_newline_edits_like_any_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "one\nlast").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    let a = e.open_fixture(&path).unwrap();
    e.open_picker(Kind::Search);
    e.picker_items_fixture(vec![Item {
        badge: None,
        text: "a.txt:2".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path.clone()),
            line: 2,
            col: 1,
            match_len: 4,
            line_text: "last".into(),
        },
    }]);
    e.feed(Key::CtrlO);
    let view = e.current();
    at(&mut e, "last");
    type_keys(&mut e, "A!<esc>");
    assert_eq!(text(&e, a), "one\nlast!");
    assert_view_is_its_sources(&e, view);
    type_keys(&mut e, "omore<esc>");
    assert_eq!(text(&e, a), "one\nlast!\nmore");
    assert_view_is_its_sources(&e, view);
    type_keys(&mut e, "yyp");
    assert_eq!(text(&e, a), "one\nlast!\nmore\nmore");
    assert_view_is_its_sources(&e, view);
    type_keys(&mut e, "dd");
    assert_eq!(text(&e, a), "one\nlast!\nmore\n");
    assert_view_is_its_sources(&e, view);
    // an opened line left empty is not a line the source has
    type_keys(&mut e, "Go<esc>");
    assert_view_is_its_sources(&e, view);
}

#[test]
fn substitute_in_a_collection_changes_excerpts_and_never_a_border() {
    let (mut e, a, b) = fixture();
    e.feed(Key::CtrlO);
    let view = e.current();
    let (a, b) = (source(&e, &a), source(&e, &b));
    // `a` also spells part of both headings (a.txt, b.txt)
    type_keys(&mut e, ":%s/a/A/g<cr>");
    assert_eq!(text(&e, a), "AlphA one\nAlphA two\n");
    assert_eq!(text(&e, b), "betA one\nbetA two\n");
    assert!(current_text(&e).lines().any(|row| row == "a.txt"));
    assert_view_is_its_sources(&e, view);
    type_keys(&mut e, "u");
    assert_eq!(text(&e, a), "alpha one\nalpha two\n");
    assert_eq!(text(&e, b), "beta one\nbeta two\n");
    assert_view_is_its_sources(&e, view);
}

#[test]
fn a_read_only_source_refuses_by_name() {
    let (mut e, a, _) = fixture();
    e.feed(Key::CtrlO);
    let a = source(&e, &a);
    e.doc_mut(a).buf.readonly = true;
    at(&mut e, "alpha one");
    type_keys(&mut e, "x");
    assert!(e.message.contains("a.txt is read-only"), "{}", e.message);
    assert_eq!(text(&e, a), "alpha one\nalpha two\n");
}
