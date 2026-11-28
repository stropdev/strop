//! Search entry points. A program that is one literal run streams
//! through the literal scanner (file contents are never materialized);
//! everything else runs the bounded engine in `query::exec`. One seam,
//! two speeds — same answers, decided by the compiled program itself.

mod literal;

use std::ops::ControlFlow;

use strop_core::id::ByteOffset;
use strop_core::Buffer;

use crate::query::exec::{first_from, Matcher};
use crate::query::{CompiledQuery, QueryError, SearchMatch};
use literal::{scan, Scan};

/// Run the program's literal through the scanner from `from` (a char
/// boundary). `None` when the program is not a literal run.
fn scan_literal(
    buf: &Buffer,
    query: &CompiledQuery,
    from: usize,
    mut matched: impl FnMut(SearchMatch) -> ControlFlow<()>,
) -> Option<Result<Scan, QueryError>> {
    let lit = query.program().literal.as_ref()?;
    let matcher = Matcher::new(buf.text(), query);
    let n = lit.bytes.len();
    Some(scan(
        buf.text(),
        &lit.bytes,
        lit.fold,
        from,
        &|| query.cancelled(),
        |start, end| {
            (!lit.word_before || matcher.word_before(start))
                && (!lit.word_after || matcher.word_after(end))
        },
        |at| matched(hit(at, at + n)),
    ))
}

fn hit(start: usize, end: usize) -> SearchMatch {
    SearchMatch {
        start: ByteOffset::new(start),
        end: ByteOffset::new(end),
    }
}

/// Occurrence selection's matcher (0049 §7): plain bytes, no pattern
/// syntax, never cancelled — the same scanner a literal `/word` search
/// uses, so punctuation in the needle is just bytes. Returns the first
/// `[start, end)` at or after `from` (clamped up to a char boundary).
pub fn literal_from(buf: &Buffer, from: usize, needle: &[u8]) -> Option<(usize, usize)> {
    let from = buf.ceil_boundary(from.min(buf.len_bytes()));
    let mut found = None;
    // infallible: never cancelled, and an exact needle never resumes
    let _ = scan(
        buf.text(),
        needle,
        false,
        from,
        &|| false,
        |_, _| true,
        |offset| {
            found = Some((offset, offset + needle.len()));
            ControlFlow::Break(())
        },
    );
    found
}

pub(crate) fn forward(
    buf: &Buffer,
    from: usize,
    query: &CompiledQuery,
) -> Result<Option<SearchMatch>, QueryError> {
    let from = buf.ceil_boundary(from.min(buf.len_bytes()));
    let mut found = None;
    let resume = match scan_literal(buf, query, from, |hit| {
        found = Some(hit);
        ControlFlow::Break(())
    }) {
        None => from,
        Some(scanned) => match scanned? {
            Scan::Done => return Ok(found),
            Scan::Resume(at) => buf.ceil_boundary(at),
        },
    };
    first_from(&Matcher::new(buf.text(), query), query.program(), resume)
}

pub(crate) fn backward(
    buf: &Buffer,
    from: usize,
    query: &CompiledQuery,
) -> Result<Option<SearchMatch>, QueryError> {
    let from = from.min(buf.len_bytes());
    let mut found = None;
    visit_all(buf, query, |hit| {
        if hit.start.get() >= from {
            return ControlFlow::Break(());
        }
        found = Some(hit);
        ControlFlow::Continue(())
    })?;
    Ok(found)
}

pub(crate) fn all(buf: &Buffer, query: &CompiledQuery) -> Result<Vec<SearchMatch>, QueryError> {
    let mut hits = Vec::new();
    visit_all(buf, query, |hit| {
        hits.push(hit);
        ControlFlow::Continue(())
    })?;
    Ok(hits)
}

pub(crate) fn visit_all(
    buf: &Buffer,
    query: &CompiledQuery,
    mut visitor: impl FnMut(SearchMatch) -> ControlFlow<()>,
) -> Result<(), QueryError> {
    visit_captures(buf, query, 0, |hit, _| visitor(hit))
}

/// Every match starting at or after `from` (a character boundary) with
/// its capture groups (`:s` replacements read `\1`). Text before `from`
/// is still context for anchors. A literal run has no groups, so it
/// takes the scanner until (if ever) the scanner hands back.
pub(crate) fn visit_captures(
    buf: &Buffer,
    query: &CompiledQuery,
    from: usize,
    mut visitor: impl FnMut(SearchMatch, &[Option<SearchMatch>]) -> ControlFlow<()>,
) -> Result<(), QueryError> {
    let mut stopped = false;
    let resume = match scan_literal(buf, query, from, |hit| {
        let flow = visitor(hit, &[]);
        stopped = flow.is_break();
        flow
    }) {
        None => from,
        Some(scanned) => match scanned? {
            Scan::Done => return Ok(()),
            Scan::Resume(at) => buf.ceil_boundary(at),
        },
    };
    debug_assert!(!stopped, "a stopped walk never resumes");
    crate::query::exec::visit_captures(
        &Matcher::new(buf.text(), query),
        query.program(),
        resume,
        visitor,
    )
}

#[cfg(test)]
mod tests;
