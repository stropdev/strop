//! Project boundaries in one pinned workspace incarnation. Each `u64` names one
//! complete native path component. The host must intern by exact native equality
//! (including non-Unicode names), not by a digest or display name; all paths
//! use the same interner and are relative to the same observed workspace.

use alloc::vec::Vec;

fn encloses(root: &[u64], path: &[u64]) -> bool {
    if root.len() > path.len() {
        return false;
    }
    let mut i = 0;
    while i < root.len() {
        if root[i] != path[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn consider(path: &[u64], root: &[u64], best: Option<usize>) -> Option<usize> {
    match best {
        Some(depth) if root.len() <= depth => return best,
        _ => {}
    }
    if encloses(root, path) {
        Some(root.len())
    } else {
        best
    }
}

/// Select the deepest enclosing boundary from all declared and discovered roots,
/// in any order. `Some(d)` identifies the project by `path[..d]` within this
/// workspace, so duplicate evidence at the same path denotes just one project.
/// `None` means no enclosing record; an explicit workspace root gives `Some(0)`.
#[must_use]
pub fn deepest(path: &[u64], roots: &[Vec<u64>]) -> Option<usize> {
    let mut best = None;
    let mut i = 0;
    while i < roots.len() {
        best = consider(path, &roots[i], best);
        i += 1;
    }
    best
}
