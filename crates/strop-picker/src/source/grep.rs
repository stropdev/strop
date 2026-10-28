//! The local grep source searches selected files in process. Selection,
//! dirty snapshots and exact admission stay shared with the remote source;
//! only local content matching and result streaming live here.

use grep::regex::RegexMatcherBuilder;
use grep::searcher::{sinks::UTF8, SearcherBuilder};

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
    let pattern = content.provider_pattern().to_string();
    if content.is_literal() && pattern.is_empty() {
        return Outcome::Success(());
    }
    let cancelled = || {
        tx.heartbeat();
        cancel.is_cancelled()
    };
    // Retained baseline (0063 residual, 0058 S7): with push coverage the
    // catalog/symbol scan reuses unchanged subtrees and rescans only
    // hinted paths; without it, scan per search exactly as before. The
    // lock never spans filesystem I/O; a cancelled scan abandons (its
    // invalidation snapshot is restored, never consumed).
    let baseline = cache.lock().begin(&cwd);
    let invalidated = super::cache::Caches::invalidated(&baseline).to_vec();
    let full =
        baseline.catalog.is_none() || (content.needs_symbols() && baseline.symbols.is_none());
    // One bounded discovery scan per search (0063 §2): the catalog
    // decides `repo:` atoms exactly; a missed project only overfetches.
    let (catalog, catalog_stats) = match baseline.catalog.as_deref() {
        Some(prior) => {
            crate::source::catalog::ProjectCatalog::refresh(prior, &cwd, &invalidated, &cancelled)
        }
        None => (
            crate::source::catalog::ProjectCatalog::discover(&cwd, &cancelled),
            crate::source::catalog::CatalogStats::default(),
        ),
    };
    if cancelled() {
        cache.lock().abandon(&cwd, baseline);
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    // Every hit line goes through the exact plan: the provider pattern
    // only prefilters (0063 §4). Eligible paths come from the shared
    // selection authority with bounded count and encoded path bytes.
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let mut path_bytes = 0usize;
    let mut path_limit = false;
    let walk = super::selection::walk(&cwd, &plan, policy, &cancelled, |rel| {
        path_bytes = path_bytes.saturating_add(rel.as_os_str().as_encoded_bytes().len());
        if paths.len() >= super::selection::PATH_LIMIT || path_bytes > super::selection::PATH_BYTES
        {
            path_limit = true;
            return false;
        }
        paths.push(rel);
        true
    });
    if path_limit {
        cache.lock().abandon(&cwd, baseline);
        return Outcome::failed(
            FailureKind::Unavailable,
            "search selection exceeds 100000 paths or 16 MiB; narrow the scope",
        );
    }
    if let Err(error) = walk {
        cache.lock().abandon(&cwd, baseline);
        return Outcome::failed(FailureKind::Io, error);
    }
    if cancelled() {
        cache.lock().abandon(&cwd, baseline);
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    // Syntax-fallback symbol index (0063 §2): built only when the
    // query carries a `kind:` atom, from the selection's own eligible
    // files. Absent or incomplete entries stay Unknown and admit.
    let disk_symbols = if content.needs_symbols() {
        let (index, stats) = crate::source::symbols::SymbolIndex::refresh(
            baseline.symbols.as_deref(),
            &cwd,
            &paths,
            &invalidated,
            &cancelled,
        );
        Some((index, stats))
    } else {
        None
    };
    if cancelled() {
        cache.lock().abandon(&cwd, baseline);
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    // Settle the disk-pure baseline BEFORE the dirty overlay touches
    // the working copy — the retained index never absorbs unsaved text.
    cache.lock().settle(
        &cwd,
        disk_symbols
            .as_ref()
            .map(|(index, stats)| (std::sync::Arc::new(index.clone()), *stats)),
        Some((std::sync::Arc::new(catalog.clone()), catalog_stats)),
        full,
    );
    let mut symbols = disk_symbols.map(|(index, _)| index);
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
            catalog: Some(&catalog),
            symbols: symbols.as_mut(),
        },
    ) {
        return if cancel.is_cancelled() {
            Outcome::Cancelled(CancelReason::Superseded)
        } else {
            Outcome::failed(FailureKind::Protocol, error)
        };
    }
    let symbols = symbols.map(std::sync::Arc::new);
    let mut matcher = RegexMatcherBuilder::new();
    matcher
        .case_smart(content.case == crate::query::CaseMode::Smart)
        .case_insensitive(content.case == crate::query::CaseMode::Ignore)
        .fixed_strings(content.is_literal());
    let matcher = match matcher.build(&pattern) {
        Ok(matcher) => matcher,
        Err(error) => {
            return Outcome::failed(FailureKind::Protocol, format!("invalid regex: {error}"))
        }
    };
    let mut searcher = SearcherBuilder::new().line_number(true).build();
    for path in paths {
        tx.heartbeat();
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        let location = strop_workspace::ResourceLocation::local(cwd.join(&path));
        let relative = path.to_string_lossy();
        let display = strop_workspace::directory::display_path(&path);
        let mut terminal = None;
        let mut sink = UTF8(|line, text| {
            if cancel.is_cancelled() {
                return Ok(false);
            }
            if text.len() > super::query::RECORD_LIMIT {
                terminal = Some(Outcome::failed(
                    FailureKind::Protocol,
                    format!("{display}:{line}: source line exceeds the 1 MiB search bound"),
                ));
                return Ok(false);
            }
            let text: std::sync::Arc<str> = text.strip_suffix('\n').unwrap_or(text).into();
            if !content.admits(
                crate::query::Evidence {
                    catalog: Some(&catalog),
                    symbols: symbols.as_deref(),
                    path: Some(&relative),
                    line: Some(line as usize),
                    symbol: None,
                },
                &text,
            ) {
                return Ok(true);
            }
            let short: String = text.trim().chars().take(80).collect();
            let mut items = Vec::new();
            let mut expanded = text.len();
            for hit in content.regex.find_iter(&text).take(4097) {
                if items.len() == 4096 {
                    terminal = Some(Outcome::failed(
                        FailureKind::Protocol,
                        format!("{display}:{line}: source line exceeds 4096 matches"),
                    ));
                    return Ok(false);
                }
                let item = crate::Item {
                    badge: None,
                    text: format!("{display}:{line} · {short}"),
                    payload: crate::Payload::Grep {
                        location: location.clone(),
                        line: line as usize,
                        col: hit.start() + 1,
                        match_len: hit.len(),
                        line_text: text.clone(),
                    },
                };
                expanded = expanded.saturating_add(super::flow::row_bytes(&item));
                if expanded > super::flow::BACKLOG_BYTES {
                    terminal = Some(Outcome::failed(
                        FailureKind::Protocol,
                        format!("{display}:{line}: source line exceeds 4 MiB of result rows"),
                    ));
                    return Ok(false);
                }
                items.push(item);
            }
            if let Err(error) = tx.batch(items, &cancel) {
                terminal = Some(error.finish(&tx));
                return Ok(false);
            }
            Ok(true)
        });
        let searched = searcher.search_path(&matcher, cwd.join(&path), &mut sink);
        if let Some(outcome) = terminal {
            return outcome;
        }
        if cancel.is_cancelled() {
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        if let Err(error) = searched {
            return Outcome::failed(FailureKind::Io, format!("{display}: {error}"));
        }
    }
    Outcome::Success(())
}

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
        let directory = tempfile::tempdir().unwrap();
        let mut text = "needle on disk\n".to_string();
        text.push_str(&"filler line\n".repeat(100_000));
        std::fs::write(directory.path().join("large.txt"), text).unwrap();
        let (tx, rx) = channel();
        let (worker, request) = spawn_query("needle", directory.path(), tx);
        match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
            PickerMsg::Items(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].text, "large.txt:1 · needle on disk");
            }
            other => panic!("the disk row must arrive first: {other:?}"),
        }
        request.cancel(CancelReason::OwnerClosed);
        match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
            PickerMsg::Finished(Outcome::Cancelled(CancelReason::OwnerClosed)) => {}
            other => panic!("cancellation must settle without late rows: {other:?}"),
        }
        drop(worker);
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_secs(30)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ));
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
