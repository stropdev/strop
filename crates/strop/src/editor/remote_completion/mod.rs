//! RW2 remote host/path completion (0036 "Completion and homes").
//!
//! Ex Tab on a file-bearing command's `ssh://` operand completes from
//! two honest sources only: local host data (`strop_remote::hosts`,
//! read on a worker — never `ssh -G`, never `Match exec`) and an
//! already-live connection's directory listing via
//! `RemoteClient::list_connected`, which must never authenticate or
//! open a connection. Without a live connection the user gets cached
//! candidates or an explicit connect instruction — completion never
//! surprises anyone with an authentication prompt.
//!
//! Ownership is the second half: a request captures the exact prompt
//! moment (focus, document, revision, full input text and cursor) and
//! every delivery re-checks it, so a result that lands after an edit,
//! Esc, focus change or close can neither rewrite the input line nor
//! open a picker. Cycling reuses the one prompt grammar
//! (`PendingEvent::CompleteEx`); there is no second prompt surface.
//! Applied candidates are canonical `RemoteFile` URIs, so native
//! filename bytes (spaces, non-UTF-8) survive as percent escapes.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_remote::{HostCandidate, HostSources, RemoteClient, RemoteEntryKind, RemoteFile};

use super::document::DocumentSource;
use super::pending::{PendingEvent, PromptContext};
use super::Editor;

#[cfg(test)]
mod tests;

/// Bounded fallback cache: successful live listings remembered so a
/// later offline Tab still completes from the last observed truth.
const CACHE_DIRS: usize = 32;

/// Commands whose final argument is a remote URI, and how many numeric
/// arguments may precede it (`(min, max)`). `w`/`wq` refuse remote
/// targets outright (execution refuses them too), so they are absent
/// here and answered with an honest message instead.
fn remote_operand_shape(command: &str) -> Option<(usize, usize)> {
    match command {
        "e" | "e!" | "view" | "sp" | "split" | "vs" | "vsplit" | "browse" | "follow" => {
            Some((0, 0))
        }
        // `:tail [BYTES] URI` — the byte count is optional.
        "tail" => Some((0, 1)),
        // `:range START BYTES URI` — both numbers required.
        "range" => Some((2, 2)),
        _ => None,
    }
}

/// Which local question a completion asked. Pure serde data — the
/// replayable half of the exchange; worker inputs (sources, history,
/// the client) are live values and never serialize.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum RemoteCompletionQuery {
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
}

impl RemoteCompletionQuery {
    fn label(&self) -> &'static str {
        match self {
            Self::Hosts { .. } => "hosts",
            Self::Path { .. } => "path",
        }
    }
}

/// One completion answer item: the canonical URI text that replaces
/// the typed `ssh://…` token. Directories carry a trailing `/` so the
/// next Tab descends into them; files are complete URIs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RemoteCandidate {
    pub uri: String,
    pub directory: bool,
}

/// Where candidates came from — surfaced so cache is never mistaken
/// for live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum CandidateSource {
    Config,
    Connection,
    Cache,
}

/// The typed moment a request owns; every delivery re-checks it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RemoteCompletionKey {
    pub focus: u64,
    pub document: DocumentId,
    pub revision: BufferRevision,
    /// Full prompt text (sigil included) at request time.
    pub text: String,
    pub cursor: usize,
    pub query: RemoteCompletionQuery,
}

/// The worker's terminal answer. Failures travel as
/// `Outcome::Failed`; this carries only successes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) enum RemoteCompletionResult {
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
pub(crate) type RemoteCompletionEvent = Completion<RemoteCompletionKey, RemoteCompletionResult>;

/// Landed candidates plus the moment they were applied to — the cycle
/// state between Tabs.
#[derive(Debug, Clone)]
struct ReadyCompletion {
    /// Prompt body before the `ssh://` token (e.g. `e ` or `tail 64k `).
    prefix_body: String,
    /// Full prompt text after our last apply; a mismatch means the
    /// user typed since, and Tab starts a fresh request instead.
    applied: String,
    candidates: Vec<RemoteCandidate>,
    index: usize,
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
    ready: Option<ReadyCompletion>,
    cache: VecDeque<(String, Vec<RemoteCandidate>)>,
}

