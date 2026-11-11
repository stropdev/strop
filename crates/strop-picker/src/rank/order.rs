//! Orders that are not match scores (0093 §3.4, §4.2): what the user
//! reached for lately, and where a content hit sits in the tree.

use crate::{Item, Payload};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

/// Paths the user reached for, best first. The empty query lists them
/// ahead of the walk; a typed query adds a small bonus, so among equal
/// matches the familiar file leads and a better match still wins.
#[derive(Debug, Clone, Default)]
pub struct Priors(Arc<HashMap<String, u32>>);

impl Priors {
    pub fn new(best_first: impl IntoIterator<Item = String>) -> Self {
        let mut places = HashMap::new();
        for path in best_first {
            let place = places.len() as u32;
            places.entry(path).or_insert(place);
        }
        Self(Arc::new(places))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn place(&self, text: &str) -> Option<u32> {
        self.0.get(text).copied()
    }

    /// The empty query's key: listed paths in their order, then the rest.
    pub(super) fn standing(&self, text: &str) -> u32 {
        self.place(text).map_or(0, |place| u32::MAX - place)
    }

    /// A typed query's bonus: about three matched characters for the
    /// handful most reached for, tapering to one.
    pub(super) fn bonus(&self, text: &str) -> u32 {
        match self.place(text) {
            Some(0..8) => 48,
            Some(8..32) => 32,
            Some(_) => 16,
            None => 0,
        }
    }
}

/// Tree order for two paths: component by component, the way a sorted
/// directory walk visits them. Comparing bytes with the separator below
/// every other byte is that order without splitting either path.
pub fn tree_order(a: &[u8], b: &[u8]) -> Ordering {
    let rank = |byte: u8| if byte == b'/' { 0 } else { u16::from(byte) + 1 };
    for (left, right) in a.iter().zip(b) {
        if left != right {
            return rank(*left).cmp(&rank(*right));
        }
    }
    a.len().cmp(&b.len())
}

/// Content hits read in tree order, then by position in the file. Search
/// workers finish files in whatever order the disk answers; this is what
/// makes the list the same every time (0093 §4.2).
pub(super) fn hit_order(a: &Item, b: &Item) -> Ordering {
    match (&a.payload, &b.payload) {
        (
            Payload::Grep {
                location: left,
                line: left_line,
                col: left_col,
                ..
            },
            Payload::Grep {
                location: right,
                line: right_line,
                col: right_col,
                ..
            },
        ) => tree_order(
            left.path.as_os_str().as_encoded_bytes(),
            right.path.as_os_str().as_encoded_bytes(),
        )
        .then(left_line.cmp(right_line))
        .then(left_col.cmp(right_col)),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_order_is_the_sorted_walk_not_the_byte_order() {
        // `a-b` sorts before `a/` as bytes, but a walk lists directory
        // `a`'s children under `a` before it reaches the sibling `a-b`.
        assert_eq!(tree_order(b"a/x", b"a-b/x"), Ordering::Less);
        assert_eq!(tree_order(b"a/x", b"a/x"), Ordering::Equal);
        assert_eq!(tree_order(b"a", b"a/x"), Ordering::Less);
        assert_eq!(tree_order(b"b/a", b"a/b"), Ordering::Greater);
        let mut paths = ["src/b.rs", "src-old/a.rs", "src/a/z.rs", "Makefile"];
        paths.sort_by(|a, b| tree_order(a.as_bytes(), b.as_bytes()));
        assert_eq!(
            paths,
            ["Makefile", "src/a/z.rs", "src/b.rs", "src-old/a.rs"]
        );
    }

    #[test]
    fn priors_keep_the_first_place_a_path_was_given() {
        let priors = Priors::new(["b".to_string(), "a".to_string(), "b".to_string()]);
        assert!(priors.standing("b") > priors.standing("a"));
        assert!(priors.standing("a") > priors.standing("c"));
        assert_eq!(priors.standing("c"), 0);
        assert_eq!(priors.bonus("c"), 0);
        assert!(priors.bonus("b") > 0);
    }
}
