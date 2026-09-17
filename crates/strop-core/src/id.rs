//! Stable identities and typed coordinates (0014 wave 2).
//!
//! Two families:
//! - **IDs**: generational arena keys (`DocumentId`, `ViewId`, …). A
//!   closed document's id fails lookup instead of silently resolving to
//!   whatever moved into its old vector slot.
//! - **Coordinates**: newtypes so byte offsets, line indexes, and the
//!   three column kinds (bytes / UTF-16 / display) can't mix silently.

mod seed;
pub use seed::{ArenaSeed, ArenaSeedError};
use vstd::prelude::*;

verus! {

/// The slot-reuse decision, mathematically: a reused slot's occupant
/// generation strictly advances — a generation that would wrap retires
/// the slot for good, so a stale key can never alias a new occupant
/// (0056 AR13).
pub open spec fn generation_advances(current: int, next: int) -> bool {
    next == current + 1
}

/// The next occupant generation for a reused slot, or retirement.
/// `Arena::try_insert` consults this exact decision; a retired slot is
/// dropped from the free list and never hands out an id again.
pub fn next_generation(current: u32) -> (next: Option<u32>)
    ensures
        next.is_some() == (current < u32::MAX),
        next.is_some() ==> generation_advances(current as int, next.unwrap() as int),
        next.is_some() ==> next.unwrap() != current,
{
    if current == u32::MAX {
        None
    } else {
        Some(current + 1)
    }
}

/// The index-space decision: a fresh slot exists only while the slot
/// count fits the u32 index space — a full index space refuses instead
/// of truncating `slots.len()` onto a live slot (0056 AR13).
/// `Arena::try_insert` consults this exact decision.
pub fn index_for_len(len: usize) -> (index: Option<u32>)
    ensures
        index.is_some() == (len <= u32::MAX as usize),
        index.is_some() ==> index.unwrap() as usize == len,
{
    if len > u32::MAX as usize {
        None
    } else {
        Some(len as u32)
    }
}

/// Reuse never aliases: the generation a slot hands out next differs
/// from every generation it handed out before.
proof fn reused_slot_never_aliases(current: int, next: int)
    requires
        generation_advances(current, next),
    ensures
        next != current,
        next > current,
{
}

}

/// A generational-arena key: the index names the slot, the generation
/// names the occupant. Stale keys fail lookup.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(bound = "")]
pub struct Id<K> {
    #[serde(rename = "slot", alias = "index")]
    index: u32,
    generation: u32,
    #[serde(skip)]
    _kind: std::marker::PhantomData<K>,
}

impl<K> Id<K> {
    pub fn index(self) -> usize {
        self.index as usize
    }

    pub fn generation(self) -> u32 {
        self.generation
    }
}

/// Marker kinds for the arena's identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentKind;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewKind;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneKind;
/// Registry keys for bound workspace contexts (0042 slice 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceKind;

pub type DocumentId = Id<DocumentKind>;
pub type ViewId = Id<ViewKind>;
pub type PaneId = Id<PaneKind>;
pub type WorkspaceId = Id<WorkspaceKind>;

/// A minimal generational arena (house rule: 40 boring lines beat a
/// dependency). Slots are reused; each reuse bumps the generation.
/// Insertion is checked (0056 AR13): a slot whose generation would wrap
/// is retired — never reused — so a stale key can never alias a new
/// occupant, and a full index space refuses instead of truncating
/// `slots.len()` onto a live slot.
pub struct Arena<K, T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    _kind: std::marker::PhantomData<K>,
}

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Identity allocation failed (0056 AR13): every slot is live or
/// retired and the `u32` index space is full. Nothing was inserted and
/// every existing id still resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("arena identity space exhausted")]
pub struct ArenaExhausted;

impl<K, T> Default for Arena<K, T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            _kind: std::marker::PhantomData,
        }
    }
}

