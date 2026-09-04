//! One selection model for everything (0014 wave 2): normal mode is a
//! collapsed selection, visual mode is a stretched one, multicursor is
//! several. Cursor / anchor / extra-cursors used to be three fields that
//! could disagree; the set owns them with the invariants in one place.

/// One selection: the anchor sits, the head moves. Collapsed (equal) is
/// a cursor. Byte offsets, always char boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn cursor(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    pub fn collapsed(self) -> bool {
        self.anchor == self.head
    }

    /// (start, end) ordered.
    pub fn range(self) -> (usize, usize) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }
}

/// The editor's selections: a primary plus zero or more extras.
/// Invariants (enforced by `normalize`): extras sorted, deduped, none
/// equal to the primary's head.
#[derive(Debug, Clone)]
pub struct SelectionSet {
    primary: Selection,
    extras: Vec<Selection>,
}

impl Default for SelectionSet {
    fn default() -> Self {
        Self {
            primary: Selection::cursor(0),
            extras: Vec::new(),
        }
    }
}

impl SelectionSet {
    pub fn primary(&self) -> Selection {
        self.primary
    }

    /// Every head, primary first (0013 §3 cascade order).
    pub fn heads(&self) -> Vec<usize> {
        std::iter::once(self.primary.head)
            .chain(self.extras.iter().map(|s| s.head))
            .collect()
    }

    pub fn extra_heads(&self) -> &[Selection] {
        &self.extras
    }

    pub fn count(&self) -> usize {
        1 + self.extras.len()
    }

    /// Move the primary head (motions, edits).
    pub fn set_head(&mut self, head: usize) {
        self.primary.head = head;
    }

    /// Move head and anchor together (leaving visual mode, plain moves).
    pub fn collapse_primary(&mut self, at: usize) {
        self.primary = Selection::cursor(at);
    }

    /// Enter/extend visual: the anchor stays, the head walks.
    pub fn stretch_primary(&mut self, anchor: usize, head: usize) {
        self.primary = Selection { anchor, head };
    }

    /// `Q`: drop the extra under the primary, else plant one there.
    pub fn toggle_extra(&mut self) {
        if let Some(i) = self.extras.iter().position(|s| s.head == self.primary.head) {
            self.extras.remove(i);
        } else {
            self.extras.push(self.primary);
        }
        self.normalize();
    }

    /// Plant an extra cursor (both ends at `at`). Never on the
    /// primary's head — `Space c` and match-planting skip that spot;
    /// stacking on the primary is `Q`'s job (toggle_extra).
    pub fn plant_extra(&mut self, at: usize) {
        if at != self.primary.head && !self.extras.iter().any(|s| s.head == at) {
            self.extras.push(Selection::cursor(at));
        }
        self.normalize();
    }

    /// Sorted, deduped. An extra MAY sit on the primary's head — `Q`
    /// plants exactly there, then a motion walks them apart (0013).
    pub fn normalize(&mut self) {
        self.extras.sort_by_key(|s| s.head);
        self.extras.dedup();
    }

    /// Esc: extras die, primary stays.
    pub fn collapse_extras(&mut self) {
        self.extras.clear();
    }

    /// Replace the extras wholesale — the motion cascade replants
    /// computed heads, and stacked-on-primary extras survive (they were
    /// planted by Q on purpose, 0013 §3).
    pub fn set_extras(&mut self, heads: impl IntoIterator<Item = usize>) {
        self.extras = heads.into_iter().map(Selection::cursor).collect();
        self.normalize();
    }

    /// After an edit shifted bytes: remap every head/anchor by delta at
    /// a point (the mirrored-edit cascade's bookkeeping).
    pub fn remap(&mut self, at: usize, delta: isize) {
        let shift = |p: &mut usize| {
            if *p >= at {
                *p = (*p as isize + delta).max(0) as usize;
            }
        };
        shift(&mut self.primary.head);
        shift(&mut self.primary.anchor);
        for s in &mut self.extras {
            shift(&mut s.head);
            shift(&mut s.anchor);
        }
        self.normalize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariants_hold() {
        let mut s = SelectionSet::default();
        s.set_head(5);
        s.toggle_extra(); // Q plants ON the primary (0013 semantics)
        assert_eq!(s.count(), 2);
        s.plant_extra(2);
        s.plant_extra(2); // dup dies
        assert_eq!(s.count(), 3);
        assert_eq!(s.heads(), vec![5, 2, 5]);
        s.toggle_extra(); // an extra sits under the primary → drops it
        assert_eq!(s.count(), 2);
        assert_eq!(s.heads(), vec![5, 2]);
        s.collapse_extras();
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn remap_shifts_past_the_edit() {
        let mut s = SelectionSet::default();
        s.collapse_primary(10);
        s.plant_extra(20);
        s.remap(5, 3); // 3 bytes inserted at 5
        assert_eq!(s.heads(), vec![13, 23]);
        s.remap(5, -3);
        assert_eq!(s.heads(), vec![10, 20]);
    }
}
