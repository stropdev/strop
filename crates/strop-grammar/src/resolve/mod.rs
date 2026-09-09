//! Commands → byte ranges. THE function: execute and preview both
//! consume `resolve`. Search runs through the compiled query engine
//! (`crate::query`) — bounded, typed errors, explicit match ranges.

use std::sync::atomic::{AtomicBool, Ordering};
use strop_core::{Buffer, Range};

use crate::types::*;

mod find;
mod motions;
pub use find::find_character;
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
    resolve_controlled(buf, cursor, cmd, None)
}

fn resolve_controlled(
    buf: &Buffer,
    cursor: usize,
    cmd: &Command,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Resolved>, crate::query::QueryError> {
    check_cancel(cancel)?;
    let count = cmd.count.unwrap_or(1);
    let mut motion_target = None;
    let (range, inclusive, mut spec) = match &cmd.target {
        Target::Linewise => {
            let line = buf.line_of(cursor);
            let start = buf.line_start(line);
            let end_line = line.saturating_add(count).min(buf.len_lines());
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
                Object::Word | Object::BigWord => {
                    let big = matches!(obj, Object::BigWord);
                    let (s, e) = none!(word_object(buf, cursor, big, *inner));
                    let class = if big { "WORD" } else { "word" };
                    (
                        s,
                        e,
                        if *inner {
                            format!("inner {class}")
                        } else {
                            format!("around {class}")
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
            let r = none!(resolve_controlled(buf, cursor, &sub, cancel)?);
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
                    check_cancel(cancel)?;
                    let previous = pos;
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
                    if pos == previous {
                        break;
                    }
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
                        (line, line.saturating_add(count))
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
                        line.saturating_add(count).min(buf.len_lines() - 1)
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
                    check_cancel(cancel)?;
                    let next = word_end(buf, pos, big);
                    if next == pos {
                        break;
                    }
                    pos = next;
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
                    check_cancel(cancel)?;
                    let next = word_forward(buf, pos, big);
                    if next == pos {
                        break;
                    }
                    pos = next;
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
                    check_cancel(cancel)?;
                    let next = word_backward(buf, pos, big);
                    if next == pos {
                        break;
                    }
                    pos = next;
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
                    check_cancel(cancel)?;
                    let next = word_end(buf, pos, big);
                    if next == pos {
                        break;
                    }
                    pos = next;
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
                    check_cancel(cancel)?;
                    let next = word_end_backward(buf, pos, big);
                    if next == pos {
                        break;
                    }
                    pos = next;
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
                    check_cancel(cancel)?;
                    let previous = line;
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
                    if line == previous {
                        break;
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
                let target = none!(find_character(buf, cursor.into(), *ch, *backward, count)).get();
                // till lands one before/after the char
                let land = if *till {
                    if *backward {
                        buf.ceil_boundary(target + 1)
                    } else {
                        buf.clamp_boundary(target.saturating_sub(1))
                    }
                } else {
                    target
                };
                motion_target = Some(land);
                let inclusive = !till;
                let (s, e) = if land >= cursor {
                    (cursor, buf.ceil_boundary(land + 1))
                } else {
                    (land, buf.ceil_boundary(cursor + 1))
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
                    check_cancel(cancel)?;
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
    check_cancel(cancel)?;
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
        Target::Motion(Motion::MatchPair) => {
            // bare %: cursor lands on the mate (the far end)
            if r.range.end - 1 == _cursor {
                r.range.start.get()
            } else {
                r.range.end.get() - 1
            }
        }
        Target::Motion(Motion::Search(_) | Motion::SearchBackward(_) | Motion::FindChar { .. }) => {
            unreachable!("resolved motion carries its exact target; returned above")
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActionPlan {
    /// Sorted by start, deduped, non-overlapping (overlaps keep the
    /// lower range — the first cursor to claim a region owns it).
    pub targets: Vec<PlannedTarget>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
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
    let resolved = resolve_many(buf, cursors, cmd)?;
    Ok(ActionPlan::from_resolved(cursors, &resolved))
}

impl ActionPlan {
    /// Preserve the same sorted, non-overlapping plan without running the
    /// resolver twice when a worker also needs each cursor's motion result.
    pub fn from_resolved(cursors: &[usize], resolved: &[Option<Resolved>]) -> Option<Self> {
        debug_assert_eq!(cursors.len(), resolved.len());
        let mut targets: Vec<_> = cursors
            .iter()
            .zip(resolved)
            .filter_map(|(&cursor, resolved)| {
                resolved.as_ref().map(|resolved| PlannedTarget {
                    cursor,
                    range: resolved.range,
                })
            })
            .collect();
        if targets.is_empty() {
            return None;
        }
        targets.sort_by_key(|target| target.range.start);
        targets.dedup_by_key(|target| (target.range.start, target.range.end));
        let mut kept: Vec<PlannedTarget> = Vec::with_capacity(targets.len());
        for target in targets {
            if kept
                .last()
                .is_none_or(|previous| target.range.start >= previous.range.end)
            {
                kept.push(target);
            }
        }
        Some(Self { targets: kept })
    }
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

/// The same pure resolver under a native work owner's stop flag.
pub fn resolve_many_cancellable(
    buf: &Buffer,
    cursors: &[usize],
    cmd: &Command,
    cancel: &AtomicBool,
) -> Result<Vec<Option<Resolved>>, crate::query::QueryError> {
    cursors
        .iter()
        .map(|&cursor| resolve_controlled(buf, cursor, cmd, Some(cancel)))
        .collect()
}

fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), crate::query::QueryError> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(crate::query::QueryError::Cancelled)
    } else {
        Ok(())
    }
}
