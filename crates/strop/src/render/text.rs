//! Printable, grapheme-safe display-cell clipping for editor chrome.
use std::borrow::Cow;

use ratatui::text::Span;
use strop_core::layout::{printable_grapheme, printable_text};
use unicode_segmentation::UnicodeSegmentation;

fn grapheme_width(grapheme: &str) -> usize {
    Span::raw(printable_grapheme(grapheme)).width()
}

pub(super) fn width(text: &str) -> usize {
    text.graphemes(true).map(grapheme_width).sum()
}

fn append_printable(output: &mut String, text: &str) {
    for grapheme in text.graphemes(true) {
        output.push_str(printable_grapheme(grapheme));
    }
}

/// Keep the beginning, reserving a cell for an ellipsis only when clipped.
pub(super) fn clip_end(text: &str, cells: usize) -> Cow<'_, str> {
    if cells == 0 {
        return Cow::Borrowed("");
    }
    let mut used = 0;
    let mut edge = 0;
    for (byte, grapheme) in text.grapheme_indices(true) {
        used += grapheme_width(grapheme);
        if used < cells {
            edge = byte + grapheme.len();
        }
        if used > cells {
            let mut output = String::with_capacity(edge + '…'.len_utf8());
            append_printable(&mut output, &text[..edge]);
            output.push('…');
            return Cow::Owned(output);
        }
    }
    printable_text(text)
}

/// Keep the end, so a long path can retain its filename instead of its prefix.
pub(super) fn clip_start(text: &str, cells: usize) -> Cow<'_, str> {
    if cells == 0 {
        return Cow::Borrowed("");
    }
    let mut used = 0;
    let mut edge = text.len();
    for (byte, grapheme) in text.grapheme_indices(true).rev() {
        used += grapheme_width(grapheme);
        if used < cells {
            edge = byte;
        }
        if used > cells {
            let mut output = String::with_capacity(text.len() - edge + '…'.len_utf8());
            output.push('…');
            append_printable(&mut output, &text[edge..]);
            return Cow::Owned(output);
        }
    }
    printable_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_keeps_whole_wide_and_combining_graphemes() {
        assert_eq!(clip_end("界e\u{301}abc", 4), "界e\u{301}…");
        assert_eq!(clip_start("abc界e\u{301}", 4), "…界e\u{301}");
        assert_eq!(clip_end("界", 1), "…");
        assert_eq!(clip_end("界", 2), "界");
        assert_eq!(clip_start("界", 0), "");
    }

    #[test]
    fn controls_are_measured_and_emitted_as_printable_cells() {
        assert_eq!(width("界e\u{301}\x1b"), 4);
        assert_eq!(clip_end("a\r\nb", 8), "a\u{fffd}b");
        assert_eq!(clip_start("a\r\nb", 8), "a\u{fffd}b");
        assert_eq!(clip_end("a\x1bbcd", 3), "a\u{fffd}…");
    }
}
