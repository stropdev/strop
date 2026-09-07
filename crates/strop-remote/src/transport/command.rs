//! The ssh(1) invocation for one remote SFTP read. Pure construction, no
//! side effects: the remote path never appears in this argv — filenames
//! travel only inside the SFTP protocol on the session's pipes.

use super::super::address::RemoteFile;
use std::process::{Command, Stdio};

/// A dedicated, noninteractive connection to the remote `sftp` subsystem.
///
/// User configuration stays in force (aliases, Include/Match, keys, agent,
/// ProxyJump) because no `-F` is passed and no identity options override
/// it. The options here remove every interactive or side-effecting
/// capability a general `ssh` invocation could have:
///
/// - `-s` runs the named subsystem instead of a remote shell;
/// - `-T` never allocates a TTY, so nothing can prompt;
/// - `BatchMode=yes` fails authentication instead of asking;
/// - `StrictHostKeyChecking=yes` refuses unknown and changed host keys —
///   never auto-accepts;
/// - `-a`/`-x`/`ClearAllForwardings` disable agent, X11 and port
///   forwarding;
/// - `ControlMaster=no` with `ControlPath=none` creates no multiplexed
///   master and attaches to none, so cancelling this read can neither
///   leak a master process nor disturb the user's other sessions.
pub(super) fn sftp_subsystem(file: &RemoteFile) -> Command {
    let mut command = Command::new("ssh");
    command
        .arg("-s")
        .arg("-T")
        .arg("-a")
        .arg("-x")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=yes")
        .arg("-o")
        .arg("ClearAllForwardings=yes")
        .arg("-o")
        .arg("ControlMaster=no")
        .arg("-o")
        .arg("ControlPath=none")
        .arg("-o")
        .arg("RemoteCommand=none")
        .arg("-o")
        .arg("ForkAfterAuthentication=no")
        .arg("-o")
        .arg("ControlPersist=no");
    if let Some(user) = file.user() {
        command.arg("-l").arg(user);
    }
    if let Some(port) = file.port() {
        command.arg("-p").arg(port.get().to_string());
    }
    // `--` ends option parsing so a host alias cannot be read as a flag.
    // The host is a standalone argv — literal IPv6 needs no brackets here —
    // and the only operand after it is the subsystem name.
    command.arg("--").arg(file.host()).arg("sftp");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}
