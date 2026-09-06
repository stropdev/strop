//! The transaction gateway (0024): one validated entry point for
//! document mutation.
//!
//! Two shapes share one commit:
//!
//! - `apply(doc, base, changeset)` — atomic validated edits (replace,
//!   git discard, future LSP edits): base-revision check, readonly
//!   check, range validity, typed errors.
//! - `tx_begin`/`tx_commit` — multi-step sessions (typing, operators):
//!   the commit side effects (history, anchors, tree bridge, clock)
//!   live here, once.
//!
//! The rule this module exists to enforce: **no feature edits a rope
//! directly and forgets the anchors, the tree, the clock, or the undo
//! record.** Buffer::insert/delete are mechanics; the gateway is the
//! contract.

use strop_core::history::Edit;

/// One batch of edits, applied in order (0024). `undo_open` keeps the
/// undo group open — an insert session is one unit across many applies.
#[derive(Debug, Clone)]
pub struct ChangeSet {
    pub edits: Vec<Edit>,
    pub undo_open: bool,
}

/// What the gateway refuses, typed (never a panic, never a silent
/// no-op).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The document is gone.
    NoDocument,
    /// Readonly content (a surface, a view, an out-of-workspace jump).
    ReadOnly,
    /// The base revision the caller computed against has moved on.
    StaleRevision { expected: u64, found: u64 },
    /// An edit range falls outside the document or off a char boundary.
    InvalidRange,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::NoDocument => write!(f, "no such document"),
            ApplyError::ReadOnly => write!(f, "readonly buffer"),
            ApplyError::StaleRevision { expected, found } => {
                write!(f, "stale revision: expected {expected}, found {found}")
            }
            ApplyError::InvalidRange => write!(f, "invalid edit range"),
        }
    }
}

/// A committed change: the document's new text clock value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Committed {
    pub revision: u64,
}

impl super::Editor {
    /// The atomic path: validate, apply, commit as one undo unit.
    pub(crate) fn apply(
        &mut self,
        doc: strop_core::id::DocumentId,
        base: u64,
        cs: ChangeSet,
    ) -> Result<Committed, ApplyError> {
        let Some(d) = self.docs.get(doc) else {
            return Err(ApplyError::NoDocument);
        };
        if d.buf.readonly {
            return Err(ApplyError::ReadOnly);
        }
        let found = d.buf.epoch;
        if found != base {
            return Err(ApplyError::StaleRevision {
                expected: base,
                found,
            });
        }
        // every edit must sit inside the document on char boundaries
        for e in &cs.edits {
            let end = match e.kind {
                strop_core::history::EditKind::Delete => e.at + e.text.len(),
                strop_core::history::EditKind::Insert => e.at,
            };
            if end > d.buf.len_bytes() || !d.buf.is_boundary(e.at) || !d.buf.is_boundary(end) {
                return Err(ApplyError::InvalidRange);
            }
        }
        // apply in reverse position order: earlier offsets stay valid.
        // the transaction side effects (anchors, tree, clock) live in
        // tx_commit and track the CURRENT document — drive the view to
        // the target doc so a cross-document apply is never misplaced
        let saved = self.current();
        self.view_mut().doc = doc;
        let mut ordered = cs.edits.clone();
        ordered.sort_by_key(|e| std::cmp::Reverse(e.at));
        self.tx_begin();
        {
            let buf = &mut self.docs.get_mut(doc).unwrap().buf;
            for e in &ordered {
                match e.kind {
                    strop_core::history::EditKind::Insert => buf.insert(e.at, &e.text),
                    strop_core::history::EditKind::Delete => {
                        let _ = buf.delete(strop_core::Range::charwise(e.at, e.at + e.text.len()));
                    }
                }
            }
        }
        // an open group means the session commits later (typing); the
        // field is the contract, not decoration
        if !cs.undo_open {
            self.tx_commit();
        }
        self.view_mut().doc = saved;
        Ok(Committed {
            revision: self.docs.get(doc).map(|d| d.buf.epoch).unwrap_or(found + 1),
        })
    }
}
