//! The gutters of 0100 §3.5 as data: what each cell of a block row's
//! gutter holds. Every function paints exactly the width it is given.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::render::style::Tokens;
use strop_git::{DiffLine, LineOrigin};

/// What a list gutter's sign column shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sign {
    Blank,
    /// The cursor row: `▌` in the accent.
    Cursor,
    /// A row nested under an expanded list row: `│` in `fg.faint`.
    Rail,
}

/// The list gutter (status, log, files): the sign column and a space.
pub(crate) fn list(t: &Tokens, sign: Sign) -> Vec<Span<'static>> {
    let mark = match sign {
        Sign::Blank => Span::raw(" "),
        Sign::Cursor => Span::styled("▌", Style::default().fg(t.accent)),
        Sign::Rail => Span::styled("│", Style::default().fg(t.faint)),
    };
    vec![mark, Span::raw(" ")]
}

/// The lines and source gutters: a sign, the number right-aligned, a
/// space. No number (a title, a blank row) leaves the cells blank; the
/// cursor row's number is the accent.
pub(crate) fn numbered(
    t: &Tokens,
    sign: Span<'static>,
    number: Option<usize>,
    cursor: bool,
    width: usize,
) -> Vec<Span<'static>> {
    let digits = width.saturating_sub(2);
    let ink = Style::default().fg(if cursor { t.accent } else { t.muted });
    let cell = match number {
        Some(number) => format!("{number:>digits$} "),
        None => " ".repeat(digits + 1),
    };
    vec![sign, Span::styled(cell, ink)]
}

/// A break's gutter: `⋮` in `fg.faint` in the last digit cell of each
/// number column (`columns`, in cells from the gutter's start).
pub(crate) fn jump(t: &Tokens, width: usize, columns: &[usize]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = 0;
    for &column in columns {
        if column < at || column >= width {
            continue;
        }
        if column > at {
            spans.push(Span::raw(" ".repeat(column - at)));
        }
        spans.push(Span::styled("⋮", Style::default().fg(t.faint)));
        at = column + 1;
    }
    if width > at {
        spans.push(Span::raw(" ".repeat(width - at)));
    }
    spans
}

/// The source gutter's number column ends one cell before the space.
pub(crate) fn source_jump(width: usize) -> [usize; 1] {
    [width.saturating_sub(2)]
}

/// The pair gutter's two number columns: `[sign][old] [new] `.
pub(crate) fn pair_jump(width: usize) -> [usize; 2] {
    let digits = pair_digits(width);
    [digits, 2 * digits + 1]
}

/// Digits per side of a pair gutter `width` cells wide.
pub(crate) fn pair_digits(width: usize) -> usize {
    width.saturating_sub(3) / 2
}

/// The pair gutter for one diff line: origin sign + both sides' numbers,
/// right-aligned, an absent side blank (never `0`). The cursor row's
/// numbers light up in the accent.
pub(crate) fn pair(t: &Tokens, line: &DiffLine, cursor: bool, width: usize) -> Vec<Span<'static>> {
    let digits = pair_digits(width);
    // the sign column keeps one vocabulary (0088 rule 5): `▎` in the
    // origin colour
    let (marker, color) = match line.origin {
        LineOrigin::Addition => ("▎", t.diff_add),
        LineOrigin::Deletion => ("▎", t.diff_del),
        LineOrigin::Context => (" ", t.muted),
    };
    let number_ink = Style::default().fg(if cursor { t.accent } else { t.muted });
    let number = |n: Option<usize>| match n {
        Some(n) => Span::styled(format!("{n:>digits$}"), number_ink),
        None => Span::raw(" ".repeat(digits)),
    };
    vec![
        Span::styled(
            marker,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        number(line.old_lineno),
        Span::raw(" "),
        number(line.new_lineno),
        Span::raw(" "),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::clip;

    fn width(spans: &[Span<'_>]) -> usize {
        spans.iter().map(|span| clip::width(&span.content)).sum()
    }

    /// §9: each gutter paints exactly its width.
    #[test]
    fn every_gutter_paints_its_width() {
        let t = Tokens::new(strop_core::theme::ThemeId::Hone);
        for sign in [Sign::Blank, Sign::Cursor, Sign::Rail] {
            assert_eq!(
                width(&list(&t, sign)),
                strop_engine::editor::prepare::LIST_GUTTER
            );
        }
        for cells in [5, 6, 9, 15] {
            for number in [None, Some(1), Some(10usize.pow(cells as u32 - 2) - 1)] {
                let sign = Span::raw(" ");
                assert_eq!(width(&numbered(&t, sign, number, true, cells)), cells);
            }
            assert_eq!(width(&jump(&t, cells, &source_jump(cells))), cells);
            assert_eq!(width(&jump(&t, cells, &pair_jump(cells))), cells);
        }
        let jumped: String = jump(&t, 9, &pair_jump(9))
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(jumped, "   ⋮   ⋮ ", "each mark at its column's last digit");
        for digits in 3..=6 {
            let line = DiffLine {
                origin: LineOrigin::Addition,
                old_lineno: None,
                new_lineno: Some(10usize.pow(digits as u32) - 1),
                text: Vec::new(),
                has_newline: true,
            };
            let cells = 2 * digits + 3;
            for cursor in [false, true] {
                assert_eq!(width(&pair(&t, &line, cursor, cells)), cells);
            }
        }
    }
}
