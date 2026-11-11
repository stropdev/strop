//! `it` / `at`: the XML/HTML element around the cursor. A tag is what
//! Vim's pattern accepts: `<name …>` opens (unless it ends `/>`),
//! `</name>` closes; an opening tag with no matching close encloses
//! nothing.

use strop_core::Buffer;

/// An element's opening and closing tags, each as `[start, end)`.
pub(crate) struct Element {
    pub(crate) open: (usize, usize),
    pub(crate) close: (usize, usize),
}

enum Tag {
    Open,
    Close,
}

/// The tag starting at `at` (a `<`): its kind, name range and end.
fn tag_at(buf: &Buffer, at: usize) -> Option<(Tag, (usize, usize), usize)> {
    let n = buf.len_bytes();
    let name_end = |from: usize| {
        let mut end = from;
        while end < n && !matches!(buf.byte(end), b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
            end += 1;
        }
        end
    };
    let close_at = |from: usize| (from..n).find(|&i| buf.byte(i) == b'>');
    match buf.byte_at(at + 1)? {
        b'/' => {
            let name = (at + 2, name_end(at + 2));
            let end = close_at(name.1)?;
            (name.1 > name.0).then_some((Tag::Close, name, end + 1))
        }
        b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'!' => None,
        _ => {
            let name = (at + 1, name_end(at + 1));
            let end = close_at(name.1)?;
            (buf.byte(end - 1) != b'/').then_some((Tag::Open, name, end + 1))
        }
    }
}

fn same_name(buf: &Buffer, a: (usize, usize), b: (usize, usize)) -> bool {
    a.1 - a.0 == b.1 - b.0 && (0..a.1 - a.0).all(|i| buf.byte(a.0 + i) == buf.byte(b.0 + i))
}

/// The closing tag that matches the opening tag named `name`, scanning
/// from `from` and skipping nested elements of the same name.
fn matching_close(buf: &Buffer, name: (usize, usize), from: usize) -> Option<(usize, usize)> {
    let n = buf.len_bytes();
    let mut depth = 0usize;
    let mut at = from;
    while at < n {
        if buf.byte(at) == b'<' {
            if let Some((kind, other, end)) = tag_at(buf, at) {
                if same_name(buf, name, other) {
                    match kind {
                        Tag::Open => depth += 1,
                        Tag::Close if depth == 0 => return Some((at, end)),
                        Tag::Close => depth -= 1,
                    }
                }
                at = end;
                continue;
            }
        }
        at += 1;
    }
    None
}

/// The `count`-th element enclosing `cursor`, innermost first.
pub(crate) fn element(buf: &Buffer, cursor: usize, count: usize) -> Option<Element> {
    let mut from = Some(cursor.min(buf.len_bytes().checked_sub(1)?));
    let mut found = None;
    for _ in 0..count {
        found = None;
        while found.is_none() {
            let at = (0..=from?).rev().find(|&i| buf.byte(i) == b'<')?;
            from = at.checked_sub(1);
            let Some((Tag::Open, name, end)) = tag_at(buf, at) else {
                continue;
            };
            found = matching_close(buf, name, end)
                .filter(|close| close.1 > cursor)
                .map(|close| Element {
                    open: (at, end),
                    close,
                });
        }
    }
    found
}
