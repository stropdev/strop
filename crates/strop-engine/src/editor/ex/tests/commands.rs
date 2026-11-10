//! Names, ranges and their refusals; `:normal` as a function; repeats.

use super::super::table::{lookup, COMMANDS};
use super::{editor, text};
use crate::editor::Mode;

#[test]
fn vims_abbreviations_name_their_commands() {
    for (typed, name) in [
        ("s", "substitute"),
        ("su", "substitute"),
        ("norm", "normal"),
        ("g", "global"),
        ("v", "vglobal"),
        ("d", "delete"),
        ("y", "yank"),
        ("m", "move"),
        ("t", "t"),
        ("co", "copy"),
        ("j", "join"),
        ("pu", "put"),
        ("sor", "sort"),
        ("k", "k"),
        ("ma", "mark"),
        ("delm", "delmarks"),
        ("u", "undo"),
        ("red", "redo"),
        ("b", "buffer"),
        ("bn", "bnext"),
        ("bp", "bprevious"),
        ("bN", "bNext"),
        ("bd", "bdelete"),
        ("ls", "ls"),
        ("buffers", "buffers"),
        ("files", "files"),
        ("reg", "registers"),
        ("di", "display"),
        ("marks", "marks"),
        ("w", "write"),
        ("wa", "wall"),
        ("wq", "wq"),
        ("wqa", "wqall"),
        ("xa", "xall"),
        ("x", "xit"),
        ("up", "update"),
        ("q", "quit"),
        ("qa", "qall"),
        ("e", "edit"),
        ("ene", "enew"),
        ("vne", "vnew"),
        ("on", "only"),
        ("clo", "close"),
        ("cd", "cd"),
        ("pw", "pwd"),
        ("noh", "nohlsearch"),
        ("se", "set"),
        ("sp", "split"),
        ("vs", "vsplit"),
        ("h", "help"),
        ("te", "terminal"),
        ("&", "&"),
        ("~", "~"),
        (">", ">"),
        ("!", "!"),
    ] {
        assert_eq!(
            lookup(typed).map(|command| command.name),
            Some(name),
            ":{typed}"
        );
    }
    // shorter than Vim allows is not a command
    for typed in ["nor", "no", "de-", "sy", "apply", "p", "c"] {
        assert!(lookup(typed).is_none(), ":{typed}");
    }
}

#[test]
fn every_row_is_reached_by_its_name_and_its_shortest_form() {
    for command in COMMANDS {
        for typed in [command.name, &command.name[..command.min]] {
            assert_eq!(
                lookup(typed).map(|found| found.name),
                Some(command.name),
                ":{typed}"
            );
        }
    }
}

#[test]
fn a_bad_range_says_so_and_changes_nothing() {
    for (keys, message) in [
        (":9d<cr>", "invalid range"),
        (":2,9d<cr>", "invalid range"),
        (":3,1d<cr>", "backwards range"),
        (":'qd<cr>", "mark not set"),
        (":'<lt>,'>d<cr>", "mark not set"),
        (":/zzz/d<cr>", "pattern not found: zzz"),
        (":2pwd<cr>", "no range allowed"),
        (":frob 3<cr>", "not an editor command: frob 3"),
        (":2,3m2<cr>", "cannot move a range of lines into itself"),
        (":m9<cr>", "invalid range"),
        (":d 0<cr>", "positive count required"),
        (":d a b<cr>", "trailing characters: b"),
    ] {
        let mut e = editor("one\ntwo\nthree\n");
        e.feed_text(keys);
        assert_eq!(e.message(), message, "{keys}");
        assert_eq!(text(&e), "one\ntwo\nthree\n", "{keys}");
        assert!(!e.buf().is_dirty(), "{keys}");
    }
}

#[test]
fn a_readonly_buffer_refuses_the_commands_that_edit() {
    for keys in [
        ":d<cr>",
        ":s/one/x/<cr>",
        ":m$<cr>",
        ":sort!<cr>",
        ":><cr>",
        ":1!cat<cr>",
    ] {
        let mut e = editor("one\ntwo\n");
        e.feed_text(":set ro<cr>");
        e.feed_text(keys);
        assert_eq!(text(&e), "one\ntwo\n", "{keys}");
        assert!(!e.message().is_empty(), "{keys}");
    }
    // a yank is not an edit
    let mut e = editor("one\ntwo\n");
    e.feed_text(":set ro<cr>:2y<cr>");
    assert_eq!(e.register(None).text, "two\n");
}

