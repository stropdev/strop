//! Streaming sources. Workers run on the shared terminal worker
//! (`strop_core::worker`): every launch — success, failure, panic or
//! cancellation — settles exactly once with a terminal `Finished`
//! (0001 §5.6: input never waits on a source; R9: no silent empty
//! successes, no stream that never ends).

mod cache;
pub use cache::InventoryRoot;
mod grep;
#[cfg(test)]
mod lifecycle_traces;
mod query;
mod remote;
mod snapshots;
mod worker;
pub use worker::{SearchHost, SourceWorker};

mod flow;
use flow::StreamSender;
pub use flow::{InventoryGuard, ItemBatch, SourceSink};
pub use snapshots::SourceSnapshot;

use std::path::PathBuf;

use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};

use crate::{Item, Payload, SymbolRow};

/// Messages workers post to the event loop.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PickerMsg {
    Items(ItemBatch),
    /// A successful source can still issue a useful warning (rg's
    /// stderr on exit 0): it lands in the picker, not the void.
    Warning(String),
    /// Rows may open individually, but cannot authorize complete-only actions.
    PartialCoverage,
    /// Independent native root/language proposals for the existing resolver.
    ScopeProjects(Vec<catalog::WarmProject>),
    QueryError(crate::query::QueryDiagnostic),
    /// Terminal: exactly one per request, on every path — success,
    /// failure (keeping whatever items already streamed), panic or
    /// cancellation. Streaming stops here.
    Finished(Outcome<()>),
}

pub mod catalog;
pub mod selection;
pub mod symbols;

pub use selection::{display_path, SelectionPolicy, RG_PROTECTED_ARGS};

/// Walk the working directory through the shared selection authority
/// (0051 §3): one FileSelectionPlan filters candidates for every
/// surface; dotfiles show by default; `.git` is never a result. A walk
/// error terminates visibly after the batches already streamed — never
/// as a silent empty success.
fn run_files(
    cwd: PathBuf,
    query: std::sync::Arc<crate::query::SearchQuery>,
    policy: selection::SelectionPolicy,
    tx: StreamSender,
    cancel: CancelToken,
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
    if query.exact_expression {
        if let Err(diagnostic) = crate::query::ContentPlan::compile(&query) {
            let message = diagnostic.message.clone();
            let _ = tx.control(PickerMsg::QueryError(diagnostic));
            return Outcome::failed(FailureKind::Protocol, message);
        }
    }
    let mut batch = Vec::with_capacity(512);
    let mut delivery_error = None;
    let cancelled = || cancel.is_cancelled();
    let result = selection::walk(&cwd, &plan, policy, &cancelled, |rel| {
        batch.push(Item {
            badge: None,
            text: display_path(&rel).into_owned(),
            payload: Payload::File(rel),
            facts: Default::default(),
        });
        if batch.len() >= 512 {
            return match tx.batch(std::mem::take(&mut batch), &cancel) {
                Ok(()) => true,
                Err(error) => {
                    delivery_error = Some(error);
                    false
                }
            };
        }
        true
    });
    if let Some(error) = delivery_error {
        return error.finish(&tx);
    }
    if let Err(error) = result {
        if !batch.is_empty() {
            if let Err(error) = tx.batch(std::mem::take(&mut batch), &cancel) {
                return error.finish(&tx);
            }
        }
        return Outcome::failed(FailureKind::Io, error);
    }
    if let Err(error) = tx.batch(batch, &cancel) {
        return error.finish(&tx);
    }
    Outcome::Success(())
}

