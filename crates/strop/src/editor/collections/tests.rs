//! Collection behavior (0044): build, write-back, refusals, remapping.

use crate::editor::Editor;
use strop_core::Buffer;
use strop_picker::{Item, Kind, Payload};

/// Two open files and a grep picker listing hits in both.
fn fixture() -> (Editor, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "alpha one\nalpha two\n").unwrap();
    std::fs::write(&b, "beta one\nbeta two\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.clone());
    e.open_fixture(&a).unwrap();
    e.open_fixture(&b).unwrap();
    e.open_picker(Kind::Grep);
    let items = vec![
        Item {
            text: "a.txt:1: alpha one".into(),
            payload: Payload::Grep {
                path: a.clone(),
                line: 1,
                col: 1,
                match_len: 5,
                line_text: "alpha one".into(),
            },
        },
        Item {
            text: "b.txt:2: beta two".into(),
            payload: Payload::Grep {
                path: b.clone(),
                line: 2,
                col: 1,
                match_len: 4,
                line_text: "beta two".into(),
            },
        },
    ];
    if let Some(glue) = e.picker.as_mut() {
        glue.picker.append(items);
    }
    (e, a, b)
}

fn current_text(e: &Editor) -> String {
    e.buf().text().to_string()
}

#[test]
fn collection_builds_from_picker_hits() {
    let (mut e, _, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let text = current_text(&e);
    assert!(text.contains("alpha one"), "{text}");
    assert!(text.contains("beta two"), "{text}");
    assert!(text.contains("a.txt:1"), "{text}");
    assert_eq!(e.message, "collection: 2 excerpt(s)");
}

#[test]
fn editing_an_excerpt_writes_back_to_the_source() {
    let (mut e, a, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let body_line = e.buf().line_start(2);
    e.set_head(body_line + 8); // on the 'e' of "one"
    e.feed_text("x"); // delete the 'e' of "one"
    assert_eq!(e.message, "collection edit: applied to 1 buffer(s)");
    // the collection view regenerated from the source
    assert!(
        current_text(&e).contains("alpha on\n"),
        "{}",
        current_text(&e)
    );
    // and the source document itself carries the edit
    let source = e
        .docs
        .iter()
        .find(|(_, d)| d.buf.path.as_ref() == Some(&a))
        .map(|(_, d)| d.buf.text().to_string())
        .unwrap();
    assert_eq!(source, "alpha on\nalpha two\n");
}

#[test]
fn editing_a_header_is_refused_and_the_view_refreshes() {
    let (mut e, _, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    // land on the 'a' of "a.txt" inside the header line
    let header_text = e.buf().text().to_string();
    let at = header_text.find("a.txt:1").unwrap();
    e.set_head(at);
    e.feed_text("x");
    assert!(
        e.message.contains("refused"),
        "header edit refused: {}",
        e.message
    );
    assert!(current_text(&e).contains("──"), "{}", current_text(&e));
}

#[test]
fn an_edit_spanning_excerpts_is_refused() {
    let (mut e, _, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    // delete from the first excerpt's body through the second's header
    e.set_head(e.buf().line_start(2));
    e.feed_text("Vjd");
    assert!(e.message.contains("refused"), "{}", e.message);
}

#[test]
fn a_source_edited_elsewhere_refuses_the_write_back() {
    let (mut e, a, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let collection_id = e.current();
    // edit the source directly in its own buffer
    let source_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    e.switch_to(source_id);
    e.feed_text("0rX"); // alpha -> Xlpha
                        // back to the collection; an edit there must refuse (stale fingerprint)
    e.switch_to(collection_id);
    e.set_head(e.buf().line_start(2));
    e.feed_text("x");
    assert!(e.message.contains("changed elsewhere"), "{}", e.message);
}

#[test]
fn anchors_remap_when_the_source_grows() {
    let (mut e, a, b) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let collection_id = e.current();
    // grow a.txt at the top: the excerpt anchor shifts, sync stays true
    let source_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    e.switch_to(source_id);
    e.feed_text("Oinserted first<esc>"); // new first line
    e.switch_to(collection_id);
    // edit inside the b.txt excerpt's body line
    let at = e.buf().text().to_string().find("beta two").unwrap() + 7;
    e.set_head(at);
    e.feed_text("x");
    assert_eq!(e.message, "collection edit: applied to 1 buffer(s)");
    let source = e
        .docs
        .iter()
        .find(|(_, d)| d.buf.path.as_ref() == Some(&b))
        .map(|(_, d)| d.buf.text().to_string())
        .unwrap();
    assert_eq!(source, "beta one\nbeta tw\n");
}

/// Hits on a.txt:1 and b.txt:1 (both contain "one") — the multi-region
/// fixture: one ranged substitution touches both excerpts in one commit.
fn two_file_fixture() -> (Editor, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "alpha one\n").unwrap();
    std::fs::write(&b, "beta one\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.clone());
    e.open_fixture(&a).unwrap();
    e.open_fixture(&b).unwrap();
    e.open_picker(Kind::Grep);
    let item = |path: &std::path::Path| Item {
        text: "hit".into(),
        payload: Payload::Grep {
            path: path.to_path_buf(),
            line: 1,
            col: 1,
            match_len: 3,
            line_text: "x one".into(),
        },
    };
    if let Some(glue) = e.picker.as_mut() {
        glue.picker.append(vec![item(&a), item(&b)]);
    }
    (e, a, b)
}

#[test]
fn one_commit_across_excerpts_writes_back_to_both_sources() {
    let (mut e, a, b) = two_file_fixture();
    e.feed(crate::editor::Key::CtrlO);
    // title 1, header-a 2, body-a 3, header-b 4, body-b 5 (1-based)
    e.feed_text(":3,5s/one/1/\r");
    assert_eq!(e.message, "collection edit: applied to 2 buffer(s)");
    for (path, want) in [(&a, "alpha 1\n"), (&b, "beta 1\n")] {
        let text = e
            .docs
            .iter()
            .find(|(_, d)| d.buf.path.as_ref() == Some(path))
            .map(|(_, d)| d.buf.text().to_string())
            .unwrap();
        assert_eq!(text, want);
    }
}

#[test]
fn inserting_a_line_inside_a_body_writes_the_wider_span_back() {
    let (mut e, a, _b) = two_file_fixture();
    e.feed(crate::editor::Key::CtrlO);
    let body = e.buf().text().to_string().find("alpha one").unwrap();
    e.set_head(body);
    e.feed_text("Oinserted first<esc>");
    assert_eq!(e.message, "collection edit: applied to 1 buffer(s)");
    let text = e
        .docs
        .iter()
        .find(|(_, d)| d.buf.path.as_ref() == Some(&a))
        .map(|(_, d)| d.buf.text().to_string())
        .unwrap();
    assert_eq!(text, "inserted first\nalpha one\n");
}

#[test]
fn inserting_between_excerpts_is_refused() {
    let (mut e, _a, _b) = two_file_fixture();
    e.feed(crate::editor::Key::CtrlO);
    // O on the b header line inserts between the excerpts — structure
    let header = e.buf().text().to_string().find("b.txt").unwrap();
    e.set_head(header);
    e.feed_text("Onope<esc>");
    assert!(e.message.contains("refused"), "{}", e.message);
}

#[test]
fn unopened_sources_load_in_the_background_and_assemble() {
    // 0044 v2: hits on files that are not open load as real buffers,
    // focus never moves, and the collection assembles when they land
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let a = root.join("late-a.txt");
    std::fs::write(&a, "late one\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.clone());
    e.open_picker(Kind::Grep);
    if let Some(glue) = e.picker.as_mut() {
        glue.picker.append(vec![Item {
            text: "hit".into(),
            payload: Payload::Grep {
                path: a.clone(),
                line: 1,
                col: 1,
                match_len: 3,
                line_text: "late one".into(),
            },
        }]);
    }
    e.feed(crate::editor::Key::CtrlO);
    assert!(e.message.contains("loading"), "{}", e.message);
    assert!(e.collections.is_empty(), "not yet");
    e.wait_io().unwrap();
    assert!(e.collections.len() == 1, "assembled after the load");
    assert!(e.buf().text().to_string().contains("late one"));
    let text = e.buf().text().to_string();
    assert!(text.contains("late one"), "{text}");
}

#[test]
fn remote_sources_join_collections_and_refuse_without_a_permit() {
    // 0044 v2 + 0040: a remote hit on an open remote document lands in
    // the collection; without :remote edit the write-back refuses and
    // names the way out
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let a = root.join("a.txt");
    std::fs::write(&a, "alpha one\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.clone());
    e.open_fixture(&a).unwrap();
    let remote_text = "remote one\n";
    let remote_doc = crate::editor::document::Document::remote(
        Buffer::from_text(remote_text),
        crate::editor::document::RemoteDocument {
            file: strop_workspace::RemoteFile::parse("ssh://fixture/repo/app.log").unwrap(),
            window: strop_remote::RemoteWindow::resolve(
                &strop_remote::ReadSelection::Full,
                strop_remote::RemoteSize::new(remote_text.len() as u64),
            ),
            selection: strop_remote::ReadSelection::Full,
            connection: None,
            return_to: None,
            write: None,
        },
    );
    e.docs.insert(remote_doc);
    e.open_picker(Kind::Grep);
    if let Some(glue) = e.picker.as_mut() {
        glue.picker.append(vec![
            Item {
                text: "local".into(),
                payload: Payload::Grep {
                    path: a.clone(),
                    line: 1,
                    col: 1,
                    match_len: 3,
                    line_text: "alpha one".into(),
                },
            },
            Item {
                text: "remote".into(),
                payload: Payload::Remote {
                    endpoint: strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
                    path: "/repo/app.log".into(),
                    line: 1,
                    col: 1,
                },
            },
        ]);
    }
    e.feed(crate::editor::Key::CtrlO);
    let text = e.buf().text().to_string();
    assert!(text.contains("alpha one"), "{text}");
    assert!(
        text.contains("remote one"),
        "remote excerpt in view: {text}"
    );
    // edit the remote excerpt's line (the last body line)
    let at = e.buf().text().to_string().find("remote one").unwrap() + 7;
    e.set_head(at);
    e.feed_text("x");
    assert!(
        e.message.contains("read-only") && e.message.contains(":remote edit"),
        "{}",
        e.message
    );
}
