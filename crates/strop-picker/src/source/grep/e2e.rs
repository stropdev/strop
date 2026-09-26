//! 0063 §6.8 end-to-end: the full mixed-directory shape — a nested
//! repository holding a linked worktree, a Python marker project,
//! non-Git C++/Lua/Rust sources, Unicode names and a dirty-source
//! target — through the REAL pipeline (rg child, reader threads,
//! project catalog, symbol index, dirty overlay, exact admission).
//! Beside `grep::tests::mixed_directory_queries_narrow_exactly_end_to_end`,
//! which owns the base repo:/kind:/NOT narrowing corpus; nothing here
//! weakens it. Missing-server behavior stays at its engine warm-up
//! unit level; remote catalog/kind evidence stays 0058-deferred, but
//! the remote namespace boundary is pinned hermetically at the seam
//! remote.rs uses.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use strop_core::worker::{CancelHandle, CancelReason, Outcome};
use strop_workspace::ResourceLocation;

use super::super::selection::SelectionPolicy;
use super::super::{PickerMsg, SourceSnapshot, SourceWorker};

const TIMEOUT: Duration = Duration::from_secs(30);

/// The full §6.8 fixture: the nested `engine` repository with a linked
/// worktree inside it, the `tools` Python project, non-Git C++, Lua
/// and Rust sources, a Unicode file, a dirty-source target whose disk
/// copy is stale by design, and a big file for snapshot overlays.
fn mixed_fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for dir in ["engine/src", "engine/wt-feature", "tools", "loose"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::create_dir(root.join("engine/.git")).unwrap();
    std::fs::create_dir(root.join("tools/.git")).unwrap();
    std::fs::write(root.join("tools/pyproject.toml"), "").unwrap();
    // A linked worktree: `.git` is a FILE pointing at the real gitdir.
    std::fs::write(
        root.join("engine/wt-feature/.git"),
        "gitdir: /elsewhere/engine/.git/worktrees/feature\n",
    )
    .unwrap();
    std::fs::write(
        root.join("engine/src/lib.rs"),
        "fn parser() {\n    let retry = request;\n}\nlet bare_parser = 1;\n",
    )
    .unwrap();
    std::fs::write(
        root.join("engine/wt-feature/feat.rs"),
        "fn parser() {\n    let worktree_parser = 1;\n}\n",
    )
    .unwrap();
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
    std::fs::write(
        root.join("loose/ünïcode.rs"),
        "fn naïve_parser() {}\nlet ünïcode_ref = naïve_parser;\n",
    )
    .unwrap();
    std::fs::write(
        root.join("loose/dirty.rs"),
        "fn on_disk() {\n    let disk_marker = 1;\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("loose/big.rs"), "needle on disk\n").unwrap();
    directory
}

fn spawn_search(
    query: &str,
    cwd: &std::path::Path,
    snapshots: Vec<SourceSnapshot>,
    tx: Sender<PickerMsg>,
) -> (SourceWorker, CancelHandle) {
    let worker = SourceWorker::new().unwrap();
    let request = worker.search(
        std::sync::Arc::new(crate::query::SearchQuery::parse(query)),
        SelectionPolicy {
            hidden: true,
            respect_ignore: true,
        },
        ResourceLocation::local(cwd.to_path_buf()),
        snapshots,
        None,
        tx,
    );
    (worker, request)
}

/// Drain one search to its terminal event and return the published
/// rows; any terminal shape other than success fails the test.
fn collect_items(rx: &Receiver<PickerMsg>, query: &str) -> Vec<crate::Item> {
    let mut items = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(PickerMsg::Items(batch)) => items.extend(batch.iter().cloned()),
            Ok(PickerMsg::Warning(_)) | Ok(PickerMsg::ScopeProjects(_)) => {}
            Ok(PickerMsg::Finished(Outcome::Success(()))) => return items,
            Ok(other) => panic!("unexpected terminal for {query}: {other:?}"),
            Err(RecvTimeoutError::Timeout) => {
                assert!(Instant::now() < deadline, "the search must settle: {query}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the search channel closed before Finished: {query}")
            }
        }
    }
}

