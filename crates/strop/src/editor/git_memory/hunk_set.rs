//! Worker-built working/index hunk lookups. No per-row signs allocation or scan.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::Arc;
use strop_git::{Hunk, HunkKind, LineOrigin, Sign};

#[derive(Debug, Clone)]
pub struct HunkSet(Arc<HunkIndex>);
#[derive(Debug, Default)]
struct HunkIndex {
    hunks: Vec<Arc<Hunk>>,
    total_lines: usize,
    signs: BTreeMap<usize, (usize, char)>,
    additions: BTreeSet<usize>,
}
impl Default for HunkSet {
    fn default() -> Self {
        static EMPTY: std::sync::LazyLock<Arc<HunkIndex>> =
            std::sync::LazyLock::new(|| Arc::new(HunkIndex::default()));
        Self(Arc::clone(&EMPTY))
    }
}
impl HunkSet {
    pub(crate) fn new(hunks: Vec<Hunk>, total_lines: usize) -> Self {
        let mut signs = BTreeMap::new();
        let mut additions = BTreeSet::new();
        for (index, hunk) in hunks.iter().enumerate() {
            for (line, kind) in hunk.signs() {
                let (line, symbol) = match kind {
                    Sign::DeleteAfter => (line.min(total_lines), '-'),
                    Sign::AddOrChange => (line, if hunk.kind == HunkKind::Add { '+' } else { '~' }),
                };
                signs.entry(line).or_insert((index, symbol));
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
            total_lines,
            signs,
            additions,
        }))
    }
    pub(crate) fn sign(&self, line: usize) -> Option<char> {
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
        (&self.0.hunks, self.0.total_lines).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for HunkSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (hunks, total_lines) = Deserialize::deserialize(deserializer)?;
        Ok(Self::new(hunks, total_lines))
    }
}
