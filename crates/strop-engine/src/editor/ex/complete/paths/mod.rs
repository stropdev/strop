//! Path completion for file and directory arguments, local and SSH.
//! Listings run on workers, fetched only on `Tab`/`Ctrl-D`; typing then
//! refilters the landed listing with no new job. SSH paths use only
//! existing live connections or bounded cached observations; `Tab`
//! never authenticates.
//!
//! Every reply captures the exact line, focus, document revision and
//! cursor; a late reply for an older moment is dropped. Candidates are
//! lossless resource URIs so native filename bytes cannot become display
//! aliases.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_remote::{HostCandidate, HostSources, RemoteClient, RemoteEntryKind};
use strop_workspace::RemoteFile;

use super::super::table::ExCommand;
use super::{rank, MenuKind, Row, Text};
use crate::editor::cmdline::Context;
use crate::editor::document::DocumentSource;
use crate::editor::Editor;

mod directory;
mod run;
use run::run_completion;
#[cfg(test)]
mod tests;

/// Bounded fallback cache: successful live listings remembered so a
/// later offline Tab still completes from the last observed truth.
const CACHE_DIRS: usize = 32;

/// Commands that open what they name, the only ones an `ssh://`
/// argument completes for; the `:w` family refuses instead.
fn opens_remote(command: &ExCommand) -> Option<bool> {
    match command.name() {
        "edit" | "view" | "split" | "vsplit" => Some(true),
        "write" | "wq" | "xit" | "update" => Some(false),
        _ => None,
    }
}

/// Which local question a completion asked. Pure serde data — the
/// replayable half of the exchange; worker inputs (sources, history,
/// the client) are live values and never serialize.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RemoteCompletionQuery {
    /// Complete the endpoint token typed after `ssh://` (no `/` yet).
    Hosts { partial: String },
    /// Complete the final path segment of `ssh://<authority>/dir/seg…`
    /// against a live read-only connection. `directory` is URI text
    /// with a leading `/` (the canonical listing target is derived and
    /// validated through `RemoteFile::parse`).
    Path {
        authority: String,
        directory: String,
        segment: String,
    },
    Directory {
        location: strop_workspace::ResourceLocation,
        segment: Vec<u8>,
        /// The directory part exactly as typed, when the operand is a
        /// plain relative path: candidates keep that spelling instead of
        /// turning into absolute URIs (0093 §6).
        #[serde(default)]
        typed: Option<String>,
    },
}

impl RemoteCompletionQuery {
    fn label(&self) -> &'static str {
        match self {
            Self::Hosts { .. } => "hosts",
            Self::Path { .. } => "path",
            Self::Directory { .. } => "directory",
        }
    }
}

/// One completion answer item: the canonical URI text that replaces
/// the typed `ssh://…` token. Directories carry a trailing `/` so the
/// next Tab descends into them; files are complete URIs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteCandidate {
    pub uri: String,
    pub directory: bool,
}

/// Where candidates came from — surfaced so cache is never mistaken
/// for live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CandidateSource {
    Config,
    Connection,
    Cache,
    Directory,
}

/// The typed moment a request owns; every delivery re-checks it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteCompletionKey {
    pub focus: u64,
    pub document: DocumentId,
    pub revision: BufferRevision,
    /// Full prompt text (sigil included) at request time.
    pub text: String,
    pub cursor: usize,
    pub query: RemoteCompletionQuery,
    pub prefix_body: String,
}

/// The worker's terminal answer. Failures travel as
/// `Outcome::Failed`; this carries only successes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum RemoteCompletionResult {
    Candidates {
        items: Vec<RemoteCandidate>,
        source: CandidateSource,
        /// Enumeration diagnostics (bounded) shown when nothing matched.
        notes: Vec<String>,
        /// Canonical URI of the listed directory, for the fallback
        /// cache. None for host completions.
        listed_directory: Option<String>,
    },
    /// Path completion found no live connection. Connecting is an
    /// explicit user action (open/browse); completion must not take it.
    ConnectRequired { endpoint: String },
}

/// One worker delivery, ticket-stamped like every other service.
pub type RemoteCompletionEvent = Completion<RemoteCompletionKey, RemoteCompletionResult>;

