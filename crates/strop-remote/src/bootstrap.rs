//! The audited POSIX discovery bootstrap and the one shell-quoting
//! boundary (0058 WK07; the normative text is plan 0058 §5 "SSH: small
//! bootstrap, existing security boundary").
//!
//! Deployment needs three endpoint facts no SFTP exchange can report:
//! the principal (for ownership validation), the worker-artifact target
//! (for the catalog binding) and the principal's per-user cache base.
//! They come from ONE fixed, constant POSIX command line over the
//! existing authenticated ssh exec channel — no Python, no PATH/rc/image
//! mutation, no generated shell source. Project paths, argv and file
//! contents never enter shell text: the only other shell line this
//! transport ever emits is the worker entrypoint, whose single dynamic
//! word crosses [`quote`], the one tested quoting boundary.
//!
//! A host that refuses the exec channel (restricted or SFTP-only
//! account) fails discovery with a typed refusal; the read-only SFTP
//! path is untouched by anything here.

use std::io::Read;
use std::time::{Duration, Instant};

use strop_core::worker::CancelToken;
use strop_workspace::RemoteEndpoint;

/// The fixed discovery line. Constant: there is nothing to inject.
/// Prints, one per line: numeric uid, `uname -s`, `uname -m` and the
/// per-user cache base (`$XDG_CACHE_HOME` or `$HOME/.cache`).
///
/// Audit notes: `printf`/`id`/`uname` are POSIX; the expansion is the
/// POSIX `${parameter:-word}` form; no glob, no redirect, no write, no
/// PATH/rc mutation. An unset `HOME` yields `/.cache`, which the cache
/// validator refuses honestly downstream.
const DISCOVERY_LINE: &str =
    "printf '%s\\n' \"$(id -u)\" \"$(uname -s)\" \"$(uname -m)\" \"${XDG_CACHE_HOME:-$HOME/.cache}\"";

/// Discovery must resolve inside this window; a half-alive ssh never
/// parks a deployment job unboundedly.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard bound on discovery stdout — four short lines, never more.
const MAX_DISCOVERY_BYTES: usize = 4096;
/// Bounded ssh stderr retained for diagnostics.
const MAX_STDERR_BYTES: usize = 8 * 1024;

/// The endpoint facts deployment binds to, discovered over the
/// authenticated exec channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointFacts {
    /// Remote principal as the numeric uid string — exactly the domain
    /// SFTP ATTRS reports ownership in, so cache ownership validation
    /// compares like with like.
    pub principal: String,
    /// `uname -s` / `uname -m` verbatim (platform identity evidence).
    pub system: String,
    pub machine: String,
    /// The worker-artifact target of the remote host (a release-catalog
    /// triple), never a guessed one.
    pub target: String,
    /// The principal's per-user cache base in native spelling.
    pub cache_base: String,
}

impl EndpointFacts {
    /// The target a same-binary deploy binds: this build's exact
    /// compile-time triple, when the remote runs the same arch/OS. A
    /// release build's triple IS the artifact mapping; a development
    /// build's (e.g. `-gnu` where the release artifact is `-musl`) is
    /// still the honest identity of the only local bytes we may
    /// upload. Different arch/OS is never a same-binary deploy.
    pub fn local_binary_target(&self) -> Option<&'static str> {
        (self.system == local_system() && self.machine == local_machine())
            .then_some(strop_worker_protocol::TARGET_TRIPLE)
    }
}

/// Discovery failures, classified far enough to act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BootstrapError {
    /// The ssh exec channel failed: authentication, trust, a restricted
    /// or SFTP-only account, a deadline or a transport error. The
    /// bounded ssh diagnostic travels in the message.
    #[error("exec channel unavailable: {0}")]
    Transport(String),
    /// The remote host has no worker artifact target.
    #[error("no worker artifact target for {system}/{machine}")]
    UnsupportedTarget { system: String, machine: String },
    /// The discovery reply did not decode into the four expected facts —
    /// a restricted shell or alias output can never be mistaken for
    /// discovery data.
    #[error("discovery reply malformed: {0}")]
    Malformed(String),
}

