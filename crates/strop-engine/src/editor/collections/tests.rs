//! Collection behavior (0044): build, write-back, refusals, remapping.

use crate::editor::Editor;
use strop_core::Buffer;
use strop_picker::{Item, Kind, Payload};

/// Two open files and a grep picker listing hits in both. The tempdir
/// rides along: tests that read the files back need it alive.
struct LiveFixture {
    editor: Editor,
    _dir: tempfile::TempDir,
    a: std::path::PathBuf,
    b: std::path::PathBuf,
}

fn live_fixture() -> LiveFixture {
    let dir = tempfile::tempdir().unwrap();
    let (editor, a, b) = fixture_in(dir.path());
    LiveFixture {
        editor,
        _dir: dir,
        a,
        b,
    }
}

fn fixture() -> (Editor, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    fixture_in(dir.path())
}

fn fixture_in(root: &std::path::Path) -> (Editor, std::path::PathBuf, std::path::PathBuf) {
    let root = root.to_path_buf();
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
            badge: None,
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
            badge: None,
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
    assert!(text.contains("╭─ a.txt"), "card top: {text}");
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
    let at = header_text.find("a.txt").unwrap();
    e.set_head(at);
    e.feed_text("x");
    assert!(
        e.message.contains("refused"),
        "header edit refused: {}",
        e.message
    );
    assert!(current_text(&e).contains("╭─"), "{}", current_text(&e));
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
fn a_source_edited_elsewhere_refreshes_the_view() {
    // 0049 §5: every source edit invalidates the dependent projection —
    // the view shows the new text immediately, and editing the fresh
    // view writes back (no stale fingerprint refusal).
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
    e.switch_to(collection_id);
    let view = e.buf().text().to_string();
    assert!(view.contains("Xlpha"), "the view refreshed: {view}");
    assert!(!view.contains("alpha one"), "no stale text remains: {view}");
    e.set_head(e.buf().line_start(2));
    e.feed_text("x");
    assert!(
        e.message.contains("applied") || e.message.contains("collection"),
        "the fresh view writes back: {}",
        e.message
    );
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
        badge: None,
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
    // title 1, card-a 2, body-a 3, bottom 4, card-b 5, body-b 6 (0049 §6)
    e.feed_text(":3,6s/one/1/\r");
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
            badge: None,
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
                badge: None,
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
                badge: None,
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

/// 0049 §3's exact witness: a RELATIVE startup path must not skip hits.
#[test]
fn relative_startup_path_collects_all_hits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("a.txt"), "alpha needle one\nkeep a\n").unwrap();
    std::fs::write(root.join("b.txt"), "beta needle two\nkeep b\n").unwrap();
    // the a.txt buffer opened with a RELATIVE spelling (0049 §3 repro)
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.clone());
    e.open_fixture(std::path::Path::new("a.txt")).unwrap();
    e.open_fixture(&root.join("b.txt")).unwrap();
    e.open_picker(Kind::Grep);
    let items = vec![
        Item {
            badge: None,
            text: "a.txt:1".into(),
            payload: Payload::Grep {
                path: root.join("a.txt"),
                line: 1,
                col: 1,
                match_len: 6,
                line_text: "alpha needle one".into(),
            },
        },
        Item {
            badge: None,
            text: "b.txt:1".into(),
            payload: Payload::Grep {
                path: root.join("b.txt"),
                line: 1,
                col: 1,
                match_len: 6,
                line_text: "beta needle two".into(),
            },
        },
    ];
    if let Some(glue) = e.picker.as_mut() {
        glue.picker.append(items);
    }
    e.feed(crate::editor::Key::CtrlO);
    let text = current_text(&e);
    assert!(
        text.contains("alpha needle one"),
        "relative-spelled source: {text}"
    );
    assert!(text.contains("beta needle two"), "{text}");
    assert!(!e.message.contains("skipped"), "{}", e.message);
}

/// 0049 §5: plain `u` in a collection undoes the edit group across
/// every affected source, and the view refreshes.
#[test]
fn collection_undo_restores_sources_and_the_view() {
    // 0049 §5: plain `u` in a collection undoes the collection's own
    // edit group across its actual sources; the view refreshes. Two
    // groups undo newest-first.
    let (mut e, a, b) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let collection = e.current();
    let src = |e: &Editor, path: &std::path::Path| {
        e.docs
            .iter()
            .find(|(_, d)| d.buf.path.as_ref() == Some(&path.to_path_buf()))
            .map(|(id, _)| id)
            .unwrap()
    };
    let (a_id, b_id) = (src(&e, &a), src(&e, &b));
    let text = |e: &Editor, id| e.docs.get(id).unwrap().buf.text().to_string();
    e.set_head(e.buf().line_start(2));
    e.feed_text("rx"); // group 1: a.txt alpha -> xlpha
    e.set_head(e.buf().line_start(5));
    e.feed_text("rx"); // group 2: b.txt beta -> xeta
    assert!(text(&e, a_id).starts_with("xlpha"));
    assert!(text(&e, b_id).contains("xeta two"));
    e.feed_text("u");
    assert!(text(&e, b_id).contains("beta two"), "newest group first");
    assert!(text(&e, a_id).starts_with("xlpha"), "group 1 stands");
    e.feed_text("u");
    assert!(text(&e, a_id).starts_with("alpha"), "second undo: group 1");
    // the view shows the restored text, not the stale one
    e.switch_to(collection);
    let view = current_text(&e);
    assert!(view.contains("alpha one"), "view after undos: {view}");
    assert!(!view.contains("xlpha"), "no stale text: {view}");
    // and the redo stack replays both, newest-first
    e.feed(crate::editor::Key::CtrlR);
    assert!(text(&e, a_id).starts_with("xlpha"), "redo group 1");
    e.feed(crate::editor::Key::CtrlR);
    assert!(text(&e, b_id).contains("xeta two"), "redo group 2");
}

