//! Deterministic arena seeds (R11 forensic replay): a serialized image of
//! slots, generations and the free list, so a replayed editor addresses
//! documents by the exact identities the live session used.
use serde::{Deserialize, Serialize};

use super::{Arena, Slot};

/// Per-slot `(generation, value)` plus the free list. Empty slots keep
/// their generation: reuse bumps it, and a replay that skipped them would
/// hand out different ids for later inserts. An empty slot ABSENT from
/// the free list is retired (0056 AR13): its generation wrapped out, so
/// it never hands out an id again — retirement must survive the round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArenaSeed<T> {
    pub slots: Vec<(u32, Option<T>)>,
    pub free: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArenaSeedError {
    #[error("too many arena slots")]
    TooManySlots,
    #[error("free slot outside arena")]
    FreeOutOfBounds,
    #[error("invalid or repeated free slot")]
    InvalidFreeSlot,
}

impl<K, T> Arena<K, T> {
    /// Capture the arena, mapping every live value through `f`.
    pub fn seed_with<U>(&self, mut f: impl FnMut(&T) -> U) -> ArenaSeed<U> {
        ArenaSeed {
            slots: self
                .slots
                .iter()
                .map(|slot| (slot.generation, slot.value.as_ref().map(&mut f)))
                .collect(),
            free: self.free.clone(),
        }
    }

    /// Rebuild an arena. The free list must name only empty slots, each
    /// at most once; a seed that disagrees is rejected, never coerced
    /// into a plausible-but-wrong identity layout. Empty slots missing
    /// from the free list are retired identities, not corruption.
    pub fn from_seed(seed: ArenaSeed<T>) -> Result<Self, ArenaSeedError> {
        if seed.slots.len() > u32::MAX as usize {
            return Err(ArenaSeedError::TooManySlots);
        }
        let mut free = vec![false; seed.slots.len()];
        for &index in &seed.free {
            let Some(seen) = free.get_mut(index as usize) else {
                return Err(ArenaSeedError::FreeOutOfBounds);
            };
            if *seen || seed.slots[index as usize].1.is_some() {
                return Err(ArenaSeedError::InvalidFreeSlot);
            }
            *seen = true;
        }
        Ok(Self {
            slots: seed
                .slots
                .into_iter()
                .map(|(generation, value)| Slot { generation, value })
                .collect(),
            free: seed.free,
            _kind: std::marker::PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Arena, DocumentKind};
    use super::{ArenaSeed, ArenaSeedError};

    #[test]
    fn seed_round_trips_ids_free_slots_and_generations() {
        let mut arena: Arena<DocumentKind, String> = Arena::default();
        let a = arena.try_insert("a".into()).unwrap();
        let b = arena.try_insert("b".into()).unwrap();
        arena.remove(a);
        let seed = arena.seed_with(|value| value.clone());
        let mut rebuilt = Arena::from_seed(seed).unwrap();
        assert_eq!(rebuilt.get(a), None, "freed id stays dead");
        assert_eq!(rebuilt.get(b).map(String::as_str), Some("b"));
        let c = rebuilt.try_insert("c".into()).unwrap();
        assert_eq!(c.index(), a.index(), "free slot is reused");
        assert_eq!(c.generation(), a.generation() + 1);
    }

    #[test]
    fn corrupt_seeds_are_rejected_not_repaired() {
        let mut arena: Arena<DocumentKind, ()> = Arena::default();
        let a = arena.try_insert(()).unwrap();
        let seed = arena.seed_with(|&()| ());
        let ArenaSeed { slots, .. } = seed;
        // A free entry naming an occupied slot.
        let corrupt = ArenaSeed {
            slots: slots.clone(),
            free: vec![a.index() as u32],
        };
        assert!(matches!(
            Arena::<DocumentKind, _>::from_seed(corrupt),
            Err(ArenaSeedError::InvalidFreeSlot)
        ));
        // An empty slot absent from the free list is a retired identity,
        // not corruption: it rebuilds and is never reused (0056 AR13).
        let retired = ArenaSeed {
            slots: vec![(u32::MAX, None)],
            free: Vec::new(),
        };
        let mut rebuilt = Arena::<DocumentKind, ()>::from_seed(retired).unwrap();
        let id = rebuilt.try_insert(()).unwrap();
        assert_eq!(id.index(), 1, "retired slot stays dead");
        let corrupt = ArenaSeed {
            slots,
            free: vec![9],
        };
        assert!(matches!(
            Arena::<DocumentKind, _>::from_seed(corrupt),
            Err(ArenaSeedError::FreeOutOfBounds)
        ));
    }
}
