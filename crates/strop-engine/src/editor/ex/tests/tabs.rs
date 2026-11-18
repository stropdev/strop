//! Tab pages on the command line (plans/0113 §8.3): every row of §6 lands
//! where §3 says, every abbreviation reaches its command, and the errors
//! say why.

use super::editor;
use crate::editor::tabs::{count, current, summary};
use crate::editor::Editor;

/// The text of each tab page's current pane, in tab order.
fn order(e: &Editor) -> Vec<String> {
    (0..count(e))
        .map(|index| {
            let document = summary(e, index).unwrap().document;
            e.doc(document).buf.text().to_string()
        })
        .collect()
}

/// `n` tab pages showing `t1` … `tn`, the last one current.
fn tabs(n: usize) -> Editor {
    let mut e = editor("t1");
    for k in 2..=n {
        e.feed_text(&format!(":tabnew<cr>it{k}<esc>"));
    }
    e
}

#[test]
fn tabnew_opens_an_empty_buffer_after_this_tab_page() {
    let mut e = tabs(3);
    e.feed_text(":tabfirst<cr>:tabnew<cr>");
    assert_eq!(order(&e), ["t1", "", "t2", "t3"]);
    assert_eq!(current(&e), 1);
    assert_eq!(e.docs.len(), 4);
    e.feed_text(":tabe<cr>");
    assert_eq!(current(&e), 2, ":tabe is :tabedit, the same command");
}

#[test]
fn tab_split_shows_this_buffer_alone_with_its_cursor() {
    let mut e = editor("one\ntwo\nthree\n");
    e.feed_text(":vs<cr>jj:tab split<cr>");
    assert_eq!(count(&e), 2);
    assert_eq!(e.panes.len(), 1);
    assert_eq!(e.buf().line_of(e.head()), 2, "the cursor came along");
    assert_eq!(e.docs.len(), 1, "the same buffer");
    // the zoom loop: `:q` closes the tab page and the split is back
    e.feed_text(":q<cr>");
    assert_eq!(count(&e), 1);
    assert_eq!(e.panes.len(), 2);
    assert_eq!(e.docs.len(), 1);
    e.feed_text(":tab sp<cr>:tab new<cr>");
    assert_eq!(count(&e), 3);
    assert_eq!(e.buf().len_bytes(), 0, ":tab new is :tabnew");
}

#[test]
fn tab_takes_split_or_new_and_nothing_else() {
    let mut e = editor("x\n");
    for line in [":tab foo<cr>", ":tab vsplit<cr>", ":tab<cr>"] {
        e.feed_text(line);
        assert_eq!(e.message(), ":tab takes split or new", "{line}");
        assert_eq!(count(&e), 1);
    }
    // `:tab-size` is its own command, not `:tab`
    e.feed_text(":tab-size 3<cr>");
    assert_ne!(e.message(), ":tab takes split or new");
    assert_eq!(count(&e), 1);
}

#[test]
fn tabnext_and_tabprevious_go_by_number_and_wrap() {
    let mut e = tabs(4);
    e.feed_text(":tabn<cr>");
    assert_eq!(current(&e), 0, "wraps to the first");
    e.feed_text(":tabn 3<cr>");
    assert_eq!(current(&e), 2);
    e.feed_text(":2tabn<cr>");
    assert_eq!(current(&e), 1);
    e.feed_text(":tabp 3<cr>");
    assert_eq!(current(&e), 2, "three back from 2 of 4 wraps to 3");
    e.feed_text(":tabN<cr>");
    assert_eq!(current(&e), 1);
    e.feed_text(":tabl<cr>");
    assert_eq!(current(&e), 3);
    e.feed_text(":tabr<cr>");
    assert_eq!(current(&e), 0);
    e.feed_text(":tabn 9<cr>");
    assert_eq!(e.message(), "no tab 9");
    assert_eq!(current(&e), 0);
    e.feed_text(":tabn 0<cr>");
    assert_eq!(e.message(), "no tab 0");
}

#[test]
fn tabclose_closes_this_or_tab_page_n_and_never_the_last() {
    let mut e = tabs(4);
    e.feed_text(":tabn 2<cr>:tabc<cr>");
    assert_eq!(order(&e), ["t1", "t3", "t4"]);
    assert_eq!(current(&e), 1, "the one to its right takes its number");
    e.feed_text(":tabl<cr>:tabc<cr>");
    assert_eq!(order(&e), ["t1", "t3"]);
    assert_eq!(current(&e), 1, "the new last");
    e.feed_text(":tabclose 1<cr>");
    assert_eq!(order(&e), ["t3"]);
    assert_eq!(e.docs.len(), 4, "its buffers stay open");
    e.feed_text(":tabc!<cr>");
    assert_eq!(e.message(), "cannot close the last tab");
    e.feed_text(":tabc 7<cr>");
    assert_eq!(e.message(), "no tab 7");
}

#[test]
fn tabonly_keeps_this_tab_page_and_its_splits() {
    let mut e = tabs(3);
    e.feed_text(":vs<cr>:tabo!<cr>");
    assert_eq!(order(&e), ["t3"]);
    assert_eq!(e.panes.len(), 2);
    e.feed_text(":tabo 2<cr>");
    assert_eq!(e.message(), "trailing characters: 2");
}

