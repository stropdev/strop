//! `split::target`: the cursor's part of a command line.

use super::super::super::table::Arg;
use super::super::split::{range_len, target, Target};

/// The target's kind and the text it completes, for compact tables.
fn at(body: &str) -> (&'static str, String) {
    at_cursor(body, body.len())
}

fn at_cursor(body: &str, cursor: usize) -> (&'static str, String) {
    match target(body, cursor) {
        Target::None => ("none", String::new()),
        Target::Name { start, end } => ("name", body[start..end].to_owned()),
        Target::Arg {
            arg, start, end, ..
        } => {
            let kind = match arg {
                Arg::File => "file",
                Arg::Directory => "dir",
                Arg::Buffer => "buffer",
                Arg::Theme => "theme",
                Arg::HelpTopic => "help",
                Arg::Words(_) => "words",
                _ => "other",
            };
            (kind, body[start..end].to_owned())
        }
    }
}

#[test]
fn names_after_ranges_bangs_and_nested_commands() {
    for (body, kind, text) in [
        ("w", "name", "w"),
        ("", "name", ""),
        ("%s", "name", "s"),
        ("%sor", "name", "sor"),
        ("'<,'>norm", "name", "norm"),
        ("2,$d", "name", "d"),
        ("/pat/,/x/y", "name", "y"),
        (": :wri", "name", "wri"),
        ("select-ne", "name", "select-ne"),
        ("g/x/d", "name", "d"),
        ("g!/x/no", "name", "no"),
        ("v/x\\/y/s", "name", "s"),
        ("cdo s", "name", "s"),
        ("cfdo w", "name", "w"),
        ("frob", "name", "frob"),
    ] {
        assert_eq!(at(body), (kind, text.to_owned()), ":{body}");
    }
}

#[test]
fn arguments_by_declared_kind() {
    for (body, kind, text) in [
        ("e foo", "file", "foo"),
        ("e! src/ma", "file", "src/ma"),
        ("w odd name", "file", "odd name"),
        ("vs ", "file", ""),
        ("cd sr", "dir", "sr"),
        ("b ", "buffer", ""),
        ("b sta", "buffer", "sta"),
        ("theme ho", "theme", "ho"),
        ("help wri", "help", "wri"),
        ("set ro", "words", "ro"),
        ("remote con", "words", "con"),
        ("sort! u n", "words", "n"),
        ("cdo b x", "buffer", "x"),
        ("g/x/e fo", "file", "fo"),
    ] {
        assert_eq!(at(body), (kind, text.to_owned()), ":{body}");
    }
}

#[test]
fn patterns_registers_shell_text_and_open_patterns_complete_nothing() {
    for body in [
        "s/a/b", "%s/x", "d a", "!ls", "r !ls", "norm dd", "g/open", "k a", "e!", "rename x",
    ] {
        assert_eq!(at(body).0, "none", ":{body}");
    }
}

#[test]
fn the_cursor_decides_not_the_end_of_the_line() {
    // mid-line: the name before the cursor
    assert_eq!(at_cursor("wri foo", 2), ("name", "wr".to_owned()));
    // in the argument, up to the cursor
    assert_eq!(at_cursor("e src/main.rs", 5), ("file", "src".to_owned()));
    // `:m-2` is `:m` with an address
    assert_eq!(at_cursor("m-2", 1), ("name", "m".to_owned()));
    assert_eq!(at("m-2").0, "none");
    // inside the range
    assert_eq!(at_cursor("12,3d", 1).0, "none");
}

#[test]
fn ranges_are_read_lexically() {
    for (text, length) in [
        ("w", 0),
        ("%s", 1),
        ("'<,'>d", 5),
        ("/a\\/b/d", 6),
        ("?x?+1,$y", 7),
        ("2;+3m0", 4),
        ("\\/d", 2),
        ("/open", 5),
    ] {
        assert_eq!(range_len(text), length, "{text}");
    }
}
