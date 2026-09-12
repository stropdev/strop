use super::*;
use strop_core::Buffer;

fn fixture() -> (tempfile::TempDir, Editor) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "needle needle\nsecond needle\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "needle other\n").unwrap();
    let editor = Editor::new_in(Buffer::from_text("origin\n"), root.path().to_path_buf());
    (root, editor)
}

#[test]
fn toggling_a_live_query_keeps_its_producer_and_parked_editors() {
    let (_root, mut editor) = fixture();
    editor.open_search(false);
    editor.paste_bracketed("needle");
    let owner = editor.picker.as_ref().unwrap().active.clone().unwrap();
    editor.feed(Key::CtrlR);
    editor.paste_bracketed("literal $1");
    editor.feed(Key::Left);
    editor.feed(Key::Esc);
    editor.feed(Key::CtrlR);
    editor.feed(Key::CtrlR);
    let glue = editor.picker.as_ref().unwrap();
    assert_eq!(
        glue.active.as_ref(),
        Some(&owner),
        "presentation never restarts a producer"
    );
    assert_eq!(glue.picker.input.text, "needle");
    assert_eq!(glue.picker.replace_input.text, "literal $1");
    assert_eq!(glue.picker.replace_input.cursor, 9);
    assert!(glue.picker.replace_input.normal);
    editor.wait_picker();
    assert_eq!(editor.picker.as_ref().unwrap().picker.accepted().count(), 4);
}

#[test]
fn find_enter_opens_sources_in_both_presentations_and_resume_keeps_scope() {
    for replacement in [false, true] {
        let (root, mut editor) = fixture();
        editor.open_search(replacement);
        editor.paste_bracketed("needle");
        editor.wait_picker();
        editor.feed(Key::Down);
        editor.feed(Key::CtrlX);
        let chosen = editor
            .picker
            .as_ref()
            .unwrap()
            .picker
            .current()
            .unwrap()
            .payload
            .clone();
        let Payload::Grep {
            path, line, col, ..
        } = chosen
        else {
            panic!("source hit")
        };
        editor.feed(Key::Enter);
        editor.wait_io().unwrap();
        assert!(!editor.picker_open());
        assert!(editor
            .doc(editor.current())
            .matches_target(&crate::files::FileTarget::Local(root.path().join(&path))));
        assert_eq!(editor.head(), editor.buf().line_start(line - 1) + col - 1);
        assert!(!editor.buf().dirty);
        let unrelated = tempfile::tempdir().unwrap();
        editor.cwd = unrelated.path().to_path_buf();
        editor.open_search(false);
        editor.wait_picker();
        assert_eq!(editor.search_scope().unwrap().root.path, root.path());
        let picker = &editor.picker.as_ref().unwrap().picker;
        assert_eq!(picker.input.text, "needle");
        let Payload::Grep {
            path: current_path,
            line: current_line,
            col: current_col,
            ..
        } = &picker.current().unwrap().payload
        else {
            panic!("retained source hit")
        };
        assert_eq!(
            (current_path, *current_line, *current_col),
            (&path, line, col)
        );
        assert_eq!(picker.excluded_count(), 1);
        assert_eq!(picker.replacement_visible, replacement);
    }
}

