//! `:ls`, `:b`, `:bn`, `:bp`, `:bd`, `:reg`, `:marks`, `:wa`, `:wqa`.

use super::super::{buffer_listing, mark_listing, register_listing};
use super::{editor, text, three_buffers};
use strop_core::Buffer;

use crate::editor::Editor;

#[test]
fn ls_lists_number_flags_name_and_line_in_vims_columns() {
    let (mut e, [_, b, _]) = three_buffers();
    e.feed_text(":b 2<cr>ox<esc>");
    assert_eq!(e.current(), b);
    assert_eq!(
        buffer_listing(&e, false),
        concat!(
            "  1 #h   \"a.txt\"                        line 0\n",
            "  2 %a + \"b.txt\"                        line 2\n",
            "  3  h   \"src/lib.rs\"                   line 0\n",
        )
    );
}

#[test]
fn ls_opens_a_readonly_listing_and_leaves_itself_out_of_it() {
    let (mut e, [a, _, _]) = three_buffers();
    e.feed_text(":ls<cr>");
    assert!(e.buf().readonly);
    assert_eq!(e.buf().name.as_deref(), Some("buffers"));
    assert_eq!(text(&e).lines().count(), 3);
    // the listing is unlisted, like Vim's help; `:ls!` shows it with `u`
    let all = buffer_listing(&e, true);
    assert!(all.contains("u%a=  \"buffers\""), "{all}");
    e.feed_text(":q<cr>");
    assert_eq!(e.current(), a);
}

#[test]
fn b_goes_by_number_by_exact_name_and_by_unique_fragment() {
    let (mut e, [a, b, c]) = three_buffers();
    e.feed_text(":b 3<cr>");
    assert_eq!(e.current(), c);
    e.feed_text(":b a.txt<cr>");
    assert_eq!(e.current(), a);
    e.feed_text(":b lib<cr>");
    assert_eq!(e.current(), c);
    e.feed_text(":2b<cr>");
    assert_eq!(e.current(), b);
    e.feed_text(":b #<cr>");
    assert_eq!(e.current(), c);
}

#[test]
fn b_refuses_an_ambiguous_or_unknown_name_and_stays_put() {
    let (mut e, [a, _, _]) = three_buffers();
    e.feed_text(":b txt<cr>");
    assert_eq!(e.message(), "more than one match for txt");
    assert_eq!(e.current(), a);
    e.feed_text(":b nope<cr>");
    assert_eq!(e.message(), "no matching buffer for nope");
    e.feed_text(":b 9<cr>");
    assert_eq!(e.message(), "buffer 9 does not exist");
    assert_eq!(e.current(), a);
}

#[test]
fn bn_and_bp_step_in_number_order_and_wrap() {
    let (mut e, [a, b, c]) = three_buffers();
    e.feed_text(":bn<cr>");
    assert_eq!(e.current(), b);
    e.feed_text(":bn<cr>:bn<cr>");
    assert_eq!(e.current(), a);
    e.feed_text(":bp<cr>");
    assert_eq!(e.current(), c);
    e.feed_text(":bN<cr>");
    assert_eq!(e.current(), b);
    e.feed_text(":2bn<cr>");
    assert_eq!(e.current(), a);
}

#[test]
fn bd_deletes_another_buffer_in_place_and_numbers_stay_put() {
    let (mut e, [a, b, c]) = three_buffers();
    e.feed_text("$");
    let head = e.head();
    e.feed_text(":bd 2<cr>");
    assert!(e.docs.get(b).is_none());
    assert_eq!(e.current(), a);
    assert_eq!(e.head(), head, "the window did not move");
    // the third buffer is still number 3
    e.feed_text(":b 3<cr>");
    assert_eq!(e.current(), c);
    e.feed_text(":bd<cr>");
    assert!(e.docs.get(c).is_none());
    assert_eq!(e.current(), a);
}

#[test]
fn bd_keeps_a_modified_buffer_unless_forced() {
    let (mut e, [a, b, _]) = three_buffers();
    e.feed_text(":b 2<cr>x:b 1<cr>");
    e.feed_text(":bd 2<cr>");
    assert!(e.docs.get(b).is_some());
    assert_eq!(e.message(), "unsaved changes — :q! to force");
    assert_eq!(e.current(), a);
    e.feed_text(":bd! 2<cr>");
    assert!(e.docs.get(b).is_none());
    assert_eq!(e.current(), a);
}