/// 0049 §5: an intervening source edit refuses the group undo by name
/// and keeps the receipt.
#[test]
fn collection_undo_preflight_refuses_a_moved_source() {
    let (mut e, a, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let collection = e.current();
    e.set_head(e.buf().line_start(2));
    e.feed_text("rx");
    let a_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    // an independent edit to the source
    e.switch_to(a_id);
    e.feed_text("Gobackchannel\n");
    e.feed(crate::editor::Key::Esc);
    e.switch_to(collection);
    e.feed_text("u");
    assert!(
        e.message.contains("refused") && e.message.contains("a.txt"),
        "named refusal: {}",
        e.message
    );
    // the receipt survives: undoing the intervening edit first frees it
    e.switch_to(a_id);
    e.feed_text("u"); // undo the backchannel line
    e.switch_to(collection);
    e.feed_text("u");
    assert!(
        e.docs
            .get(a_id)
            .unwrap()
            .buf
            .text()
            .to_string()
            .starts_with("alpha"),
        "the group undo lands after the blocker is undone: {}",
        e.message
    );
}

/// 0049 §5: `:w` saves the dirty sources; `:w PATH` refuses.
#[test]
fn collection_write_saves_dirty_sources_and_refuses_a_target() {
    let live = live_fixture();
    let (mut e, a, b) = (live.editor, live.a.clone(), live.b.clone());
    e.feed(crate::editor::Key::CtrlO);
    e.set_head(e.buf().line_start(2));
    e.feed_text("rx"); // dirty a.txt via the collection
    e.feed_text(":w<cr>");
    e.wait_io().unwrap();
    assert!(
        std::fs::read_to_string(&a).unwrap().starts_with("xlpha"),
        "the source file was written"
    );
    assert!(
        std::fs::read_to_string(&b).unwrap().starts_with("beta"),
        "the untouched source was not"
    );
    e.feed_text(":w /tmp/collection-out.txt<cr>");
    assert!(
        e.message.contains("no file of its own"),
        ":w PATH refuses: {}",
        e.message
    );
    assert!(!std::path::Path::new("/tmp/collection-out.txt").exists());
}

/// 0049 §5: `:q` closes the view with dirty sources intact.
#[test]
fn collection_close_keeps_dirty_sources() {
    let (mut e, a, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    e.set_head(e.buf().line_start(2));
    e.feed_text("rx");
    e.feed_text(":q<cr>");
    assert!(
        e.docs.iter().any(|(_, d)| d.buf.path.as_ref() == Some(&a)),
        "the source buffer is still open"
    );
    let a_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    assert!(
        e.docs.get(a_id).unwrap().buf.dirty,
        "its unsaved edits are intact"
    );
}

/// 0049 §5: g<Space> from a body row lands on the exact source position
/// and Ctrl-O returns to the collection; from a header it opens the file.
#[test]
fn collection_open_source_maps_positions_and_returns() {
    let (mut e, a, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let collection = e.current();
    // body row: line 2 is a.txt's first excerpt body
    e.set_head(e.buf().line_start(2) + 2); // 'p' of alpha
    e.collection_open_source();
    let a_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    assert_eq!(e.current(), a_id, "switched to the live source");
    assert_eq!(e.head(), 2, "exact source byte — unsaved-edit view");
    e.feed(crate::editor::Key::CtrlO);
    assert_eq!(e.current(), collection, "Ctrl-O returns to the collection");
    // header row: line 1 is a.txt's header
    e.set_head(e.buf().line_start(1));
    e.collection_open_source();
    assert_eq!(e.current(), a_id);
    assert_eq!(e.buf().line_of(e.head()), 0, "header opens at the excerpt");
}

/// 0049 §5: Enter on a header opens the source; Enter in a body row
/// keeps its vim motion.
#[test]
fn collection_enter_on_header_only() {
    let (mut e, a, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let a_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    e.set_head(e.buf().line_start(1)); // a.txt header row
    e.feed(crate::editor::Key::Enter);
    assert_eq!(e.current(), a_id, "Enter on a header opens the source");
    e.feed(crate::editor::Key::CtrlO);
    e.set_head(e.buf().line_start(2)); // body row
    e.feed(crate::editor::Key::Enter);
    assert_eq!(
        e.buf().line_of(e.head()),
        3,
        "Enter in a body row moves down"
    );
}

/// 0049 §7.2: occurrence selection in a collection matches excerpt
/// bodies only — the needle in a header row is chrome, never selected.
#[test]
fn occurrence_selection_skips_collection_chrome() {
    let (mut e, _, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    // caret on the header's "txt" (in "a.txt") — seeds "txt"
    let header_start = e.buf().line_start(1);
    let header = e.buf().line_text(1);
    let at_txt = header_start + header.find("txt").unwrap();
    e.set_head(at_txt);
    e.occurrence_all_pub();
    assert!(
        e.message.contains("no") || e.message.contains("0"),
        "headers never match: {}",
        e.message
    );
    assert_eq!(
        e.sels().heads().len(),
        1,
        "no occurrence was selected from chrome"
    );
    // a body needle DOES select across excerpts
    e.set_head(e.buf().line_start(2) + 1);
    e.occurrence_all_pub();
    assert!(
        e.sels().heads().len() > 1 || e.message.contains("occurrence"),
        "body occurrences select: {}",
        e.message
    );
}

/// 0044 v2: hits in unopened files load in the background and the
/// collection assembles when the last one lands.
#[test]
fn collection_loads_unopened_sources_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "alpha zzq one\nkeep a\n").unwrap();
    std::fs::write(&b, "beta zzq two\nkeep b\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    e.open_picker(Kind::Grep);
    if let Some(glue) = e.picker.as_mut() {
        glue.picker.append(vec![
            Item {
                badge: None,
                text: "a.txt:1".into(),
                payload: Payload::Grep {
                    path: a.clone(),
                    line: 1,
                    col: 1,
                    match_len: 3,
                    line_text: "alpha zzq one".into(),
                },
            },
            Item {
                badge: None,
                text: "b.txt:1".into(),
                payload: Payload::Grep {
                    path: b.clone(),
                    line: 1,
                    col: 1,
                    match_len: 3,
                    line_text: "beta zzq two".into(),
                },
            },
        ]);
    }
    e.feed(crate::editor::Key::CtrlO);
    assert!(
        e.message.contains("loading"),
        "the build is pending: {}",
        e.message
    );
    e.wait_io().unwrap();
    let text = e.buf().text().to_string();
    assert!(
        text.contains("alpha zzq one") && text.contains("beta zzq two"),
        "both sources assembled: {text}"
    );
    assert_eq!(e.buf().name.as_deref(), Some("collection: grep"));
}

/// 0049 §6: one card per file — disjoint excerpts of one source share a
/// card, the omitted span between them is a gap row, and gap/card rows
/// are protected chrome (edits refuse, view refreshes).
#[test]
fn one_card_per_file_with_gap_rows() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    std::fs::write(&a, "l1\nl2\nalpha three\nl4\nl5\nalpha six\nl7\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    e.open_fixture(&a).unwrap();
    e.open_picker(Kind::Grep);
    if let Some(glue) = e.picker.as_mut() {
        glue.picker.append(vec![
            Item {
                text: "a.txt:3".into(),
                badge: None,
                payload: Payload::Grep {
                    path: a.clone(),
                    line: 3,
                    col: 1,
                    match_len: 5,
                    line_text: "alpha three".into(),
                },
            },
            Item {
                text: "a.txt:6".into(),
                badge: None,
                payload: Payload::Grep {
                    path: a.clone(),
                    line: 6,
                    col: 1,
                    match_len: 5,
                    line_text: "alpha six".into(),
                },
            },
        ]);
    }
    e.feed(crate::editor::Key::CtrlO);
    let text = current_text(&e);
    assert_eq!(
        text.matches("╭─").count(),
        1,
        "one card for the file: {text}"
    );
    assert!(text.contains("⋮ 2 source lines omitted"), "gap row: {text}");
    // the gap row is protected chrome
    let gap_line = text.lines().position(|l| l.starts_with('⋮')).unwrap();
    e.set_head(e.buf().line_start(gap_line));
    e.feed_text("x");
    assert!(
        e.message.contains("refused"),
        "gap row refuses: {}",
        e.message
    );
    // and the bottom border closes the card
    assert!(text.contains('╰'), "card closes: {text}");
}

/// 0049 §5: ]f/[f walk file cards.
#[test]
fn file_card_navigation_steps_between_cards() {
    let (mut e, _, _) = fixture();
    e.feed(crate::editor::Key::CtrlO);
    let tops: Vec<usize> = e
        .collections
        .get(&e.current())
        .unwrap()
        .rows
        .iter()
        .enumerate()
        .filter_map(|(row, kind)| {
            matches!(kind, crate::editor::CollectionRow::CardTop(_)).then_some(row)
        })
        .collect();
    assert_eq!(tops.len(), 2, "two file cards");
    e.set_head(e.buf().line_start(tops[0] + 1)); // inside card A
    e.collection_file_step(true);
    assert_eq!(e.buf().line_of(e.head()), tops[1], "next card");
    e.collection_file_step(true);
    assert_eq!(e.message, "last card");
    e.collection_file_step(false);
    assert_eq!(e.buf().line_of(e.head()), tops[0], "previous card");
}
