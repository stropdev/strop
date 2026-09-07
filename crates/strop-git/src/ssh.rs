//! OpenSSH effective configuration (0033 finding 1): an SSH alias's
//! real hostname is what `ssh -G` says — `Include`, wildcard `Host`
//! and `HostName` rules included — not what a partial home-grown
//! parser guesses from `~/.ssh/config`.
//!
//! Evaluation spawns a process, so it is owned IO-worker work (never
//! the permalink input/render path); `parse_effective_hostname` is
//! the pure half, testable against canned output.

use std::process::Command;

use crate::permalink::is_safe_host;
use strop_core::worker::{CancelToken, FailureKind};

/// Why effective-host evaluation failed, typed at the boundary the UI
/// reports it. None of these ever carries a guessed hostname.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveHostError {
    /// The host text cannot be a hostname or alias — never spawned.
    InvalidHost,
    /// The ssh program could not run.
    Spawn(String),
    /// `ssh -G` exited non-zero; carries its stderr.
    Failed(String),
    /// `ssh -G` produced no usable `hostname` line.
    NoHostname,
    Process(strop_core::worker::Failure),
    Unresolved,
}

impl std::fmt::Display for EffectiveHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EffectiveHostError::InvalidHost => {
                write!(f, "not a valid hostname or alias")
            }
            EffectiveHostError::Spawn(message) => write!(f, "cannot run ssh: {message}"),
            EffectiveHostError::Failed(message) => write!(f, "ssh -G failed: {message}"),
            EffectiveHostError::NoHostname => write!(f, "ssh -G reported no hostname"),
            EffectiveHostError::Process(failure) => write!(f, "ssh -G: {}", failure.message),
            EffectiveHostError::Unresolved => {
                write!(f, "SSH alias has no configured web hostname; check ssh -G")
            }
        }
    }
}

impl std::error::Error for EffectiveHostError {}

/// The alias's effective hostname per OpenSSH's full configuration.
/// `ssh -G` resolves and prints the configuration without connecting.
pub fn effective_host(
    remote: &crate::permalink::AliasRemote,
    token: &CancelToken,
) -> Result<String, EffectiveHostError> {
    let mut command = Command::new("ssh");
    if let Some(user) = &remote.user {
        command.arg("-l").arg(user);
    }
    if let Some(port) = remote.port {
        command.arg("-p").arg(port.to_string());
    }
    effective_host_via(&mut command, remote.host(), token)
}

