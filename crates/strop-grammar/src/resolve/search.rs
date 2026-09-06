//! Literal matching streams rope chunks; file contents are never materialized.
use std::ops::ControlFlow;
use strop_core::Buffer;

fn visit(
    buf: &Buffer,
    pattern: &str,
    from: usize,
    mut matched: impl FnMut(usize) -> ControlFlow<()>,
) {
    let needle = pattern.as_bytes();
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
    for chunk in buf.rope.chunks() {
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

/// First match starting at/after `from`, clamped to a UTF-8 boundary.
pub fn search_forward(buf: &Buffer, from: usize, pattern: &str) -> Option<usize> {
    let mut found = None;
    visit(buf, pattern, from, |offset| {
        found = Some(offset);
        ControlFlow::Break(())
    });
    found
}

/// Last match whose start precedes `from`, even if its end crosses the cursor.
pub fn search_backward(buf: &Buffer, from: usize, pattern: &str) -> Option<usize> {
    let from = from.min(buf.len_bytes());
    let mut found = None;
    visit(buf, pattern, 0, |offset| {
        if offset >= from {
            return ControlFlow::Break(());
        }
        found = Some(offset);
        ControlFlow::Continue(())
    });
    found
}

/// Nonoverlapping display hits (the historical `match_indices` contract).
pub fn search_all(buf: &Buffer, pattern: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut next = 0;
    visit(buf, pattern, 0, |offset| {
        if offset >= next {
            hits.push(offset);
            next = offset + pattern.len();
        }
        ControlFlow::Continue(())
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_origins_and_chunk_crossing_match_without_slicing_panics() {
        let text = format!("{}éneedle{}", "a".repeat(1023), "b".repeat(1024));
        let buffer = Buffer::from_text(&text);
        assert_eq!(search_forward(&buffer, 1024, "needle"), Some(1025));
        assert_eq!(search_forward(&buffer, usize::MAX, "needle"), None);
        assert_eq!(search_backward(&buffer, 1027, "needle"), Some(1025));
        assert_eq!(search_all(&Buffer::from_text("aaaa"), "aa"), vec![0, 2]);
    }
}
