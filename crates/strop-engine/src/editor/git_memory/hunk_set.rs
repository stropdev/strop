//! Worker-built working/index hunk lookups. No per-row signs allocation or scan.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::Arc;
use strop_git::{Hunk, LineOrigin, Sign};

#[derive(Debug, Clone)]
pub struct HunkSet(Arc<HunkIndex>);
#[derive(Debug, Default)]
struct HunkIndex {
    hunks: Vec<Arc<Hunk>>,
    rows: usize,
    /// 1-based row → (hunk index, sign); every key is in `1..=rows`.
    signs: BTreeMap<usize, (usize, Sign)>,
    additions: BTreeSet<usize>,
}
impl Default for HunkSet {
    fn default() -> Self {
        Self::empty().clone()
    }
}
impl HunkSet {
    pub(crate) fn empty() -> &'static Self {
        static EMPTY: std::sync::LazyLock<HunkSet> =
            std::sync::LazyLock::new(|| HunkSet(Arc::new(HunkIndex::default())));
        &EMPTY
    }

    /// Public fixture construction (the worker path stays
    /// crate-private by design): parsed hunks over a known row count.
    pub fn from_parts(hunks: Vec<Hunk>, rows: usize) -> Self {
        Self::new(hunks, rows)
    }

    /// Index `hunks` over a buffer of `rows` reachable rows
    /// (`last_content_line() + 1`: an empty buffer has one). Git's
    /// coordinates may sit outside them — `0` for an emptied file, one
    /// past the end for a deletion at the end — so every sign is clamped
    /// onto a row the caret can reach: sign and action share one key.
    pub(crate) fn new(hunks: Vec<Hunk>, rows: usize) -> Self {
        debug_assert!(rows >= 1, "a buffer always has a first row");
        let last = rows.max(1);
        let mut signs = BTreeMap::new();
        let mut additions = BTreeSet::new();
        for (index, hunk) in hunks.iter().enumerate() {
            for (line, sign) in hunk.signs() {
                signs.entry(line.clamp(1, last)).or_insert((index, sign));
            }
            additions.extend(
                hunk.lines
                    .iter()
                    .filter(|line| line.origin == LineOrigin::Addition)
                    .filter_map(|line| line.new_lineno),
            );
        }
        Self(Arc::new(HunkIndex {
            hunks: hunks.into_iter().map(Arc::new).collect(),
            rows,
            signs,
            additions,
        }))
    }
    /// Allocation identity for frame-preparation stamps: every published set
    /// is a fresh allocation (the previous set is dropped only after the
    /// swap), so equal identities mean the same published hunks.
    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.0) as *const () as usize
    }
    /// Sparse changed-line iteration for the scrollbar overview
    /// (0064 §1): one item per signed line, never a full-file scan.
    pub(crate) fn sign_lines(&self) -> impl Iterator<Item = (usize, Sign)> + '_ {
        self.0.signs.iter().map(|(&line, &(_, sign))| (line, sign))
    }

    pub(crate) fn sign(&self, line: usize) -> Option<Sign> {
        self.0.signs.get(&line).map(|(_, sign)| *sign)
    }
    pub(crate) fn addition(&self, line: usize) -> bool {
        self.0.additions.contains(&line)
    }
    pub(crate) fn at_line(&self, line: usize) -> Option<usize> {
        self.0.signs.get(&line).map(|(index, _)| *index)
    }
    pub(crate) fn next_line(&self, current: usize, forward: bool) -> Option<usize> {
        use std::ops::Bound::{Excluded, Unbounded};
        if forward {
            self.0
                .signs
                .range((Excluded(current), Unbounded))
                .next()
                .map(|(&line, _)| line)
        } else {
            self.0
                .signs
                .range(..current)
                .next_back()
                .map(|(&line, _)| line)
        }
    }
}
impl Deref for HunkSet {
    type Target = [Arc<Hunk>];
    fn deref(&self) -> &Self::Target {
        &self.0.hunks
    }
}
impl Serialize for HunkSet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.0.hunks, self.0.rows).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for HunkSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (hunks, rows) = Deserialize::deserialize(deserializer)?;
        Ok(Self::new(hunks, rows))
    }
}
