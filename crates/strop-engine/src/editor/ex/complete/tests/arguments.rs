//! Argument rows by declared kind.

use super::{editor, menu};
use crate::editor::Editor;
use strop_core::Buffer;

#[test]
fn words_open_on_tab_and_refilter_as_you_type() {
    let mut e = editor();
    e.feed_text(":set ");
    assert!(e.cmdline_menu().is_none(), "arguments wait for Tab");
    e.feed_text("<c-d>");
    assert_eq!(menu(&e), ["readonly", "noreadonly", "ro", "noro"]);
    e.feed_text("n");
    assert_eq!(menu(&e)[..2], ["noreadonly", "noro"]);
    e.feed_text("<tab>");
    assert_eq!(e.cmdline.text(), ":set noreadonly");
    e.feed_text("<esc>:remote con<tab>");
    assert_eq!(e.cmdline.text(), ":remote connect");
}

#[test]
fn themes_and_help_topics() {
    let mut e = editor();
    e.feed_text(":theme <c-d>");
    assert_eq!(
        menu(&e).len(),
        strop_core::theme::ThemeId::ALL.len(),
        "every theme"
    );
    e.feed_text("<esc>:help wri<tab>");
    assert_eq!(e.cmdline.text(), ":help write");
    e.feed_text("<esc>:help gi<tab>");
    assert_eq!(e.cmdline.text(), ":help git");
}

#[test]
fn buffers_by_number_or_name_with_their_unsaved_mark() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["alpha.rs", "beta.rs", "gamma.txt"] {
        std::fs::write(dir.path().join(name), "x\n").unwrap();
    }
    let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_owned());
    for name in ["alpha.rs", "beta.rs", "gamma.txt"] {
        e.feed_text(&format!(":e {name}<cr>"));
        e.wait_io().unwrap();
    }
    e.feed_text("ix<esc>");
    e.feed_text(":b .rs<tab>");
    let rows = e.cmdline_menu().unwrap().rows().to_vec();
    assert_eq!(e.cmdline.text(), format!(":b {}", rows[0].text()));
    let mut names: Vec<_> = rows.iter().map(|row| row.text()).collect();
    names.sort_unstable();
    assert_eq!(names, ["alpha.rs", "beta.rs"]);
    assert!(rows.iter().all(|row| !row.abbrev().is_empty()), "numbers");
    e.feed_text("<esc>:b gam<c-d>");
    let row = &e.cmdline_menu().unwrap().rows()[0];
    assert_eq!(row.text(), "gamma.txt");
    assert!(row.modified(), "gamma has the unsaved x");
}
