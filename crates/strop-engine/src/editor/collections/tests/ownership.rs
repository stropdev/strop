use super::*;
use crate::editor::io::{IoEvent, OpenIntent};

fn hit(path: &std::path::Path, text: &str) -> Item {
    Item {
        badge: None,
        text: path.display().to_string(),
        payload: Payload::Grep {
            path: path.to_path_buf(),
            line: 1,
            col: 1,
            match_len: 3,
            line_text: text.into(),
        },
    }
}

#[test]
fn collecting_a_filtered_location_list_uses_only_its_visible_workset() {
    let (mut editor, _, _) = fixture();
    editor.picker.as_mut().unwrap().picker.kind = Kind::Locations;
    editor.feed_text("a.txt");
    editor.wait_picker();
    editor.feed(crate::editor::Key::CtrlO);
    let text = current_text(&editor);
    assert!(text.contains("alpha one"), "{text}");
    assert!(!text.contains("beta two"), "{text}");
}

#[test]
fn stale_source_load_cannot_complete_a_newer_collection() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.txt");
    let new = dir.path().join("new.txt");
    std::fs::write(&old, "OLD_SOURCE\n").unwrap();
    std::fs::write(&new, "NEW_SOURCE\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_picker(Kind::Grep);
    editor.picker_items_fixture(vec![hit(&old, "OLD_SOURCE")]);
    editor.feed(crate::editor::Key::CtrlO);
    let old_owner = editor.collection_build.as_ref().unwrap().owner;
    editor.open_picker(Kind::Grep);
    editor.picker_items_fixture(vec![hit(&new, "NEW_SOURCE")]);
    editor.feed(crate::editor::Key::CtrlO);
    let mut old_event = None;
    let mut new_event = None;
    for _ in 0..2 {
        let event = editor
            .io
            .rx
            .as_ref()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let stale = matches!(&event, IoEvent::Open(completion)
            if matches!(completion.ticket.key.intent, OpenIntent::CollectionSource { owner } if owner == old_owner));
        if stale {
            old_event = Some(event);
        } else {
            new_event = Some(event);
        }
    }
    editor.handle_io(old_event.unwrap());
    assert!(editor.collections.is_empty());
    assert_eq!(
        editor.docs.len(),
        1,
        "an obsolete read cannot publish a document"
    );
    editor.handle_io(new_event.unwrap());
    let text = current_text(&editor);
    assert!(text.contains("NEW_SOURCE"), "{text}");
    assert!(!text.contains("OLD_SOURCE"), "{text}");
}

#[test]
fn a_collection_finishing_after_typing_does_not_steal_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("late.txt");
    std::fs::write(&path, "LATE_SOURCE\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin\n"), dir.path().to_path_buf());
    let origin = editor.current();
    editor.open_picker(Kind::Grep);
    editor.picker_items_fixture(vec![hit(&path, "LATE_SOURCE")]);
    editor.feed(crate::editor::Key::CtrlO);
    editor.feed_text("iuser <esc>");
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), origin);
    assert_eq!(current_text(&editor), "user origin\n");
    assert_eq!(
        editor.collections.len(),
        1,
        "the requested collection is ready in buffers"
    );
}
