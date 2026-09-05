//! Editing-behavior contract tests (0003 §5, 0006 tier 1): state
//! assertions drive the real Editor — no terminal, no timing.

pub use super::*;

#[cfg(test)]
mod reviewer_battery {
    use super::*;

    #[test]
    fn counts_multiply_non_operator_commands() {
        // 2x deletes two chars
        let mut e = Editor::new(Buffer::from_text("abcde\n"));
        e.feed_text("2x");
        assert_eq!(e.buf().rope.to_string(), "cde\n");
        // 2u undoes twice
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\n"));
        e.feed_text("dd");
        e.feed_text("dd");
        assert_eq!(e.buf().rope.to_string(), "c\n");
        e.feed_text("2u");
        assert_eq!(e.buf().rope.to_string(), "a\nb\nc\n");
        // 3rx replaces three
        let mut e = Editor::new(Buffer::from_text("abcde\n"));
        e.feed_text("3rx");
        assert_eq!(e.buf().rope.to_string(), "xxxde\n"); // 3 chars, not 4
                                                         // 2p pastes twice
        let mut e = Editor::new(Buffer::from_text("ab\n"));
        e.feed_text("yiw");
        e.feed_text("2p");
        assert_eq!(e.buf().rope.to_string(), "aababb\n"); // register twice at one spot
    }

    #[test]
    fn counted_insert_repeats_text() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text("3iZ");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "ZZZx\n"); // vim: i inserts before
        e.feed_text("u"); // one undo unit for the whole counted insert
        assert_eq!(e.buf().rope.to_string(), "x\n");
    }

    #[test]
    fn cc_keeps_the_line_and_indent() {
        let mut e = Editor::new(Buffer::from_text("one\n  two\nthree\n"));
        e.feed_text("jccX");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "one\n  X\nthree\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "one\n  two\nthree\n");
        // the empty case: line empties, never merges
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("jcc");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "one\n\nthree\n");
    }

    #[test]
    fn c_enters_insert_at_deletion_start() {
        // hello world: w → 6, C deletes "world", cursor stays at 6,
        // typed text lands after the space
        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text("wCthere");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "hello there\n");
        let mut e = Editor::new(Buffer::from_text("abcdef\n"));
        e.feed_text("3lCX");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "abcX\n");
    }

    #[test]
    fn i_caret_tilde_s_work() {
        let mut e = Editor::new(Buffer::from_text("    let x = 1;\n"));
        e.feed_text("0^"); // ^ → first non-blank (4)
        assert_eq!(e.head(), 4);
        let mut e = Editor::new(Buffer::from_text("    let x = 1;\n"));
        e.feed_text("I"); // insert at first non-blank
        assert_eq!(e.mode, crate::editor::Mode::Insert);
        assert_eq!(e.head(), 4);
        e.feed(crate::editor::Key::Esc);
        // ~ toggles case and advances
        let mut e = Editor::new(Buffer::from_text("abc\n"));
        e.feed_text("~~");
        assert_eq!(e.buf().rope.to_string(), "ABc\n");
        // S = cc
        let mut e = Editor::new(Buffer::from_text("one\ntwo\n"));
        e.feed_text("SX");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "X\ntwo\n");
    }

    #[test]
    fn unknown_bare_keys_say_so() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text("="); // not implemented
        assert!(e.message.contains("not an editor command"));
    }
}
#[cfg(test)]
mod scratch_tests {
    use super::*;

    #[test]
    fn first_open_replaces_the_scratch_buffer() {
        // regression: opening over the initial scratch left it behind —
        // :q closed the file, the welcome card showed, you kept quitting
        // tempdir per test: parallel tests sharing one fixture race
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("scratch-test.rs");
        std::fs::write(&f, "fn a() {}\n").unwrap();
        let mut e = Editor::new(Buffer::from_text(""));
        e.open_buffer(f.to_str().unwrap()).unwrap();
        assert_eq!(e.docs.len(), 1, "scratch replaced, not stacked");
        assert_eq!(e.buf().path.as_deref(), f.to_str());
        e.feed_text(":q\r");

        assert!(e.should_quit, "one :q quits");
    }

