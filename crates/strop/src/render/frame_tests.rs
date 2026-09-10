//! Frame-content regressions for engine behavior (0046): these tests
//! assert on rendered cell grids, and cell-grid production is the
//! binary's — so they live beside the renderer and drive the engine
//! through `strop_engine`'s public surface.
use strop_core::Buffer;
use strop_engine::editor::{Editor, Key};

/// The tab glyph and the caret read the same layout, driven by the
/// editor's tab width.
#[test]
fn review_tab_glyph_and_caret_use_same_layout() {
    let mut e = Editor::new(Buffer::from_text("\tX\n"));
    e.feed_text("l");
    let backend = ratatui::backend::TestBackend::new(40, 8);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    let x_col = (0..40u16)
        .find(|x| terminal.backend().buffer()[(*x, 0)].symbol() == "X")
        .unwrap() as usize;
    // the contract: the glyph and the caret read the same layout,
    // driven by the editor's tab width
    let caret = 5 + e.buf().cell_col_with_tab(e.head(), e.config.tab_size).get();
    assert_eq!(x_col, caret, "rendered X and caret must agree after a tab");
}

#[test]
fn extra_cursors_render_without_panicking() {
    let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
    e.feed_text(" c");
    let frame = crate::headless::frame_string(&mut e, 40, 10).unwrap();
    assert!(frame.contains("one") && frame.contains("two"));
}

#[test]
fn key_soup_never_panics() {
    // seeded LCG drives thousands of keystrokes across buffer shapes:
    // cursors, pickers, surfaces, undo — every path must stay total
    let keys =
        "hjklwbe0$GwWbBeEdyc><iIoOaAvVspPxXuQq ?%fFtT/\",.:;[]{}()m'rcnNZSL=+-_!@#^&*|~123456789 ";
    let shapes = ["", "x\n", "fn main() {\n    let x = 1;\n}\n", "a\nb\nc\n"];
    let mut state = 0x9e3779b97f4a7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for (round, shape) in shapes.iter().enumerate() {
        let mut e = Editor::new(Buffer::from_text(shape));
        for _ in 0..3000 {
            let c = keys.as_bytes()[(next() as usize) % keys.len()] as char;
            e.feed(Key::Char(c));
            if round % 3 == 0 && next() % 7 == 0 {
                e.feed(Key::Esc);
            }
            if next() % 11 == 0 {
                e.drain_picker();
                e.drain_git_jobs();
                e.drain_lsp();
                e.drain_clipboard();
            }
        }
        // a frame render must never panic either (cursor invariants)
        let _ = crate::headless::frame_string(&mut e, 80, 24).unwrap();
    }
}

/// Golden shape: the blame column renders per line; the commit
/// sidebar renders beside the delta with the current file marked.
#[test]
fn gutters_and_sidebar_render() {
    use strop_engine::editor::test_support::git::{
        fixture, git_out, multi_file_fixture, pump, pump_ready, settle,
    };
    use strop_engine::editor::Surface;

    let (dir, mut e) = fixture();
    let root = dir.path().to_path_buf();
    settle(&mut e, |e| e.git.is_some());
    e.feed_text(" gb");
    pump_ready(&mut e, |e| e.blame_gutter_for(e.first_doc()).is_some());
    let frame = crate::headless::frame_string(&mut e, 100, 10).unwrap();
    let first_sha = git_out(&root, &["rev-parse", "HEAD~1"]);
    assert!(
        frame.contains(&format!("{} t ", &first_sha[..7])),
        "blame cell: {frame}"
    );
    assert!(
        frame.contains("fn a() {}"),
        "content still renders right of the gutter: {frame}"
    );

    let (_d, mut e) = multi_file_fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed(Key::Enter);
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::ChangedFiles { .. }))
    });
    e.feed_text("jj");
    e.feed(Key::Enter); // a.rs delta
    settle(&mut e, |e| {
        matches!(
            e.surface(),
            Some(Surface::Diff {
                commit: Some(_),
                ..
            })
        )
    });
    let frame = crate::headless::frame_string(&mut e, 100, 12).unwrap();
    assert!(frame.contains("▌a.rs"), "current file marked: {frame}");
    assert!(frame.contains(" b.rs"), "sibling files listed: {frame}");
    e.feed_text("]f");
    settle(
        &mut e,
        |e| matches!(e.surface(), Some(Surface::Diff { hunks, .. }) if hunks.label() == "b.rs"),
    );
    let frame = crate::headless::frame_string(&mut e, 100, 12).unwrap();
    assert!(frame.contains("▌b.rs"), "marker follows ]f: {frame}");
}

/// A remote snapshot's search, yank and readonly commands use the real
/// buffer, and the frame shows the remote identity.
#[test]
fn snapshot_search_yank_and_readonly_commands_use_the_real_buffer() {
    use strop_engine::editor::test_support::remote::{io_editor as editor, open};
    use strop_engine::editor::DocumentSource;

    let mut editor = editor("origin\n");
    open(&mut editor, "first\nneedle detail\nlast\n");
    editor.feed_text("/needle<cr>yy");
    assert_eq!(editor.buf().line_of(editor.head()), 1);
    assert_eq!(editor.register(None).text, "needle detail\n");
    editor.feed_text(":set noro<cr>dd");
    assert_eq!(editor.buf().text(), "first\nneedle detail\nlast\n");
    assert!(editor.buf().readonly);
    editor.feed_text(":w! /would-write-locally<cr>");
    assert!(
        !editor.io_pending(),
        "write is refused, never pending on a worker"
    );
    assert!(editor.message.contains("remote"));
    assert!(editor.buf().path.is_none());
    assert!(matches!(editor.cur().source, DocumentSource::Remote(_)));
    editor.feed(Key::Esc); // dismiss the long write error before inspecting identity
    let frame = crate::headless::frame_string(&mut editor, 100, 8).unwrap();
    assert!(frame.contains("ssh:fixture"));
    assert!(frame.contains("app.log"));
    assert!(frame.contains("[RO]"));
}

