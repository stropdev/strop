//! `:g` on large buffers and `Ctrl-C` (plan 0108 H4). Its line-by-line
//! semantics diff against Neovim in the differential corpus.

use super::{editor, text};
use strop_core::frontend_input::{Input, KeyEvent, Modifiers};

fn lines(count: usize, line: impl Fn(usize) -> String) -> String {
    (0..count).map(|index| line(index) + "\n").collect()
}

/// Every marked line moves when a command edits above it; with tens of
/// thousands of marks that must not cost a pass over all of them per
/// edit, nor a search from the top of the buffer per `:s`. These run in
/// the suite's time only when each line costs about the same.
#[test]
fn global_on_many_lines_costs_each_line_once() {
    let mut e = editor(&lines(20_000, |index| {
        format!("{} {index}", if index % 2 == 0 { "one" } else { "two" })
    }));
    e.feed_text(":g/one/d\r");
    assert_eq!(
        text(&e),
        lines(10_000, |index| format!("two {}", 2 * index + 1))
    );
    let mut e = editor(&lines(5_000, |index| format!("w{index}")));
    e.feed_text(":g/./s/w/W/\r");
    assert_eq!(text(&e), lines(5_000, |index| format!("W{index}")));
    let mut e = editor(&lines(5_000, |_| "a b".to_owned()));
    e.feed_text(":g/./norm Ax\r");
    assert_eq!(text(&e), lines(5_000, |_| "a bx".to_owned()));
    e.feed_text("u");
    assert_eq!(text(&e), lines(5_000, |_| "a b".to_owned()));
}

/// Commands that move lines below, above and onto the marks keep each
/// mark on its own line.
#[test]
fn global_marks_follow_lines_that_move() {
    for (before, command, after) in [
        ("a\nb\nc\nd\n", ":g/^/m0\r", "d\nc\nb\na\n"),
        ("a\nb\na\n", ":g/a/t$\r", "a\nb\na\na\na\n"),
        ("a\nb\na\n", ":g/a/t0\r", "a\na\na\nb\na\n"),
        ("a1\na2\nb\na3\n", ":g/a/.,+1d\r", "b\na3\n"),
        ("a\nb\na\nb\n", ":g/a/j\r", "a b\na b\n"),
        ("a\na\n", ":g/a/s/a/x\\ry/\r", "x\ny\nx\ny\n"),
        ("ab\nab\n", ":g/a/norm Ox\r", "x\nab\nx\nab\n"),
    ] {
        let mut e = editor(before);
        e.feed_text(command);
        assert_eq!(text(&e), after, "{before:?} {command:?}");
    }
}

fn ctrl_c() -> Input {
    Input::Key(KeyEvent {
        modifiers: Modifiers {
            control: true,
            ..Modifiers::default()
        },
        ..KeyEvent::press(strop_core::frontend_input::KeyCode::Char('c'))
    })
}

/// A `Ctrl-C` the key reader saw while `:g` runs stops it before the
/// next line; what it did stays one undo step. Taking that `Ctrl-C` as
/// input afterwards lowers the flag.
#[test]
fn ctrl_c_stops_global_and_normal_between_lines() {
    let mut e = editor("a\na\na\n");
    e.interrupt().raise();
    e.feed_text(":g/a/d\r");
    assert_eq!(text(&e), "a\na\na\n");
    assert_eq!(e.message, "Interrupted");
    e.feed_text(":%norm Ax\r");
    assert_eq!(
        text(&e),
        "a\na\na\n",
        "still pending: the key has not arrived"
    );
    e.handle_frontend_input(ctrl_c());
    e.feed_text(":g/a/s/a/b/\r");
    assert_eq!(text(&e), "b\nb\nb\n");
    e.feed_text("u");
    assert_eq!(text(&e), "a\na\na\n");
}