#[test]
fn tabmove_places_this_tab_page() {
    let mut e = tabs(3);
    e.feed_text(":tabm 0<cr>");
    assert_eq!(order(&e), ["t3", "t1", "t2"]);
    e.feed_text(":tabm<cr>");
    assert_eq!(order(&e), ["t1", "t2", "t3"]);
    e.feed_text(":tabm -1<cr>");
    assert_eq!(order(&e), ["t1", "t3", "t2"]);
    e.feed_text(":tabm +1<cr>");
    assert_eq!(order(&e), ["t1", "t2", "t3"]);
    e.feed_text(":tabm 1<cr>");
    assert_eq!(order(&e), ["t1", "t3", "t2"]);
    assert_eq!(current(&e), 1);
    e.feed_text(":tabm +1<cr>:tabm +1<cr>");
    assert_eq!(e.message(), "no tab there");
    e.feed_text(":tabm 9<cr>");
    assert_eq!(e.message(), "no tab 9");
    assert_eq!(order(&e), ["t1", "t2", "t3"]);
}

#[test]
fn every_abbreviation_reaches_its_command() {
    for (typed, name) in [
        ("tabn", "tabnext"),
        ("tabne", "tabnext"),
        ("tabnew", "tabnew"),
        ("tabe", "tabedit"),
        ("tabc", "tabclose"),
        ("tabo", "tabonly"),
        ("tabm", "tabmove"),
        ("tabl", "tablast"),
        ("tabp", "tabprevious"),
        ("tabN", "tabNext"),
        ("tabr", "tabrewind"),
        ("tabfir", "tabfirst"),
        ("tab", "tab"),
    ] {
        let found = crate::editor::ex::table::lookup(typed).map(|(_, name)| name);
        assert_eq!(found, Some(name), ":{typed}");
    }
    assert!(crate::editor::ex::table::lookup("tabf").is_none());
    let mut e = editor("x\n");
    e.feed_text(":tabf<cr>");
    assert!(e.message().starts_with("not an editor command"));
}

#[test]
fn tabnew_with_a_file_opens_it_through_the_open_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "beta\n").unwrap();
    let mut e = Editor::new_in(strop_core::Buffer::from_text("origin\n"), dir.path().into());
    e.feed_text(":tabnew a.txt<cr>");
    e.wait_io().unwrap();
    assert_eq!(order(&e), ["origin\n", "alpha\n"]);
    e.feed_text(":tab sp b.txt<cr>");
    e.wait_io().unwrap();
    assert_eq!(order(&e), ["origin\n", "alpha\n", "beta\n"]);
    assert_eq!(current(&e), 2);
    // completion after the name is a path
    std::fs::create_dir(dir.path().join("src")).unwrap();
    e.feed_text(":tabnew sr<tab>");
    let event = e
        .remote_completion
        .rx
        .as_ref()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    e.handle_remote_completion(event);
    assert_eq!(e.cmdline.text(), ":tabnew src/");
}

#[test]
fn tab_completion_lists_the_tab_commands() {
    let mut e = editor("x\n");
    e.feed_text(":tab");
    let menu = e.cmdline_menu().expect("the menu opens as you type");
    let names: Vec<_> = menu.rows().iter().map(|row| row.text()).collect();
    for name in ["tab", "tabnext", "tabnew", "tabclose", "tabmove"] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
}

#[test]
fn q_on_a_tab_pages_only_pane_closes_the_tab_page() {
    let mut e = editor("x\n");
    e.feed_text(":tab split<cr>:q<cr>");
    assert_eq!(count(&e), 1);
    assert!(!e.should_quit);
    e.feed_text(":tab split<cr>:close<cr>");
    assert_eq!(count(&e), 1);
    e.feed_text(":close<cr>");
    assert_eq!(e.message(), "cannot close the last window");
    e.feed_text(":tab split<cr><c-w>q");
    assert_eq!(count(&e), 1);
    // with one tab page `:q` closes the buffer, as before
    e.feed_text(":q<cr>");
    assert!(e.should_quit);
}

#[test]
fn keys_step_through_tab_pages() {
    let mut e = tabs(4);
    e.feed_text("gt");
    assert_eq!(current(&e), 0);
    e.feed_text("3gt");
    assert_eq!(current(&e), 2);
    e.feed_text("gT");
    assert_eq!(current(&e), 1);
    e.feed_text("3gT");
    assert_eq!(current(&e), 2);
    e.feed_text("g<tab>");
    assert_eq!(current(&e), 1, "the tab page used before");
    e.feed_text("g<tab>");
    assert_eq!(current(&e), 2);
    e.feed_text("9gt");
    assert_eq!(e.message(), "no tab 9");
    assert!(e.walker.is_ground());
}

#[test]
fn gt_works_in_a_readonly_buffer() {
    let mut e = editor("x\n");
    e.feed_text(":tab split<cr>:ls<cr>");
    assert!(e.buf().readonly);
    e.feed_text("gt");
    assert_eq!(current(&e), 0, "{}", e.message());
    e.feed_text("g<tab>gT");
    assert_eq!(current(&e), 0, "{}", e.message());
    e.feed_text("gt<c-w>v<c-w>T");
    assert_eq!(count(&e), 3, "{}", e.message());
}

#[test]
fn nothing_moves_while_the_command_line_window_is_open() {
    let mut e = editor("x\n");
    e.feed_text(":tab split<cr>q:");
    let window = e.current();
    for keys in [":tabn<cr>", "gt", "g<tab>", ":tabnew<cr>", "<c-w>T"] {
        e.feed_text(keys);
        assert_eq!(e.current(), window, "{keys}");
        assert_eq!(count(&e), 2, "{keys}");
    }
    e.feed_text(":tabn<cr>");
    assert_eq!(e.message(), "close the command-line window first");
}
