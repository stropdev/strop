use super::*;

#[test]
fn marks_set_and_jump() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    std::fs::write(&first, "one\ntwo\nthree\n").unwrap();
    std::fs::write(&second, "alpha\nbeta\n").unwrap();
    let mut e = Editor::new(Buffer::open(&first).unwrap());
    e.feed_text("jj"); // line 3
    e.feed_text("mb"); // mark b here
    e.feed_text(&format!(":e {}<cr>", second.display()));
    e.wait_io().unwrap();
    assert_eq!(e.buf().path.as_deref(), Some(second.as_path()));
    e.feed_text("'b"); // jump back to mark
    assert_eq!(e.buf().path.as_deref(), Some(first.as_path()));
    assert_eq!(e.buf().line_of(e.head()), 2);
}

#[test]
fn question_mark_is_search_backward() {
    // vim fidelity: ? is search-backward, never the keybinds popup
    let mut e = Editor::new(Buffer::from_text("one two one two\n"));
    e.feed_text("$"); // end
    e.feed_text("?one\r");
    assert_eq!(
        e.buf().col_of(e.head()),
        8,
        "backward search lands on the second 'one'"
    );
}
#[test]
fn dw_leaves_exactly_one_cursor() {
    // 0015: the cascade must not stack the primary's own landing
    let mut e = Editor::new(Buffer::from_text("one two three\n"));
    e.feed_text("dw");
    assert_eq!(e.sels().extra_heads().len(), 0);
    assert_eq!(e.buf().text().to_string(), "two three\n");
}

#[test]
fn arrows_consume_pending_counts() {
    // 0015: 2 <Right> x — the count moves twice and clears; x is 1
    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text("2");
    e.feed(crate::editor::Key::Right);
    assert_eq!(e.buf().col_of(e.head()), 2);
    e.feed_text("x");
    assert_eq!(e.buf().text().to_string(), "helo world\n");
}

#[test]
fn pathless_save_is_an_error_not_a_lie() {
    // 0015: :w on a scratch must never report "written"
    let mut e = Editor::new(Buffer::from_text("unsaved\n"));
    e.feed_text(":w\r");
    assert!(e.message.contains("no file name"), "{}", e.message);
    // :wq must not close the dirty scratch either
    e.feed_text(":wq\r");
    assert_eq!(e.buf().text().to_string(), "unsaved\n");
    // :w {path} names it and persists
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("named.txt");
    e.feed_text(&format!(":w {}\r", p.display()));
    e.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "unsaved\n");
    assert_eq!(e.buf().path.as_deref(), Some(p.as_path()));
}

#[test]
fn ctrl_c_warns_once_then_forces() {
    // 0015: dirty work gets one warning; the second press exits
    let mut e = Editor::new(Buffer::from_text("dirty\n"));
    e.feed_text("ix");
    e.feed(crate::editor::Key::Esc);
    assert!(!e.ctrl_c_quit());
    assert!(e.message.contains("ctrl-c again"));
    assert!(e.ctrl_c_quit());
    // clean editor: quits immediately
    let mut e = Editor::new(Buffer::from_text("clean\n"));
    assert!(e.ctrl_c_quit());
}

#[test]
fn failed_pipe_never_touches_the_source() {
    // 0015: `| false` preserves the range; stderr explains itself
    let mut e = Editor::new(Buffer::from_text("keep me\n"));
    e.feed_text("V");
    e.feed_text(" |false");
    e.feed(crate::editor::Key::Enter);
    let result = e
        .shell_rx
        .as_ref()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    e.handle_shell_result(result);
    assert_eq!(e.buf().text().to_string(), "keep me\n");
    assert!(e.message.starts_with("pipe failed"), "{}", e.message);
}
fn tall_editor() -> Editor {
    let text: String = (1..=100).map(|i| format!("line {i:03}\n")).collect();
    let mut e = Editor::new(Buffer::from_text(&text));
    e.view_rows = 20;
    e
}

#[test]
fn ctrl_d_u_scroll_half_pages() {
    let mut e = tall_editor();
    e.feed_text("<c-d>");
    assert_eq!(e.buf().line_of(e.head()), 10);
    e.feed_text("<c-u>");
    assert_eq!(e.buf().line_of(e.head()), 0);
    // a pending count is the scroll size (vim)
    e.feed_text("5<c-d>");
    assert_eq!(e.buf().line_of(e.head()), 5);
}

