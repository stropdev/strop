//! The local grep source searches selected files in process. Selection,
//! dirty snapshots and exact admission stay shared with the remote source;
//! only local content matching and result streaming live here.

use strop_core::worker::{CancelReason, FailureKind, Outcome};

use super::PickerMsg;
use super::SourceSnapshot;

/// Execute one local source request on the editor-owned source worker.
pub(super) fn run(
    query: std::sync::Arc<crate::query::SearchQuery>,
    policy: super::selection::SelectionPolicy,
    cwd: std::path::PathBuf,
    snapshots: Vec<SourceSnapshot>,
    cache: std::sync::Arc<parking_lot::Mutex<super::cache::Caches>>,
    tx: super::flow::StreamSender,
    cancel: strop_core::worker::CancelToken,
) -> Outcome<()> {
    if cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    let plan = match crate::query::FileSelectionPlan::compile(&query) {
        Ok(plan) => plan,
        Err(diagnostic) => {
            let message = diagnostic.message.clone();
            let _ = tx.control(PickerMsg::QueryError(diagnostic));
            return Outcome::failed(FailureKind::Protocol, message);
        }
    };
    let content = match crate::query::ContentPlan::compile(&query) {
        Ok(Some(content)) => content,
        Ok(None) => {
            return Outcome::failed(
                FailureKind::Protocol,
                "the query has no search expression — add text or regex",
            )
        }
        Err(diagnostic) => {
            let message = diagnostic.message.clone();
            let _ = tx.control(PickerMsg::QueryError(diagnostic));
            return Outcome::failed(FailureKind::Protocol, message);
        }
    };
    if cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    if content.is_literal() && content.provider_pattern().is_empty() {
        return Outcome::Success(());
    }
    let cancelled = || {
        tx.heartbeat();
        cancel.is_cancelled()
    };
    let threads = disk::threads();
    // Eligible paths come from the shared selection authority with
    // bounded count and encoded path bytes, gathered on every core.
    let mut paths = match super::selection::collect(&cwd, &plan, policy, &cancelled, threads) {
        Ok(paths) => paths,
        Err(super::selection::CollectError::Limit) => {
            return Outcome::failed(
                FailureKind::Unavailable,
                "search selection exceeds 100000 paths or 16 MiB; narrow the scope",
            )
        }
        Err(super::selection::CollectError::Walk(error)) => {
            return Outcome::failed(FailureKind::Io, error)
        }
    };
    if cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    // Only `repo:` and `kind:` atoms consult evidence beyond the line
    // itself (0063 §2). A plain expression pays for neither scan.
    let (catalog, mut symbols) = if content.needs_catalog() || content.needs_symbols() {
        match evidence(&cwd, &content, &paths, &cache, &cancelled) {
            Some((catalog, symbols)) => (Some(catalog), symbols),
            None => return Outcome::Cancelled(CancelReason::OwnerClosed),
        }
    } else {
        (None, None)
    };
    // Dirty open source text is authoritative, never a hidden disk
    // save. Matching happens on this worker, under the same plan.
    if let Err(error) = super::snapshots::emit_snapshots(
        &strop_workspace::ResourceLocation::local(cwd.clone()),
        &content,
        snapshots,
        &mut paths,
        &tx,
        &cancel,
        super::snapshots::Sources {
            catalog: catalog.as_ref(),
            symbols: symbols.as_mut(),
        },
    ) {
        return if cancel.is_cancelled() {
            Outcome::Cancelled(CancelReason::Superseded)
        } else {
            Outcome::failed(FailureKind::Protocol, error)
        };
    }
    // Tree order here is a courtesy, not the contract: files are claimed
    // front to back, so hits mostly arrive in the order the list shows
    // them and rows grow at the bottom instead of shuffling. The list's
    // order itself is decided where rows are ranked (0093 §4.2).
    paths.sort_unstable_by(|a, b| {
        crate::rank::tree_order(
            a.as_os_str().as_encoded_bytes(),
            b.as_os_str().as_encoded_bytes(),
        )
    });
    disk::search(
        &disk::Scope {
            cwd: &cwd,
            content: &content,
            catalog: catalog.as_ref(),
            symbols: symbols.as_ref(),
            tx: &tx,
            cancel: &cancel,
        },
        &paths,
        threads,
    )
}