#[test]
fn ex_normal_types_on_each_line_and_is_one_undo_step() {
    let mut e = editor("one\ntwo\nthree\n");
    e.ex_normal(Some((0, 1)), "A!");
    assert_eq!(text(&e), "one!\ntwo!\nthree\n");
    assert_eq!(e.mode, Mode::Normal);
    e.feed_text("u");
    assert_eq!(text(&e), "one\ntwo\nthree\n");
}

#[test]
fn ex_normal_without_a_range_types_once_at_the_cursor() {
    let mut e = editor("one two\n");
    e.feed_text("w");
    e.ex_normal(None, "x");
    assert_eq!(text(&e), "one wo\n");
}

#[test]
fn ex_normal_abandons_what_the_keys_leave_open() {
    // Insert mode, a pending operator, Visual mode, an open prompt
    for keys in ["ihi", "d", "v", ":s/one/x"] {
        let mut e = editor("one\n");
        e.ex_normal(None, keys);
        assert_eq!(e.mode, Mode::Normal, "{keys}");
        assert!(e.walker.is_ground(), "{keys}");
        assert!(!e.pending.is_active(), "{keys}");
    }
}

#[test]
fn global_and_normal_leave_the_undo_group_closed_behind_them() {
    let mut e = editor("a\nb\na\n");
    e.feed_text(":g/a/norm Ax<cr>");
    assert_eq!(text(&e), "ax\nb\nax\n");
    // the next change is its own step
    e.feed_text("x");
    e.feed_text("u");
    assert_eq!(text(&e), "ax\nb\nax\n");
    e.feed_text("u");
    assert_eq!(text(&e), "a\nb\na\n");
}

#[test]
fn global_refuses_to_nest_and_needs_a_command() {
    let mut e = editor("a\nb\n");
    e.feed_text(":g/a/g/b/d<cr>");
    assert_eq!(e.message(), "cannot do :global recursive");
    assert_eq!(text(&e), "a\nb\n");
    e.feed_text(":g/a/<cr>");
    assert_eq!(e.message(), ":g needs a command to run on the lines");
}

#[test]
fn the_command_line_and_the_substitute_repeat() {
    let mut e = editor("a a\na a\na a\na a\n");
    e.feed_text(":s/a/b/<cr>");
    assert_eq!(e.register(Some(':')).text, "s/a/b/");
    e.feed_text("j");
    e.repeat_command_line(1);
    assert_eq!(text(&e), "b a\nb a\na a\na a\n");
    // `&`: the cursor line, without the flags
    e.feed_text(":s/a/c/g<cr>j");
    e.repeat_substitute(false, false);
    assert_eq!(text(&e), "b a\nb c\nc a\na a\n");
    // `g&`: the whole buffer, the last search pattern, and the flags of
    // the last substitute — the `&` just now, which had none
    e.repeat_substitute(true, true);
    assert_eq!(text(&e), "b c\nb c\nc c\nc a\n");
    // `:@:` is not itself what `:@:` repeats
    e.feed_text(":1d<cr>:@:<cr>:@:<cr>");
    assert_eq!(text(&e), "c a\n");
    assert_eq!(e.register(Some(':')).text, "1d");
}

#[test]
fn a_range_before_a_shell_command_filters_those_lines() {
    let mut e = editor("b\na\nc\n");
    e.feed_text(":1,2!sort<cr>");
    let pipes: Vec<_> = e
        .shell_requests
        .values()
        .filter_map(|intent| match intent.ticket.key {
            crate::editor::ShellKey::Pipe { start, end, .. } => {
                Some((start, end, intent.command.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(pipes, vec![(0, 4, "sort")]);
}

#[test]
fn marks_set_and_deleted_from_the_command_line() {
    let mut e = editor("one\ntwo\nthree\n");
    e.feed_text(":2ka<cr>:3mark b<cr>");
    assert_eq!(e.mark_offset('a'), Some(4));
    assert_eq!(e.mark_offset('b'), Some(8));
    e.feed_text(":delm a<cr>");
    assert_eq!(e.mark_offset('a'), None);
    assert_eq!(e.mark_offset('b'), Some(8));
    e.feed_text(":delm!<cr>");
    assert_eq!(e.mark_offset('b'), None);
}

#[test]
fn completion_offers_the_tables_names() {
    let mut e = editor("x\n");
    e.feed_text(":bn");
    assert_eq!(e.ex_candidates(), vec![("bnext", "go to the next buffer")]);
    // a range is not a name to complete
    e.feed_text("<esc><esc>V:");
    assert_eq!(e.pending.text(), ":'<,'>");
    assert!(e.ex_candidates().is_empty());
}
