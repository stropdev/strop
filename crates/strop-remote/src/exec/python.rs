//! Native interpreter configuration is captured at SSH launch, never in a
//! RemoteCommand constructor. Both finite Git and relayed LSP use this bootstrap.
use super::supervisor::shell_single_quote;
use super::RemoteCommandError;

pub(super) enum PythonInterpreter {
    Discover,
    Explicit(String),
}

const CANDIDATES: &[&str] = &[
    "python3",
    "python3.15",
    "python3.14",
    "python3.13",
    "python3.12",
    "python3.11",
    "python3.10",
    "python3.9",
    "python3.8",
];
const PROBE: &str = "import os,sys,select; sys.exit(0 if sys.version_info >= (3,8) and all(hasattr(os,n) for n in ('fork','setsid','killpg','execvpe','set_blocking','waitpid')) and hasattr(select,'poll') else 1)";

impl PythonInterpreter {
    /// Worker-side native setup. Full replay injects the owning launch outcome
    /// and never evaluates the local environment or probes a remote interpreter.
    pub fn from_environment() -> Result<Self, RemoteCommandError> {
        match std::env::var("STROP_REMOTE_PYTHON") {
            Ok(program) => Self::explicit(program),
            Err(std::env::VarError::NotPresent) => Ok(Self::Discover),
            Err(std::env::VarError::NotUnicode(_)) => Err(RemoteCommandError::Invalid {
                detail: "STROP_REMOTE_PYTHON must be a UTF-8 executable name or absolute path"
                    .into(),
            }),
        }
    }

    fn explicit(program: String) -> Result<Self, RemoteCommandError> {
        if program.is_empty()
            || program.contains('\0')
            || program.starts_with('-')
            || (program.contains('/') && !program.starts_with('/'))
            || program.len() > 4096
        {
            return Err(RemoteCommandError::Invalid {
                detail: "STROP_REMOTE_PYTHON must be a nonempty executable name or absolute path (at most 4096 bytes)".into(),
            });
        }
        Ok(Self::Explicit(program))
    }

    /// Fixed candidate count, no remote directory walk and no helper installation.
    /// The caller's existing launch deadline/cancellation owns the whole probe.
    /// An explicit override has exactly one candidate: failure never falls back.
    pub fn bootstrap(&self, source: &str, spec: &str) -> String {
        let programs = match self {
            Self::Discover => CANDIDATES
                .iter()
                .map(|name| shell_single_quote(name))
                .collect::<Vec<_>>()
                .join(" "),
            Self::Explicit(program) => shell_single_quote(program),
        };
        let failure = match self {
            Self::Discover => "STROP-ERR no compatible Python 3.8+ on remote PATH; set STROP_REMOTE_PYTHON to the remote interpreter".to_string(),
            Self::Explicit(program) => format!("STROP-ERR configured STROP_REMOTE_PYTHON {program:?} is not a usable Python 3.8+ with POSIX process support"),
        };
        // The probe never reads stdin: it cannot consume an LSP protocol byte.
        // Isolated/no-site mode prevents startup hooks and PYTHONPATH from
        // injecting code before the lifetime supervisor owns the remote group.
        format!(
            "for strop_python in {programs}; do if \"$strop_python\" -I -S -c {} 2>/dev/null; then exec \"$strop_python\" -I -S -c {} {}; fi; done; printf '%s\\n' {} >&2; exit 127",
            shell_single_quote(PROBE), shell_single_quote(source), shell_single_quote(spec), shell_single_quote(&failure),
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    fn python() -> std::path::PathBuf {
        let output = Command::new("sh")
            .args(["-c", "command -v python3"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Python is a supervisor fixture prerequisite"
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    fn execute(interpreter: PythonInterpreter, path: &std::path::Path) -> std::process::Output {
        Command::new("/bin/sh")
            .args([
                "-c",
                &interpreter.bootstrap(
                    "import sys; sys.stdout.write(sys.argv[1])",
                    "native payload",
                ),
            ])
            .env("PATH", path)
            .env_remove("PYTHONPATH")
            .output()
            .unwrap()
    }

    #[test]
    fn versioned_only_path_and_explicit_unicode_path_execute() {
        let directory = tempfile::tempdir().unwrap();
        let program = python();
        std::os::unix::fs::symlink(&program, directory.path().join("python3.11")).unwrap();
        let discovered = execute(PythonInterpreter::Discover, directory.path());
        assert!(
            discovered.status.success(),
            "{}",
            String::from_utf8_lossy(&discovered.stderr)
        );
        assert_eq!(discovered.stdout, b"native payload");
        let named = directory.path().join("python ' édition");
        std::os::unix::fs::symlink(program, &named).unwrap();
        let explicit = execute(
            PythonInterpreter::explicit(named.to_str().unwrap().into()).unwrap(),
            directory.path(),
        );
        assert!(
            explicit.status.success(),
            "{}",
            String::from_utf8_lossy(&explicit.stderr)
        );
        assert_eq!(explicit.stdout, b"native payload");
    }

    #[test]
    fn invalid_explicit_program_cannot_execute_shell_syntax_or_fall_back() {
        let directory = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(python(), directory.path().join("python3")).unwrap();
        let marker = directory.path().join("injected");
        let value = format!("/missing'; touch '{}'; #", marker.display());
        let output = execute(
            PythonInterpreter::explicit(value).unwrap(),
            directory.path(),
        );
        assert_eq!(output.status.code(), Some(127));
        assert!(!marker.exists());
        assert!(output.stdout.is_empty());
    }
}
