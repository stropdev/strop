//! The files a project's user keeps coming back to (0093 §3.4).
//!
//! Frecency in the zoxide/Firefox sense — how often, weighted by how
//! lately — but "lately" is counted in visits, not seconds. A project's
//! clock ticks once per file visit, so the order is a pure function of
//! what was opened: replay reproduces it, tests need no clock, and a
//! project left alone for a month comes back in the order it was left.

use serde::{Deserialize, Serialize};

/// Entries one project keeps. Past it the weakest entry is forgotten.
pub(crate) const RETAINED: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    /// Workspace-relative, `/`-separated: the finder's own row text.
    path: String,
    visits: u32,
    /// The project clock at the latest visit.
    last: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentFiles {
    clock: u64,
    entries: Vec<Entry>,
}

/// How much a visit count is worth at an age, in visits since: the last
/// few files count fourfold, the working set double, the long tail half.
fn weight(age: u64) -> u64 {
    match age {
        0..8 => 8,
        8..32 => 4,
        32..128 => 2,
        _ => 1,
    }
}

impl RecentFiles {
    fn score(&self, entry: &Entry) -> u64 {
        u64::from(entry.visits) * weight(self.clock - entry.last)
    }

    pub(crate) fn visit(&mut self, path: &str) {
        self.clock += 1;
        match self.entries.iter_mut().find(|entry| entry.path == path) {
            Some(entry) => {
                entry.visits = entry.visits.saturating_add(1);
                entry.last = self.clock;
            }
            None => self.entries.push(Entry {
                path: path.to_owned(),
                visits: 1,
                last: self.clock,
            }),
        }
        if self.entries.len() > RETAINED {
            let weakest = (0..self.entries.len())
                .min_by_key(|&index| (self.score(&self.entries[index]), self.entries[index].last));
            if let Some(weakest) = weakest {
                self.entries.swap_remove(weakest);
            }
        }
    }

    /// Best first: by score, the more recent visit first among equals.
    pub(crate) fn ranked(&self) -> Vec<String> {
        let mut order: Vec<&Entry> = self.entries.iter().collect();
        order.sort_by(|a, b| self.score(b).cmp(&self.score(a)).then(b.last.cmp(&a.last)));
        order.into_iter().map(|entry| entry.path.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A stored value is data from disk: reject one this code could not
    /// have written rather than rank by it.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.entries.len() > RETAINED {
            return Err("recent files exceed their retention bound".into());
        }
        if self.entries.iter().any(|entry| entry.last > self.clock) {
            return Err("a recent file was visited after the project clock".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visited(paths: &[&str]) -> RecentFiles {
        let mut recent = RecentFiles::default();
        for path in paths {
            recent.visit(path);
        }
        recent
    }

    #[test]
    fn three_files_opened_once_read_most_recent_first() {
        assert_eq!(visited(&["a", "b", "c"]).ranked(), ["c", "b", "a"]);
    }

    #[test]
    fn a_file_returned_to_outranks_one_merely_seen_last() {
        let recent = visited(&["hub", "x", "hub", "y", "hub", "z"]);
        assert_eq!(recent.ranked(), ["hub", "z", "y", "x"]);
    }

    #[test]
    fn old_visits_fade_but_a_habit_outlasts_a_glance() {
        let mut recent = visited(&["habit"; 6]);
        recent.visit("glance");
        for index in 0..40 {
            recent.visit(&format!("noise{index}"));
        }
        let order = recent.ranked();
        let place = |path: &str| order.iter().position(|entry| entry == path).unwrap();
        // six visits long ago still beat one visit long ago…
        assert!(place("habit") < place("glance"));
        // …and one visit just now beats one visit long ago
        assert!(place("noise39") < place("glance"));
    }

    #[test]
    fn the_weakest_entry_is_forgotten_at_the_bound() {
        let mut recent = RecentFiles::default();
        recent.visit("kept");
        recent.visit("kept");
        for index in 0..RETAINED + 10 {
            recent.visit(&format!("f{index}"));
        }
        assert_eq!(recent.entries.len(), RETAINED);
        assert!(recent.ranked().contains(&"kept".to_string()));
        assert!(!recent.ranked().contains(&"f0".to_string()));
        recent.validate().unwrap();
    }

    #[test]
    fn stored_state_round_trips_and_impossible_state_is_refused() {
        let recent = visited(&["a", "b", "a"]);
        let stored = serde_json::to_string(&recent).unwrap();
        let loaded: RecentFiles = serde_json::from_str(&stored).unwrap();
        assert_eq!(loaded, recent);
        let forged: RecentFiles =
            serde_json::from_str(r#"{"clock":1,"entries":[{"path":"a","visits":1,"last":9}]}"#)
                .unwrap();
        assert!(forged.validate().is_err());
    }
}