#[test]
fn included_same_line_matches_drive_both_collect_and_review() {
    let (root, mut editor) = fixture();
    editor.open_search(false);
    editor.paste_bracketed("path:a.txt needle");
    editor.wait_picker();
    editor.feed(Key::CtrlX); // first match only, not its same-line sibling
    editor.feed(Key::CtrlD); // whole file masks the row decision
    editor.feed(Key::CtrlO);
    assert!(editor.picker_open());
    assert!(editor.collections.is_empty());
    editor.feed(Key::CtrlR);
    editor.paste_bracketed("new");
    editor.feed(Key::Enter);
    assert!(editor.picker_open());
    assert!(!editor.io_pending());
    editor.feed(Key::CtrlD); // restoring file retains the first-match exclusion
    assert_eq!(editor.picker.as_ref().unwrap().picker.accepted().count(), 2);
    editor.feed(Key::CtrlR); // collecting with replacement hidden sees the same workset
    editor.feed(Key::CtrlO);
    editor.wait_io().unwrap();
    assert_eq!(
        editor
            .collections
            .get(&editor.current())
            .unwrap()
            .match_count,
        2
    );
    editor.open_search(true);
    editor.wait_picker();
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    editor.review_apply_pub();
    let source = editor
        .docs
        .iter()
        .find(|(_, doc)| doc.buf.path.as_deref() == Some(root.path().join("a.txt").as_path()))
        .unwrap()
        .1;
    assert_eq!(source.buf.text(), "needle new\nsecond new\n");
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "needle needle\nsecond needle\n"
    );
}

#[test]
fn refreshed_source_witness_does_not_transfer_an_exclusion_to_new_text() {
    let (root, mut editor) = fixture();
    editor.open_search(false);
    editor.paste_bracketed("path:a.txt needle");
    editor.wait_picker();
    editor.feed(Key::CtrlX);
    editor.close_picker();
    std::fs::write(root.path().join("a.txt"), "needle CHANGED\nsecond needle\n").unwrap();
    editor.open_search(false);
    editor.wait_picker();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert_eq!(picker.accepted().count(), 2);
    assert_eq!(picker.excluded_count(), 0);
    assert!(picker.warning.is_some());
}

#[test]
fn obsolete_review_completion_cannot_publish_after_search_reentry() {
    let (_root, mut editor) = fixture();
    editor.open_search(true);
    editor.paste_bracketed("needle");
    editor.wait_picker();
    editor.feed(Key::Tab);
    editor.paste_bracketed("new");
    editor.feed(Key::Enter);
    let completed = editor
        .io
        .rx
        .as_ref()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    editor.open_search(false);
    editor.handle_io(completed);
    editor.wait_picker();
    assert!(editor.picker_open());
    assert!(editor
        .docs
        .iter()
        .all(|(_, doc)| doc.buf.name.as_deref() != Some("change proposal 1")));
    editor.review_apply_pub();
    assert!(editor.docs.iter().all(|(_, doc)| !doc.buf.dirty));
}

#[test]
fn late_review_does_not_steal_typing_and_stale_collect_refuses() {
    let (root, mut editor) = fixture();
    let origin = editor.current();
    editor.open_search(true);
    editor.paste_bracketed("needle");
    editor.wait_picker();
    editor.feed(Key::Tab);
    editor.paste_bracketed("new");
    editor.feed(Key::Enter);
    editor.feed_text("Ityping <esc>");
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), origin);
    assert_eq!(editor.buf().text(), "typing origin\n");
    assert!(editor
        .docs
        .iter()
        .any(|(_, doc)| doc.buf.name.as_deref() == Some("change proposal 1")));
    editor.review_cancel_pub();
    editor.wait_picker();
    editor.close_picker();
    // A fresh unopened-only query followed by a changed source must not collect old coordinates.
    let mut editor = Editor::new_in(Buffer::from_text("origin\n"), root.path().to_path_buf());
    editor.open_search(false);
    editor.paste_bracketed("path:b.txt needle");
    editor.wait_picker();
    std::fs::write(root.path().join("b.txt"), "not the old match\n").unwrap();
    editor.feed(Key::CtrlO);
    editor.wait_io().unwrap();
    assert!(editor.collections.is_empty());
    assert_eq!(editor.buf().text(), "origin\n");
}

#[test]
fn stale_open_cannot_retire_the_original_pristine_scratch() {
    let (root, _) = fixture();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    let origin = editor.current();
    editor.open_search(false);
    editor.paste_bracketed("path:b.txt needle");
    editor.wait_picker();
    std::fs::write(root.path().join("b.txt"), "changed\n").unwrap();
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), origin);
    assert_eq!(editor.docs.len(), 1);
    assert_eq!(editor.buf().text(), "");
    assert!(!editor.message.is_empty());
}

