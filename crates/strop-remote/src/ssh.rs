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
//! The exec path ([`crate::exec`], plan 0090 §5) keeps every option above
//! except the control ones: [`multiplexed_argv`] names a private control
//! socket, a master that forks after authenticating (`-M -N -f`,
//! `ControlPersist`), and clients that open channels on it (`-T`, or
//! `-tt` for a terminal) and never promote themselves to a master.
//!
//! Pure construction: nothing here spawns. [`base_command`] carries no
//! destination at all; the destination (and whether it names the `sftp`
//! subsystem or a supervised remote command) is the caller's one
//! remaining decision.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use strop_workspace::RemoteEndpoint;

/// The safety options every invocation carries, dedicated or
/// multiplexed: no prompt, no unknown host key, no forwarding, no user
/// `RemoteCommand`, plus the endpoint's own user and port.
fn policy_argv(endpoint: &RemoteEndpoint, argv: &mut Vec<OsString>) {
    for option in [
        "-a",
        "-x",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "RemoteCommand=none",
    ] {
        argv.push(option.into());
    }
    if let Some(user) = endpoint.user() {
        argv.push("-l".into());
        argv.push(user.into());
    }
    if let Some(port) = endpoint.port() {
        argv.push("-p".into());
        argv.push(port.get().to_string().into());
    }
}

/// The option argv — program excluded — shared by every dedicated
/// (unmultiplexed) transport.
///
/// Exposed separately from [`base_command`] so policy tests assert the
/// exact argv vector instead of pinning `Command` debug formatting.
pub(crate) fn base_argv(endpoint: &RemoteEndpoint) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec!["-T".into()];
    policy_argv(endpoint, &mut argv);
    for option in [
        "ControlMaster=no",
        "ControlPath=none",
        "ForkAfterAuthentication=no",
        "ControlPersist=no",
    ] {
        argv.push("-o".into());
        argv.push(option.into());
    }
    argv
}

/// One invocation's role on a per-endpoint multiplexed connection
/// (plan 0090 §5): the master that authenticates once, or a client that
/// opens one channel on it.
pub(crate) enum Mux<'a> {
    /// `ssh -M -N -f`: authenticate, then fork into the background and
    /// serve channels on `socket` until idle for `persist`. ssh's own
    /// diagnostics go to `log`, never to an inherited pipe.
    Master {
        socket: &'a Path,
        persist: &'a str,
        log: &'a Path,
    },
    /// One channel on an existing master. `tty` forces a remote PTY
    /// (`-tt`) for terminals; otherwise none (`-T`).
    Client {
        socket: &'a Path,
        log: &'a Path,
        tty: bool,
    },
}

/// The option argv for a multiplexed invocation: the same safety policy
/// as [`base_argv`], with the control socket instead of `ControlPath=none`.
///
/// Clients pass `ControlMaster=no`: when the master is gone they fall
/// back to a direct (still noninteractive) connection rather than
/// promoting themselves to a backgrounded master that would inherit the
/// caller's pipes.
pub(crate) fn multiplexed_argv(endpoint: &RemoteEndpoint, mux: &Mux<'_>) -> Vec<OsString> {
    let mut argv = Vec::new();
    let (socket, log) = match mux {
        Mux::Master {
            socket,
            persist,
            log,
        } => {
            argv.extend(
                ["-T", "-M", "-N", "-f", "-o", "ControlMaster=yes", "-o"].map(OsString::from),
            );
            argv.push(format!("ControlPersist={persist}").into());
            (socket, log)
        }
        Mux::Client { socket, log, tty } => {
            argv.push(if *tty { "-tt" } else { "-T" }.into());
            argv.extend(["-o", "ControlMaster=no"].map(OsString::from));
            (socket, log)
        }
    };
    policy_argv(endpoint, &mut argv);
    let mut control = OsString::from("ControlPath=");
    control.push(socket.as_os_str());
    argv.extend([
        OsString::from("-o"),
        control,
        "-E".into(),
        log.as_os_str().into(),
    ]);
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

/// A dedicated noninteractive OpenSSH command channel. The bootstrap
/// and deployed-worker owners build `remote_line` from fixed source
/// with dynamic words quoted by [`crate::bootstrap::quote`]. It is one
/// shell-parsed command string, never a native argv transport.
pub(crate) fn exec_command(endpoint: &RemoteEndpoint, remote_line: &str) -> Command {
    let mut command = base_command(endpoint);
    command.arg("--").arg(endpoint.host()).arg(remote_line);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
