//! Symbols through the editor (0099 §6): what a press shows, what an edit
//! makes of a kept list, and what a build in flight may hold up — nothing.

use super::store::{Build, BuildId, Producer};
use super::{open, warm, SymbolState};
use crate::editor::events::AppEvent;
use crate::editor::lsp::state::Binding;
use crate::editor::lsp::LspServer;
use crate::editor::trace::drive::Action;
use crate::editor::Editor;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use strop_core::frontend_input::Key;
use strop_core::id::LineIndex;
use strop_core::Buffer;
use strop_lsp::{
    LspEvent, PositionEncoding, ProtoSymbol, ReplyContext, RequestId, RequestKind, RequestStamp,
    ServerColumn, ServerId, ServerLocation, ServerPosition,
};
use strop_picker::Kind;

const SOURCE: &str = "fn alpha() {}\nfn beta() {}\n";

fn editor(name: &str, text: &str) -> Editor {
    let mut buffer = Buffer::from_text(text);
    buffer.path = Some(PathBuf::from("/workspace").join(name));
    Editor::new_in(buffer, PathBuf::from("/workspace"))
}

/// One turn of the event loop, as the terminal and the headless driver
/// run it: the event, then the after-event work.
fn turn(editor: &mut Editor, key: Key) {
    editor
        .apply_recorded(Action::Event(AppEvent::EditorKey(key)))
        .unwrap();
}

fn type_keys(editor: &mut Editor, keys: &str) {
    for key in keys.chars() {
        turn(editor, Key::Char(key));
    }
}

/// The picker's rows, or `None` when no symbols picker is open.
fn rows(editor: &Editor) -> Option<Vec<String>> {
    let glue = editor.picker.as_ref()?;
    (glue.picker.kind == Kind::Symbols).then(|| {
        glue.picker
            .items
            .iter()
            .map(|item| item.text.clone())
            .collect()
    })
}

fn row_of<'a>(rows: &'a [String], name: &str) -> &'a str {
    rows.iter()
        .find(|row| row.starts_with(name))
        .unwrap_or_else(|| panic!("no row for {name} in {rows:?}"))
}

#[test]
fn the_outline_is_built_ahead_of_the_key_and_a_press_starts_nothing() {
    let mut e = editor("lib.rs", SOURCE);
    turn(&mut e, Key::Esc);
    assert!(
        e.analysis.symbols.pending(),
        "the first event starts the build"
    );
    e.wait_analysis();
    open(&mut e);
    assert!(
        !e.analysis.symbols.pending(),
        "a kept list opens without a build"
    );
    let rows = rows(&e).expect("the picker opened in the same turn");
    assert!(row_of(&rows, "alpha").contains(":1"));
    assert!(row_of(&rows, "beta").contains(":2"));
}

#[test]
fn a_stale_list_is_never_served_after_an_edit() {
    let mut e = editor("lib.rs", SOURCE);
    turn(&mut e, Key::Esc);
    e.wait_analysis();
    open(&mut e);
    assert!(row_of(&rows(&e).unwrap(), "beta").contains(":2"));
    e.close_picker();

    // A line above `beta`: every kept row below it now names the wrong line.
    type_keys(&mut e, "O// moved");
    assert!(
        !e.analysis.symbols.pending(),
        "typing in Insert mode starts no build"
    );
    turn(&mut e, Key::Esc);
    assert!(e.analysis.symbols.pending(), "the finished edit starts one");

    // Pressed before the new build lands: the old rows must not appear.
    open(&mut e);
    assert_eq!(rows(&e), None, "nothing is shown from the old revision");
    e.wait_analysis();
    let rows = rows(&e).expect("the picker opens when the build lands");
    assert!(row_of(&rows, "alpha").contains(":2"), "{rows:?}");
    assert!(row_of(&rows, "beta").contains(":3"), "{rows:?}");
}

#[test]
fn an_edit_made_with_no_turn_in_between_still_misses() {
    // `feed` alone runs no after-event work: the kept list is one
    // revision behind and nothing has noticed yet.
    let mut e = editor("lib.rs", SOURCE);
    turn(&mut e, Key::Esc);
    e.wait_analysis();
    e.feed_text("O// moved");
    e.feed(Key::Esc);
    open(&mut e);
    assert_eq!(rows(&e), None);
    e.wait_analysis();
    assert!(row_of(&rows(&e).unwrap(), "beta").contains(":3"));
}