/// The retained baseline (0063 residual, 0058 S7) for queries that
/// consult it: with push coverage the catalog/symbol scan reuses
/// unchanged subtrees and rescans only hinted paths; without it, scan per
/// search. The lock never spans filesystem I/O; a cancelled scan abandons
/// (its invalidation snapshot is restored, never consumed) and reports
/// `None`. The disk-pure baseline settles BEFORE any dirty overlay — the
/// retained index never absorbs unsaved text.
fn evidence(
    cwd: &std::path::Path,
    content: &crate::query::ContentPlan,
    paths: &[std::path::PathBuf],
    cache: &parking_lot::Mutex<super::cache::Caches>,
    cancelled: &impl Fn() -> bool,
) -> Option<(
    crate::source::catalog::ProjectCatalog,
    Option<crate::source::symbols::SymbolIndex>,
)> {
    let baseline = cache.lock().begin(cwd);
    let invalidated = super::cache::Caches::invalidated(&baseline).to_vec();
    let full =
        baseline.catalog.is_none() || (content.needs_symbols() && baseline.symbols.is_none());
    // One bounded discovery scan (0063 §2): the catalog decides `repo:`
    // atoms exactly; a missed project only overfetches.
    let (catalog, catalog_stats) = match baseline.catalog.as_deref() {
        Some(prior) => {
            crate::source::catalog::ProjectCatalog::refresh(prior, cwd, &invalidated, cancelled)
        }
        None => (
            crate::source::catalog::ProjectCatalog::discover(cwd, cancelled),
            crate::source::catalog::CatalogStats::default(),
        ),
    };
    if cancelled() {
        cache.lock().abandon(cwd, baseline);
        return None;
    }
    // Syntax-fallback symbol index (0063 §2): built only when the
    // query carries a `kind:` atom, from the selection's own eligible
    // files. Absent or incomplete entries stay Unknown and admit.
    let symbols = content.needs_symbols().then(|| {
        crate::source::symbols::SymbolIndex::refresh(
            baseline.symbols.as_deref(),
            cwd,
            paths,
            &invalidated,
            cancelled,
        )
    });
    if cancelled() {
        cache.lock().abandon(cwd, baseline);
        return None;
    }
    cache.lock().settle(
        cwd,
        symbols
            .as_ref()
            .map(|(index, stats)| (std::sync::Arc::new(index.clone()), *stats)),
        Some((std::sync::Arc::new(catalog.clone()), catalog_stats)),
        full,
    );
    Some((catalog, symbols.map(|(index, _)| index)))
}

