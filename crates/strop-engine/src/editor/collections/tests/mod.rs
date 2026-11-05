//! Collection behavior: readonly inspection, source refreshes and navigation.

use crate::editor::Editor;
use strop_core::Buffer;
use strop_picker::{Item, Kind, Payload};
mod live;
mod ownership;

fn fixture() -> (Editor, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap().keep();
    fixture_in(&dir)
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
    e.open_picker(Kind::Search);
    let items = vec![
        Item {
            badge: None,
            text: "a.txt:1: alpha one".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(a.clone()),
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
                location: strop_workspace::ResourceLocation::local(b.clone()),
                line: 2,
                col: 1,
                match_len: 4,
                line_text: "beta two".into(),
            },
        },
    ];
    e.picker_items_fixture(items);
    (e, a, b)
}

fn current_text(e: &Editor) -> String {
    e.buf().text().to_string()
}

#[test]
fn collection_builds_from_picker_hits() {
    let (mut e, _, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    assert!(e.buf().readonly);
    let text = current_text(&e);
    assert!(text.contains("alpha one"), "{text}");
    assert!(text.contains("beta two"), "{text}");
    assert!(text.contains("╭─ a.txt"), "card top: {text}");
    assert_eq!(
        e.buf().readonly_reason,
        Some(strop_core::ReadonlyReason::Collection)
    );
    e.open_explain();
    let explanation = current_text(&e);
    assert!(
        explanation.contains("source-backed collection — read-only inspection"),
        "{explanation}"
    );
    assert!(!explanation.contains("projection failed"), "{explanation}");
}

/// 0056 AR14: a stale collection explains its readonly reason from the
/// failed projection — never a generic hint.
#[test]
fn stale_collection_explains_its_readonly_source() {
    let (mut e, _, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = e.current();
    e.invalidate_collection_projection(collection, strop_core::EditError::InvalidRange);
    assert_eq!(
        e.buf().readonly_reason,
        Some(strop_core::ReadonlyReason::CollectionProjection)
    );
    e.open_explain();
    let text = current_text(&e);
    assert!(
        text.contains("collection projection failed — this stale view refuses edits"),
        "{text}"
    );
}

#[test]
fn collection_header_refuses_mutation() {
    let (mut e, _, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let before = current_text(&e);
    let at = before.find("a.txt").unwrap();
    e.set_head(at);
    e.feed_text("x");
    assert!(e.message.contains("read-only"), "{}", e.message);
    assert!(e.message.contains("g<Space>"), "{}", e.message);
    assert_eq!(current_text(&e), before);
}

#[test]
fn a_source_edited_elsewhere_refreshes_the_view() {
    let (mut e, a, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
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
}

#[test]
fn stale_search_witness_is_not_projected() {
    let (mut e, a, _) = fixture();
    let source = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    e.replace_system(source, "changed\n").unwrap();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let view = current_text(&e);
    assert!(view.contains("beta two"), "{view}");
    assert!(!view.contains("alpha one"), "stale hit projected: {view}");
    assert!(e.message.contains("1 hit(s) skipped"), "{}", e.message);
}

#[test]
fn anchors_remap_when_the_source_grows() {
    let (mut e, a, b) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let collection_id = e.current();
    let source_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    let other_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&b)).then_some(id))
        .unwrap();
    e.switch_to(source_id);
    e.feed_text("Oinserted first<esc>");
    e.switch_to(collection_id);
    assert!(current_text(&e).contains("inserted first"));
    let at = current_text(&e).find("beta two").unwrap() + 7;
    e.set_head(at);
    assert_eq!(
        e.source_position(collection_id, at),
        Some((other_id, e.doc(other_id).buf.line_start(1) + 7))
    );
    assert_eq!(e.doc(other_id).buf.text(), "beta one\nbeta two\n");
}

/// Hits on a.txt:1 and b.txt:1 provide two source-backed excerpts.
fn two_file_fixture() -> (Editor, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap().keep();
    let root = dir;
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, "alpha one\n").unwrap();
    std::fs::write(&b, "beta one\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.clone());
    e.open_fixture(&a).unwrap();
    e.open_fixture(&b).unwrap();
    e.open_picker(Kind::Search);
    let item = |path: &std::path::Path, witness: &str| Item {
        badge: None,
        text: "hit".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path.to_path_buf()),
            line: 1,
            col: 1,
            match_len: 3,
            line_text: witness.into(),
        },
    };
    e.picker_items_fixture(vec![item(&a, "alpha one"), item(&b, "beta one")]);
    (e, a, b)
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
    e.open_picker(Kind::Search);
    e.picker_items_fixture(vec![Item {
        badge: None,
        text: "hit".into(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(a.clone()),
            line: 1,
            col: 1,
            match_len: 3,
            line_text: "late one".into(),
        },
    }]);
    e.feed(strop_core::frontend_input::Key::CtrlO);
    assert!(e.message.contains("loading"), "{}", e.message);
    assert!(e.collections.is_empty(), "not yet");
    e.wait_io().unwrap();
    assert!(e.collections.len() == 1, "assembled after the load");
    assert!(e.buf().text().to_string().contains("late one"));
    let text = e.buf().text().to_string();
    assert!(text.contains("late one"), "{text}");
}

