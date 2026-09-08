//! Local SSH host-candidate enumeration (0036 "Completion and homes").
//!
//! Completion is a read of candidate data and nothing else: explicit
//! ssh_config files (with bounded `Include` expansion), explicit
//! known_hosts files, and caller-supplied history endpoints. Nothing
//! here runs `ssh` — `ssh -G` stays a resolver for an already chosen
//! host (0033), `Match exec` commands never run merely because
//! completion asked, and no connection or authentication is attempted.
//! All inputs are explicit paths: this module never reads ambient
//! state, so enumeration is pure, bounded and testable against fixture
//! directories. A candidate endpoint's admissibility is delegated to
//! the one address grammar (`RemoteFile::parse`); no second notion of
//! a valid host exists here. `HostName` values are deliberately not
//! candidates: users type aliases, and resolving an alias is OpenSSH's
//! job at open time — never enumeration's.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};

use crate::address::RemoteFile;

#[cfg(test)]
mod tests;

/// Bounds that keep one Tab cheap against pathological configs.
/// Every bound hit is a note, never a silent truncation.
const MAX_INCLUDE_DEPTH: usize = 16;
const MAX_CONFIG_FILES: usize = 128;
const MAX_FILE_BYTES: u64 = 1 << 20;
const MAX_TOTAL_CONFIG_BYTES: usize = 4 << 20;
const MAX_GLOB_MATCHES: usize = 256;
const MAX_CANDIDATES: usize = 1000;
const MAX_NOTES: usize = 16;

/// Where a candidate came from — shown so users can tell config data
/// from hosts they actually connected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateOrigin {
    /// A literal `Host` alias in a read ssh_config file.
    Config,
    /// A literal host pattern in a read known_hosts file.
    KnownHosts,
    /// An endpoint the caller already opened (editor documents).
    History,
}

/// One completion candidate: the endpoint local data knows about.
/// `token()` renders exactly what can be typed after `ssh://`; the
/// constructor is pure and `admissible()` is judged by the address
/// grammar alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCandidate {
    host: String,
    user: Option<String>,
    port: Option<NonZeroU16>,
    origin: CandidateOrigin,
}