/// Every declaration in the opened scope (0063 §2), syntax-fallback
/// tier: one walk through the shared selection authority, one bounded
/// symbol index, one row per declaration. Coverage is honest — the
/// index's bounds surface as a visible warning, never as silence. The
/// rows carry their facts; the ranking pass applies the row filters to
/// them and to the servers' rows alike (0109 §3.2d).
fn run_workspace_symbols(
    cwd: PathBuf,
    query: std::sync::Arc<crate::query::SearchQuery>,
    policy: selection::SelectionPolicy,
    snapshots: Vec<SourceSnapshot>,
    cache: std::sync::Arc<parking_lot::Mutex<cache::Caches>>,
    tx: StreamSender,
    cancel: CancelToken,
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
    let cancelled = || cancel.is_cancelled() || !tx.inventory_current();
    let inventory = match cache::inventory(
        &cache,
        cache::ScanRequest {
            root: &cwd,
            query: &query,
            plan: &plan,
            policy,
            need_symbols: true,
        },
        1,
        &cancel,
        &|| tx.heartbeat(),
    ) {
        Ok(inventory) => inventory,
        Err(error) => return error.outcome("workspace symbols"),
    };
    if !tx.bind_inventory(inventory.epoch) {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    let Some(mut index) = inventory.symbols else {
        return Outcome::failed(
            FailureKind::Protocol,
            "workspace inventory returned no syntax producer",
        );
    };
    let catalog = inventory.catalog;
    let mut extractors = symbols::Extractors::default();
    for snapshot in snapshots {
        let Ok(relative) = snapshot.path.strip_prefix(&cwd) else {
            continue;
        };
        if inventory.paths.iter().any(|p| p == relative) {
            std::sync::Arc::make_mut(&mut index).overlay(
                relative,
                &snapshot.text,
                &mut extractors,
                &cancelled,
            );
        }
    }
    tx.workspace_scope(inventory.paths, catalog.clone(), index.clone());
    let (warm, limited) = catalog.warm_projects();
    if limited {
        tx.mark_partial();
        let _ = tx.control(PickerMsg::Warning(
            "project discovery or warm-up is bounded; semantic coverage is partial".into(),
        ));
    }
    let _ = tx.control(PickerMsg::ScopeProjects(warm));
    let root = strop_workspace::ResourceLocation::local(cwd);
    let mut batch = Vec::with_capacity(512);
    let mut first = true;
    // Deterministic rows: path order, then declaration order in file.
    let mut entries: Vec<_> = index.iter().collect();
    entries.sort_by_key(|(path, _)| *path);
    for (relative, declarations) in entries {
        let shown = display_path(relative);
        let shared: std::sync::Arc<str> = shown.as_ref().into();
        for declaration in declarations {
            if cancelled() {
                return Outcome::Cancelled(CancelReason::OwnerClosed);
            }
            // An impl block declares nothing new: like rust-analyzer's
            // workspace/symbol, the list is what it declares. It still
            // answers `kind:impl` for a line inside it (0107).
            if declaration.kind == strop_core::symbol_kind::SymbolKind::Impl {
                continue;
            }
            let (row, column) = declaration.selection[0];
            batch.push(Item::symbol(SymbolRow {
                name: &declaration.name,
                container: None,
                detail: format_args!("{shown} · :{}", row as usize + 1),
                kind: declaration.kind,
                path: Some(shared.clone()),
                payload: Payload::Symbol {
                    location: strop_workspace::ResourceLocation {
                        filesystem: root.filesystem.clone(),
                        path: root.path.join(relative),
                    },
                    line: strop_core::id::LineIndex::new(row as usize),
                    column: crate::SymbolColumn::Byte(strop_core::id::ByteColumn::new(
                        column as usize,
                    )),
                },
            }));
            if batch.len() >= if first { 32 } else { 512 } {
                if let Err(error) = tx.batch(std::mem::take(&mut batch), &cancel) {
                    return error.finish(&tx);
                }
                first = false;
            }
        }
    }
    if let Err(error) = tx.batch(batch, &cancel) {
        return error.finish(&tx);
    }
    if index.coverage_gap() {
        tx.mark_partial();
        let _ = tx.control(PickerMsg::Warning(
            "workspace symbols: incomplete syntax coverage (bound, read or extraction failure)"
                .into(),
        ));
    }
    Outcome::Success(())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    #[test]
    fn files_worker_walks() {
        let dir = std::env::temp_dir().join("strop-picker-walk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.rs"), "").unwrap();
        let (tx, rx) = channel();
        let worker = super::SourceWorker::new().unwrap();
        let _request = worker.files(
            dir.clone(),
            std::sync::Arc::new(crate::query::SearchQuery::default()),
            super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            tx,
        );
        let mut found = false;
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(super::PickerMsg::Items(batch)) => {
                    if batch.iter().any(|i| i.text.contains("x.rs")) {
                        found = true;
                    }
                }
                Ok(super::PickerMsg::Warning(_)) => {}
                Ok(super::PickerMsg::Finished(super::Outcome::Success(()))) => done = true,
                Ok(other) => panic!("unexpected terminal: {other:?}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
        assert!(found, "walked file was streamed");
        assert!(done, "walk settled with a terminal event");
    }

    #[test]
    fn workspace_symbols_worker_lists_declarations_with_chips() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "struct St;\nfn wrap() {\n    let x = 1;\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "fn not rust\n").unwrap();
        std::fs::create_dir_all(dir.path().join("engine/src")).unwrap();
        std::fs::write(
            dir.path().join("engine/Cargo.toml"),
            "[package]\nname = \"engine\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("engine/src/lib.rs"),
            "pub fn engine_fn() {}\n",
        )
        .unwrap();
        let (tx, rx) = channel();
        let worker = super::SourceWorker::new().unwrap();
        let _request = worker.workspace_symbols(
            dir.path().to_path_buf(),
            std::sync::Arc::new(crate::query::SearchQuery::default()),
            super::selection::SelectionPolicy {
                hidden: true,
                respect_ignore: true,
            },
            Vec::new(),
            tx,
        );
        let mut rows: Vec<(Option<String>, String)> = Vec::new();
        let mut scope = None;
        let mut done = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done && std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(super::PickerMsg::Items(batch)) => rows.extend(
                    batch
                        .iter()
                        .map(|item| (item.badge.as_deref().map(str::to_owned), item.text.clone())),
                ),
                Ok(super::PickerMsg::Warning(_)) => {}
                Ok(super::PickerMsg::ScopeProjects(projects)) => scope = Some(projects),
                Ok(super::PickerMsg::Finished(super::Outcome::Success(()))) => done = true,
                Ok(other) => panic!("unexpected terminal: {other:?}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        drop(worker);
        assert!(done, "source settled with a terminal event");
        // Symbol-row convention: "name  container · :line" with the
        // kind chip in the badge column (0047 §1 / 0063 §2).
        assert!(rows.contains(&(Some("struct".into()), "St  lib.rs · :1".into())));
        assert!(rows.contains(&(Some("fn".into()), "wrap  lib.rs · :2".into())));
        assert!(
            !rows.iter().any(|(_, text)| text.contains("notes.txt")),
            "non-source files contribute no rows"
        );
        // The scope worker reports the native root and inferred language.
        let scope = scope.expect("scope projects message");
        assert_eq!(
            scope,
            vec![super::catalog::WarmProject {
                root: dir.path().join("engine"),
                hint: super::catalog::WarmHint::Language("rust".into()),
            }],
            "one candidate for the observed project/language"
        );
    }

    #[test]
    fn workspace_symbol_rows_carry_the_facts_the_ranking_pass_filters() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "struct St;\nimpl St {\n    fn method(&self) {}\n}\nfn wrap() {}\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("engine/src")).unwrap();
        std::fs::write(
            dir.path().join("engine/src/lib.rs"),
            "pub fn engine_fn() {}\npub struct EngineSt;\n",
        )
        .unwrap();
        let run = |input: &str| -> Vec<crate::Item> {
            let (tx, rx) = channel();
            let worker = super::SourceWorker::new().unwrap();
            let _request = worker.workspace_symbols(
                dir.path().to_path_buf(),
                std::sync::Arc::new(crate::query::parse_search(input)),
                super::selection::SelectionPolicy {
                    hidden: true,
                    respect_ignore: true,
                },
                Vec::new(),
                tx,
            );
            let mut rows = Vec::new();
            let mut done = false;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !done && std::time::Instant::now() < deadline {
                match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                    Ok(super::PickerMsg::Items(batch)) => rows.extend(batch.iter().cloned()),
                    Ok(super::PickerMsg::Warning(_) | super::PickerMsg::ScopeProjects(_)) => {}
                    Ok(super::PickerMsg::Finished(super::Outcome::Success(()))) => done = true,
                    Ok(other) => panic!("unexpected terminal: {other:?}"),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            drop(worker);
            assert!(done, "source settled with a terminal event");
            rows
        };
        // The walk lists every declaration whatever the row filters;
        // only the walk's scope (`path:` and kin) prunes it.
        let all = run("kind:function wrap");
        assert_eq!(
            all.len(),
            5,
            "{:?}",
            all.iter().map(|i| &i.text).collect::<Vec<_>>()
        );
        assert_eq!(run("path:engine").len(), 2);
        // Each row names itself, its kind and its path for the ranking
        // pass; `kind:` then narrows exactly, methods under function.
        let query = crate::query::parse_search("kind:function");
        let admission = crate::query::Admission::compile(&query).unwrap().unwrap();
        let kept: Vec<&str> = all
            .iter()
            .filter(|item| admission.admits(&item.facts(), &Default::default()))
            .map(|item| item.facts().text)
            .collect();
        assert_eq!(kept, ["engine_fn", "method", "wrap"]);
        let wrap = all.iter().find(|item| item.facts().text == "wrap").unwrap();
        assert_eq!(wrap.facts().path, Some("lib.rs"));
        assert_eq!(wrap.text, "wrap  lib.rs · :5");
    }
}