#[test]
fn a_build_overtaken_by_an_edit_is_dropped() {
    let (release, gate) = mpsc::channel::<()>();
    let mut e = editor("lib.rs", SOURCE);
    let mut extractors = HashMap::new();
    e.analysis.symbols = SymbolState::with_extract(Box::new(move |work| {
        gate.recv().ok();
        super::outline::extract(&mut extractors, work)
    }));
    turn(&mut e, Key::Esc);
    let first = *e.analysis.symbols.store.building().unwrap();
    type_keys(&mut e, "O// moved");
    turn(&mut e, Key::Esc);
    release.send(()).unwrap();
    release.send(()).unwrap();
    e.wait_analysis();
    assert!(
        !e.analysis
            .symbols
            .store
            .has(first.document, Producer::Outline, first.revision),
        "the overtaken build's list was not kept"
    );
    // the next turn builds for the text as it is
    turn(&mut e, Key::Esc);
    e.wait_analysis();
    open(&mut e);
    assert!(row_of(&rows(&e).unwrap(), "beta").contains(":3"));
}

#[test]
fn nothing_waits_for_a_build_that_is_stuck() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("lib.rs"), SOURCE).unwrap();
    std::fs::write(directory.path().join("other.rs"), "fn gamma() {}\n").unwrap();
    let (release, gate) = mpsc::channel::<()>();
    let mut buffer = Buffer::from_text(SOURCE);
    buffer.path = Some(PathBuf::from("lib.rs"));
    let mut e = Editor::new_in(buffer, directory.path().to_path_buf());
    let mut extractors = HashMap::new();
    e.analysis.symbols = SymbolState::with_extract(Box::new(move |work| {
        gate.recv().ok();
        super::outline::extract(&mut extractors, work)
    }));
    turn(&mut e, Key::Esc);
    assert!(
        e.analysis.symbols.pending(),
        "a build is in flight, and held"
    );

    // input and motion
    type_keys(&mut e, "ofn delta() {}");
    turn(&mut e, Key::Esc);
    type_keys(&mut e, "ggj");
    assert_eq!(e.buf().line_of(e.head()), 1);
    assert!(e.buf().text().to_string().contains("fn delta() {}"));

    // the finder lists
    e.open_picker(Kind::Files);
    e.wait_picker();
    let listed: Vec<String> = e
        .picker
        .as_ref()
        .unwrap()
        .picker
        .items
        .iter()
        .map(|item| item.text.clone())
        .collect();
    assert!(listed.iter().any(|path| path == "other.rs"), "{listed:?}");
    e.close_picker();

    // a save completes
    e.feed_text(":w!\r");
    e.wait_io().unwrap();
    assert!(std::fs::read_to_string(directory.path().join("lib.rs"))
        .unwrap()
        .contains("fn delta() {}"));

    assert!(
        e.analysis.symbols.pending(),
        "all of that happened with the build still held"
    );
    assert_eq!(rows(&e), None);

    // Released at last, its list is for text that no longer exists.
    release.send(()).unwrap();
    drop(release);
    e.wait_analysis();
    let revision = e.buf().revision();
    assert!(!e
        .analysis
        .symbols
        .store
        .has(e.current(), Producer::Outline, revision));
}

#[test]
fn a_file_with_no_grammar_and_no_server_says_so() {
    let mut e = editor("notes.txt", "alpha\n");
    turn(&mut e, Key::Esc);
    assert!(!e.analysis.symbols.pending(), "nothing to build");
    open(&mut e);
    assert_eq!(rows(&e), None);
    assert!(
        e.message.contains("document symbols"),
        "the refusal names the verb: {}",
        e.message
    );
}

// ---- the server's list ----------------------------------------------------

const SERVER: ServerId = ServerId::new(7);

/// The current document open on a ready server, as after `didOpen`.
fn serve(e: &mut Editor) {
    let (sender, rx) = mpsc::channel();
    drop(sender);
    e.lsp_servers.push(LspServer {
        id: SERVER,
        name: "fake".into(),
        language: "rust".into(),
        root: PathBuf::from("/workspace"),
        target: strop_workspace::Filesystem::Local,
        client: None,
        rx,
        ready: true,
    });
    let document = e.current();
    e.lsp_state.bindings.insert(
        document,
        Binding {
            server: SERVER,
            revision: e.buf().revision(),
            path: PathBuf::from("/workspace/lib.rs"),
            root: PathBuf::from("/workspace"),
            language: "rust".into(),
            target: strop_workspace::Filesystem::Local,
        },
    );
}

