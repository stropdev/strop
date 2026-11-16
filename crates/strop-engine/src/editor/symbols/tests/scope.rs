//! The breadcrumb through the editor (0107 §10.2): the kept list read at
//! the cursor, carried through typing, following focus and source.

use super::*;
use strop_lsp::LineSpan;

/// What the modeline would show: the names at the cursor, or `None`
/// where the buffer has no symbol list.
fn path(e: &Editor) -> Option<Vec<String>> {
    e.scope_path()
        .map(|path| path.names().map(str::to_owned).collect())
}

fn names(names: &[&str]) -> Option<Vec<String>> {
    Some(names.iter().map(|name| (*name).to_owned()).collect())
}

/// Run every analysis event through the loop as the terminal does: the
/// event, then the after-event work (`warm`, `follow`).
fn settle(e: &mut Editor) {
    while e.analysis.pending() {
        let event = e
            .analysis
            .rx
            .as_ref()
            .expect("local analysis channel")
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("analysis settles");
        e.apply_recorded(Action::Event(AppEvent::Analysis(event)))
            .unwrap();
    }
}

fn at_line(e: &mut Editor, line: usize) {
    type_keys(e, &format!("{}G", line + 1));
}

fn spanning(symbol: ProtoSymbol, first: usize, last: usize) -> ProtoSymbol {
    ProtoSymbol {
        span: LineSpan {
            first: LineIndex::new(first),
            last: LineIndex::new(last),
        },
        ..symbol
    }
}

const NESTED: &str = "\
impl A {
    fn f() {
        let x = 1;
    }
}

fn g() {}
";

/// rust-analyzer's answer for [`NESTED`].
fn server_list() -> Vec<ProtoSymbol> {
    vec![
        spanning(symbol("impl A", "", "Object", 0, 0), 0, 4),
        spanning(symbol("f", "impl A", "Method", 1, 7), 1, 3),
        spanning(symbol("g", "", "Function", 6, 3), 6, 6),
    ]
}

fn served(text: &str) -> Editor {
    let mut e = editor("lib.rs", text);
    serve(&mut e);
    let context = request(&mut e, 0, false);
    reply(&mut e, context, server_list());
    turn(&mut e, Key::Esc);
    e
}

#[test]
fn the_path_is_the_server_list_read_at_the_cursor() {
    let mut e = served(NESTED);
    for (line, expected) in [
        (2, names(&["impl A", "f"])),
        (0, names(&["impl A"])),
        (6, names(&["g"])),
        (5, names(&[])),
    ] {
        at_line(&mut e, line);
        assert_eq!(path(&e), expected, "line {line}");
    }
    // whole lines: the first column and the last say the same
    at_line(&mut e, 1);
    type_keys(&mut e, "0");
    assert_eq!(path(&e), names(&["impl A", "f"]));
    type_keys(&mut e, "$");
    assert_eq!(path(&e), names(&["impl A", "f"]));
}

#[test]
fn a_motion_starts_no_build_and_asks_nobody() {
    let mut e = editor("lib.rs", "fn alpha() {\n}\n\nfn beta() {\n}\n");
    turn(&mut e, Key::Esc);
    settle(&mut e);
    assert_eq!(path(&e), names(&["alpha"]));
    let decided = e.analysis.symbols.settled;
    at_line(&mut e, 3);
    assert_eq!(path(&e), names(&["beta"]));
    at_line(&mut e, 2);
    assert_eq!(path(&e), names(&[]));
    assert!(!e.analysis.symbols.pending(), "no build");
    assert_eq!(e.analysis.symbols.settled, decided, "nothing decided again");
    assert!(e.analysis.symbols.scope.as_ref().unwrap().exact);
}

#[test]
fn typing_carries_the_path_until_the_exact_list_lands() {
    let mut e = served(NESTED);
    at_line(&mut e, 2);
    type_keys(&mut e, "olet y = 2;");
    assert_eq!(path(&e), names(&["impl A", "f"]), "typing inside f");
    assert!(!e.analysis.symbols.scope.as_ref().unwrap().exact);
    turn(&mut e, Key::Esc);
    // `o` on f's closing brace: a new line inside impl A, after f
    at_line(&mut e, 4);
    type_keys(&mut e, "o");
    assert_eq!(path(&e), names(&["impl A"]));
    type_keys(&mut e, "fn h() {}");
    assert_eq!(path(&e), names(&["impl A"]), "not named before Esc");
}

#[test]
fn switching_panes_reads_the_other_files_kept_list() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("a.rs"), "fn alpha() {\n}\n").unwrap();
    std::fs::write(directory.path().join("b.rs"), "fn beta() {\n}\n").unwrap();
    let mut buffer = Buffer::from_text("fn alpha() {\n}\n");
    buffer.path = Some(directory.path().join("a.rs"));
    let mut e = Editor::new_in(buffer, directory.path().to_path_buf());
    turn(&mut e, Key::Esc);
    settle(&mut e);
    assert_eq!(path(&e), names(&["alpha"]));
    e.feed_text(":vsplit b.rs\r");
    e.wait_io().unwrap();
    turn(&mut e, Key::Esc);
    settle(&mut e);
    assert_eq!(path(&e), names(&["beta"]));
    turn(&mut e, Key::CtrlW);
    turn(&mut e, Key::Char('w'));
    assert_eq!(path(&e), names(&["alpha"]), "back on a.rs");
    assert!(
        !e.analysis.symbols.pending(),
        "the kept list answers without a build"
    );
    turn(&mut e, Key::CtrlW);
    turn(&mut e, Key::Char('w'));
    assert_eq!(path(&e), names(&["beta"]));
    assert!(!e.analysis.symbols.pending());
}

