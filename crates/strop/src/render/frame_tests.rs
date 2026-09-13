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

#[test]
fn narrow_remote_modeline_retains_source_identity_during_status_messages() {
    use strop_engine::editor::test_support::remote::{io_editor as editor, open};

    let mut editor = editor("origin\n");
    open(&mut editor, "remote contents\n");
    editor.message = "an active operation has detailed status information ".repeat(8);
    let frame = crate::headless::frame_string(&mut editor, 80, 8).unwrap();
    let modeline = frame.lines().last().unwrap();
    assert!(modeline.contains("ssh:fixture"), "{modeline}");
    assert!(modeline.contains("app.log"), "{modeline}");
    assert!(modeline.contains("[RO]"), "{modeline}");
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
    e.open_search(true);

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
    e.open_search(true);
    // Semantic compilation belongs to the owned worker, never input/render.
    e.feed_text("foo glob:\"**/bad[\"");
    e.wait_picker();
    let err = e
        .picker
        .as_ref()
        .unwrap()
        .picker
        .error
        .clone()
        .expect("query error captured");
    // navigation, not a query edit: the error survives (a query
    // edit clears it — the new search might be valid)
    e.feed(Key::Esc); // field normal mode
    e.feed(Key::Char('j'));
    let frame = crate::headless::frame_string(&mut e, 80, 20).unwrap();
    assert!(
        frame.contains(&err.chars().take(24).collect::<String>()),
        "error in the card: {frame}"
    );
}

#[test]
fn directory_metadata_preserves_unknown_zero_and_native_row_identity() {
    use strop_engine::editor::test_support::remote::snapshot_editor as editor;
    use strop_engine::editor::{Directory, Document};
    use strop_workspace::{
        DirectoryEntry, DirectorySnapshot, EntryKind, EntryName, ListingState, Observation,
        Permissions, ResourceLocation,
    };
    let entry = |name: &str, kind, permissions, size| {
        let mut observation = Observation::unknown(kind);
        observation.permissions = permissions;
        observation.size = size;
        DirectoryEntry {
            name: EntryName::new(name.into()).unwrap(),
            observation,
            error: None,
        }
    };
    let location = ResourceLocation::remote(
        strop_workspace::RemoteEndpoint::parse("ssh://fixture").unwrap(),
        "/repo".into(),
    );
    let directory = Directory::new(DirectorySnapshot {
        location,
        entries: vec![
            entry(
                "line\nbreak",
                EntryKind::SymbolicLink,
                Some(Permissions::new(0o777).unwrap()),
                Some(7),
            ),
            entry("missing", EntryKind::Unknown, None, None),
            entry(
                "zero",
                EntryKind::File,
                Some(Permissions::new(0).unwrap()),
                Some(0),
            ),
        ]
        .into(),
        state: ListingState::Complete,
    });
    let mut editor = editor("origin\n");
    let document = Document::directory(Buffer::from_text(&directory.text()), directory);
    let id = editor.docs.insert(document);
    editor.switch_to(id);
    let frame = crate::headless::frame_string(&mut editor, 80, 10).unwrap();
    let missing = frame.lines().find(|line| line.contains("missing")).unwrap();
    let zero = frame.lines().find(|line| line.contains("zero")).unwrap();
    assert!(
        missing.contains('?') && !missing.contains("  0"),
        "{missing}"
    );
    assert!(
        zero.contains("----------") && zero.contains("  0") && !zero.contains('?'),
        "{zero}"
    );
    let text = editor.buf().text().to_string();
    let link = text.find("line\\nbreak").unwrap();
    editor.set_head(link);
    let source = editor
        .directory()
        .unwrap()
        .entry_location(strop_core::id::LineIndex::new(
            editor.buf().line_of(editor.head()),
        ))
        .unwrap();
    assert_eq!(source.path, std::path::Path::new("/repo/line\nbreak"));
    editor.feed_text("yy");
    assert_eq!(
        editor.register(None).text.matches('\n').count(),
        1,
        "a native newline never creates another actionable row"
    );
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

#[test]
fn same_line_replacements_share_one_exact_styled_review() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "foo foo\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    let source = editor.open_fixture(&path).unwrap();
    editor.open_search(true);
    editor.paste_bracketed("foo");
    editor.wait_picker();
    editor.feed(Key::Tab);
    editor.paste_bracketed("bar");
    editor.feed(Key::Enter);
    editor.wait_io().unwrap();
    let text = editor.buf().text().to_string();
    assert_eq!(text.matches("-foo foo").count(), 1, "{text}");
    assert_eq!(text.matches("+bar bar").count(), 1, "{text}");
    let removed = editor.buf().line_of(text.find("-foo foo").unwrap()) as u16;
    let added = editor.buf().line_of(text.find("+bar bar").unwrap()) as u16;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| crate::render::render(&mut editor, frame))
        .unwrap();
    let grid = terminal.backend().buffer();
    assert_eq!(grid[(5, removed)].fg, crate::render::diff::DEL_FG);
    assert_eq!(grid[(5, added)].fg, crate::render::diff::ADD_FG);
    assert_ne!(grid[(5, removed)].bg, grid[(5, added)].bg);
    editor.feed_text(":apply-change<cr>");
    assert_eq!(editor.doc(source).buf.text().to_string(), "bar bar\n");
}

#[test]
fn search_keeps_outer_geometry_and_recovers_active_field_after_tiny_resize() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.txt");
    let text = "needle\n".repeat(40);
    std::fs::write(&path, &text).unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), root.path().to_path_buf());
    editor.open_fixture(&path).unwrap();
    editor.open_search(false);
    editor.paste_bracketed("needle");
    editor.wait_picker();
    for _ in 0..20 {
        editor.feed(Key::Down);
    }
    for (width, height) in [(140, 40), (100, 30), (80, 24)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::render::render(&mut editor, frame))
            .unwrap();
        let before = terminal.backend().buffer().clone();
        editor.feed(Key::CtrlR);
        terminal
            .draw(|frame| crate::render::render(&mut editor, frame))
            .unwrap();
        let after = terminal.backend().buffer();
        for (x, y) in [
            (1, 0),
            (width - 2, 0),
            (1, height - 2),
            (width - 2, height - 2),
        ] {
            assert_eq!(before[(x, y)].symbol(), after[(x, y)].symbol());
        }
        assert_eq!(editor.picker.as_ref().unwrap().picker.selected, 20);
        editor.feed(Key::CtrlR);
    }
    editor.feed(Key::CtrlR);
    editor.paste_bracketed("replacement");
    let top = editor.picker.as_ref().unwrap().picker.scroll_top;
    let tiny = crate::headless::frame_string(&mut editor, 12, 4).unwrap();
    assert!(tiny.contains("With"), "{tiny}");
    assert_eq!(editor.picker.as_ref().unwrap().picker.scroll_top, top);
    for (width, height) in [(1, 1), (2, 2), (3, 3), (140, 40)] {
        crate::headless::frame_string(&mut editor, width, height).unwrap();
    }
    let restored = crate::headless::frame_string(&mut editor, 140, 40).unwrap();
    assert!(restored.contains("replacement"), "{restored}");
    assert_eq!(editor.picker.as_ref().unwrap().picker.selected, 20);
}
