//! The projection-admission kernel, verified (0057 VF18): whether a
//! journal edit to a collection's projected view lands inside one
//! excerpt's writable body. Generated chrome (headers, gap rows, card
//! borders) owns no edit — write-back refuses it — and this decision is
//! the guard `editor/collections/editing.rs` runs per sequential change
//! on the real path, extracted pure and called from that loop, never a
//! copied algorithm. `cargo verus verify` checks it; the normal build
//! compiles the `verus!` block as plain Rust (ghost code erases), the
//! same arrangement as `editmap.rs` (0045).
//!
//! Verified contract (Verus 0.2026.09.06, rustc 1.98.0, z3 4.16.0):
//! - WritableSpanOnly: a span owns an edit iff the edit starts inside
//!   the excerpt's writable view body and the extent it replaces ends
//!   within the rendered span — `span_owns_edit`;
//! - an owned edit's start is bounded by the body, so bytes owned by no
//!   span (chrome, and the seams between excerpts) reject every edit.

use vstd::prelude::*;

verus! {

/// Admission, mathematically: the edit starts within the writable body
/// `[span_start, body_end]`, its replaced extent ends by the rendered
/// span end, and a start at the rendered end is admitted only for the
/// degenerate empty span.
pub open spec fn span_owns(
    edit_start: int,
    edit_old_end: int,
    span_start: int,
    body_end: int,
    rendered_end: int,
) -> bool {
    &&& edit_start >= span_start
    &&& edit_start <= body_end
    &&& edit_old_end <= rendered_end
    &&& (edit_start < rendered_end || span_start == rendered_end)
}

/// Does this span own this edit? The collection write-back loop's
/// per-change admission check calls this exact decision; `None` from
/// the owner's search means chrome or a seam, and the edit is refused.
pub fn span_owns_edit(
    edit_start: usize,
    edit_old_end: usize,
    span_start: usize,
    body_end: usize,
    rendered_end: usize,
) -> (owned: bool)
    ensures
        owned == span_owns(
            edit_start as int,
            edit_old_end as int,
            span_start as int,
            body_end as int,
            rendered_end as int,
        ),
{
    edit_start >= span_start
        && edit_start <= body_end
        && edit_old_end <= rendered_end
        && (edit_start < rendered_end || span_start == rendered_end)
}

/// An owned edit starts inside the writable body and its replaced
/// extent stays within the rendered span: chrome is owned by no span,
/// so it rejects every edit.
proof fn owned_edit_stays_in_span(
    edit_start: int,
    edit_old_end: int,
    span_start: int,
    body_end: int,
    rendered_end: int,
)
    requires
        span_owns(edit_start, edit_old_end, span_start, body_end, rendered_end),
    ensures
        span_start <= edit_start <= body_end,
        edit_old_end <= rendered_end,
{
}

}
