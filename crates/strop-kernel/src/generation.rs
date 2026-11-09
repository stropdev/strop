//! Generational-arena arithmetic (0056 AR13): a reused slot's generation
//! strictly advances, and a generation that would wrap retires the slot, so
//! a stale key can never alias a new occupant. `strop_core::Arena` consults
//! these exact decisions.
//!
//! Proved (proofs/Kernel/Generation.lean): `next_generation` is `Some
//! (current + 1)` exactly when `current < u32::MAX`; `index_for_len` is
//! `Some(len)` exactly when `len` fits the u32 index space.

/// The next occupant generation for a reused slot, or `None`: retire it.
pub fn next_generation(current: u32) -> Option<u32> {
    if current == u32::MAX {
        None
    } else {
        Some(current + 1)
    }
}

/// A fresh slot's index while the slot count fits the u32 index space — a
/// full index space refuses instead of truncating onto a live slot.
pub fn index_for_len(len: usize) -> Option<u32> {
    if len > u32::MAX as usize {
        None
    } else {
        Some(len as u32)
    }
}
