//! Retained per-scope search baselines (0063 residual; 0058 §2): when
//! the editor holds a live filesystem-notification subscription over a
//! scope, the symbol index and project catalog survive across searches
//! and invalidate by hint; without one, every search scans exactly as
//! before — watching is never the only cache-validity mechanism (the
//! symbol index re-stats every reused file; the catalog re-stats every
//! reused directory).
//!
//! ## Ordering (Notify.tla ScanNeverErasesNewer)
//!
//! A refresh takes its invalidation snapshot BEFORE scanning and only
//! that snapshot is consumed; hints landing mid-scan stay recorded for
//! the next refresh. A cancelled refresh stores nothing and restores
//! the snapshot, so a partial scan never poses as a baseline.
//!
//! Overflow, hint-set bounds and scope-count bounds all coalesce into
//! one conservative `rescan` obligation — the staleness sign is never
//! silently dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::catalog::{CatalogStats, ProjectCatalog};
use super::symbols::{ScanStats, SymbolIndex};

/// Retained invalidation prefixes per scope before a hint storm
/// coalesces into a conservative rescan obligation.
const INVALIDATION_LIMIT: usize = 512;
/// Scopes with retained baselines per editor; beyond it the oldest
/// untouched scope is dropped rather than growing unboundedly.
const SCOPE_LIMIT: usize = 4;

/// The last refresh's accounting, surfaced for tests and coverage notes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BaselineStats {
    pub symbols: ScanStats,
    pub catalog: CatalogStats,
    /// True when the refresh rebuilt from scratch (no subscription,
    /// first build, or a rescan obligation).
    pub full: bool,
}

#[derive(Default)]
struct ScopeCache {
    /// The editor's subscription covers this scope (push coverage).
    watching: bool,
    /// Conservative rescan obligation: overflow, hint-set bound, or a
    /// coverage gap (watching lost and regained). Never cleared by
    /// anything short of a completed full rebuild.
    rescan: bool,
    /// Workspace-relative invalidated prefixes, consumed by snapshot.
    invalidated: Vec<PathBuf>,
    symbols: Option<Arc<SymbolIndex>>,
    catalog: Option<Arc<ProjectCatalog>>,
    stats: BaselineStats,
}

impl ScopeCache {
    /// The invalidation snapshot for one refresh; arrivals after this
    /// point land on the fresh vector and survive the scan.
    fn take_invalidated(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.invalidated)
    }

    fn restore_invalidated(&mut self, mut snapshot: Vec<PathBuf>) {
        snapshot.append(&mut self.invalidated);
        self.invalidated = snapshot;
        if self.invalidated.len() > INVALIDATION_LIMIT {
            self.invalidated.clear();
            self.rescan = true;
        }
    }
}
/// The retained baselines of every watched scope. Shared between the
/// editor (hint invalidation, coverage transitions) and the source
/// worker (refresh); the lock is never held across filesystem I/O.
#[derive(Default)]
pub(crate) struct Caches {
    scopes: HashMap<PathBuf, ScopeCache>,
}

/// What one refresh may reuse.
pub(crate) struct Baseline {
    pub symbols: Option<Arc<SymbolIndex>>,
    pub catalog: Option<Arc<ProjectCatalog>>,
    invalidated: Vec<PathBuf>,
}

impl Caches {
    /// Push coverage for one scope changed (subscription established,
    /// degraded or lost). Losing coverage invalidates the baseline; the
    /// next watched refresh rebuilds it fully before claiming freshness.
    pub fn set_watching(&mut self, root: &Path, watching: bool) {
        let scope = self.scope_mut(root);
        if scope.watching && !watching {
            scope.rescan = true;
        }
        scope.watching = watching;
    }

    /// Record hints against every covered scope (`root` is the
    /// subscription's root; `paths` are relative to it). A scope outside
    /// the subscription is untouched — it was never watching.
    pub fn invalidate(&mut self, root: &Path, paths: &[PathBuf], rescan: bool) {
        let roots: Vec<PathBuf> = self
            .scopes
            .keys()
            .filter(|scope| scope.as_path() == root || scope.starts_with(root))
            .cloned()
            .collect();
        for scope_root in roots {
            let scope = self.scopes.get_mut(&scope_root);
            let Some(scope) = scope else { continue };
            if rescan {
                scope.rescan = true;
                scope.invalidated.clear();
                continue;
            }
            let relative = scope_root.strip_prefix(root).unwrap_or(scope_root.as_ref());
            for path in paths {
                // Translate subscription-relative hints into this
                // scope's vocabulary; hints outside the scope don't
                // touch its baseline.
                let Ok(translated) = path.strip_prefix(relative) else {
                    continue;
                };
                scope.invalidated.push(translated.to_path_buf());
            }
            if scope.invalidated.len() > INVALIDATION_LIMIT {
                scope.invalidated.clear();
                scope.rescan = true;
            }
        }
    }

    /// The reuse baseline for one search, taken under the lock; the
    /// scan itself runs unlocked and settles with [`Caches::settle`].
    pub(crate) fn begin(&mut self, root: &Path) -> Baseline {
        let scope = self.scope_mut(root);
        let reusable = scope.watching && !scope.rescan;
        Baseline {
            symbols: if reusable {
                scope.symbols.clone()
            } else {
                None
            },
            catalog: if reusable {
                scope.catalog.clone()
            } else {
                None
            },
            invalidated: if reusable {
                scope.take_invalidated()
            } else {
                // A full rebuild re-establishes the baseline: pending
                // invalidations are covered by it.
                scope.invalidated.clear();
                Vec::new()
            },
        }
    }