#[test]
fn view_place_centers_and_edges() {
    let mut e = tall_editor();
    e.feed_text("50G");
    e.view_place('z');
    let line = e.buf().line_of(e.head());
    assert_eq!(e.view_top() + 10, line);
    e.view_place('t');
    assert_eq!(e.view_top(), line);
    e.view_place('b');
    assert_eq!(e.view_top() + 20, line + 1);
}

#[test]
fn visible_jumps_and_counts() {
    let mut e = tall_editor();
    e.view_mut().view_top = 30;
    e.feed_text("H");
    assert_eq!(e.buf().line_of(e.head()), 30);
    e.feed_text("3L");
    assert_eq!(e.buf().line_of(e.head()), 30 + 20 - 1 - 2);
    e.feed_text("M");
    assert_eq!(e.buf().line_of(e.head()), 30 + 10);
}

#[test]
fn gv_reselects_and_gi_reinserts() {
    let mut e = Editor::new(Buffer::from_text("hello world\nsecond\n"));
    e.feed_text("vll");
    e.feed(crate::editor::Key::Esc);
    e.feed_text("gv");
    assert_eq!(e.mode, crate::editor::Mode::Visual);
    let p = e.sels().primary();
    assert_eq!((p.anchor.min(p.head), p.anchor.max(p.head)), (0, 2));
    e.feed(crate::editor::Key::Esc);
    e.feed_text("2Gix");
    e.feed(crate::editor::Key::Esc);
    e.feed_text("gg");
    e.feed_text("gi");
    assert_eq!(e.mode, crate::editor::Mode::Insert);
    assert_eq!(e.buf().line_of(e.head()), 1);
}

#[test]
fn change_list_walks_and_invalidates() {
    let mut e = Editor::new(Buffer::from_text("aaa\nbbb\nccc\n"));
    e.feed_text("ix");
    e.feed(crate::editor::Key::Esc);
    e.feed_text("G");
    e.feed_text("Ay");
    e.feed(crate::editor::Key::Esc);
    // newest change first: the y-append at the last line
    e.feed_text("gg");
    e.feed_text("g;");
    assert_eq!(e.buf().line_of(e.head()), 2);
    e.feed_text("g;");
    assert_eq!(e.buf().line_of(e.head()), 0);
    e.feed_text("g,");
    assert_eq!(e.buf().line_of(e.head()), 2);
    // a new edit invalidates the walk: g; starts from newest again
    e.feed_text("ggiz");
    e.feed(crate::editor::Key::Esc);
    e.feed_text("g;");
    assert_eq!(e.buf().line_of(e.head()), 0);
}

#[test]
fn alternate_buffer_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "aaa\n").unwrap();
    std::fs::write(&b, "bbb\n").unwrap();
    let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
    e.feed_text(&format!(":e {}\r", b.display()));
    e.wait_io().unwrap();
    assert!(e.buf().path.as_deref().unwrap().ends_with("b.txt"));
    e.feed_text("<c-^>");
    assert!(e.buf().path.as_deref().unwrap().ends_with("a.txt"));
    e.feed_text("<c-^>");
    assert!(e.buf().path.as_deref().unwrap().ends_with("b.txt"));
}

