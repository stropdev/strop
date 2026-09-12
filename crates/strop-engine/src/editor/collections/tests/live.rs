use super::*;

#[test]
fn typing_publishes_each_key_and_keeps_one_source_undo_group() {
    let (mut editor, path, _) = fixture();
    editor.feed(crate::editor::Key::CtrlO);
    let collection = editor.current();
    let source = editor
        .docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&path)).then_some(id))
        .unwrap();
    editor.set_head(current_text(&editor).find("alpha one").unwrap());
    editor.feed_text("iXYZ");
    assert_eq!(
        editor.docs.get(source).unwrap().buf.text().to_string(),
        "XYZalpha one\nalpha two\n"
    );
    assert_eq!(editor.mode, crate::editor::Mode::Insert);
    editor.feed(crate::editor::Key::Esc);
    editor.feed_text("u");
    assert_eq!(
        editor.docs.get(source).unwrap().buf.text().to_string(),
        "alpha one\nalpha two\n"
    );
    editor.feed(crate::editor::Key::CtrlR);
    assert!(editor
        .docs
        .get(source)
        .unwrap()
        .buf
        .text()
        .to_string()
        .starts_with("XYZalpha"));
    assert_eq!(editor.current(), collection);
}

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
            path: path.clone(),
            line: 2,
            col: column,
            match_len: 3,
            line_text: "fn hit() { hit(); }".into(),
        },
    });
    editor.picker_items_fixture(items.to_vec());
    editor.feed(crate::editor::Key::CtrlO);
    assert!(current_text(&editor).contains("2 match(es)"));
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
fn redo_refuses_a_different_history_branch_without_consuming_the_receipt() {
    let (mut editor, path, _) = fixture();
    editor.feed(crate::editor::Key::CtrlO);
    let collection = editor.current();
    let source = editor
        .docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&path)).then_some(id))
        .unwrap();
    editor.set_head(current_text(&editor).find("alpha one").unwrap());
    editor.feed_text("rXu");
    editor.switch_to(source);
    editor.set_head(0);
    editor.feed_text("rY");
    editor.switch_to(collection);
    editor.feed(crate::editor::Key::CtrlR);
    assert!(editor
        .docs
        .get(source)
        .unwrap()
        .buf
        .text()
        .to_string()
        .starts_with("Ylpha"));
    assert_eq!(editor.current(), collection);
    editor.switch_to(source);
    editor.feed_text("u");
    editor.switch_to(collection);
    editor.feed(crate::editor::Key::CtrlR);
    assert!(editor
        .docs
        .get(source)
        .unwrap()
        .buf
        .text()
        .to_string()
        .starts_with("Xlpha"));
}

#[test]
fn closing_a_source_removes_only_its_excerpts_and_keeps_the_view_usable() {
    let (mut editor, path, _) = fixture();
    editor.feed(crate::editor::Key::CtrlO);
    let collection = editor.current();
    let source = editor
        .docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&path)).then_some(id))
        .unwrap();
    editor.switch_to(source);
    assert!(editor.close_buffer(false));
    editor.switch_to(collection);
    assert!(!current_text(&editor).contains("alpha one"));
    assert!(current_text(&editor).contains("beta two"));
    editor.set_head(current_text(&editor).find("beta two").unwrap());
    editor.feed_text("rX");
    assert!(current_text(&editor).contains("Xeta two"));
}

#[test]
fn final_line_without_newline_deletes_and_undoes_without_editing_chrome() {
    let (mut editor, path, _) = fixture();
    let source = editor
        .docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&path)).then_some(id))
        .unwrap();
    editor.replace_system(source, "alpha").unwrap();
    let mut items: Vec<_> = editor
        .picker
        .as_ref()
        .unwrap()
        .picker
        .items
        .iter()
        .cloned()
        .collect();
    if let Payload::Grep { line_text, .. } = &mut items[0].payload {
        *line_text = "alpha".into();
    }
    editor.picker.as_mut().unwrap().picker.clear_items();
    editor.picker_items_fixture(items);
    editor.feed(crate::editor::Key::CtrlO);
    editor.set_head(current_text(&editor).find("alpha").unwrap());
    editor.feed_text("dd");
    assert_eq!(editor.docs.get(source).unwrap().buf.text().to_string(), "");
    assert!(current_text(&editor).contains("beta two"));
    editor.feed_text("xu");
    assert_eq!(
        editor.docs.get(source).unwrap().buf.text().to_string(),
        "alpha"
    );
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
    editor.feed(crate::editor::Key::CtrlO);
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
    editor.feed(crate::editor::Key::Enter);
    assert_eq!(editor.doc(a_id).indent.width, 2);
    assert_eq!(editor.doc(b_id).indent.width, 4);
    assert_eq!(editor.current(), b_id);
}

