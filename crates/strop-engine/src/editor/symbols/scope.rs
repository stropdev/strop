//! The scope index (0107 §6.2): which symbols contain a byte, read from
//! the kept symbol list of the current document — the breadcrumb on the
//! modeline. Pure: no editor, no channels (the shape of `store.rs`), so
//! the property tests in `scope_tests.rs` drive exactly this.
//!
//! Spans are byte ranges `start..end`, sorted by start with the outer
//! span first, each with a link to the span that contains it. A lookup is
//! a binary search and a walk up those links; an edit maps every span
//! through the proved edit map (`strop_kernel::editmap`), so the index
//! follows the text through typing until the next exact list replaces it.

use std::cmp::Reverse;
use std::sync::Arc;

use ropey::Rope;
use strop_core::id::{BufferRevision, DocumentId};
use strop_lsp::{LineSpan, ProtoSymbol};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::store::Producer;

/// The breadcrumb's slot is never wider than this.
pub const SCOPE_CAP: usize = 40;
/// One segment (a symbol's name) is cut to this many cells.
pub const SEGMENT_CAP: usize = 32;
/// ` › ` between two segments.
pub const SEPARATOR_CELLS: usize = 3;
/// A chain deeper than this shows nothing.
pub const MAX_DEPTH: usize = 64;
/// An edit batch costing more span maps than this drops the index
/// instead: a `:%s` over a huge file must not cost the loop a readout.
const WORK_CAP: usize = 1 << 20;
const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
    /// The span that contains this one, or [`NONE`].
    parent: u32,
    /// The row of the kept list this span is.
    symbol: u32,
}

pub(crate) struct Scopes {
    pub(crate) document: DocumentId,
    pub(crate) producer: Producer,
    /// The kept list: the names live here.
    list: Arc<Vec<ProtoSymbol>>,
    spans: Vec<Span>,
    /// Cells of the widest chain this list can show, capped.
    slot: usize,
    /// The text these bytes describe.
    pub(crate) revision: BufferRevision,
    /// False once an edit was mapped through.
    pub(crate) exact: bool,
}

/// The chain at one byte, outermost first: what the modeline shows.
/// Borrowed from the index; no allocation.
pub struct ScopePath<'a> {
    list: &'a [ProtoSymbol],
    chain: [u32; MAX_DEPTH],
    len: usize,
    /// Cells the breadcrumb's slot takes for this file.
    pub slot: usize,
}

