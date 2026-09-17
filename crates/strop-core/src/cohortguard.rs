//! The recovery-cohort kernel, verified (0057 VF18): the checkpoint
//! completeness decision, the save-retirement staleness rule and the
//! cohort watermark step for draft recovery (0056 AR04 §5). These are
//! the decisions `editor/recovery/{mod,store}.rs` make on the real
//! checkpoint path — extracted pure and called from those sites, never
//! a copied algorithm. `cargo verus verify` checks them; the normal
//! build compiles the `verus!` block as plain Rust (ghost code erases),
//! the same arrangement as `editmap.rs` (0045).
//!
//! Verified contract (Verus 0.2026.09.06, rustc 1.98.0, z3 4.16.0):
//! - CohortComplete: a draft's bytes join the cohort iff they fit WHOLE
//!   in the remaining budget; anything else is reported over-bound,
//!   never truncated, never silently dropped — `fits`;
//! - SaveRetirement: a captured checkpoint is current iff it holds the
//!   draft's live revision — a confirmed save makes the draft clean
//!   (ineligible) and edits after the saved snapshot move the live
//!   revision past the capture, so either way staleness republishes the
//!   cohort without the superseded checkpoint — `capture_is_current`,
//!   `retained_is_stale`;
//! - WatermarkOrdering: cohort identities strictly increase within a
//!   session, so a completed publication never moves the durable
//!   watermark backwards — `next_cohort`.

use vstd::prelude::*;

verus! {

/// Completeness, mathematically: whole or reported, within the limit.
pub open spec fn fits_spec(used: int, len: int, limit: int) -> bool {
    used + len <= limit
}

/// Does one more draft fit WHOLE in the bounded cohort? The publisher
/// consults this exact decision per record; a draft that does not fit
/// is recorded over-bound (reported, not durable) — never truncated,
/// never silently dropped. Callers uphold `used <= limit`: the budget
/// starts below the limit and grows only by drafts that fit.
pub fn fits(used: u64, len: u64, limit: u64) -> (fits: bool)
    requires
        used <= limit,
    ensures
        fits == fits_spec(used as int, len as int, limit as int),
{
    len <= limit - used
}

/// Staleness, per eligible draft: the captured checkpoint clock holds
/// exactly the draft's live revision. Anything else republishes.
pub fn capture_is_current(captured: Option<u64>, live: u64) -> (current: bool)
    ensures
        current == (captured.is_some() && captured.unwrap() == live),
{
    match captured {
        Some(revision) => revision == live,
        None => false,
    }
}

/// Staleness, per retained (captured) entry: a checkpoint whose draft
/// closed or left eligibility must be republished without it. This is
/// the half of the save-retirement rule that retires a saved draft's
/// checkpoint — a confirmed save makes the draft clean, hence
/// ineligible.
pub fn retained_is_stale(still_open: bool, still_eligible: bool) -> (stale: bool)
    ensures
        stale == (!still_open || !still_eligible),
{
    !still_open || !still_eligible
}

/// The next cohort identity. Cohorts strictly increase within a
/// session, so a completed publication's watermark never names an older
/// cohort than the one already durable. The bound is unreachable in
/// practice (2^64 published cohorts); the precondition keeps the step
/// wrap-free by construction.
pub fn next_cohort(current: u64) -> (next: u64)
    requires
        current < u64::MAX,
    ensures
        next == current + 1,
        next > current,
{
    current + 1
}

/// A draft that does not fit whole exceeds the limit: the cohort either
/// carries every byte or reports the draft, never a prefix.
proof fn unfit_draft_exceeds_limit(used: int, len: int, limit: int)
    requires
        0 <= used <= limit,
        !fits_spec(used, len, limit),
    ensures
        used + len > limit,
{
}

/// A fitting draft keeps the budget inside the limit for the next one.
proof fn fit_draft_preserves_budget(used: int, len: int, limit: int)
    requires
        0 <= used <= limit,
        0 <= len,
        fits_spec(used, len, limit),
    ensures
        0 <= used + len <= limit,
{
}

}
