//! The commit message buffer's three inks (0100 §4.6): it is a file and
//! gets no headings or virtual text, only the syntax git gives it. The
//! subject is bold; its cells past column 72 and a second line that is
//! not empty are the two mistakes the buffer exists to avoid, in
//! `diag.warning`; `#` lines are the commentary git leaves out, in the
//! comment ink and italic like every other comment (0088 rule 7).

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::render::clip;
use crate::render::style::Tokens;

/// Where a subject stops being comfortable to read in `git log`.
const SUBJECT_CELLS: usize = 72;

/// Styled spans for line `line` of a commit message whose text is
/// `text`; they concatenate to exactly the line. `None` leaves the line
/// as plain text (the body).
pub(super) fn spans(t: &Tokens, line: usize, text: &str) -> Option<Vec<Span<'static>>> {
    if text.starts_with('#') {
        return Some(vec![Span::styled(
            text.to_owned(),
            Style::default()
                .fg(t.syn.comment)
                .add_modifier(Modifier::ITALIC),
        )]);
    }
    match line {
        0 => {
            let subject = Style::default().fg(t.text).add_modifier(Modifier::BOLD);
            let mut cells = 0;
            let mut split = text.len();
            for (at, grapheme) in
                unicode_segmentation::UnicodeSegmentation::grapheme_indices(text, true)
            {
                cells += clip::width(grapheme);
                if cells > SUBJECT_CELLS {
                    split = at;
                    break;
                }
            }
            let (fits, over) = text.split_at(split);
            let mut spans = vec![Span::styled(fits.to_owned(), subject)];
            if !over.is_empty() {
                spans.push(Span::styled(over.to_owned(), subject.fg(t.warning)));
            }
            Some(spans)
        }
        1 if !text.is_empty() => Some(vec![Span::styled(
            text.to_owned(),
            Style::default().fg(t.warning),
        )]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_subject_warns_past_column_72_and_keeps_its_bytes() {
        let t = Tokens::new(strop_core::theme::ThemeId::Hone);
        let subject = format!("{}{}", "s".repeat(72), "past");
        let painted = spans(&t, 0, &subject).unwrap();
        let joined: String = painted.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, subject);
        assert_eq!(painted[0].content.len(), 72);
        assert!(painted[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(painted[1].content, "past");
        assert_eq!(painted[1].style.fg, Some(t.warning));
        let short = spans(&t, 0, "docs: say how").unwrap();
        assert_eq!(short.len(), 1);
    }

    #[test]
    fn the_second_line_and_comments() {
        let t = Tokens::new(strop_core::theme::ThemeId::Vellum);
        assert!(spans(&t, 1, "").is_none());
        assert_eq!(spans(&t, 1, "oops").unwrap()[0].style.fg, Some(t.warning));
        let comment = spans(&t, 5, "# On branch main").unwrap();
        assert_eq!(comment[0].style.fg, Some(t.syn.comment));
        assert!(comment[0].style.add_modifier.contains(Modifier::ITALIC));
        assert!(spans(&t, 4, "body text").is_none());
    }
}