/// A landed listing, refiltered as the user types in the same directory.
#[derive(Debug, Clone)]
pub(crate) struct Listing {
    /// Body offset of the completed token.
    start: usize,
    /// The token up to the segment the listing filters (`src/`,
    /// `ssh://build/var/`, `ssh://`).
    base: String,
    /// The segment the worker filtered by; typing past it refilters.
    segment: String,
    items: Vec<RemoteCandidate>,
    /// The worker's answer is in.
    landed: bool,
    /// `:cd` lists directories only.
    directories: bool,
}

impl Listing {
    /// The rows for `token` at `start`, or false when this listing does
    /// not cover it (another directory, a shorter segment).
    pub(crate) fn rows(&self, start: usize, token: &str, rows: &mut Vec<Row>) -> bool {
        let Some(segment) = token.strip_prefix(self.base.as_str()) else {
            return false;
        };
        if !self.landed
            || start != self.start
            || segment.contains('/')
            || !segment.starts_with(&self.segment)
        {
            return false;
        }
        // the worker filtered by the queried segment (natively); typing
        // past it narrows by the shown name
        let narrowed = segment != self.segment;
        rows.clear();
        let matched = rank::prefix_mask(segment.chars().count());
        for item in &self.items {
            if self.directories && !item.directory {
                continue;
            }
            let label = label(&item.uri);
            if !narrowed || label.starts_with(segment) {
                rows.push(Row {
                    text: Text::Owned(item.uri.as_str().into()),
                    label: Some(Text::Owned(label.into())),
                    abbrev: Text::Static(""),
                    doc: Text::Static(""),
                    key: None,
                    matched,
                    mark: false,
                });
            }
        }
        true
    }

    /// The menu's title: the directory listed.
    fn title(&self) -> Text {
        match self.base.as_str() {
            "" => Text::Static("files"),
            "ssh://" => Text::Static("hosts"),
            base => Text::Owned(base.into()),
        }
    }
}

/// `token` cut where the listing's filter begins.
fn base_of(token: &str) -> (&str, &str) {
    let split = match token.strip_prefix("ssh://") {
        Some(rest) if !rest.contains('/') => "ssh://".len(),
        _ => token.rfind('/').map_or(0, |at| at + 1),
    };
    token.split_at(split)
}

/// The editor's remote-completion slot: one in-flight request, one
/// landed cycle, one bounded fallback cache, one delivery channel.
#[derive(Debug)]
pub(crate) struct RemoteCompletionState {
    /// Worker deliveries; the TUI forwards this onto the app channel
    /// (Main wires `AppEvent::RemoteCompletion`), headless drains it.
    pub tx: Sender<RemoteCompletionEvent>,
    pub rx: Option<Receiver<RemoteCompletionEvent>>,
    pub(crate) pending: Option<Ticket<RemoteCompletionKey>>,
    cache: VecDeque<(String, Vec<RemoteCandidate>)>,
}

impl Default for RemoteCompletionState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Some(rx),
            pending: None,
            cache: VecDeque::new(),
        }
    }
}

impl RemoteCompletionState {
    fn cached(&self, canonical_dir: &str) -> Option<Vec<RemoteCandidate>> {
        self.cache
            .iter()
            .rev()
            .find(|(key, _)| key == canonical_dir)
            .map(|(_, items)| items.clone())
    }

    fn store_cache(&mut self, canonical_dir: String, items: Vec<RemoteCandidate>) {
        if items.is_empty() {
            return;
        }
        self.cache.retain(|(key, _)| key != &canonical_dir);
        self.cache.push_back((canonical_dir, items));
        while self.cache.len() > CACHE_DIRS {
            self.cache.pop_front();
        }
    }

    #[cfg(test)]
    fn ticket(&self) -> Option<Ticket<RemoteCompletionKey>> {
        self.pending.clone()
    }
}

impl Editor {
    fn revoke_path_completion(&mut self) {
        if let Some(old) = self.remote_completion.pending.take() {
            if let Some(handle) = self.worker_handles.remove(&old.request) {
                handle.cancel(CancelReason::Superseded);
            }
        }
        self.cmdline.completion.listing = None;
    }