fn run_items(
    root: &std::path::Path,
    query: &str,
    snapshots: Vec<SourceSnapshot>,
) -> Vec<crate::Item> {
    let (tx, rx) = channel();
    let (worker, _request) = spawn_search(query, root, snapshots, tx);
    let items = collect_items(&rx, query);
    drop(worker);
    items
}

fn run_texts(root: &std::path::Path, query: &str, snapshots: Vec<SourceSnapshot>) -> Vec<String> {
    let mut texts: Vec<String> = run_items(root, query, snapshots)
        .iter()
        .map(|item| item.text.clone())
        .collect();
    texts.sort();
    texts
}

/// A first buffer line the query matches, then enough non-matching
/// filler that the snapshot scan is still busy when the test acts:
/// receiving the first row proves the search is running, and the scan
/// of the remaining lines cannot finish in the receive-to-cancel gap.
fn busy_snapshot(root: &std::path::Path, first_line: &str) -> SourceSnapshot {
    let mut text = first_line.to_string();
    text.push('\n');
    for _ in 0..100_000 {
        text.push_str("filler line\n");
    }
    SourceSnapshot {
        path: root.join("loose/big.rs"),
        text: ropey::Rope::from_str(&text),
    }
}

/// §6.8 worktree: `repo:` names the worktree's own basename, and the
/// enclosing repository does not swallow the worktree's files
/// (deepest-enclosing ownership, catalog.rs).
#[test]
fn a_worktree_narrows_repo_to_its_basename_not_the_parent() {
    let directory = mixed_fixture();
    let root = directory.path();
    let worktree = run_texts(root, "repo:wt-feature AND parser", Vec::new());
    assert_eq!(
        worktree,
        vec![
            "engine/wt-feature/feat.rs:1 · fn parser() {".to_string(),
            "engine/wt-feature/feat.rs:2 · let worktree_parser = 1;".to_string(),
        ]
    );
    let engine = run_texts(root, "repo:engine AND parser", Vec::new());
    assert_eq!(
        engine,
        vec![
            "engine/src/lib.rs:1 · fn parser() {".to_string(),
            "engine/src/lib.rs:4 · let bare_parser = 1;".to_string(),
        ],
        "the worktree's files belong to repo:wt-feature, not repo:engine"
    );
}

/// §6.8 dirty sources: the open buffer's unsaved text is authoritative
/// over disk for content hits AND for `kind:` evidence — the symbol
/// overlay replaces whatever the disk pass extracted (snapshots.rs,
/// symbols.rs SymbolIndex::overlay).
#[test]
fn dirty_source_text_is_authoritative_for_content_and_kind_evidence() {
    let directory = mixed_fixture();
    let root = directory.path();
    // Contradicts disk: a different function on different lines
    // (on disk `fn on_disk` spans lines 1-3; here lines 2-5).
    let dirty = "let head = 0;\nfn in_buffer() {\n    let buffer_marker = 1;\n    let buffer_tail = 2;\n}\n";
    let snapshot = || {
        vec![SourceSnapshot {
            path: root.join("loose/dirty.rs"),
            text: ropey::Rope::from_str(dirty),
        }]
    };
    // Content hits come from the buffer, badged as such…
    let items = run_items(root, "buffer_marker", snapshot());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].text, "loose/dirty.rs:3 · let buffer_marker = 1;");
    assert_eq!(items[0].badge.as_deref(), Some("buffer"));
    // …and the stale disk line is invisible while the buffer is open.
    let disk_line = run_items(root, "disk_marker", snapshot());
    assert!(
        disk_line.is_empty(),
        "stale disk text leaked: {disk_line:?}"
    );
    // kind: evidence is the buffer's too: line 4 sits inside the
    // buffer's fn (the disk fn ended at line 3)…
    let inside = run_texts(root, "kind:function AND buffer_tail", snapshot());
    assert_eq!(
        inside,
        vec!["loose/dirty.rs:4 · let buffer_tail = 2;".to_string()]
    );
    // …and line 1 sits outside it, though the disk fn covered line 1.
    let outside = run_items(root, "kind:function AND head", snapshot());
    assert!(
        outside.is_empty(),
        "disk symbol evidence leaked through the overlay: {outside:?}"
    );
    // Differential control: with no snapshot the disk line is found,
    // so the misses above are the overlay, not a missing file.
    let disk = run_texts(root, "disk_marker", Vec::new());
    assert_eq!(
        disk,
        vec!["loose/dirty.rs:2 · let disk_marker = 1;".to_string()]
    );
}

