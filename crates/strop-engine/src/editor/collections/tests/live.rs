use super::*;

use strop_core::id::DocumentId;

#[test]
fn context_merges_preserve_same_line_matches_and_source_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    std::fs::write(&path, "before\nfn hit() { hit(); }\nafter\nmore\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_fixture(&path).unwrap();
    editor.open_picker(Kind::Search);
    let items = [4, 12].map(|column| Item {
        badge: None,
        text: "hit".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path.clone()),
            line: 2,
            col: column,
            match_len: 3,
            line_text: "fn hit() { hit(); }".into(),
        },
        facts: Default::default(),
    });
    editor.picker_items_fixture(items.to_vec());
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    assert!(current_text(&editor).contains("2 hits in 1 file"));
    assert!(current_text(&editor).contains("before\nfn hit() { hit(); }\nafter"));
    let at = current_text(&editor).find("fn hit").unwrap() + 3;
    editor.set_head(at);
    editor.feed_text("--");
    assert!(!current_text(&editor).contains("before\n"));
    assert_eq!(
        editor
            .buf()
            .text()
            .byte_slice(editor.head()..editor.head() + 3)
            .to_string(),
        "hit"
    );
    editor.feed_text("+");
    assert!(current_text(&editor).contains("before\n"));
    assert_eq!(
        editor
            .buf()
            .text()
            .byte_slice(editor.head()..editor.head() + 3)
            .to_string(),
        "hit"
    );
}

#[test]
fn context_expansion_preserves_marks_selections_and_return_points() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lines");
    std::fs::write(
        &path,
        "one\ntwo\nthree\nfour\nneedle here\nsix\nseven\neight\nnine\n",
    )
    .unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_fixture(&path).unwrap();
    let source = editor.current();
    editor.open_picker(Kind::Search);
    editor.picker_items_fixture(vec![Item {
        badge: None,
        text: "needle".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path),
            line: 5,
            col: 1,
            match_len: 6,
            line_text: "needle here".into(),
        },
        facts: Default::default(),
    }]);
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = editor.current();
    let needle = current_text(&editor).find("needle here").unwrap();
    let source_needle = editor.doc(source).buf.line_start(4);
    editor.set_head(needle + 6);
    editor.open_help();
    let help = editor.current();
    editor.switch_to(collection);
    editor.set_head(needle + 3);
    editor.feed_text("ma");
    editor.view_mut().sels.plant_extra(needle + 1);
    let original = editor.active_pane;
    editor.split(true, None);
    editor.set_head(needle);
    editor.feed_text("+");
    let new_needle = current_text(&editor).find("needle here").unwrap();
    assert_ne!(needle, new_needle);
    assert_eq!(editor.head(), new_needle);
    let split = &editor.panes[original].sels;
    assert_eq!(split.primary().head, new_needle + 3);
    assert_eq!(split.extra_heads()[0].head, new_needle + 1);
    editor.feed_text("`a");
    assert_eq!(
        editor.source_position(collection, editor.head()),
        Some((source, source_needle + 3))
    );
    editor.switch_to(help);
    assert!(editor.close_buffer(false));
    assert_eq!(editor.current(), collection);
    assert_eq!(
        editor.source_position(collection, editor.head()),
        Some((source, source_needle + 6))
    );
}

#[test]
fn closing_a_source_removes_only_its_excerpts_and_keeps_the_view_usable() {
    let (mut editor, path, _) = fixture();
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = editor.current();
    let source = editor
        .docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&path)).then_some(id))
        .unwrap();
    let survivor = current_text(&editor).find("beta two").unwrap() + 2;
    editor.set_head(survivor);
    editor.feed_text("mb");
    editor.set_head(current_text(&editor).find("alpha one").unwrap());
    editor.feed_text("ma");
    editor.switch_to(source);
    assert!(editor.close_buffer(false));
    editor.switch_to(collection);
    assert!(!current_text(&editor).contains("alpha one"));
    assert!(current_text(&editor).contains("beta two"));
    editor.feed_text("`a");
    assert_eq!(editor.head(), 0, "a removed file lands on the title");
    editor.feed_text("`b");
    let beta = current_text(&editor).find("beta two").unwrap();
    assert_eq!(editor.head(), beta + 2, "an earlier excerpt disappeared");
    editor.set_head(beta);
    editor.feed_text("rX");
    assert!(current_text(&editor).contains("Xeta two"));
}

#[test]
fn indentation_controls_capture_the_excerpt_source_not_the_projection() {
    let (mut editor, a, b) = fixture();
    let source = |editor: &Editor, path: &std::path::Path| {
        editor
            .docs
            .iter()
            .find_map(|(id, doc)| (doc.buf.path.as_deref() == Some(path)).then_some(id))
            .unwrap()
    };
    let (a_id, b_id) = (source(&editor, &a), source(&editor, &b));
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    editor.set_head(current_text(&editor).find("alpha one").unwrap());
    editor.feed_text(":tab-size 3<cr>");
    assert_eq!(editor.doc(a_id).indent.width, 3);
    assert_eq!(
        editor.indentation_at(editor.current(), editor.head()).width,
        3
    );
    editor.feed_text(":tab-size<cr>");
    editor.wait_picker();
    editor.switch_to(b_id);
    editor.feed(strop_core::frontend_input::Key::Enter);
    assert_eq!(editor.doc(a_id).indent.width, 2);
    assert_eq!(editor.doc(b_id).indent.width, 4);
    assert_eq!(editor.current(), b_id);
}

