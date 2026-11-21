//! Exact disk verification shared by streaming selection and the gathered
//! evidence path. Both feed the same ContentPlan; ranking owns final order.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use grep::regex::{RegexMatcher, RegexMatcherBuilder};
use grep::searcher::{sinks::UTF8, Searcher, SearcherBuilder};
use strop_core::worker::{CancelReason, CancelToken, FailureKind, Outcome};

use super::super::flow::StreamSender;
use crate::query::{CaseMode, ContentPlan, Evidence, RowFacts};
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

pub(super) fn matcher(content: &ContentPlan) -> Result<RegexMatcher, Outcome<()>> {
    let mut matcher = RegexMatcherBuilder::new();
    matcher
        .case_smart(content.provider_case() == CaseMode::Smart)
        .case_insensitive(content.provider_case() == CaseMode::Ignore)
        .fixed_strings(content.is_literal());
    matcher
        .build(content.provider_pattern())
        .map_err(|error| Outcome::failed(FailureKind::Protocol, format!("invalid regex: {error}")))
}

pub(super) fn search(scope: &Scope<'_>, paths: &[PathBuf], threads: usize) -> Outcome<()> {
    let matcher = match matcher(scope.content) {
        Ok(matcher) => matcher,
        Err(outcome) => return outcome,
    };
    let send = |items| {
        scope
            .tx
            .batch(items, scope.cancel)
            .map_err(|error| error.finish(scope.tx))
    };
    let first = AtomicBool::new(true);
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
                if let Err(outcome) =
                    search_file(scope, &matcher, &mut searcher, path, &first, &send)
                {
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
pub(super) fn search_file(
    scope: &Scope<'_>,
    matcher: &RegexMatcher,
    searcher: &mut Searcher,
    path: &Path,
    first: &AtomicBool,
    send: &impl Fn(Vec<crate::Item>) -> Result<(), Outcome<()>>,
) -> Result<bool, Outcome<()>> {
    let Scope {
        cwd,
        content,
        tx,
        cancel,
        ..
    } = *scope;
    let location = strop_workspace::ResourceLocation::local(cwd.join(path));
    let relative = path.to_string_lossy();
    let label = std::cell::OnceCell::new();
    let display = || label.get_or_init(|| strop_workspace::directory::display_path(path));
    let mut terminal = None;
    let mut items = Vec::new();
    let mut matched = false;
    let mut sink = UTF8(|line, text| {
        if cancel.is_cancelled() || !tx.inventory_current() {
            return Ok(false);
        }
        if text.len() > super::super::query::RECORD_LIMIT {
            terminal = Some(Outcome::failed(
                FailureKind::Protocol,
                format!(
                    "{}:{line}: source line exceeds the 1 MiB search bound",
                    display()
                ),
            ));
            return Ok(false);
        }
        let text = text.strip_suffix('\n').unwrap_or(text);
        let row = RowFacts {
            path: Some(&relative),
            native_path: Some(path),
            line: Some(line as usize),
            text,
            ..RowFacts::default()
        };
        let evidence = Evidence {
            catalog: scope.catalog,
            symbols: scope.symbols,
        };
        if !content.admits(&row, &evidence) {
            return Ok(true);
        }
        let text = std::sync::Arc::from(text);
        let previous = items.len();
        if let Err(error) = super::super::query::append_matches(
            &mut items,
            content,
            &location,
            display(),
            line as usize,
            &text,
            None,
        ) {
            terminal = Some(Outcome::failed(FailureKind::Protocol, error));
            return Ok(false);
        }
        matched |= items.len() > previous;
        if items.len() >= BATCH
            || (!items.is_empty()
                && first.load(Ordering::Relaxed)
                && first.swap(false, Ordering::Relaxed))
        {
            if let Err(error) = send(std::mem::take(&mut items)) {
                terminal = Some(error);
                return Ok(false);
            }
        }
        Ok(true)
    });
    // A kind decision and its match must see the same observation: the
    // pathname could change between extraction and this worker's turn.
    let searched = match scope.symbols.and_then(|index| index.disk_shard(path)) {
        Some(shard) => searcher.search_slice(matcher, shard.source(), &mut sink),
        None => searcher.search_path(matcher, &location.path, &mut sink),
    };
    if let Some(outcome) = terminal {
        return Err(outcome);
    }
    if cancel.is_cancelled() {
        return Err(Outcome::Cancelled(CancelReason::OwnerClosed));
    }
    if let Err(error) = searched {
        return Err(Outcome::failed(
            FailureKind::Io,
            format!("{}: {error}", display()),
        ));
    }
    if !items.is_empty() {
        send(items)?;
    }
    Ok(matched)
}
