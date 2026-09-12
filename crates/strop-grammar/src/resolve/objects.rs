//! Text objects: bracket/quote pair scans, inner word, the surround
//! table — nesting-aware, cursor-on-delimiter counts as inside (vim).

use strop_core::Buffer;

/// % — matching pair. On a bracket: its mate. Else: first bracket on the
/// line right of cursor, then its mate (vim semantics).
pub fn match_pair(buf: &Buffer, pos: usize) -> Option<usize> {
    let end = buf.line_end(buf.line_of(pos));
    let from = (pos..end).find(|&at| super::pairs::delimiter_pair(buf.byte(at)).is_some())?;
    super::pairs::matching_delimiter_at(buf, from, || false)
        .ok()
        .flatten()
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

/// Object classes: blank / word / punct (a WORD object merges the last
/// two). Chars classify as a whole — continuation bytes inherit their
/// char's class, so run boundaries always land on char edges.
const BLANK: u8 = 0;
const WORD: u8 = 1;
const PUNCT: u8 = 2;

fn object_class(buf: &Buffer, pos: usize, big: bool) -> u8 {
    let b = buf.byte(pos);
    let character = if b.is_ascii() {
        b as char
    } else {
        buf.text().char(buf.text().byte_to_char(pos))
    };
    if character.is_whitespace() {
        return BLANK;
    }
    if big || character.is_alphanumeric() || character == '_' {
        WORD
    } else {
        PUNCT
    }
}

/// iw/aw/iW/aW — vim's current_word, count one, outside Visual mode:
///
/// - inner: the run under the cursor of its own class (a blank run is
///   itself a selectable object; word and punct runs are distinct).
/// - around a word/punct run: plus the trailing blank run on the line;
///   with none, plus the leading blank run — never indent (a blank run
///   starting at column 0 stays).
/// - around a blank run: plus the following word/punct run, across line
///   breaks; no following run refuses the whole command (vim's FAIL).
pub(crate) fn word_object(
    buf: &Buffer,
    pos: usize,
    big: bool,
    inner: bool,
) -> Option<(usize, usize)> {
    let n = buf.len_bytes();
    if pos >= n {
        return None;
    }
    let class = object_class(buf, pos, big);
    let mut s = pos;
    while s > 0 && object_class(buf, s - 1, big) == class {
        s -= 1;
    }
    let mut e = pos + 1;
    while e < n && object_class(buf, e, big) == class {
        e += 1;
    }
    if inner {
        return Some((s, e));
    }
    if class == BLANK {
        let mut end = e;
        while end < n && object_class(buf, end, big) == BLANK {
            end += 1;
        }
        if end >= n {
            return None; // trailing blank run with nothing after: vim FAILs
        }
        let word = object_class(buf, end, big);
        while end < n && object_class(buf, end, big) == word {
            end += 1;
        }
        return Some((s, end));
    }
    let mut trail = e;
    while trail < n && buf.byte(trail) != b'\n' && object_class(buf, trail, big) == BLANK {
        trail += 1;
    }
    if trail > e {
        return Some((s, trail));
    }
    if s > 0 && buf.byte(s - 1) != b'\n' && object_class(buf, s - 1, big) == BLANK {
        let mut lead = s;
        while lead > 0 && buf.byte(lead - 1) != b'\n' && object_class(buf, lead - 1, big) == BLANK {
            lead -= 1;
        }
        if lead > 0 && buf.byte(lead - 1) != b'\n' {
            return Some((lead, e));
        }
    }
    Some((s, e))
}

/// The occurrence-selection seed (0049 §7): the word or punctuation
/// run under `pos` by vim's small-word classes — `gb` selects exactly
/// what `iw` classifies, and a caret on whitespace seeds nothing.
pub fn word_run(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
    let n = buf.len_bytes();
    if pos >= n {
        return None;
    }
    let class = object_class(buf, pos, false);
    if class == BLANK {
        return None;
    }
    let mut start = pos;
    while start > 0 && object_class(buf, start - 1, false) == class {
        start -= 1;
    }
    let mut end = pos + 1;
    while end < n && object_class(buf, end, false) == class {
        end += 1;
    }
    Some((start, end))
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