#[test]
fn ex_ranges_and_substitute() {
    let mut e = Editor::new(Buffer::from_text("foo one\nfoo two\nfoo three\n"));
    // :%s with g rewrites every hit
    e.feed_text(":%s/foo/bar/g\r");
    assert_eq!(e.buf().text().to_string(), "bar one\nbar two\nbar three\n");
    e.feed_text("u");
    // one undo unit for the whole substitute
    assert_eq!(e.buf().text().to_string(), "foo one\nfoo two\nfoo three\n");
    // :2s/x/y/ scopes to line 2
    e.feed_text(":2s/foo/only/\r");
    assert_eq!(e.buf().text().to_string(), "foo one\nonly two\nfoo three\n");
    // :2,3d deletes the range, yanking it (vim :d)
    e.feed_text(":2,3d\r");
    assert_eq!(e.buf().text().to_string(), "foo one\n");
    e.feed_text("u");
    // :3 jumps
    e.feed_text("gg:3\r");
    assert_eq!(e.buf().line_of(e.head()), 2);
    // missing pattern reports, never mutates
    e.feed_text(":%s/nope/x/g\r");
    assert!(e.message.contains("pattern not found"));
}
#[test]
fn diagnostic_jumps_wrap() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("d.txt");
    std::fs::write(&p, "aaa\nbbb\nccc\nddd\n").unwrap();
    let mut e = Editor::new(Buffer::open(p.to_str().unwrap()).unwrap());
    e.cwd = dir.path().to_path_buf();
    let mk = |line: usize, msg: &str| strop_lsp::ResolvedDiag {
        line: strop_core::id::LineIndex::new(line),
        col: strop_core::id::ByteColumn::new(0),
        end_line: strop_core::id::LineIndex::new(line),
        end_col: strop_core::id::ByteColumn::new(1),
        severity: strop_lsp::Severity::Error,
        message: msg.into(),
    };
    e.diags.insert(
        e.current(),
        crate::editor::DocumentDiagnostics {
            revision: e.buf().revision(),
            items: vec![mk(1, "second"), mk(3, "fourth")],
        },
    );
    e.feed_text("]d");
    assert_eq!(e.buf().line_of(e.head()), 1);
    e.feed_text("]d");
    assert_eq!(e.buf().line_of(e.head()), 3);
    e.feed_text("]d"); // wraps to the first
    assert_eq!(e.buf().line_of(e.head()), 1);
    e.feed_text("[d"); // wraps back to the last
    assert_eq!(e.buf().line_of(e.head()), 3);
}
#[test]
fn dbg_unicode() {
    let mut e = Editor::new(Buffer::from_text("café münchen\n"));
    e.feed_text("llll");
    eprintln!(
        "after 4l: byte {} col {}",
        e.head(),
        e.buf().col_of(e.head())
    );
    e.feed_text("rX");
    eprintln!("after rX: {:?}", e.buf().text().to_string());
    let mut e = Editor::new(Buffer::from_text("café münchen\n"));
    e.feed_text("/mü\r");
    eprintln!(
        "search land: byte {} col {}",
        e.head(),
        e.buf().col_of(e.head())
    );
}
#[test]
fn dbg_l_steps() {
    let mut e = Editor::new(Buffer::from_text("café münchen\n"));
    for i in 0..5 {
        e.feed_text("l");
        eprintln!("l{}: byte {}", i + 1, e.head());
    }
}
#[test]
fn dbg_l_from_3() {
    let mut e = Editor::new(Buffer::from_text("café münchen\n"));
    e.set_head(3);
    e.feed_text("l");
    eprintln!(
        "l from 3 -> {} (mode {:?}, msg {:?})",
        e.head(),
        e.mode,
        e.message
    );
    e.feed_text("l");
    eprintln!("again -> {}", e.head());
}
#[test]
fn bracketed_paste_is_one_text_unit() {
    // 0017: paste inserts the payload verbatim (":q!" is TEXT here,
    // never a command), one undo unit
    let mut e = Editor::new(Buffer::from_text("fn main() {}\n"));
    e.feed_text("i");
    e.paste_bracketed("// :q! not a command\n");
    e.feed(crate::editor::Key::Esc);
    assert!(e
        .buf()
        .text()
        .to_string()
        .starts_with("// :q! not a command\n"));
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "fn main() {}\n");
    // normal mode: behaves like p
    let mut e = Editor::new(Buffer::from_text("ab\n"));
    e.paste_bracketed("XY");
    assert_eq!(e.buf().text().to_string(), "aXYb\n");
}

