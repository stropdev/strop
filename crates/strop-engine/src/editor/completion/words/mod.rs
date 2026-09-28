//! Worker-owned keyword counts. Queries never borrow the live editor buffer.
mod journal;
pub(crate) use journal::WordJournal;
mod scan;
#[cfg(test)]
mod tests;
mod windows;

use ropey::Rope;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};
use std::ops::Bound;
use std::sync::Arc;
use strop_core::id::BufferRevision;

pub(super) const MAX_WORD_BYTES: usize = 256;
pub(super) const MAX_UNIQUE_WORDS: usize = 65_536;
pub(super) const MAX_INDEX_WORD_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_QUERY_WORDS: usize = 128;
pub(super) const MAX_JOURNAL_CHANGES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum IndexCoverage {
    Complete,
    /// At least one word exceeded the token, distinct-word or text-byte bound.
    Limited,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct WordCandidate {
    pub text: Arc<str>,
    pub occurrences: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct WordSuggestions {
    pub revision: BufferRevision,
    pub items: Vec<WordCandidate>,
    pub coverage: IndexCoverage,
    /// Window truncation is separate from incomplete index coverage.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct IndexWork {
    pub scanned_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum IndexError {
    #[error("word-index maintenance was cancelled")]
    Cancelled,
    #[error("word index does not own the requested source revision")]
    StaleRevision,
    #[error("word-index journal is missing a source revision")]
    JournalGap,
    #[error("word-index journal exceeds its bounded update window")]
    JournalLimit,
    #[error("word-index journal has invalid edit geometry")]
    InvalidGeometry,
    #[error("word-index counts disagree with the prior source snapshot")]
    InvalidCounts,
    #[error("completion prefix exceeds the keyword byte bound")]
    PrefixTooLong,
}

enum Snapshot {
    Current {
        revision: BufferRevision,
        rope: Rope,
    },
    /// Interrupted updates cannot expose partially updated counts as current.
    Invalid,
}

pub(crate) struct WordIndex {
    words: BTreeMap<Arc<str>, u64>,
    word_bytes: usize,
    coverage: IndexCoverage,
    snapshot: Snapshot,
}

impl WordIndex {
    pub fn build(
        revision: BufferRevision,
        rope: Rope,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(Self, IndexWork), IndexError> {
        let mut index = Self {
            words: BTreeMap::new(),
            word_bytes: 0,
            coverage: IndexCoverage::Complete,
            snapshot: Snapshot::Invalid,
        };
        let work = scan::visit(&rope, 0..rope.len_bytes(), &mut cancelled, |word| {
            index.add(word);
            Ok(())
        })?;
        index.snapshot = Snapshot::Current { revision, rope };
        Ok((index, work))
    }

    pub fn revision(&self) -> Option<BufferRevision> {
        match self.snapshot {
            Snapshot::Current { revision, .. } => Some(revision),
            Snapshot::Invalid => None,
        }
    }

    pub fn retained_words(&self) -> usize {
        self.words.len()
    }

    pub fn retained_word_bytes(&self) -> usize {
        self.word_bytes
    }

    /// The cancellation domain is maintenance ownership (close/disable/shutdown),
    /// not query supersession. A gap requires a fresh snapshot build on the worker.
    pub fn advance(
        &mut self,
        rope: Rope,
        journal: &WordJournal,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<IndexWork, IndexError> {
        let Snapshot::Current {
            revision: before,
            rope: previous,
        } = &self.snapshot
        else {
            return Err(IndexError::StaleRevision);
        };
        let dirty = windows::changed(previous, *before, &rope, journal)?;
        let revision = journal.revision();
        if cancelled() {
            return Err(IndexError::Cancelled);
        }
        let Snapshot::Current { rope: previous, .. } =
            std::mem::replace(&mut self.snapshot, Snapshot::Invalid)
        else {
            unreachable!("snapshot ownership checked before replacement");
        };
        let mut work = IndexWork::default();
        for range in dirty.before {
            work.scanned_bytes +=
                scan::visit(&previous, range, &mut cancelled, |word| self.remove(word))?
                    .scanned_bytes;
        }
        for range in dirty.after {
            work.scanned_bytes += scan::visit(&rope, range, &mut cancelled, |word| {
                self.add(word);
                Ok(())
            })?
            .scanned_bytes;
        }
        if cancelled() {
            return Err(IndexError::Cancelled);
        }
        self.snapshot = Snapshot::Current { revision, rope };
        Ok(work)
    }

    pub fn query(
        &self,
        revision: BufferRevision,
        prefix: &str,
        include_exact: bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<WordSuggestions, IndexError> {
        if self.revision() != Some(revision) {
            return Err(IndexError::StaleRevision);
        }
        if prefix.len() > MAX_WORD_BYTES {
            return Err(IndexError::PrefixTooLong);
        }
        if cancelled() {
            return Err(IndexError::Cancelled);
        }
        let mut matching = 0usize;
        let mut best = BinaryHeap::with_capacity(MAX_QUERY_WORDS);
        for (word, &occurrences) in self
            .words
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
        {
            if !word.starts_with(prefix) {
                break;
            }
            if !include_exact && word.as_ref() == prefix {
                continue;
            }
            matching += 1;
            if matching.is_multiple_of(128) && cancelled() {
                return Err(IndexError::Cancelled);
            }
            let candidate = RankedWord { word, occurrences };
            if best.len() < MAX_QUERY_WORDS {
                best.push(candidate);
            } else if best.peek().is_some_and(|worst| candidate < *worst) {
                best.pop();
                best.push(candidate);
            }
        }
        if cancelled() {
            return Err(IndexError::Cancelled);
        }
        let mut ranked = best.into_vec();
        ranked.sort_unstable();
        let items = ranked
            .into_iter()
            .map(|candidate| WordCandidate {
                text: candidate.word.clone(),
                occurrences: candidate.occurrences,
            })
            .collect();
        Ok(WordSuggestions {
            revision,
            items,
            coverage: self.coverage,
            truncated: matching > MAX_QUERY_WORDS,
        })
    }

    fn add(&mut self, word: Option<&str>) {
        let Some(word) = word else {
            self.coverage = IndexCoverage::Limited;
            return;
        };
        if let Some(count) = self.words.get_mut(word) {
            // A count cannot exceed the addressable source byte count.
            *count += 1;
        } else if self.words.len() < MAX_UNIQUE_WORDS
            && self.word_bytes + word.len() <= MAX_INDEX_WORD_BYTES
        {
            self.word_bytes += word.len();
            self.words.insert(Arc::from(word), 1);
        } else {
            self.coverage = IndexCoverage::Limited;
        }
    }

    fn remove(&mut self, word: Option<&str>) -> Result<(), IndexError> {
        let Some(word) = word else { return Ok(()) };
        let Some(count) = self.words.get_mut(word) else {
            return if self.coverage == IndexCoverage::Limited {
                Ok(())
            } else {
                Err(IndexError::InvalidCounts)
            };
        };
        debug_assert!(*count > 0, "only present occurrences stay indexed");
        *count -= 1;
        if *count == 0 {
            self.words.remove(word);
            self.word_bytes -= word.len();
        }
        Ok(())
    }
}

/// Heap maximum is the least useful retained row: lower frequency, then later
/// spelling. Only the final bounded window acquires shared string owners.
#[derive(Eq, PartialEq)]
struct RankedWord<'a> {
    word: &'a Arc<str>,
    occurrences: u64,
}
impl Ord for RankedWord<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .occurrences
            .cmp(&self.occurrences)
            .then_with(|| self.word.cmp(other.word))
    }
}
impl PartialOrd for RankedWord<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
