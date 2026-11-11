//! Where each motion goes and how an operator reads it (exclusive,
//! inclusive, linewise) — one arm per motion, each following the Vim
//! command it implements (`nv_*` in normal.c).

use std::sync::atomic::AtomicBool;
use strop_core::Buffer;

use super::walk::Walk;
use super::{blocks, check_cancel, find_character, match_pair, words, Kind};
use crate::query::{search_backward, search_forward, QueryError};
use crate::types::{Motion, Op};

/// A motion's destination.
pub(crate) struct Moved {
    pub(crate) to: usize,
    pub(crate) kind: Kind,
}

fn exclusive(to: usize) -> Option<Moved> {
    Some(Moved {
        to,
        kind: Kind::Exclusive,
    })
}
fn inclusive(to: usize) -> Option<Moved> {
    Some(Moved {
        to,
        kind: Kind::Inclusive,
    })
}
fn linewise(to: usize) -> Option<Moved> {
    Some(Moved {
        to,
        kind: Kind::Linewise,
    })
}

/// The first non-blank of `line` (its end when it has none).
pub(crate) fn first_non_blank(buf: &Buffer, line: usize) -> usize {
    let (start, end) = (buf.line_start(line), buf.line_end(line));
    (start..end)
        .find(|&at| !matches!(buf.byte(at), b' ' | b'\t'))
        .unwrap_or(end)
}

/// Where a whole-line command (`dd`, `_`) leaves the cursor in `line`:
/// yank, delete and the shifts keep the column (Neovim's
/// `nostartofline`), every other operator goes to the first non-blank.
pub(crate) fn line_landing(buf: &Buffer, cursor: usize, line: usize, op: Option<Op>) -> usize {
    match op {
        Some(Op::Yank | Op::Delete | Op::Indent | Op::Dedent) => same_column(buf, cursor, line),
        _ => first_non_blank(buf, line),
    }
}

/// The same byte column on another line, clamped to its text.
fn same_column(buf: &Buffer, cursor: usize, line: usize) -> usize {
    let (start, end) = (buf.line_start(line), buf.line_end(line));
    let last = buf.clamp_boundary(end.saturating_sub(1)).max(start);
    buf.clamp_boundary((start + buf.col_of(cursor)).min(last))
}

/// `count` lines down or up. `None` when not even one line can be moved
/// (Vim's `cursor_down` / `cursor_up` FAIL); a larger count clamps.
fn line_step(buf: &Buffer, cursor: usize, down: bool, count: usize) -> Option<usize> {
    let line = buf.line_of(cursor).min(buf.last_content_line());
    if count == 0 {
        return Some(line);
    }
    if down {
        (line < buf.last_content_line())
            .then(|| line.saturating_add(count).min(buf.last_content_line()))
    } else {
        (line > 0).then(|| line.saturating_sub(count))
    }
}

/// Vim's `adjust_cursor`: under an operator, a motion that stopped on a
/// non-empty line's end steps back onto its last character and becomes
/// inclusive. With no operator the position stands: Normal clamps the
/// cursor itself, and Visual lets it rest on the line's end.
fn off_line_end(walk: &mut Walk, kind: Kind, pending: bool) -> Kind {
    if pending && walk.col() > 0 && walk.at_end() {
        walk.dec();
        return Kind::Inclusive;
    }
    kind
}