/// regression (0.3.3 user crash): Space R, type a query, type more —
/// the respawn cleared items but not rows, and the replace renderer
/// indexed items[stale_row] → panic
#[test]
fn respawn_never_renders_stale_rows() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "alpha one\nalpha two\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "alpha three\n").unwrap();
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.cwd = dir.path().to_path_buf();
    e.open_picker(strop_picker::Kind::Replace);

    e.feed_text("alpha");
    e.wait_picker();
    assert!(
        !e.picker.as_ref().unwrap().picker.items.is_empty(),
        "rg delivered matches"
    );
    e.feed_text("b"); // respawn: items + rows both clear
    let frame = crate::headless::frame_string(&mut e, 80, 20).unwrap();
    assert!(frame.contains("replace"), "{frame}");
}

/// a bad filter must read as an error in the card, not a silent empty
/// list or a modeline flash (cleared on the next key)
#[test]
fn rg_error_is_sticky_in_the_card() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "foo\n").unwrap();
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.cwd = dir.path().to_path_buf();
    e.open_picker(strop_picker::Kind::Replace);
    e.feed_text("foo --glob/**/bad[");
    e.wait_picker();
    let err = e.picker.as_ref().unwrap().picker.error.clone();
    assert!(err.is_some(), "rg error captured");
    // navigation, not a query edit: the error survives (a query
    // edit clears it — the new search might be valid)
    e.feed(Key::Esc); // field normal mode
    e.feed(Key::Char('j'));
    let frame = crate::headless::frame_string(&mut e, 80, 20).unwrap();
    assert!(
        frame.contains("unclosed character class"),
        "error in the card: {frame}"
    );
}

#[test]
fn directory_metadata_preserves_unknown_zero_and_native_row_identity() {
    use strop_engine::editor::test_support::remote::snapshot_editor as editor;
    use strop_engine::editor::{Document, RemoteDirectory};
    use strop_remote::{RemoteEntry, RemoteEntryKind, RemotePermissions, RemoteSize};
    use strop_workspace::RemoteFile;

    let root = RemoteFile::parse("ssh://fixture/repo").unwrap();
    let entries = vec![
        RemoteEntry {
            file: root.with_path("/repo/missing".into()).unwrap(),
            kind: RemoteEntryKind::Unknown,
            permissions: None,
            size: None,
        },
        RemoteEntry {
            file: root.with_path("/repo/zero".into()).unwrap(),
            kind: RemoteEntryKind::File,
            permissions: Some(RemotePermissions::new(0).unwrap()),
            size: Some(RemoteSize::new(0)),
        },
        RemoteEntry {
            file: root.with_path("/repo/line\nbreak".into()).unwrap(),
            kind: RemoteEntryKind::SymbolicLink,
            permissions: Some(RemotePermissions::new(0o777).unwrap()),
            size: Some(RemoteSize::new(7)),
        },
    ];
    let directory = RemoteDirectory {
        directory: root,
        entries: entries.into(),
        visible: vec![0, 1, 2],
        filter: String::new(),
        connection: None,
        return_to: None,
    };
    let mut editor = editor("origin\n");
    let document = Document::directory(Buffer::from_text(&directory.text()), directory);
    let id = editor.docs.insert(document);
    editor.switch_to(id);
    let frame = crate::headless::frame_string(&mut editor, 80, 10).unwrap();
    assert!(frame.contains("??????????     ? missing"), "{frame}");
    assert!(frame.contains("----------     0 zero"), "{frame}");
    assert!(frame.contains("lrwxrwxrwx     7 line�break@"), "{frame}");
    editor.feed_text("3Gyy");
    assert!(editor
        .register(None)
        .text
        .starts_with("----------     0 zero"));
}

#[test]
fn last_search_highlights_persistently() {
    let mut e = Editor::new(Buffer::from_text("foo bar foo\n"));
    e.feed_text("/foo\r");
    // committed: no pending pattern, but hits must still compute
    assert!(e.search_pattern().is_none());
    assert_eq!(e.last_search.as_ref().unwrap().query.source(), "foo");
    let frame = crate::headless::frame_string(&mut e, 40, 8).unwrap();
    assert!(frame.contains("foo bar foo"));
}

#[test]
fn backward_prompt_is_present_in_the_actual_headless_frame() {
    let mut e = Editor::new(Buffer::from_text("foo\nbar\n"));
    e.feed_text("?ba");
    let frame = crate::headless::frame_string(&mut e, 40, 8).unwrap();
    assert!(
        frame.contains("? ba"),
        "the backward prompt retains its typed pattern: {frame}"
    );
}

#[test]
fn pipe_prompt_owns_a_visible_input_card_on_normal_and_visual_surfaces() {
    for visual in [false, true] {
        let mut e = Editor::new(Buffer::from_text("unchanged\n"));
        if visual {
            e.feed_text("v");
        }
        e.feed_text(" |tr a-z A-Z");
        let frame = crate::headless::frame_string(&mut e, 60, 10).unwrap();
        assert!(frame.contains("pipe"), "{frame}");
        assert!(frame.contains("tr a-z A-Z"), "{frame}");
        assert_eq!(e.buf().text().to_string(), "unchanged\n");
    }
}
