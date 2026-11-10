//! The one safe `ssh(1)` invocation policy for every strop-remote
//! transport (plan 0090 §5): one multiplexed master per endpoint and
//! channels on it, so no second flag set can drift into existence.
//!
//! User configuration stays in force — aliases, `Include`/`Match`, keys,
//! agent, `ProxyJump` — because no `-F` is passed and no identity options
//! override it. The options here remove every interactive or
//! side-effecting capability a general `ssh` invocation could have:
//!
//! - `BatchMode=yes` fails authentication instead of asking;
//! - `StrictHostKeyChecking=yes` refuses unknown and changed host keys —
//!   never auto-accepts;
//! - `-a`/`-x`/`ClearAllForwardings` disable agent, X11 and port
//!   forwarding;
//! - `RemoteCommand=none` neutralizes the same-named user option;
//! - the control socket lives in a private directory the editor owns:
//!   the master authenticates once and forks (`-M -N -f`,
//!   `ControlPersist`), and clients open channels on it (`-T`, or `-tt`
//!   for a terminal) and never promote themselves to a master.
//!
//! Pure construction: nothing here spawns.

use std::ffi::OsString;
use std::path::Path;
use strop_workspace::RemoteEndpoint;

/// The safety options every invocation carries: no prompt, no unknown host key, no forwarding, no user
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

/// The option argv for one invocation: the safety policy plus the
/// control socket.
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