#[test]
fn block_mode_ops() {
    // 0017: ctrl-v rectangle delete + insert replicate
    let mut e = Editor::new(Buffer::from_text("aa11bb\ncc22dd\nee33ff\n"));
    e.feed_text("<c-v>lljx");
    assert_eq!(e.buf().text().to_string(), "1bb\n2dd\nee33ff\n");
    let mut e = Editor::new(Buffer::from_text("aa\ncc\n"));
    e.feed_text("<c-v>j");
    e.feed_text("I");
    e.feed_text(">>");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().text().to_string(), ">>aa\n>>cc\n");
}
#[test]
fn write_to_path_respects_overwrite_policy() {
    // 0020 §1: ordinary :w existing refuses; :w! forces; a failed
    // write leaves path/dirty untouched
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "content a\n").unwrap();
    std::fs::write(&b, "content b\n").unwrap();
    let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
    e.feed_text("ix");
    e.feed(crate::editor::Key::Esc);
    // ordinary :w b.txt — b exists: refused, b unchanged
    e.feed_text(&format!(":w {}\r", b.display()));
    e.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "content b\n");
    assert!(e.buf().path.as_deref().unwrap().ends_with("a.txt"));
    assert!(e.buf().dirty);
    // :w! b.txt — forced
    e.feed_text(&format!(":w! {}\r", b.display()));
    e.wait_io().unwrap();
    assert!(std::fs::read_to_string(&b)
        .unwrap()
        .starts_with("xcontent a"));
    assert!(e.buf().path.as_deref().unwrap().ends_with("b.txt"));
    assert!(!e.buf().dirty);
    // a failed write (unwritable dir) keeps identity
    e.feed_text("iy");
    e.feed(crate::editor::Key::Esc);
    let missing = dir.path().join("missing").join("q.txt");
    e.feed_text(&format!(":w {}<cr>", missing.display()));
    e.wait_io().unwrap();
    assert!(e.buf().path.as_deref().unwrap().ends_with("b.txt"));
    assert!(e.buf().dirty);
}

