//! The interactive exec channel (DC1b): `docker exec -i` for programs
//! that speak a protocol over stdio — language servers, Git. Owned like
//! every strop process: piped stdio, argv arrays (never a shell), and
//! the local client's lifetime bounds the in-container program (stdin
//! EOF on drop; the daemon ends the session).
//!
//! There is deliberately no pre-probe for "does this binary exist":
//! on a distroless container no shell or `which` exists to answer, and
//! the spawn itself classifies a missing executable truthfully from the
//! engine's own error.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::engine::{capture, Captured, EngineRef};
use crate::ContainerError;
use strop_core::worker::CancelToken;
use strop_workspace::ContainerId;

/// A `docker exec -i` command channel for one in-container program:
/// piped stdin/stdout/stderr, working directory inside the container.
/// The caller owns process supervision (group, kill-on-drop) as with
/// every strop process launch. The id is the canonical inspect id —
/// incarnation pinning is the read path's concern; a spawned session's
/// liveness is scoped to itself.
pub fn exec_command(id: &ContainerId, program: &str, args: &[String], cwd: &Path) -> Command {
    let mut command = Command::new("docker");
    command
        .arg("exec")
        .arg("-i")
        .arg("--workdir")
        .arg(cwd)
        .arg(id.as_str())
        .arg(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Wall-clock budget for one bounded in-container command run.
const EXEC_DEADLINE: Duration = Duration::from_secs(30);

/// One bounded command run inside the container (0037 DC1b): Git and
/// friends. argv-only, supervised, deadline-bounded pipes. The engine
/// was probed by the caller; the container id is canonical.
pub fn exec_capture(
    engine: &EngineRef,
    id: &ContainerId,
    program: &str,
    args: &[String],
    cwd: &Path,
    stdout_limit: u64,
    token: &CancelToken,
) -> Result<Captured, ContainerError> {
    let _ = engine;
    let mut argv: Vec<&str> = vec!["exec", "--workdir"];
    let Some(cwd_text) = cwd.to_str() else {
        return Err(ContainerError::CapabilityRefused {
            what: "exec: the working directory is not UTF-8".into(),
        });
    };
    argv.push(cwd_text);
    argv.push(id.as_str());
    argv.push(program);
    argv.extend(args.iter().map(String::as_str));
    capture(&argv, stdout_limit, EXEC_DEADLINE, token)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn reference() -> ContainerId {
        ContainerId::canonical("a".repeat(64)).unwrap()
    }

    #[test]
    fn exec_command_is_argv_only_with_piped_stdio() {
        let command = exec_command(
            &reference(),
            "pyright-langserver",
            &["--stdio".into()],
            &PathBuf::from("/src"),
        );
        let text = format!("{command:?}");
        assert!(text.contains("exec"), "{text}");
        assert!(text.contains("--workdir"), "{text}");
        assert!(text.contains("/src"), "{text}");
        assert!(text.contains(&"a".repeat(64)), "{text}");
        assert!(text.contains("pyright-langserver"), "{text}");
        assert!(!text.contains("sh -c"), "no shell, ever: {text}");
    }
}
