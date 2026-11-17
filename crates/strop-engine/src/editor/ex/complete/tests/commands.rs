//! Command rows: the top row is what `Enter` runs; prefixes, then fuzzy.

use super::super::super::command_word;
use super::super::super::table::COMMANDS;
use super::{editor, menu};

/// The menu cannot lie: for every name and every prefix Vim accepts,
/// the menu opens as it is typed and row 0 is the command `Enter` would
/// run for it.
#[test]
fn the_top_row_is_what_enter_runs_for_every_prefix() {
    for command in COMMANDS.iter() {
        for name in command.names {
            for end in name.min..=name.name.len() {
                let typed = &name.name[..end];
                let mut e = editor();
                e.feed_text(":");
                e.cmdline_paste(typed);
                let rows = menu(&e);
                let (runs, runs_name, _) = command_word(typed).expect("a name");
                assert_eq!(
                    rows.first().map(String::as_str),
                    Some(runs_name),
                    ":{typed} runs :{}",
                    runs.name()
                );
            }
        }
    }
}

#[test]
fn e_lists_edit_first_and_the_other_e_commands_after_it() {
    let mut e = editor();
    e.feed_text(":e");
    let rows = menu(&e);
    assert_eq!(rows[0], "edit", "Enter runs :edit, not :exit");
    assert_eq!(rows[1..], ["exit", "enew", "explain"], "then table order");
}

#[test]
fn prefixes_come_before_fuzzy_matches_and_aliases_fold() {
    let mut e = editor();
    e.feed_text(":sym");
    assert_eq!(menu(&e), ["symbols"]);
    e.feed_text("<esc>:tloc");
    assert_eq!(menu(&e)[0], "terminal-local", "fuzzy finds it");
    e.feed_text("<esc>:bu");
    let rows = menu(&e);
    // `buffers` is `:ls`'s other name: one row for the command
    assert_eq!(rows.iter().filter(|row| row.starts_with("buf")).count(), 2);
    e.feed_text("<esc>:wq");
    let rows = menu(&e);
    assert_eq!(&rows[..2], ["wq", "wqall"]);
}

#[test]
fn the_menu_opens_as_you_type_after_a_range_but_not_on_an_empty_line() {
    let mut e = editor();
    e.feed_text(":");
    assert!(e.cmdline_menu().is_none(), "an empty line has no menu");
    e.feed_text("%sor");
    assert_eq!(menu(&e)[0], "sort");
    e.feed_text("<esc>:'<lt>,'>nor");
    assert!(menu(&e).contains(&"normal".to_owned()), "{:?}", menu(&e));
    e.feed_text("<esc>:g/x/de");
    assert_eq!(menu(&e)[0], "delete");
    e.feed_text("<esc>:cdo s");
    assert_eq!(menu(&e)[0], "substitute");
    // nothing is selected until Tab
    assert_eq!(e.cmdline_menu().unwrap().selected(), None);
    // a pattern closes it
    e.feed_text("/");
    assert!(e.cmdline_menu().is_none());
}

#[test]
fn tab_on_an_empty_line_lists_every_command() {
    let mut e = editor();
    e.feed_text(":<c-d>");
    assert_eq!(menu(&e).len(), COMMANDS.len());
    assert_eq!(e.cmdline.text(), ":", "Ctrl-D selects nothing");
}

#[test]
fn matched_letters_are_marked_for_prefix_and_fuzzy_rows() {
    let mut e = editor();
    e.feed_text(":wr");
    let row = &e.cmdline_menu().unwrap().rows()[0];
    assert_eq!(row.text(), "write");
    assert!(row.matched(0) && row.matched(1) && !row.matched(2));
    assert_eq!(row.abbrev(), "w");
    e.feed_text("<esc>:tloc");
    let row = &e.cmdline_menu().unwrap().rows()[0];
    let marked: String = row
        .label()
        .chars()
        .enumerate()
        .filter(|(at, _)| row.matched(*at))
        .map(|(_, c)| c)
        .collect();
    assert_eq!(marked, "tloc");
}