impl HostCandidate {
    pub fn new(
        host: String,
        user: Option<String>,
        port: Option<NonZeroU16>,
        origin: CandidateOrigin,
    ) -> Self {
        Self {
            host,
            user,
            port,
            origin,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    pub fn port(&self) -> Option<NonZeroU16> {
        self.port
    }

    pub fn origin(&self) -> CandidateOrigin {
        self.origin
    }

    /// The endpoint as typed into an `ssh://` URI (`user@host:port`,
    /// IPv6 bracketed). Display spelling of identity data.
    pub fn token(&self) -> String {
        let mut out = self.host_port_token();
        if let Some(user) = &self.user {
            out.insert_str(0, &format!("{user}@"));
        }
        out
    }

    /// The `host[:port]` region without a user — what a typed port
    /// prefix must extend.
    fn host_port_token(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::with_capacity(self.host.len() + 8);
        if self.host.contains(':') {
            out.push('[');
            out.push_str(&self.host);
            out.push(']');
        } else {
            out.push_str(&self.host);
        }
        if let Some(port) = self.port {
            let _ = write!(out, ":{port}");
        }
        out
    }

    /// True when the one address grammar admits this endpoint. This is
    /// the only validity notion in the crate — there is no local
    /// re-implementation of hostname rules.
    pub fn admissible(&self) -> bool {
        RemoteFile::parse(&format!("ssh://{}/", self.token())).is_ok()
    }

    /// The completed endpoint token for a typed prefix, or None when
    /// this candidate does not extend what the user typed. Typing a
    /// user wins over a configured one; typing past `:` must extend
    /// the candidate's port, never reset it.
    pub fn matches_typed(&self, typed: &str) -> Option<String> {
        if let Some((typed_user, typed_rest)) = typed.split_once('@') {
            if !self.host.starts_with(host_prefix_of(typed_rest)) {
                return None;
            }
            let mut token = format!("{typed_user}@");
            token.push_str(&self.host_port_token());
            return Some(token);
        }
        if !self.host.starts_with(host_prefix_of(typed)) {
            return None;
        }
        if typed.contains(':') {
            let bare = self.host_port_token();
            return bare.starts_with(typed).then(|| self.token());
        }
        Some(self.token())
    }
}

/// The host region of a typed authority: text before an explicit port.
fn host_prefix_of(typed: &str) -> &str {
    match typed.rsplit_once(':') {
        Some((host, digits))
            if !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && !host.contains(':')
                && !host.is_empty() =>
        {
            host
        }
        _ => typed,
    }
}

/// Explicit enumeration inputs. The caller owns path resolution, so
/// enumeration itself is isolated from ambient state.
#[derive(Debug, Clone, Default)]
pub struct HostSources {
    config: Vec<PathBuf>,
    known_hosts: Vec<PathBuf>,
    include_base: Option<PathBuf>,
}

impl HostSources {
    pub fn push_config(&mut self, path: PathBuf) -> &mut Self {
        self.config.push(path);
        self
    }

    pub fn push_known_hosts(&mut self, path: PathBuf) -> &mut Self {
        self.known_hosts.push(path);
        self
    }

    /// Relative `Include` arguments resolve against this directory
    /// (ssh_config(5) resolves them against `~/.ssh`).
    pub fn set_include_base(&mut self, base: PathBuf) -> &mut Self {
        self.include_base = Some(base);
        self
    }

    /// The standard locations for one home directory. Pure: nothing is
    /// read here; unreadable or missing files become notes (or
    /// silence) at enumeration time.
    pub fn discover(home: Option<&Path>) -> Self {
        let mut sources = Self::default();
        if let Some(home) = home {
            let ssh = home.join(".ssh");
            sources.config.push(ssh.join("config"));
            sources.known_hosts.push(ssh.join("known_hosts"));
            sources.include_base = Some(ssh);
        }
        sources.config.push(PathBuf::from("/etc/ssh/ssh_config"));
        sources
            .known_hosts
            .push(PathBuf::from("/etc/ssh/ssh_known_hosts"));
        sources
    }
}

/// Everything enumeration found: candidates plus bounded notes. Notes
/// are diagnostics, not failures — an unreadable config still completes
/// from the remaining sources.
#[derive(Debug, Clone, Default)]
pub struct HostEnumeration {
    candidates: Vec<HostCandidate>,
    notes: Vec<String>,
}

impl HostEnumeration {
    pub fn candidates(&self) -> &[HostCandidate] {
        &self.candidates
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The sorted, deduplicated endpoint tokens extending `typed`
    /// (`""` lists every candidate).
    pub fn complete(&self, typed: &str) -> Vec<String> {
        let mut tokens: Vec<String> = self
            .candidates
            .iter()
            .filter_map(|candidate| candidate.matches_typed(typed))
            .collect();
        tokens.sort();
        tokens.dedup();
        tokens
    }
}

/// Shared state of one enumeration walk.
struct Walk<'a> {
    out: HostEnumeration,
    sources: &'a HostSources,
    seen_files: HashSet<PathBuf>,
    seen_tokens: HashSet<String>,
    files_read: usize,
    bytes_read: usize,
    skipped_names: usize,
    capped: bool,
    depth_noted: bool,
    budget_noted: bool,
}

impl Walk<'_> {
    fn note(&mut self, message: String) {
        if self.out.notes.len() < MAX_NOTES {
            self.out.notes.push(message);
        }
    }
}

/// Enumerate host candidates from explicit local data. Pure and
/// bounded: reads only the given files, spawns nothing, authenticates
/// nothing. History candidates are admitted first (the endpoints the
/// user actually opens), then config files in order, then known_hosts.
pub fn enumerate_hosts(sources: &HostSources, history: &[HostCandidate]) -> HostEnumeration {
    let mut walk = Walk {
        out: HostEnumeration::default(),
        sources,
        seen_files: HashSet::new(),
        seen_tokens: HashSet::new(),
        files_read: 0,
        bytes_read: 0,
        skipped_names: 0,
        capped: false,
        depth_noted: false,
        budget_noted: false,
    };
    for candidate in history {
        admit(&mut walk, candidate.clone());
    }
    for path in &sources.config {
        parse_config(&mut walk, path, 0);
    }
    for path in &sources.known_hosts {
        parse_known_hosts(&mut walk, path);
    }
    if walk.skipped_names > 0 {
        walk.note(format!(
            "{} host names are not valid endpoints and were skipped",
            walk.skipped_names
        ));
    }
    walk.out
}

/// Admit one candidate: deduplicated by token, validated by the
/// address grammar, capped in total.
fn admit(walk: &mut Walk, candidate: HostCandidate) -> Option<usize> {
    if !candidate.admissible() {
        walk.skipped_names += 1;
        return None;
    }
    let token = candidate.token();
    if !walk.seen_tokens.insert(token) {
        return None;
    }
    if walk.out.candidates.len() >= MAX_CANDIDATES {
        if !walk.capped {
            walk.capped = true;
            walk.note(format!("candidate list capped at {MAX_CANDIDATES}"));
        }
        return None;
    }
    walk.out.candidates.push(candidate);
    Some(walk.out.candidates.len() - 1)
}
/// Read one file, bounded; a missing file is silence, anything else is
/// a note. Returns None when the file contributes nothing.
fn read_bounded(walk: &mut Walk, path: &Path) -> Option<String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            walk.note(format!("{}: {error}", path.display()));
            return None;
        }
    };
    let mut bytes = Vec::new();
    if let Err(error) = file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes) {
        walk.note(format!("{}: {error}", path.display()));
        return None;
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        walk.note(format!(
            "{} larger than {} bytes; truncated",
            path.display(),
            MAX_FILE_BYTES
        ));
        bytes.truncate(MAX_FILE_BYTES as usize);
    }
    match String::from_utf8(bytes) {
        Ok(text) => {
            walk.bytes_read = walk.bytes_read.saturating_add(text.len());
            Some(text)
        }
        Err(_) => {
            walk.note(format!("{}: not UTF-8; skipped", path.display()));
            None
        }
    }
}

