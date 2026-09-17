//! The prepared-view freshness kernel, verified (0057 VF18): the
//! staleness re-key rule for the published PreparedView (0056 AR03). A
//! prepared pane window is keyed by document identity + revision and
//! the owning view generation; when the live document revision moves
//! past the prepared one, the window is stale and must not drive
//! coordinate conversions or viewport decisions. The decision is the
//! one `editor/prepare.rs::PreparedPane::is_stale` makes on the real
//! paint path — extracted pure and called from it, never a copied
//! algorithm. `cargo verus verify` checks it; the normal build compiles
//! the `verus!` block as plain Rust (ghost code erases), the same
//! arrangement as `editmap.rs` (0045).
//!
//! Verified contract (Verus 0.2026.09.06, rustc 1.98.0, z3 4.16.0):
//! - FreshnessByKey: a prepared window is fresh iff its keyed revision
//!   IS the live revision — `revision_is_stale`; a stale window is
//!   never painted as current and a fresh window's facts describe the
//!   live document.

use vstd::prelude::*;

verus! {

/// Staleness, mathematically: the keyed revision is not the live one.
pub open spec fn stale_key(prepared: int, live: int) -> bool {
    prepared != live
}

/// Is this prepared window stale against the live revision? The paint
/// path's re-key check calls this exact decision.
pub fn revision_is_stale(prepared: u64, live: u64) -> (stale: bool)
    ensures
        stale == stale_key(prepared as int, live as int),
{
    prepared != live
}

/// A fresh window's keyed revision is the live revision: its viewport
/// facts describe the document as it stands.
proof fn fresh_window_is_live(prepared: int, live: int)
    requires
        !stale_key(prepared, live),
    ensures
        prepared == live,
{
}

}
