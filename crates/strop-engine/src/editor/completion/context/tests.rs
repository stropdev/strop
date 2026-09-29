use super::*;
use strop_core::Buffer;

fn editing(text: &str, caret: usize) -> Editor {
    let mut editor = Editor::new_in(
        Buffer::from_text(text),
        PathBuf::from("/completion-fixture"),
    );
    editor.feed_text("i");
    editor.set_head(caret);
    editor
}

#[test]
fn source_edits_focus_and_settings_independently_revoke_context() {
    let mut editor = editing("alpha al", 8);
    let context = capture(&editor, Invocation::ManualWords, 4, None).unwrap();
    assert!(context.still_owns(&editor, 4));
    assert!(!context.still_owns(&editor, 5));
    editor.feed_text("p");
    assert!(!context.still_owns(&editor, 4));
    let current = capture(&editor, Invocation::ManualWords, 4, Some(&context.scope)).unwrap();
    assert!(current.still_owns(&editor, 4));
    let document = editor.current();
    editor.split_document(true, document);
    editor.close_pane_or_buffer(true);
    assert!(
        !current.still_owns(&editor, 4),
        "same document and caret do not reuse a closed view"
    );
}

#[test]
fn compatible_multicursors_keep_exact_selection_ownership() {
    let mut editor = editing("al al", 2);
    editor.sels_mut().set_extras([5]);
    let context = capture(&editor, Invocation::ManualWords, 0, None).unwrap();
    assert!(context.still_owns(&editor, 0));
    editor.sels_mut().set_extras([4]);
    assert!(!context.still_owns(&editor, 0));
    assert_eq!(
        capture(&editor, Invocation::ManualWords, 0, None),
        Err(ContextRefusal::MixedPrefixes)
    );
}

#[test]
fn disabled_and_manual_only_admission_do_not_depend_on_callbacks() {
    let mut editor = editing("alpha al", 8);
    let automatic = capture(&editor, Invocation::Automatic, 0, None).unwrap();
    editor.config.completion.auto_popup = false;
    assert!(!automatic.still_owns(&editor, 0));
    assert_eq!(
        capture(&editor, Invocation::Automatic, 0, None),
        Err(ContextRefusal::AutomaticDisabled)
    );
    let manual = capture(&editor, Invocation::ManualCombined, 0, None).unwrap();
    assert!(manual.still_owns(&editor, 0));
    editor.config.completion.enabled = false;
    assert!(!manual.still_owns(&editor, 0));
    assert_eq!(
        capture(&editor, Invocation::ManualCombined, 0, None),
        Err(ContextRefusal::Disabled)
    );
}

#[test]
fn identity_changes_revoke_even_when_bytes_and_revision_match() {
    let mut buffer = Buffer::from_text("alpha al");
    buffer.path = Some(PathBuf::from("/completion-fixture/one.rs"));
    let mut editor = Editor::new_in(buffer, PathBuf::from("/completion-fixture"));
    editor.feed_text("i");
    editor.set_head(8);
    let context = capture(&editor, Invocation::ManualCombined, 0, None).unwrap();
    let source = editor.current();
    editor.docs.get_mut(source).unwrap().buf.path =
        Some(PathBuf::from("/completion-fixture/two.rs"));
    assert!(!context.still_owns(&editor, 0));
    let renamed = capture(&editor, Invocation::ManualCombined, 0, None).unwrap();
    editor.docs.get_mut(source).unwrap().buf.readonly = true;
    assert!(!renamed.still_owns(&editor, 0));
    assert_eq!(
        capture(&editor, Invocation::ManualWords, 0, None),
        Err(ContextRefusal::ReadOnly)
    );
}

#[test]
fn prefix_capture_is_unicode_exact_and_refuses_oversized_tokens() {
    let rope = Rope::from_str("x élan yz");
    assert_eq!(keyword_span(&rope, 5), Ok((2, 7)));
    assert_eq!(keyword_span(&rope, 3), Err(ContextRefusal::InvalidPosition));
    let huge = Rope::from_str(&format!("{} alpha", "x".repeat(1024 * 1024)));
    assert_eq!(
        keyword_span(&huge, 500_000),
        Err(ContextRefusal::KeywordTooLong)
    );
    assert_eq!(
        keyword_span(&huge, huge.len_bytes()),
        Ok((1024 * 1024 + 1, huge.len_bytes()))
    );
}

