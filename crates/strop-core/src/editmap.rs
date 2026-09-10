//! The anchor-mapping kernel, verified (0045): `map_position` carries the
//! proof of its contract in-tree; `cargo verus verify` checks it, and the
//! normal build compiles the `verus!` block as plain Rust (ghost code
//! erases). This is the production function `editor/transact.rs` calls —
//! not a copy.
//!
//! Verified contract (Verus 0.2026.09.06, rustc 1.98.0, z3 4.16.0):
//! - anchors before the edit never move;
//! - anchors at/past the old end shift by the delta (right affinity);
//! - anchors inside the replaced range collapse to the edit start;
//! - the mapping is monotone and stays inside the new buffer length.

use vstd::prelude::*;

verus! {

/// The mapping, mathematically: right-affinity anchor through one edit.
pub open spec fn map_spec(position: int, start: int, old_end: int, new_end: int) -> int {
    if position < start {
        position
    } else if position >= old_end {
        new_end + (position - old_end)
    } else {
        start
    }
}

/// One anchor through one edit. Callers guarantee no overflow: positions
/// are byte offsets bounded by the buffer length, and `new_end` is the
/// post-edit end of the replaced range (`start + replacement length`).
pub fn map_position(position: usize, start: usize, old_end: usize, new_end: usize) -> (mapped: usize)
    requires
        start <= old_end,
        position >= old_end ==> new_end + (position - old_end) <= usize::MAX,
    ensures
        mapped as int == map_spec(position as int, start as int, old_end as int, new_end as int),
{
    if position < start {
        position
    } else if position >= old_end {
        new_end.saturating_add(position - old_end)
    } else {
        start
    }
}

/// Monotonicity: anchors keep their relative order through one edit.
/// `start <= new_end` holds by construction (the post-edit end of a
/// replacement is its start plus the replacement length).
proof fn map_spec_monotone(a: int, b: int, start: int, old_end: int, new_end: int)
    requires
        start <= old_end,
        start <= new_end,
        a <= b,
    ensures
        map_spec(a, start, old_end, new_end) <= map_spec(b, start, old_end, new_end),
{
}

/// Bounds: an anchor inside the old buffer stays inside the new one.
proof fn map_spec_in_bounds(
    position: int,
    start: int,
    old_end: int,
    new_end: int,
    old_len: int,
    new_len: int,
)
    requires
        0 <= start <= old_end <= old_len,
        0 <= start <= new_end <= new_len,
        0 <= position <= old_len,
        new_len == old_len - (old_end - start) + (new_end - start),
    ensures
        0 <= map_spec(position, start, old_end, new_end) <= new_len,
{
}

}