    #[test]
    fn view_marks_readonly_and_edits_refuse() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\n"));
        e.feed_text(":view\r");
        assert!(e.buf().readonly);
        e.feed_text("x");
        assert_eq!(e.buf().rope.to_string(), "one\ntwo\n", "no edit landed");
        assert!(e.message.contains("readonly"));
    }

    #[test]
    fn edited_scratch_survives() {
        let mut e = Editor::new(Buffer::from_text(""));
        e.feed_text("ix"); // scratch has content now
        e.feed(crate::editor::Key::Esc);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("scratch-test.rs");
        std::fs::write(&f, "fn a() {}\n").unwrap();
        e.open_buffer(f.to_str().unwrap()).unwrap();
        assert_eq!(e.docs.len(), 2, "edited scratch is real work");
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    fn editor_with(text: &str) -> Editor {
        Editor::new(Buffer::from_text(text))
    }

    fn text(e: &Editor) -> String {
        e.buf().rope.to_string()
    }

    #[test]
    fn named_registers_yank_and_paste() {
        let mut e = editor_with("alpha\nbeta\ngamma\n");

        e.feed_text("\"ayy"); // yank line into register a
        assert_eq!(e.register(Some('a')).0, "alpha\n");
        e.feed_text("j");
        e.feed_text("\"ap"); // paste a below beta
        assert_eq!(text(&e), "alpha\nbeta\nalpha\ngamma\n");
        // unnamed register untouched
        assert!(e.register(None).0.is_empty());
    }

    #[test]
    fn space_y_yanks_motion_to_system_register() {
        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text(" yw");
        assert_eq!(e.register(Some('+')).0, "hello ");
        assert!(e.osc52.is_some(), "OSC52 payload staged for the TUI");
    }

    #[test]
    fn visual_space_y_yanks_selection_to_system_register() {
        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text("vl y");
        assert_eq!(e.register(Some('+')).0, "he");
        assert!(e.osc52.is_some());
    }

    #[test]
    fn clipboard_paste_inserts_read_result() {
        let mut e = Editor::new(Buffer::from_text("ab\n"));
        e.clip_paste_pending = Some(false);
        e.clip_tx.send(Some("XY".into())).unwrap();
        e.drain_clipboard();
        assert_eq!(e.buf().rope.to_string(), "aXYb\n");
    }

    #[test]
    fn clipboard_paste_reports_missing_provider() {
        let mut e = Editor::new(Buffer::from_text("ab\n"));
        e.clip_paste_pending = Some(false);
        e.clip_tx.send(None).unwrap();
        e.drain_clipboard();
        assert!(e.message.contains("clipboard"));
        assert_eq!(e.buf().rope.to_string(), "ab\n");
    }

    #[test]
    fn alias_verbs() {
        let mut e = editor_with("let edge = hone;\n");
        e.feed_text("0wD"); // delete from 'edge' to EOL
        assert_eq!(text(&e), "let \n");
        let mut e = editor_with("let x = 1;\n");
        e.feed_text("0wY"); // yy
        assert_eq!(e.register(None).0, "let x = 1;\n");
        let mut e = editor_with("abc\n");
        e.feed_text("sZ"); // cl + insert Z
        e.feed(crate::editor::Key::Esc);
        assert_eq!(text(&e), "Zbc\n");
    }

    #[test]
    fn replace_char_and_join() {
        let mut e = editor_with("abc\ndef\n");
        e.feed_text("rX");
        assert_eq!(text(&e), "Xbc\ndef\n");
        e.feed_text("J");
        assert_eq!(text(&e), "Xbc def\n");
    }

    #[test]
    fn indent_and_dedent() {
        let mut e = editor_with("a\nb\nc\n");
        e.feed_text("2>>");
        assert_eq!(text(&e), "    a\n    b\nc\n");
        e.feed_text("0<<");
        assert_eq!(text(&e), "a\n    b\nc\n");
    }

    #[test]
    fn dot_repeat_replays_insert() {
        let mut e = editor_with("one\ntwo\n");
        e.feed_text("A!");
        e.feed(crate::editor::Key::Esc);
        e.feed_text("j.");
        assert_eq!(text(&e), "one!\ntwo!\n");
    }

    #[test]
    fn visual_line_deletes_whole_lines() {
        let mut e = editor_with("a\nb\nc\nd\n");
        e.feed_text("Vjd");
        assert_eq!(text(&e), "c\nd\n");
        assert!(e.register(None).1); // linewise
        e.feed_text("P");
        assert_eq!(text(&e), "a\nb\nc\nd\n"); // paste linewise above
    }

    #[test]
    fn paste_is_one_undo_unit() {
        // regression: a lone paste never committed its revision — `u`
        // after yank+paste said "already at oldest change"
        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text("yiw"); // yank "hello"
        e.feed_text("ep"); // paste after the word: "hellohello world"

        assert_eq!(e.buf().rope.to_string(), "hellohello world\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "hello world\n");
    }

    #[test]
    fn semicolon_and_comma_repeat_find() {
        let mut e = Editor::new(Buffer::from_text("a.b.c.d\n"));
        e.feed_text("f."); // find first '.'
        assert_eq!(e.head(), 1);
        e.feed_text(";");
        assert_eq!(e.head(), 3);
        e.feed_text(";");
        assert_eq!(e.head(), 5);
        e.feed_text(","); // reverse
        assert_eq!(e.head(), 3);
    }

    #[test]
    fn star_searches_word_under_cursor_whole_word() {
        let mut e = Editor::new(Buffer::from_text("hone honed hone\n"));
        e.feed_text("*"); // on "hone" at 0 → next whole-word match at 11
        assert_eq!(e.head(), 11);
        e.feed_text("n"); // wraps to 0
        assert_eq!(e.head(), 0);
        e.feed_text("#"); // backward: wraps to 11
        assert_eq!(e.head(), 11);
        // whole-word: "honed" is skipped as a match for "hone" — the
        // only other candidate, so the search wraps back to 0
        let mut e = Editor::new(Buffer::from_text("hone honed\n"));
        e.feed_text("*");
        assert_eq!(e.head(), 0, "honed is not a whole-word match for hone");
    }

    #[test]
    fn count_motions_and_ex_line_jump() {
        // 30j: the 0 after a count digit is a digit, not line-start
        let mut e = Editor::new(Buffer::from_text("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n"));
        e.feed_text("10j");
        assert_eq!(e.buf().line_of(e.head()), 10);
        // :30 jumps (and clamps past EOF)
        e.feed_text(":30\r");
        assert_eq!(e.buf().line_of(e.head()), 11);
        e.feed_text(":4\r");
        assert_eq!(e.buf().line_of(e.head()), 3);
    }

    #[test]
    fn visual_indent_and_dedent() {
        let mut e = Editor::new(Buffer::from_text("a\nb\nc\n"));
        e.feed_text("Vj>");
        assert_eq!(e.buf().rope.to_string(), "    a\n    b\nc\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "a\nb\nc\n");
        e.feed_text("Vj>");
        e.feed_text("Vj<");
        assert_eq!(e.buf().rope.to_string(), "a\nb\nc\n");
    }

    #[test]
    fn noh_clears_search_highlight() {
        let mut e = Editor::new(Buffer::from_text("foo bar\n"));
        e.feed_text("/foo\r");
        assert!(e.last_search.is_some());
        e.feed_text(":noh\r");
        assert!(e.last_search.is_none());
    }

    #[test]
    fn dot_repeats_delete_and_change() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed_text("dd");
        assert_eq!(e.buf().rope.to_string(), "two\nthree\n");
        e.feed_text(".");
        assert_eq!(e.buf().rope.to_string(), "three\n");
        let mut e = Editor::new(Buffer::from_text("aa bb\ncc dd\n"));
        e.feed_text("cwX");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "X bb\ncc dd\n");
        e.feed_text("j"); // to line 2 — repeat there
        e.feed_text(".");
        assert_eq!(e.buf().rope.to_string(), "X bb\nX dd\n");
    }

    #[test]
    fn last_search_highlights_persistently() {
        let mut e = Editor::new(Buffer::from_text("foo bar foo\n"));
        e.feed_text("/foo\r");
        // committed: no pending pattern, but hits must still compute
        assert!(e.search_pattern().is_none());
        assert_eq!(e.last_search.as_ref().unwrap().pattern, "foo");
        let frame = crate::headless::frame_string(&mut e, 40, 8);
        assert!(frame.contains("foo bar foo"));
    }

    #[test]
    fn empty_search_repeats_last() {
        // vim: bare / repeats the last search in its direction, bare ?
        // reverses it — never an "editor command" error
        let mut e = Editor::new(Buffer::from_text("aa line\nbb line\ncc line\n"));
        e.feed_text("/line\r");
        assert_eq!(e.buf().line_of(e.head()), 0);
        e.feed_text("/\r");
        assert_eq!(e.buf().line_of(e.head()), 1, "empty / repeats forward");
        e.feed_text("/\r");
        assert_eq!(e.buf().line_of(e.head()), 2);
        e.feed_text("?\r");
        assert_eq!(e.buf().line_of(e.head()), 1, "empty ? reverses");
        assert!(e.message.is_empty());
    }

    #[test]
    fn x_on_multibyte_char_deletes_the_whole_char() {
        // regression: x deleted cursor..cursor+1 raw bytes — on é that
        // split the char and ropey panicked (unicode crash)
        let mut e = Editor::new(Buffer::from_text("héllo\n"));
        e.feed_text("lx"); // onto é, delete it whole
        assert_eq!(e.buf().rope.to_string(), "hllo\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "héllo\n");
        e.feed_text("a"); // append lands past the char, not mid-char
        assert!(e.buf().is_boundary(e.head()));
    }

    #[test]
    fn arrows_speak_hjkl() {
        // the translation layer used to drop KeyCode::Up/Down — arrows
        // did nothing anywhere (user report: picker nav needed Tab)
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        e.feed(crate::editor::Key::Down);
        assert_eq!(e.buf().line_of(e.head()), 1, "Down is j");
        e.feed(crate::editor::Key::Right);
        assert_eq!(e.head(), e.buf().line_start(1) + 1, "Right is l");
        e.feed(crate::editor::Key::Up);
        assert_eq!(e.buf().line_of(e.head()), 0, "Up is k");
        e.feed(crate::editor::Key::Left);
        assert_eq!(e.head(), 0, "Left is h");
    }

    #[test]
    fn wq_never_closes_a_failed_save() {
        // 0014 P0: a disk error or external change must keep the buffer
        // open and dirty — closing would be silent data loss
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("wq.txt");
        std::fs::write(&f, "one\n").unwrap();
        let mut e = Editor::new(Buffer::open(f.to_str().unwrap()).unwrap());
        e.feed_text("ix");
        e.feed(crate::editor::Key::Esc);
        std::fs::write(&f, "external\n").unwrap(); // someone else writes
        e.feed_text(":wq\r");
        assert!(!e.should_quit, "failed save must not close");
        assert!(e.buf().dirty, "still dirty");
        assert!(e.message.contains("changed on disk"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "external\n");
        e.feed_text(":wq!\r");
        assert!(e.should_quit, "forced write quits");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "xone\n");
    }
    #[test]
    fn ex_open_and_close_buffers() {
        std::fs::write("/tmp/strop-test-b.rs", "second\n").unwrap();
        let mut e = editor_with("first\n");
        e.feed_text(":e /tmp/strop-test-b.rs<cr>");

        assert_eq!(e.docs.len(), 2);
        assert_eq!(text(&e), "second\n");
        e.feed_text(":q<cr>");
        assert_eq!(e.docs.len(), 1);
        assert_eq!(text(&e), "first\n");
        // dirty buffer refuses :q, allows :q!
        e.feed_text("ix");
        e.feed(crate::editor::Key::Esc);
        e.feed_text(":q<cr>");
        assert_eq!(e.docs.len(), 1);
        assert!(e.message.contains("unsaved"));
        e.feed_text(":q!<cr>");
        assert!(e.should_quit);
    }
}

#[cfg(test)]
mod indent_tests {
    use super::*;

    #[test]
    fn enter_copies_and_deepens_indent() {
        let mut e = Editor::new(Buffer::from_text("fn f() {\n    let x = 1;\n}\n"));
        e.feed_text("j$"); // on the let line, at EOL
        e.feed(crate::editor::Key::Char('a'));
        e.feed(crate::editor::Key::Enter);
        e.feed_text("let y = 2;");
        assert_eq!(
            e.buf().rope.to_string(),
            "fn f() {\n    let x = 1;\n    let y = 2;\n}\n"
        );
        // after an opener, one level deeper
        e.feed(crate::editor::Key::Esc);
        e.feed_text("gg$");
        e.feed(crate::editor::Key::Char('a'));
        e.feed(crate::editor::Key::Enter);
        e.feed_text("// body");
        let got = e.buf().rope.to_string();
        assert!(got.starts_with("fn f() {\n    // body"), "got: {got:?}");
    }

    #[test]
    fn o_auto_indents() {
        let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
        e.feed_text("o");
        e.feed_text("let x = 1;");
        assert_eq!(e.buf().rope.to_string(), "fn f() {\n    let x = 1;\n}\n");
    }

    #[test]
    fn tab_size_from_config() {
        let mut e = Editor::new(Buffer::from_text("a\nb\n"));
        e.config = crate::config::Config {
            tab_size: 2,
            ..Default::default()
        };
        e.feed_text(">>");
        assert_eq!(e.buf().rope.to_string(), "  a\nb\n");
        e.feed_text("<<");
        assert_eq!(e.buf().rope.to_string(), "a\nb\n");
    }

    #[test]
    fn new_file_opens_empty_and_saves() {
        let path = "/tmp/strop-newfile-test.rs";
        std::fs::remove_file(path).ok();
        let mut e = Editor::new(Buffer::open(path).expect("missing file is a new buffer"));
        assert_eq!(e.buf().len_bytes(), 0);
        e.feed_text("ifresh");
        e.feed(crate::editor::Key::Esc);
        e.feed_text(":w<cr>");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "fresh");
        std::fs::remove_file(path).ok();
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::*;

    /// Opens, surfaces, and closes never panic and keep the document
    /// set honest (pre-0.4.1 this pinned the parallel-vectors alignment;
    /// the Document struct made that invariant the type system).
    #[test]
    fn document_set_stays_honest() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("align-a.rs");
        let b = dir.path().join("align-b.rs");
        std::fs::write(&a, "a\n").unwrap();
        std::fs::write(&b, "b\n").unwrap();
        let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
        e.open_buffer(b.to_str().unwrap()).unwrap();
        assert_eq!(e.docs.len(), 2);
        e.open_diff_surface("delta", "f.rs", vec![], None);
        assert_eq!(e.docs.len(), 3);
        assert!(e.cur().surface.is_some());
        e.close_buffer(true);
        assert_eq!(e.docs.len(), 2);
        e.close_buffer(true);
        e.close_buffer(true);
        assert!(e.should_quit, "closing the last document quits");
    }
}