/// The one POSIX single-quote boundary for SSH exec shell text. `'`
/// becomes `'\''`; every other byte is literal inside the quotes.
/// Native path spelling is preserved exactly — no encoding, no glob
/// survives quoting.
pub fn quote(word: &str) -> String {
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('\'');
    for byte in word.chars() {
        if byte == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(byte);
        }
    }
    quoted.push('\'');
    quoted
}

/// Map a discovered `uname -s`/`uname -m` pair to the release-catalog
/// worker-artifact target, or `None` when no artifact exists for the
/// host (an honest fallback, never a guessed triple). Linux artifacts
/// are the static musl builds, which run under any libc (0058 WK05).
pub fn target_for(system: &str, machine: &str) -> Option<&'static str> {
    match (system, machine) {
        ("Linux", "x86_64" | "amd64") => Some("x86_64-unknown-linux-musl"),
        ("Linux", "aarch64" | "arm64") => Some("aarch64-unknown-linux-musl"),
        ("Darwin", "x86_64" | "amd64") => Some("x86_64-apple-darwin"),
        ("Darwin", "arm64" | "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

/// This client's own worker-artifact target: an endpoint whose
/// discovered target matches can run this install's own binary
/// (0058 §5 — same-target endpoints use the verified local artifact).
pub fn local_target() -> Option<&'static str> {
    target_for(local_system(), local_machine())
}

const fn local_system() -> &'static str {
    #[cfg(target_os = "linux")]
    return "Linux";
    #[cfg(target_os = "macos")]
    return "Darwin";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return "other";
}

const fn local_machine() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    return "x86_64";
    #[cfg(target_arch = "aarch64")]
    return "aarch64";
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return "other";
}

