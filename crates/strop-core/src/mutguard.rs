//! The mutation-authority kernel, verified (0057 VF18): who may apply a
//! text mutation, who may save, and when a confirmed save retires dirty
//! state. The decisions are the guards `buffer/mutation.rs` and
//! `buffer/io.rs` run on the real path — extracted pure and called from
//! those guards, never a copied algorithm. `cargo verus verify` checks
//! them; the normal build compiles the `verus!` block as plain Rust
//! (ghost code erases), the same arrangement as `editmap.rs` (0045).
//!
//! Verified contract (Verus 0.2026.09.06, rustc 1.98.0, z3 4.16.0):
//! - MutationAuthority: a batch is admitted iff the buffer is writable
//!   and the caller's base IS the live revision — `classify_edit`; the
//!   refusal order (readonly before stale) is part of the contract, it
//!   is the error precedence callers observe;
//! - WriteAuthority: the streaming edit leases refuse a readonly buffer
//!   before touching text — `writable`;
//! - SaveAuthority: a readonly buffer's save needs the explicit force
//!   opt-in — `save_admitted`;
//! - SaveRetirement: a confirmed save retires dirty text only when it
//!   acknowledges the live revision, so an older snapshot's confirmed
//!   write never clears edits made after it — `save_ack_is_current`.

use vstd::prelude::*;

verus! {

/// The batch-admission decision as data; the guard sites map it onto
/// the typed `EditError` callers already see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAdmission {
    /// Writable buffer and the caller's base is the live revision.
    Admitted,
    /// A readonly buffer refuses before any revision question.
    Readonly,
    /// The caller's base is not the live revision.
    Stale,
}

/// Who may apply, mathematically: a writable buffer whose live revision
/// is exactly the caller's base.
pub open spec fn edit_admitted(readonly: bool, base: int, live: int) -> bool {
    !readonly && base == live
}

/// Who may save, mathematically: a writable buffer, or the explicit
/// force opt-in.
pub open spec fn save_allowed(readonly: bool, force: bool) -> bool {
    !readonly || force
}

/// Retirement, mathematically: the save receipt names the live revision.
pub open spec fn ack_current(saved: int, live: int) -> bool {
    saved == live
}

/// May this batch apply? `Buffer::prepare_replacements` and
/// `Buffer::apply_prepared` consult this exact decision; the refusal
/// order (readonly before stale) is the error precedence callers
/// observe.
pub fn classify_edit(readonly: bool, base: u64, live: u64) -> (decision: EditAdmission)
    ensures
        (decision == EditAdmission::Admitted) == edit_admitted(readonly, base as int, live as int),
        (decision == EditAdmission::Readonly) == readonly,
        (decision == EditAdmission::Stale) == (!readonly && base as int != live as int),
{
    if readonly {
        EditAdmission::Readonly
    } else if base != live {
        EditAdmission::Stale
    } else {
        EditAdmission::Admitted
    }
}

/// Write authority alone, for the streaming edit leases where no base
/// revision is in play: a readonly buffer refuses before text moves.
pub fn writable(readonly: bool) -> (yes: bool)
    ensures
        yes == !readonly,
{
    !readonly
}

/// May this save proceed? `:w!` is the explicit authority override; a
/// readonly buffer without it refuses before any path is resolved.
pub fn save_admitted(readonly: bool, force: bool) -> (admitted: bool)
    ensures
        admitted == save_allowed(readonly, force),
{
    !readonly || force
}

/// Does a confirmed save acknowledge the LIVE revision? Only then does
/// it retire dirty text; a receipt for an older snapshot leaves edits
/// made after that snapshot dirty (the retirement rule of 0056 AR04 §5
/// at the buffer clock).
pub fn save_ack_is_current(saved: u64, live: u64) -> (current: bool)
    ensures
        current == ack_current(saved as int, live as int),
{
    saved == live
}

/// A readonly buffer admits no mutation, whatever the base.
proof fn readonly_never_admitted(base: int, live: int)
    ensures
        !edit_admitted(true, base, live),
{
}

/// A stale base admits no mutation, however writable the buffer.
proof fn stale_base_never_admitted(base: int, live: int)
    requires
        base != live,
    ensures
        !edit_admitted(false, base, live),
{
}

/// A save receipt that names an older revision never retires live edits.
proof fn older_save_retires_nothing(saved: int, live: int)
    requires
        saved != live,
    ensures
        !ack_current(saved, live),
{
}

}