#[cfg(test)]
mod smartindent_tests {
    use super::*;

    #[test]
    fn closer_dedents_on_indent_only_line() {
        // open a line inside fn f() { } — auto-indented, then '}' dedents
        let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
        e.feed_text("o"); // indented to one level
        assert_eq!(e.buf().line_text(1), "    ");
        e.feed_text("}"); // closer on the indent-only line → dedent first
                          // the new line sits at col 0; the file's own closing brace is untouched
        assert_eq!(e.buf().rope.to_string(), "fn f() {\n}\n}\n");
    }

    #[test]
    fn closer_noop_with_real_text_before() {
        let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
        e.feed_text("o"); // indented one level
        e.feed_text("let x = 1;"); // real text on the line
        e.feed_text("}"); // closer after text: no dedent
        assert_eq!(e.buf().line_text(1), "    let x = 1;}");
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;

    #[test]
    fn insert_session_undoes_as_one_unit() {
        let mut e = Editor::new(Buffer::from_text("hello\n"));
        e.feed_text("A world"); // append " world" at EOL
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "hello world\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "hello\n");
        e.feed(crate::editor::Key::CtrlR);
        assert_eq!(e.buf().rope.to_string(), "hello world\n");
    }

    #[test]
    fn change_op_holds_one_undo_unit() {
        let mut e = Editor::new(Buffer::from_text("say [old] now\n"));
        e.feed_text("w"); // onto [old]
        e.feed_text("ci["); // change inside brackets
        e.feed_text("new");
        e.feed(crate::editor::Key::Esc);
        assert_eq!(e.buf().rope.to_string(), "say [new] now\n");
        e.feed_text("u"); // ONE undo restores the whole change
        assert_eq!(e.buf().rope.to_string(), "say [old] now\n");
    }

    #[test]
    fn n_repeats_search_and_wraps() {
        let mut e = Editor::new(Buffer::from_text("foo bar\nfoo baz\n"));
        e.feed_text("/foo\r"); // lands on the *next* match (vim)
        assert_eq!(e.head(), 8);
        e.feed_text("n"); // wraps to the first
        assert_eq!(e.head(), 0);
        e.feed_text("N"); // backward, wraps from top
        assert_eq!(e.head(), 8);
    }

    #[test]
    fn edit_after_undo_forks_and_ctrlr_redoes_last_branch() {
        let mut e = Editor::new(Buffer::from_text("ab\n"));
        e.feed_text("rx"); // replace a with x
        e.feed_text("u");
        e.feed_text("ry"); // fork: replace a with y
        e.feed_text("u"); // back to ab
        e.feed(crate::editor::Key::CtrlR); // redo the last-visited branch
        assert_eq!(e.buf().rope.to_string(), "yb\n");
        // redo once more: nothing (the fork tip is current)
        e.feed(crate::editor::Key::CtrlR);
        assert_eq!(e.buf().rope.to_string(), "yb\n");
    }

    #[test]
    fn readonly_buffers_refuse_undo() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.buf_mut().readonly = true;
        e.feed_text("u");
        assert!(e.message.contains("readonly"));
        assert_eq!(e.buf().rope.to_string(), "x\n");
    }
}