    pub(crate) fn invalidate_filesystem_completions(&mut self) {
        self.revoke_path_completion();
        self.remote_completion.cache.clear();
    }

    /// Complete the path argument at `[start, end)` of the body: from the
    /// landed listing when it covers what is typed, else (on `Tab`/
    /// `Ctrl-D`) from a new listing job.
    pub(in crate::editor::ex::complete) fn path_complete(
        &mut self,
        command: &'static ExCommand,
        directories: bool,
        start: usize,
        end: usize,
        explicit: bool,
    ) {
        let (line, completion) = self.cmdline.parts_mut();
        let Some(line) = line else { return };
        let body = line.body();
        let token = &body[start..end];
        if let Some(listing) = completion.listing.take() {
            let covered = listing.rows(start, token, &mut completion.rows);
            let title = listing.title();
            completion.listing = Some(listing);
            if covered {
                completion.show(MenuKind::Paths, title, body, start, end);
                return;
            }
        }
        completion.close();
        if !explicit {
            return;
        }
        let prefix_body = body[..start].to_owned();
        let token = body[start..end].to_owned();
        let token = token.as_str();
        let query = if let Some(remote) = token.strip_prefix("ssh://") {
            if token.contains(char::is_whitespace) {
                self.message = "remote URI cannot contain a raw space (type %20)".into();
                return;
            }
            match opens_remote(command) {
                Some(true) => classify_remote_operand(remote),
                Some(false) => {
                    self.message = "remote save-as completion is unsupported".into();
                    return;
                }
                None => return,
            }
        } else if token.contains(" ssh://") {
            self.message = format!(":{} takes no argument before the URI", command.name());
            return;
        } else {
            directory::classify(token, self.open_context())
        };
        match query {
            Ok(query) => {
                let (base, segment) = base_of(token);
                let listing = Listing {
                    start,
                    base: base.to_owned(),
                    segment: segment.to_owned(),
                    items: Vec::new(),
                    landed: false,
                    directories,
                };
                self.start_remote_completion(query, prefix_body, listing);
            }
            Err(error) => self.message = error,
        }
    }