/// A `documentSymbol` request for the current document at its current
/// revision, owned as a build — what `start_server` registers before the
/// request leaves.
fn request(e: &mut Editor, id: u64, open: bool) -> ReplyContext {
    let stamp = RequestStamp {
        request: RequestId::new(id),
        server: SERVER,
        document: e.current(),
        revision: e.buf().revision(),
    };
    e.analysis.symbols.store.begin(Build {
        id: BuildId::Server(stamp),
        document: stamp.document,
        revision: stamp.revision,
        producer: Producer::Server(SERVER),
        open,
    });
    ReplyContext {
        stamp,
        encoding: PositionEncoding::Utf8,
        kind: RequestKind::DocumentSymbols,
    }
}

fn symbol(name: &str, container: &str, kind: &str, line: usize, column: usize) -> ProtoSymbol {
    ProtoSymbol {
        name: name.into(),
        container: container.into(),
        kind: kind.into(),
        location: ServerLocation {
            doc: strop_workspace::ResourceLocation::local(PathBuf::from("/workspace/lib.rs")),
            position: ServerPosition {
                line: LineIndex::new(line),
                column: ServerColumn::new(column),
            },
        },
    }
}

fn reply(e: &mut Editor, context: ReplyContext, symbols: Vec<ProtoSymbol>) {
    e.handle_lsp_event(LspEvent::Symbols { context, symbols });
}

#[test]
fn the_servers_reply_opens_the_picker_and_accepting_jumps() {
    // 0047 §1: the reply becomes picker rows; Enter lands on the
    // symbol and records a jumplist entry for ctrl-o.
    let mut e = editor(
        "lib.rs",
        "struct Foo;\nimpl Foo { fn bar() {} }\nfn main() {}\n",
    );
    serve(&mut e);
    e.feed_text("G");
    let origin = e.head();
    let context = request(&mut e, 0, true);
    reply(
        &mut e,
        context,
        vec![
            symbol("Foo", "", "Struct", 0, 7),
            symbol("bar", "Foo", "Method", 1, 14),
        ],
    );
    let glue = e.picker.as_ref().expect("the symbols picker opened");
    assert_eq!(glue.picker.kind, Kind::Symbols);
    let items = &glue.picker.items;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].badge.as_deref(), Some("struct"), "kind chip");
    assert!(items[0].text.contains("Foo") && items[0].text.contains(":1"));
    assert_eq!(items[1].badge.as_deref(), Some("meth"));
    assert!(
        items[1].text.contains("bar")
            && items[1].text.contains("Foo")
            && items[1].text.contains(":2")
    );
    e.wait_picker();
    e.feed(Key::Enter);
    assert!(!e.picker_open());
    assert_eq!(e.head(), 7, "landed on the struct identifier");
    e.feed(Key::CtrlO);
    assert_eq!(e.head(), origin, "ctrl-o returns to the pre-jump spot");
}

#[test]
fn a_second_press_at_the_same_revision_asks_nobody() {
    let mut e = editor("lib.rs", SOURCE);
    serve(&mut e);
    let context = request(&mut e, 0, false);
    reply(&mut e, context, vec![symbol("alpha", "", "Function", 0, 3)]);
    assert_eq!(rows(&e), None, "a background reply opens nothing");

    for _ in 0..2 {
        open(&mut e);
        assert!(
            !e.analysis.symbols.pending(),
            "the kept list answers: no request is owned"
        );
        assert_eq!(rows(&e).unwrap().len(), 1);
        e.close_picker();
    }

    // After an edit the kept list is for another revision: the press goes
    // to the server (this fixture has no connection, so it is refused in
    // words) and the old row is not shown.
    e.feed_text("Ofn first() {}");
    e.feed(Key::Esc);
    open(&mut e);
    assert_eq!(rows(&e), None);
    assert!(e.message.starts_with("lsp:"), "{}", e.message);
}

