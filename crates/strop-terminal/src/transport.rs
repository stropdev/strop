//! Where one session's shell runs (0090 §4). Every session is an
//! in-process PTY; a remote one is that PTY running `ssh -tt` on the
//! endpoint's multiplexed connection, so Ctrl-C is a byte into the PTY
//! and a resize is `TIOCSWINSZ`, which ssh forwards as a window change.
use crate::{launch::Launch, Error};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use strop_remote::exec::ExecHost;

/// The caller's choice of where the shell runs.
#[derive(Debug, Clone)]
pub enum Target {
    /// This machine: the launch runs as given.
    Local,
    /// The endpoint behind `host`: the launch's program, arguments and
    /// directory are the remote side's (see [`Launch::shell_remote`]).
    Remote(ExecHost),
}

/// What the local PTY runs for `launch` on `target`, and whether
/// typeahead is held until the child first speaks.
pub(crate) fn local_launch(target: &Target, launch: &Launch) -> Result<(Launch, bool), Error> {
    let Target::Remote(host) = target else {
        return Ok((launch.clone(), false));
    };
    let argv = remote_argv(launch);
    let argv: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
    let ssh = host.terminal(&argv, Some(&launch.directory));
    // ssh itself needs no particular directory; the editor's own keeps
    // any relative path in the user's ssh configuration meaningful.
    let directory = std::env::current_dir()
        .ok()
        .filter(|directory| directory.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"));
    let local = Launch {
        program: ssh.get_program().to_os_string(),
        arguments: ssh.get_args().map(OsStr::to_os_string).collect(),
        directory,
        // ssh sends its own TERM in the PTY request.
        environment: vec![("TERM".into(), "xterm-256color".into())],
    };
    local.validate()?;
    Ok((local, true))
}

/// The remote argv: ssh forwards no environment beyond TERM, so the
/// launch's variables travel as `env NAME=VALUE` words ahead of it.
fn remote_argv(launch: &Launch) -> Vec<OsString> {
    let mut argv = Vec::with_capacity(2 + launch.environment.len() + launch.arguments.len());
    argv.push(OsString::from("env"));
    for (name, value) in &launch.environment {
        let mut assignment = name.clone();
        assignment.push("=");
        assignment.push(value);
        argv.push(assignment);
    }
    argv.push(launch.program.clone());
    argv.extend(launch.arguments.iter().cloned());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What sshd runs after `cd <directory> &&`: the remote argv lands in
    /// the user's login shell with the terminal identity set.
    #[test]
    fn the_remote_argv_runs_a_login_shell_with_the_terminal_identity() {
        let home = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let launch = Launch::shell_remote(
            directory.path().to_path_buf(),
            Some(r#"printf '<%s|%s|%s|%s>' "$TERM_PROGRAM" "$COLORTERM" "$0" "$(pwd -P)""#.into()),
        );
        let argv = remote_argv(&launch);
        let output = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(&launch.directory)
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let expected = format!(
            "<strop|truecolor|/bin/sh|{}>",
            directory.path().canonicalize().unwrap().display()
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(&expected),
            "{output:?}"
        );
    }
}
