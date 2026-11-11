//! Text objects: bracket blocks, quotes, tags, paragraphs and the
//! surround table — nesting-aware, cursor-on-delimiter counts as inside
//! (vim). Each object answers with an `Extent` in Vim's own terms (a
//! start, an end position and how the end counts), so the range rules in
//! `resolve` read objects and motions the same way.

use strop_core::Buffer;

use super::walk::{Step, Walk};
use super::{blocks, tags, words, Kind};
use crate::types::Object;

/// What a text object covers: `end` is a cursor position, read by `kind`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Extent {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: Kind,
}

/// Resolve a text object at `cursor`. `selection` is a Visual selection
/// of more than one character, as `(anchor, head)`: word, paragraph and
/// bracket objects extend it, and a word object then answers in that
/// order (`start` is the anchor, `end` the new head).
pub(crate) fn extent(
    buf: &Buffer,
    cursor: usize,
    selection: Option<(usize, usize)>,
    object: Object,
    inner: bool,
    count: usize,
) -> Option<Extent> {
    let extending = selection.filter(|(anchor, head)| anchor != head);
    match object {
        Object::Word | Object::BigWord => {
            let big = object == Object::BigWord;
            let word = words::object(buf, cursor, extending, count, !inner, big)?;
            Some(Extent {
                start: word.start,
                end: word.end,
                kind: if word.inclusive {
                    Kind::Inclusive
                } else {
                    Kind::Exclusive
                },
            })
        }
        Object::Paragraph => {
            if let Some((anchor, head)) = extending {
                if buf.line_of(anchor) != buf.line_of(head) {
                    let line = blocks::paragraph_extend(buf, anchor, head, count, !inner)?;
                    return Some(Extent {
                        start: anchor,
                        end: buf.line_start(line),
                        kind: Kind::Linewise,
                    });
                }
            }
            let (first, last) = blocks::paragraph_object(buf, cursor, count, !inner)?;
            Some(Extent {
                start: buf.line_start(first),
                end: buf.line_start(last),
                kind: Kind::Linewise,
            })
        }
        Object::Quote(quote) => {
            let (open, close) = quote_pair(buf, cursor, quote)?;
            let line_end = buf.line_end(buf.line_of(open));
            let blank = |at: usize| matches!(buf.byte(at), b' ' | b'\t');
            let (mut start, mut end) = (open, close + 1);
            if inner && count < 2 {
                (start, end) = (open + 1, close);
            } else if !inner {
                if end < line_end && blank(end) {
                    while end < line_end && blank(end) {
                        end += 1;
                    }
                } else {
                    let line_start = buf.line_start(buf.line_of(open));
                    while start > line_start && blank(start - 1) {
                        start -= 1;
                    }
                }
            }
            Some(Extent {
                start,
                end,
                kind: Kind::Exclusive,
            })
        }
        Object::Bracket { open, close } => {
            block(buf, cursor, selection, count, !inner, open, close)
        }
        Object::Tag => {
            let element = tags::element(buf, cursor, count)?;
            let exclusive = |start, end| Extent {
                start,
                end,
                kind: Kind::Exclusive,
            };
            if !inner {
                return Some(exclusive(element.open.0, element.close.1));
            }
            let (start, end) = (element.open.1, element.close.0);
            if start == end {
                return Some(exclusive(start, end));
            }
            let mut last = Walk::new(buf, end);
            last.dec();
            Some(Extent {
                start,
                end: last.pos,
                kind: Kind::Inclusive,
            })
        }
    }
}

/// `i(` / `a(` and friends with a count (Vim's `current_block`): the
/// count-th enclosing pair. Inner leaves out the brackets, and — when
/// the closing bracket has only indent before it — that indent and the
/// line break, which is what makes `di{` take whole lines.
fn block(
    buf: &Buffer,
    cursor: usize,
    selection: Option<(usize, usize)>,
    count: usize,
    around: bool,
    open: char,
    close: char,
) -> Option<Extent> {
    let (open, close) = (open as u8, close as u8);
    let extending = selection.filter(|(anchor, head)| anchor != head);
    let (low, high) = extending.map_or((cursor, cursor), |(a, h)| (a.min(h), a.max(h)));
    let mut from = Walk::new(buf, low);
    if extending.is_none() {
        if open == b'{' {
            while from.in_indent(1) {
                if from.inc() != Step::Char {
                    break;
                }
            }
        }
        if from.ch() == Some(char::from(open)) {
            from.inc();
        }
    }
    let mut at = from.pos;
    let mut start = None;
    // Not inside a block: the next one that opens after the cursor
    // (Vim searches forward for the opening bracket, skipping closers).
    let enclosed = blocks::unmatched(buf, at, open, close, false, 1).is_some();
    for _ in 0..count {
        let found = if enclosed {
            blocks::unmatched(buf, at, open, close, false, 1)
        } else {
            blocks::unmatched(buf, at, close, open, true, 1)
        };
        // fewer blocks than the count: Vim selects nothing
        let found = found?;
        at = found;
        start = Some(found);
    }
    let mut start = start?;
    let mut end = blocks::unmatched(buf, start, open, close, true, 1)?;
    if around {
        return Some(Extent {
            start,
            end,
            kind: Kind::Inclusive,
        });
    }
    loop {
        let mut first = Walk::new(buf, start);
        first.incl();
        let mut last = Walk::new(buf, end);
        let mut line_start = last.col() == 0;
        last.decl();
        while last.in_indent(1) {
            line_start = true;
            if last.decl() != Step::Char {
                break;
            }
        }
        // A selection that already covers this block's inside grows to
        // the next block out.
        if selection.is_some() && first.pos >= low && last.pos <= high && first.pos != last.pos {
            let mut outer = Walk::new(buf, low);
            outer.decl();
            start = blocks::unmatched(buf, outer.pos, open, close, false, 1)?;
            end = blocks::unmatched(buf, start, open, close, true, 1)?;
            continue;
        }
        if selection.is_some() {
            if first.pos > last.pos {
                // nothing between the brackets: Visual selects nothing
                return None;
            }
            if line_start && !last.at_end() {
                last.inc();
            }
            return Some(Extent {
                start: first.pos,
                end: last.pos,
                kind: Kind::Inclusive,
            });
        }
        let (end, kind) = if line_start {
            last.incl();
            (last.pos, Kind::Exclusive)
        } else if first.pos <= last.pos {
            (last.pos, Kind::Inclusive)
        } else {
            // nothing between the brackets
            (first.pos, Kind::Exclusive)
        };
        return Some(Extent {
            start: first.pos,
            end,
            kind,
        });
    }
}

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
