//! Tab, Shift-Tab, Ctrl-N/Ctrl-P, Up/Down, Ctrl-Y, Ctrl-E.

use super::{editor, menu};

#[test]
fn tab_visits_every_row_once_and_returns_to_what_was_typed() {
    let mut e = editor();
    e.feed_text(":w");
    let rows = menu(&e);
    assert!(rows.len() > 2, "{rows:?}");
    for row in &rows {
        e.feed_text("<tab>");
        assert_eq!(e.cmdline.text(), format!(":{row}"));
    }
    e.feed_text("<tab>");
    assert_eq!(e.cmdline.text(), ":w", "past the last row: what was typed");
    assert_eq!(e.cmdline_menu().unwrap().selected(), None);
    e.feed_text("<tab>");
    assert_eq!(e.cmdline.text(), format!(":{}", rows[0]), "and round again");
}

#[test]
fn shift_tab_and_ctrl_p_go_backwards() {
    let mut e = editor();
    e.feed_text(":w");
    let rows = menu(&e);
    e.feed_text("<s-tab>");
    assert_eq!(e.cmdline.text(), format!(":{}", rows[rows.len() - 1]));
    e.feed_text("<c-p>");
    assert_eq!(e.cmdline.text(), format!(":{}", rows[rows.len() - 2]));
    e.feed_text("<c-n><c-n>");
    assert_eq!(e.cmdline.text(), ":w");
}

#[test]
fn up_and_down_move_in_the_menu_only_once_a_row_is_selected() {
    let mut e = editor();
    e.feed_text(":2y<cr>:w");
    e.feed_text("<up>");
    assert_eq!(e.cmdline.text(), ":w", "no history line starts with w");
    e.feed_text("<tab><down>");
    let rows = menu(&e);
    assert_eq!(e.cmdline.text(), format!(":{}", rows[1]));
    e.feed_text("<up>");
    assert_eq!(e.cmdline.text(), format!(":{}", rows[0]));
}

#[test]
fn ctrl_e_puts_back_what_was_typed_and_ctrl_y_keeps_the_row() {
    let mut e = editor();
    e.feed_text(":wq<tab><tab>");
    assert_eq!(e.cmdline.text(), ":wqall");
    e.feed_text("<c-e>");
    assert_eq!(e.cmdline.text(), ":wq");
    assert!(e.cmdline_menu().is_none());
    // with nothing selected Ctrl-E is End
    e.feed_text("<home><c-e>x");
    assert_eq!(e.cmdline.text(), ":wqx");
    e.feed_text("<esc>:wq<tab><tab><c-y>");
    assert_eq!(e.cmdline.text(), ":wqall");
    assert!(e.cmdline_menu().is_none());
}

#[test]
fn typing_after_tab_keeps_the_completed_text() {
    let mut e = editor();
    e.feed_text(":hel<tab>");
    assert_eq!(e.cmdline.text(), ":help");
    e.feed_text(" g");
    assert_eq!(e.cmdline.text(), ":help g");
    e.feed_text("<esc>:hel<tab>p");
    assert_eq!(e.cmdline.text(), ":helpp");
}

#[test]
fn one_match_completes_and_closes_the_menu() {
    let mut e = editor();
    e.feed_text(":symb<tab>");
    assert_eq!(e.cmdline.text(), ":symbols");
    assert!(e.cmdline_menu().is_none());
}

#[test]
fn completion_works_after_a_range_and_mid_line() {
    let mut e = editor();
    e.feed_text(":%sor<tab>");
    assert_eq!(e.cmdline.text(), ":%sort");
    e.feed_text("<esc>:wr foo<home><right><right><tab>");
    assert_eq!(e.cmdline.text(), ":write foo");
}

#[test]
fn search_and_pipe_lines_have_no_menu() {
    let mut e = editor();
    e.feed_text("/wr<tab>");
    assert!(e.cmdline_menu().is_none());
    assert_eq!(e.cmdline.text(), "/wr");
}