/// §6.8 cancellation: a search cancelled mid-flight settles with
/// exactly one terminal Finished(Cancelled) and publishes no row after
/// the cancel; logical cancellation then becomes physical completion
/// (every sender drops).
#[test]
fn cancelling_a_running_search_settles_exactly_once_with_no_late_rows() {
    let directory = mixed_fixture();
    let root = directory.path();
    let (tx, rx) = channel();
    let (worker, request) = spawn_search(
        "needle",
        root,
        vec![busy_snapshot(root, "needle in the buffer")],
        tx,
    );
    // The dirty-buffer row arrives before rg even spawns: the search
    // is provably running.
    let first = rx.recv_timeout(TIMEOUT).unwrap();
    let PickerMsg::Items(batch) = first else {
        panic!("the dirty-buffer row must arrive first: {first:?}")
    };
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].text, "loose/big.rs:1 · needle in the buffer");
    assert_eq!(batch[0].badge.as_deref(), Some("buffer"));
    request.cancel(CancelReason::OwnerClosed);
    let outcome = match rx.recv_timeout(TIMEOUT).unwrap() {
        PickerMsg::Finished(outcome) => outcome,
        other => panic!("nothing may publish after the cancel: {other:?}"),
    };
    assert_eq!(outcome, Outcome::Cancelled(CancelReason::OwnerClosed));
    drop(worker);
    assert!(
        matches!(
            rx.recv_timeout(TIMEOUT),
            Err(RecvTimeoutError::Disconnected)
        ),
        "physical completion follows: no late rows, no second terminal"
    );
}

/// §6.8 restart/supersede: a retired generation never publishes; the
/// current generation publishes the complete result set (the
/// SearchLifecycle RowsCurrent invariant, worker.rs machinery).
#[test]
fn a_superseded_generation_never_publishes() {
    let directory = mixed_fixture();
    let root = directory.path();
    let worker = SourceWorker::new().unwrap();
    let policy = || SelectionPolicy {
        hidden: true,
        respect_ignore: true,
    };
    // Generation A is provably running once its buffer row arrives.
    let (tx_a, rx_a) = channel();
    let handle_a = worker.search(
        std::sync::Arc::new(crate::query::SearchQuery::parse("gena_hit")),
        policy(),
        ResourceLocation::local(root.to_path_buf()),
        vec![busy_snapshot(root, "gena_hit in the buffer")],
        None,
        tx_a,
    );
    let first = rx_a.recv_timeout(TIMEOUT).unwrap();
    assert!(
        matches!(&first, PickerMsg::Items(batch)
            if batch.len() == 1 && batch[0].text == "loose/big.rs:1 · gena_hit in the buffer"),
        "generation A must be running: {first:?}"
    );
    // Generation B supersedes A on the one source worker.
    let (tx_b, rx_b) = channel();
    let _handle_b = worker.search(
        std::sync::Arc::new(crate::query::SearchQuery::parse("parser")),
        policy(),
        ResourceLocation::local(root.to_path_buf()),
        Vec::new(),
        None,
        tx_b,
    );
    // A settles exactly once as superseded and never publishes again.
    let outcome = match rx_a.recv_timeout(TIMEOUT).unwrap() {
        PickerMsg::Finished(outcome) => outcome,
        other => panic!("a retired generation must not publish: {other:?}"),
    };
    assert_eq!(outcome, Outcome::Cancelled(CancelReason::Superseded));
    match rx_a.recv_timeout(TIMEOUT) {
        Err(RecvTimeoutError::Disconnected) => {}
        Ok(other) => panic!("no late rows from the retired generation: {other:?}"),
        Err(RecvTimeoutError::Timeout) => {
            panic!("the retired generation must finish physically")
        }
    }
    drop(handle_a);
    // B, the current generation, publishes the complete result set
    // across the whole mixed fixture.
    let mut texts: Vec<String> = collect_items(&rx_b, "parser")
        .iter()
        .map(|item| item.text.clone())
        .collect();
    texts.sort();
    assert_eq!(
        texts,
        vec![
            "engine/src/lib.rs:1 · fn parser() {".to_string(),
            "engine/src/lib.rs:4 · let bare_parser = 1;".to_string(),
            "engine/wt-feature/feat.rs:1 · fn parser() {".to_string(),
            "engine/wt-feature/feat.rs:2 · let worktree_parser = 1;".to_string(),
            "loose/app.cpp:3 · void parser() {}".to_string(),
            "loose/game.lua:1 · function mod:parser() end".to_string(),
            "loose/ünïcode.rs:1 · fn naïve_parser() {}".to_string(),
            "loose/ünïcode.rs:2 · let ünïcode_ref = naïve_parser;".to_string(),
            "tools/mod.py:2 · def parser(self):".to_string(),
        ]
    );
    drop(worker);
}

