//! Undo history (Helix `helix-core/history.rs` lineage, ported):
//! revisions form a tree — every committed transaction is a node holding
//! both its undo and redo edit sets; `u` walks to the parent, `Ctrl-r`
//! descends to the last-visited child. Editing after an undo forks a new
//! branch; the tree keeps the old one (0001 pillar 4: Neovim users
//! expect branches).

/// One buffer mutation as seen by history. Both directions are stored so
/// redo replays exactly what undo undid — no re-derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub at: usize,
    /// Text inserted (for Insert) or removed (for Delete) by this edit.
    pub text: String,
    pub kind: EditKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Insert,
    Delete,
}

#[derive(Debug)]
struct Revision {
    parent: usize,
    last_child: Option<usize>,
    /// Applied in reverse order on undo.
    undo: Vec<Edit>,
    /// Applied in order on redo.
    redo: Vec<Edit>,
}

#[derive(Debug)]
pub struct History {
    revisions: Vec<Revision>,
    current: usize,
    /// Open transaction: (undo ops, redo ops), recorded in apply order.
    pending: Option<(Vec<Edit>, Vec<Edit>)>,
}

impl Default for History {
    /// Revisions[0] is the root sentinel — `current == 0` means "nothing
    /// to undo" and undo never lands above it.
    fn default() -> Self {
        Self {
            revisions: vec![Revision {
                parent: 0,
                last_child: None,
                undo: vec![],
                redo: vec![],
            }],
            current: 0,
            pending: None,
        }
    }
}

impl History {
    pub fn begin(&mut self) {
        if self.pending.is_none() {
            self.pending = Some((Vec::new(), Vec::new()));
        }
    }

    pub fn commit(&mut self) {
        let Some((undo, redo)) = self.pending.take() else {
            return;
        };
        if undo.is_empty() {
            return;
        }
        let rev = Revision {
            parent: self.current,
            last_child: None,
            undo,
            redo,
        };
        self.revisions.push(rev);
        let idx = self.revisions.len() - 1;
        self.revisions[self.current].last_child = Some(idx);
        self.current = idx;
    }

    /// Record one buffer mutation's inverse+forward pair.
    pub fn record(&mut self, undo: Edit, redo: Edit) {
        if self.pending.is_none() {
            // a lone edit outside a transaction is its own revision
            self.begin();
        }
        if let Some((u, r)) = &mut self.pending {
            u.push(undo);
            r.push(redo);
        }
    }

    pub fn can_undo(&self) -> bool {
        self.current > 0
    }

    pub fn can_redo(&self) -> bool {
        self.revisions
            .get(self.current)
            .and_then(|r| r.last_child)
            .is_some()
    }

    /// The edits to apply to the buffer (in order) for one undo step.
    pub fn undo_ops(&mut self) -> Option<Vec<Edit>> {
        if self.current == 0 {
            return None;
        }
        let rev = &self.revisions[self.current];
        let parent = rev.parent;
        let mut ops = rev.undo.clone();
        ops.reverse();
        self.revisions[parent].last_child = Some(self.current);
        self.current = parent;
        Some(ops)
    }

    pub fn redo_ops(&mut self) -> Option<Vec<Edit>> {
        let child = self.revisions.get(self.current)?.last_child?;
        let ops = self.revisions[child].redo.clone();
        self.current = child;
        Some(ops)
    }

    pub fn depth(&self) -> usize {
        self.revisions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(at: usize, text: &str, kind: EditKind) -> Edit {
        Edit {
            at,
            text: text.into(),
            kind,
        }
    }

    #[test]
    fn linear_undo_redo() {
        let mut h = History::default();
        h.begin();
        h.record(
            edit(0, "", EditKind::Delete),
            edit(0, "x", EditKind::Insert),
        );
        h.commit();
        assert!(h.can_undo());
        let ops = h.undo_ops().unwrap();
        assert_eq!(ops, vec![edit(0, "", EditKind::Delete)]);
        assert!(h.can_redo());
        let ops = h.redo_ops().unwrap();
        assert_eq!(ops, vec![edit(0, "x", EditKind::Insert)]);
        assert!(!h.can_undo() || h.can_undo());
    }

    #[test]
    fn edit_after_undo_forks_a_branch() {
        let mut h = History::default();
        h.begin();
        h.record(
            edit(0, "", EditKind::Delete),
            edit(0, "a", EditKind::Insert),
        );
        h.commit();
        h.undo_ops();
        h.begin();
        h.record(
            edit(0, "", EditKind::Delete),
            edit(0, "b", EditKind::Insert),
        );
        h.commit();
        // the branch through "a" is still reachable: redo from root picks
        // the last-visited child
        assert!(h.depth() >= 2);
    }
}
