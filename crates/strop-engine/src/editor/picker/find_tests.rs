//! The finder's contracts (0093 §6.2): what Enter opens while a catalog
//! streams, what the empty query leads with, and what the field's keys do.
use super::*;
use crate::editor::Editor;
use strop_core::frontend_input::{Key, KeyCode, KeyEvent, Modifiers};
use strop_core::worker::Outcome;
use strop_core::Buffer;

fn file(path: &str) -> Item {
    Item {
        badge: None,
        text: path.into(),
        payload: Payload::File(path.into()),
    }
}

fn project(files: &[&str]) -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for path in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "text\n").unwrap();
    }
    let editor = Editor::new_in(Buffer::from_text(""), root);
    (dir, editor)
}

/// Take the Files stream away from the real walk: the test owns the
/// ticket and delivers the catalog batch by batch.
fn own_files_stream(editor: &mut Editor) -> Ticket<PickerKey> {
    editor.open_picker(Kind::Files);
    let request = editor.worker_ids.allocate().unwrap();
    let glue = editor.picker.as_mut().unwrap();
    glue.revoke(CancelReason::Superseded);
    glue.picker.clear_items();
    let ticket = Ticket {
        request,
        key: PickerKey {
            picker: glue.id,
            cwd: glue.file_root.as_ref().unwrap().path.clone(),
        },
    };
    glue.active = Some(ticket.clone());
    glue.picker.streaming = true;
    ticket
}

fn settle_ranking(editor: &mut Editor) {
    while editor
        .picker
        .as_ref()
        .is_some_and(|glue| glue.rank_pending.is_some())
    {
        let event = editor
            .picker_ranking
            .rx
            .as_ref()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        editor.handle_picker_ranking(event);
    }
}

fn rows(editor: &Editor) -> Vec<String> {
    let picker = &editor.picker.as_ref().unwrap().picker;
    picker
        .rows
        .iter()
        .map(|row| picker.items[row.item].text.clone())
        .collect()
}

fn selected(editor: &Editor) -> String {
    editor
        .picker
        .as_ref()
        .unwrap()
        .picker
        .current()
        .unwrap()
        .text
        .clone()
}

fn alt(letter: char) -> strop_core::frontend_input::Input {
    let mut key = KeyEvent::press(KeyCode::Char(letter));
    key.modifiers = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    strop_core::frontend_input::Input::Key(key)
}

/// 0091 B1, the acceptance as written: the catalog streams in three
/// batches while the query is typed in one burst; the bar is row 0 at
/// every drain; Enter opens the top-ranked path.
#[test]
fn a_burst_typed_query_opens_the_top_ranked_file_of_a_streaming_catalog() {
    let (_dir, mut editor) = project(&[
        "Documentation/Makefile",
        "Documentation/index.rst",
        "Makefile",
        "arch/Makefile",
        "scripts/Makefile",
        "tools/Makefile.include",
    ]);
    let root = editor.cwd.clone();
    let ticket = own_files_stream(&mut editor);
    editor.feed_text("Makefile");
    for batch in [
        vec![
            file("Documentation/Makefile"),
            file("Documentation/index.rst"),
        ],
        vec![file("Makefile"), file("arch/Makefile")],
        vec![file("scripts/Makefile"), file("tools/Makefile.include")],
    ] {
        editor.handle_picker_event(PickerEvent {
            ticket: ticket.clone(),
            msg: PickerMsg::Items(batch.into()),
        });
        settle_ranking(&mut editor);
        let picker = &editor.picker.as_ref().unwrap().picker;
        assert_eq!(picker.selected, 0, "the bar is row 0 at every drain");
        assert_eq!(picker.scroll_top, 0);
    }
    assert_eq!(
        rows(&editor)[..2],
        ["Makefile", "arch/Makefile"],
        "the name itself, then the shortest path that holds it"
    );
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    assert!(!editor.picker_open());
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(root.join("Makefile").as_path())
    );
}

/// Enter typed before the ranking of the last batch lands waits for that
/// ranking instead of opening a row the newer catalog has displaced.
#[test]
fn enter_ahead_of_the_ranking_opens_what_the_ranking_puts_first() {
    let (_dir, mut editor) = project(&["Documentation/Makefile", "Makefile"]);
    let root = editor.cwd.clone();
    let ticket = own_files_stream(&mut editor);
    editor.feed_text("Makefile");
    editor.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Items(vec![file("Documentation/Makefile")].into()),
    });
    settle_ranking(&mut editor);
    assert_eq!(selected(&editor), "Documentation/Makefile");
    editor.handle_picker_event(PickerEvent {
        ticket: ticket.clone(),
        msg: PickerMsg::Items(vec![file("Makefile")].into()),
    });
    // the new batch is in the catalog, its ranking is still in flight
    assert!(editor.picker.as_ref().unwrap().rank_pending.is_some());
    editor.feed(Key::Enter);
    assert!(editor.picker_open(), "the accept waits for the ranking");
    settle_ranking(&mut editor);
    editor.wait_io().unwrap();
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(root.join("Makefile").as_path())
    );
}

