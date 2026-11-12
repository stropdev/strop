//! The cache core (0099): which entry may be served, which entries an
//! invalidation removes, and which slot a new entry takes. `strop-engine`'s
//! `editor/cache.rs` keeps the values beside these slots and asks every
//! question here; no flow decides validity for itself.
//!
//! An entry is named by three integers: its **subject** (what it describes),
//! its **producer** (who computed it) and the **version** of the subject it
//! was computed for. Validity is equality on all three; there is no dirty
//! flag an edit could forget to set. The table is a slice of fixed length the
//! owner allocates once, so it can never hold more entries than that, and a
//! value whose cost is over the limit is refused, so the total cost is at most
//! `length × limit`.
//!
//! Proved (proofs/Kernel/Cache.lean):
//!
//! - `find` returns the first live slot whose subject, producer and version
//!   all equal the ones asked for, and `None` exactly when there is none: an
//!   entry for another version is never served;
//! - `retire` kills every selected slot and touches no other; a lookup for a
//!   retired subject or producer misses, whatever the version;
//! - `place` keeps the table's length and its cost bound, refuses a value over
//!   the limit without changing anything, leaves the new entry findable under
//!   its key and no other live entry for its (subject, producer) pair;
//! - the slot `place` overwrites is a dead one whenever one exists, and
//!   otherwise the least recently used;
//! - `touch` changes one slot's last-use tick and nothing else.

/// One table slot. A dead slot's other fields mean nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub live: bool,
    pub subject: u64,
    pub producer: u64,
    pub version: u64,
    pub cost: usize,
    pub used: u64,
}

/// A dead slot: what the owner fills a new table with.
pub fn vacant() -> Slot {
    Slot {
        live: false,
        subject: 0,
        producer: 0,
        version: 0,
        cost: 0,
        used: 0,
    }
}

/// Whether `slot` holds the entry for exactly this subject, producer and
/// version.
pub fn holds(slot: Slot, subject: u64, producer: u64, version: u64) -> bool {
    slot.live && slot.subject == subject && slot.producer == producer && slot.version == version
}

/// The slot holding the entry for this subject, producer and version.
pub fn find(slots: &[Slot], subject: u64, producer: u64, version: u64) -> Option<usize> {
    let mut k = 0;
    while k < slots.len() {
        if holds(slots[k], subject, producer, version) {
            return Some(k);
        }
        k += 1;
    }
    None
}

/// Record a use of slot `index` at tick `now`.
pub fn touch(slots: &mut [Slot], index: usize, now: u64) {
    if index < slots.len() {
        let mut slot = slots[index];
        slot.used = now;
        slots[index] = slot;
    }
}

/// Whether an invalidation by subject and/or producer covers `slot`. With
/// neither it covers every slot.
pub fn selects(
    slot: Slot,
    subject: u64,
    producer: u64,
    by_subject: bool,
    by_producer: bool,
) -> bool {
    (!by_subject || slot.subject == subject) && (!by_producer || slot.producer == producer)
}

/// Kill every slot the invalidation covers.
pub fn retire(
    slots: &mut [Slot],
    subject: u64,
    producer: u64,
    by_subject: bool,
    by_producer: bool,
) {
    let mut k = 0;
    while k < slots.len() {
        let mut slot = slots[k];
        if selects(slot, subject, producer, by_subject, by_producer) {
            slot.live = false;
            slots[k] = slot;
        }
        k += 1;
    }
}

/// The slot a new entry takes: the first dead one, or with none dead the
/// least recently used. `slots.len()` only for an empty table.
pub fn victim(slots: &[Slot]) -> usize {
    let mut best = slots.len();
    let mut oldest = 0;
    let mut k = 0;
    while k < slots.len() {
        let slot = slots[k];
        if !slot.live {
            return k;
        }
        if best == slots.len() || slot.used < oldest {
            best = k;
            oldest = slot.used;
        }
        k += 1;
    }
    best
}

/// Store the entry for `(subject, producer)` at `version`, replacing the
/// pair's previous entry. `None`: the value costs more than `limit` (nothing
/// changes) or the table has no slots.
pub fn place(
    slots: &mut [Slot],
    subject: u64,
    producer: u64,
    version: u64,
    cost: usize,
    limit: usize,
    now: u64,
) -> Option<usize> {
    if cost > limit {
        return None;
    }
    retire(slots, subject, producer, true, true);
    let at = victim(slots);
    if at >= slots.len() {
        return None;
    }
    slots[at] = Slot {
        live: true,
        subject,
        producer,
        version,
        cost,
        used: now,
    };
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(len: usize) -> alloc::vec::Vec<Slot> {
        alloc::vec![vacant(); len]
    }

    #[test]
    fn only_the_exact_version_is_found() {
        let mut slots = table(4);
        let at = place(&mut slots, 7, 1, 3, 10, 100, 1).unwrap();
        assert_eq!(find(&slots, 7, 1, 3), Some(at));
        assert_eq!(find(&slots, 7, 1, 4), None);
        assert_eq!(find(&slots, 7, 2, 3), None);
        assert_eq!(find(&slots, 8, 1, 3), None);
    }

    #[test]
    fn a_new_version_replaces_the_pairs_entry() {
        let mut slots = table(4);
        place(&mut slots, 7, 1, 3, 10, 100, 1).unwrap();
        place(&mut slots, 7, 2, 3, 10, 100, 2).unwrap();
        place(&mut slots, 7, 1, 4, 10, 100, 3).unwrap();
        assert_eq!(find(&slots, 7, 1, 3), None);
        assert!(find(&slots, 7, 1, 4).is_some());
        assert!(find(&slots, 7, 2, 3).is_some());
        assert_eq!(slots.iter().filter(|slot| slot.live).count(), 2);
    }

    #[test]
    fn retiring_a_subject_or_a_producer_misses_afterwards() {
        let mut slots = table(4);
        place(&mut slots, 7, 1, 3, 10, 100, 1).unwrap();
        place(&mut slots, 7, 2, 3, 10, 100, 2).unwrap();
        place(&mut slots, 8, 1, 3, 10, 100, 3).unwrap();
        retire(&mut slots, 0, 1, false, true);
        assert_eq!(find(&slots, 7, 1, 3), None);
        assert_eq!(find(&slots, 8, 1, 3), None);
        assert!(find(&slots, 7, 2, 3).is_some());
        retire(&mut slots, 7, 0, true, false);
        assert_eq!(find(&slots, 7, 2, 3), None);
    }

    #[test]
    fn a_full_table_gives_up_its_least_recently_used_entry() {
        let mut slots = table(2);
        let first = place(&mut slots, 1, 0, 0, 1, 100, 1).unwrap();
        place(&mut slots, 2, 0, 0, 1, 100, 2).unwrap();
        touch(&mut slots, first, 3);
        place(&mut slots, 3, 0, 0, 1, 100, 4).unwrap();
        assert!(find(&slots, 1, 0, 0).is_some());
        assert_eq!(find(&slots, 2, 0, 0), None);
        assert!(find(&slots, 3, 0, 0).is_some());
    }

    #[test]
    fn a_value_over_the_limit_is_refused_and_changes_nothing() {
        let mut slots = table(2);
        place(&mut slots, 1, 0, 0, 5, 100, 1).unwrap();
        let before = slots.clone();
        assert_eq!(place(&mut slots, 1, 0, 1, 101, 100, 2), None);
        assert!(slots == before);
        assert_eq!(place(&mut [], 1, 0, 0, 1, 100, 1), None);
    }
}