#[test]
fn the_servers_list_replaces_the_outlines_when_it_is_ready() {
    let mut e = editor("lib.rs", NESTED);
    turn(&mut e, Key::Esc);
    settle(&mut e);
    at_line(&mut e, 2);
    let outline = path(&e).expect("the outline's path");
    assert_eq!(outline.last().map(String::as_str), Some("f"));
    assert_eq!(
        e.analysis.symbols.scope.as_ref().unwrap().producer,
        Producer::Outline
    );
    serve(&mut e);
    let context = request(&mut e, 0, false);
    reply(&mut e, context, server_list());
    turn(&mut e, Key::Esc);
    assert_eq!(
        e.analysis.symbols.scope.as_ref().unwrap().producer,
        Producer::Server(SERVER)
    );
    assert_eq!(path(&e), names(&["impl A", "f"]), "the server's names");
}

#[test]
fn buffers_with_no_symbol_list_have_no_path() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("a.rs"), "fn alpha() {\n}\n").unwrap();
    std::fs::write(directory.path().join("notes.md"), "# alpha\n").unwrap();
    let mut buffer = Buffer::from_text("fn alpha() {\n}\n");
    buffer.path = Some(directory.path().join("a.rs"));
    let mut e = Editor::new_in(buffer, directory.path().to_path_buf());
    turn(&mut e, Key::Esc);
    settle(&mut e);
    assert_eq!(path(&e), names(&["alpha"]));

    // a folder
    e.browse_directory(&directory.path().display().to_string())
        .unwrap();
    e.wait_io().unwrap();
    turn(&mut e, Key::Esc);
    settle(&mut e);
    assert!(e.directory().is_some());
    assert_eq!(path(&e), None, "a folder");

    // a file whose language has neither a server nor an outline
    e.feed_text(":e notes.md\r");
    e.wait_io().unwrap();
    turn(&mut e, Key::Esc);
    settle(&mut e);
    assert_eq!(path(&e), None, "markdown");

    // a collection of excerpts from a.rs
    e.feed_text(":e a.rs\r");
    e.wait_io().unwrap();
    e.open_picker(Kind::Search);
    e.picker_items_fixture(vec![strop_picker::Item {
        badge: None,
        text: "fn alpha() {".into(),
        payload: strop_picker::Payload::Grep {
            location: strop_workspace::ResourceLocation::local(directory.path().join("a.rs")),
            line: 1,
            col: 1,
            match_len: 2,
            line_text: "fn alpha() {".into(),
        },
    }]);
    turn(&mut e, Key::CtrlO);
    settle(&mut e);
    assert!(
        e.collection_title(e.current()).is_some(),
        "a collection opened"
    );
    assert_eq!(path(&e), None, "a collection");

    // a terminal
    e.feed_text(":terminal-local printf x\r");
    turn(&mut e, Key::Esc);
    assert!(e.terminal_document(e.current()).is_some());
    assert_eq!(path(&e), None, "a terminal");
}

#[test]
fn a_stuck_build_holds_nothing_up_and_the_path_is_carried() {
    let (release, gate) = mpsc::channel::<()>();
    let mut e = editor("lib.rs", NESTED);
    let mut extractors = HashMap::new();
    let mut first = true;
    e.analysis.symbols = SymbolState::with_extract(Box::new(move |work| {
        // the first build runs; every later one is held
        if !std::mem::take(&mut first) {
            gate.recv().ok();
        }
        super::super::outline::extract(&mut extractors, work)
    }));
    turn(&mut e, Key::Esc);
    settle(&mut e);
    at_line(&mut e, 2);
    let before = path(&e);
    assert_eq!(
        before.as_ref().and_then(|p| p.last()).map(String::as_str),
        Some("f")
    );

    // an edit above: the next build is held
    type_keys(&mut e, "ggO// moved");
    turn(&mut e, Key::Esc);
    assert!(
        e.analysis.symbols.pending(),
        "a build is in flight, and held"
    );
    at_line(&mut e, 3);
    assert_eq!(path(&e), before, "the carried path, a line lower");
    at_line(&mut e, 6);
    assert_eq!(path(&e), names(&[]));
    assert!(e.analysis.symbols.pending(), "still held");

    release.send(()).unwrap();
    drop(release);
    settle(&mut e);
    at_line(&mut e, 3);
    assert_eq!(path(&e), before, "the exact list says the same");
    assert!(e.analysis.symbols.scope.as_ref().unwrap().exact);
}

#[test]
fn a_commits_diff_has_no_path() {
    use crate::editor::test_support::git::{fixture, pump, settle as settle_git};
    use crate::editor::Surface;
    let (_directory, mut e) = fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed(Key::Enter);
    settle_git(&mut e, |e| {
        matches!(e.surface(), Some(Surface::ChangedFiles { .. }))
    });
    e.feed(Key::Enter);
    settle_git(&mut e, |e| {
        matches!(e.surface(), Some(Surface::Diff { .. }))
    });
    turn(&mut e, Key::Esc);
    settle(&mut e);
    assert!(e.buf().text().to_string().contains("fn b() {}"));
    assert_eq!(path(&e), None, "a diff of a Rust file");
}