#[test]
fn the_empty_finder_leads_with_open_files_then_frecent_ones() {
    let (_dir, mut editor) = project(&["a.rs", "b.rs", "c.rs", "old.rs", "zz/never.rs"]);
    let root = editor.cwd.clone();
    // a frecent file that is no longer open, then three open ones
    for name in ["old.rs", "a.rs", "b.rs", "c.rs"] {
        editor.open_fixture(&root.join(name)).unwrap();
    }
    let old = editor
        .docs
        .iter()
        .find(|(_, document)| document.buf.path.as_deref() == Some(root.join("old.rs").as_path()))
        .map(|(id, _)| id)
        .unwrap();
    editor.switch_to(old);
    editor.feed_text(":bd\r");
    let current = editor.buf().path.clone().unwrap();
    assert_eq!(current, root.join("c.rs"), "closing old.rs returns to c.rs");
    editor.open_picker(Kind::Files);
    editor.wait_picker();
    assert_eq!(
        rows(&editor),
        ["b.rs", "a.rs", "c.rs", "old.rs", "zz/never.rs"],
        "the other open files most recent first, the current file, what was \
         visited before, then the tree"
    );
    // Enter on the untouched list is the file you were just in
    assert_eq!(selected(&editor), "b.rs");
    // typing leaves that order: the best match leads
    editor.feed_text("never");
    editor.wait_picker();
    assert_eq!(selected(&editor), "zz/never.rs");
}

#[test]
fn a_frecent_file_leads_its_equals_once_a_query_is_typed() {
    let (_dir, mut editor) = project(&["one/mod.rs", "two/mod.rs", "six/mod.rs", "x.rs"]);
    let root = editor.cwd.clone();
    editor.open_picker(Kind::Files);
    editor.feed_text("mod.rs");
    editor.wait_picker();
    assert_eq!(
        selected(&editor),
        "one/mod.rs",
        "equal matches in tree order"
    );
    editor.close_picker();
    editor.open_fixture(&root.join("two/mod.rs")).unwrap();
    editor.open_fixture(&root.join("x.rs")).unwrap();
    editor.feed_text(":bd\r");
    editor.open_picker(Kind::Files);
    editor.feed_text("mod.rs");
    editor.wait_picker();
    assert_eq!(selected(&editor), "two/mod.rs");
}

#[test]
fn the_buffer_list_rests_on_the_alternate_buffer() {
    let (_dir, mut editor) = project(&["a.rs", "b.rs", "c.rs"]);
    let root = editor.cwd.clone();
    for name in ["a.rs", "b.rs", "c.rs"] {
        editor.open_fixture(&root.join(name)).unwrap();
    }
    editor.open_picker(Kind::Buffers);
    editor.wait_picker();
    assert_eq!(
        rows(&editor)[..2],
        ["c.rs", "b.rs"],
        "the current buffer leads"
    );
    assert_eq!(selected(&editor), "b.rs", "the bar rests on the alternate");
    editor.feed(Key::Enter);
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(root.join("b.rs").as_path())
    );
    // a typed query is a search of the list: its best match is selected
    editor.open_picker(Kind::Buffers);
    editor.feed_text("rs");
    editor.wait_picker();
    assert_eq!(editor.picker.as_ref().unwrap().picker.selected, 0);
}

/// 0091 B12: in a query field, readline's Ctrl-U wins.
#[test]
fn ctrl_u_clears_the_query_back_to_the_start() {
    let (_dir, mut editor) = project(&["alpha.rs", "beta.rs"]);
    editor.open_picker(Kind::Files);
    editor.feed_text("alp");
    editor.wait_picker();
    assert_eq!(rows(&editor), ["alpha.rs"]);
    editor.feed(Key::CtrlU);
    editor.wait_picker();
    assert_eq!(editor.picker.as_ref().unwrap().picker.input.text(), "");
    assert_eq!(rows(&editor).len(), 2, "the whole list is back");
    // only the text before the caret goes, as in a shell
    editor.feed_text("alpha");
    editor.feed(Key::Left);
    editor.feed(Key::Left);
    editor.feed(Key::CtrlU);
    assert_eq!(editor.picker.as_ref().unwrap().picker.input.text(), "ha");
}

