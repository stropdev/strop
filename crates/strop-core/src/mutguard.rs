//! Mutation authority: who may apply a text mutation, who may save, and
//! when a confirmed save retires dirty state. `buffer/mutation.rs` and
//! `buffer/io.rs` call these decisions. The refusal order (readonly before
//! stale) is the error precedence callers observe.

/// The batch-admission decision as data; the guard sites map it onto
/// the typed `EditError` callers already see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAdmission {
    /// Writable buffer and the caller's base is the live revision.
    Admitted,
    /// A readonly buffer refuses before any revision question.
    Readonly,
    /// The caller's base is not the live revision.
    Stale,
}

/// May this batch apply? `Buffer::prepare_replacements` and
/// `Buffer::apply_prepared` consult this exact decision; the refusal
/// order (readonly before stale) is the error precedence callers
/// observe.
pub fn classify_edit(readonly: bool, base: u64, live: u64) -> EditAdmission {
    if readonly {
        EditAdmission::Readonly
    } else if base != live {
        EditAdmission::Stale
    } else {
        EditAdmission::Admitted
    }
}

/// Write authority alone, for the streaming edit leases where no base
/// revision is in play: a readonly buffer refuses before text moves.
pub fn writable(readonly: bool) -> bool {
    !readonly
}

/// May this save proceed? `:w!` is the explicit authority override; a
/// readonly buffer without it refuses before any path is resolved.
pub fn save_admitted(readonly: bool, force: bool) -> bool {
    !readonly || force
}

/// Does a confirmed save acknowledge the LIVE revision? Only then does
/// it retire dirty text; a receipt for an older snapshot leaves edits
/// made after that snapshot dirty (the retirement rule of 0056 AR04 §5
/// at the buffer clock).
pub fn save_ack_is_current(saved: u64, live: u64) -> bool {
    saved == live
}
