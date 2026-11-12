//! Collection behavior: building and source refreshes. Editing through a
//! collection is in `write.rs`, getting in and moving between hits in
//! `flow.rs`, `:cdo` and the other quickfix names in `quickfix.rs`.

use crate::editor::Editor;
use strop_core::Buffer;
use strop_picker::{Item, Kind, Payload};
mod convergence;
mod flow;
mod live;
mod ownership;
mod quickfix;
mod write;

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

/// Type `text` and let any deferred resolution (indentation, a large
/// substitution) land, as the event loop would.
fn type_keys(e: &mut Editor, text: &str) {
    e.feed_text(text);
    while e.resolution.pending() {
        if !e.resolution.blocked() && !e.resolution.queue.is_empty() {
            e.resume_deferred_input();
        } else {
            let event = e
                .resolution
                .rx
                .as_ref()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("resolution settles");
            e.handle_resolution(event);
        }
    }
}

fn current_text(e: &Editor) -> String {
    e.buf().text().to_string()
}

#[test]
fn collection_builds_from_picker_hits() {
    let (mut e, _, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    assert!(!e.buf().readonly, "a collection is an editing surface");
    let text = current_text(&e);
    assert!(text.contains("alpha one"), "{text}");
    assert!(text.contains("beta two"), "{text}");
    assert!(text.lines().any(|row| row == "a.txt"), "heading: {text}");
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
    assert!(e.message.contains("not text"), "{}", e.message);
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
    assert!(e.message.contains("1 hit left out"), "{}", e.message);
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
fn remote_sources_join_collections() {
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
            stamp: strop_remote::Stamp { size: 1, mtime: 1 },
            connection: None,
            return_to: None,
        },
    );
    let remote = e.docs.try_insert(remote_doc).unwrap();
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
    let before = e.doc(e.current()).buf.text().to_string();
    let at = before.find("remote one").unwrap() + 7;
    e.set_head(at);
    e.feed_text("x");
    assert_eq!(e.doc(remote).buf.text().to_string(), "remote ne\n");
    assert!(current_text(&e).contains("remote ne"));
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
    assert!(
        e.doc(source).buf.is_dirty(),
        "unsaved source edits are intact"
    );
}

/// 0049 §7.2: occurrence selection in a collection matches excerpt
/// bodies only — the needle in a header row is chrome, never selected.
#[test]
fn occurrence_selection_skips_collection_chrome() {
    let (mut e, _, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    // caret on the header's "txt" (in "a.txt") — seeds "txt"
    let header_start = e.buf().line_start(2);
    let header = e.buf().line_text(2);
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
    e.set_head(e.buf().line_start(3) + 1);
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
    let rows = e.collections.get(&e.current()).unwrap().rows.clone();
    assert_eq!(
        rows.iter()
            .filter(|row| matches!(row, crate::editor::CollectionRow::Heading(_)))
            .count(),
        1,
        "one block for the file: {text}"
    );
    // the buffer holds words: no drawing characters anywhere
    assert!(!text.contains(['╭', '╰', '─', '⋮']), "{text}");
    let gap_line = text.lines().position(|l| l == "2 lines not shown").unwrap();
    assert_eq!(rows[gap_line], crate::editor::CollectionRow::Gap);
    // the gap row, the heading and the blank row are protected
    for line in [gap_line, 2, 1] {
        e.set_head(e.buf().line_start(line));
        // `x` on a word, `iq` on the empty row: both edit chrome
        e.feed_text(if line == 1 { "iq" } else { "x" });
        assert!(
            e.message.contains("not text"),
            "row {line} refuses: {}",
            e.message
        );
        assert_eq!(current_text(&e), text);
    }
    // nothing closes the block but the end of the buffer
    assert_eq!(rows.last(), Some(&crate::editor::CollectionRow::Body));
}

/// 0100 §9: `yy` on a heading yanks the path, on a gap its words; a
/// visual yank over a block holds no drawing character.
#[test]
fn a_yank_of_a_block_is_its_words() {
    let (mut e, _, _) = fixture();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let text = current_text(&e);
    let heading = text.lines().position(|l| l == "a.txt").unwrap();
    e.set_head(e.buf().line_start(heading));
    e.feed_text("yy");
    assert_eq!(e.register(None).text, "a.txt\n");
    e.feed_text("ggVGy");
    let yanked = e.register(None).text.clone();
    assert!(yanked.contains("a.txt\n"), "{yanked}");
    assert!(!yanked.contains(['╭', '╰', '─', '⋮', '│']), "{yanked}");
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
            matches!(kind, crate::editor::CollectionRow::Heading(_)).then_some(row)
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