#[test]
fn collection_completion_uses_real_source_and_rejects_chrome() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.txt");
    std::fs::write(&path, "alpha al\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), directory.path().to_owned());
    editor.open_fixture(&path).unwrap();
    let source = editor.current();
    editor.open_picker(strop_picker::Kind::Search);
    editor.picker_items_fixture(vec![strop_picker::Item {
        badge: None,
        text: "source.txt:1: alpha al".into(),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path),
            line: 1,
            col: 7,
            match_len: 2,
            line_text: "alpha al".into(),
        },
    }]);
    editor.feed(strop_core::frontend_input::Key::CtrlO);
    let view = editor.current();
    let excerpt = editor.collections[&view].excerpts[0].clone();
    editor.set_head(excerpt.view_start + 7);
    editor.feed_text("a");
    let context = capture(&editor, Invocation::ManualWords, 0, None).unwrap();
    assert_eq!(context.primary().unwrap().source, source);
    assert!(context.still_owns(&editor, 0));
    editor.feed_text("p");
    assert!(!context.still_owns(&editor, 0));
    assert_eq!(editor.doc(source).buf.text().to_string(), "alpha alp\n");
    editor.feed(strop_core::frontend_input::Key::Esc);
    editor.set_head(editor.buf().line_start(excerpt.view_line));
    editor.feed_text("i");
    assert!(capture(&editor, Invocation::ManualWords, 0, None).is_err());
}

#[test]
fn namespace_write_policy_and_full_windows_are_independent_of_the_readonly_flag() {
    use crate::editor::document::RemoteDocument;
    use strop_remote::{ReadLimit, ReadSelection, RemoteSize, RemoteWindow};
    let mut editor = editing("alpha al", 8);
    let document = editor.current();
    let file = strop_workspace::RemoteFile::parse("ssh://fixture/work/source.txt").unwrap();
    let permit = serde_json::from_value(serde_json::json!({
        "id": 1, "version": {"file": file, "modified": {"seconds": 1, "nanos": 0},
            "attributes": null, "size": 8, "identity": null, "content": vec![0u8; 32]}
    }))
    .unwrap();
    editor.docs.get_mut(document).unwrap().source =
        DocumentSource::Remote(Box::new(RemoteDocument {
            file,
            window: RemoteWindow::resolve(&ReadSelection::Full, RemoteSize::new(8)),
            selection: ReadSelection::Full,
            connection: None,
            return_to: None,
            write: Some(permit),
        }));
    let context = capture(&editor, Invocation::ManualWords, 0, None).unwrap();
    let DocumentSource::Remote(source) = &mut editor.docs.get_mut(document).unwrap().source else {
        unreachable!()
    };
    source.window = RemoteWindow::resolve(
        &ReadSelection::Tail(ReadLimit::new(8).unwrap()),
        RemoteSize::new(16),
    );
    assert!(
        !context.still_owns(&editor, 0),
        "same bytes in a partial window are not the complete source"
    );
    assert_eq!(
        capture(&editor, Invocation::ManualWords, 0, None),
        Err(ContextRefusal::ReadOnly)
    );
    let DocumentSource::Remote(source) = &mut editor.docs.get_mut(document).unwrap().source else {
        unreachable!()
    };
    source.window = RemoteWindow::resolve(&ReadSelection::Full, RemoteSize::new(8));
    source.file = strop_workspace::RemoteFile::parse("ssh://fixture/work/other.txt").unwrap();
    assert_eq!(
        capture(&editor, Invocation::ManualWords, 0, None),
        Err(ContextRefusal::ReadOnly)
    );
    let DocumentSource::Remote(source) = &mut editor.docs.get_mut(document).unwrap().source else {
        unreachable!()
    };
    source.file = strop_workspace::RemoteFile::parse("ssh://fixture/work/source.txt").unwrap();
    source.write = None;
    assert_eq!(
        capture(&editor, Invocation::ManualWords, 0, None),
        Err(ContextRefusal::ReadOnly)
    );
    editor.docs.get_mut(document).unwrap().source = DocumentSource::Container {
        container: strop_workspace::ContainerId::canonical("a".repeat(64)).unwrap(),
        path: "/source.txt".into(),
    };
    assert_eq!(
        capture(&editor, Invocation::ManualWords, 0, None),
        Err(ContextRefusal::ReadOnly)
    );
}