#[cfg(test)]
mod surround_tests {
    use super::*;

    #[test]
    fn ds_deletes_pair() {
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("w"); // onto "hi"
        e.feed_text("ds\"");
        assert_eq!(e.buf().rope.to_string(), "say hi now\n");
        // and undo restores the pair as one unit
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "say \"hi\" now\n");
    }

    #[test]
    fn cs_changes_pair() {
        let mut e = Editor::new(Buffer::from_text("call(a, b)\n"));
        e.feed_text("f("); // onto the open paren (on-pair counts as inside)
        e.feed_text("cs(["); // change (…) to […]
        assert_eq!(e.buf().rope.to_string(), "call[a, b]\n");
        // and undo restores as one unit
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "call(a, b)\n");
    }

    #[test]
    fn ysiw_wraps_word() {
        let mut e = Editor::new(Buffer::from_text("make it sharp\n"));
        e.feed_text("w"); // onto "it"
        e.feed_text("ysiw\"");
        assert_eq!(e.buf().rope.to_string(), "make \"it\" sharp\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "make it sharp\n");
    }

    #[test]
    fn visual_s_wraps_selection() {
        let mut e = Editor::new(Buffer::from_text("wrap me up\n"));
        e.feed_text("ve"); // select "wrap"
        e.feed_text("S(");
        assert_eq!(e.buf().rope.to_string(), "(wrap) me up\n");
    }
}

