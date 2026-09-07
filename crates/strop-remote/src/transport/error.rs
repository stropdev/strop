//! Typed remote-read failures: the stage that stopped, a classified kind,
//! bounded ssh(1) stderr and an actionable hint. Errors are diagnostics
//! data, never buffer identity.

use std::fmt;

/// Where a remote read stopped. Retained so diagnostics name the phase
/// instead of a bare transport error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadStage {
    /// Starting the local ssh(1) client.
    Spawn,
    /// Negotiating the SFTP session: connection, authentication, host
    /// keys, subsystem availability.
    Connect,
    /// Opening the remote file by name through SFTP.
    Open,
    /// Proving the opened handle is a regular file of known length.
    Inspect,
    /// Transferring exactly the length captured at inspection.
    Transfer,
    /// Validating the snapshot as UTF-8 text.
    Validate,
    /// Closing file and session within the deadline.
    Teardown,
}

impl ReadStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::Connect => "connect",
            Self::Open => "open",
            Self::Inspect => "inspect",
            Self::Transfer => "transfer",
            Self::Validate => "validate",
            Self::Teardown => "teardown",
        }
    }
}

impl fmt::Display for ReadStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What went wrong, classified far enough to act on. Auth, trust and
/// install problems carry hints; the rest are honest transport facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadFailureKind {
    /// The local ssh(1) executable could not be started.
    Spawn,
    /// Noninteractive authentication failed (BatchMode refuses prompts).
    Auth,
    /// Host key unknown or changed; trust is refused, never auto-accepted.
    Trust,
    /// Network-level failure: DNS, routing, refusal, reset.
    Network,
    /// The remote host does not offer the sftp subsystem.
    Subsystem,
    /// Session establishment failed without a finer classification.
    Connect,
    /// The remote path does not exist.
    NotFound,
    /// The server denied access to the path.
    Permission,
    /// The SFTP protocol exchange failed.
    Protocol,
    /// Local pipe/transport I/O failure.
    Io,
    /// The opened handle is not proven to be a regular file.
    NotRegularFile,
    /// The server reported no length for the opened handle.
    UnknownLength,
    /// The snapshot exceeds the in-memory cap.
    TooLarge,
    /// Fewer bytes than the captured length arrived.
    ShortRead,
    /// The snapshot is not valid UTF-8.
    InvalidUtf8,
    /// The read was cancelled by its owner.
    Cancelled,
    /// The total connection/read/close deadline elapsed.
    Deadline,
    /// Process supervision is unavailable on this platform.
    #[cfg(not(unix))]
    Unsupported,
}

impl ReadFailureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Spawn => "local ssh unavailable",
            Self::Auth => "authentication failed",
            Self::Trust => "host key refused",
            Self::Network => "network failure",
            Self::Subsystem => "sftp subsystem unavailable",
            Self::Connect => "connection failed",
            Self::NotFound => "remote file missing",
            Self::Permission => "remote access denied",
            Self::Protocol => "sftp protocol failure",
            Self::Io => "transport I/O failure",
            Self::NotRegularFile => "not a regular file",
            Self::UnknownLength => "unknown file length",
            Self::TooLarge => "snapshot too large",
            Self::ShortRead => "short read",
            Self::InvalidUtf8 => "invalid UTF-8",
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline exceeded",
            #[cfg(not(unix))]
            Self::Unsupported => "unsupported platform",
        }
    }
}

impl fmt::Display for ReadFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The failure a worker sees before diagnostics are attached. The
/// orchestrator turns it into a [`RemoteReadError`] with stderr, exit
/// status and the remote URI.
#[derive(Debug, Clone)]
pub(super) struct Fault {
    stage: ReadStage,
    kind: ReadFailureKind,
    detail: String,
}

impl Fault {
    pub(super) fn new(stage: ReadStage, kind: ReadFailureKind, detail: impl Into<String>) -> Self {
        Self {
            stage,
            kind,
            detail: detail.into(),
        }
    }

    /// Session establishment failed; the orchestrator refines the kind
    /// once ssh's retained stderr is available.
    pub(super) fn connect(detail: impl Into<String>) -> Self {
        Self::new(ReadStage::Connect, ReadFailureKind::Connect, detail)
    }

