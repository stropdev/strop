//! Commands → byte ranges. THE function: execute and preview both
//! consume `resolve`. Search runs through the compiled query engine
//! (`crate::query`) — bounded, typed errors, explicit match ranges.

use strop_core::{Buffer, Range};

use crate::types::*;

mod motions;
mod objects;
pub(crate) mod search;

use motions::*;
use objects::*;

use crate::query::{search_backward, search_forward};
pub use objects::match_pair;

/// Unwrap an Option helper's None into the command resolving to
/// nothing (Ok(None)) — the body keeps its `?` shape where helpers
/// return Option, while query errors propagate as Err.
macro_rules! none {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return Ok(None),
        }
    };
}

/// Resolve a complete command against the buffer at `cursor`.
/// This is THE function: execute and preview both consume it.
/// Err carries a query failure (the engine's step budget); Ok(None)
/// means the command finds nothing here.
pub fn resolve(
    buf: &Buffer,
    cursor: usize,
    cmd: &Command,
) -> Result<Option<Resolved>, crate::query::QueryError> {
    let count = cmd.count.unwrap_or(1);
    let mut motion_target = None;
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
                    let (s, e) = none!(inner_word(buf, cursor));
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
                    let (o, c) = none!(quote_pair(buf, cursor, *q));
                    let spec = format!("{} {}", if *inner { "inner" } else { "around" }, *q);
                    if *inner {
                        (o + 1, c, spec)
                    } else {
                        (o, c + 1, spec)
                    }
                }
                Object::Bracket { open, close } => {
                    let (o, c) = none!(bracket_pair(buf, cursor, *open, *close));
                    let spec = format!("{} {}", if *inner { "inner" } else { "around" }, *open);
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
            let (open, close) = none!(surround_pair(*ch));
            let (o, c) = if open == close {
                none!(quote_pair(buf, cursor, open))
            } else {
                none!(bracket_pair(buf, cursor, open, close))
            };
            (Range::charwise(o, c + 1), true, format!("surround {}", *ch))
        }
        Target::SurroundAdd { ch, inner } => {
            // resolve the inner motion as if yanked, then wrap its range
            let sub = Command {
                op: Some(Op::Yank),
                register: None,
                count: Some(1),
                target: (**inner).clone(),
                keys: String::new(),
            };
            let r = none!(resolve(buf, cursor, &sub)?);
            (
                r.range,
                r.range.inclusive(),
                format!("surround with {}", *ch),
            )
        }
        Target::Motion(m) => match m {
            Motion::Left | Motion::Right => {
                // h/l never leave the line (vim) and step CHAR
                // boundaries (0017): a byte step from a multibyte lead
                // lands on a continuation byte and clamps straight back
                let line = buf.line_of(cursor);
                let lo = buf.line_start(line);
                let hi = buf.line_end(line).saturating_sub(1).max(lo);
                let is_cont = |p: usize| p < buf.len_bytes() && buf.byte(p) & 0xC0 == 0x80;
                let mut pos = cursor;
                for _ in 0..count {
                    pos = if *m == Motion::Left {
                        let mut p = pos.saturating_sub(1).max(lo);
                        while p > lo && is_cont(p) {
                            p -= 1;
                        }
                        p
                    } else {
                        let mut p = (pos + 1).min(hi);
                        while p < hi && is_cont(p) {
                            p += 1;
                        }
                        p
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
            Motion::WordEndBackward | Motion::BigWordEndBackward => {
                let big = matches!(m, Motion::BigWordEndBackward);
                let mut pos = cursor;
                for _ in 0..count {
                    pos = word_end_backward(buf, pos, big);
                }
                (
                    Range::charwise(pos.min(cursor), pos.max(cursor) + 1),
                    true,
                    if big {
                        "WORD end backward".to_string()
                    } else {
                        "word end backward".to_string()
                    },
                )
            }
            Motion::ParagraphBackward | Motion::ParagraphForward => {
                // vim: the first blank line strictly past the cursor, or
                // the file edge when no blank remains
                let forward = matches!(m, Motion::ParagraphForward);
                let n = buf.len_lines();
                let mut line = buf.line_of(cursor);
                for _ in 0..count {
                    if forward {
                        loop {
                            line += 1;
                            if line >= n {
                                line = n.saturating_sub(1);
                                break;
                            }
                            if line_blank(buf, line) {
                                break;
                            }
                        }
                    } else {
                        loop {
                            if line == 0 {
                                break;
                            }
                            line -= 1;
                            if line_blank(buf, line) {
                                break;
                            }
                        }
                    }
                }
                let target = buf.line_start(line);
                (
                    Range::charwise(cursor.min(target), cursor.max(target)),
                    false,
                    if forward {
                        "paragraph forward".to_string()
                    } else {
                        "paragraph backward".to_string()
                    },
                )
            }
            Motion::MatchPair => {
                let target = none!(match_pair(buf, cursor));
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
            Motion::FirstNonBlank => {
                // ^ — past the line's leading blanks (stays on the line)
                let line = buf.line_of(cursor);
                let (lo, hi) = (buf.line_start(line), buf.line_end(line));
                let mut s = lo;
                while s < hi && buf.byte(s).is_ascii_whitespace() {
                    s += 1;
                }
                (
                    Range::charwise(s.min(cursor), s.max(cursor)),
                    false,
                    "first non-blank".to_string(),
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
            Motion::Column => {
                // vim `|`: count names the 1-based column (bare `|` = 1)
                let line = buf.line_of(cursor);
                let start = buf.line_start(line);
                let pos = start + (count - 1).min(buf.line_end(line) - start);
                (
                    Range::charwise(cursor.min(pos), cursor.max(pos)),
                    false,
                    "column".to_string(),
                )
            }
            Motion::FirstLine | Motion::LastLine => {
                let target = if *m == Motion::FirstLine {
                    count - 1
                } else {
                    // G with an explicit count is that line (vim); bare G
                    // is the last content line — count is typed now
                    // (0016), no more digit-sniffing in keys
                    match cmd.count {
                        Some(n) => n.saturating_sub(1),
                        None => buf.last_content_line(),
                    }
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
                // char-honest: f é must find é, never a continuation byte
                let line_text = buf.line_text(line);
                let mut found = None;
                let mut hits = 0;
                if !backward {
                    for (off, c) in line_text.char_indices() {
                        let i = lo + off;
                        if i <= cursor {
                            continue;
                        }
                        if i >= hi {
                            break;
                        }
                        if c == *ch {
                            hits += 1;
                            if hits == count {
                                found = Some(i);
                                break;
                            }
                        }
                    }
                } else {
                    for (off, c) in line_text.char_indices().rev() {
                        let i = lo + off;
                        if i >= cursor.min(hi) {
                            continue;
                        }
                        if c == *ch {
                            hits += 1;
                            if hits == count {
                                found = Some(i);
                                break;
                            }
                        }
                    }
                }
                let target = none!(found);
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
                    format!("{verb} '{}'", *ch),
                )
            }
            Motion::Search(q) | Motion::SearchBackward(q) => {
                let backward = matches!(m, Motion::SearchBackward(_));
                let mut target = cursor;
                for _ in 0..count {
                    // wrap at the file edge like `n`/`N`; query errors
                    // (step budget) propagate as Err
                    let hit = if backward {
                        match search_backward(buf, target, q)? {
                            Some(h) => Some(h),
                            None => search_backward(buf, buf.len_bytes(), q)?,
                        }
                    } else {
                        match search_forward(buf, buf.ceil_boundary(target.saturating_add(1)), q)? {
                            Some(h) => Some(h),
                            None => search_forward(buf, 0, q)?,
                        }
                    };
                    // the landing is the match start — an explicit
                    // range, never pattern-length math
                    target = none!(hit).start.get();
                }
                motion_target = Some(target);
                let range = if target < cursor {
                    Range::charwise(buf.ceil_boundary(target + 1), cursor)
                } else {
                    Range::charwise(cursor, target)
                };
                (
                    range,
                    false,
                    format!("search {}{}", if backward { '?' } else { '/' }, q.source()),
                )
            }
        },
    };
    if range.is_empty() && cmd.op.is_some() {
        return Ok(None);
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
    Ok(Some(Resolved {
        range: range.with_inclusive(inclusive),
        motion_target,
        spec,
    }))
}

/// Where the cursor lands after a resolved motion command.
pub fn cursor_after(buf: &Buffer, _cursor: usize, cmd: &Command, r: &Resolved) -> usize {
    if let Some(target) = r.motion_target {
        return target;
    }
    match &cmd.target {
        Target::Motion(Motion::Down | Motion::Up) => r.range.start.get(),
        Target::Motion(Motion::WordBackward | Motion::LineStart) => r.range.start.get(),
        Target::Motion(Motion::FirstNonBlank | Motion::Column) => {
            // ^ lands on the non-blank — whichever side of the cursor
            // that is (the range is (min, max) of cursor and target)
            if _cursor <= r.range.start.get() {
                r.range.end.get()
            } else {
                r.range.start.get()
            }
        }
        Target::Motion(Motion::WordForward | Motion::BigWordForward) => {
            r.range.end.get().min(buf.len_bytes().saturating_sub(1))
        }
        Target::Motion(Motion::WordEnd | Motion::BigWordEnd | Motion::LineEnd) => {
            r.range.end.get().saturating_sub(1)
        }
        Target::Motion(Motion::FirstLine | Motion::LastLine) => {
            let line = if matches!(cmd.target, Target::Motion(Motion::FirstLine)) {
                cmd.count.unwrap_or(1) - 1
            } else {
                match cmd.count {
                    Some(n) => n.saturating_sub(1),
                    None => buf.last_content_line(),
                }
            };
            buf.line_start(line.min(buf.len_lines().saturating_sub(1)))
        }
        Target::Motion(Motion::FindChar { backward, .. }) => {
            if *backward {
                r.range.start.get()
            } else {
                r.range.end.get().saturating_sub(1)
            }
        }
        Target::Motion(Motion::MatchPair) => {
            // bare %: cursor lands on the mate (the far end)
            if r.range.end - 1 == _cursor {
                r.range.start.get()
            } else {
                r.range.end.get() - 1
            }
        }
        Target::Motion(Motion::Search(_) | Motion::SearchBackward(_)) => {
            unreachable!("search carries motion_target; returned above")
        }
        Target::Motion(Motion::Right) => r.range.end.get(),
        Target::Motion(Motion::ParagraphForward) => r.range.end.get(),
        Target::Motion(
            Motion::ParagraphBackward | Motion::WordEndBackward | Motion::BigWordEndBackward,
        ) => r.range.start.get(),
        _ => r.range.start.get(),
    }
}

/// A command resolved against a cursor SET (0014 wave 3): preview
/// renders exactly these ranges, execute applies exactly these ranges.
/// The preview cannot lie because both consume this object.
#[derive(Debug, Clone)]
pub struct ActionPlan {
    /// Sorted by start, deduped, non-overlapping (overlaps keep the
    /// lower range — the first cursor to claim a region owns it).
    pub targets: Vec<PlannedTarget>,
}

#[derive(Debug, Clone, Copy)]
pub struct PlannedTarget {
    /// The cursor this range was resolved from.
    pub cursor: usize,
    pub range: Range,
}

/// Resolve one command over every cursor; Ok(None) when nothing
/// resolves; Err carries a query failure from any cursor's attempt.
pub fn plan(
    buf: &Buffer,
    cursors: &[usize],
    cmd: &Command,
) -> Result<Option<ActionPlan>, crate::query::QueryError> {
    let mut targets: Vec<PlannedTarget> = Vec::with_capacity(cursors.len());
    for &c in cursors {
        if let Some(r) = resolve(buf, c, cmd)? {
            targets.push(PlannedTarget {
                cursor: c,
                range: r.range,
            });
        }
    }
    if targets.is_empty() {
        return Ok(None);
    }
    targets.sort_by_key(|t| t.range.start);
    targets.dedup_by_key(|t| (t.range.start, t.range.end));
    let mut kept: Vec<PlannedTarget> = Vec::with_capacity(targets.len());
    for t in targets {
        if kept.last().is_none_or(|k| t.range.start >= k.range.end) {
            kept.push(t);
        }
    }
    Ok(Some(ActionPlan { targets: kept }))
}

/// The same command resolved at every cursor, independently — no
/// overlap dedup (that is plan()'s shape for edits). Cursor movement
/// and incsearch consume this: every cursor seeks from its own
/// position with the exact command semantics.
pub fn resolve_many(
    buf: &Buffer,
    cursors: &[usize],
    cmd: &Command,
) -> Result<Vec<Option<Resolved>>, crate::query::QueryError> {
    cursors.iter().map(|&c| resolve(buf, c, cmd)).collect()
}