mod disk;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Sender};

    /// The old pattern-string helper through the query language (0051):
    /// bare text becomes a literal content expression.
    fn spawn_query(
        text: &str,
        cwd: &std::path::Path,
        tx: Sender<PickerMsg>,
    ) -> (super::super::SourceWorker, strop_core::worker::CancelHandle) {
        let query = crate::query::SearchQuery::parse(text);
        let worker = super::super::SourceWorker::new().unwrap();
        let request = worker.search(
            std::sync::Arc::new(query),
            super::super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            strop_workspace::ResourceLocation::local(cwd.to_path_buf()),
            Vec::new(),
            None,
            tx,
        );
        (worker, request)
    }

    #[test]
    fn exhausted_catalog_keeps_the_bounded_prefix_with_a_truncation_warning() {
        // the catalog bound is memory management, not a failure: the
        // source stops walking, the already-streamed bounded prefix stays
        // ranked and openable, the note is a warning and the finished
        // outcome is success so Enter keeps working (the red-error
        // contract this replaced is why large repos felt broken)
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("hits.txt"),
            "needle\n".repeat(100_001),
        )
        .unwrap();
        let (tx, rx) = channel();
        let worker = super::super::SourceWorker::new().unwrap();
        let _request = worker.search(
            std::sync::Arc::new(crate::query::SearchQuery::parse("needle")),
            super::super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: false,
            },
            strop_workspace::ResourceLocation::local(directory.path().to_path_buf()),
            Vec::new(),
            None,
            tx,
        );
        let mut truncated = false;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
                PickerMsg::Items(_) => {}
                PickerMsg::Warning(message) => {
                    assert!(message.contains("100,000"), "truncation note: {message}");
                    truncated = true;
                }
                PickerMsg::Finished(Outcome::Success(())) => {
                    assert!(
                        truncated,
                        "the retained prefix carries a truncation warning"
                    );
                    break;
                }
                event => panic!("expected rows, a warning, then success: {event:?}"),
            }
        }
    }

    #[test]
    fn content_search_defaults_to_smart_case_and_sensitive_is_exact() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("MixedCase.txt"), "needle\nNEEDLE\n").unwrap();
        let run = |query: &str| {
            let (tx, rx) = channel();
            let (_worker, _request) = spawn_query(query, directory.path(), tx);
            let mut hits = 0;
            loop {
                match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
                    PickerMsg::Items(batch) => hits += batch.len(),
                    PickerMsg::Warning(_) | PickerMsg::ScopeProjects(_) => {}
                    PickerMsg::Finished(Outcome::Success(())) => break hits,
                    other => panic!("unexpected search event {other:?}"),
                }
            }
        };
        assert_eq!(run("needle"), 2);
        assert_eq!(run("NEEDLE"), 1);
        assert_eq!(run("case:insensitive NEEDLE"), 2);
        assert_eq!(run("case:sensitive needle"), 1);
        assert_eq!(run("case:sensitive path:MIXEDCASE.TXT NEEDLE"), 0);
        assert_eq!(run("case:sensitive path:MixedCase.txt NEEDLE"), 1);
        assert_eq!(run("path:mixedcase.txt needle"), 2);
    }

    #[test]
    fn smart_case_boolean_prefilter_preserves_each_atoms_admission() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("cases.txt"), "foo\nFOO\nBar\nBAR\n").unwrap();
        let (tx, rx) = channel();
        let (_worker, _request) =
            spawn_query("case:smart text:foo OR text:Bar", directory.path(), tx);
        let mut lines = Vec::new();
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap() {
                PickerMsg::Items(batch) => {
                    lines.extend(batch.iter().map(|item| item.text.clone()));
                }
                PickerMsg::Warning(_) | PickerMsg::ScopeProjects(_) => {}
                PickerMsg::Finished(Outcome::Success(())) => break,
                other => panic!("unexpected search event {other:?}"),
            }
        }
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines.iter().any(|line| line.contains("foo")));
        assert!(lines.iter().any(|line| line.contains("FOO")));
        assert!(lines.iter().any(|line| line.contains("Bar")));
        assert!(!lines.iter().any(|line| line.contains("BAR")));
    }

    #[test]
    fn filter_only_query_is_a_named_failure_not_a_silent_everything_search() {
        let (tx, rx) = channel();
        let worker = spawn_query("language:rust", &std::env::temp_dir(), tx);
        match rx.recv() {
            Ok(PickerMsg::Finished(Outcome::Failed { failure, .. })) => {
                assert!(failure.message.contains("no search expression"));
            }
            other => panic!("filter-only grep must name the need, got {other:?}"),
        }
        drop(worker);
    }

    #[test]
    fn an_invalid_regex_is_a_diagnostic_at_plan_time() {
        let query = crate::query::SearchQuery::parse("regex:\"x [\"");
        let error = crate::query::ContentPlan::compile(&query).unwrap_err();
        assert!(error.message.contains("invalid regex"), "{}", error.message);
    }

    #[test]
    fn grep_worker_streams_and_finishes() {
        let dir = std::env::temp_dir().join("strop-picker-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn sharpen() {}\nlet x = sharpen;\n").unwrap();
        let (tx, rx) = channel();
        let worker = spawn_query("sharpen", &dir, tx);
        let mut items = 0;
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(PickerMsg::Items(batch)) => items += batch.len(),
                Ok(PickerMsg::Warning(_)) => {}
                Ok(PickerMsg::Finished(Outcome::Success(()))) => done = true,
                Ok(other) => panic!("unexpected terminal: {other:?}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
        assert!(done, "worker never sent Finished");
        assert_eq!(items, 2);
    }

    #[test]
    fn cancelling_disk_search_stops_the_in_process_searcher() {
        // One file with far more hits than the delivery backlog admits.
        // The first batch is held, so the searcher can only run as far as
        // its 4 MiB of credit before it must notice the cancellation.
        const LINES: usize = 90_000;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("large.txt"),
            "needle on disk\n".repeat(LINES),
        )
        .unwrap();
        let (tx, rx) = channel();
        let (worker, request) = spawn_query("needle", directory.path(), tx);
        let held = match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
            PickerMsg::Items(items) => {
                assert_eq!(items[0].text, "large.txt:1 · needle on disk");
                items
            }
            other => panic!("the disk rows must arrive first: {other:?}"),
        };
        request.cancel(CancelReason::OwnerClosed);
        let mut rows = held.len();
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
                PickerMsg::Items(items) => rows += items.len(),
                PickerMsg::Finished(Outcome::Cancelled(_)) => break,
                other => panic!("cancellation must settle as cancelled: {other:?}"),
            }
        }
        assert!(rows < LINES, "the searcher stopped early: {rows}");
        drop(worker);
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_secs(30)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn hits_from_many_files_rank_in_tree_order_however_they_arrive() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        // enough files that several threads claim work, with names whose
        // byte order differs from their tree order (`a-b` vs `a/`)
        let mut expected = Vec::new();
        for dir in ["a", "a-b", "a/deep", "b"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for dir in ["a", "a/deep", "a-b", "b"] {
            for file in 0..40 {
                let relative = format!("{dir}/f{file:02}.txt");
                std::fs::write(root.join(&relative), "x\nneedle one\nneedle two needle\n").unwrap();
                expected.push(relative);
            }
        }
        // tree order: a/deep/* (a directory's children) before a/f*, then a-b, then b
        expected.sort_by(|a, b| crate::rank::tree_order(a.as_bytes(), b.as_bytes()));
        let (tx, rx) = channel();
        let (_worker, _request) = spawn_query("needle", root, tx);
        let mut items = Vec::new();
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
                PickerMsg::Items(batch) => items.extend(batch.into_items()),
                PickerMsg::Warning(_) | PickerMsg::ScopeProjects(_) => {}
                PickerMsg::Finished(Outcome::Success(())) => break,
                other => panic!("unexpected search event {other:?}"),
            }
        }
        assert_eq!(items.len(), expected.len() * 3);
        let picker = crate::Picker::search(items, false, false);
        let ranking = crate::rank::rank(&picker.filter_request(), || false)
            .unwrap()
            .unwrap();
        let listed: Vec<(String, usize, usize)> = ranking
            .rows
            .iter()
            .map(|row| match &picker.items[row.item].payload {
                crate::Payload::Grep {
                    location,
                    line,
                    col,
                    ..
                } => (
                    location
                        .path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    *line,
                    *col,
                ),
                other => panic!("grep rows only: {other:?}"),
            })
            .collect();
        let wanted: Vec<(String, usize, usize)> = expected
            .iter()
            .flat_map(|path| {
                [
                    (path.clone(), 2, 1),
                    (path.clone(), 3, 1),
                    (path.clone(), 3, 12),
                ]
            })
            .collect();
        assert_eq!(listed, wanted);
    }

    /// 0063 §6.8 shape: a mixed parent directory — nested repositories,
    /// a vendor subtree, marker subprojects, non-Git sources in four
    /// languages — through the REAL in-process search pipeline (catalog,
    /// symbol index, dirty overlay, exact admission).
    #[test]
    fn mixed_directory_queries_narrow_exactly_end_to_end() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for dir in ["engine/src", "engine/vendor", "tools", "loose"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::create_dir(root.join("engine/.git")).unwrap();
        std::fs::create_dir(root.join("tools/.git")).unwrap();
        std::fs::write(root.join("tools/pyproject.toml"), "").unwrap();
        std::fs::write(
            root.join("engine/src/lib.rs"),
            "fn parser() {\n    let retry = request;\n}\nlet bare_parser = 1;\n",
        )
        .unwrap();
        std::fs::write(root.join("engine/vendor/v.rs"), "parser\n").unwrap();
        std::fs::write(
            root.join("tools/mod.py"),
            "class Cfg:\n    def parser(self):\n        pass\n",
        )
        .unwrap();
        std::fs::write(root.join("loose/game.lua"), "function mod:parser() end\n").unwrap();
        std::fs::write(
            root.join("loose/app.cpp"),
            "class App {\npublic:\n    void parser() {}\n};\n",
        )
        .unwrap();

        let run = |query: &str| -> Vec<String> {
            let (tx, rx) = channel();
            let (_worker, _request) = spawn_query(query, root, tx);
            let mut items = Vec::new();
            let mut done = false;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !done && std::time::Instant::now() < deadline {
                match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                    Ok(PickerMsg::Items(batch)) => {
                        items.extend(batch.iter().map(|item| item.text.clone()))
                    }
                    Ok(PickerMsg::Warning(_)) | Ok(PickerMsg::ScopeProjects(_)) => {}
                    Ok(PickerMsg::Finished(Outcome::Success(()))) => done = true,
                    Ok(other) => panic!("unexpected terminal: {other:?}"),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            assert!(done, "the search must settle: {query}");
            items.sort();
            items
        };

        // Repository narrowing: only engine paths, vendor included.
        let engine = run("repo:engine AND parser");
        assert!(!engine.is_empty());
        assert!(
            engine.iter().all(|text| text.starts_with("engine/")),
            "{engine:?}"
        );
        // Branch + exclusion: engine and tools, never vendor.
        let branched = run("(repo:engine OR repo:tools) NOT glob:**/vendor/** parser");
        assert!(
            branched.iter().all(|text| !text.contains("vendor/")),
            "{branched:?}"
        );
        assert!(
            branched
                .iter()
                .any(|text| text.starts_with("engine/src/lib.rs")),
            "{branched:?}"
        );
        assert!(
            branched.iter().any(|text| text.starts_with("tools/mod.py")),
            "{branched:?}"
        );
        // Syntax evidence narrows kind: exactly the line inside the
        // rust function carries both facts.
        let functions = run("kind:function AND retry");
        assert_eq!(
            functions,
            vec!["engine/src/lib.rs:2 · let retry = request;".to_string()],
            "{functions:?}"
        );
        // Classes across languages: the python method line and the
        // cpp member line, both inside class spans.
        let classes = run("kind:class AND parser");
        assert_eq!(
            classes,
            vec![
                "loose/app.cpp:3 · void parser() {}".to_string(),
                "tools/mod.py:2 · def parser(self):".to_string(),
            ],
            "{classes:?}"
        );
    }
}

#[cfg(test)]
mod e2e;
