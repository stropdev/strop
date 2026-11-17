//! Cell/shape boundaries independently probed against Neovim buffer operations.
use super::*;
use crate::editor::registers::RegisterShape;
use strop_core::id::DisplayColumn;

fn editor(text: &str) -> Editor {
    Editor::new(Buffer::from_text(text))
}
fn block(editor: &mut Editor, rows: &str, width: usize) {
    editor.yank_register(None, Register::blockwise(rows, DisplayColumn::new(width)));
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
    e.yank_register(None, Register::characterwise("÷"));
    e.set_head(1);
    e.feed_text("p");
    assert_eq!(e.buf().text().to_string(), "a÷÷b\n");
    assert_eq!(e.head(), 3);
}

#[test]
fn a_recording_is_register_content_and_a_register_replays_as_keys() {
    // vim: `qa` fills "a, and `@a` executes whatever "a holds
    let mut e = editor("  text\nIZ\n");
    e.feed_text("qaIX<esc>q");
    assert_eq!(e.register(Some('a')).text, "IX\u{1b}");
    assert_eq!(e.register(None).text, "", "recording is not a yank");
    e.feed_text("@a");
    assert_eq!(e.buf().text().to_string(), "  XXtext\nIZ\n");
    // a yank into "a replaces the recording
    e.feed_text("j0\"ayiw@a");
    assert_eq!(e.buf().text().to_string(), "  XXtext\nZIZ\n");
}

#[test]
fn black_hole_reads_empty_and_keeps_the_unnamed_register() {
    let mut e = editor("one\ntwo\n");
    e.feed_text("yiwj\"_dd\"_p");
    assert_eq!(e.buf().text().to_string(), "one\n");
    assert_eq!(e.register(None).text, "one");
}

#[test]
fn a_plain_yank_reaches_the_system_clipboard_and_a_delete_does_not() {
    // 0092 §1.4: yank, then the terminal's paste carries it
    let mut e = editor("one two\nthree\n");
    e.feed_text("yiw");
    assert_eq!(e.osc52.take().as_deref(), Some("one"));
    assert_eq!(e.register(Some('+')).text, "one");
    e.feed_text("jdd");
    assert_eq!(e.osc52, None, "a delete never overwrites the clipboard");
    e.feed_text("x");
    assert_eq!(e.osc52, None);
    e.feed_text("\"ayiw");
    assert_eq!(e.osc52, None, "a named yank stays in its register");
    assert_eq!(e.register(Some('+')).text, "one");
    e.feed_text("Vy");
    assert_eq!(e.osc52.take().as_deref(), Some("ne two\n"));
    // too large to push through the terminal: kept, and said
    let mut e = editor(&"x".repeat((1 << 20) + 1));
    e.feed_text("yy");
    assert_eq!(e.osc52, None);
    assert!(e.message.contains("too large"), "{}", e.message);
    assert_eq!(e.register(None).text.len(), (1 << 20) + 1);
}

/// 0108 H1: a linewise put below a last row without its break grows the
/// break; the landing is the first non-blank of the pasted row, counted
/// on the register text — never sliced by the grown break's length.
#[test]
fn linewise_put_below_an_unterminated_last_row_lands_on_the_first_non_blank() {
    for sep in ["\n", "\r\n"] {
        for terminated in [true, false] {
            for first in ["a", "é", "日", "🎉", "  é", "\t日"] {
                let tail = if terminated { sep } else { "" };
                let text = format!("{first}x{sep}{first}y{tail}");
                for keys in ["Gyyp", "GyyP", "Gyy]p", "ggyyGp"] {
                    let mut e = editor(&text);
                    e.feed_text(keys);
                    let row = e.buf().line_of(e.head());
                    let start = e.buf().line_start(row);
                    let blank = first.len() - first.trim_start().len();
                    assert_eq!(
                        e.head(),
                        start + blank,
                        "{keys} on {text:?} -> {:?}",
                        e.buf().text().to_string()
                    );
                    let pasted = e.buf().line_text(row);
                    assert!(pasted.starts_with(first), "{keys} on {text:?}: {pasted:?}");
                    e.feed_text("u");
                    assert_eq!(e.buf().text().to_string(), text, "{keys} undo");
                }
            }
        }
    }
}

/// 0108 H1: a bracketed paste of whole lines in Normal mode is a
/// linewise put; an empty buffer or a last row without its break must
/// not slice the payload by the grown break.
#[test]
fn bracketed_linewise_paste_below_an_unterminated_row_is_whole_chars() {
    for (buffer, payload) in [("", "é日\r\n"), ("ab", "🎉x\n"), ("aé\r\nb", "é\r\n")] {
        let mut e = editor(buffer);
        e.feed_text("G");
        e.paste_bracketed(payload);
        let row = e.buf().line_of(e.head());
        assert_eq!(
            e.head(),
            e.buf().line_start(row),
            "{buffer:?} + {payload:?}"
        );
        assert!(e.buf().text().to_string().contains(payload.trim_end()));
    }
}