#[test]
fn remote_sources_join_readonly_collections() {
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
    e.docs.try_insert(remote_doc).unwrap();
    e.open_picker(Kind::Search);
    e.picker_items_fixture(vec![
        Item {
            badge: None,
            text: "local".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(a.clone()),
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
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let text = e.buf().text().to_string();
    assert!(text.contains("alpha one"), "{text}");
    assert!(
        text.contains("remote one"),
        "remote excerpt in view: {text}"
    );
    assert!(e.buf().readonly);
    let before = e.doc(e.current()).buf.text().to_string();
    let at = before.find("remote one").unwrap() + 7;
    e.set_head(at);
    e.feed_text("x");
    assert!(e.message.contains("read-only"), "{}", e.message);
    assert_eq!(current_text(&e), before);
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
    e.open_picker(Kind::Search);
    let items = vec![
        Item {
            badge: None,
            text: "a.txt:1".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(root.join("a.txt")),
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
                location: strop_workspace::ResourceLocation::local(root.join("b.txt")),
                line: 1,
                col: 1,
                match_len: 6,
                line_text: "beta needle two".into(),
            },
        },
    ];
    e.picker_items_fixture(items);
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let text = current_text(&e);
    assert!(
        text.contains("alpha needle one"),
        "relative-spelled source: {text}"
    );
    assert!(text.contains("beta needle two"), "{text}");
    assert!(!e.message.contains("skipped"), "{}", e.message);
}

#[test]
fn collection_mutations_refuse_without_touching_sources() {
    let (mut e, a, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let source = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    let view = current_text(&e);
    let source_text = e.doc(source).buf.text().to_string();
    e.set_head(e.buf().line_start(2));
    for key in ["i", "x", "u"] {
        e.feed_text(key);
        assert!(e.message.contains("read-only"), "{key}: {}", e.message);
        assert!(e.message.contains("g<Space>"), "{key}: {}", e.message);
        assert_eq!(current_text(&e), view, "{key} changed the view");
        assert_eq!(
            e.doc(source).buf.text().to_string(),
            source_text,
            "{key} changed source"
        );
        assert_eq!(e.mode, crate::editor::Mode::Normal);
    }
    e.feed(strop_core::frontend_input::Key::CtrlR);
    assert!(e.message.contains("read-only"));
    assert_eq!(current_text(&e), view);
    assert_eq!(e.doc(source).buf.text().to_string(), source_text);
    e.feed_text(":set noro\r");
    assert!(e.buf().readonly, ":set noro must not unlock a collection");
    assert_eq!(current_text(&e), view);
}

#[test]
fn collection_write_commands_never_save_dirty_sources() {
    let (mut e, a, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = e.current();
    let source = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    e.switch_to(source);
    e.feed_text("0rX");
    e.switch_to(collection);
    for command in [":w\r", ":w!\r", ":wq\r", ":wq!\r"] {
        e.feed_text(command);
        assert_eq!(e.current(), collection, "{command} closed the view");
        assert!(e.doc(source).buf.dirty, "{command} saved the source");
        assert!(a.exists(), "{command}: source file is missing");
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "alpha one\nalpha two\n"
        );
        assert!(e.message.contains("read-only"), "{command}: {}", e.message);
    }
}

#[test]
fn collection_close_keeps_dirty_sources() {
    let (mut e, a, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = e.current();
    let source = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    e.switch_to(source);
    e.feed_text("0rX");
    e.switch_to(collection);
    e.feed_text(":q\r");
    assert!(e.docs.get(collection).is_none());
    assert_eq!(e.doc(source).buf.text(), "Xlpha one\nalpha two\n");
    assert!(e.doc(source).buf.dirty, "unsaved source edits are intact");
}

/// 0049 §5: g<Space> from a body row lands on the exact source position
/// and Ctrl-O returns to the collection; from a header it opens the file.
#[test]
fn collection_open_source_maps_positions_and_returns() {
    let (mut e, a, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = e.current();
    // body row: line 2 is a.txt's first excerpt body
    e.set_head(e.buf().line_start(2) + 2); // 'p' of alpha
    e.feed_text("g ");
    let a_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    assert_eq!(e.current(), a_id, "switched to the live source");
    assert_eq!(e.head(), 2, "exact source byte — unsaved-edit view");
    e.feed(strop_core::frontend_input::Key::CtrlO);
    assert_eq!(e.current(), collection, "Ctrl-O returns to the collection");
    // header row: line 1 is a.txt's header
    e.set_head(e.buf().line_start(1));
    e.feed_text("g ");
    assert_eq!(e.current(), a_id);
    assert_eq!(e.buf().line_of(e.head()), 0, "header opens at the excerpt");
}

/// 0049 §5: Enter on a header opens the source; Enter in a body row
/// keeps its vim motion.
#[test]
fn collection_enter_on_header_only() {
    let (mut e, a, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let a_id = e
        .docs
        .iter()
        .find_map(|(id, d)| (d.buf.path.as_ref() == Some(&a)).then_some(id))
        .unwrap();
    e.set_head(e.buf().line_start(1)); // a.txt header row
    e.feed(strop_core::frontend_input::Key::Enter);
    assert_eq!(e.current(), a_id, "Enter on a header opens the source");
    e.feed(strop_core::frontend_input::Key::CtrlO);
    e.set_head(e.buf().line_start(2)); // body row
    e.feed(strop_core::frontend_input::Key::Enter);
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
    e.feed(strop_core::frontend_input::Key::CtrlO);
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
    e.open_picker(Kind::Search);
    e.picker_items_fixture(vec![
        Item {
            badge: None,
            text: "a.txt:1".into(),
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(a.clone()),
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
                location: strop_workspace::ResourceLocation::local(b.clone()),
                line: 1,
                col: 1,
                match_len: 3,
                line_text: "beta zzq two".into(),
            },
        },
    ]);
    e.feed(strop_core::frontend_input::Key::CtrlO);
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
}

/// 0049 §6: one card per file — disjoint excerpts of one source share a
/// card, the omitted span between them is a gap row, and gap/card rows
/// are protected chrome (edits refuse, view refreshes).
#[test]
fn one_card_per_file_with_gap_rows() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    std::fs::write(
        &a,
        "l1\nl2\nalpha three\nl4\nl5\nl6\nl7\nl8\nl9\nalpha ten\nl11\n",
    )
    .unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    e.open_fixture(&a).unwrap();
    e.open_picker(Kind::Search);
    e.picker_items_fixture(vec![
        Item {
            text: "a.txt:3".into(),
            badge: None,
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(a.clone()),
                line: 3,
                col: 1,
                match_len: 5,
                line_text: "alpha three".into(),
            },
        },
        Item {
            text: "a.txt:10".into(),
            badge: None,
            payload: Payload::Grep {
                location: strop_workspace::ResourceLocation::local(a.clone()),
                line: 10,
                col: 1,
                match_len: 5,
                line_text: "alpha ten".into(),
            },
        },
    ]);
    e.feed(strop_core::frontend_input::Key::CtrlO);
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
        e.message.contains("read-only"),
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
    e.feed(strop_core::frontend_input::Key::CtrlO);
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
    e.feed_text("]f");
    assert_eq!(e.buf().line_of(e.head()), tops[1], "next card");
    e.feed_text("]f[f");
    assert_eq!(e.buf().line_of(e.head()), tops[0], "previous card");
}

#[test]
fn excerpt_navigation_works_on_readonly_collections() {
    let (mut e, _, _) = two_file_fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let collection = e.current();
    let starts: Vec<_> = e.collections[&collection]
        .excerpts
        .iter()
        .map(|excerpt| excerpt.view_start)
        .collect();
    assert_eq!(starts.len(), 2);
    e.set_head(starts[0]);
    e.feed_text("]e");
    assert_eq!(e.head(), starts[1]);
    e.feed_text("[e");
    assert_eq!(e.head(), starts[0]);
}

#[test]
fn collection_search_motions_and_yank_remain_available() {
    let (mut e, _, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let view = current_text(&e);
    e.feed_text("/beta two\r");
    assert_eq!(
        e.buf().text().byte_slice(e.head()..e.head() + 8),
        "beta two"
    );
    e.feed_text("w");
    assert_eq!(e.buf().text().byte_slice(e.head()..e.head() + 3), "two");
    e.feed_text("0yy");
    assert_eq!(e.register(None).text, "beta two\n");
    assert_eq!(current_text(&e), view);
}
