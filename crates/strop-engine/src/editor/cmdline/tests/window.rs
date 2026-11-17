//! The command-line window: `q:`, `q/`, `Ctrl-F`.

use super::editor;

#[test]
fn q_colon_shows_the_history_as_a_buffer_and_enter_runs_a_line() {
    let mut e = editor("one\ntwo\nthree\nfour\n");
    e.feed_text(":1y<cr>:3y<cr>q:");
    assert_eq!(e.panes().len(), 2, "a split below");
    assert_eq!(e.buf().name.as_deref(), Some("command line"));
    assert_eq!(e.buf().text().to_string(), "1y\n3y\n\n");
    assert_eq!(e.buf().line_of(e.head()), 2, "on the line being typed");
    // a real buffer: motions and operators; Enter runs the cursor's line
    e.feed_text("kcwd<esc>");
    assert_eq!(e.buf().line_text(1), "d");
    e.feed_text("i2<esc><cr>");
    assert_eq!(e.panes().len(), 1, "the window closed");
    assert_eq!(e.buf().text().to_string(), "one\nthree\nfour\n");
    let history: Vec<_> = e.cmdline.history.ex.iter().collect();
    assert_eq!(history, ["1y", "3y", "2d"], "the line run is history");
    assert!(
        !e.docs
            .iter()
            .any(|(_, d)| d.buf.name.as_deref() == Some("command line")),
        "its buffer is gone"
    );
}

#[test]
fn enter_in_insert_mode_runs_and_ctrl_c_or_q_closes() {
    let mut e = editor("one\ntwo\n");
    e.feed_text("q:i2d<cr>");
    assert_eq!(e.buf().text().to_string(), "one\n");
    assert_eq!(e.mode, crate::editor::Mode::Normal);
    e.feed_text("q:<c-c>");
    assert_eq!(e.panes().len(), 1);
    e.feed_text("q:ix<esc>:q<cr>");
    assert_eq!(e.panes().len(), 1);
    assert_eq!(e.buf().text().to_string(), "one\n", "nothing ran");
}

#[test]
fn ctrl_f_carries_the_open_line_in_and_q_slash_is_search_history() {
    let mut e = editor("one\ntwo\nthree\n");
    e.feed_text(":2d<c-f>");
    assert!(!e.cmdline.is_open());
    assert_eq!(e.buf().text().to_string(), "2d\n");
    e.feed_text("<cr>");
    assert_eq!(e.buf().text().to_string(), "one\nthree\n");
    e.feed_text("/thr<cr>ggq/");
    assert_eq!(e.buf().text().to_string(), "thr\n\n");
    e.feed_text("k<cr>");
    assert_eq!(e.buf().line_of(e.head()), 1, "the search ran");
}