    /// Classify the typed operand and launch the owned worker request.
    fn start_remote_completion(
        &mut self,
        query: RemoteCompletionQuery,
        prefix_body: String,
        listing: Listing,
    ) {
        let Some((text, cursor, document, revision)) =
            self.cmdline
                .line()
                .and_then(|prompt| match prompt.context() {
                    Context::Ex(origin) => Some((
                        prompt.text().to_owned(),
                        prompt.cursor(),
                        origin.pane.doc,
                        origin.revision,
                    )),
                    _ => None,
                })
        else {
            return;
        };
        // A path query needs the canonical listing target up front:
        // admission happens once, through the address grammar. The
        // canonical directory URI is also the fallback-cache key.
        let (dir_file, fallback) = match &query {
            RemoteCompletionQuery::Path {
                authority,
                directory,
                ..
            } => match RemoteFile::parse(&format!("ssh://{authority}{directory}")) {
                Ok(file) => {
                    let fallback = self.remote_completion.cached(&file.to_string());
                    (Some(file), fallback)
                }
                Err(error) => {
                    self.message = format!("invalid remote address: {error}");
                    return;
                }
            },
            RemoteCompletionQuery::Hosts { .. } | RemoteCompletionQuery::Directory { .. } => {
                (None, None)
            }
        };
        // A new request replaces any in-flight one; the old worker's
        // late delivery is rejected by ticket mismatch.
        self.revoke_path_completion();
        let key = RemoteCompletionKey {
            focus: self.focus_epoch,
            document,
            revision,
            text,
            cursor,
            query: query.clone(),
            prefix_body,
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = Ticket {
            request,
            key: key.clone(),
        };
        self.remote_completion.pending = Some(ticket.clone());
        self.cmdline.completion.listing = Some(listing);
        self.message = "completing…".into();
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"remote-completion","request":request.get(),
                "query":query.label(),
            })
        });
        match self.tape.request("remote.completion", &ticket) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_remote_completion(Completion {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return;
            }
        }
        let sources = completion_host_sources();
        let history = self.remote_history();
        let client = self.remote_client();
        let local = self.filesystem.local().clone();
        let tx = self.remote_completion.tx.clone();
        let handle = worker::spawn(
            "strop-remote-complete",
            move |outcome| {
                let _ = tx.send(Completion { ticket, outcome });
            },
            move |cancel| {
                let job = CompletionJob {
                    query,
                    directory: dir_file,
                    fallback,
                    sources,
                    history,
                    client,
                    local,
                };
                run_completion(job, cancel)
            },
        );
        self.worker_handles.insert(request, handle);
    }

    /// One worker delivery: only the owning ticket may touch the
    /// model, and only a still-fresh prompt moment may be rewritten.
    pub(crate) fn handle_remote_completion(&mut self, event: RemoteCompletionEvent) {
        if self.remote_completion.pending.as_ref() != Some(&event.ticket) {
            strop_trace::record_with(
                strop_trace::EventKind::JobRejected,
                || serde_json::json!({"service":"remote-completion","reason":"superseded"}),
            );
            return;
        }
        let ticket = event.ticket;
        self.remote_completion.pending = None;
        self.worker_handles.remove(&ticket.request);
        if !self.completion_prompt_fresh(&ticket.key) {
            strop_trace::record_with(
                strop_trace::EventKind::JobRejected,
                || serde_json::json!({"service":"remote-completion","reason":"stale prompt"}),
            );
            return;
        }
        match event.outcome {
            Outcome::Success(RemoteCompletionResult::Candidates {
                items,
                source,
                notes,
                listed_directory,
            }) => {
                if let Some(directory) = &listed_directory {
                    self.remote_completion
                        .store_cache(directory.clone(), items.clone());
                }
                if items.is_empty() {
                    self.cmdline.completion.listing = None;
                    self.message = match notes.first() {
                        Some(note) => format!("no path matches: {note}"),
                        None => "no path matches".into(),
                    };
                    return;
                }
                self.message.clear();
                let completion = &mut self.cmdline.completion;
                let Some(listing) = completion.listing.as_mut() else {
                    return;
                };
                listing.items = items;
                listing.landed = true;
                let mut note = match source {
                    CandidateSource::Cache => Some("cached".to_owned()),
                    CandidateSource::Connection => Some("live".to_owned()),
                    CandidateSource::Config | CandidateSource::Directory => None,
                };
                if let Some(first) = notes.first() {
                    note = Some(match note {
                        Some(source) => format!("{source} · {first}"),
                        None => first.clone(),
                    });
                }
                completion.note = note.map(String::into_boxed_str);
                let start = ticket.key.prefix_body.len();
                let end = ticket.key.cursor - 1;
                let select = std::mem::take(&mut completion.select_on_arrival);
                self.path_rows(start, end);
                if select && self.cmdline.completion.is_open() {
                    super::step(self, super::Step::Next);
                }
            }
            Outcome::Success(RemoteCompletionResult::ConnectRequired { endpoint }) => {
                self.message = format!(
                    "no live connection to {endpoint}; completion never connects \
                     — open the remote first"
                );
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }

    /// The request's moment still describes the editor: same `:` line,
    /// focus, document, revision, text and cursor.
    fn completion_prompt_fresh(&self, key: &RemoteCompletionKey) -> bool {
        let Some(prompt) = self.cmdline.line() else {
            return false;
        };
        matches!(prompt.context(), Context::Ex(_))
            && !self.docs.is_empty()
            && self.current() == key.document
            && self.focus_epoch == key.focus
            && self.buf().revision() == key.revision
            && prompt.text() == key.text
            && prompt.cursor() == key.cursor
    }

    /// Show the landed listing's rows for `[start, end)` of the body.
    fn path_rows(&mut self, start: usize, end: usize) {
        let (line, completion) = self.cmdline.parts_mut();
        let Some(line) = line else { return };
        let body = line.body();
        let Some(listing) = completion.listing.take() else {
            return;
        };
        if listing.rows(start, &body[start..end], &mut completion.rows) {
            completion.show(MenuKind::Paths, listing.title(), body, start, end);
        }
        completion.listing = Some(listing);
    }

    /// Host candidates from endpoints this editor already opened.
    fn remote_history(&self) -> Vec<HostCandidate> {
        self.docs
            .iter()
            .filter_map(|(_, document)| match &document.source {
                DocumentSource::Remote(file) => {
                    let endpoint = file.file.endpoint();
                    Some(HostCandidate::new(
                        endpoint.host().to_owned(),
                        endpoint.user().map(str::to_owned),
                        endpoint.port(),
                        strop_remote::CandidateOrigin::History,
                    ))
                }
                _ => None,
            })
            .collect()
    }
}

/// What the menu shows for a candidate: its last segment (a directory
/// keeps its `/`), or a host without `ssh://`. Escaped form — never a
/// lossy decode of native bytes.
fn label(uri: &str) -> &str {
    let trimmed = uri.strip_suffix('/').unwrap_or(uri);
    let start = match trimmed.strip_prefix("ssh://") {
        Some(host) if !host.contains('/') => "ssh://".len(),
        _ => trimmed.rfind('/').map_or(0, |at| at + 1),
    };
    &uri[start..]
}

/// Split the typed operand into a query, or an honest refusal. `~`
/// entries are unresolved home queries (RemoteLocation's domain): they
/// need a negotiated connection, so completion refuses instead of
/// guessing a home.
fn classify_remote_operand(typed: &str) -> Result<RemoteCompletionQuery, String> {
    let refuse_home =
        || "cannot complete `~` paths: open the remote file so its home resolves first".to_string();
    if typed.starts_with('~') {
        return Err(refuse_home());
    }
    let Some((authority, path)) = typed.split_once('/') else {
        return Ok(RemoteCompletionQuery::Hosts {
            partial: typed.to_owned(),
        });
    };
    if authority.is_empty() {
        return Err("ssh:// needs a host before the path".to_string());
    }
    if path.split('/').next() == Some("~") {
        return Err(refuse_home());
    }
    let (directory, segment) = match path.rsplit_once('/') {
        Some((before, last)) => (format!("/{before}"), last.to_owned()),
        None => ("/".to_owned(), path.to_owned()),
    };
    Ok(RemoteCompletionQuery::Path {
        authority: authority.to_owned(),
        directory,
        segment,
    })
}

/// Standard local host-data locations for this process's home. Only
/// path names are resolved here — reading happens on the worker.
fn completion_host_sources() -> HostSources {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    HostSources::discover(home.as_deref())
}

/// The worker side of one completion request. Host completion reads
/// local data; path completion uses `list_connected` — never a new
/// connection — and falls back to the caller's cached listing when the
/// endpoint is not connected.
/// One completion request's worker-side inputs (bundled: the engine's
/// remote/session state snapshot for exactly one prompt moment).
struct CompletionJob {
    query: RemoteCompletionQuery,
    directory: Option<RemoteFile>,
    fallback: Option<Vec<RemoteCandidate>>,
    sources: HostSources,
    history: Vec<HostCandidate>,
    client: RemoteClient,
    local: crate::editor::namespace::LocalFs,
}

/// The authority region of a canonical URI, for the connect
/// instruction.
fn endpoint_display(file: &RemoteFile) -> String {
    let uri = file.to_string();
    let rest = uri.strip_prefix("ssh://").unwrap_or(&uri);
    let end = rest.find('/').unwrap_or(rest.len());
    format!("ssh://{}", &rest[..end])
}

/// Decode a typed segment for native prefix matching. Malformed or
/// half-typed escapes stay literal: this filters names, it never
/// admits one — the applied candidate is always a canonical URI.
fn lenient_percent_decode(text: &str) -> Vec<u8> {
    fn hex_value(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => byte - b'A' + 10,
        }
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let high = bytes.get(at + 1).copied().filter(|b| b.is_ascii_hexdigit());
            let low = bytes.get(at + 2).copied().filter(|b| b.is_ascii_hexdigit());
            if let (Some(high), Some(low)) = (high, low) {
                out.push((hex_value(high) << 4) | hex_value(low));
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    out
}
