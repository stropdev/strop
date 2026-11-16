//! The breadcrumb (0107 §4): the names of the symbols that contain the
//! cursor, outer to inner, `outer › inner`, in the slot the layout gave
//! it. When the slot is narrower than the chain, whole outer segments
//! leave first behind one `…`, then the innermost name is cut at its end:
//! the innermost name is the last thing to lose a cell.

use std::borrow::Cow;

use ratatui::style::Style;

use crate::editor::SEGMENT_CAP;
use crate::render::clip::{clip_end, width};
use crate::render::style::Tokens;

use super::Row;

/// `›` between two segments, one cell either side.
const SEPARATOR: &str = " › ";
const SEPARATOR_CELLS: usize = 3;
/// `…` and its separator, standing for every dropped outer segment.
const ELIDED_CELLS: usize = 1 + SEPARATOR_CELLS;

/// What the slot shows: the kept segments, outermost first, each cut to
/// [`SEGMENT_CAP`], and whether outer segments were dropped.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Shown<'a> {
    pub elided: bool,
    pub segments: Vec<Cow<'a, str>>,
}

impl Shown<'_> {
    /// The cells this takes on the row.
    pub(super) fn cells(&self) -> usize {
        let names: usize = self.segments.iter().map(|segment| width(segment)).sum();
        let separators = SEPARATOR_CELLS * self.segments.len().saturating_sub(1);
        names + separators + if self.elided { ELIDED_CELLS } else { 0 }
    }
}

/// The breadcrumb `names` in at most `cells` cells (§4.3): whole if it
/// fits; else drop outer segments, outermost first, behind one `…`; else
/// cut the innermost name's end.
pub(super) fn fit(names: &[String], cells: usize) -> Shown<'_> {
    let segments: Vec<Cow<'_, str>> = names
        .iter()
        .map(|name| clip_end(name, SEGMENT_CAP))
        .collect();
    let widths: Vec<usize> = segments.iter().map(|segment| width(segment)).collect();
    let chain = |from: usize| -> usize {
        widths[from..].iter().sum::<usize>()
            + SEPARATOR_CELLS * (widths.len() - from).saturating_sub(1)
    };
    let Some(innermost) = segments.len().checked_sub(1) else {
        return Shown {
            elided: false,
            segments,
        };
    };
    if chain(0) <= cells {
        return Shown {
            elided: false,
            segments,
        };
    }
    if let Some(from) = (1..=innermost).find(|&from| ELIDED_CELLS + chain(from) <= cells) {
        return Shown {
            elided: true,
            segments: segments.into_iter().skip(from).collect(),
        };
    }
    let last = &names[innermost];
    if innermost > 0 && cells > ELIDED_CELLS {
        return Shown {
            elided: true,
            segments: vec![clip_end(last, cells - ELIDED_CELLS)],
        };
    }
    Shown {
        elided: false,
        segments: vec![clip_end(last, cells)],
    }
}

/// Draw the breadcrumb into its slot (§4.2): the innermost name in
/// `fg.secondary`, outer names in `fg.muted`, `›` and a leading `…` in
/// `fg.faint`. The slot is padded to its grant, so nothing after it
/// moves while the cursor does.
pub(super) fn draw<'a>(row: &mut Row<'a>, names: &'a [String], cells: usize, t: &Tokens) {
    let shown = fit(names, cells);
    let faint = Style::default().fg(t.faint);
    let used = shown.cells();
    if shown.elided {
        row.push("…", faint);
        row.push(SEPARATOR, faint);
    }
    let last = shown.segments.len().saturating_sub(1);
    for (at, segment) in shown.segments.into_iter().enumerate() {
        if at > 0 {
            row.push(SEPARATOR, faint);
        }
        let ink = if at == last { t.secondary } else { t.muted };
        row.push(segment, Style::default().fg(ink));
    }
    row.pad(cells.saturating_sub(used));
}