    pub(super) fn cancelled(stage: ReadStage) -> Self {
        Self::new(stage, ReadFailureKind::Cancelled, "the read was cancelled")
    }

    pub(super) fn deadline(stage: ReadStage) -> Self {
        Self::new(
            stage,
            ReadFailureKind::Deadline,
            "connection, transfer or close exceeded the total deadline",
        )
    }

    fn into_parts(self) -> (ReadStage, ReadFailureKind, String) {
        (self.stage, self.kind, self.detail)
    }
}

/// A fully diagnosed remote read failure. Carries everything a user needs
/// to act: stage, classification, detail, bounded ssh stderr, the child's
/// exit line when observed, and a hint for fixable causes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteReadError {
    remote: String,
    stage: ReadStage,
    kind: ReadFailureKind,
    detail: String,
    stderr: Option<String>,
    exit: Option<String>,
}

impl RemoteReadError {
    /// A failure with no diagnostics to attach (early exit, unsupported
    /// platform, spawn refusal).
    pub(super) fn bare(stage: ReadStage, kind: ReadFailureKind, detail: impl Into<String>) -> Self {
        Self {
            remote: String::new(),
            stage,
            kind,
            detail: detail.into(),
            stderr: None,
            exit: None,
        }
    }

    pub(super) fn fault(remote: &str, fault: Fault) -> Self {
        let (stage, kind, detail) = fault.into_parts();
        Self {
            remote: remote.to_owned(),
            stage,
            kind,
            detail,
            stderr: None,
            exit: None,
        }
    }

    pub(super) fn remote(mut self, remote: &str) -> Self {
        self.remote = remote.to_owned();
        self
    }

    pub(super) fn stderr(mut self, stderr: Option<String>) -> Self {
        self.stderr = stderr;
        self
    }

    pub(super) fn exit(mut self, exit: Option<String>) -> Self {
        self.exit = exit;
        self
    }

    pub fn kind(&self) -> ReadFailureKind {
        self.kind
    }

    /// Cancellation is reported so callers can prefer their own outcome.
    pub fn is_cancellation(&self) -> bool {
        self.kind == ReadFailureKind::Cancelled
    }

    /// An actionable instruction, when one exists.
    pub fn hint(&self) -> Option<&'static str> {
        match self.kind {
            ReadFailureKind::Spawn => {
                Some("install the OpenSSH client; strop runs ssh(1) found on PATH")
            }
            ReadFailureKind::Auth => Some(
                "strop authenticates noninteractively: load the key into ssh-agent \
                 or configure it in ~/.ssh/config, then verify `ssh` to the host \
                 answers without any prompt",
            ),
            ReadFailureKind::Trust => Some(
                "connect to the host once outside strop to establish trust, or \
                 repair its known_hosts entry; strop never accepts an unknown or \
                 changed host key",
            ),
            ReadFailureKind::Subsystem => {
                Some("the remote sshd must offer the SFTP subsystem (internal-sftp)")
            }
            ReadFailureKind::TooLarge => Some(
                "remote snapshots are capped at 256 MiB in memory; read a \
                 smaller file or tail it on the host",
            ),
            ReadFailureKind::InvalidUtf8 => {
                Some("strop buffers are text; this remote file is not valid UTF-8")
            }
            ReadFailureKind::Deadline => Some(
                "connection, transfer and close must finish within the total \
                 deadline; check reachability and file size",
            ),
            #[cfg(not(unix))]
            ReadFailureKind::Unsupported => Some("remote reads require Unix process supervision"),
            _ => None,
        }
    }

    /// Refine a generic connect failure now that ssh's stderr and the
    /// protocol detail are both known. ssh's stderr wording is its
    /// documented diagnostic surface, not an implementation detail of strop.
    pub(super) fn refine_connect(&mut self) {
        debug_assert_eq!(self.kind, ReadFailureKind::Connect);
        let mut haystack = format!("{}\n{}", self.detail, self.stderr.as_deref().unwrap_or(""));
        haystack.make_ascii_lowercase();
        self.kind = classify_connect(&haystack);
    }
}

