//! The disk pass of a local content search (0093 §4): the selected files
//! are searched on several threads, each claiming the next few paths.
//! Rows leave in whatever order files finish; the ranking pass puts them
//! in tree order. The first failure or a cancellation stops every thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use grep::regex::{RegexMatcher, RegexMatcherBuilder};
use grep::searcher::{sinks::UTF8, Searcher, SearcherBuilder};
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};

use super::super::flow::StreamSender;
use crate::query::{CaseMode, ContentPlan, Evidence};
use crate::source::{catalog::ProjectCatalog, symbols::SymbolIndex};

/// ripgrep stops adding threads at twelve; past eight a warm tree is
/// bound by memory, not cores, and an editor shares the machine.
const MAX_THREADS: usize = 8;
/// Paths one thread claims at a time: few enough that hits arrive close
/// to tree order, enough that the claim counter is never contended.
const CLAIM: usize = 8;
/// Rows one delivery carries before a file with many hits flushes.
const BATCH: usize = 512;

pub(super) fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(MAX_THREADS)
}

pub(super) struct Scope<'a> {
    pub cwd: &'a Path,
    pub content: &'a ContentPlan,
    pub catalog: Option<&'a ProjectCatalog>,
    pub symbols: Option<&'a SymbolIndex>,
    pub tx: &'a StreamSender,
    pub cancel: &'a CancelToken,
}

pub(super) fn search(scope: &Scope<'_>, paths: &[PathBuf], threads: usize) -> Outcome<()> {
    let content = scope.content;
    let mut matcher = RegexMatcherBuilder::new();
    matcher
        .case_smart(content.provider_case() == CaseMode::Smart)
        .case_insensitive(content.provider_case() == CaseMode::Ignore)
        .fixed_strings(content.is_literal());
    let matcher = match matcher.build(content.provider_pattern()) {
        Ok(matcher) => matcher,
        Err(error) => {
            return Outcome::failed(FailureKind::Protocol, format!("invalid regex: {error}"))
        }
    };
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failed = parking_lot::Mutex::new(None);
    let work = || {
        let mut searcher = SearcherBuilder::new().line_number(true).build();
        loop {
            let start = next.fetch_add(CLAIM, Ordering::Relaxed);
            if start >= paths.len() {
                return;
            }
            for path in &paths[start..paths.len().min(start + CLAIM)] {
                scope.tx.heartbeat();
                if stop.load(Ordering::Acquire) || scope.cancel.is_cancelled() {
                    return;
                }
                if let Err(outcome) = search_file(scope, &matcher, &mut searcher, path) {
                    stop.store(true, Ordering::Release);
                    failed.lock().get_or_insert(outcome);
                    return;
                }
            }
        }
    };
    std::thread::scope(|spawn| {
        for _ in 1..threads.min(paths.len().div_ceil(CLAIM)) {
            spawn.spawn(work);
        }
        work();
    });
    if let Some(outcome) = failed.into_inner() {
        return outcome;
    }
    if scope.cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    Outcome::Success(())
}

/// One file's hits, delivered together. Every hit line goes through the
/// exact plan: the provider pattern only prefilters (0063 §4).
fn search_file(
    scope: &Scope<'_>,
    matcher: &RegexMatcher,
    searcher: &mut Searcher,
    path: &Path,
) -> Result<(), Outcome<()>> {
    let Scope {
        cwd,
        content,
        tx,
        cancel,
        ..
    } = *scope;
    let location = strop_workspace::ResourceLocation::local(cwd.join(path));
    let relative = path.to_string_lossy();
    let display = strop_workspace::directory::display_path(path);
    let mut terminal = None;
    let mut items = Vec::new();
    let mut sink = UTF8(|line, text| {
        if cancel.is_cancelled() {
            return Ok(false);
        }
        if text.len() > super::super::query::RECORD_LIMIT {
            terminal = Some(Outcome::failed(
                FailureKind::Protocol,
                format!("{display}:{line}: source line exceeds the 1 MiB search bound"),
            ));
            return Ok(false);
        }
        let text: std::sync::Arc<str> = text.strip_suffix('\n').unwrap_or(text).into();
        if !content.admits(
            Evidence {
                catalog: scope.catalog,
                symbols: scope.symbols,
                path: Some(&relative),
                line: Some(line as usize),
                symbol: None,
            },
            &text,
        ) {
            return Ok(true);
        }
        let short: String = text.trim().chars().take(80).collect();
        let first = items.len();
        let mut expanded = text.len();
        for hit in content.regex.find_iter(&text).take(4097) {
            if items.len() - first == 4096 {
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
            expanded = expanded.saturating_add(super::super::flow::row_bytes(&item));
            if expanded > super::super::flow::BACKLOG_BYTES {
                terminal = Some(Outcome::failed(
                    FailureKind::Protocol,
                    format!("{display}:{line}: source line exceeds 4 MiB of result rows"),
                ));
                return Ok(false);
            }
            items.push(item);
        }
        if items.len() >= BATCH {
            if let Err(error) = tx.batch(std::mem::take(&mut items), cancel) {
                terminal = Some(error.finish(tx));
                return Ok(false);
            }
        }
        Ok(true)
    });
    let searched = searcher.search_path(matcher, cwd.join(path), &mut sink);
    if let Some(outcome) = terminal {
        return Err(outcome);
    }
    if cancel.is_cancelled() {
        return Err(Outcome::Cancelled(CancelReason::OwnerClosed));
    }
    if let Err(error) = searched {
        return Err(Outcome::failed(
            FailureKind::Io,
            format!("{display}: {error}"),
        ));
    }
    tx.batch(items, cancel).map_err(|error| error.finish(tx))
}
