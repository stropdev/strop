//! Stable identities and typed coordinates (0014 wave 2).
//!
//! Two families:
//! - **IDs**: generational arena keys (`DocumentId`, `ViewId`, …). A
//!   closed document's id fails lookup instead of silently resolving to
//!   whatever moved into its old vector slot.
//! - **Coordinates**: newtypes so byte offsets, line indexes, and the
//!   three column kinds (bytes / UTF-16 / display) can't mix silently.

/// A generational-arena key: the index names the slot, the generation
/// names the occupant. Stale keys fail lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id<K> {
    index: u32,
    generation: u32,
    _kind: std::marker::PhantomData<K>,
}

impl<K> Id<K> {
    pub fn index(self) -> usize {
        self.index as usize
    }
}

/// Marker kinds for the arena's identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentKind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewKind;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneKind;

pub type DocumentId = Id<DocumentKind>;
pub type ViewId = Id<ViewKind>;
pub type PaneId = Id<PaneKind>;

/// A minimal generational arena (house rule: 40 boring lines beat a
/// dependency). Slots are reused; each reuse bumps the generation.
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
    pub fn insert(&mut self, value: T) -> Id<K> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.generation += 1;
            slot.value = Some(value);
            return Id {
                index,
                generation: slot.generation,
                _kind: std::marker::PhantomData,
            };
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        });
        Id {
            index,
            generation: 0,
            _kind: std::marker::PhantomData,
        }
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

/// Byte offset into a document's UTF-8 text. The storage coordinate.
pub type ByteOffset = usize;
/// Zero-based line index.
pub type LineIndex = usize;

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
        let one = a.insert("one".into());
        let two = a.insert("two".into());
        assert_eq!(a.get(one).map(String::as_str), Some("one"));
        a.remove(one);
        assert_eq!(a.get(one), None, "removed");
        let three = a.insert("three".into()); // reuses the slot
        assert_eq!(a.get(one), None, "stale generation must not resolve");
        assert_eq!(a.get(three).map(String::as_str), Some("three"));
        assert_eq!(a.get(two).map(String::as_str), Some("two"));
        assert_eq!(a.len(), 2);
    }
}
