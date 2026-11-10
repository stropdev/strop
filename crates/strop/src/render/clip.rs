//! Printable, grapheme-safe display-cell clipping for editor chrome.
use std::borrow::Cow;

use ratatui::text::Span;
use strop_core::layout::{printable_grapheme, printable_text};
use unicode_segmentation::UnicodeSegmentation;

fn grapheme_width(grapheme: &str) -> usize {
    Span::raw(printable_text(grapheme)).width()
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

/// Word-wrap into at most `rows` lines of `cells` cells. A word wider than
/// a line breaks between graphemes; text left over after the last line
/// clips it with an ellipsis.
pub(super) fn wrap(text: &str, cells: usize, rows: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() && lines.len() < rows && cells > 0 {
        if width(rest) <= cells {
            lines.push(printable_text(rest).into_owned());
            break;
        }
        if lines.len() + 1 == rows {
            lines.push(clip_end(rest, cells).into_owned());
            break;
        }
        // the longest prefix that fits, broken after its last space
        let (mut used, mut fit, mut space) = (0, 0, None);
        for (byte, grapheme) in rest.grapheme_indices(true) {
            used += grapheme_width(grapheme);
            if used > cells {
                break;
            }
            fit = byte + grapheme.len();
            if grapheme == " " {
                space = Some(byte);
            }
        }
        let fit = fit.max(rest.graphemes(true).next().map_or(0, str::len));
        let cut = space.filter(|&at| at > 0).unwrap_or(fit);
        lines.push(printable_text(rest[..cut].trim_end()).into_owned());
        rest = rest[cut..].trim_start();
    }
    lines
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

/// Keep the greatest whole-grapheme suffix without spending a cell on an
/// ellipsis. Workspace-relative pane titles use this when one parent character
/// is the only duplicate-basename disambiguator available at narrow widths.
pub(super) fn suffix(text: &str, cells: usize) -> Cow<'_, str> {
    if cells == 0 {
        return Cow::Borrowed("");
    }
    let mut used = 0;
    let mut edge = text.len();
    for (byte, grapheme) in text.grapheme_indices(true).rev() {
        let cells_for_grapheme = grapheme_width(grapheme);
        if used + cells_for_grapheme > cells {
            break;
        }
        used += cells_for_grapheme;
        edge = byte;
    }
    let mut output = String::new();
    append_printable(&mut output, &text[edge..]);
    Cow::Owned(output)
}

/// Keep a local path's basename and a short parent-prefix disambiguator.
/// The parent yields first at narrow widths; no grapheme is split.
pub(super) fn path_suffix(text: &str, cells: usize) -> Cow<'_, str> {
    if cells == 0 {
        return Cow::Borrowed("");
    }
    if width(text) <= cells {
        return printable_text(text);
    }
    let Some((dir, name)) = text.rsplit_once('/') else {
        return suffix(text, cells);
    };
    let name_width = width(name);
    let parent = dir.rsplit('/').next().unwrap_or(dir);
    if name_width > cells {
        let mut output = String::new();
        let mut parent_width = 0;
        if let Some(grapheme) = parent.graphemes(true).next() {
            let cells_for_grapheme = grapheme_width(grapheme);
            if cells_for_grapheme + 1 < cells {
                parent_width = cells_for_grapheme;
                output.push_str(printable_grapheme(grapheme));
            }
        }
        if parent_width != 0 {
            output.push('/');
            let suffix = suffix(name, cells - parent_width - 1);
            output.push_str(&suffix);
            return Cow::Owned(output);
        }
        return suffix(name, cells);
    }
    let parent_budget = cells - name_width - 1;
    if parent_budget == 0 {
        return suffix(name, cells);
    }
    let mut output = String::new();
    let mut used = 0;
    for grapheme in parent.graphemes(true) {
        let cells_for_grapheme = grapheme_width(grapheme);
        if used + cells_for_grapheme > parent_budget {
            break;
        }
        used += cells_for_grapheme;
        output.push_str(printable_grapheme(grapheme));
    }
    output.push('/');
    append_printable(&mut output, name);
    Cow::Owned(output)
}

#[cfg(test)]
mod tests {
    #[test]
    fn wrap_breaks_at_spaces_then_clips_the_last_row() {
        assert_eq!(
            super::wrap("alpha beta gamma", 11, 3),
            ["alpha beta", "gamma"]
        );
        assert_eq!(super::wrap("abcdefgh", 3, 3), ["abc", "def", "gh"]);
        assert_eq!(
            super::wrap("one two three four five", 9, 2),
            ["one two", "three fo…"]
        );
        assert_eq!(super::wrap("界界界", 3, 3), ["界", "界", "界"]);
    }

    use super::*;

    #[test]
    fn clipping_keeps_whole_wide_and_combining_graphemes() {
        assert_eq!(clip_end("界e\u{301}abc", 4), "界e\u{301}…");
        assert_eq!(clip_start("abc界e\u{301}", 4), "…界e\u{301}");
        assert_eq!(clip_end("界", 1), "…");
        assert_eq!(clip_end("界", 2), "界");
        assert_eq!(clip_start("界", 0), "");
        assert_eq!(suffix("one/shared.rs", 12), "ne/shared.rs");
        assert_eq!(suffix("tab\t界e\u{301}.rs", 7), "�界e\u{301}.rs");
        assert_eq!(path_suffix("one/shared.rs", 11), "o/shared.rs");
        assert_eq!(path_suffix("two/shared.rs", 11), "t/shared.rs");
        assert_eq!(path_suffix("deep/tab\t界e\u{301}.rs", 7), "d/e\u{301}.rs");
    }

    #[test]
    fn controls_are_measured_and_emitted_as_printable_cells() {
        assert_eq!(width("界e\u{301}\x1b"), 4);
        assert_eq!(clip_end("a\r\nb", 8), "a\u{fffd}b");
        assert_eq!(clip_start("a\r\nb", 8), "a\u{fffd}b");
        assert_eq!(clip_end("a\x1bbcd", 3), "a\u{fffd}…");
    }
}