/// Run the fixed discovery line over one authenticated ssh exec channel
/// and decode the four endpoint facts. Cancellation and the deadline
/// both terminate the child; nothing here ever writes to the host.
pub fn discover(
    endpoint: &RemoteEndpoint,
    token: &CancelToken,
) -> Result<EndpointFacts, BootstrapError> {
    let mut command = crate::ssh::exec_command(endpoint, DISCOVERY_LINE);
    let mut child = strop_core::process::OwnedProcess::spawn(&mut command, token)
        .map_err(|error| BootstrapError::Transport(format!("ssh spawn: {}", error.message)))?;
    let mut stdout = child
        .take_stdout()
        .ok_or_else(|| BootstrapError::Transport("ssh stdout pipe unavailable".into()))?;
    let stderr = child.take_stderr().map(|stderr| {
        let (sender, receiver) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut captured = Vec::new();
            let mut source = std::io::BufReader::new(stderr);
            let mut chunk = [0_u8; 2048];
            loop {
                match source.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if captured.len() < MAX_STDERR_BYTES {
                            let room = MAX_STDERR_BYTES - captured.len();
                            captured.extend_from_slice(&chunk[..count.min(room)]);
                        }
                    }
                }
            }
            let _ = sender.send(captured);
        });
        receiver
    });
    let mut output = Vec::new();
    let read = (&mut stdout)
        .take(MAX_DISCOVERY_BYTES as u64 + 1)
        .read_to_end(&mut output);
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let status = loop {
        match child.has_exited() {
            Ok(true) => break child.wait().ok(),
            Ok(false) => {}
            Err(_) => break None,
        }
        if Instant::now() >= deadline {
            let _ = child.terminate();
            let _ = child.wait();
            return Err(BootstrapError::Transport(format!(
                "discovery exceeded the {}s deadline",
                DISCOVERY_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if let Err(error) = read {
        let _ = child.terminate();
        let _ = child.wait();
        return Err(BootstrapError::Transport(format!(
            "discovery read: {error}"
        )));
    }
    if output.len() > MAX_DISCOVERY_BYTES {
        return Err(BootstrapError::Malformed(
            "discovery reply exceeds its bound".into(),
        ));
    }
    let succeeded = status.map(|status| status.success()).unwrap_or(false);
    if !succeeded {
        // Every exit path reaps and releases the token's cancel slot.
        let _ = child.wait();
        let detail = stderr
            .and_then(|receiver| receiver.recv_timeout(Duration::from_secs(1)).ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "ssh exited without a discovery reply".into());
        return Err(BootstrapError::Transport(detail));
    }
    parse_reply(&output)
}

/// Decode the four discovery lines. Anything but exactly uid/sysname/
/// machine/cache-base is malformed — never a guessed fact.
fn parse_reply(output: &[u8]) -> Result<EndpointFacts, BootstrapError> {
    let text = std::str::from_utf8(output)
        .map_err(|_| BootstrapError::Malformed("discovery reply is not UTF-8".into()))?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != 4 {
        return Err(BootstrapError::Malformed(format!(
            "expected 4 discovery lines, got {}",
            lines.len()
        )));
    }
    let principal = lines[0].trim();
    if principal.is_empty() || !principal.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BootstrapError::Malformed(
            "principal is not a numeric uid".into(),
        ));
    }
    let (system, machine) = (lines[1].trim(), lines[2].trim());
    let target = target_for(system, machine).ok_or_else(|| BootstrapError::UnsupportedTarget {
        system: system.to_string(),
        machine: machine.to_string(),
    })?;
    let cache_base = lines[3].trim();
    if !cache_base.starts_with('/') {
        return Err(BootstrapError::Malformed(
            "cache base is not an absolute native path".into(),
        ));
    }
    Ok(EndpointFacts {
        principal: principal.to_string(),
        system: system.to_string(),
        machine: machine.to_string(),
        target: target.to_string(),
        cache_base: cache_base.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_wraps_every_word_and_escapes_only_single_quotes() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("with space"), "'with space'");
        assert_eq!(quote("don't"), "'don'\\''t'");
        assert_eq!(quote("a'b'c"), "'a'\\''b'\\''c'");
        // Glob, dollar, backtick and newline stay literal inside quotes.
        assert_eq!(quote("$HOME/*`id`\nx"), "'$HOME/*`id`\nx'");
        assert_eq!(quote("\\backslash"), "'\\backslash'");
    }

    #[test]
    fn target_mapping_names_exact_artifact_triples() {
        assert_eq!(
            target_for("Linux", "x86_64"),
            Some("x86_64-unknown-linux-musl")
        );
        assert_eq!(
            target_for("Linux", "aarch64"),
            Some("aarch64-unknown-linux-musl")
        );
        assert_eq!(target_for("Darwin", "arm64"), Some("aarch64-apple-darwin"));
        assert_eq!(target_for("FreeBSD", "x86_64"), None);
        assert_eq!(target_for("Linux", "riscv64"), None);
    }
    #[test]
    fn the_local_target_round_trips_through_the_same_table() {
        let expected = target_for(local_system(), local_machine());
        assert_eq!(local_target(), expected);
        // This workspace ships Linux and macOS workers; the table holds.
        assert!(local_target().is_some());
    }

    #[test]
    fn discovery_reply_decoding_is_strict() {
        let facts = parse_reply(b"1000\nLinux\nx86_64\n/home/alice/.cache\n")
            .expect("well-formed reply decodes");
        assert_eq!(facts.principal, "1000");
        assert_eq!(facts.target, "x86_64-unknown-linux-musl");
        assert_eq!(facts.cache_base, "/home/alice/.cache");

        assert!(matches!(
            parse_reply(b"alice\nLinux\nx86_64\n/home/alice/.cache\n"),
            Err(BootstrapError::Malformed(_))
        ));
        assert!(matches!(
            parse_reply(b"1000\nLinux\nx86_64\nrelative/cache\n"),
            Err(BootstrapError::Malformed(_))
        ));
        assert!(matches!(
            parse_reply(b"1000\nLinux\n"),
            Err(BootstrapError::Malformed(_))
        ));
        assert!(matches!(
            parse_reply(b"1000\nPlan9\nx86_64\n/tmp\n"),
            Err(BootstrapError::UnsupportedTarget { .. })
        ));
    }
}
