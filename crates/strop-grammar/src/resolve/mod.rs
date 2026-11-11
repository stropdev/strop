//! Commands → byte ranges. THE function: execute and preview both
//! consume `resolve`. Search runs through the compiled query engine
//! (`crate::query`) — bounded, typed errors, explicit match ranges.
//!
//! A target (a motion from the cursor, or a text object) first becomes a
//! `Span` in Vim's own terms: two positions and how the far one counts.
//! Vim's range rules then apply to the span in one place (`operated`),
//! whatever produced it.

use std::sync::atomic::{AtomicBool, Ordering};
use strop_core::{Buffer, Range};

use crate::types::*;

mod blocks;
mod find;
mod motion;
mod objects;
mod pairs;
pub(crate) mod search;
mod tags;
mod walk;
mod words;

pub use find::find_character;
pub use objects::match_pair;
pub use pairs::{delimiter_pair, matching_delimiter_at, MatchCancelled};
pub use search::literal_from;
pub use words::{is_keyword, word_run};

use crate::query::{search_backward, search_forward, QueryError};
use objects::{bracket_pair, quote_pair, surround_pair};
use walk::Walk;

/// How the far end of a span counts (Vim's `motion_type` + `inclusive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Up to, not including, the end position.
    Exclusive,
    /// Through the character at the end position.
    Inclusive,
    /// Every line from the start's to the end's.
    Linewise,
}

/// The text a target covers, before the range rules: `start <= end`.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    kind: Kind,
}

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
pub fn resolve(buf: &Buffer, cursor: usize, cmd: &Command) -> Result<Option<Resolved>, QueryError> {
    resolve_controlled(buf, cursor, cmd, None)
}

fn object_name(obj: Object) -> String {
    match obj {
        Object::Word => "word".into(),
        Object::BigWord => "WORD".into(),
        Object::Paragraph => "paragraph".into(),
        Object::Tag => "tag".into(),
        Object::Quote(q) => q.to_string(),
        Object::Bracket { open, .. } => open.to_string(),
    }
}

