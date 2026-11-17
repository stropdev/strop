//! Vim's command-line editing keys, and their `?` rows.

use super::super::keys::{Action, KEYS};
use super::{editor, line_after};
use strop_core::frontend_input::Key;

#[test]
fn ctrl_w_deletes_one_class_of_characters_before_the_cursor() {
    assert_eq!(line_after(":w foo<c-w>"), ":w ");
    assert_eq!(line_after(":w foo  <c-w>"), ":w ");
    assert_eq!(line_after(":e src/main.rs<c-w>"), ":e src/main.");
    assert_eq!(line_after(":e src/main.<c-w>"), ":e src/main");
    assert_eq!(line_after(":s/a_b<c-w>"), ":s/");
    // the sigil is never part of a word
    assert_eq!(line_after(":w<c-w><c-w>"), ":");
}

#[test]
fn ctrl_u_deletes_to_the_start_and_keeps_what_follows() {
    assert_eq!(line_after(":w foo<c-u>"), ":");
    assert_eq!(line_after(":w foo<left><left><left><c-u>"), ":foo");
}

#[test]
fn home_end_and_ctrl_b_ctrl_e_move_then_typing_inserts_there() {
    assert_eq!(line_after(":w foo<home>x"), ":xw foo");
    assert_eq!(line_after(":w foo<c-b>x"), ":xw foo");
    assert_eq!(line_after(":w foo<c-b><end>x"), ":w foox");
    assert_eq!(line_after(":w foo<c-b><c-e>x"), ":w foox");
    assert_eq!(line_after(":abc<left><left>x"), ":axbc");
    assert_eq!(line_after(":abc<home><left>x"), ":xabc");
    assert_eq!(line_after(":abc<right>x"), ":abcx");
}

#[test]
fn delete_takes_the_character_under_the_cursor_or_before_it_at_the_end() {
    assert_eq!(line_after(":abc<home><delete>"), ":bc");
    assert_eq!(line_after(":abc<delete>"), ":ab");
    assert_eq!(line_after(":a<delete><delete>"), ":");
}

#[test]
fn esc_ctrl_c_and_backspace_on_an_empty_line_cancel_at_once() {
    for keys in [
        ":w<esc>",
        ":w<c-c>",
        ":<bs>",
        ":w<bs><bs>",
        "/x<esc>",
        "?x<c-c>",
    ] {
        let mut e = editor("one\n");
        e.feed_text(keys);
        assert!(!e.cmdline.is_open(), "{keys} left the line open");
        assert_eq!(e.buf().text().to_string(), "one\n", "{keys}");
    }
    // backspace in a line with text only deletes
    let mut e = editor("one\n");
    e.feed_text(":w<home><bs>");
    assert_eq!(e.cmdline.text(), ":w");
}

#[test]
fn ctrl_r_inserts_registers_and_the_word_under_the_cursor() {
    let mut e = editor("alpha beta-gamma\n");
    e.feed_text("yiw:<c-r>\"");
    assert_eq!(e.cmdline.text(), ":alpha");
    e.feed_text("<esc>\"ayiw:<c-r>a x");
    assert_eq!(e.cmdline.text(), ":alpha x");
    e.feed_text("<esc>w:<c-r><c-w>");
    assert_eq!(e.cmdline.text(), ":beta");
    e.feed_text("<esc>:<c-r><c-a>");
    assert_eq!(e.cmdline.text(), ":beta-gamma");
    // a key that names nothing ends the wait and inserts nothing
    e.feed_text("<esc>:<c-r><left>x");
    assert_eq!(e.cmdline.text(), ":x");
}

#[test]
fn a_pasted_newline_is_refused_and_text_is_literal() {
    let mut e = editor("x\n");
    e.feed_text(":");
    e.paste_bracketed("w <esc>q");
    assert_eq!(e.cmdline.text(), ":w <esc>q");
    e.paste_bracketed("a\nb");
    assert_eq!(e.cmdline.text(), ":w <esc>q");
    assert_eq!(e.message, "input line cannot contain a newline");
}

/// The `?` row a key action renders in (0003 §5.7).
fn row_id(action: Action) -> &'static str {
    match action {
        Action::Run => "cmdline-run",
        Action::Cancel => "cmdline-cancel",
        Action::Backspace => "cmdline-backspace",
        Action::Delete => "cmdline-delete",
        Action::DeleteWord => "cmdline-delete-word",
        Action::DeleteToStart => "cmdline-delete-to-start",
        Action::Left => "cmdline-left",
        Action::Right => "cmdline-right",
        Action::Home => "cmdline-home",
        Action::End => "cmdline-end",
        Action::Register => "cmdline-register",
        Action::Complete => "cmdline-complete",
        Action::CompleteBack => "cmdline-complete-back",
        Action::Older => "cmdline-older",
        Action::Newer => "cmdline-newer",
        Action::OlderPrefix => "cmdline-older-prefix",
        Action::NewerPrefix => "cmdline-newer-prefix",
        Action::Accept => "cmdline-accept",
        Action::List => "cmdline-list",
        Action::Window => "cmdline-window",
        Action::Repaint => "cmdline-repaint",
    }
}

/// How a key is spelled in a `?` row.
fn notation(key: Key) -> &'static str {
    match key {
        Key::Enter => "enter",
        Key::Esc => "esc",
        Key::CtrlC => "ctrl-c",
        Key::Backspace => "backspace",
        Key::Delete => "delete",
        Key::CtrlW => "ctrl-w",
        Key::CtrlU => "ctrl-u",
        Key::Left => "left",
        Key::Right => "right",
        Key::CtrlB => "ctrl-b",
        Key::Home => "home",
        Key::CtrlE => "ctrl-e",
        Key::End => "end",
        Key::CtrlR => "ctrl-r",
        Key::Tab => "tab",
        Key::Backtab => "s-tab",
        Key::CtrlP => "ctrl-p",
        Key::CtrlN => "ctrl-n",
        Key::Up => "up",
        Key::Down => "down",
        Key::CtrlY => "ctrl-y",
        Key::CtrlD => "ctrl-d",
        Key::CtrlF => "ctrl-f",
        Key::CtrlL => "ctrl-l",
        other => panic!("no notation for {other:?}"),
    }
}

/// Both directions of 0003 §5.7: every bound key renders in its
/// action's `?` row, and every key row of the section is an action.
#[test]
fn every_key_renders_in_help_and_every_row_is_bound() {
    let rows: Vec<_> = crate::keymap::BINDINGS
        .iter()
        .filter(|row| row.sections.contains(&"cmdline"))
        .collect();
    for (key, action) in KEYS {
        let row = rows
            .iter()
            .find(|row| row.id == row_id(*action))
            .unwrap_or_else(|| panic!("{action:?} has no ? row"));
        assert!(
            row.keys.split(' ').any(|token| token == notation(*key)),
            "{key:?} is missing from the {} row",
            row.id
        );
    }
    for row in rows {
        if matches!(row.id, "cmdline-open" | "cmdline-ranges") {
            continue;
        }
        assert!(
            KEYS.iter().any(|(_, action)| row_id(*action) == row.id),
            "{} documents no bound action",
            row.id
        );
    }
}
