//! Search entry points. The literal fast path streams rope chunks
//! (file contents are never materialized); everything else runs the
//! bounded engine in `query::exec`. One seam, two speeds — same
//! semantics, decided by the compiled program itself.

use strop_core::id::ByteOffset;
use strop_core::Buffer;

use crate::query::exec::{all_matches, first_from, Matcher};
use crate::query::{CompiledQuery, QueryError, SearchMatch};

/// KMP over rope chunks for a plain byte needle (the program is a
/// case-sensitive literal — the common `/word` search).
fn visit(
    buf: &Buffer,
    needle: &[u8],
    from: usize,
    mut matched: impl FnMut(usize) -> std::ops::ControlFlow<()>,
) {
    if needle.is_empty() {
        return;
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
    let mut offset = 0;
    for chunk in buf.text().chunks() {
        for &byte in chunk.as_bytes() {
            offset += 1;
            if offset <= from {
                continue;
            }
            while length > 0 && byte != needle[length] {
                length = prefixes[length - 1];
            }
            if byte == needle[length] {
                length += 1;
            }
            if length == needle.len() {
                if matched(offset - length).is_break() {
                    return;
                }
                length = prefixes[length - 1];
            }
        }
    }
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
        visit(buf, needle, from, |offset| {
            found = Some(literal_hit(needle, offset));
            std::ops::ControlFlow::Break(())
        });
        return Ok(found);
    }
    let matcher = Matcher::new(buf.text());
    let from = buf.ceil_boundary(from.min(buf.len_bytes()));
    first_from(&matcher, prog, from).map_err(QueryError::from)
}

pub(crate) fn backward(
    buf: &Buffer,
    from: usize,
    query: &CompiledQuery,
) -> Result<Option<SearchMatch>, QueryError> {
    let from = from.min(buf.len_bytes());
    let prog = query.program();
    if let Some(needle) = &prog.literal {
        let mut found = None;
        visit(buf, needle, 0, |offset| {
            if offset < from {
                found = Some(literal_hit(needle, offset));
            }
            std::ops::ControlFlow::Continue(())
        });
        return Ok(found);
    }
    // last match whose start precedes `from`, even if its end crosses
    // the cursor — the `?pat` contract the resolver's tests pin
    let matcher = Matcher::new(buf.text());
    Ok(all_matches(&matcher, prog)?
        .into_iter()
        .rfind(|hit| hit.start.get() < from))
}

pub(crate) fn all(buf: &Buffer, query: &CompiledQuery) -> Result<Vec<SearchMatch>, QueryError> {
    let prog = query.program();
    if let Some(needle) = &prog.literal {
        let mut hits = Vec::new();
        visit(buf, needle, 0, |offset| {
            hits.push(literal_hit(needle, offset));
            std::ops::ControlFlow::Continue(())
        });
        return Ok(hits);
    }
    let matcher = Matcher::new(buf.text());
    all_matches(&matcher, prog).map_err(QueryError::from)
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