#[cfg(test)]
mod visual_object_tests {
    use super::*;

    #[test]
    fn vi_paren_selects_inner() {
        let mut e = Editor::new(Buffer::from_text("call(a, b)\n"));
        e.feed_text("f("); // onto the open paren
        e.feed_text("vi(");
        let r = e.visual_range().expect("visual range");
        assert_eq!(e.buf().slice_string(r), "a, b");
        // and operators consume it
        e.feed_text("d");
        assert_eq!(e.buf().rope.to_string(), "call()\n");
    }

    #[test]
    fn va_quote_includes_quotes() {
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("w"); // onto "hi"
        e.feed_text("va\"");
        let r = e.visual_range().expect("visual range");
        assert_eq!(e.buf().slice_string(r), "\"hi\"");
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    #[test]
    fn undo_after_visual_delete() {
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("ved"); // vim: deletes "say", the space stays
        assert_eq!(e.buf().rope.to_string(), " \"hi\" now\n");
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "say \"hi\" now\n");
    }

    #[test]
    fn quote_object_scans_forward_on_the_line() {
        // vim i" special case: cursor before the string uses the next pair
        let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
        e.feed_text("vi\"");
        let r = e.visual_range().expect("selection");
        assert_eq!(e.buf().slice_string(r), "hi");
    }