fn resolve_controlled(
    buf: &Buffer,
    cursor: usize,
    cmd: &Command,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Resolved>, QueryError> {
    check_cancel(cancel)?;
    let count = cmd.count.unwrap_or(1);
    let mut motion_target = None;
    let (span, mut spec) = match &cmd.target {
        Target::Linewise => {
            let last = buf.last_content_line();
            let line = buf.line_of(cursor).min(last);
            if count > 1 && line == last {
                // Vim: `2dd` on the last line cannot move down, and fails
                return Ok(None);
            }
            let end_line = line.saturating_add(count - 1).min(last);
            (
                Span {
                    start: motion::line_landing(buf, cursor, line, cmd.op),
                    end: buf.line_start(end_line),
                    kind: Kind::Linewise,
                },
                format!("{count} line{}", if count > 1 { "s" } else { "" }),
            )
        }
        Target::Object { inner, obj } => {
            let extent = none!(objects::extent(buf, cursor, None, *obj, *inner, count));
            (
                Span {
                    start: extent.start,
                    end: extent.end,
                    kind: extent.kind,
                },
                format!(
                    "{} {}",
                    if *inner { "inner" } else { "around" },
                    object_name(*obj)
                ),
            )
        }
        Target::Match { backward, query } => {
            let query = none!(query.as_ref());
            let mut hit = None;
            let mut from = cursor;
            for step in 0..count {
                check_cancel(cancel)?;
                hit = if *backward {
                    match search_backward(
                        buf,
                        buf.ceil_boundary(from + usize::from(step == 0)),
                        query,
                    )? {
                        Some(hit) => Some(hit),
                        None => search_backward(buf, buf.len_bytes(), query)?,
                    }
                } else {
                    // a match under the cursor is the one selected
                    let under = search_backward(buf, buf.ceil_boundary(from + 1), query)?
                        .filter(|hit| step == 0 && hit.end.get() > from);
                    match under {
                        Some(hit) => Some(hit),
                        None => match search_forward(buf, buf.ceil_boundary(from + 1), query)? {
                            Some(hit) => Some(hit),
                            None => search_forward(buf, 0, query)?,
                        },
                    }
                };
                from = none!(hit).start.get();
            }
            let hit = none!(hit);
            (
                Span {
                    start: hit.start.get(),
                    end: hit.end.get(),
                    kind: Kind::Exclusive,
                },
                format!("match /{}", query.source()),
            )
        }
        Target::SurroundDelete(ch) | Target::SurroundChange { from: ch, .. } => {
            let (open, close) = none!(surround_pair(*ch));
            let (o, c) = if open == close {
                none!(quote_pair(buf, cursor, open))
            } else {
                none!(bracket_pair(buf, cursor, open, close))
            };
            (
                Span {
                    start: o,
                    end: c,
                    kind: Kind::Inclusive,
                },
                format!("surround {}", *ch),
            )
        }
        Target::SurroundAdd { ch, inner } => {
            // resolve the inner motion as if yanked, then wrap its range
            let sub = Command {
                op: Some(Op::Yank),
                register: None,
                count: Some(1),
                target: (**inner).clone(),
                force: None,
                keys: String::new(),
            };
            let mut resolved = none!(resolve_controlled(buf, cursor, &sub, cancel)?);
            resolved.spec = format!(
                "yank, surround with {}, {} bytes",
                *ch,
                resolved.range.len()
            );
            return Ok(Some(resolved));
        }
        Target::Motion(m) => {
            let moved = none!(motion::moved(buf, cursor, m, cmd.count, cmd.op, cancel)?);
            motion_target = Some(moved.to);
            (
                Span {
                    start: cursor.min(moved.to),
                    end: cursor.max(moved.to),
                    kind: moved.kind,
                },
                motion::name(m, cmd.op),
            )
        }
    };
    check_cancel(cancel)?;
    let Some(op) = cmd.op else {
        // No operator: the span as it stands (a motion's travel, or the
        // object Visual mode selects).
        let (range, origin, end_adjusted) = operated(buf, span, None, None);
        let inclusive = span.kind == Kind::Inclusive;
        spec = format!(
            "{spec} · {}",
            if inclusive { "inclusive" } else { "exclusive" }
        );
        return Ok(Some(Resolved {
            range,
            motion_target,
            origin,
            end_adjusted,
            spec,
        }));
    };
    if cmd.force == Some(Force::Blockwise) {
        // a forced block needs a column model the resolver does not have
        return Ok(None);
    }
    let (mut range, origin, end_adjusted) = operated(buf, span, Some(op), cmd.force);
    if range.is_empty()
        && matches!(op, Op::Upper | Op::Lower | Op::ToggleCase | Op::Rot13)
        && span.kind == Kind::Exclusive
    {
        // Vim steps the end of an exclusive region back one character.
        // From a line's first column that step leaves the line, and the
        // operator then runs to the line's end — or, on the first line,
        // where there is nowhere to step to, over one character.
        let line = buf.line_of(span.start);
        if span.start == buf.line_start(line) {
            let end = if line == 0 {
                buf.ceil_boundary(span.start + 1).min(buf.line_end(line))
            } else {
                buf.line_end(line)
            };
            range = Range::charwise(span.start, end);
        }
    }
    spec = format!(
        "{}, {}, {} bytes · {}",
        op.name(),
        spec,
        range.len(),
        if range.inclusive() || range.is_linewise() {
            "inclusive"
        } else {
            "exclusive"
        }
    );
    Ok(Some(Resolved {
        range,
        motion_target,
        origin,
        end_adjusted,
        spec,
    }))
}

/// Vim's range rules (`do_pending_operator`), in its order: forcing
/// (`:help o_v`), exclusive-linewise (`:help exclusive-linewise`), the
/// `d` exception (`:help d`), then the bytes.
fn operated(
    buf: &Buffer,
    span: Span,
    op: Option<Op>,
    force: Option<Force>,
) -> (Range, usize, bool) {
    let Span {
        start,
        mut end,
        mut kind,
    } = span;
    match force {
        Some(Force::Charwise) => {
            kind = match kind {
                Kind::Linewise | Kind::Inclusive => Kind::Exclusive,
                Kind::Exclusive => Kind::Inclusive,
            }
        }
        Some(Force::Linewise) => kind = Kind::Linewise,
        Some(Force::Blockwise) | None => {}
    }
    let (start_line, end_line) = (buf.line_of(start), buf.line_of(end));
    let mut end_adjusted = false;
    let start_walk = Walk::new(buf, start);
    let multiline = end_line > start_line;
    if op.is_some() && kind == Kind::Exclusive && multiline && end == buf.line_start(end_line) {
        // The motion stopped in column 1: the line break stays, and if
        // the start had only blanks before it whole lines are meant.
        let previous = end_line - 1;
        if start_walk.in_indent(0) {
            kind = Kind::Linewise;
            end_adjusted = true;
            end = buf.line_start(previous);
        } else {
            let (line_start, line_end) = (buf.line_start(previous), buf.line_end(previous));
            if line_end > line_start {
                end = buf.clamp_boundary(line_end - 1).max(line_start);
                kind = Kind::Inclusive;
            } else {
                end = line_start;
            }
        }
    }
    if op.is_some_and(Op::on_lines) {
        // operators that act on lines read what is left as its lines
        kind = Kind::Linewise;
    }
    let end_line = buf.line_of(end);
    // One past the last operated byte of a characterwise span.
    let past = if kind == Kind::Inclusive && end < buf.line_end(end_line) {
        buf.ceil_boundary(end + 1)
    } else {
        end
    };
    if op == Some(Op::Delete)
        && force.is_none()
        && kind != Kind::Linewise
        && end_line > start_line
        && start_walk.in_indent(0)
        && (past..buf.line_end(end_line)).all(|at| matches!(buf.byte(at), b' ' | b'\t'))
    {
        kind = Kind::Linewise;
    }
    let range = match kind {
        Kind::Linewise => {
            let first = buf.line_start(start_line);
            let after = if end_line + 1 >= buf.len_lines() {
                buf.len_bytes()
            } else {
                buf.line_start(end_line + 1)
            };
            Range::linewise(first, after)
        }
        Kind::Inclusive => Range::charwise(start, past.max(start)).with_inclusive(true),
        Kind::Exclusive => Range::charwise(start, end.max(start)),
    };
    (range, start, end_adjusted)
}

/// A text object in Visual mode: the selection it leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectSelection {
    pub anchor: usize,
    pub head: usize,
    /// The object is whole lines (`ip`): Visual becomes linewise.
    pub linewise: bool,
}

