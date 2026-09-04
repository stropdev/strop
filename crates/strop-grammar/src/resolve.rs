//! Commands → byte ranges. THE function: execute and preview both
//! consume `resolve`. Plus the plain-substring search helpers (prototype;
//! 0001 §2.5's transpiled regex lands with the real search layer).

use strop_core::{Buffer, Range};

use crate::types::*;

pub(crate) fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Word-class of a byte: WORD motions (big) only split on whitespace.
fn class_of(b: u8, big: bool) -> u8 {
    if big {
        u8::from(!b.is_ascii_whitespace())
    } else {
        u8::from(is_word(b))
    }
}

fn word_forward(buf: &Buffer, mut pos: usize, big: bool) -> usize {
    let n = buf.len_bytes();
    if pos >= n {
        return n;
    }
    let start_class = class_of(buf.byte(pos), big);
    while pos < n
        && class_of(buf.byte(pos), big) == start_class
        && !buf.byte(pos).is_ascii_whitespace()
    {
        pos += 1;
    }
    while pos < n && (buf.byte(pos).is_ascii_whitespace()) {
        pos += 1;
    }
    pos
}

fn word_backward(buf: &Buffer, mut pos: usize, big: bool) -> usize {
    if pos == 0 {
        return 0;
    }
    pos -= 1;
    while pos > 0 && buf.byte(pos).is_ascii_whitespace() {
        pos -= 1;
    }
    let class = class_of(buf.byte(pos), big);
    while pos > 0
        && !buf.byte(pos - 1).is_ascii_whitespace()
        && class_of(buf.byte(pos - 1), big) == class
    {
        pos -= 1;
    }
    pos
}

fn word_end(buf: &Buffer, mut pos: usize, big: bool) -> usize {
    let n = buf.len_bytes();
    if pos + 1 >= n {
        return n.saturating_sub(1);
    }
    pos += 1;
    while pos < n && buf.byte(pos).is_ascii_whitespace() {
        pos += 1;
    }
    let class = class_of(buf.byte(pos), big);
    while pos + 1 < n
        && !buf.byte(pos + 1).is_ascii_whitespace()
        && class_of(buf.byte(pos + 1), big) == class
    {
        pos += 1;
    }
    pos
}

/// cw's target (vim): end of the word UNDER the cursor — unlike `e`,
/// never jumps to the next word when the cursor is already on a word's
/// last char. On whitespace, behaves like `e`.
fn change_word_end(buf: &Buffer, pos: usize, big: bool) -> usize {
    let n = buf.len_bytes();
    if pos >= n || buf.byte(pos).is_ascii_whitespace() {
        return word_end(buf, pos, big);
    }
    let class = class_of(buf.byte(pos), big);
    let mut end = pos;
    while end + 1 < n
        && !buf.byte(end + 1).is_ascii_whitespace()
        && class_of(buf.byte(end + 1), big) == class
    {
        end += 1;
    }
    end
}

/// % — matching pair. On a bracket: its mate. Else: first bracket on the
/// line right of cursor, then its mate (vim semantics).
pub fn match_pair(buf: &Buffer, pos: usize) -> Option<usize> {
    const PAIRS: &[(u8, u8)] = &[(b'(', b')'), (b'[', b']'), (b'{', b'}'), (b'<', b'>')];

    let on = buf
        .byte_at(pos)
        .and_then(|b| PAIRS.iter().find(|(o, c)| *o == b || *c == b));
    let (open, close, from) = match on {
        Some(&(o, c)) => (o, c, pos),
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
                    break (o, c, i);
                }
                i += 1;
            }
        }
    };
    let (o, c) = bracket_pair(buf, from, open, close)?;
    let b = buf.byte_at(from)?;
    if b == open {
        Some(c)
    } else {
        Some(o)
    }
}

