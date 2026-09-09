//! The one safe `ssh(1)` invocation policy for every strop-remote
//! transport (crate-private; pooled SFTP and supervised exec both build
//! on it, so no second flag set can drift into existence).
//!
//! User configuration stays in force — aliases, `Include`/`Match`, keys,
//! agent, `ProxyJump` — because no `-F` is passed and no identity options
//! override it. The options here remove every interactive or
//! side-effecting capability a general `ssh` invocation could have:
//!
//! - `-T` never allocates a TTY, so nothing can prompt or echo;
//! - `BatchMode=yes` fails authentication instead of asking;
//! - `StrictHostKeyChecking=yes` refuses unknown and changed host keys —
//!   never auto-accepts;
//! - `-a`/`-x`/`ClearAllForwardings` disable agent, X11 and port
//!   forwarding;
//! - `ControlMaster=no` with `ControlPath=none` creates no multiplexed
//!   master and attaches to none, so cancelling this operation can
//!   neither leak a master process nor disturb the user's other
//!   sessions, and termination of the local `ssh` really terminates the
//!   transport rather than a client of a persistent master;
//! - `RemoteCommand=none` and `ForkAfterAuthentication=no` neutralize
//!   the same-named user options.
//!
//! Pure construction: nothing here spawns. [`base_command`] carries no
//! destination at all; the destination (and whether it names the `sftp`
//! subsystem or a supervised remote command) is the caller's one
//! remaining decision.

use std::ffi::OsString;
use std::process::{Command, Stdio};
use strop_workspace::RemoteEndpoint;

/// The option argv — program excluded — shared by every transport.
///
/// Exposed separately from [`base_command`] so policy tests assert the
/// exact argv vector instead of pinning `Command` debug formatting.
pub(crate) fn base_argv(endpoint: &RemoteEndpoint) -> Vec<OsString> {
    let mut argv: Vec<OsString> = [
        "-T",
        "-a",
        "-x",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "-o",
        "RemoteCommand=none",
        "-o",
        "ForkAfterAuthentication=no",
        "-o",
        "ControlPersist=no",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    if let Some(user) = endpoint.user() {
        argv.push("-l".into());
        argv.push(user.into());
    }
    if let Some(port) = endpoint.port() {
        argv.push("-p".into());
        argv.push(port.get().to_string().into());
    }
    argv
}

/// A dedicated, noninteractive connection with no destination selected.
/// Stdio is left to the caller.
pub(crate) fn base_command(endpoint: &RemoteEndpoint) -> Command {
    let mut command = Command::new("ssh");
    for argument in base_argv(endpoint) {
        command.arg(argument);
    }
    command
}

/// A dedicated, noninteractive connection to the remote `sftp`
/// subsystem, with all three pipes owned by the caller.
///
/// `--` ends option parsing so a host alias cannot be read as a flag;
/// the host is a standalone argv (literal IPv6 needs no brackets here)
/// and the only operand after it is the subsystem name. File SFTP stays
/// shell-independent: remote path names never appear in this argv.
pub(crate) fn sftp_subsystem(endpoint: &RemoteEndpoint) -> Command {
    let mut command = base_command(endpoint);
    command.arg("-s").arg("--").arg(endpoint.host()).arg("sftp");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// A dedicated, noninteractive connection whose remote command is the
/// supervised lifecycle bootstrap (`remote_line`), with all three pipes
/// piped for the caller to own.
///
/// `remote_line` is one argv element — the entire supervised command
/// string the remote login shell will parse. OpenSSH remote command
/// arguments are not a native argv transport, so `remote_line` must
/// already be correctly POSIX single-quote escaped by its builder (see
/// `exec::supervisor::command_line`).
pub(crate) fn exec_command(endpoint: &RemoteEndpoint, remote_line: &str) -> Command {
    let mut command = base_command(endpoint);
    command.arg("--").arg(endpoint.host()).arg(remote_line);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