impl<K, T> Arena<K, T> {
    /// Checked insertion: reuses a free slot only when its generation
    /// can advance without wrapping; retired (wrap-risk) slots are
    /// dropped from the free list for good. Fails only when no slot is
    /// reusable and the index space itself is full — the arena is left
    /// unchanged, so an in-flight operation keeps every existing
    /// document/edit.
    pub fn try_insert(&mut self, value: T) -> Result<Id<K>, ArenaExhausted> {
        while let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            // The verified retirement rule (0057 VF18): a generation that
            // cannot advance without wrapping retires the slot for good.
            let Some(generation) = next_generation(slot.generation) else {
                continue; // retire: this slot never hands out an id again
            };
            slot.generation = generation;
            slot.value = Some(value);
            return Ok(Id {
                index,
                generation,
                _kind: std::marker::PhantomData,
            });
        }
        // The verified index-space rule (0057 VF18): full space refuses
        // instead of truncating `slots.len()` onto a live slot.
        let Some(index) = index_for_len(self.slots.len()) else {
            return Err(ArenaExhausted);
        };
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        });
        Ok(Id {
            index,
            generation: 0,
            _kind: std::marker::PhantomData,
        })
    }

    /// How many more inserts this arena can serve: reusable free slots
    /// plus the remaining index space. Multi-document operations
    /// preflight against this instead of failing halfway through.
    pub fn insert_capacity(&self) -> u64 {
        let reusable = self
            .free
            .iter()
            .filter(|&&index| self.slots[index as usize].generation < u32::MAX)
            .count() as u64;
        reusable + (u64::from(u32::MAX) - self.slots.len() as u64)
    }

    /// None for a stale id — never the wrong document.
    pub fn get(&self, id: Id<K>) -> Option<&T> {
        self.slots
            .get(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.value.as_ref())
    }

    pub fn get_mut(&mut self, id: Id<K>) -> Option<&mut T> {
        self.slots
            .get_mut(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.value.as_mut())
    }

    /// Remove and return the value; stale ids get None.
    pub fn remove(&mut self, id: Id<K>) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        self.free.push(id.index);
        Some(value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Id<K>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.value.as_ref().map(|v| {
                (
                    Id {
                        index: i as u32,
                        generation: s.generation,
                        _kind: std::marker::PhantomData,
                    },
                    v,
                )
            })
        })
    }

    /// Mutable values without copying keys or exposing vacant slots.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|slot| slot.value.as_mut())
    }

    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.value.is_some()).count()
    }

    /// Drop every live value (slot reuse still bumps generations).
    pub fn clear(&mut self) {
        let live: Vec<u32> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.value.is_some())
            .map(|(i, _)| i as u32)
            .collect();
        for s in &mut self.slots {
            s.value = None;
        }
        self.free.extend(live);
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A coordinate newtype: the inner value is bytes (ByteOffset), lines
/// (LineIndex), or columns in a named unit (ByteColumn/Utf16Column/
/// DisplayColumn). Copy, ordered, hashable; arithmetic is explicit.
macro_rules! coordinate {
    ($name:ident, $unit:literal) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Default,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[repr(transparent)]
        #[serde(transparent)]
        pub struct $name(usize);

        impl $name {
            #[inline]
            pub fn new(v: usize) -> Self {
                Self(v)
            }
            /// Raw units (bytes / lines / columns per the type) — escape
            /// hatch for arithmetic; naming the unit is the point.
            #[inline]
            pub fn get(self) -> usize {
                self.0
            }
            #[inline]
            pub fn saturating_sub(self, n: usize) -> Self {
                Self(self.0.saturating_sub(n))
            }
        }

        impl std::ops::AddAssign<usize> for $name {
            #[inline]
            fn add_assign(&mut self, n: usize) {
                self.0 += n;
            }
        }
        impl std::ops::SubAssign<usize> for $name {
            #[inline]
            fn sub_assign(&mut self, n: usize) {
                self.0 -= n;
            }
        }
        /// Raw-unit comparison: `offset > 0` reads naturally; the type
        /// system still stops offset-vs-line mixes (the bug class).
        impl PartialEq<usize> for $name {
            #[inline]
            fn eq(&self, other: &usize) -> bool {
                self.0 == *other
            }
        }
        impl PartialOrd<usize> for $name {
            #[inline]
            fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
                self.0.partial_cmp(other)
            }
        }
        impl std::ops::Add<usize> for $name {
            type Output = $name;
            #[inline]
            fn add(self, n: usize) -> $name {
                $name(self.0 + n)
            }
        }
        impl std::ops::Sub<usize> for $name {
            type Output = $name;
            #[inline]
            fn sub(self, n: usize) -> $name {
                $name(self.0 - n)
            }
        }
        impl std::ops::Sub<$name> for $name {
            type Output = usize; // a length
            #[inline]
            fn sub(self, other: $name) -> usize {
                self.0 - other.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{} {}", self.0, $unit)
            }
        }
        impl From<$name> for usize {
            #[inline]
            fn from(v: $name) -> usize {
                v.0
            }
        }
        impl From<usize> for $name {
            #[inline]
            fn from(v: usize) -> $name {
                $name(v)
            }
        }
    };
}