    #[test]
    fn quit_leaves_editor_drain_safe() {
        // regression: the last :q! emptied the buffer list and the
        // post-feed drain tick panicked indexing buffers[current]
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":q!\r");
        assert!(e.should_quit);
        assert!(e.docs.is_empty());
        e.drain_picker();
        e.drain_git_jobs();
        e.drain_lsp();
        e.lsp_sync_changed();
    }

    #[test]
    fn undo_lands_cursor_at_change_start() {
        // regression: undo took the first replayed op (the tail of the
        // change) — vim lands at the start of the undone region
        let mut e = Editor::new(Buffer::from_text("hello\n"));
        e.feed_text("A world");
        e.feed(crate::editor::Key::Esc);
        e.feed_text("0"); // move away from the change
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "hello\n");
        // the change started at byte 5 (" world"); normal-mode clamp
        // pulls 5 onto the last char of the line
        assert_eq!(e.head(), 4);
    }
}

#[cfg(test)]
mod keybinds_tests {
    use super::*;

    #[test]
    fn marks_set_and_jump() {
        std::fs::write("/tmp/strop-mark-a.rs", "one\ntwo\nthree\n").unwrap();
        std::fs::write("/tmp/strop-mark-b.rs", "alpha\nbeta\n").unwrap();
        let mut e = Editor::new(Buffer::open("/tmp/strop-mark-a.rs").unwrap());
        e.feed_text("jj"); // line 3
        e.feed_text("mb"); // mark b here
        e.feed_text(":e /tmp/strop-mark-b.rs<cr>");
        e.feed_text("'b"); // jump back to mark
        assert_eq!(e.buf().path.as_deref(), Some("/tmp/strop-mark-a.rs"));
        assert_eq!(e.buf().line_of(e.head()), 2);
        std::fs::remove_file("/tmp/strop-mark-a.rs").ok();
        std::fs::remove_file("/tmp/strop-mark-b.rs").ok();
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
        assert_eq!(e.buf().rope.to_string(), "two three\n");
    }

