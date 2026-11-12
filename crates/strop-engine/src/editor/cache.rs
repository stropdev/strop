//! A keyed, bounded value store over the proved core (0099 §3).
//!
//! `strop_kernel::cache` decides everything: which entry may be served, what
//! an invalidation removes, which slot a new entry takes. This file only
//! keeps the values beside the slots — `values[i]` is the value placed with
//! `slots[i]` — and drops a value the moment its slot dies. It is the only
//! caller of the kernel module; flows hold a `Cache<V>` and never look at a
//! slot.
//!
//! One owner, on the event-loop thread: no locks, nothing here blocks.

use strop_kernel::cache::{self, Slot};

/// What an entry is about, who computed it, and the state of the subject it
/// was computed for. A lookup matches on all three or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub subject: u64,
    pub producer: u64,
    pub version: u64,
}

pub(crate) struct Cache<V> {
    slots: Vec<Slot>,
    values: Vec<Option<V>>,
    /// The largest cost one entry may have.
    limit: usize,
    /// Logical time: one tick per use, so eviction order is a function of
    /// what was asked, never of a wall clock.
    clock: u64,
}

impl<V> Cache<V> {
    /// A table of `entries` slots, each value costing at most `limit`: the
    /// store never holds more than `entries × limit`.
    pub fn new(entries: usize, limit: usize) -> Self {
        Self {
            slots: vec![cache::vacant(); entries],
            values: std::iter::repeat_with(|| None).take(entries).collect(),
            limit,
            clock: 0,
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    /// The value for exactly this key. A hit counts as a use.
    pub fn get(&mut self, key: Key) -> Option<&V> {
        let at = cache::find(&self.slots, key.subject, key.producer, key.version)?;
        let now = self.tick();
        cache::touch(&mut self.slots, at, now);
        self.values[at].as_ref()
    }

    /// Whether `get` would hit, without counting as a use.
    pub fn contains(&self, key: Key) -> bool {
        cache::find(&self.slots, key.subject, key.producer, key.version).is_some()
    }

    /// Store `value` under `key`, replacing the (subject, producer) pair's
    /// previous entry. `false`: the value costs more than the limit and
    /// nothing changed.
    pub fn put(&mut self, key: Key, cost: usize, value: V) -> bool {
        let now = self.tick();
        let placed = cache::place(
            &mut self.slots,
            key.subject,
            key.producer,
            key.version,
            cost,
            self.limit,
            now,
        );
        let Some(at) = placed else {
            return false;
        };
        self.values[at] = Some(value);
        self.sweep();
        true
    }

    /// Drop every entry about `subject`, whoever computed it.
    pub fn retire_subject(&mut self, subject: u64) {
        cache::retire(&mut self.slots, subject, 0, true, false);
        self.sweep();
    }

    /// Drop every entry `producer` computed, whatever it is about.
    pub fn retire_producer(&mut self, producer: u64) {
        cache::retire(&mut self.slots, 0, producer, false, true);
        self.sweep();
    }

    /// A dead slot keeps no value: retirement and replacement free memory,
    /// they do not only hide it.
    fn sweep(&mut self) {
        for (slot, value) in self.slots.iter().zip(self.values.iter_mut()) {
            if !slot.live {
                *value = None;
            }
        }
        debug_assert!(
            self.slots
                .iter()
                .zip(&self.values)
                .all(|(slot, value)| slot.live == value.is_some()),
            "a live slot has its value and a dead one has none"
        );
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.values.iter().filter(|value| value.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The obvious cache: a list of entries, one per (subject, producer),
    /// least recently used first out. `Cache` must answer like it.
    struct Reference {
        entries: Vec<(Key, u32, u64)>,
        capacity: usize,
        limit: usize,
        clock: u64,
    }

    impl Reference {
        fn get(&mut self, key: Key) -> Option<u32> {
            self.clock += 1;
            let entry = self.entries.iter_mut().find(|entry| entry.0 == key)?;
            entry.2 = self.clock;
            Some(entry.1)
        }
        fn put(&mut self, key: Key, cost: usize, value: u32) -> bool {
            self.clock += 1;
            if cost > self.limit {
                return false;
            }
            self.entries
                .retain(|entry| (entry.0.subject, entry.0.producer) != (key.subject, key.producer));
            if self.entries.len() == self.capacity {
                let oldest = self
                    .entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.2)
                    .map(|(index, _)| index)
                    .unwrap();
                self.entries.remove(oldest);
            }
            self.entries.push((key, value, self.clock));
            true
        }
    }

    /// xorshift64: the sequence is a function of the seed, nothing else.
    fn next(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[test]
    fn answers_like_the_obvious_cache_over_random_operations() {
        for seed in 1..=64u64 {
            let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut cache: Cache<u32> = Cache::new(4, 10);
            let mut reference = Reference {
                entries: Vec::new(),
                capacity: 4,
                limit: 10,
                clock: 0,
            };
            for step in 0..400u32 {
                let roll = next(&mut state);
                let key = Key {
                    subject: (roll >> 8) % 4,
                    producer: (roll >> 16) % 2,
                    version: (roll >> 24) % 3,
                };
                match roll % 8 {
                    0..=2 => {
                        // both clocks only order uses; neither needs the
                        // other's values
                        let expected = reference.get(key);
                        assert_eq!(cache.get(key).copied(), expected, "seed {seed} step {step}");
                    }
                    3..=5 => {
                        let cost = ((roll >> 32) % 12) as usize;
                        let accepted = reference.put(key, cost, step);
                        assert_eq!(cache.put(key, cost, step), accepted);
                    }
                    6 => {
                        reference
                            .entries
                            .retain(|entry| entry.0.subject != key.subject);
                        cache.retire_subject(key.subject);
                    }
                    _ => {
                        reference
                            .entries
                            .retain(|entry| entry.0.producer != key.producer);
                        cache.retire_producer(key.producer);
                    }
                }
                assert_eq!(
                    cache.len(),
                    reference.entries.len(),
                    "seed {seed} step {step}"
                );
                assert!(cache.len() <= 4, "the table never grows");
                for entry in &reference.entries {
                    assert!(cache.contains(entry.0), "seed {seed} step {step}");
                }
            }
        }
    }

    #[test]
    fn a_retired_or_replaced_value_is_dropped_not_hidden() {
        let alive = std::rc::Rc::new(());
        let mut cache: Cache<std::rc::Rc<()>> = Cache::new(2, 10);
        let key = |subject, version| Key {
            subject,
            producer: 0,
            version,
        };
        assert!(cache.put(key(1, 0), 1, alive.clone()));
        assert!(cache.put(key(2, 0), 1, alive.clone()));
        assert_eq!(std::rc::Rc::strong_count(&alive), 3);
        // a new version replaces the pair's value
        assert!(cache.put(key(1, 1), 1, alive.clone()));
        assert_eq!(std::rc::Rc::strong_count(&alive), 3);
        // a full table evicts
        assert!(cache.put(key(3, 0), 1, alive.clone()));
        assert_eq!(std::rc::Rc::strong_count(&alive), 3);
        cache.retire_subject(3);
        assert_eq!(std::rc::Rc::strong_count(&alive), 2);
        cache.retire_producer(0);
        assert_eq!(std::rc::Rc::strong_count(&alive), 1);
        assert!(!cache.put(key(1, 2), 11, alive.clone()), "over the limit");
        assert_eq!(std::rc::Rc::strong_count(&alive), 1);
    }
}
