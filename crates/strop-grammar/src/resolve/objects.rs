//! Text objects: bracket/quote pair scans, inner word, the surround
//! table — nesting-aware, cursor-on-delimiter counts as inside (vim).

use strop_core::Buffer;

use super::motions::*;

/// % — matching pair. On a bracket: its mate. Else: first bracket on the
/// line right of cursor, then its mate (vim semantics).
pub fn match_pair(buf: &Buffer, pos: usize) -> Option<usize> {
    const PAIRS: &[(u8, u8)] = &[(b'(', b')'), (b'[', b']'), (b'{', b'}'), (b'<', b'>')];

    let on = buf
        .byte_at(pos)
        .and_then(|b| PAIRS.iter().find(|(o, c)| *o == b || *c == b));
    let (open, close, from) = match on {
        Some(&(o, c)) => (o as char, c as char, pos),
        None => {
            let end = buf.line_end(buf.line_of(pos));
            let mut i = pos;
            loop {
                if i >= end {
                    return None;
                }
                if let Some(&(o, c)) = PAIRS
                    .iter()
                    .find(|(o, c)| *o == buf.byte(i) || *c == buf.byte(i))
                {
                    break (o as char, c as char, i);
                }
                i += 1;
            }
        }
    };
    let (o, c) = bracket_pair(buf, from, open, close)?;
    let b = buf.byte_at(from)?;
    if b == open as u8 {
        Some(c)
    } else {
        Some(o)
    }
}

/// Find the enclosing bracket pair around `pos` (nesting-aware scan).
/// Cursor on either delimiter counts as inside the pair (vim semantics):
/// the backward scan starts just inside a close, the forward scan just
/// past the open.
pub(crate) fn bracket_pair(
    buf: &Buffer,
    pos: usize,
    open: char,
    close: char,
) -> Option<(usize, usize)> {
    // delimiters are ASCII by construction (the parser's alias map)
    let (open, close) = (open as u32, close as u32);
    let open = u8::try_from(open).expect("ascii delimiter");
    let close = u8::try_from(close).expect("ascii delimiter");
    let n = buf.len_bytes();
    if n == 0 {
        return None;
    }
    let mut o = pos.min(n - 1);
    if buf.byte(o) == close && o > 0 {
        o -= 1;
    }
    // scan back for the unmatched open
    let mut depth = 0i32;
    loop {
        let b = buf.byte(o);
        if b == close {
            depth += 1;
        } else if b == open {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        if o == 0 {
            return None;
        }
        o -= 1;
    }
    let open_pos = o;
    // scan forward from just past the open for its close
    let mut depth = 0i32;
    let mut c = open_pos + 1;
    loop {
        if c >= n {
            return None;
        }
        let b = buf.byte(c);
        if b == open {
            depth += 1;
        } else if b == close {
            if depth == 0 {
                return Some((open_pos, c));
            }
            depth -= 1;
        }
        c += 1;
    }
}

/// Quote pair on the current line. Vim's quote objects scan the whole
/// line: enclosing pair when inside or on a quote; the *next* pair when
/// the cursor sits before any quote on the line; nothing when past the
/// last pair.
pub(crate) fn quote_pair(buf: &Buffer, pos: usize, q: char) -> Option<(usize, usize)> {
    let q = u8::try_from(q as u32).expect("ascii delimiter");
    let line = buf.line_of(pos);
    let start = buf.line_start(line);
    let end = buf.line_end(line);
    let open = (start..=pos.min(end)).rev().find(|&i| buf.byte(i) == q);
    let open = match open {
        Some(o) => o,
        None => (pos..end).find(|&i| buf.byte(i) == q)?, // forward-scan fallback
    };
    let close = (open + 1..end).find(|&i| buf.byte(i) == q)?;
    if pos > close {
        return None;
    }
    Some((open, close))
}

pub(crate) fn inner_word(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
    if pos >= buf.len_bytes() || !is_word(buf.byte(pos)) {
        return None;
    }
    let mut s = pos;
    while s > 0 && is_word(buf.byte(s - 1)) {
        s -= 1;
    }
    let mut e = pos;
    while e + 1 < buf.len_bytes() && is_word(buf.byte(e + 1)) {
        e += 1;
    }
    Some((s, e + 1)) // half-open
}

/// Search forward for `pat` (prototype: plain substring; 0001 §2.5's
/// transpiled regex lands with the real search layer).
/// Map a surround char to its pair (sandwich aliases b/B/r/a).
pub(crate) fn surround_pair(ch: char) -> Option<(char, char)> {
    Some(match ch {
        'b' | '(' | ')' => ('(', ')'),
        'B' | '{' | '}' => ('{', '}'),
        'r' | '[' | ']' => ('[', ']'),
        'a' | '<' | '>' => ('<', '>'),
        q @ ('"' | '\'' | '`') => (q, q),
        _ => return None,
    })
}
