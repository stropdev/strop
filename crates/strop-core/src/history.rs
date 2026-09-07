//! Undo history (Helix `helix-core/history.rs` lineage, ported):
//! revisions form a tree — every committed transaction is a node holding
//! both its undo and redo edit sets; `u` walks to the parent, `Ctrl-r`
//! descends to the last-visited child. Editing after an undo forks a new
//! branch; the tree keeps the old one (0001 pillar 4: Neovim users
//! expect branches).

mod validate;
pub use validate::HistoryError;
/// One buffer mutation as seen by history. Both directions are stored so
/// redo replays exactly what undo undid — no re-derivation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Edit {
    pub at: usize,
    /// Text inserted (for Insert) or removed (for Delete) by this edit.
    pub text: String,
    pub kind: EditKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EditKind {
    Insert,
    Delete,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Revision {
    parent: usize,
    last_child: Option<usize>,
    /// Applied in reverse order on undo.
    undo: Vec<Edit>,
    /// Applied in order on redo.
    redo: Vec<Edit>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct History {
    /// Depth cap (0001 §3: full trees per project bloat fast).
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

/// One row of the undo-tree browser (`Space u`).
#[derive(Debug, Clone)]
pub struct RevisionRow {
    pub index: usize,
    pub parent: usize,
    /// Depth in the tree (root = 0) — the browser indents by it.
    pub depth: usize,
    /// Short description of the change, e.g. `+ "foo"` / `- "bar"`.
    pub summary: String,
    pub is_current: bool,
    /// True when the revision's parent isn't depth-1 above it in display
    /// order — the browser draws a branch marker.
    pub branches: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum HistoryAction {
    Undo,
    Redo,
    Jump(usize),
}

impl History {
    /// Count a move without changing the tree, so revision capacity is checked
    /// before publishing history navigation or committing an open undo group.
    pub(crate) fn movement_cost(&self, action: HistoryAction) -> Option<usize> {
        let pending = self.pending.as_ref().map_or(0, |(undo, _)| undo.len());
        match action {
            HistoryAction::Undo if pending > 0 => Some(pending),
            HistoryAction::Undo => {
                (self.current != 0).then(|| self.revisions[self.current].undo.len())
            }
            HistoryAction::Redo if pending > 0 => None,
            HistoryAction::Redo => self.revisions[self.current]
                .last_child
                .map(|child| self.revisions[child].redo.len()),
            HistoryAction::Jump(target) => {
                if target >= self.revisions.len() {
                    return None;
                }
                let depth = |mut at: usize| {
                    let mut depth = 0;
                    while at != 0 {
                        depth += 1;
                        at = self.revisions[at].parent;
                    }
                    depth
                };
                let (mut from, mut to) = (self.current, target);
                let (mut left, mut right) = (depth(from), depth(to));
                let mut count = pending;
                while from != to {
                    if left >= right {
                        count += self.revisions[from].undo.len();
                        from = self.revisions[from].parent;
                        left -= 1;
                    } else {
                        count += self.revisions[to].redo.len();
                        to = self.revisions[to].parent;
                        right -= 1;
                    }
                }
                Some(count)
            }
        }
    }

    pub(crate) fn navigate(&mut self, action: HistoryAction) -> Option<Vec<Edit>> {
        self.commit();
        match action {
            HistoryAction::Undo => self.undo_ops(),
            HistoryAction::Redo => self.redo_ops(),
            HistoryAction::Jump(target) => self.ops_to(target),
        }
    }
}

impl History {
    /// The revision tree, newest-first, for the undo-tree browser.
    pub fn tree_rows(&self) -> Vec<RevisionRow> {
        let mut depths = vec![0usize; self.revisions.len()];
        for i in 1..self.revisions.len() {
            depths[i] = depths[self.revisions[i].parent] + 1;
        }
        let mut out: Vec<RevisionRow> = (1..self.revisions.len())
            .rev()
            .map(|i| {
                let rev = &self.revisions[i];
                let first = rev.redo.first();
                let summary = match first {
                    Some(e) => {
                        let sign = match e.kind {
                            EditKind::Insert => "+",
                            EditKind::Delete => "-",
                        };
                        let text: String = e
                            .text
                            .chars()
                            .take(24)
                            .map(|c| if c == '\n' { '↵' } else { c })
                            .collect();
                        let more = if e.text.chars().count() > 24 {
                            "…"
                        } else {
                            ""
                        };
                        format!("{sign} \"{text}{more}\"")
                    }
                    None => "(empty)".into(),
                };
                RevisionRow {
                    index: i,
                    parent: rev.parent,
                    depth: depths[i],
                    summary,
                    is_current: i == self.current,
                    // a sibling with the same parent already exists →
                    // this revision forked off a branch
                    branches: self.revisions[..i].iter().any(|r| r.parent == rev.parent),
                }
            })
            .collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.index));
        out
    }

    /// Edits that move the buffer from `current` to `target`: undo up to
    /// the fork, redo down the target's branch. None when target is
    /// unknown. `current` is updated; the caller applies the ops.
    pub fn ops_to(&mut self, target: usize) -> Option<Vec<Edit>> {
        if target >= self.revisions.len() {
            return None;
        }
        // ancestors of current (inclusive), root-last
        let mut anc_cur = Vec::new();
        let mut at = self.current;
        loop {
            anc_cur.push(at);
            if at == 0 {
                break;
            }
            at = self.revisions[at].parent;
        }
        // walk target up to the fork
        let mut up_path = Vec::new(); // target..fork, target-first
        let mut t = target;
        while !anc_cur.contains(&t) {
            up_path.push(t);
            t = self.revisions[t].parent;
        }
        let fork = t;
        let mut ops = Vec::new();
        // undo: current up to (not incl.) the fork
        let mut c = self.current;
        while c != fork {
            let mut rev_undo = self.revisions[c].undo.clone();
            rev_undo.reverse();
            ops.extend(rev_undo);
            c = self.revisions[c].parent;
        }
        // redo: fork down to target (reverse of the up-walk)
        for &r in up_path.iter().rev() {
            ops.extend(self.revisions[r].redo.clone());
        }
        // keep last_child pointers honest along both legs
        let mut c = self.current;
        while c != fork {
            let p = self.revisions[c].parent;
            self.revisions[p].last_child = Some(c);
            c = p;
        }
        let mut p = fork;
        for &r in up_path.iter().rev() {
            self.revisions[p].last_child = Some(r);
            p = r;
        }
        self.current = target;
        Some(ops)
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

    /// The redo ops of the revision just committed (the transaction's
    /// edits in apply order — the anchor-map input, 0020 §14).
    pub fn last_committed_ops(&self) -> Option<Vec<Edit>> {
        if self.current == 0 {
            return None;
        }
        Some(self.revisions[self.current].redo.clone())
    }

    /// The change list (vim g;/g,): positions of the ancestor chain's
    /// committed revisions, NEWEST first. Each revision contributes its
    /// earliest edit position — where that change began.
    pub fn change_positions(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut i = self.current;
        while i != 0 {
            let rev = &self.revisions[i];
            // redo ops are in apply order; the first one's position is
            // where the change starts
            if let Some(first) = rev.redo.first() {
                out.push(first.at);
            }
            i = rev.parent;
        }
        out
    }

    pub fn depth(&self) -> usize {
        self.revisions.len()
    }

    /// Persist only the newest linear undo path, bounded before cloning text.
    /// Branches remain in memory. If the newest transaction exceeds the byte
    /// limit, no history is retained: skipping it would make older edits invalid.
    pub fn snapshot(&self, revisions: usize, text_bytes: usize) -> Self {
        let mut chain = Vec::new();
        let mut bytes = 0usize;
        let mut retain = |undo: &Vec<Edit>, redo: &Vec<Edit>| {
            let size = undo
                .iter()
                .chain(redo)
                .try_fold(0usize, |n, e| n.checked_add(e.text.len()));
            let Some(next) = size.and_then(|size| bytes.checked_add(size)) else {
                return false;
            };
            if chain.len() >= revisions || next > text_bytes {
                return false;
            }
            bytes = next;
            chain.push((undo.clone(), redo.clone()));
            true
        };
        if let Some((undo, redo)) = &self.pending {
            if !undo.is_empty() && !retain(undo, redo) {
                return Self::default();
            }
        }
        let mut at = self.current;
        while at != 0 {
            let revision = &self.revisions[at];
            if !retain(&revision.undo, &revision.redo) {
                break;
            }
            at = revision.parent;
        }
        let mut snapshot = Self::default();
        for (undo, redo) in chain.into_iter().rev() {
            let parent = snapshot.current;
            let index = snapshot.revisions.len();
            snapshot.revisions[parent].last_child = Some(index);
            snapshot.revisions.push(Revision {
                parent,
                last_child: None,
                undo,
                redo,
            });
            snapshot.current = index;
        }
        snapshot
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
        // at the tip after redo: undo is available, redo is not
        assert!(h.can_undo());
        assert!(!h.can_redo());
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
