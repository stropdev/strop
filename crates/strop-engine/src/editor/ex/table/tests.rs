//! The registry's own contract: names and usages are unique, every
//! prefix reaches the right command, and a listed Normal key does what
//! its command does.

use super::{lookup, COMMANDS};
use crate::editor::Editor;
use strop_core::Buffer;

#[test]
fn names_and_usages_are_unique() {
    let mut names = std::collections::HashSet::new();
    let mut usages = std::collections::HashSet::new();
    for command in COMMANDS.iter() {
        assert!(usages.insert(command.usage), "usage {}", command.usage);
        assert!(!command.doc.is_empty(), "{} has no doc", command.name());
        for name in command.names {
            assert!(names.insert(name.name), "name {}", name.name);
            assert!(
                (1..=name.name.len()).contains(&name.min),
                "{}: min {}",
                name.name,
                name.min
            );
        }
    }
}

/// Precedence where abbreviations overlap, stated once: the short
/// forms Vim gives to the older command.
#[test]
fn overlapping_abbreviations_name_vims_command() {
    for (typed, name) in [
        ("s", "substitute"),
        ("se", "set"),
        ("e", "edit"),
        ("exi", "exit"),
        ("d", "delete"),
        ("di", "display"),
        ("delm", "delmarks"),
        ("c", ""),
        ("co", "copy"),
        ("cc", "cc"),
        ("cn", "cnext"),
        ("m", "move"),
        ("ma", "mark"),
        ("marks", "marks"),
        ("t", "t"),
        ("te", "terminal"),
        ("b", "buffer"),
        ("bN", "bNext"),
        ("bp", "bprevious"),
        ("w", "write"),
        ("wa", "wall"),
        ("wq", "wq"),
        ("wqa", "wqall"),
        ("x", "xit"),
        ("xa", "xall"),
        ("q", "quit"),
        ("qa", "qall"),
        ("quita", "quitall"),
    ] {
        assert_eq!(
            lookup(typed).map(|(_, found)| found).unwrap_or(""),
            name,
            ":{typed}"
        );
    }
}

/// Every prefix of every name, from its shortest form on, reaches its
/// command or a command earlier in the table — never a later one.
#[test]
fn every_prefix_reaches_its_command_or_an_earlier_row() {
    for (index, command) in COMMANDS.iter().enumerate() {
        for name in command.names {
            for end in name.min..=name.name.len() {
                let typed = &name.name[..end];
                let found = lookup(typed).map(|(found, _)| found.name());
                let at = COMMANDS
                    .iter()
                    .position(|row| Some(row.name()) == found)
                    .unwrap_or_else(|| panic!(":{typed} reaches nothing"));
                assert!(at <= index, ":{typed} reaches the later {found:?}");
            }
        }
    }
}

/// A command's `key` is a live binding, and pressing it does what the
/// command does: from the same start, both leave the same text, cursor,
/// windows, picker and message.
#[test]
fn a_listed_key_does_what_its_command_does() {
    let state = |e: &Editor| {
        (
            e.buf().text().to_string(),
            e.head(),
            e.panes().len(),
            e.active_pane,
            e.picker_open(),
            e.sels().heads(),
        )
    };
    for command in COMMANDS.iter() {
        let Some(key) = command.key else { continue };
        assert!(
            crate::keymap::BINDINGS
                .iter()
                .filter(|row| row.live)
                .any(|row| crate::keymap::lookup::expand(row.keys)
                    .iter()
                    .any(|sequence| sequence.join(" ") == key)),
            "{}: {key} is not a live binding",
            command.name()
        );
        // a start where the command has something to do
        let setup = match command.name() {
            "undo" => "x",
            "redo" => "xu",
            "close" | "only" => "<c-w>v",
            _ => "",
        };
        let fresh = || {
            let mut e = Editor::new(Buffer::from_text("one two one\ntwo one\n"));
            e.feed_text(setup);
            e
        };
        let mut by_key = fresh();
        let keys = key
            .replace("ctrl-r", "<c-r>")
            .replace("ctrl-w ", "<c-w>")
            .replace("space ", " ");
        by_key.feed_text(&keys);
        let mut by_command = fresh();
        by_command.feed_text(&format!(":{}<cr>", command.name()));
        assert_eq!(
            state(&by_key),
            state(&by_command),
            "{key} and :{}",
            command.name()
        );
    }
}