/// §6.8 Unicode locations: non-ASCII file names and symbols keep exact
/// 1-based line numbers and rg's byte columns through the real pipeline.
#[test]
fn unicode_names_and_symbols_keep_exact_locations_end_to_end() {
    let directory = mixed_fixture();
    let root = directory.path();
    let items = run_items(root, "naïve", Vec::new());
    let mut texts: Vec<String> = items.iter().map(|item| item.text.clone()).collect();
    texts.sort();
    assert_eq!(
        texts,
        vec![
            "loose/ünïcode.rs:1 · fn naïve_parser() {}".to_string(),
            "loose/ünïcode.rs:2 · let ünïcode_ref = naïve_parser;".to_string(),
        ]
    );
    for item in &items {
        let crate::Payload::Grep {
            location,
            line,
            col,
            match_len,
            ..
        } = &item.payload
        else {
            panic!("grep rows only")
        };
        assert!(location.path.ends_with("loose/ünïcode.rs"));
        // Columns are byte offsets; the multi-byte ü/ï must not shift
        // them ("naïve" is 6 bytes).
        match line {
            1 => assert_eq!((*col, *match_len), (4, 6)),
            2 => assert_eq!((*col, *match_len), (21, 6)),
            other => panic!("unexpected line {other}"),
        }
    }
}

/// §6.8 remote namespace isolation, pinned hermetically at the seam
/// remote.rs uses (source/query.rs): a remote hit keeps the Remote
/// filesystem namespace and stays inside its captured scope; forged
/// escaping paths and invalid native names are rejected before any
/// item exists. Remote catalog/kind evidence stays 0058-deferred.
#[test]
fn remote_hits_carry_the_remote_namespace_and_never_escape_the_scope() {
    use base64::Engine;
    let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://builder").unwrap();
    let root = ResourceLocation::remote(endpoint, "/srv/repo".into());
    let record = |path: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({
            "type": "match",
            "data": {
                "path": path,
                "lines": { "text": "needle on the remote\n" },
                "line_number": 7,
                "submatches": [{ "start": 0, "end": 6 }]
            }
        }))
        .unwrap()
    };
    // A legitimate remote hit presents the scope-relative path to
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let admit = move |path: &str, line: Option<usize>, _text: &str| {
        recorded.lock().push((path.to_string(), line));
        true
    };
    let items = super::super::query::parse_json_match_with(
        &record(serde_json::json!({ "text": "src/lib.rs" })),
        &root,
        Some(&admit),
    )
    .unwrap();
    assert_eq!(
        seen.lock().as_slice(),
        &[("src/lib.rs".to_string(), Some(7))]
    );
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].text, "src/lib.rs:7 · needle on the remote");
    let crate::Payload::Grep { location, line, .. } = &items[0].payload else {
        panic!("grep rows only")
    };
    assert_eq!(location.filesystem, root.filesystem);
    assert_eq!(
        location.path,
        std::path::PathBuf::from("/srv/repo/src/lib.rs")
    );
    assert_eq!(*line, 7);
    // Forged escapes never become items.
    for forged in [
        record(serde_json::json!({ "text": "../outside.rs" })),
        record(serde_json::json!({ "text": "/etc/passwd" })),
    ] {
        let Err(error) = super::super::query::parse_json_match_with(&forged, &root, None) else {
            panic!("an escaping remote path must be rejected")
        };
        assert!(error.contains("escaped"), "{error}");
    }
    // A NUL in a native remote name is invalid at the wire boundary.
    let nul = record(serde_json::json!({
        "bytes": base64::prelude::BASE64_STANDARD.encode(b"lo\0se.rs")
    }));
    assert!(super::super::query::parse_json_match_with(&nul, &root, None).is_err());
}