    /// Store a completed refresh. Invalidations recorded mid-scan were
    /// never in its snapshot and stay pending for the next refresh.
    pub(crate) fn settle(
        &mut self,
        root: &Path,
        symbols: Option<(Arc<SymbolIndex>, ScanStats)>,
        catalog: Option<(Arc<ProjectCatalog>, CatalogStats)>,
        full: bool,
    ) {
        let scope = self.scope_mut(root);
        if let Some((index, stats)) = symbols {
            scope.symbols = Some(index);
            scope.stats.symbols = stats;
        }
        if let Some((discovered, stats)) = catalog {
            scope.catalog = Some(discovered);
            scope.stats.catalog = stats;
        }
        if full {
            scope.rescan = false;
        }
        scope.stats.full = full;
    }

    /// A cancelled refresh publishes nothing and restores the snapshot,
    /// so no invalidation is ever consumed by a scan that did not finish.
    pub(crate) fn abandon(&mut self, root: &Path, baseline: Baseline) {
        self.scope_mut(root)
            .restore_invalidated(baseline.invalidated);
    }

    /// The invalidation prefixes a refresh must honor.
    pub(crate) fn invalidated(baseline: &Baseline) -> &[PathBuf] {
        &baseline.invalidated
    }

    /// Last refresh accounting (tests).
    #[cfg(test)]
    pub(crate) fn stats(&self, root: &Path) -> Option<BaselineStats> {
        self.scopes.get(root).map(|scope| scope.stats)
    }

    fn scope_mut(&mut self, root: &Path) -> &mut ScopeCache {
        if !self.scopes.contains_key(root) && self.scopes.len() >= SCOPE_LIMIT {
            // Bounded retention: drop one untouched scope rather than
            // growing without limit. The dropped scope's next search
            // simply rebuilds (today's behavior).
            if let Some(victim) = self.scopes.keys().next().cloned() {
                self.scopes.remove(&victim);
            }
        }
        self.scopes.entry(root.to_path_buf()).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidation_snapshot_survives_mid_scan_arrivals() {
        let mut caches = Caches::default();
        let root = PathBuf::from("/scope");
        caches.set_watching(&root, true);
        caches.invalidate(&root, &[PathBuf::from("a.rs")], false);
        let baseline = caches.begin(&root);
        assert_eq!(Caches::invalidated(&baseline), &[PathBuf::from("a.rs")]);
        // A hint landing mid-scan is not consumed by it.
        caches.invalidate(&root, &[PathBuf::from("b.rs")], false);
        let next = caches.begin(&root);
        assert_eq!(Caches::invalidated(&next), &[PathBuf::from("b.rs")]);
    }

    #[test]
    fn abandon_restores_the_snapshot() {
        let mut caches = Caches::default();
        let root = PathBuf::from("/scope");
        caches.set_watching(&root, true);
        caches.invalidate(&root, &[PathBuf::from("a.rs")], false);
        let baseline = caches.begin(&root);
        caches.abandon(&root, baseline);
        let next = caches.begin(&root);
        assert_eq!(Caches::invalidated(&next), &[PathBuf::from("a.rs")]);
    }

    #[test]
    fn rescan_latch_coalesces_storms_and_full_build_clears_it() {
        let mut caches = Caches::default();
        let root = PathBuf::from("/scope");
        caches.set_watching(&root, true);
        caches.invalidate(&root, &[], true);
        let baseline = caches.begin(&root);
        assert!(
            baseline.symbols.is_none(),
            "rescan obligation forces a full rebuild"
        );
        caches.settle(&root, None, None, true);
        caches.set_watching(&root, true);
        // settled full build clears the obligation: reuse is on again
        // (no retained index here, so symbols stay absent).
        assert!(!caches.scopes[&root].rescan);
    }

    #[test]
    fn losing_coverage_invalidates_the_baseline() {
        let mut caches = Caches::default();
        let root = PathBuf::from("/scope");
        caches.set_watching(&root, true);
        caches.set_watching(&root, false);
        let scope = caches.scopes.get(&root).unwrap();
        assert!(scope.rescan, "a coverage gap conservatively rescans");
    }

    #[test]
    fn hints_translate_into_deeper_scopes_only() {
        let mut caches = Caches::default();
        let root = PathBuf::from("/scope");
        let sub = PathBuf::from("/scope/sub");
        let other = PathBuf::from("/elsewhere");
        caches.set_watching(&sub, true);
        caches.set_watching(&other, true);
        caches.invalidate(
            &root,
            &[PathBuf::from("sub/a.rs"), PathBuf::from("top.rs")],
            false,
        );
        assert_eq!(
            caches.scopes[&sub].invalidated,
            vec![PathBuf::from("a.rs")],
            "the deeper scope sees its own relative vocabulary"
        );
        assert!(
            caches.scopes[&other].invalidated.is_empty(),
            "an uncovered scope is untouched"
        );
    }
}