/// Find the enclosing bracket pair around `pos` (nesting-aware scan).
/// Cursor on either delimiter counts as inside the pair (vim semantics):
/// the backward scan starts just inside a close, the forward scan just
/// past the open.
fn bracket_pair(buf: &Buffer, pos: usize, open: u8, close: u8) -> Option<(usize, usize)> {
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
fn quote_pair(buf: &Buffer, pos: usize, q: u8) -> Option<(usize, usize)> {
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

fn inner_word(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
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
fn surround_pair(ch: u8) -> Option<(u8, u8)> {
    Some(match ch {
        b'b' | b'(' | b')' => (b'(', b')'),
        b'B' | b'{' | b'}' => (b'{', b'}'),
        b'r' | b'[' | b']' => (b'[', b']'),
        b'a' | b'<' | b'>' => (b'<', b'>'),
        q @ (b'"' | b'\'' | b'`') => (q, q),
        _ => return None,
    })
}

/// Search backward for `pat` before `from` (prototype: plain substring).
pub fn search_backward(buf: &Buffer, from: usize, pat: &str) -> Option<usize> {
    if from == 0 {
        return None;
    }
    let text = buf.rope.byte_slice(..from).to_string();
    text.rfind(pat)
}

pub fn search_forward(buf: &Buffer, from: usize, pat: &str) -> Option<usize> {
    let text = buf.rope.byte_slice(from.min(buf.len_bytes())..).to_string();
    text.find(pat).map(|i| from + i)
}

/// All matches of `pat` (incsearch highlight).
pub fn search_all(buf: &Buffer, pat: &str) -> Vec<usize> {
    if pat.is_empty() {
        return vec![];
    }
    // prototype: materializes; §2.5 promises rope-chunk search before M0 ships
    let text = buf.rope.to_string();
    text.match_indices(pat).map(|(i, _)| i).collect()
}

/// Resolve a complete command against the buffer at `cursor`.
/// This is THE function: execute and preview both consume it.
pub fn resolve(buf: &Buffer, cursor: usize, cmd: &Command) -> Option<Resolved> {
    let count = cmd.count.max(1);
    let (range, inclusive, mut spec) = match &cmd.target {
        Target::Linewise => {
            let line = buf.line_of(cursor);
            let start = buf.line_start(line);
            let end_line = (line + count).min(buf.len_lines());
            let end = if end_line >= buf.len_lines() {
                buf.len_bytes()
            } else {
                buf.line_start(end_line)
            };
            (
                Range::linewise(start, end),
                true,
                format!("{count} line{}", if count > 1 { "s" } else { "" }),
            )
        }
        Target::Object { inner, obj } => {
            let (s, e, spec) = match obj {
                Object::Word => {
                    let (s, e) = inner_word(buf, cursor)?;
                    (
                        s,
                        e,
                        if *inner {
                            "inner word".to_string()
                        } else {
                            "around word".to_string()
                        },
                    )
                }
                Object::Quote(q) => {
                    let (o, c) = quote_pair(buf, cursor, *q)?;
                    let spec =
                        format!("{} {}", if *inner { "inner" } else { "around" }, *q as char);
                    if *inner {
                        (o + 1, c, spec)
                    } else {
                        (o, c + 1, spec)
                    }
                }
                Object::Bracket { open, close } => {
                    let (o, c) = bracket_pair(buf, cursor, *open, *close)?;
                    let spec = format!(
                        "{} {}",
                        if *inner { "inner" } else { "around" },
                        *open as char
                    );
                    if *inner {
                        (o + 1, c, spec)
                    } else {
                        (o, c + 1, spec)
                    }
                }
            };
            (Range::charwise(s, e), true, spec)
        }
        Target::SurroundDelete(ch) | Target::SurroundChange { from: ch, .. } => {
            let (open, close) = surround_pair(*ch)?;
            let (o, c) = if open == close {
                quote_pair(buf, cursor, open)?
            } else {
                bracket_pair(buf, cursor, open, close)?
            };
            (
                Range::charwise(o, c + 1),
                true,
                format!("surround {}", *ch as char),
            )
        }
        Target::SurroundAdd { ch, inner } => {
            // resolve the inner motion as if yanked, then wrap its range
            let sub = Command {
                op: Some(Op::Yank),
                register: None,
                count: 1,
                target: (**inner).clone(),
                keys: String::new(),
            };
            let r = resolve(buf, cursor, &sub)?;
            (
                r.range,
                r.inclusive,
                format!("surround with {}", *ch as char),
            )
        }
        Target::Motion(m) => match m {
            Motion::Left | Motion::Right => {
                // h/l never leave the line (vim)
                let line = buf.line_of(cursor);
                let lo = buf.line_start(line);
                let hi = buf.line_end(line).saturating_sub(1).max(lo);
                let mut pos = cursor;
                for _ in 0..count {
                    pos = if *m == Motion::Left {
                        pos.saturating_sub(1).max(lo)
                    } else {
                        (pos + 1).min(hi)
                    };
                }
                let (s, e) = if pos <= cursor {
                    (pos, cursor)
                } else {
                    (cursor, pos)
                };
                let name = if *m == Motion::Left { "left" } else { "right" };
                (Range::charwise(s, e), false, name.to_string())
            }
            Motion::Down | Motion::Up => {
                // operator + j/k is linewise in vim
                if cmd.op.is_some() {
                    let line = buf.line_of(cursor);
                    let (a, b) = if *m == Motion::Down {
                        (line, line + count)
                    } else {
                        (line.saturating_sub(count), line)
                    };
                    let start = buf.line_start(a);
                    let end = if b + 1 >= buf.len_lines() {
                        buf.len_bytes()
                    } else {
                        buf.line_start(b + 1)
                    };
                    (
                        Range::linewise(start, end),
                        true,
                        format!("{} lines", b - a + 1),
                    )
                } else {
                    let line = buf.line_of(cursor);
                    let target = if *m == Motion::Down {
                        (line + count).min(buf.len_lines() - 1)
                    } else {
                        line.saturating_sub(count)
                    };
                    let col = buf
                        .col_of(cursor)
                        .min(buf.line_end(target) - buf.line_start(target));
                    let pos = buf.line_start(target) + col;
                    (
                        Range::charwise(pos, pos),
                        false,
                        if *m == Motion::Down {
                            "down".into()
                        } else {
                            "up".into()
                        },
                    )
                }
            }
            Motion::WordForward | Motion::BigWordForward if matches!(cmd.op, Some(Op::Change)) => {
                // vim: cw/cW behave like ce/cE — the change never eats
                // the whitespace after the word
                let big = matches!(m, Motion::BigWordForward);
                let mut pos = change_word_end(buf, cursor, big);
                for _ in 1..count {
                    pos = word_end(buf, pos, big);
                }
                (
                    Range::charwise(cursor.min(pos), pos.max(cursor) + 1),
                    true,
                    if big {
                        "WORD forward (change=end)".to_string()
                    } else {
                        "word forward (change=end)".to_string()
                    },
                )
            }
            Motion::WordForward | Motion::BigWordForward => {
                let big = matches!(m, Motion::BigWordForward);
                let mut pos = cursor;
                for _ in 0..count {
                    pos = word_forward(buf, pos, big);
                }
                // exclusive: [cursor, target)
                (
                    Range::charwise(cursor.min(pos), pos.max(cursor)),
                    false,
                    if big {
                        "WORD forward".to_string()
                    } else {
                        "word forward".to_string()
                    },
                )
            }
            Motion::WordBackward | Motion::BigWordBackward => {
                let big = matches!(m, Motion::BigWordBackward);
                let mut pos = cursor;
                for _ in 0..count {
                    pos = word_backward(buf, pos, big);
                }
                (
                    Range::charwise(pos, cursor),
                    false,
                    if big {
                        "WORD backward".to_string()
                    } else {
                        "word backward".to_string()
                    },
                )
            }
            Motion::WordEnd | Motion::BigWordEnd => {
                let big = matches!(m, Motion::BigWordEnd);
                let mut pos = cursor;
                for _ in 0..count {
                    pos = word_end(buf, pos, big);
                }
                (
                    Range::charwise(cursor.min(pos), pos.max(cursor) + 1),
                    true,
                    if big {
                        "WORD end".to_string()
                    } else {
                        "word end".to_string()
                    },
                )
            }
            Motion::MatchPair => {
                let target = match_pair(buf, cursor)?;
                let (s, e) = if target >= cursor {
                    (cursor, target + 1)
                } else {
                    (target, cursor + 1)
                };
                (Range::charwise(s, e), true, "match pair %".to_string())
            }
            Motion::LineStart => {
                let s = buf.line_start(buf.line_of(cursor));
                (
                    Range::charwise(s.min(cursor), s.max(cursor)),
                    false,
                    "line start".to_string(),
                )
            }
            Motion::LineEnd => {
                let e = buf.line_end(buf.line_of(cursor));
                let e = e.saturating_sub(1).max(buf.line_start(buf.line_of(cursor)));
                (
                    Range::charwise(cursor.min(e), cursor.max(e) + 1),
                    true,
                    "line end".to_string(),
                )
            }
            Motion::FirstLine | Motion::LastLine => {
                let target = if *m == Motion::FirstLine {
                    count - 1
                } else {
                    buf.len_lines() - 1
                };
                let target = target.min(buf.len_lines() - 1);
                let (a, b) = if buf.line_of(cursor) <= target {
                    (buf.line_of(cursor), target)
                } else {
                    (target, buf.line_of(cursor))
                };
                let start = buf.line_start(a);
                let end = if b + 1 >= buf.len_lines() {
                    buf.len_bytes()
                } else {
                    buf.line_start(b + 1)
                };
                let name = if *m == Motion::FirstLine { "gg" } else { "G" };
                (Range::linewise(start, end), true, name.to_string())
            }
            Motion::FindChar { ch, till, backward } => {
                let line = buf.line_of(cursor);
                let (lo, hi) = (buf.line_start(line), buf.line_end(line));
                let mut found = None;
                let mut hits = 0;
                if !backward {
                    let mut i = cursor + 1;
                    while i < hi {
                        if buf.byte(i) == *ch {
                            hits += 1;
                            if hits == count {
                                found = Some(i);
                                break;
                            }
                        }
                        i += 1;
                    }
                } else {
                    let mut i = cursor.min(hi);
                    while i > lo {
                        i -= 1;
                        if buf.byte(i) == *ch {
                            hits += 1;
                            if hits == count {
                                found = Some(i);
                                break;
                            }
                        }
                    }
                }
                let target = found?;
                // till lands one before/after the char
                let land = if *till {
                    if *backward {
                        target + 1
                    } else {
                        target.saturating_sub(1).max(cursor.min(target))
                    }
                } else {
                    target
                };
                let inclusive = !till;
                let (s, e) = if land >= cursor {
                    (cursor, land + 1)
                } else {
                    (land, cursor + 1)
                };
                let verb = if *till { "till" } else { "find" };
                (
                    Range::charwise(s, e),
                    inclusive,
                    format!("{verb} '{}'", *ch as char),
                )
            }
            Motion::Search(pat) => {
                let target = search_forward(buf, cursor + 1, pat)?;
                // exclusive: up to but not including the match
                (
                    Range::charwise(cursor, target),
                    false,
                    format!("search /{pat}"),
                )
            }
            Motion::SearchBackward(pat) => {
                let target = search_backward(buf, cursor, pat)?;
                // exclusive backward: covers (match, cursor) — vim d?pat
                // is exclusive of the target's first char
                (
                    Range::charwise(target + pat.len().min(1), cursor),
                    false,
                    format!("search ?{pat}"),
                )
            }
        },
    };
    if range.is_empty() && cmd.op.is_some() {
        return None;
    }
    if let Some(op) = cmd.op {
        spec = format!(
            "{}, {}, {} bytes · {}",
            op.name(),
            spec,
            range.len(),
            if inclusive { "inclusive" } else { "exclusive" }
        );
    } else {
        spec = format!(
            "{spec} · {}",
            if inclusive { "inclusive" } else { "exclusive" }
        );
    }
    Some(Resolved {
        range,
        inclusive,
        spec,
    })
}

/// Where the cursor lands when `cmd` is a bare motion (no operator).
pub fn cursor_after(buf: &Buffer, _cursor: usize, cmd: &Command, r: &Resolved) -> usize {
    match &cmd.target {
        Target::Motion(Motion::Down | Motion::Up) => r.range.start,
        Target::Motion(Motion::WordBackward | Motion::LineStart) => r.range.start,
        Target::Motion(Motion::WordForward) => r.range.end.min(buf.len_bytes().saturating_sub(1)),
        Target::Motion(Motion::WordEnd | Motion::LineEnd) => r.range.end.saturating_sub(1),
        Target::Motion(Motion::FirstLine | Motion::LastLine) => {
            let line = if matches!(cmd.target, Target::Motion(Motion::FirstLine)) {
                cmd.count - 1
            } else {
                buf.last_content_line()
            };
            buf.line_start(line.min(buf.len_lines() - 1))
        }
        Target::Motion(Motion::FindChar { backward, .. }) => {
            if *backward {
                r.range.start
            } else {
                r.range.end.saturating_sub(1)
            }
        }
        Target::Motion(Motion::MatchPair) => {
            // bare %: cursor lands on the mate (the far end)
            if r.range.end - 1 == _cursor {
                r.range.start
            } else {
                r.range.end - 1
            }
        }
        Target::Motion(Motion::Search(_)) => r.range.end,
        Target::Motion(Motion::SearchBackward(pat)) => {
            // land on the match start: range is (target+1, cursor) exclusive
            r.range.start.saturating_sub(pat.len().min(1))
        }
        Target::Motion(Motion::Right) => r.range.end,
        _ => r.range.start,
    }
}