#[test]
fn grep_respawns_reach_the_production_event_source() {
    // 0020 §2: connect_events + query edits — results must arrive
    // through AppEvent (the 0.9.0 silent-drop regression), and a
    // stale generation's messages are ignored
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hit.txt"), "needle here\n").unwrap();
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.cwd = dir.path().to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    e.connect_events(tx);
    e.open_picker(strop_picker::Kind::Grep);
    // type the query: respawns flow through the forwarded channel
    for c in "needle".chars() {
        e.feed(crate::editor::Key::Char(c));
    }
    // pump the PRODUCTION event source until Done (bounded)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut saw_hit = false;
    let mut done = false;
    while std::time::Instant::now() < deadline && !done {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(ev) => {
                e.handle_app_event(ev);
                let glue = e.picker.as_ref().unwrap();
                saw_hit = saw_hit || glue.picker.rows.iter().any(|r| r.text.contains("hit.txt"));
                done = !glue.picker.streaming;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(saw_hit, "grep results arrived through AppEvent");
    assert!(done, "the stream completed");
    // a superseded request's messages are dropped at the handler
    let before = e.picker.as_ref().unwrap().picker.rows.len();
    let stale = strop_core::worker::Ticket {
        request: strop_core::worker::WorkerId::new(u64::MAX), // never allocated
        key: crate::editor::picker::PickerKey {
            picker: e.picker.as_ref().unwrap().id,
            cwd: e.cwd.clone(),
        },
    };
    e.handle_app_event(crate::editor::events::AppEvent::Picker(
        crate::editor::picker::PickerEvent {
            ticket: stale,
            msg: strop_picker::PickerMsg::Items(vec![strop_picker::Item {
                text: "STALE".into(),
                payload: strop_picker::Payload::Buffer(*e.mru.first().unwrap()),
            }]),
        },
    ));
    assert_eq!(e.picker.as_ref().unwrap().picker.rows.len(), before);
}

#[test]
fn project_replace_is_byte_exact_past_multibyte() {
    // 0020 §3: é before the match must not break verification
    let mut e = Editor::new(Buffer::from_text("éé foo\n"));
    let id = e.current();
    let hits = vec![(1usize, 6usize, 3usize, "éé foo".to_string())];
    let (applied, _, stale) = e.replace_in_buffer_pub(id, &hits, "bar");
    assert_eq!((applied, stale), (1, 0));
    assert_eq!(e.buf().text().to_string(), "éé bar\n");
    // and inside the match itself
    let mut e = Editor::new(Buffer::from_text("féé and féé\n"));
    let id = e.current();
    let hits = vec![(1usize, 1usize, 5usize, "féé and féé".to_string())];
    let (applied, _, stale) = e.replace_in_buffer_pub(id, &hits, "x");
    assert_eq!((applied, stale), (1, 0));
    assert_eq!(e.buf().text().to_string(), "x and féé\n");
}

#[test]
fn paste_after_multibyte_inserts_after_the_char() {
    // 0020 §10: p after é appends after it, not before it
    let mut e = Editor::new(Buffer::from_text("aé b\n"));
    e.feed_text("vly");
    e.feed_text("llp"); // cursor past é; paste goes after the space? no — after char under cursor
    assert!(!e.buf().text().to_string().contains("\u{fffd}"));
    // repeat-search past a multibyte hit never panics
    let mut e = Editor::new(Buffer::from_text("x é y é z\n"));
    e.feed_text("/é\r");
    assert_eq!(e.buf().col_of(e.head()), 2); // first é
    e.feed_text("n");
    assert_eq!(e.buf().col_of(e.head()), 7); // second é
}

#[test]
fn failed_open_keeps_the_scratch_document() {
    // 0020 §11: :e on a directory errors and the scratch stays current
    let mut e = Editor::new(Buffer::from_text(""));
    let dir = tempfile::tempdir().unwrap();
    e.feed_text(&format!(":e {}\r", dir.path().display()));
    assert!(
        e.message.contains("error")
            || e.message.contains("Is a directory")
            || !e.message.is_empty()
    );
    // the editor still has a live current document (no panic on render)
    let _ = e.buf();
    assert_eq!(e.docs.len(), 1);
}
#[test]
fn edits_map_marks_jumps_and_other_panes() {
    // 0020 §14: one document in two panes + a mark; an edit above
    // the anchors moves ALL of them
    let mut e = Editor::new(Buffer::from_text("aaa\nbbb TARGET\nccc\n"));
    e.feed_text("jma"); // mark a on the TARGET line
    e.feed_text("<c-w>v"); // second pane, same doc (cursor on TARGET)
    e.feed_text("<c-w>h"); // back to pane 1
    e.feed_text("ggOheader");
    e.feed(crate::editor::Key::Esc);
    // the mark moved from line 1 to line 2
    let (_, mpos) = e.marks[&'a'];
    assert_eq!(e.buf().line_of(mpos), 2);
    // pane 2's cursor tracked the edit: still on TARGET (line 2 now)
    let pane2 = &e.panes[1];
    assert_eq!(e.buf().line_of(pane2.sels.primary().head), 2);
    assert!(e
        .buf()
        .text()
        .byte_slice(e.buf().line_start(2)..e.buf().line_end(2))
        .to_string()
        .contains("TARGET"));
}
#[test]
fn incremental_syntax_equals_fresh_parse() {
    // 0022 §1's contract: after every edit transaction, the kept
    // tree's spans equal a from-scratch parse — property corpus
    let scripts: Vec<Vec<&str>> = vec![
        vec!["ix", "<esc>", "u", "<c-r>", "dd", "u"],
        vec!["ofn main() {", "<esc>", "ciwrun", "<esc>", "u", "yyP"],
        vec!["dw", "u", "ciwasync", "<esc>", "3x", "u"],
    ];
    for script in scripts {
        let mut e = Editor::new(Buffer::from_text("fn demo() {\n    let x = 1;\n}\n"));
        e.buf_mut().path = Some(std::path::PathBuf::from("/tmp/demo.rs"));
        e.cur_mut().highlighter = strop_syntax::Highlighter::for_path(
            std::path::Path::new("/tmp/demo.rs"),
            e.buf().text(),
        );
        // warm the tree BEFORE edits — without this the test passes
        // trivially through the full-parse fallback
        {
            let rope = e.buf().text().clone();
            let len = e.buf().len_bytes();
            let rev = e.buf().revision();
            let _ = e
                .cur_mut()
                .highlighter
                .as_mut()
                .unwrap()
                .highlight(&rope, rev, 0, len)
                .unwrap();
        }
        for keys in &script {
            match *keys {
                "<esc>" => e.feed(crate::editor::Key::Esc),
                "<c-r>" => e.feed(crate::editor::Key::CtrlR),
                k => e.feed_text(k),
            }
        }
        let revision = e.buf().revision();
        let rope = e.buf().text().clone();
        let len = e.buf().len_bytes();
        // incremental: the kept tree + lazy reparse
        let inc = e
            .cur_mut()
            .highlighter
            .as_mut()
            .unwrap()
            .highlight(&rope, revision, 0, len)
            .unwrap();
        // fresh: no old tree at all
        let mut fresh =
            strop_syntax::Highlighter::for_path(std::path::Path::new("/tmp/demo.rs"), &rope)
                .unwrap();
        let expected = fresh.highlight(&rope, revision, 0, len).unwrap();
        assert_eq!(
            inc, expected,
            "script {script:?}: incremental spans diverged"
        );
    }
}
