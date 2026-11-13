//! One look for the collection and the git buffers (0100): a block is a
//! ruled heading, its rows and one empty row; a jump inside a block is a
//! band; the buffer holds words, the renderer draws lines.
//!
//! Every surface's adapter turns its typed row into a [`Painted`] row —
//! what the gutter holds and the styled text the content column shows —
//! and the pane's row loop composes overlays (search, selection, the
//! cursor row) over it exactly as it does over a file's line.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::render::style::Tokens;

pub(crate) mod diff_line;
pub(crate) mod gutter;
pub(crate) mod heading;

/// One painted block row. `text` begins with the buffer row's own bytes
/// (a clipped heading name excepted, 0100 §3.8); anything after them is
/// render-only — padding, a leader, a fact.
#[derive(Default)]
pub(crate) struct Painted<'a> {
    pub gutter: Vec<Span<'static>>,
    pub text: Vec<Span<'static>>,
    /// The row's background across the text column (a break's band, the
    /// cursor row's band, a diff tint).
    pub row_bg: Option<Color>,
    /// Laid out against the pane, not the line: headings, breaks and
    /// titles ignore the pane's sideways scroll.
    pub fixed: bool,
    /// A changed line: its origin drives the inks, and the changed byte
    /// range (in the row's own bytes) is emphasised over the tint.
    pub line: Option<(&'a strop_git::DiffLine, Option<(usize, usize)>)>,
    /// A list row that runs past the last content cell ends in `…`
    /// there (0100 §3.9); it still scrolls sideways like any long line.
    pub cut: bool,
    /// One ink for the whole row's text (the review buffer's rows).
    pub row_fg: Option<Color>,
}

impl Painted<'_> {
    /// A row with no gutter mark and plain text.
    pub(crate) fn blank(gutter_cells: usize) -> Self {
        Self {
            gutter: vec![Span::raw(" ".repeat(gutter_cells))],
            text: Vec::new(),
            row_bg: None,
            fixed: false,
            ..Default::default()
        }
    }

    /// A heading laid across the gutter and the text column.
    pub(crate) fn heading(t: &Tokens, heading: &heading::Heading<'_>, text_cells: usize) -> Self {
        let drawn = heading.draw(t, heading.gutter + text_cells);
        Self {
            gutter: drawn.gutter,
            text: drawn.text,
            row_bg: None,
            fixed: true,
            ..Default::default()
        }
    }

    /// A break (0100 §3.1): the band from the pane's first column to the
    /// last content column, `⋮` where the numbers jump, the label muted
    /// and cut with `…`.
    pub(crate) fn band(
        t: &Tokens,
        gutter: Vec<Span<'static>>,
        label: Vec<Span<'static>>,
        text_cells: usize,
    ) -> Self {
        Self {
            gutter: gutter
                .into_iter()
                .map(|span| {
                    let style = span.style.bg(t.surface);
                    span.style(style)
                })
                .collect(),
            text: heading::clip_spans(label, text_cells),
            row_bg: Some(t.surface),
            fixed: true,
            ..Default::default()
        }
    }

    /// A title: what the buffer is, cut at the last content column.
    pub(crate) fn title(gutter_cells: usize, text: Vec<Span<'static>>, text_cells: usize) -> Self {
        Self {
            gutter: vec![Span::raw(" ".repeat(gutter_cells))],
            text: heading::clip_spans(text, text_cells),
            row_bg: None,
            fixed: true,
            ..Default::default()
        }
    }
}

/// The fewest cells a row's words keep before its fact gives way.
const LEAD_FLOOR: usize = 8;

/// A list row with its fact at the right edge (0100 §3.8): the words,
/// cut with `…` to leave two cells before the fact, then padding, then
/// the fact ending at `cells`. Too narrow for both, the fact goes and
/// the words are cut at `cells`.
pub(crate) fn fact_row(
    lead: Vec<Span<'static>>,
    fact: Vec<Span<'static>>,
    cells: usize,
) -> Vec<Span<'static>> {
    let fact_cells = heading::spans_width(&fact);
    if fact.is_empty() || cells < fact_cells + 2 + LEAD_FLOOR {
        return heading::clip_spans(lead, cells);
    }
    let mut spans = heading::clip_spans(lead, cells - fact_cells - 2);
    let used = heading::spans_width(&spans);
    spans.push(Span::raw(" ".repeat(cells - fact_cells - used)));
    spans.extend(fact);
    spans
}

/// A break's label ink.
pub(crate) fn muted(t: &Tokens, text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(t.muted))
}