#[test]
fn live_preview_witness_cache_rechecks_each_source_revision() {
    let (root, mut editor) = fixture();
    let source = editor.open_fixture(&root.path().join("b.txt")).unwrap();
    editor.open_search(false);
    editor.paste_bracketed("path:b.txt needle");
    editor.wait_picker();
    let preview = PreviewSource::Buffer(source);
    assert!(editor.picker_preview_range(&preview).unwrap().is_some());
    editor.replace_system(source, "changed\n").unwrap();
    assert!(editor.picker_preview_range(&preview).is_err());
    editor.replace_system(source, "needle other\n").unwrap();
    assert!(editor.picker_preview_range(&preview).unwrap().is_some());
}

#[cfg(unix)]
#[test]
fn canonical_aliases_prepare_only_one_edit_plan_for_the_source() {
    let (root, mut editor) = fixture();
    editor.open_search(true);
    editor.paste_bracketed("path:a.txt needle");
    editor.wait_picker();
    std::os::unix::fs::symlink(root.path().join("a.txt"), root.path().join("alias.txt")).unwrap();
    editor.picker_items_fixture(vec![Item {
        badge: None,
        text: "alias".into(),
        payload: Payload::Grep {
            path: "alias.txt".into(),
            line: 1,
            col: 1,
            match_len: 6,
            line_text: "needle needle".into(),
        },
    }]);
    editor.feed(Key::Tab);
    editor.paste_bracketed("new");
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    editor.review_apply_pub();
    let source = editor
        .docs
        .iter()
        .find(|(_, doc)| doc.buf.path.as_deref() == Some(root.path().join("a.txt").as_path()))
        .unwrap()
        .1;
    assert_eq!(source.buf.text(), "new new\nsecond new\n");
}

#[test]
fn save_as_cannot_retarget_a_prepared_search_plan() {
    let (root, mut editor) = fixture();
    let source = editor.open_fixture(&root.path().join("b.txt")).unwrap();
    editor.open_search(true);
    editor.paste_bracketed("path:b.txt needle");
    editor.wait_picker();
    editor.feed(Key::Tab);
    editor.paste_bracketed("new");
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    editor.switch_to(source);
    let renamed = root.path().join("renamed.txt");
    editor.request_save(Some(renamed.clone()), false, false);
    editor.wait_io().unwrap();
    editor.review_apply_pub();
    assert_eq!(editor.doc(source).buf.text(), "needle other\n");
    assert_eq!(std::fs::read_to_string(renamed).unwrap(), "needle other\n");
    assert_eq!(
        std::fs::read_to_string(root.path().join("b.txt")).unwrap(),
        "needle other\n"
    );
}

#[test]
fn dirty_source_identity_survives_cwd_and_relative_display_spelling() {
    let (root, mut editor) = fixture();
    let source = editor.open_fixture(&root.path().join("a.txt")).unwrap();
    editor.feed_text("Iunsaved <esc>");
    editor.doc_mut(source).buf.path = Some("a.txt".into());
    editor.open_search(false);
    editor.paste_bracketed("unsaved");
    editor.wait_picker();
    assert_eq!(editor.picker.as_ref().unwrap().picker.accepted().count(), 1);
    editor.close_picker();
    let other = tempfile::tempdir().unwrap();
    editor.cwd = other.path().to_path_buf();
    editor.open_search(false);
    editor.wait_picker();
    let hit = &editor
        .picker
        .as_ref()
        .unwrap()
        .picker
        .current()
        .unwrap()
        .payload;
    assert!(matches!(hit, Payload::Grep { line_text, .. } if line_text.starts_with("unsaved ")));
}