impl Default for RemoteCompletionState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Some(rx),
            pending: None,
            ready: None,
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
    /// Tab on the ex line's remote operand: cycle landed candidates or
    /// start a request. Returns true when the line was a remote
    /// completion (even when the answer is a refusal message), so the
    /// caller's command-name cycling never touches a remote line.
    pub(crate) fn remote_completion_tab(&mut self) -> bool {
        let Some((text, cursor)) = self
            .pending
            .prompt()
            .map(|prompt| (prompt.text().to_owned(), prompt.cursor()))
        else {
            return false;
        };
        let Some(body) = text.strip_prefix(':') else {
            return false;
        };
        let Some((cmd, rest)) = body.split_once(' ') else {
            return false;
        };
        let tokens: Vec<&str> = rest.split(' ').filter(|token| !token.is_empty()).collect();
        let Some(operand) = tokens.last().copied() else {
            return false;
        };
        if !operand.starts_with("ssh://") {
            // A URI stranded before the final token means a raw space
            // was typed into it; complete only well-formed lines.
            if tokens.iter().any(|token| token.starts_with("ssh://")) {
                self.message = "remote URI cannot contain a raw space (type %20)".into();
                return true;
            }
            return false;
        }
        if matches!(cmd, "w" | "w!" | "wq" | "wq!") {
            self.message = "remote save-as completion is unsupported".into();
            return true;
        }
        let Some((min_args, max_args)) = remote_operand_shape(cmd) else {
            return false;
        };
        let leading = tokens.len() - 1;
        if leading < min_args || leading > max_args {
            self.message = match cmd {
                "range" => ":range needs START BYTES before the URI".into(),
                "tail" => ":tail takes at most one byte count before the URI".into(),
                _ => format!(":{cmd} takes no argument before the URI"),
            };
            return true;
        }
        if cursor != text.len() {
            self.message = "completion needs the cursor at the end of the line".into();
            return true;
        }
        if let Some(ready) = self.remote_completion.ready.as_ref() {
            if self.pending.text() == ready.applied && ready.candidates.len() > 1 {
                let next = (ready.index + 1) % ready.candidates.len();
                let uri = ready.candidates[next].uri.clone();
                let prefix = ready.prefix_body.clone();
                self.apply_completion(&prefix, &uri);
                if let Some(ready) = self.remote_completion.ready.as_mut() {
                    ready.index = next;
                    ready.applied = self.pending.text().to_owned();
                }
                return true;
            }
            // A sole candidate descends into directories or re-queries.
        }
        let typed = operand.strip_prefix("ssh://").unwrap_or_default();
        self.start_remote_completion(typed);
        true
    }

    /// Classify the typed operand and launch the owned worker request.
    fn start_remote_completion(&mut self, typed: &str) {
        let query = match classify_remote_operand(typed) {
            Ok(query) => query,
            Err(message) => {
                self.message = message;
                return;
            }
        };
        let Some((text, cursor, document, revision)) =
            self.pending
                .prompt()
                .and_then(|prompt| match prompt.context() {
                    PromptContext::Ex(origin) => Some((
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
            RemoteCompletionQuery::Hosts { .. } => (None, None),
        };
        // A new request replaces any in-flight one; the old worker's
        // late delivery is rejected by ticket mismatch.
        if let Some(old) = self.remote_completion.pending.take() {
            if let Some(handle) = self.worker_handles.remove(&old.request) {
                handle.cancel(CancelReason::Superseded);
            }
        }
        self.remote_completion.ready = None;
        let key = RemoteCompletionKey {
            focus: self.focus_epoch,
            document,
            revision,
            text,
            cursor,
            query: query.clone(),
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
        let tx = self.remote_completion.tx.clone();
        let handle = worker::spawn(
            "strop-remote-complete",
            move |outcome| {
                let _ = tx.send(Completion { ticket, outcome });
            },
            move |cancel| {
                run_completion(query, dir_file, fallback, sources, history, client, cancel)
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
                    self.message = match notes.first() {
                        Some(note) => format!("no remote matches: {note}"),
                        None => "no remote matches".into(),
                    };
                    return;
                }
                let prefix_body = completion_prefix_body(&ticket.key);
                self.apply_completion(&prefix_body, &items[0].uri);
                self.remote_completion.ready = Some(ReadyCompletion {
                    prefix_body,
                    applied: self.pending.text().to_owned(),
                    candidates: items.clone(),
                    index: 0,
                });
                self.message = candidates_message(&items, source);
            }
            Outcome::Success(RemoteCompletionResult::ConnectRequired { endpoint }) => {
                self.message = format!(
                    "no live connection to {endpoint}; completion never connects \
                     — open or browse the remote first"
                );
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }

    /// The request's prompt moment still describes the editor: same
    /// ex prompt, focus, document, revision, input text and cursor.
    fn completion_prompt_fresh(&self, key: &RemoteCompletionKey) -> bool {
        let Some(prompt) = self.pending.prompt() else {
            return false;
        };
        matches!(prompt.context(), PromptContext::Ex(_))
            && !self.docs.is_empty()
            && self.current() == key.document
            && self.focus_epoch == key.focus
            && self.buf().revision() == key.revision
            && prompt.text() == key.text
            && prompt.cursor() == key.cursor
    }

    /// Apply one candidate through the one prompt grammar.
    fn apply_completion(&mut self, prefix_body: &str, uri: &str) {
        self.feed_pending_event(PendingEvent::CompleteEx(format!("{prefix_body}{uri}")));
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

/// The prompt body before the remote operand (`e `, `tail 64k `),
/// rebuilt from the request's captured text. The operand is the last
/// token, so the last `ssh://` is its start.
fn completion_prefix_body(key: &RemoteCompletionKey) -> String {
    let body = key.text.strip_prefix(':').unwrap_or(&key.text);
    match body.rfind("ssh://") {
        Some(at) => body[..at].to_owned(),
        None => body.to_owned(),
    }
}

/// Sort, show and label the candidate list for the message line.
fn candidates_message(items: &[RemoteCandidate], source: CandidateSource) -> String {
    let mut text = items
        .iter()
        .take(6)
        .map(display_segment)
        .collect::<Vec<_>>()
        .join("  ");
    if items.len() > 6 {
        text.push_str(&format!("  (+{})", items.len() - 6));
    }
    match source {
        CandidateSource::Cache => text.push_str("  (cached)"),
        CandidateSource::Connection => text.push_str("  (live)"),
        CandidateSource::Config => {}
    }
    if text.len() > 160 {
        text.truncate(160);
    }
    text
}

/// Safe display of one candidate: the final URI segment (directories
/// keep their trailing `/`). Escaped form — never a lossy decode of
/// native bytes.
fn display_segment(candidate: &RemoteCandidate) -> &str {
    let uri = &candidate.uri;
    let cut = match uri.rfind('/') {
        Some(at) if at + 1 == uri.len() => uri[..at].rfind('/').map_or(at, |prev| prev + 1),
        Some(at) => at + 1,
        None => return uri.strip_prefix("ssh://").unwrap_or(uri),
    };
    &uri[cut..]
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
fn run_completion(
    query: RemoteCompletionQuery,
    directory: Option<RemoteFile>,
    fallback: Option<Vec<RemoteCandidate>>,
    sources: HostSources,
    history: Vec<HostCandidate>,
    client: RemoteClient,
    cancel: worker::CancelToken,
) -> Outcome<RemoteCompletionResult> {
    if cancel.is_cancelled() {
        return Outcome::Cancelled(CancelReason::OwnerClosed);
    }
    match query {
        RemoteCompletionQuery::Hosts { partial } => {
            let enumeration = strop_remote::enumerate_hosts(&sources, &history);
            let items = enumeration
                .complete(&partial)
                .into_iter()
                .map(|token| RemoteCandidate {
                    uri: format!("ssh://{token}"),
                    directory: false,
                })
                .collect();
            Outcome::Success(RemoteCompletionResult::Candidates {
                items,
                source: CandidateSource::Config,
                notes: enumeration.notes().to_vec(),
                listed_directory: None,
            })
        }
        RemoteCompletionQuery::Path { segment, .. } => {
            let Some(dir) = directory else {
                return Outcome::failed(
                    FailureKind::Protocol,
                    "path completion without a listing target",
                );
            };
            let prefix = lenient_percent_decode(&segment);
            match client.list_connected(&dir, &cancel) {
                Ok(entries) => {
                    let mut items: Vec<RemoteCandidate> = entries
                        .into_iter()
                        .filter_map(|entry| {
                            // `.`/`..` have no file_name; browsing owns
                            // parent navigation, completion owns names.
                            let name = entry.file.path().file_name()?;
                            if !name.as_encoded_bytes().starts_with(&prefix) {
                                return None;
                            }
                            let directory = matches!(entry.kind, RemoteEntryKind::Directory);
                            let mut uri = entry.file.to_string();
                            if directory && !uri.ends_with('/') {
                                uri.push('/');
                            }
                            Some(RemoteCandidate { uri, directory })
                        })
                        .collect();
                    items.sort_by(|a, b| {
                        b.directory
                            .cmp(&a.directory)
                            .then_with(|| a.uri.cmp(&b.uri))
                    });
                    let listed = dir.to_string();
                    Outcome::Success(RemoteCompletionResult::Candidates {
                        items,
                        source: CandidateSource::Connection,
                        notes: Vec::new(),
                        listed_directory: Some(listed),
                    })
                }
                Err(_not_connected) => {
                    if cancel.is_cancelled() {
                        return Outcome::Cancelled(CancelReason::OwnerClosed);
                    }
                    if let Some(cached) = fallback {
                        return Outcome::Success(RemoteCompletionResult::Candidates {
                            items: cached,
                            source: CandidateSource::Cache,
                            notes: Vec::new(),
                            listed_directory: None,
                        });
                    }
                    Outcome::Success(RemoteCompletionResult::ConnectRequired {
                        endpoint: endpoint_display(&dir),
                    })
                }
            }
        }
    }
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