/// Parse one ssh_config file, following `Include` directives inline
/// (depth-first like OpenSSH) with cycle and budget guards.
fn parse_config(walk: &mut Walk, path: &Path, depth: usize) {
    if depth >= MAX_INCLUDE_DEPTH {
        if !walk.depth_noted {
            walk.depth_noted = true;
            walk.note(format!("Include deeper than {MAX_INCLUDE_DEPTH} skipped"));
        }
        return;
    }
    if walk.files_read >= MAX_CONFIG_FILES {
        walk.note("config file limit reached; further includes skipped".into());
        return;
    }
    if walk.bytes_read >= MAX_TOTAL_CONFIG_BYTES {
        if !walk.budget_noted {
            walk.budget_noted = true;
            walk.note(format!(
                "config budget of {MAX_TOTAL_CONFIG_BYTES} bytes reached; further files skipped"
            ));
        }
        return;
    }
    let identity = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !walk.seen_files.insert(identity) {
        if depth > 0 {
            walk.note(format!("Include cycle skipped: {}", path.display()));
        }
        return;
    }
    let Some(text) = read_bounded(walk, path) else {
        return;
    };
    walk.files_read += 1;
    parse_config_text(walk, &text, depth);
}

/// Line-oriented ssh_config reading. Only `Host` contributes names;
/// `User`/`Port` qualify the open `Host` block (or the file's global
/// defaults); `Match` opens a conditional block whose commands are
/// never evaluated; `Include` is expanded inline. An `Include` resets
/// the includer's block context — an approximation of OpenSSH's
/// fully-ordered merge, documented here: resolving the final
/// configuration remains `ssh -G`'s job at open time, never
/// enumeration's.
fn parse_config_text(walk: &mut Walk, text: &str, depth: usize) {
    let mut global_user: Option<String> = None;
    let mut global_port: Option<NonZeroU16> = None;
    let mut open_block: Vec<usize> = Vec::new();
    let mut file_candidates: Vec<usize> = Vec::new();

    for line in text.lines() {
        let tokens = tokenize(line);
        let Some(keyword) = tokens.first().map(|token| token.to_ascii_lowercase()) else {
            continue;
        };
        match keyword.as_str() {
            "host" => {
                open_block.clear();
                for pattern in &tokens[1..] {
                    // Patterns and negations are match rules, not names
                    // a user can type; literal aliases only.
                    if pattern.contains(['*', '?', '!']) {
                        continue;
                    }
                    let candidate =
                        HostCandidate::new(pattern.clone(), None, None, CandidateOrigin::Config);
                    if let Some(index) = admit(walk, candidate) {
                        open_block.push(index);
                        file_candidates.push(index);
                    }
                }
            }
            "match" => open_block.clear(),
            "user" => {
                let Some(value) = tokens.get(1) else {
                    continue;
                };
                if open_block.is_empty() {
                    global_user.get_or_insert_with(|| value.clone());
                } else {
                    for &index in &open_block {
                        let candidate = &mut walk.out.candidates[index];
                        candidate.user.get_or_insert_with(|| value.clone());
                    }
                }
            }
            "port" => {
                let Some(value) = tokens.get(1) else {
                    continue;
                };
                let Some(port) = parse_port(value) else {
                    walk.note(format!("port {value}: not a port; ignored"));
                    continue;
                };
                if open_block.is_empty() {
                    global_port.get_or_insert(port);
                } else {
                    for &index in &open_block {
                        let candidate = &mut walk.out.candidates[index];
                        candidate.port.get_or_insert(port);
                    }
                }
            }
            "include" => {
                for argument in &tokens[1..] {
                    for path in expand_include(walk, argument) {
                        parse_config(walk, &path, depth + 1);
                    }
                }
                open_block.clear();
            }
            _ => {}
        }
    }

    // File-level defaults qualify this file's candidates that lack a
    // block-level value.
    for index in file_candidates {
        let candidate = &mut walk.out.candidates[index];
        if candidate.user.is_none() {
            candidate.user.clone_from(&global_user);
        }
        if candidate.port.is_none() {
            candidate.port = global_port;
        }
    }
}