coordinate!(ByteOffset, "B");
coordinate!(LineIndex, "L");
coordinate!(ByteColumn, "col:B");
coordinate!(Utf16Column, "col:u16");
coordinate!(DisplayColumn, "col:dsp");

/// A document's content clock, not an LSP version, request ID or history node.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct BufferRevision(u64);

impl BufferRevision {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl std::fmt::Display for BufferRevision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<u64> for BufferRevision {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

// NOTE: full newtype wrappers (ByteOffset(usize), Utf16Column(u32), …)
// are the target; the pragmatic cutover is to name the domains first
// (this module + the conversion functions) and tighten the Buffer API
// per call-site cluster as waves 2–4 touch them. A big-bang usize→newtype
// rewrite of every arithmetic site would be unreviewable.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_ids_fail_lookup() {
        let mut a: Arena<DocumentKind, String> = Arena::default();
        let one = a.try_insert("one".into()).unwrap();
        let two = a.try_insert("two".into()).unwrap();
        assert_eq!(a.get(one).map(String::as_str), Some("one"));
        a.remove(one);
        assert_eq!(a.get(one), None, "removed");
        let three = a.try_insert("three".into()).unwrap(); // reuses the slot
        assert_eq!(a.get(one), None, "stale generation must not resolve");
        assert_eq!(a.get(three).map(String::as_str), Some("three"));
        assert_eq!(a.get(two).map(String::as_str), Some("two"));
        assert_eq!(a.len(), 2);
    }

    /// Seeded at the generation boundary (0056 AR13): a slot at
    /// u32::MAX-1 hands out one last id, then retires — the generation
    /// never wraps onto a stale key.
    #[test]
    fn generation_wrap_retires_the_slot() {
        let mut a: Arena<DocumentKind, String> = Arena::from_seed(ArenaSeed {
            slots: vec![(u32::MAX - 1, None)],
            free: vec![0],
        })
        .unwrap();
        assert_eq!(a.insert_capacity(), u64::from(u32::MAX));
        let last = a.try_insert("last".into()).unwrap();
        assert_eq!((last.index(), last.generation()), (0, u32::MAX));
        a.remove(last);
        // The slot is now at u32::MAX: reuse would wrap onto `last`.
        let fresh = a.try_insert("fresh".into()).unwrap();
        assert_eq!((fresh.index(), fresh.generation()), (1, 0));
        assert_eq!(a.get(last), None, "a wrapped generation would alias");
        assert_eq!(a.get(fresh).map(String::as_str), Some("fresh"));
        // Retirement is permanent: the freed fresh slot is reused, the
        // retired slot stays dead, and capacity reflects both facts.
        a.remove(fresh);
        let again = a.try_insert("again".into()).unwrap();
        assert_eq!((again.index(), again.generation()), (1, 1));
        assert_eq!(a.insert_capacity(), u64::from(u32::MAX) - 2);
    }

    /// Retirement survives a seed round-trip: the rebuilt arena never
    /// revives the dead slot (0056 AR13/R11).
    #[test]
    fn retired_slots_stay_dead_across_seeds() {
        let mut a: Arena<DocumentKind, String> = Arena::from_seed(ArenaSeed {
            slots: vec![(u32::MAX, None)],
            free: vec![0],
        })
        .unwrap();
        let live = a.try_insert("live".into()).unwrap();
        assert_eq!(live.index(), 1, "the maxed slot was retired on reuse");
        let seed = a.seed_with(|value| value.clone());
        let mut rebuilt = Arena::from_seed(seed).unwrap();
        let after = rebuilt.try_insert("after".into()).unwrap();
        assert_eq!(after.index(), 2, "retirement is part of the seed");
        assert_eq!(rebuilt.get(live).map(String::as_str), Some("live"));
    }
}