    #[test]
    fn arrows_consume_pending_counts() {
        // 0015: 2 <Right> x — the count moves twice and clears; x is 1
        let mut e = Editor::new(Buffer::from_text("hello world\n"));
        e.feed_text("2");
        e.feed(crate::editor::Key::Right);
        assert_eq!(e.buf().col_of(e.head()), 2);
        e.feed_text("x");
        assert_eq!(e.buf().rope.to_string(), "helo world\n");
    }

    #[test]
    fn pathless_save_is_an_error_not_a_lie() {
        // 0015: :w on a scratch must never report "written"
        let mut e = Editor::new(Buffer::from_text("unsaved\n"));
        e.feed_text(":w\r");
        assert!(e.message.contains("no file name"), "{}", e.message);
        // :wq must not close the dirty scratch either
        e.feed_text(":wq\r");
        assert_eq!(e.buf().rope.to_string(), "unsaved\n");
        // :w {path} names it and persists
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("named.txt");
        e.feed_text(&format!(":w {}\r", p.display()));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "unsaved\n");
        assert_eq!(e.buf().path.as_deref(), Some(p.to_str().unwrap()));
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
        for _ in 0..200 {
            e.drain_shell();
            if e.message.starts_with("pipe failed") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(e.buf().rope.to_string(), "keep me\n");
        assert!(e.message.starts_with("pipe failed"), "{}", e.message);
    }
}
