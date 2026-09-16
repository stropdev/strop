//! The search publication-boundary kernel, verified (0063 §6.6): the
//! decisions SearchLifecycle.tla names as its invariants, carried as
//! proof obligations on the production functions the guard sites call —
//! not a copy. `cargo verus verify` checks them; the normal build
//! compiles the `verus!` block as plain Rust (ghost code erases), the
//! same arrangement as `editmap.rs` (0045).
//!
//! Verified contract (Verus 0.2026.09.06, rustc 1.98.0, z3 4.16.0),
//! one decision per named invariant:
//! - RowsCurrent: a generation-tagged job publishes iff its generation
//!   is the live one — `generation_is_live` (the picker stream's ticket
//!   request id and the workspace-symbols generation both decide here);
//! - StaleAcceptsNever: a row extracted against one source revision is
//!   accepted iff the source is still observed at that revision —
//!   `revision_is_current`;
//! - CompletionHonest: the "no more coming" flag requires the live
//!   generation's terminal event and no truncation —
//!   `completion_is_honest`;
//! - WarmBounded: a warm attach starts only strictly below the bound —
//!   `warm_slot_free`.

use vstd::prelude::*;

verus! {

/// RowsCurrent: publication is guarded on the job's generation being
/// the live one.
pub open spec fn rows_current(job_gen: int, live_gen: int) -> bool {
    job_gen == live_gen
}

/// StaleAcceptsNever: acceptance re-checks that the source still stands
/// at the revision the row was extracted against.
pub open spec fn revision_current(extracted_rev: int, observed_rev: int) -> bool {
    extracted_rev == observed_rev
}

/// CompletionHonest: the complete flag implies the live generation's
/// terminal, untruncated.
pub open spec fn completion_honest(terminal_gen: int, live_gen: int, truncated: bool) -> bool {
    terminal_gen == live_gen && !truncated
}

/// WarmBounded: warm-up concurrency within the bound.
pub open spec fn warm_bounded(in_flight: int, limit: int) -> bool {
    0 <= in_flight <= limit
}

/// Publish a generation-tagged delivery? Callers pair the generation
/// the job was armed with against the generation the surface currently
/// owns; anything else is a retired query's late reply.
pub fn generation_is_live(job_gen: u64, live_gen: u64) -> (live: bool)
    ensures
        live == rows_current(job_gen as int, live_gen as int),
{
    job_gen == live_gen
}

/// Accept a row extracted at `extracted_rev` while the source is
/// observed at `observed_rev`? A moved source is never consumed
/// knowingly.
pub fn revision_is_current(extracted_rev: u64, observed_rev: u64) -> (current: bool)
    ensures
        current == revision_current(extracted_rev as int, observed_rev as int),
{
    extracted_rev == observed_rev
}

/// Publish the completion flag for `terminal_gen`'s terminal event?
/// Only the live generation's terminal counts, and a truncated result
/// set stays visibly incomplete.
pub fn completion_is_honest(terminal_gen: u64, live_gen: u64, truncated: bool) -> (honest: bool)
    ensures
        honest == completion_honest(terminal_gen as int, live_gen as int, truncated),
{
    terminal_gen == live_gen && !truncated
}

/// May one more warm attach start? A slot is free strictly below the
/// limit, so taking it keeps the flight within the bound.
pub fn warm_slot_free(in_flight: usize, limit: usize) -> (free: bool)
    ensures
        free == (in_flight < limit),
        free ==> warm_bounded(in_flight as int + 1, limit as int),
{
    in_flight < limit
}

/// A retired query can never publish: under one live generation, a
/// distinct generation decides oppositely.
proof fn retired_generation_never_live(job_gen: int, retired_gen: int, live_gen: int)
    requires
        rows_current(job_gen, live_gen),
        retired_gen != live_gen,
    ensures
        !rows_current(retired_gen, live_gen),
{
}

/// A moved source is never accepted: distinct revisions decide
/// oppositely.
proof fn moved_source_never_current(extracted_rev: int, observed_rev: int)
    requires
        extracted_rev != observed_rev,
    ensures
        !revision_current(extracted_rev, observed_rev),
{
}

/// Honesty pins the terminal to the live generation and rules out
/// truncation.
proof fn honest_completion_is_current(terminal_gen: int, live_gen: int, truncated: bool)
    requires
        completion_honest(terminal_gen, live_gen, truncated),
    ensures
        terminal_gen == live_gen,
        !truncated,
{
}

}
