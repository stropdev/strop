//! Draft-recovery cohort decisions (0056 AR04 §5): checkpoint completeness,
//! the save-retirement staleness rule and the cohort watermark step.
//! `editor/recovery/{mod,store}.rs` call these decisions. A draft joins the
//! cohort only if it fits whole; cohort identities strictly increase.

/// Does one more draft fit WHOLE in the bounded cohort? The publisher
/// consults this exact decision per record; a draft that does not fit
/// is recorded over-bound (reported, not durable) — never truncated,
/// never silently dropped. Callers uphold `used <= limit`: the budget
/// starts below the limit and grows only by drafts that fit.
pub fn fits(used: u64, len: u64, limit: u64) -> bool {
    len <= limit - used
}

/// Staleness, per eligible draft: the captured checkpoint clock holds
/// exactly the draft's live revision. Anything else republishes.
pub fn capture_is_current(captured: Option<u64>, live: u64) -> bool {
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
pub fn retained_is_stale(still_open: bool, still_eligible: bool) -> bool {
    !still_open || !still_eligible
}

/// The next cohort identity. Cohorts strictly increase within a
/// session, so a completed publication's watermark never names an older
/// cohort than the one already durable. The bound is unreachable in
/// practice (2^64 published cohorts); the precondition keeps the step
/// wrap-free by construction.
pub fn next_cohort(current: u64) -> u64 {
    current + 1
}