impl<'a> ScopePath<'a> {
    /// The names, outermost first.
    pub fn names(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.chain[..self.len]
            .iter()
            .filter_map(|&symbol| self.list.get(symbol as usize))
            .map(|symbol| symbol.name.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Display cells of a name, as the modeline measures them: printable
/// graphemes, each as wide as the terminal draws it.
pub fn cells(text: &str) -> usize {
    text.graphemes(true)
        .map(|grapheme| strop_core::layout::printable_grapheme(grapheme).width())
        .sum()
}

/// The bytes of whole lines `span.first..=span.last`: from the first
/// line's first byte to the last line's end before its line break.
fn byte_range(rope: &Rope, span: LineSpan) -> (usize, usize) {
    let lines = rope.len_lines();
    let first = span.first.get().min(lines.saturating_sub(1));
    let last = span.last.get().clamp(first, lines.saturating_sub(1));
    let start = rope.line_to_byte(first);
    let mut end = rope.line_to_byte(last + 1);
    if last + 1 < lines {
        // strip the break: `\n`, or `\r\n`
        end = end.saturating_sub(1);
        if end > start && rope.byte(end - 1) == b'\r' {
            end -= 1;
        }
    }
    (start, end.max(start))
}

impl Scopes {
    /// The index of `list`, which describes `rope` at `revision`.
    pub(crate) fn build(
        document: DocumentId,
        producer: Producer,
        revision: BufferRevision,
        list: Arc<Vec<ProtoSymbol>>,
        rope: &Rope,
    ) -> Self {
        let ranges = list
            .iter()
            .map(|symbol| byte_range(rope, symbol.span))
            .collect();
        Self::from_ranges(document, producer, revision, list, ranges)
    }

    /// The index over explicit byte ranges, one per row of `list`.
    pub(super) fn from_ranges(
        document: DocumentId,
        producer: Producer,
        revision: BufferRevision,
        list: Arc<Vec<ProtoSymbol>>,
        ranges: Vec<(usize, usize)>,
    ) -> Self {
        debug_assert_eq!(ranges.len(), list.len());
        let mut spans: Vec<Span> = ranges
            .into_iter()
            .enumerate()
            .map(|(symbol, (start, end))| Span {
                start,
                end: end.max(start),
                parent: NONE,
                symbol: symbol as u32,
            })
            .collect();
        spans.sort_by_key(|span| (span.start, Reverse(span.end), span.symbol));
        nest(&mut spans);
        let slot = slot(&spans, &list);
        Self {
            document,
            producer,
            list,
            spans,
            slot,
            revision,
            exact: true,
        }
    }

    /// Cells of the breadcrumb's slot for this list.
    pub(crate) fn slot(&self) -> usize {
        self.slot
    }

    /// The innermost span containing `byte`, as its index in `spans`.
    fn innermost(&self, byte: usize) -> Option<usize> {
        let mut at = self
            .spans
            .partition_point(|span| span.start <= byte)
            .checked_sub(1)?;
        for _ in 0..MAX_DEPTH {
            let span = self.spans[at];
            if byte < span.end {
                return Some(at);
            }
            if span.parent == NONE {
                return None;
            }
            at = span.parent as usize;
        }
        None
    }

    /// The symbol rows containing `byte`, outermost first, into `chain`;
    /// the count written. Zero at top level and for a chain deeper than
    /// `chain` holds.
    pub(crate) fn path(&self, byte: usize, chain: &mut [u32]) -> usize {
        let Some(mut at) = self.innermost(byte) else {
            return 0;
        };
        let mut len = 0;
        loop {
            let Some(slot) = chain.get_mut(len) else {
                return 0;
            };
            let span = self.spans[at];
            *slot = span.symbol;
            len += 1;
            if span.parent == NONE {
                break;
            }
            at = span.parent as usize;
        }
        chain[..len].reverse();
        len
    }

    /// The breadcrumb at `byte`.
    pub(crate) fn at(&self, byte: usize) -> ScopePath<'_> {
        let mut chain = [0; MAX_DEPTH];
        let len = self.path(byte, &mut chain);
        ScopePath {
            list: &self.list,
            chain,
            len,
            slot: self.slot,
        }
    }

    /// Carry the index through `changes`. `false`: the index cannot follow
    /// them (a revision it never saw, or too much work) and must be
    /// dropped.
    pub(crate) fn apply(&mut self, changes: &[strop_core::Change]) -> bool {
        if changes.len().saturating_mul(self.spans.len()) > WORK_CAP {
            return false;
        }
        for change in changes {
            if change.revision.get() != self.revision.get().saturating_add(1) {
                return false;
            }
            let (start, old_end, new_end) = change.extent();
            // parents come first: an empty span (it contains nothing) is
            // kept inside its parent, so the order stays sorted
            for at in 0..self.spans.len() {
                let mut span = map_span(self.spans[at], start, old_end, new_end);
                if span.start == span.end && span.parent != NONE {
                    let outer = self.spans[span.parent as usize].end;
                    span.start = span.start.min(outer);
                    span.end = span.start;
                }
                self.spans[at] = span;
            }
            self.revision = change.revision;
            self.exact = false;
        }
        true
    }

    /// Every span as `(symbol, start, end, parent's symbol)`, in index
    /// order.
    #[cfg(test)]
    pub(super) fn spans(&self) -> Vec<(u32, usize, usize, Option<u32>)> {
        self.spans
            .iter()
            .map(|span| {
                let parent = (span.parent != NONE).then(|| self.spans[span.parent as usize].symbol);
                (span.symbol, span.start, span.end, parent)
            })
            .collect()
    }
}

/// One span through one edit. The start maps as it is; the end maps as
/// the byte before it, plus one — so text typed after a closing brace
/// stays outside and text typed in a body grows the span — and never
/// past where the byte at the end goes, so spans that did not overlap
/// still do not. An empty span stays empty.
fn map_span(span: Span, start: usize, old_end: usize, new_end: usize) -> Span {
    use strop_kernel::editmap::map_position;
    let first = map_position(span.start, start, old_end, new_end);
    let end = if span.end > span.start {
        let inside = map_position(span.end - 1, start, old_end, new_end) + 1;
        inside.min(map_position(span.end, start, old_end, new_end))
    } else {
        first
    };
    Span {
        start: first,
        end: end.max(first),
        ..span
    }
}

/// Link every span to the innermost earlier span that contains its
/// start. A span that half-overlaps its neighbour (a sloppy server) is
/// clamped to the parent it starts in.
fn nest(spans: &mut [Span]) {
    let mut stack: Vec<u32> = Vec::new();
    for at in 0..spans.len() {
        while let Some(&top) = stack.last() {
            if spans[at].start < spans[top as usize].end {
                break;
            }
            stack.pop();
        }
        if let Some(&top) = stack.last() {
            let outer = spans[top as usize].end;
            let span = &mut spans[at];
            span.end = span.end.min(outer);
            span.parent = top;
        }
        stack.push(at as u32);
    }
}

/// Cells of the widest chain any span can show, capped at [`SCOPE_CAP`].
fn slot(spans: &[Span], list: &[ProtoSymbol]) -> usize {
    let mut chains = vec![0usize; spans.len()];
    let mut widest = 0;
    for at in 0..spans.len() {
        let span = spans[at];
        let name = list
            .get(span.symbol as usize)
            .map_or(0, |symbol| cells(&symbol.name).min(SEGMENT_CAP));
        let outer = if span.parent == NONE {
            0
        } else {
            chains[span.parent as usize] + SEPARATOR_CELLS
        };
        chains[at] = outer + name;
        widest = widest.max(chains[at]);
    }
    widest.min(SCOPE_CAP)
}