#[test]
fn alt_h_and_alt_i_toggle_hidden_and_ignored_files_in_the_finder() {
    let (_dir, mut editor) = project(&[".env", "src/main.rs", "target/out.rs"]);
    let root = editor.cwd.clone();
    std::fs::create_dir(root.join(".git")).unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    editor.open_picker(Kind::Files);
    editor.wait_picker();
    assert_eq!(rows(&editor), [".env", ".gitignore", "src/main.rs"]);
    assert_eq!(editor.picker.as_ref().unwrap().status_row, "");

    editor.handle_frontend_input(alt('i'));
    editor.wait_picker();
    assert!(rows(&editor).contains(&"target/out.rs".to_string()));
    assert_eq!(
        editor.picker.as_ref().unwrap().status_row,
        "ignored files shown"
    );

    editor.handle_frontend_input(alt('h'));
    editor.wait_picker();
    assert_eq!(rows(&editor), ["src/main.rs", "target/out.rs"]);
    assert_eq!(
        editor.picker.as_ref().unwrap().status_row,
        "hidden files skipped · ignored files shown"
    );

    // the toggles are the session's: a reopened finder keeps them, and
    // says so
    editor.close_picker();
    editor.open_picker(Kind::Files);
    editor.wait_picker();
    assert_eq!(rows(&editor), ["src/main.rs", "target/out.rs"]);
    editor.handle_frontend_input(alt('h'));
    editor.handle_frontend_input(alt('i'));
    editor.wait_picker();
    assert_eq!(editor.picker.as_ref().unwrap().status_row, "");
    // the finder has no expression: its other chords stay Escape + letter
    editor.handle_frontend_input(alt('c'));
    assert!(editor.picker.as_ref().unwrap().picker.input_normal());
    assert_eq!(editor.picker.as_ref().unwrap().status_row, "");
}

#[test]
fn search_toggles_change_what_matches_and_name_themselves() {
    let (_dir, mut editor) = project(&[]);
    let root = editor.cwd.clone();
    std::fs::write(
        root.join("blade.txt"),
        "edge\nEdge\nhedgerow\nedges\nfo+ literal\nfooo\n",
    )
    .unwrap();
    let hits = |editor: &mut Editor| -> Vec<String> {
        editor.wait_picker();
        let picker = &editor.picker.as_ref().unwrap().picker;
        assert!(picker.error.is_none(), "{:?}", picker.error);
        picker
            .rows
            .iter()
            .map(|row| match &picker.items[row.item].payload {
                Payload::Grep { line_text, .. } => line_text.to_string(),
                other => panic!("grep rows only: {other:?}"),
            })
            .collect()
    };
    editor.open_search(false);
    editor.feed_text("edge");
    assert_eq!(hits(&mut editor), ["edge", "Edge", "hedgerow", "edges"]);
    assert_eq!(editor.picker.as_ref().unwrap().status_row, "");

    editor.handle_frontend_input(alt('c'));
    assert_eq!(hits(&mut editor), ["edge", "hedgerow", "edges"]);
    assert_eq!(editor.picker.as_ref().unwrap().status_row, "match case");

    editor.handle_frontend_input(alt('w'));
    assert_eq!(hits(&mut editor), ["edge"]);
    assert_eq!(
        editor.picker.as_ref().unwrap().status_row,
        "match case · whole word"
    );
    editor.handle_frontend_input(alt('c'));
    assert_eq!(hits(&mut editor), ["edge", "Edge"]);

    editor.handle_frontend_input(alt('w'));
    editor.feed(Key::CtrlU);
    editor.feed_text("fo+");
    assert_eq!(hits(&mut editor), ["fo+ literal"], "literal by default");
    editor.handle_frontend_input(alt('r'));
    assert_eq!(hits(&mut editor), ["fo+ literal", "fooo"]);
    assert_eq!(editor.picker.as_ref().unwrap().status_row, "regex");
    // the query text is the user's: no toggle rewrites the field
    assert_eq!(editor.picker.as_ref().unwrap().picker.input.text(), "fo+");
    editor.handle_frontend_input(alt('r'));
    assert_eq!(hits(&mut editor), ["fo+ literal"]);
}

#[test]
fn project_search_is_smart_case() {
    let (_dir, mut editor) = project(&[]);
    std::fs::write(editor.cwd.join("a.txt"), "needle\nNeedle\nNEEDLE\n").unwrap();
    let count = |editor: &mut Editor, query: &str| {
        editor.feed(Key::CtrlU);
        editor.feed_text(query);
        editor.wait_picker();
        editor.picker.as_ref().unwrap().picker.rows.len()
    };
    editor.open_search(false);
    assert_eq!(
        count(&mut editor, "needle"),
        3,
        "lowercase matches any case"
    );
    assert_eq!(
        count(&mut editor, "Needle"),
        1,
        "a capital makes case count"
    );
    assert_eq!(count(&mut editor, "case:ignore Needle"), 3);
}

/// The stream ends with a terminal event however the toggles moved it.
#[test]
fn a_toggle_supersedes_the_running_search_and_the_new_one_settles() {
    let (_dir, mut editor) = project(&[]);
    std::fs::write(editor.cwd.join("a.txt"), "Edge\nedge\n").unwrap();
    editor.open_search(false);
    editor.feed_text("edge");
    let first = editor.picker.as_ref().unwrap().active.clone().unwrap();
    editor.handle_frontend_input(alt('c'));
    let second = editor.picker.as_ref().unwrap().active.clone().unwrap();
    assert_ne!(first, second, "the toggle launched a new request");
    // the superseded request's rows can no longer publish
    editor.handle_picker_event(PickerEvent {
        ticket: first,
        msg: PickerMsg::Finished(Outcome::Success(())),
    });
    assert!(editor.picker.as_ref().unwrap().picker.streaming);
    editor.wait_picker();
    assert_eq!(editor.picker.as_ref().unwrap().picker.rows.len(), 1);
}
