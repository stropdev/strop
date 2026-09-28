use super::*;
use strop_picker::{Item, Payload};

#[test]
fn collection_words_come_from_primary_source_and_apply_to_compatible_sources() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("alphabet_header_only.txt");
    let second = directory.path().join("other.txt");
    std::fs::write(&first, "alpha al\n").unwrap();
    std::fs::write(&second, "alien al\n").unwrap();
    let mut editor = Editor::new_in(
        Buffer::from_text("unrelated_alword\n"),
        directory.path().to_owned(),
    );
    editor.open_fixture(&first).unwrap();
    let first_source = editor.current();
    editor.open_fixture(&second).unwrap();
    let second_source = editor.current();
    editor.open_picker(strop_picker::Kind::Search);
    editor.picker_items_fixture(vec![
        Item {
            badge: None,
            text: "alphabet_header_only.txt:1".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(first),
                line: 1,
                col: 7,
                match_len: 2,
                line_text: "alpha al".into(),
            },
        },
        Item {
            badge: None,
            text: "other.txt:1".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(second),
                line: 1,
                col: 7,
                match_len: 2,
                line_text: "alien al".into(),
            },
        },
    ]);
    editor.feed(Key::CtrlO);
    let view = editor.current();
    let excerpts = editor.collections[&view].excerpts.clone();
    let first_at = excerpts
        .iter()
        .find(|excerpt| excerpt.source == first_source)
        .unwrap()
        .view_start
        + 7;
    let second_at = excerpts
        .iter()
        .find(|excerpt| excerpt.source == second_source)
        .unwrap()
        .view_start
        + 7;
    editor.set_head(first_at);
    editor.feed(Key::Char('a'));
    editor.sels_mut().set_extras([second_at + 1]);
    let (tx, events) = events::channel();
    editor.app_tx = Some(tx);
    editor.feed(Key::CtrlN);
    until(&mut editor, &events, selected);
    let words = editor.completion.session.as_ref().unwrap().words().unwrap();
    assert_eq!(
        words
            .items
            .iter()
            .map(|word| word.text.as_ref())
            .collect::<Vec<_>>(),
        ["alpha"]
    );
    editor.feed(Key::CtrlY);
    until(&mut editor, &events, |editor| {
        editor.completion.session.is_none()
    });
    assert_eq!(
        editor.doc(first_source).buf.text().to_string(),
        "alpha alpha\n"
    );
    assert_eq!(
        editor.doc(second_source).buf.text().to_string(),
        "alien alpha\n"
    );
    assert_eq!(
        editor.source_edit_position(view, editor.head()),
        Some((first_source, 11))
    );
    assert_eq!(
        editor.source_edit_position(view, editor.extra_selections()[0].head),
        Some((second_source, 11))
    );
    editor.feed_text("<esc>u");
    assert_eq!(
        editor.doc(first_source).buf.text().to_string(),
        "alpha al\n"
    );
    assert_eq!(
        editor.doc(second_source).buf.text().to_string(),
        "alien al\n"
    );
}

#[test]
fn collection_semantic_acceptance_maps_caret_and_undo_with_an_import_outside_the_excerpt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.txt");
    let original = format!("// import slot\n{}res\n", "padding\n".repeat(20));
    std::fs::write(&path, &original).unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("unrelated"), directory.path().to_owned());
    editor.open_fixture(&path).unwrap();
    let source = editor.current();
    editor.open_picker(strop_picker::Kind::Search);
    editor.picker_items_fixture(vec![Item {
        badge: None,
        text: "source.txt:22".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path),
            line: 22,
            col: 1,
            match_len: 3,
            line_text: "res".into(),
        },
    }]);
    editor.feed(Key::CtrlO);
    let view = editor.current();
    let excerpt = &editor.collections[&view].excerpts[0];
    assert!(
        excerpt.start > 0,
        "the import site is deliberately outside the excerpt"
    );
    let at = excerpt.view_start + original.len() - 2 - excerpt.start;
    editor.set_head(at);
    editor.feed(Key::Char('a'));
    let original_view = editor.buf().text().to_string();
    let delivery = super::language::prepare(&mut editor, super::language::import_item());
    editor.completion_apply_prepared(&delivery);
    let expected = format!("// imported\n{}", original.replace("res\n", "result\n"));
    assert_eq!(editor.doc(source).buf.text().to_string(), expected);
    assert_eq!(
        editor.source_edit_position(view, editor.head()),
        Some((source, expected.len() - 1))
    );
    assert!(editor.buf().text().lines().any(|line| line == "result\n"));
    editor.feed_text("<esc>u");
    assert_eq!(editor.doc(source).buf.text().to_string(), original);
    assert_eq!(editor.buf().text().to_string(), original_view);
}
