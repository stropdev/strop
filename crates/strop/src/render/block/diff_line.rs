//! One changed-line painter (0100 §6.1), used by the diff view and the
//! status buffer: the origin's inks, the quiet tint under a changed row,
//! the hunk header's ranges and, in the status buffer, the `+`/`-`
//! prefix. Intra-line emphasis is a byte range the row loop paints over
//! the tint (delta's two tiers).

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::render::style::Tokens;
use strop_git::{DiffLine, Hunk, LineOrigin};

pub(crate) fn origin_fg(t: &Tokens, origin: LineOrigin) -> Color {
    match origin {
        LineOrigin::Addition => t.diff_add,
        LineOrigin::Deletion => t.diff_del,
        LineOrigin::Context => t.text,
    }
}

/// Quiet full-row backgrounds for add/del rows — a scan signal that does
/// not shout; context rows stay on the base plane.
pub(crate) fn origin_bg(t: &Tokens, origin: LineOrigin) -> Option<Color> {
    match origin {
        LineOrigin::Addition => Some(t.diff_add_bg),
        LineOrigin::Deletion => Some(t.diff_del_bg),
        LineOrigin::Context => None,
    }
}

/// The hunk header's label: `@@` quiet, the old range in the deletion
/// ink, the new range in the addition ink. The spans concatenate to
/// exactly `Hunk::header()`, the string the buffer row holds.
pub(crate) fn hunk_header(t: &Tokens, hunk: &Hunk) -> Vec<Span<'static>> {
    vec![
        Span::styled("@@ ", Style::default().fg(t.muted)),
        Span::styled(
            format!("-{},{}", hunk.old_start, hunk.old_count),
            Style::default().fg(t.diff_del),
        ),
        Span::styled(" ", Style::default().fg(t.muted)),
        Span::styled(
            format!("+{},{}", hunk.new_start, hunk.new_count),
            Style::default().fg(t.diff_add),
        ),
        Span::styled(" @@", Style::default().fg(t.muted)),
    ]
}

/// A status buffer's diff line (0100 §4.2): the prefix in the diff ink,
/// the text in `fg.text`. The spans are the buffer row's own bytes.
pub(crate) fn prefixed(t: &Tokens, line: &DiffLine) -> Vec<Span<'static>> {
    let prefix = match line.origin {
        LineOrigin::Addition => "+",
        LineOrigin::Deletion => "-",
        LineOrigin::Context => " ",
    };
    vec![
        Span::styled(prefix, Style::default().fg(origin_fg(t, line.origin))),
        Span::styled(line.text_str().into_owned(), Style::default().fg(t.text)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_header_spans_keep_the_buffer_rows_bytes() {
        let hunk = Hunk {
            kind: strop_git::HunkKind::Change,
            new_start: 41,
            new_count: 7,
            old_start: 40,
            old_count: 6,
            lines: Vec::new(),
        };
        let t = Tokens::new(strop_core::theme::ThemeId::Hone);
        let spans = hunk_header(&t, &hunk);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        // the buffer row IS Hunk::header(): byte equality keeps the
        // caret, search and yank aligned with what is on screen
        assert_eq!(joined, hunk.header());
        let ink = |text: &str| spans.iter().find(|s| s.content == text).unwrap().style.fg;
        assert_eq!(ink("-40,6"), Some(t.diff_del));
        assert_eq!(ink("+41,7"), Some(t.diff_add));
    }

    #[test]
    fn a_status_line_is_its_prefix_and_text() {
        let t = Tokens::new(strop_core::theme::ThemeId::Vellum);
        let line = DiffLine {
            origin: LineOrigin::Deletion,
            old_lineno: Some(2),
            new_lineno: None,
            text: b"line 2".to_vec(),
            has_newline: true,
        };
        let spans = prefixed(&t, &line);
        assert_eq!(spans[0].content, "-");
        assert_eq!(spans[0].style.fg, Some(t.diff_del));
        assert_eq!(spans[1].content, "line 2");
        assert_eq!(spans[1].style.fg, Some(t.text));
    }
}