/// Select `cmd`'s text object in Visual mode. A selection of one
/// character selects the object under it; a longer one is extended by
/// the object (`viwiw`, `vipip`, `vi(i(`), in the selection's direction.
pub fn select_object(
    buf: &Buffer,
    anchor: usize,
    head: usize,
    cmd: &Command,
) -> Option<ObjectSelection> {
    let Target::Object { inner, obj } = &cmd.target else {
        return None;
    };
    let count = cmd.count.unwrap_or(1);
    let extent = objects::extent(buf, head, Some((anchor, head)), *obj, *inner, count)?;
    let (start, end) = (extent.start, extent.end);
    Some(match extent.kind {
        Kind::Linewise => ObjectSelection {
            anchor: start,
            head: end,
            linewise: true,
        },
        Kind::Inclusive => ObjectSelection {
            anchor: start,
            head: end,
            linewise: false,
        },
        Kind::Exclusive if end > start => ObjectSelection {
            anchor: start,
            head: buf.clamp_boundary(end - 1),
            linewise: false,
        },
        Kind::Exclusive => return None,
    })
}

/// Where the cursor lands after a resolved motion command.
pub fn cursor_after(_buf: &Buffer, _cursor: usize, _cmd: &Command, r: &Resolved) -> usize {
    r.motion_target.unwrap_or(r.range.start.get())
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
    /// `Resolved::origin`: the operated text's first position.
    pub origin: usize,
    /// `Resolved::end_adjusted`.
    pub end_adjusted: bool,
    /// `Resolved::motion_target`.
    pub motion_target: Option<usize>,
}

/// Resolve one command over every cursor; Ok(None) when nothing
/// resolves; Err carries a query failure from any cursor's attempt.
pub fn plan(
    buf: &Buffer,
    cursors: &[usize],
    cmd: &Command,
) -> Result<Option<ActionPlan>, QueryError> {
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
                    origin: resolved.origin,
                    end_adjusted: resolved.end_adjusted,
                    motion_target: resolved.motion_target,
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
) -> Result<Vec<Option<Resolved>>, QueryError> {
    cursors.iter().map(|&c| resolve(buf, c, cmd)).collect()
}

/// The same pure resolver under a native work owner's stop flag.
pub fn resolve_many_cancellable(
    buf: &Buffer,
    cursors: &[usize],
    cmd: &Command,
    cancel: &AtomicBool,
) -> Result<Vec<Option<Resolved>>, QueryError> {
    cursors
        .iter()
        .map(|&cursor| resolve_controlled(buf, cursor, cmd, Some(cancel)))
        .collect()
}

pub(crate) fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), QueryError> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(QueryError::Cancelled)
    } else {
        Ok(())
    }
}