#[test]
fn reg_lists_type_name_and_content() {
    let mut e = editor("one two\nthree\n");
    e.feed_text("yiw\"ayyjdd:pwd<cr>");
    assert_eq!(
        register_listing(&e, ""),
        concat!(
            "Type Name Content\n",
            "  l  \"\"   three^J\n",
            "  c  \"0   one\n",
            "  l  \"1   three^J\n",
            "  l  \"a   one two^J\n",
            // a plain yank is a copy: the clipboard registers hold it too
            "  c  \"*   one\n",
            "  c  \"+   one\n",
            "  c  \":   pwd\n",
        )
    );
    assert_eq!(
        register_listing(&e, "a0"),
        "Type Name Content\n  c  \"0   one\n  l  \"a   one two^J\n"
    );
    e.feed_text(":reg a<cr>");
    assert!(e.buf().readonly);
    assert_eq!(e.buf().name.as_deref(), Some("registers"));
    assert_eq!(text(&e), "Type Name Content\n  l  \"a   one two^J\n");
}

#[test]
fn marks_lists_mark_line_column_and_text_or_file() {
    let (mut e, [_, b, _]) = three_buffers();
    e.buf_mut().edit().insert(0, "  indented\n").unwrap();
    e.feed_text("ggllmajV<esc>");
    e.marks.insert('B', (b, 2));
    assert_eq!(
        mark_listing(&e),
        concat!(
            "mark line  col file/text\n",
            " a      1    2 indented\n",
            " B      1    2 b.txt\n",
            " <      2    2 alpha\n",
            " >      2    2 alpha\n",
        )
    );
    e.feed_text(":marks<cr>");
    assert_eq!(e.buf().name.as_deref(), Some("marks"));
}

/// Three files on disk, all open; the second and third modified.
fn on_disk() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "text\n").unwrap();
    }
    let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_owned());
    for name in ["a.txt", "b.txt", "c.txt"] {
        e.open_fixture(&dir.path().join(name)).unwrap();
        if name != "a.txt" {
            e.feed_text("x");
        }
    }
    (dir, e)
}

#[test]
fn wa_writes_every_modified_file_and_says_how_many() {
    let (dir, mut e) = on_disk();
    e.feed_text(":wa<cr>");
    assert_eq!(e.message(), "2 buffers written");
    e.wait_io().unwrap();
    for name in ["b.txt", "c.txt"] {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(name)).unwrap(),
            "ext\n"
        );
    }
    assert!(!e.any_dirty());
}

#[test]
fn wa_reports_a_modified_buffer_that_has_no_name() {
    let (dir, mut e) = on_disk();
    e.feed_text(":enew<cr>iunnamed<esc>");
    e.feed_text(":wa<cr>");
    assert_eq!(
        e.message(),
        "2 buffers written; 1 modified buffer without a file name not written (:w NAME)"
    );
    e.wait_io().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "ext\n"
    );
    assert!(e.any_dirty(), "the unnamed text is still there");
}

#[test]
fn wqa_quits_once_the_writes_have_landed() {
    let (dir, mut e) = on_disk();
    e.feed_text(":wqa<cr>");
    assert!(!e.should_quit(), "not before the files are written");
    e.wait_io().unwrap();
    assert!(e.should_quit());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("c.txt")).unwrap(),
        "ext\n"
    );
}

#[test]
fn wqa_stays_open_when_an_unnamed_buffer_would_be_lost() {
    let (_dir, mut e) = on_disk();
    e.feed_text(":enew<cr>iunnamed<esc>:xa<cr>");
    assert!(e.message().contains("without a file name"));
    e.wait_io().unwrap();
    assert!(!e.should_quit());
}

#[test]
fn x_and_up_write_only_a_modified_buffer() {
    let (dir, mut e) = on_disk();
    let path = dir.path().join("c.txt");
    // unmodified: nothing is written, the stamp on disk is not touched
    e.feed_text(":b a.txt<cr>:up<cr>");
    assert!(!e.io_pending());
    e.feed_text(":b c.txt<cr>:up<cr>");
    e.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ext\n");
    assert!(e.docs.len() > 1, ":up does not close");
    let before = e.docs.len();
    e.feed_text(":x<cr>");
    e.wait_io().unwrap();
    assert_eq!(e.docs.len(), before - 1, ":x on a clean buffer closes it");
}

#[test]
fn new_buffers_and_window_commands() {
    let (mut e, [a, _, _]) = three_buffers();
    e.feed_text(":vnew<cr>");
    assert_eq!(e.panes.len(), 2);
    assert_ne!(e.current(), a);
    e.feed_text(":b 1<cr>:only<cr>");
    assert_eq!(e.panes.len(), 1);
    assert_eq!(e.current(), a);
    e.feed_text(":close<cr>");
    assert_eq!(e.message(), "cannot close the last window");
}