/// Run workspace symbols to completion on a RETAINED worker, returning
/// the published rows.
fn workspace_symbol_texts(
    worker: &SourceWorker,
    root: &std::path::Path,
    query: &str,
) -> Vec<String> {
    let (tx, rx) = channel();
    let _request = worker.workspace_symbols(
        root.to_path_buf(),
        std::sync::Arc::new(crate::query::SearchQuery::parse(query)),
        SelectionPolicy {
            hidden: true,
            respect_ignore: true,
        },
        tx,
    );
    let mut texts: Vec<String> = collect_items(&rx, query)
        .iter()
        .map(|item| item.text.clone())
        .collect();
    texts.sort();
    texts
}

/// 0063 residual (0058 S7): with push coverage, an idle second search
/// reuses every unchanged file and every unchanged catalog subtree —
/// no full rescan — and publishes the identical rows.
#[test]
fn unchanged_scope_is_reused_not_rescanned() {
    let fixture = mixed_fixture();
    let root = fixture.path();
    let worker = SourceWorker::new().unwrap();
    worker.set_watching(root, true);
    let first = workspace_symbol_texts(&worker, root, "parser");
    let stats = worker.scan_stats(root).expect("a settled scan");
    assert!(stats.full, "the first scan builds the baseline");
    assert!(stats.symbols.parsed > 0, "the fixture has eligible files");
    assert_eq!(stats.symbols.reused, 0);

    let second = workspace_symbol_texts(&worker, root, "parser");
    assert_eq!(first, second, "reuse never changes the rows");
    let stats = worker.scan_stats(root).unwrap();
    assert!(!stats.full, "the second scan refreshes incrementally");
    assert_eq!(
        stats.symbols.parsed, 0,
        "no file changed: nothing re-parses ({} reused)",
        stats.symbols.reused
    );
    assert!(stats.symbols.reused > 0);
    assert!(
        stats.catalog.reused > 0,
        "unchanged catalog subtrees are reused: {stats:?}"
    );
}

/// 0063 residual (0058 S7): a write during an idle period plus its
/// notification hint makes the next search reflect the change WITHOUT
/// a full rescan — only the invalidated file re-parses.
#[test]
fn a_hinted_write_rescans_only_the_invalidated_file() {
    let fixture = mixed_fixture();
    let root = fixture.path();
    let worker = SourceWorker::new().unwrap();
    worker.set_watching(root, true);
    let named = |rows: &[String]| {
        rows.iter()
            .filter(|row| row.contains("fresh_symbol_after_write"))
            .count()
    };
    let before = workspace_symbol_texts(&worker, root, "parser");
    assert_eq!(named(&before), 0, "the symbol does not exist yet");

    // The external write lands between searches (idle period), then its
    // hint arrives, exactly as the editor applies it.
    std::fs::write(
        root.join("engine/src/lib.rs"),
        "fn parser() {\n    let retry = request;\n}\nfn fresh_symbol_after_write() {}\nlet bare_parser = 1;\n",
    )
    .unwrap();
    worker.invalidate(
        root,
        &[std::path::PathBuf::from("engine/src/lib.rs")],
        false,
    );

    let after = workspace_symbol_texts(&worker, root, "parser");
    assert_eq!(named(&after), 1, "the next search sees the new declaration");
    assert!(after
        .iter()
        .any(|row| row.contains("fresh_symbol_after_write") && row.contains("engine/src/lib.rs")));
    let stats = worker.scan_stats(root).unwrap();
    assert!(!stats.full);
    assert_eq!(
        stats.symbols.parsed, 1,
        "only the hinted file re-parses: {stats:?}"
    );
    assert!(stats.symbols.reused > 0, "the rest is reused: {stats:?}");
    assert!(
        stats.catalog.reused > 0 && stats.catalog.visited < 5,
        "the catalog prunes untouched subtrees: {stats:?}"
    );
}