fn classify_connect(haystack: &str) -> ReadFailureKind {
    const AUTH: &[&str] = &[
        "permission denied",
        "authentication failed",
        "no supported authentication methods",
        "too many authentication failures",
        "passphrase",
    ];
    const TRUST: &[&str] = &[
        "host key verification failed",
        "host key for server changed",
    ];
    const NETWORK: &[&str] = &[
        "could not resolve hostname",
        "name or service not known",
        "connection refused",
        "timed out",
        "timeout",
        "network is unreachable",
        "no route to host",
        "connection reset",
        "connection aborted",
    ];
    const SUBSYSTEM: &[&str] = &["subsystem request failed"];
    if TRUST.iter().any(|needle| haystack.contains(needle)) {
        return ReadFailureKind::Trust;
    }
    if AUTH.iter().any(|needle| haystack.contains(needle)) {
        return ReadFailureKind::Auth;
    }
    if SUBSYSTEM.iter().any(|needle| haystack.contains(needle)) {
        return ReadFailureKind::Subsystem;
    }
    if NETWORK.iter().any(|needle| haystack.contains(needle)) {
        return ReadFailureKind::Network;
    }
    ReadFailureKind::Connect
}

impl fmt::Display for RemoteReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.remote.is_empty() {
            write!(
                formatter,
                "remote read failed at {}: {}",
                self.stage, self.detail
            )?;
        } else {
            write!(
                formatter,
                "remote read of {} failed at {}: {}",
                self.remote, self.stage, self.detail
            )?;
        }
        if let Some(stderr) = &self.stderr {
            write!(formatter, "\nssh stderr: {stderr}")?;
        }
        if let Some(exit) = &self.exit {
            write!(formatter, "\nssh exit: {exit}")?;
        }
        if let Some(hint) = self.hint() {
            write!(formatter, "\nhint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RemoteReadError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn refined(detail: &str, stderr: &str) -> ReadFailureKind {
        let mut error = RemoteReadError::fault("ssh://host/file", Fault::connect(detail))
            .stderr(Some(stderr.to_owned()));
        error.refine_connect();
        error.kind()
    }

    #[test]
    fn host_key_refusal_is_trust() {
        // ssh(1) prints this for unknown and changed keys under BatchMode.
        assert_eq!(
            refined("", "Host key verification failed."),
            ReadFailureKind::Trust
        );
    }

    #[test]
    fn batchmode_auth_failure_is_auth() {
        assert_eq!(
            refined("", "user@host: Permission denied (publickey)."),
            ReadFailureKind::Auth
        );
    }

    #[test]
    fn missing_subsystem_is_named() {
        assert_eq!(
            refined("", "subsystem request failed on channel 0"),
            ReadFailureKind::Subsystem
        );
    }

    #[test]
    fn network_failures_are_network() {
        assert_eq!(
            refined(
                "",
                "ssh: connect to host devbox port 22: Connection refused"
            ),
            ReadFailureKind::Network
        );
    }

    #[test]
    fn unclassified_stays_connect() {
        assert_eq!(
            refined("hello message invalid", ""),
            ReadFailureKind::Connect
        );
    }

    #[test]
    fn trust_outranks_auth_wording() {
        // A changed key can also mention permission denied; trust is the
        // actionable classification.
        assert_eq!(
            refined("", "Host key verification failed.\nPermission denied."),
            ReadFailureKind::Trust
        );
    }

    #[test]
    fn display_carries_context_without_invented_hint() {
        let error = RemoteReadError::fault(
            "ssh://devbox/var/log/app.log",
            Fault::new(
                ReadStage::Transfer,
                ReadFailureKind::ShortRead,
                "expected 10 bytes, received 4",
            ),
        )
        .stderr(Some("killed".to_owned()))
        .exit(Some("signal: 9 (SIGKILL)".to_owned()));
        let text = error.to_string();
        assert!(text.contains("ssh://devbox/var/log/app.log"));
        assert!(text.contains("transfer"));
        assert!(text.contains("expected 10 bytes, received 4"));
        assert!(text.contains("ssh stderr: killed"));
        assert!(text.contains("ssh exit: signal: 9"));
        assert!(!text.contains("hint:"));
    }

    #[test]
    fn cancellation_is_identifiable() {
        let error = RemoteReadError::fault("ssh://h/f", Fault::cancelled(ReadStage::Transfer));
        assert!(error.is_cancellation());
    }
}