#[test]
fn grouped_history_preflights_every_sources_write_authority() {
    let (mut editor, a, b) = two_file_fixture();
    editor.feed(crate::editor::Key::CtrlO);
    let source = |editor: &Editor, path: &std::path::Path| {
        editor
            .docs
            .iter()
            .find_map(|(id, doc)| (doc.buf.path.as_deref() == Some(path)).then_some(id))
            .unwrap()
    };
    let (a_id, b_id) = (source(&editor, &a), source(&editor, &b));
    editor.feed_text(":3,6s/one/1/\r");
    editor.doc_mut(b_id).buf.readonly = true;
    editor.feed_text("u");
    assert_eq!(editor.doc(a_id).buf.text(), "alpha 1\n");
    assert_eq!(editor.doc(b_id).buf.text(), "beta 1\n");
    editor.doc_mut(b_id).buf.readonly = false;
    editor.feed_text("u");
    assert_eq!(editor.doc(a_id).buf.text(), "alpha one\n");
    assert_eq!(editor.doc(b_id).buf.text(), "beta one\n");
    editor.doc_mut(b_id).buf.readonly = true;
    editor.feed(crate::editor::Key::CtrlR);
    assert_eq!(editor.doc(a_id).buf.text(), "alpha one\n");
    assert_eq!(editor.doc(b_id).buf.text(), "beta one\n");
    editor.doc_mut(b_id).buf.readonly = false;
    editor.feed(crate::editor::Key::CtrlR);
    assert_eq!(editor.doc(a_id).buf.text(), "alpha 1\n");
    assert_eq!(editor.doc(b_id).buf.text(), "beta 1\n");
}

#[test]
fn long_insert_undo_restores_the_collection_source_position() {
    let (mut editor, a, _) = fixture();
    editor.feed(crate::editor::Key::CtrlO);
    let collection = editor.current();
    let source = editor
        .docs
        .iter()
        .find_map(|(id, doc)| (doc.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    let start = current_text(&editor).find("alpha one").unwrap();
    editor.set_head(start);
    editor.feed_text("i");
    editor.feed_text(&"x".repeat(200));
    editor.feed(crate::editor::Key::Esc);
    editor.feed_text("u");
    assert_eq!(
        editor.source_position(collection, editor.head()),
        Some((source, 0))
    );
    assert_eq!(editor.doc(source).buf.text(), "alpha one\nalpha two\n");
}

#[test]
fn source_edits_preserve_collection_carets_and_jump_anchors() {
    let (mut editor, a, _) = fixture();
    editor.feed(crate::editor::Key::CtrlO);
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

#[test]
fn gap_refresh_keeps_carets_and_history_on_their_own_sources() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(
        &a,
        (0..20)
            .map(|line| format!("alpha {line:02}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(&b, "beta one\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
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
                path: a.clone(),
                line: 1,
                col: 1,
                match_len: 5,
                line_text: "alpha 00".into(),
            },
        },
        Item {
            badge: None,
            text: "a later".into(),
            payload: Payload::Grep {
                path: a.clone(),
                line: 15,
                col: 1,
                match_len: 5,
                line_text: "alpha 14".into(),
            },
        },
        Item {
            badge: None,
            text: "b".into(),
            payload: Payload::Grep {
                path: b,
                line: 1,
                col: 1,
                match_len: 4,
                line_text: "beta one".into(),
            },
        },
    ]);
    editor.feed(crate::editor::Key::CtrlO);
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
