//! Inclusion belongs to source witnesses, never transient catalog indices.
//! Same-query refresh can preserve only the exact location AND line contents.
use crate::Payload;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use strop_workspace::ResourceLocation;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Position {
    line: usize,
    column: usize,
    length: usize,
}
struct Decision {
    text: Arc<str>,
    seen: bool,
}
#[derive(Default)]
struct FileCounts {
    items: usize,
    excluded_rows: usize,
}
#[derive(Default)]
pub(crate) struct Workset {
    rows: HashMap<ResourceLocation, HashMap<Position, Decision>>,
    files: HashSet<ResourceLocation>,
    counts: HashMap<ResourceLocation, FileCounts>,
    excluded: usize,
}

/// Immutable decisions consumed by owned review preparation.
pub struct WorksetSnapshot {
    rows: HashMap<ResourceLocation, HashMap<Position, Arc<str>>>,
    files: HashSet<ResourceLocation>,
}
impl WorksetSnapshot {
    pub fn is_excluded(&self, payload: &Payload) -> bool {
        let Some(hit) = source_match(payload) else {
            return false;
        };
        self.files.contains(hit.path)
            || self
                .rows
                .get(hit.path)
                .and_then(|rows| rows.get(&hit.position))
                .is_some_and(|text| *text == *hit.text)
    }
}
struct Match<'a> {
    path: &'a ResourceLocation,
    position: Position,
    text: &'a Arc<str>,
}
fn source_match(payload: &Payload) -> Option<Match<'_>> {
    let Payload::Grep {
        location: path,
        line,
        col,
        match_len,
        line_text,
    } = payload
    else {
        return None;
    };
    Some(Match {
        path,
        position: Position {
            line: *line,
            column: *col,
            length: *match_len,
        },
        text: line_text,
    })
}

impl Workset {
    pub fn snapshot(&self) -> WorksetSnapshot {
        WorksetSnapshot {
            rows: self
                .rows
                .iter()
                .map(|(path, rows)| {
                    (
                        path.clone(),
                        rows.iter()
                            .map(|(position, decision)| (*position, decision.text.clone()))
                            .collect(),
                    )
                })
                .collect(),
            files: self.files.clone(),
        }
    }
    pub fn count(&self) -> usize {
        self.excluded
    }

    pub fn file_count(&self, payload: &Payload) -> Option<usize> {
        self.counts
            .get(source_match(payload)?.path)
            .map(|counts| counts.items)
    }

    pub fn is_excluded(&self, payload: &Payload) -> bool {
        let Some(hit) = source_match(payload) else {
            return false;
        };
        self.files.contains(hit.path)
            || self
                .rows
                .get(hit.path)
                .and_then(|rows| rows.get(&hit.position))
                .is_some_and(|decision| decision.text == *hit.text)
    }

    pub fn observe(&mut self, payload: &Payload) {
        let Some(hit) = source_match(payload) else {
            return;
        };
        let excluded_row = self
            .rows
            .get_mut(hit.path)
            .and_then(|rows| rows.get_mut(&hit.position))
            .is_some_and(|decision| {
                let same = decision.text == *hit.text;
                decision.seen |= same;
                same
            });
        let counts = self.counts.entry(hit.path.clone()).or_default();
        counts.items += 1;
        counts.excluded_rows += usize::from(excluded_row);
        self.excluded += usize::from(excluded_row || self.files.contains(hit.path));
    }

    pub fn toggle_row(&mut self, payload: &Payload) -> bool {
        let Some(hit) = source_match(payload) else {
            return false;
        };
        let rows = self.rows.entry(hit.path.clone()).or_default();
        let removed = rows
            .get(&hit.position)
            .is_some_and(|decision| decision.text == *hit.text);
        if removed {
            rows.remove(&hit.position);
        } else {
            rows.insert(
                hit.position,
                Decision {
                    text: hit.text.clone(),
                    seen: true,
                },
            );
        }
        let Some(counts) = self.counts.get_mut(hit.path) else {
            return false;
        };
        if removed {
            counts.excluded_rows -= 1;
        } else {
            counts.excluded_rows += 1;
        }
        if !self.files.contains(hit.path) {
            if removed {
                self.excluded -= 1;
            } else {
                self.excluded += 1;
            }
        }
        true
    }

    pub fn toggle_file(&mut self, payload: &Payload) -> bool {
        let Some(hit) = source_match(payload) else {
            return false;
        };
        let Some(counts) = self.counts.get(hit.path) else {
            return false;
        };
        let changed = counts.items - counts.excluded_rows;
        if self.files.remove(hit.path) {
            self.excluded -= changed;
        } else {
            self.files.insert(hit.path.clone());
            self.excluded += changed;
        }
        true
    }

    pub fn begin_refresh(&mut self) {
        self.counts.clear();
        self.excluded = 0;
        for rows in self.rows.values_mut() {
            for decision in rows.values_mut() {
                decision.seen = false;
            }
        }
    }

    /// Retire decisions that cannot be matched without guessing. The caller
    /// names this loss only after a complete, successful replacement dataset.
    pub fn finish_refresh(&mut self) -> usize {
        let mut lost = 0;
        self.rows.retain(|_, rows| {
            rows.retain(|_, decision| {
                lost += usize::from(!decision.seen);
                decision.seen
            });
            !rows.is_empty()
        });
        self.files.retain(|path| {
            let seen = self.counts.contains_key(path);
            lost += usize::from(!seen);
            seen
        });
        lost
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_exclusion_never_crosses_namespace_or_host() {
        let payload = |location| Payload::Grep {
            location,
            line: 1,
            col: 1,
            match_len: 6,
            line_text: "needle".into(),
        };
        let local = payload(ResourceLocation::local("/work/file".into()));
        let remote = payload(ResourceLocation::remote(
            strop_workspace::RemoteEndpoint::parse("ssh://one").unwrap(),
            "/work/file".into(),
        ));
        let other = payload(ResourceLocation::remote(
            strop_workspace::RemoteEndpoint::parse("ssh://two").unwrap(),
            "/work/file".into(),
        ));
        let mut workset = Workset::default();
        for hit in [&local, &remote, &other] {
            workset.observe(hit);
        }
        assert!(workset.toggle_file(&remote));
        assert_eq!(workset.count(), 1);
        let snapshot = workset.snapshot();
        assert!(snapshot.is_excluded(&remote));
        assert!(!snapshot.is_excluded(&local));
        assert!(!snapshot.is_excluded(&other));
    }
}