/// Resolve one motion from `cursor`. `Ok(None)` is Vim's failed motion:
/// nothing moves and a pending operator is cancelled.
pub(crate) fn moved(
    buf: &Buffer,
    cursor: usize,
    motion: &Motion,
    typed_count: Option<usize>,
    op: Option<Op>,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Moved>, QueryError> {
    let count = typed_count.unwrap_or(1);
    let pending = op.is_some();
    let line = buf.line_of(cursor).min(buf.last_content_line());
    let (line_start, line_end) = (buf.line_start(line), buf.line_end(line));
    let last_char = buf
        .clamp_boundary(line_end.saturating_sub(1))
        .max(line_start);
    Ok(match motion {
        Motion::Left => {
            let mut walk = Walk::new(buf, cursor);
            for _ in 0..count {
                if walk.col() == 0 {
                    break;
                }
                walk.dec();
            }
            exclusive(walk.pos)
        }
        Motion::Right => {
            // The last character is as far as `l` goes; an operator that
            // is stopped there takes that character too.
            let mut pos = cursor.min(last_char);
            let mut blocked = false;
            for _ in 0..count {
                if pos >= last_char {
                    blocked = true;
                    break;
                }
                pos = buf.ceil_boundary(pos + 1);
            }
            if pending && blocked && line_end > line_start {
                inclusive(pos)
            } else {
                exclusive(pos)
            }
        }
        Motion::Down | Motion::Up => {
            let Some(target) = line_step(buf, cursor, *motion == Motion::Down, count) else {
                return Ok(None);
            };
            linewise(same_column(buf, cursor, target))
        }
        Motion::ScreenDown { column } | Motion::ScreenUp { column } => {
            let down = matches!(motion, Motion::ScreenDown { .. });
            let Some(target) = line_step(buf, cursor, down, count) else {
                return Ok(None);
            };
            // unlike j/k, an operator is cancelled when the count runs out
            if pending && target.abs_diff(line) < count {
                return Ok(None);
            }
            let (start, end) = (buf.line_start(target), buf.line_end(target));
            let last = buf.clamp_boundary(end.saturating_sub(1)).max(start);
            let wanted = column.unwrap_or_else(|| buf.col_of(cursor));
            exclusive(buf.clamp_boundary(start.saturating_add(wanted).min(last)))
        }
        Motion::LineDown | Motion::LineUp => {
            let Some(target) = line_step(buf, cursor, *motion == Motion::LineDown, count) else {
                return Ok(None);
            };
            linewise(first_non_blank(buf, target))
        }
        Motion::Underscore => {
            let Some(target) = line_step(buf, cursor, true, count - 1) else {
                return Ok(None);
            };
            linewise(line_landing(buf, cursor, target, op))
        }
        Motion::WordForward | Motion::BigWordForward => {
            let big = matches!(motion, Motion::BigWordForward);
            let mut walk = Walk::new(buf, cursor);
            let mut kind = Kind::Exclusive;
            let on_text = !matches!(walk.ch(), None | Some(' ' | '\t'));
            if op == Some(Op::Change) && on_text {
                // Vim: `cw` on a word is `ce` — the blanks after it stay.
                words::end(&mut walk, count, big, true, false);
                kind = Kind::Inclusive;
            } else {
                words::forward(&mut walk, count, big, pending);
            }
            if walk.pos > cursor {
                kind = off_line_end(&mut walk, kind, pending);
            }
            Some(Moved { to: walk.pos, kind })
        }
        Motion::WordBackward | Motion::BigWordBackward => {
            let big = matches!(motion, Motion::BigWordBackward);
            let mut walk = Walk::new(buf, cursor);
            if !words::backward(&mut walk, count, big, false) && pending {
                return Ok(None);
            }
            exclusive(walk.pos)
        }
        Motion::WordEnd | Motion::BigWordEnd => {
            let big = matches!(motion, Motion::BigWordEnd);
            let mut walk = Walk::new(buf, cursor);
            words::end(&mut walk, count, big, false, false);
            let mut kind = Kind::Inclusive;
            if walk.pos > cursor {
                kind = off_line_end(&mut walk, kind, pending);
            }
            Some(Moved { to: walk.pos, kind })
        }
        Motion::WordEndBackward | Motion::BigWordEndBackward => {
            let big = matches!(motion, Motion::BigWordEndBackward);
            let mut walk = Walk::new(buf, cursor);
            if !words::end_backward(&mut walk, count, big, false) && pending {
                return Ok(None);
            }
            inclusive(walk.pos)
        }
        Motion::ParagraphBackward | Motion::ParagraphForward => {
            let forward = matches!(motion, Motion::ParagraphForward);
            match blocks::paragraph(buf, cursor, forward, count) {
                Some((to, true)) => inclusive(to),
                Some((to, false)) => exclusive(to),
                None => None,
            }
        }
        Motion::SentenceBackward | Motion::SentenceForward => {
            let forward = matches!(motion, Motion::SentenceForward);
            let Some(to) = blocks::sentence(buf, cursor, forward, count) else {
                return Ok(None);
            };
            let mut walk = Walk::new(buf, to);
            let kind = off_line_end(&mut walk, Kind::Exclusive, pending);
            Some(Moved { to: walk.pos, kind })
        }
        Motion::UnmatchedOpen(open) | Motion::UnmatchedClose(open) => {
            let forward = matches!(motion, Motion::UnmatchedClose(_));
            let close = if *open == '(' { b')' } else { b'}' };
            blocks::unmatched_upto(buf, cursor, *open as u8, close, forward, count)
                .and_then(exclusive)
        }
        Motion::MatchPair => match typed_count {
            // N%: that percentage of the way through the buffer.
            Some(percent) if percent > 100 => None,
            Some(percent) => {
                let lines = buf.last_content_line() + 1;
                let target = (percent * lines).div_ceil(100).max(1) - 1;
                linewise(same_column(buf, cursor, target))
            }
            None => match_pair(buf, cursor).and_then(inclusive),
        },
        Motion::LineStart => exclusive(line_start),
        Motion::FirstNonBlank => exclusive(first_non_blank(buf, line).min(last_char)),
        Motion::LineEnd | Motion::LastNonBlank => {
            let Some(target) = line_step(buf, cursor, true, count - 1) else {
                return Ok(None);
            };
            let (start, end) = (buf.line_start(target), buf.line_end(target));
            let mut to = buf.clamp_boundary(end.saturating_sub(1)).max(start);
            if *motion == Motion::LastNonBlank {
                while to > start && matches!(buf.byte(to), b' ' | b'\t') {
                    to = buf.clamp_boundary(to - 1);
                }
            }
            inclusive(to)
        }
        Motion::Column => {
            exclusive(buf.clamp_boundary(line_start + (count - 1).min(last_char - line_start)))
        }
        Motion::FirstLine | Motion::LastLine => {
            let target = match (motion, typed_count) {
                (_, Some(n)) => n.saturating_sub(1),
                (Motion::FirstLine, None) => 0,
                (_, None) => buf.last_content_line(),
            };
            linewise(same_column(
                buf,
                cursor,
                target.min(buf.last_content_line()),
            ))
        }
        Motion::FindChar { ch, till, backward } => {
            let Some(target) = find_character(buf, cursor.into(), *ch, *backward, count) else {
                return Ok(None);
            };
            let target = target.get();
            match (till, backward) {
                (false, false) => inclusive(target),
                (true, false) => inclusive(buf.clamp_boundary(target.saturating_sub(1))),
                (false, true) => exclusive(target),
                (true, true) => exclusive(buf.ceil_boundary(target + 1)),
            }
        }
        Motion::Search(query) | Motion::SearchBackward(query) => {
            let backward = matches!(motion, Motion::SearchBackward(_));
            let mut target = cursor;
            for _ in 0..count {
                check_cancel(cancel)?;
                // wrap at the file edge like `n`/`N`; query errors
                // (step budget) propagate as Err
                let hit = if backward {
                    match search_backward(buf, target, query)? {
                        Some(hit) => Some(hit),
                        None => search_backward(buf, buf.len_bytes(), query)?,
                    }
                } else {
                    match search_forward(buf, buf.ceil_boundary(target.saturating_add(1)), query)? {
                        Some(hit) => Some(hit),
                        None => search_forward(buf, 0, query)?,
                    }
                };
                let Some(hit) = hit else {
                    return Ok(None);
                };
                target = hit.start.get();
            }
            exclusive(target)
        }
        Motion::Mark { at, exact, .. } => match (at, exact) {
            (None, _) => None,
            (Some(at), true) => exclusive((*at).min(buf.len_bytes())),
            (Some(at), false) => linewise(first_non_blank(
                buf,
                buf.line_of((*at).min(buf.len_bytes()))
                    .min(buf.last_content_line()),
            )),
        },
        Motion::Window { line: target, .. } => target.and_then(|target| {
            linewise(same_column(
                buf,
                cursor,
                target.min(buf.last_content_line()),
            ))
        }),
        // The engine binds these to the find or search they repeat
        // before resolution; unbound, there is nothing to repeat.
        Motion::RepeatFind { .. } | Motion::RepeatSearch { .. } => None,
    })
}

/// The spec-footer name of a motion.
pub(crate) fn name(motion: &Motion, op: Option<Op>) -> String {
    let fixed = match motion {
        Motion::Left => "left",
        Motion::Right => "right",
        Motion::Down | Motion::ScreenDown { .. } => "down",
        Motion::Up | Motion::ScreenUp { .. } => "up",
        Motion::LineDown => "line down",
        Motion::LineUp => "line up",
        Motion::Underscore | Motion::FirstNonBlank => "first non-blank",
        Motion::WordForward if op == Some(Op::Change) => "word forward (change=end)",
        Motion::BigWordForward if op == Some(Op::Change) => "WORD forward (change=end)",
        Motion::WordForward => "word forward",
        Motion::BigWordForward => "WORD forward",
        Motion::WordBackward => "word backward",
        Motion::BigWordBackward => "WORD backward",
        Motion::WordEnd => "word end",
        Motion::BigWordEnd => "WORD end",
        Motion::WordEndBackward => "word end backward",
        Motion::BigWordEndBackward => "WORD end backward",
        Motion::ParagraphForward => "paragraph forward",
        Motion::ParagraphBackward => "paragraph backward",
        Motion::SentenceForward => "sentence forward",
        Motion::SentenceBackward => "sentence backward",
        Motion::UnmatchedOpen(_) => "unmatched open",
        Motion::UnmatchedClose(_) => "unmatched close",
        Motion::MatchPair => "match pair %",
        Motion::LineStart => "line start",
        Motion::LineEnd => "line end",
        Motion::LastNonBlank => "last non-blank",
        Motion::Column => "column",
        Motion::FirstLine => "gg",
        Motion::LastLine => "G",
        Motion::Window { .. } => "window line",
        Motion::RepeatFind { .. } => "repeat find",
        Motion::RepeatSearch { .. } => "repeat search",
        Motion::FindChar { ch, till, .. } => {
            return format!("{} '{ch}'", if *till { "till" } else { "find" });
        }
        Motion::Search(query) => return format!("search /{}", query.source()),
        Motion::SearchBackward(query) => return format!("search ?{}", query.source()),
        Motion::Mark { name, exact, .. } => {
            return format!("mark {}{name}", if *exact { '`' } else { '\'' });
        }
    };
    fixed.to_owned()
}
