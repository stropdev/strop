use strop_core::worker::{Completion, FailureKind, Outcome};

use super::*;

fn editor_with(text: &str) -> Editor {
    Editor::new(Buffer::from_text(text))
}

fn text(e: &Editor) -> String {
    e.buf().text().to_string()
}

#[test]
fn named_registers_yank_and_paste() {
    let mut e = editor_with("alpha\nbeta\ngamma\n");

    e.feed_text("\"ayy"); // yank line into register a
    assert_eq!(e.register(Some('a')).text, "alpha\n");
    e.feed_text("j");
    e.feed_text("\"ap"); // paste a below beta
    assert_eq!(text(&e), "alpha\nbeta\nalpha\ngamma\n");
    // unnamed register untouched
    assert!(e.register(None).text.is_empty());
}

#[test]
fn space_y_yanks_motion_to_system_register() {
    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text(" yw");
    assert_eq!(e.register(Some('+')).text, "hello ");
    assert!(e.osc52.is_some(), "OSC52 payload staged for the TUI");
}

#[test]
fn visual_space_y_yanks_selection_to_system_register() {
    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text("vl y");
    assert_eq!(e.register(Some('+')).text, "he");
    assert!(e.osc52.is_some());
}

#[test]
fn clipboard_paste_inserts_read_result() {
    let mut e = Editor::new(Buffer::from_text("ab\n"));
    let ticket = clipboard_ticket(&mut e);
    e.clip_paste_pending = Some((false, ticket.clone()));
    e.clip_tx
        .send(Completion {
            ticket,
            outcome: Outcome::Success("XY".into()),
        })
        .unwrap();
    e.drain_clipboard();
    assert_eq!(e.buf().text().to_string(), "aXYb\n");
}

#[test]
fn clipboard_paste_reports_missing_provider() {
    let mut e = Editor::new(Buffer::from_text("ab\n"));
    let ticket = clipboard_ticket(&mut e);
    e.clip_paste_pending = Some((false, ticket.clone()));
    e.clip_tx
        .send(Completion {
            ticket,
            outcome: Outcome::failed(FailureKind::Unavailable, "no clipboard provider succeeded"),
        })
        .unwrap();
    e.drain_clipboard();
    assert!(e.message.contains("clipboard"));
    assert_eq!(e.buf().text().to_string(), "ab\n");
}

#[test]
fn clipboard_stale_ticket_never_pastes_into_newer_request() {
    let mut e = Editor::new(Buffer::from_text("ab\n"));
    let stale = clipboard_ticket(&mut e);
    e.clip_paste_pending = Some((false, stale.clone()));
    // a newer request superseded it before the stale result landed
    let fresh = clipboard_ticket(&mut e);
    e.clip_paste_pending = Some((false, fresh));
    e.clip_tx
        .send(Completion {
            ticket: stale,
            outcome: Outcome::Success("POISON".into()),
        })
        .unwrap();
    e.drain_clipboard();
    assert_eq!(
        e.buf().text().to_string(),
        "ab\n",
        "stale clipboard result must not paste over a newer request"
    );
    assert!(e.clip_paste_pending.is_some(), "newer request survives");
}

fn clipboard_ticket(e: &mut Editor) -> strop_core::worker::Ticket<crate::editor::ClipboardKey> {
    strop_core::worker::Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: crate::editor::ClipboardKey {
            document: e.current(),
        },
    }
}

#[test]
fn alias_verbs() {
    let mut e = editor_with("let edge = hone;\n");
    e.feed_text("0wD"); // delete from 'edge' to EOL
    assert_eq!(text(&e), "let \n");
    let mut e = editor_with("let x = 1;\n");
    e.feed_text("0wY"); // yy
    assert_eq!(e.register(None).text, "let x = 1;\n");
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
    assert_eq!(
        e.register(None).shape,
        crate::editor::registers::RegisterShape::Linewise
    );
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

    assert_eq!(e.buf().text().to_string(), "hellohello world\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "hello world\n");
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
    assert_eq!(e.buf().text().to_string(), "    a\n    b\nc\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "a\nb\nc\n");
    e.feed_text("Vj>");
    e.feed_text("Vj<");
    assert_eq!(e.buf().text().to_string(), "a\nb\nc\n");
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
    assert_eq!(e.buf().text().to_string(), "two\nthree\n");
    e.feed_text(".");
    assert_eq!(e.buf().text().to_string(), "three\n");
    let mut e = Editor::new(Buffer::from_text("aa bb\ncc dd\n"));
    e.feed_text("cwX");
    e.feed(crate::editor::Key::Esc);
    assert_eq!(e.buf().text().to_string(), "X bb\ncc dd\n");
    e.feed_text("j"); // to line 2 — repeat there
    e.feed_text(".");
    assert_eq!(e.buf().text().to_string(), "X bb\nX dd\n");
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
    assert_eq!(e.buf().text().to_string(), "hllo\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "héllo\n");
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
    std::fs::File::options()
        .write(true)
        .open(&f)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(123))
        .unwrap();
    e.feed_text(":wq\r");
    e.wait_io().unwrap();
    assert!(!e.should_quit, "failed save must not close");
    assert!(e.buf().dirty, "still dirty");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "external\n");
    e.feed_text(":wq!\r");
    e.wait_io().unwrap();
    assert!(e.should_quit, "forced write quits");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "xone\n");
}
#[test]
fn ex_open_and_close_buffers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("second.txt");
    std::fs::write(&path, "second\n").unwrap();
    let mut e = editor_with("first\n");
    e.feed_text(&format!(":e {}<cr>", path.display()));
    e.wait_io().unwrap();

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