/// Same, with the ssh program named — the seam hermetic tests drive
/// with a fake binary. The host is validated before anything spawns
/// and then rides one argv element after `-G`: no shell, no string
/// concatenation, no option position.
fn effective_host_via(
    command: &mut Command,
    host: &str,
    token: &CancelToken,
) -> Result<String, EffectiveHostError> {
    let host = is_safe_host(host)
        .then_some(host)
        .ok_or(EffectiveHostError::InvalidHost)?;
    command.arg("-G").arg(host);
    let output = strop_core::process::capture(command, token).map_err(|failure| {
        if failure.kind == FailureKind::Spawn {
            EffectiveHostError::Spawn(failure.message)
        } else {
            EffectiveHostError::Process(failure)
        }
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(EffectiveHostError::Failed(stderr.trim().to_string()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hostname = parse_effective_hostname(&stdout).ok_or(EffectiveHostError::NoHostname)?;
    if hostname == host && !hostname.contains('.') {
        return Err(EffectiveHostError::Unresolved);
    }
    Ok(hostname)
}

/// Pull the effective `hostname` value out of `ssh -G` output. First
/// usable line wins; the value must still be hostname-shaped, so a
/// corrupt line cannot smuggle text into a URL.
pub fn parse_effective_hostname(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("hostname"), Some(host)) if is_safe_host(host) => Some(host.to_string()),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(program: &str, host: &str) -> Result<String, EffectiveHostError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let program = program.to_owned();
        let host = host.to_owned();
        let handle = strop_core::worker::spawn(
            "ssh-test",
            move |outcome| {
                tx.send(outcome).unwrap();
            },
            move |token| {
                strop_core::worker::Outcome::Success(effective_host_via(
                    &mut Command::new(program),
                    &host,
                    &token,
                ))
            },
        );
        let strop_core::worker::Outcome::Success(result) = rx.recv().unwrap() else {
            panic!("worker failed")
        };
        drop(handle);
        result
    }

    /// Write a fake ssh that records its argv and prints canned
    /// effective configuration. Hermetic: no real ssh, no HOME read,
    /// no network — `ssh -G` output is decided by the script.
    #[cfg(unix)]
    fn fake_ssh(dir: &std::path::Path, hostname: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-ssh");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.argv\"\nprintf 'user git\\nhostname {hostname}\\nport 22\\n'\n",
        );
        std::fs::write(&path, script).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[test]
    fn parses_effective_hostname_from_g_output() {
        let output = "user git\nhostname bbgithub.dev.bloomberg.com\nport 22\n";
        assert_eq!(
            parse_effective_hostname(output).as_deref(),
            Some("bbgithub.dev.bloomberg.com")
        );
        // tab-separated keys (ssh -G has used both shapes)
        assert_eq!(
            parse_effective_hostname("hostname\thost.example.com").as_deref(),
            Some("host.example.com")
        );
        assert_eq!(parse_effective_hostname("user git\nport 22"), None);
        assert_eq!(parse_effective_hostname(""), None);
    }

    /// A resolved hostname that is not hostname-shaped is refused —
    /// nothing malformed rides into a URL.
    #[test]
    fn refuses_non_hostname_shaped_output() {
        assert_eq!(parse_effective_hostname("hostname -oProxy"), None);
        assert_eq!(parse_effective_hostname("hostname "), None);
    }

    /// The spawn boundary: the host is ONE argv element after `-G` —
    /// never a shell string, never an option position — and the
    /// effective hostname maps through.
    #[cfg(unix)]
    #[test]
    fn effective_host_spawns_one_safe_argv_element() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = fake_ssh(dir.path(), "bbgithub.dev.bloomberg.com");
        let argv_file = dir.path().join("fake-ssh.argv");

        let host = resolve(ssh.to_str().unwrap(), "bbgithub").unwrap();
        assert_eq!(host, "bbgithub.dev.bloomberg.com");
        assert_eq!(
            std::fs::read_to_string(&argv_file).unwrap(),
            "-G\nbbgithub\n",
            "argv must be exactly [-G, bbgithub]"
        );
    }

    /// Option-shaped host text never reaches a process: InvalidHost,
    /// and the binary was not executed.
    #[cfg(unix)]
    #[test]
    fn option_shaped_hosts_never_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = fake_ssh(dir.path(), "should-not-run");
        let argv_file = dir.path().join("fake-ssh.argv");
        for host in ["-oProxyCommand=evil", "", "git@bb", "bb github"] {
            assert_eq!(
                resolve(ssh.to_str().unwrap(), host),
                Err(EffectiveHostError::InvalidHost),
                "should refuse: {host:?}"
            );
        }
        assert!(!argv_file.exists(), "no process may run for invalid hosts");
    }

    /// A failing `ssh -G` surfaces its stderr, not a guess.
    #[cfg(unix)]
    #[test]
    fn failing_ssh_reports_stderr() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("failing-ssh");
        std::fs::write(
            &path,
            "#!/bin/sh\necho 'Bad configuration option.' >&2\nexit 255\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();

        match resolve(path.to_str().unwrap(), "bbgithub") {
            Err(EffectiveHostError::Failed(message)) => {
                assert!(message.contains("Bad configuration option."), "{message}")
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// A missing ssh program is a Spawn failure, not a guess.
    #[test]
    fn missing_program_is_spawn_failure() {
        let missing = "/nonexistent/strop-test-ssh";
        match resolve(missing, "bbgithub") {
            Err(EffectiveHostError::Spawn(_)) => {}
            other => panic!("expected Spawn, got {other:?}"),
        }
    }
}
