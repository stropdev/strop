//! Search publication decisions (0063 §6.6): a generation-tagged job
//! publishes only for the live generation, a row extracted against one
//! source revision is accepted only while the source is still at that
//! revision, and warm attaches stay strictly below their bound.

/// Publish a generation-tagged delivery? Callers pair the generation
/// the job was armed with against the generation the surface currently
/// owns; anything else is a retired query's late reply.
pub fn generation_is_live(job_gen: u64, live_gen: u64) -> bool {
    job_gen == live_gen
}

/// Accept a row extracted at `extracted_rev` while the source is
/// observed at `observed_rev`? A moved source is never consumed
/// knowingly.
pub fn revision_is_current(extracted_rev: u64, observed_rev: u64) -> bool {
    extracted_rev == observed_rev
}

/// May one more warm attach start? A slot is free strictly below the
/// limit, so taking it keeps the flight within the bound.
pub fn warm_slot_free(in_flight: usize, limit: usize) -> bool {
    in_flight < limit
}