fn parse_port(value: &str) -> Option<NonZeroU16> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u16>().ok().and_then(NonZeroU16::new)
}

/// Resolve one `Include` argument to concrete files: absolute paths
/// pass through; relative paths resolve against the include base
/// (ssh_config(5) uses `~/.ssh`); glob wildcards (`*` and `?`, one
/// directory — OpenSSH's cross-component globs are not completion
/// data) expand sorted and bounded. Without a base, relative includes
/// are skipped with a note.
fn expand_include(walk: &mut Walk, argument: &str) -> Vec<PathBuf> {
    let literal = Path::new(argument);
    let resolved = if literal.is_absolute() {
        literal.to_path_buf()
    } else {
        match &walk.sources.include_base {
            Some(base) => base.join(literal),
            None => {
                walk.note(format!(
                    "Include {argument}: relative without an include base; skipped"
                ));
                return Vec::new();
            }
        }
    };
    if !argument.contains(['*', '?']) {
        return vec![resolved];
    }
    glob_expand(walk, &resolved)
}

/// Expand a glob with `*` and `?` over one directory, sorted, bounded.
fn glob_expand(walk: &mut Walk, pattern: &Path) -> Vec<PathBuf> {
    let text = pattern.to_string_lossy().into_owned();
    let Some(wildcard) = text.find(['*', '?']) else {
        return vec![pattern.to_path_buf()];
    };
    let split = text[..wildcard].rfind('/').map_or(0, |at| at + 1);
    let directory = match std::fs::canonicalize(Path::new(&text[..split])) {
        Ok(absolute) => absolute,
        Err(_) => return Vec::new(),
    };
    let pattern = directory
        .join(&text[split..])
        .to_string_lossy()
        .into_owned();
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if glob_match(&pattern, &path.to_string_lossy()) {
            matches.push(path);
            if matches.len() > MAX_GLOB_MATCHES {
                walk.note(format!(
                    "Include glob matched more than {MAX_GLOB_MATCHES} files; truncated"
                ));
                break;
            }
        }
    }
    matches.sort();
    matches
}

/// Anchored glob match with `*` and `?` (ssh wildcards minus character
/// classes, which no completion source relies on).
fn glob_match(pattern: &str, text: &str) -> bool {
    fn inner(pattern: &[u8], text: &[u8]) -> bool {
        match (pattern.first(), text.first()) {
            (None, None) => true,
            (Some(b'*'), _) => {
                inner(&pattern[1..], text) || (!text.is_empty() && inner(pattern, &text[1..]))
            }
            (Some(b'?'), Some(_)) => inner(&pattern[1..], &text[1..]),
            (Some(expected), Some(actual)) if expected == actual => {
                inner(&pattern[1..], &text[1..])
            }
            _ => false,
        }
    }
    inner(pattern.as_bytes(), text.as_bytes())
}

/// ssh_config tokenization: whitespace-separated, single- or
/// double-quoted tokens, `#` starts a comment only before the first
/// token of a line.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for character in line.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None => match character {
                '#' if current.is_empty() && tokens.is_empty() => break,
                '"' | '\'' if current.is_empty() => quote = Some(character),
                character if character.is_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                }
                character => current.push(character),
            },
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parse one known_hosts file. Literal hosts become candidates
/// (`[host]:port` carries its port); hashed entries cannot be reversed
/// and are counted in a note; markers, negations and wildcard patterns
/// are not connect targets.
fn parse_known_hosts(walk: &mut Walk, path: &Path) {
    let Some(text) = read_bounded(walk, path) else {
        return;
    };
    let mut hashed = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        let Some(patterns) = line.split_whitespace().next() else {
            continue;
        };
        for pattern in patterns.split(',') {
            if pattern.is_empty() {
                continue;
            }
            if pattern.starts_with('|') {
                hashed += 1;
                continue;
            }
            if pattern.starts_with('!') || pattern.contains(['*', '?']) {
                continue;
            }
            let Some((host, port)) = split_known_host(pattern) else {
                continue;
            };
            let candidate = HostCandidate::new(host, None, port, CandidateOrigin::KnownHosts);
            admit(walk, candidate);
        }
    }
    if hashed > 0 {
        walk.note(format!(
            "{hashed} hashed known_hosts entries cannot be completed"
        ));
    }
}

/// Split one known_hosts pattern: `[host]:port`, `[host]` or `host`.
fn split_known_host(pattern: &str) -> Option<(String, Option<NonZeroU16>)> {
    if let Some(bracketed) = pattern.strip_prefix('[') {
        let (host, tail) = bracketed.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(digits) => Some(parse_port(digits)?),
            None if tail.is_empty() => None,
            None => return None,
        };
        return Some((host.to_owned(), port));
    }
    Some((pattern.to_owned(), None))
}