#[test]
fn a_reply_for_text_that_has_changed_is_dropped() {
    let mut e = editor("lib.rs", SOURCE);
    serve(&mut e);
    let context = request(&mut e, 0, true);
    let asked_at = e.buf().revision();
    e.feed_text("Ofn first() {}");
    e.feed(Key::Esc);
    reply(&mut e, context, vec![symbol("alpha", "", "Function", 0, 3)]);
    assert_eq!(rows(&e), None, "rows of the old text are not shown");
    assert!(!e
        .analysis
        .symbols
        .store
        .has(e.current(), Producer::Server(SERVER), asked_at));
    assert!(e.message.contains("the buffer changed"), "{}", e.message);
}

#[test]
fn a_restarted_servers_lists_are_gone() {
    let mut e = editor("lib.rs", SOURCE);
    serve(&mut e);
    let context = request(&mut e, 0, false);
    reply(&mut e, context, vec![symbol("alpha", "", "Function", 0, 3)]);
    let (document, revision) = (e.current(), e.buf().revision());
    assert!(e
        .analysis
        .symbols
        .store
        .has(document, Producer::Server(SERVER), revision));

    // a request in flight when the server dies: its reply is nobody's
    let late = request(&mut e, 1, true);
    e.lsp_failed(SERVER);
    assert!(!e
        .analysis
        .symbols
        .store
        .has(document, Producer::Server(SERVER), revision));
    reply(&mut e, late, vec![symbol("alpha", "", "Function", 0, 3)]);
    assert_eq!(rows(&e), None);
    assert!(!e
        .analysis
        .symbols
        .store
        .has(document, Producer::Server(SERVER), revision));

    // with no server the outline answers
    turn(&mut e, Key::Esc);
    e.wait_analysis();
    open(&mut e);
    assert_eq!(rows(&e).unwrap().len(), 2);
}

#[test]
fn an_empty_reply_names_the_document_and_is_kept() {
    let mut e = editor("lib.rs", SOURCE);
    serve(&mut e);
    let context = request(&mut e, 0, true);
    reply(&mut e, context, Vec::new());
    assert_eq!(rows(&e), None);
    assert_eq!(e.message, "no symbols in this document");
    e.message.clear();
    open(&mut e);
    assert!(!e.analysis.symbols.pending());
    assert_eq!(e.message, "no symbols in this document");
}

#[test]
fn closing_the_buffer_drops_its_lists() {
    let mut e = editor("lib.rs", SOURCE);
    turn(&mut e, Key::Esc);
    e.wait_analysis();
    let (document, revision) = (e.current(), e.buf().revision());
    assert!(e
        .analysis
        .symbols
        .store
        .has(document, Producer::Outline, revision));
    e.analysis
        .forget(crate::editor::analysis::AnalysisTarget::Document(document));
    assert!(!e
        .analysis
        .symbols
        .store
        .has(document, Producer::Outline, revision));
}

#[test]
fn the_warm_check_decides_once_per_state() {
    let mut e = editor("notes.txt", "alpha\n");
    warm(&mut e);
    let settled = e.analysis.symbols.settled;
    assert!(settled.is_some());
    warm(&mut e);
    assert_eq!(e.analysis.symbols.settled, settled);
}

#[test]
fn a_server_is_not_asked_unasked_about_a_huge_buffer() {
    let server = Some(Producer::Server(SERVER));
    assert_eq!(super::unasked(server, 5_000), server);
    assert_eq!(super::unasked(server, 5_001), None);
    // the outline is local: its size costs a thread some time, nothing else
    assert_eq!(
        super::unasked(Some(Producer::Outline), 1_000_000),
        Some(Producer::Outline)
    );
    assert_eq!(super::unasked(None, 1), None);
}

#[test]
fn pressing_again_while_waiting_asks_again() {
    let (release, gate) = mpsc::channel::<()>();
    let mut e = editor("lib.rs", SOURCE);
    let mut extractors = HashMap::new();
    e.analysis.symbols = SymbolState::with_extract(Box::new(move |work| {
        gate.recv().ok();
        super::outline::extract(&mut extractors, work)
    }));
    turn(&mut e, Key::Esc);
    let background = e.analysis.symbols.store.building().unwrap().id;
    open(&mut e);
    assert_eq!(
        e.analysis.symbols.store.building().unwrap().id,
        background,
        "the first press adopts the build already running"
    );
    open(&mut e);
    let asked = e.analysis.symbols.store.building().unwrap().id;
    assert_ne!(asked, background, "the second press is a new request");
    release.send(()).unwrap();
    release.send(()).unwrap();
    e.wait_analysis();
    assert_eq!(rows(&e).unwrap().len(), 2);
}