#[test]
fn source_edits_preserve_collection_carets_and_jump_anchors() {
    let (mut editor, a, _) = fixture();
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = editor.current();
    let source = editor
        .docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    let start = current_text(&editor).find("alpha one").unwrap();
    editor.set_head(start + 5);
    editor.push_jump();
    editor.doc_mut(source).buf.edit().insert(0, "\n").unwrap();
    assert_eq!(
        editor.source_position(collection, editor.head()),
        Some((source, 6))
    );
    editor.set_head(editor.buf().line_start(editor.buf().last_content_line()));
    editor.jump_back();
    assert_eq!(
        editor.source_position(collection, editor.head()),
        Some((source, 6))
    );
}

fn gap_fixture(dir: &std::path::Path) -> (Editor, DocumentId, DocumentId) {
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(
        &a,
        (0..20)
            .map(|line| format!("alpha {line:02}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(&b, "beta one\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.to_path_buf());
    editor.open_fixture(&a).unwrap();
    let a_id = editor.current();
    editor.open_fixture(&b).unwrap();
    let b_id = editor.current();
    editor.open_picker(Kind::Search);
    editor.picker_items_fixture(vec![
        Item {
            badge: None,
            text: "a first".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(a.clone()),
                line: 1,
                col: 1,
                match_len: 5,
                line_text: "alpha 00".into(),
            },
            facts: Default::default(),
        },
        Item {
            badge: None,
            text: "a later".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(a.clone()),
                line: 15,
                col: 1,
                match_len: 5,
                line_text: "alpha 14".into(),
            },
            facts: Default::default(),
        },
        Item {
            badge: None,
            text: "b".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(b),
                line: 1,
                col: 1,
                match_len: 4,
                line_text: "beta one".into(),
            },
            facts: Default::default(),
        },
    ]);
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    (editor, a_id, b_id)
}

#[test]
fn gap_refresh_keeps_carets_and_history_on_their_own_sources() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, a_id, b_id) = gap_fixture(dir.path());
    let collection = editor.current();
    editor.set_head(current_text(&editor).find("beta one").unwrap());
    editor.push_jump();
    editor.set_head(current_text(&editor).find("alpha 14").unwrap());
    let expected = editor.doc(a_id).buf.line_start(14) + 6;
    let gap = editor.doc(a_id).buf.line_start(6);
    editor
        .doc_mut(a_id)
        .buf
        .edit()
        .insert(gap, "extra\n")
        .unwrap();
    assert_eq!(
        editor.source_position(collection, editor.head()),
        Some((a_id, expected))
    );
    editor.jump_back();
    assert_eq!(
        editor.source_position(collection, editor.head()),
        Some((b_id, 0))
    );
}

#[test]
fn context_merge_and_split_follow_the_byte_not_the_excerpt_slot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, source, _) = gap_fixture(dir.path());
    let later = current_text(&editor).find("alpha 14").unwrap() + 2;
    editor.set_head(later);
    editor.feed_text("ma");
    editor.set_head(current_text(&editor).find("alpha 00").unwrap());
    editor.feed_text("+++++++++");
    editor.feed_text("`a");
    assert_eq!(
        editor.head(),
        current_text(&editor).find("alpha 14").unwrap() + 2
    );

    // At radius 6 the two hits split apart again, hiding alpha 07.
    // Its first byte is closest to the end of the preceding surviving line.
    editor.set_head(current_text(&editor).find("alpha 07").unwrap());
    editor.feed_text("mb");
    editor.set_head(current_text(&editor).find("alpha 00").unwrap());
    editor.feed_text("-----");
    assert!(!current_text(&editor).contains("alpha 07"));
    editor.feed_text("`a");
    assert_eq!(
        editor.head(),
        current_text(&editor).find("alpha 14").unwrap() + 2
    );
    editor.feed_text("`b");
    let (owner, byte) = editor
        .source_position(editor.current(), editor.head())
        .unwrap();
    assert_eq!((owner, editor.doc(source).buf.line_of(byte)), (source, 6));
}

#[test]
fn structural_source_edit_preserves_parked_collection_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let (mut editor, source, _) = gap_fixture(dir.path());
    let collection = editor.current();
    crate::editor::tabs::open_document(&mut editor, collection);
    editor.view_rows = 3;
    let later = current_text(&editor).find("alpha 14").unwrap();
    let old_top = editor.buf().line_of(later);
    editor.set_head(later);
    editor.feed_text("zt");
    assert_eq!(editor.view_top(), old_top);
    crate::editor::tabs::go(&mut editor, 0);
    editor.switch_to(source);
    // Delete across the first excerpt's edge, forcing a structural rebuild
    // which removes visible rows above the parked pane.
    let end = editor.buf().line_start(5);
    editor
        .doc_mut(source)
        .buf
        .edit()
        .replace(strop_core::Range::charwise(0, end), "")
        .unwrap();
    crate::editor::tabs::go(&mut editor, 1);
    assert_eq!(editor.current(), collection);
    let top = editor.view().view_top;
    assert_ne!(top, old_top);
    let source_line = editor.doc(source).buf.line_start(14 - 5);
    assert_eq!(
        editor.source_position(collection, editor.buf().line_start(top)),
        Some((source, source_line))
    );
}