/// 0063 residual: a missed hint self-heals — the refresh's stat
/// observation notices the write even without invalidation, and only
/// that file re-parses. Watching is never the only cache-validity
/// mechanism.
#[test]
fn a_missed_hint_self_heals_by_observation() {
    let fixture = mixed_fixture();
    let root = fixture.path();
    let worker = SourceWorker::new().unwrap();
    worker.set_watching(root, true);
    let named = |rows: &[String]| {
        rows.iter()
            .filter(|row| row.contains("observed_without_hint"))
            .count()
    };
    let before = workspace_symbol_texts(&worker, root, "parser");
    assert_eq!(named(&before), 0);

    std::fs::write(
        root.join("tools/mod.py"),
        "class Cfg:\n    def parser(self):\n        pass\n\ndef observed_without_hint():\n    pass\n",
    )
    .unwrap();
    // No invalidate() call: the hint was lost.

    let after = workspace_symbol_texts(&worker, root, "parser");
    assert_eq!(named(&after), 1, "observation, not the hint, heals it");
    let stats = worker.scan_stats(root).unwrap();
    assert_eq!(
        stats.symbols.parsed, 1,
        "only the stat-mismatched file re-parses: {stats:?}"
    );
}

/// 0063 residual (0058 S7): overflow/loss is a conservative rescan
/// obligation — the next search rebuilds the whole baseline instead of
/// trusting retained state.
#[test]
fn overflow_forces_a_full_rescan() {
    let fixture = mixed_fixture();
    let root = fixture.path();
    let worker = SourceWorker::new().unwrap();
    worker.set_watching(root, true);
    workspace_symbol_texts(&worker, root, "parser");
    worker.invalidate(root, &[], true);

    std::fs::write(
        root.join("loose/overflow_marker.rs"),
        "fn landed_during_overflow() {}\n",
    )
    .unwrap();
    let rows = workspace_symbol_texts(&worker, root, "parser");
    assert!(
        rows.iter()
            .any(|row| row.contains("landed_during_overflow")),
        "the rescan observes the overflow window"
    );
    let stats = worker.scan_stats(root).unwrap();
    assert!(stats.full, "the rescan obligation rebuilt: {stats:?}");
    assert_eq!(stats.symbols.reused, 0);
}

/// 0063 residual (0058 S7 honest degradation): without a subscription
/// every search scans fresh — the pre-notification behavior — and a
/// write is still reflected without any invalidation call.
#[test]
fn without_coverage_every_search_scans_fresh() {
    let fixture = mixed_fixture();
    let root = fixture.path();
    let worker = SourceWorker::new().unwrap();
    workspace_symbol_texts(&worker, root, "parser");
    let stats = worker.scan_stats(root).unwrap();
    assert!(stats.full);
    assert_eq!(stats.symbols.reused, 0, "no coverage, no reuse");

    std::fs::write(
        root.join("loose/game.lua"),
        "function mod:parser() end\nfunction unwatched_write_visible() end\n",
    )
    .unwrap();
    let rows = workspace_symbol_texts(&worker, root, "parser");
    assert!(
        rows.iter()
            .any(|row| row.contains("unwatched_write_visible")),
        "the per-search scan sees it"
    );
    let stats = worker.scan_stats(root).unwrap();
    assert_eq!(stats.symbols.reused, 0, "still no reuse: {stats:?}");
}
