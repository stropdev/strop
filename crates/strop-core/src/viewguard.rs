//! Prepared-view freshness (0056 AR03): a prepared pane window is keyed by
//! document revision; once the live revision moves past it the window is
//! stale and must not drive coordinate conversions or viewport decisions.
//! `editor/prepare.rs::PreparedPane::is_stale` calls this decision.

/// Is this prepared window stale against the live revision? The paint
/// path's re-key check calls this exact decision.
pub fn revision_is_stale(prepared: u64, live: u64) -> bool {
    prepared != live
}
