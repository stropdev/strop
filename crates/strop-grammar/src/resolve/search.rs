//! Search entry points. The literal fast path streams rope chunks
//! (file contents are never materialized); everything else runs the
//! bounded engine in `query::exec`. One seam, two speeds — same
//! semantics, decided by the compiled program itself.

use strop_core::id::ByteOffset;
use strop_core::Buffer;

use crate::query::exec::{first_from, visit_matches, Matcher};
use crate::query::{CompiledQuery, QueryError, SearchMatch};

/// KMP over rope chunks for a plain byte needle (the program is a
/// case-sensitive literal — the common `/word` search, and occurrence
/// selection's matcher). `cancelled` is the work owner's cooperative
/// stop; occurrence scans pass a never-cancelled flag.
fn kmp(
    buf: &Buffer,
    needle: &[u8],
    from: usize,
    cancelled: &dyn Fn() -> bool,
    mut matched: impl FnMut(usize) -> std::ops::ControlFlow<()>,
) -> Result<(), QueryError> {
    if cancelled() {
        return Err(QueryError::Cancelled);
    }
    if needle.is_empty() {
        return Ok(());
    }
    let from = buf.ceil_boundary(from.min(buf.len_bytes()));
    let mut prefixes = vec![0; needle.len()];
    for index in 1..needle.len() {
        let mut length = prefixes[index - 1];
        while length > 0 && needle[index] != needle[length] {
            length = prefixes[length - 1];
        }
        if needle[index] == needle[length] {
            length += 1;
        }
        prefixes[index] = length;
    }
    let mut length = 0;
    let mut offset = from;
    let suffix = buf.text().byte_slice(from..);
    for chunk in suffix.chunks() {
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        for &byte in chunk.as_bytes() {
            offset += 1;
            while length > 0 && byte != needle[length] {
                length = prefixes[length - 1];
            }
            if byte == needle[length] {
                length += 1;
            }
            if length == needle.len() {
                if matched(offset - length).is_break() {
                    return Ok(());
                }
                length = prefixes[length - 1];
            }
        }
    }
    if cancelled() {
        Err(QueryError::Cancelled)
    } else {
        Ok(())
    }
}

fn visit(
    buf: &Buffer,
    needle: &[u8],
    from: usize,
    query: &CompiledQuery,
    matched: impl FnMut(usize) -> std::ops::ControlFlow<()>,
) -> Result<(), QueryError> {
    kmp(buf, needle, from, &|| query.cancelled(), matched)
}

/// Occurrence selection's matcher (0049 §7): plain bytes, no pattern
/// syntax, never cancelled — the same chunk-streaming KMP a literal
/// `/word` search uses, so punctuation in the needle is just bytes.
/// Returns the first `[start, end)` at or after `from` (clamped up to
/// a char boundary).
pub fn literal_from(buf: &Buffer, from: usize, needle: &[u8]) -> Option<(usize, usize)> {
    let mut found = None;
    // infallible: no query engine, no cancellation
    let _ = kmp(buf, needle, from, &|| false, |offset| {
        found = Some((offset, offset + needle.len()));
        std::ops::ControlFlow::Break(())
    });
    found
}

fn literal_hit(needle: &[u8], at: usize) -> SearchMatch {
    SearchMatch {
        start: ByteOffset::new(at),
        end: ByteOffset::new(at + needle.len()),
    }
}

pub(crate) fn forward(
    buf: &Buffer,
    from: usize,
    query: &CompiledQuery,
) -> Result<Option<SearchMatch>, QueryError> {
    let prog = query.program();
    if let Some(needle) = &prog.literal {
        let mut found = None;
        visit(buf, needle, from, query, |offset| {
            found = Some(literal_hit(needle, offset));
            std::ops::ControlFlow::Break(())
        })?;
        return Ok(found);
    }
    let matcher = Matcher::new(buf.text(), query);
    let from = buf.ceil_boundary(from.min(buf.len_bytes()));
    first_from(&matcher, prog, from)
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
            return std::ops::ControlFlow::Break(());
        }
        found = Some(hit);
        std::ops::ControlFlow::Continue(())
    })?;
    Ok(found)
}

pub(crate) fn all(buf: &Buffer, query: &CompiledQuery) -> Result<Vec<SearchMatch>, QueryError> {
    let mut hits = Vec::new();
    visit_all(buf, query, |hit| {
        hits.push(hit);
        std::ops::ControlFlow::Continue(())
    })?;
    Ok(hits)
}

pub(crate) fn visit_all(
    buf: &Buffer,
    query: &CompiledQuery,
    mut visitor: impl FnMut(SearchMatch) -> std::ops::ControlFlow<()>,
) -> Result<(), QueryError> {
    let program = query.program();
    if let Some(needle) = &program.literal {
        visit(buf, needle, 0, query, |offset| {
            visitor(literal_hit(needle, offset))
        })
    } else {
        visit_matches(&Matcher::new(buf.text(), query), program, visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(buf: &Buffer, pat: &str) -> Vec<(usize, usize)> {
        let q = CompiledQuery::compile(pat, false).expect("compiles");
        all(buf, &q)
            .expect("runs")
            .into_iter()
            .map(|m| (m.start.get(), m.end.get()))
            .collect()
    }

    #[test]
    fn literal_hits_carry_explicit_ends() {
        let buf = Buffer::from_text("ab cd ab\n");
        assert_eq!(hits(&buf, "ab"), vec![(0, 2), (6, 8)]);
    }

    #[test]
    fn regex_hits_carry_explicit_ends() {
        let buf = Buffer::from_text("a ab abc\n");
        assert_eq!(hits(&buf, "a\\+"), vec![(0, 1), (2, 3), (5, 6)]);
    }

    #[test]
    fn empty_matches_walk_but_never_past_eof() {
        // vim counts 3 matches for x* on "aaa": one per position, none
        // after the final char
        let buf = Buffer::from_text("bbb\n");
        assert_eq!(hits(&buf, "x*"), vec![(0, 0), (1, 1), (2, 2), (3, 3)]);
    }
}
