//! Cell/shape boundaries independently probed against Neovim buffer operations.
use super::*;
use crate::editor::registers::RegisterShape;
use strop_core::id::DisplayColumn;

fn editor(text: &str) -> Editor {
    Editor::new(Buffer::from_text(text))
}
fn block(editor: &mut Editor, rows: &str, width: usize) {
    editor.set_register(None, Register::blockwise(rows, DisplayColumn::new(width)));
}

#[test]
fn block_paste_before_after_and_count_preserve_rectangle_shape() {
    for (keys, expected, head) in [
        ("P", "abcbcdef\nghihijkl\n", 1),
        ("p", "abbccdef\nghhiijkl\n", 2),
        ("2p", "abbcbccdef\nghhihiijkl\n", 2),
    ] {
        let mut e = editor("abcdef\nghijkl\n");
        block(&mut e, "bc\nhi", 2);
        e.set_head(1);
        e.feed_text(keys);
        assert_eq!(e.buf().text().to_string(), expected);
        assert_eq!(e.head(), head);
        e.feed_text("u");
        assert_eq!(e.buf().text().to_string(), "abcdef\nghijkl\n");
    }
}

#[test]
fn configured_tabs_and_empty_rows_keep_block_paste_columns() {
    let mut e = editor("\tabcdef\n\tghijkl\n");
    e.config.tab_size = 4;
    block(&mut e, "bc\nhi", 2);
    e.set_head(1);
    e.feed_text("p");
    assert_eq!(e.buf().text().to_string(), "\tabcbcdef\n\tghihijkl\n");
    assert_eq!(e.head(), 2);
    let mut e = editor("abcde\n\nxyz\n");
    block(&mut e, "ab\nxy", 2);
    e.set_head(6);
    e.feed_text("p");
    assert_eq!(e.buf().text().to_string(), "abcde\nab\nxyxyz\n");
}

#[test]
fn partial_wide_cluster_is_cells_not_broken_utf8() {
    let mut e = editor("aXbc\n界z\naYbc\n");
    e.feed_text("l<c-v>jjy");
    assert_eq!(e.register(None).text, "X\n \nY");
    assert_eq!(e.head(), 1);
    e.feed_text("<c-v>jjd");
    assert_eq!(e.buf().text().to_string(), "abc\n z\nabc\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "aXbc\n界z\naYbc\n");
}

#[test]
fn tab_intersection_preserves_unselected_cells_and_full_tabs() {
    let mut e = editor("aXbc\n\tz\n");
    e.config.tab_size = 4;
    e.feed_text("l<c-v>jld");
    assert_eq!(e.register(None).text, "Xbc\n   z");
    assert_eq!(e.buf().text().to_string(), "a\n \n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "aXbc\n\tz\n");
    let mut e = editor("\tz\n\tw\n");
    e.feed_text("<c-v>jy");
    assert_eq!(e.register(None).text, "\t\n\t");
    assert_eq!(
        e.register(None).shape,
        RegisterShape::Blockwise {
            width: DisplayColumn::new(4)
        }
    );
}

#[test]
fn block_change_skips_short_rows_and_undo_restores_one_unit() {
    let mut e = editor("abcdef\nx\nghijkl\n");
    e.set_head(3);
    e.feed_text("<c-v>jjcZ<esc>");
    assert_eq!(e.buf().text().to_string(), "abcZef\nx\nghiZkl\n");
    assert_eq!(e.register(None).text, "d\n \nj");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "abcdef\nx\nghijkl\n");
}

#[test]
fn block_append_pads_short_rows_while_insert_skips_them() {
    for (keys, expected) in [
        ("<c-v>jjIZ<esc>", "abcZdef\nx\nghiZjkl\n"),
        ("<c-v>jjAZ<esc>", "abcdZef\nx   Z\nghijZkl\n"),
    ] {
        let mut e = editor("abcdef\nx\nghijkl\n");
        e.set_head(3);
        e.feed_text(keys);
        assert_eq!(e.buf().text().to_string(), expected);
        e.feed_text("u");
        assert_eq!(e.buf().text().to_string(), "abcdef\nx\nghijkl\n");
    }
}

#[test]
fn block_paste_creates_crlf_rows_without_mixing_endings() {
    let mut e = editor("ab\r\ncd\r\n");
    block(&mut e, "x\ny", 1);
    e.set_head(4);
    e.feed_text("p");
    assert_eq!(e.buf().text().to_string(), "ab\r\ncxd\r\n y\r\n");
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), "ab\r\ncd\r\n");
}

#[test]
fn multibyte_characterwise_paste_lands_on_a_character_boundary() {
    let mut e = editor("a÷b\n");
    e.set_register(None, Register::characterwise("÷"));
    e.set_head(1);
    e.feed_text("p");
    assert_eq!(e.buf().text().to_string(), "a÷÷b\n");
    assert_eq!(e.head(), 3);
}
