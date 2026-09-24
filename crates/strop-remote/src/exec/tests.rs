//! Real-boundary tests for the supervised remote execution layer.
//!
//! Everything here runs the actual supervisor end to end through a real
//! POSIX `sh` (the same login-shell boundary ssh hands the command to)
//! and a real `python3` — the exact program text `command()` sends — so
//! spec encoding, shell quoting, the Python parser, the anchor/worker
//! lifecycle and the status-record channel are all exercised together.
//! The ssh transport itself is covered by the repository's real SSH
//! fixtures (Main's `remote_ssh.rs`); these tests deliberately do not
//! fake it.

use super::run::{classify, STDERR_LIMIT, STDERR_TAIL, STDOUT_LIMIT};
use super::spec::Spec;
use super::supervisor;
use super::{
    RemoteCommand, RemoteCommandError, RemoteExitStatus, StdinMode, SupervisionKey,
    SupervisionOutcome,
};
use crate::test_support::in_worker;
use std::ffi::OsString;
use std::io::{BufRead as _, Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use strop_core::process::{capture_with, CaptureError, CapturePolicy, StdinPolicy};

/// The supervised command line aimed at a local login-shell stand-in:
/// identical construction to `command()`, minus the ssh hop.
fn local_supervised(
    command: &RemoteCommand,
    mode: StdinMode,
) -> Result<(Command, SupervisionKey), RemoteCommandError> {
    let key = SupervisionKey::generate();
    let spec = Spec::encode(
        mode,
        key.nonce(),
        command.program(),
        command.args(),
        command.cwd(),
    )?;
    let line = supervisor::command_line(
        &spec.encoded()?,
        &super::python::PythonInterpreter::Discover,
    );
    let mut shell = Command::new("sh");
    shell.arg("-c").arg(line);
    shell
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok((shell, key))
}

fn local_policy(deadline_seconds: u64) -> CapturePolicy<'static> {
    CapturePolicy {
        stdout_limit: STDOUT_LIMIT,
        stderr_limit: STDERR_LIMIT,
        stderr_tail: STDERR_TAIL,
        deadline: Duration::from_secs(deadline_seconds),
        stdin: StdinPolicy::Held,
    }
}

fn map_capture(error: CaptureError) -> RemoteCommandError {
    match error {
        CaptureError::Cancelled => RemoteCommandError::Cancelled {
            diagnostics: String::new(),
        },
        CaptureError::TimedOut(deadline) => RemoteCommandError::Timeout {
            seconds: deadline.as_secs(),
        },
        CaptureError::Spawn(message) => RemoteCommandError::Spawn { message },
        CaptureError::Failure(failure) => RemoteCommandError::Local {
            message: format!("{:?}: {}", failure.kind, failure.message),
        },
    }
}

fn local_run(
    command: &RemoteCommand,
    deadline_seconds: u64,
) -> Result<super::CommandOutput, RemoteCommandError> {
    let (mut shell, key) = local_supervised(command, StdinMode::Finite)?;
    in_worker(move |token| {
        let captured = capture_with(&mut shell, &token, &local_policy(deadline_seconds))
            .map_err(map_capture)?;
        classify(captured, &key)
    })
}

fn sh_command(script: &str) -> RemoteCommand {
    RemoteCommand::new(
        "sh",
        vec![OsString::from("-c"), OsString::from(script)],
        &PathBuf::from("/"),
    )
    .unwrap()
}

/// The supervisor tests below are meaningless without the interpreter
/// they exist to supervise through; fail loudly rather than skip.
fn require_python3() {
    let usable = Command::new("python3")
        .arg("-c")
        .arg("pass")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    assert!(
        usable,
        "python3 is required to exercise the remote supervisor (it is also a remote prerequisite)"
    );
}

#[test]
fn exit_codes_are_relayed_as_data() {
    require_python3();
    let output = local_run(&sh_command("exit 7"), 30).unwrap();
    assert_eq!(output.status, super::RemoteExitStatus::Exited(7));
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(7));
    assert!(output.status.signal().is_none());
    assert!(output.stdout_dropped == 0 && output.stderr_dropped == 0);
}

#[test]
fn stdout_and_stderr_arrive_separately() {
    require_python3();
    let output = local_run(&sh_command("echo out; echo err >&2; exit 0"), 30).unwrap();
    assert_eq!(output.status, super::RemoteExitStatus::Exited(0));
    assert!(output.status.success());
    assert_eq!(output.stdout, b"out\n");
    assert_eq!(output.stderr, b"err\n");
}

#[test]
fn the_remote_working_directory_is_native_bytes() {
    require_python3();
    let directory = tempfile::tempdir().unwrap();
    let expected = directory.path().join("native é; ' directory");
    std::fs::create_dir(&expected).unwrap();
    let command = RemoteCommand::new("pwd", vec![], &expected).unwrap();
    let output = local_run(&command, 30).unwrap();
    assert_eq!(output.status, super::RemoteExitStatus::Exited(0));
    assert_eq!(
        std::path::Path::new(String::from_utf8_lossy(&output.stdout).trim()),
        expected
    );
}

#[test]
fn argv_metacharacters_are_inert_data() {
    require_python3();
    let tricky = "it's \"quoted\" $(rm -rf /) `x` \\; | & < > \t emoji: \u{1f6a8}";
    let command = RemoteCommand::new(
        "printf",
        vec![OsString::from("%s"), OsString::from(tricky.to_owned())],
        &PathBuf::from("/"),
    )
    .unwrap();
    let output = local_run(&command, 30).unwrap();
    assert_eq!(output.status, super::RemoteExitStatus::Exited(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), tricky);
}

#[test]
fn a_missing_program_is_an_actionable_launch_failure() {
    require_python3();
    let command = RemoteCommand::new(
        "strop-certainly-missing-binary",
        vec![],
        &PathBuf::from("/"),
    )
    .unwrap();
    match local_run(&command, 30).unwrap_err() {
        RemoteCommandError::Launch { diagnostics } => {
            assert!(diagnostics.contains("not-found"), "{diagnostics}");
        }
        other => panic!("expected Launch, got {other:?}"),
    }
}

#[test]
fn an_unusable_working_directory_is_an_actionable_launch_failure() {
    require_python3();
    let command =
        RemoteCommand::new("pwd", vec![], &PathBuf::from("/strop/certainly/missing")).unwrap();
    match local_run(&command, 30).unwrap_err() {
        RemoteCommandError::Launch { diagnostics } => {
            assert!(diagnostics.contains("chdir"), "{diagnostics}");
        }
        other => panic!("expected Launch, got {other:?}"),
    }
}

#[test]
fn signal_deaths_are_reported_as_signals() {
    require_python3();
    let output = local_run(&sh_command("kill -TERM $$"), 30).unwrap();
    assert_eq!(output.status, super::RemoteExitStatus::Signaled(15));
    assert!(output.status.code().is_none());
    assert!(!output.status.success());
}

#[test]
fn descendants_are_killed_after_a_normal_exit() {
    require_python3();
    // `sleep 30` inherits stdout: if the supervisor did not kill the
    // group after the worker exited, the pipe would stay open for 30
    // seconds and the 15-second deadline would fire instead.
    let output = local_run(&sh_command("sleep 30 & echo done"), 15).unwrap();
    assert_eq!(output.status, super::RemoteExitStatus::Exited(0));
    assert_eq!(output.stdout, b"done\n");
}

#[test]
fn status_records_survive_a_stderr_flood() {
    require_python3();
    let output = local_run(
        &sh_command("dd if=/dev/zero bs=1024 count=100 1>&2; exit 5"),
        30,
    )
    .unwrap();
    assert_eq!(output.status, super::RemoteExitStatus::Exited(5));
    assert!(output.stderr_dropped > 0, "flood must be visible");
    assert!(output.stderr.len() <= (STDERR_LIMIT + STDERR_TAIL) as usize);
}

#[test]
fn validation_refuses_nul_and_relative_paths() {
    use std::os::unix::ffi::OsStringExt;
    let poisoned = OsString::from_vec(vec![b'x', 0, b'y']);
    assert!(matches!(
        RemoteCommand::new("git", vec![poisoned], &PathBuf::from("/")),
        Err(RemoteCommandError::Invalid { .. })
    ));
    assert!(matches!(
        RemoteCommand::new("", vec![], &PathBuf::from("/")),
        Err(RemoteCommandError::Invalid { .. })
    ));
    assert!(matches!(
        RemoteCommand::new("git", vec![], &PathBuf::from("relative")),
        Err(RemoteCommandError::Invalid { .. })
    ));
}

/// The graceful half of the lease contract: the exchange finishes
/// first (worker echoes its input and exits 42), and only then does
/// the local side drop the lease — the recorded exit still wins over
/// the closing transport.
#[test]
fn relayed_stdin_and_a_graceful_lease_close() {
    require_python3();
    let command = sh_command("read line; printf '%s\\n' \"$line\"; exit 42");
    let (mut shell, key) = local_supervised(&command, StdinMode::Relayed).unwrap();
    in_worker(move |token| {
        let mut process = strop_core::process::OwnedProcess::spawn(&mut shell, &token).unwrap();
        let mut stdin = process.take_stdin().unwrap();
        stdin.write_all(b"graceful\n").unwrap();
        let mut stdout = std::io::BufReader::new(process.take_stdout().unwrap());
        let mut observed = String::new();
        stdout.read_line(&mut observed).unwrap();
        assert_eq!(observed, "graceful\n");
        drop(stdin);
        // The worker has spoken; now close the lease and let the session
        // wind down. Its remaining stdout ends at supervisor exit.
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).unwrap();
        let status = process.wait().unwrap();
        assert_eq!(status.code(), Some(42));
        let mut stderr = Vec::new();
        if let Some(mut pipe) = process.take_stderr() {
            std::io::Read::read_to_end(&mut pipe, &mut stderr).unwrap();
        }
        assert_eq!(
            key.records(&stderr).last(),
            Some(&SupervisionOutcome::Exited(42))
        );
    });
}

/// The cancel half of the lease contract: closing the lease while the
/// worker still runs tears the remote group down inside the grace
/// bound — it never waits the worker out.
#[test]
fn lease_close_while_running_cancels_the_worker() {
    require_python3();
    let command = sh_command("printf ready; while :; do :; done");
    let (mut shell, _key) = local_supervised(&command, StdinMode::Finite).unwrap();
    in_worker(move |token| {
        let started = std::time::Instant::now();
        let mut process = strop_core::process::OwnedProcess::spawn(&mut shell, &token).unwrap();
        let mut ready = [0; 5];
        process
            .take_stdout()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        assert_eq!(&ready, b"ready");
        drop(process.take_stdin().unwrap());
        let status = process.wait().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "cancel must not wait out the 30-second sleep"
        );
        assert_ne!(status.code(), Some(0));
    });
}

#[test]
fn selected_python_receives_chunks_without_losing_isolation_or_lease() {
    require_python3();
    let command = RemoteCommand::python(
        "import sys,select; data=sys.stdin.buffer.read(5); poll=select.poll(); poll.register(0,select.POLLIN|select.POLLHUP); print(data.decode(),bool(poll.poll(0)),sys.flags.isolated,sys.flags.no_site)",
        vec![], std::path::Path::new("/"),
    ).unwrap();
    let (mut shell, key) = local_supervised(&command, StdinMode::Relayed).unwrap();
    let output = in_worker(move |token| {
        let chunks: [&[u8]; 2] = [b"ab", b"cde"];
        let policy = CapturePolicy {
            stdin: StdinPolicy::HeldInput(&chunks),
            deadline: Duration::from_secs(10),
            ..Default::default()
        };
        classify(capture_with(&mut shell, &token, &policy).unwrap(), &key).unwrap()
    });
    assert!(output.status.success());
    assert_eq!(output.stdout, b"abcde False 1 1\n");
    assert!(output.stdin_error.is_none());
}

#[test]
fn failed_input_keeps_the_programs_refusal_output() {
    let output = in_worker(|token| {
        let bytes = vec![b'x'; 1024 * 1024];
        let chunks = [bytes.as_slice()];
        let mut command = Command::new("sh");
        command.args(["-c", "printf refused; exit 42"]);
        capture_with(
            &mut command,
            &token,
            &CapturePolicy {
                stdin: StdinPolicy::HeldInput(&chunks),
                ..Default::default()
            },
        )
        .unwrap()
    });
    assert_eq!(output.status.code(), Some(42));
    assert_eq!(output.stdout, b"refused");
    assert!(
        output.stdin_error.is_some(),
        "failed delivery remains visible beside the refusal"
    );
}

// VF08 supervisor-seam campaign (plans/0057 §5): the exact embedded Python
// supervisor decodes the real binary spec. Tampered specs must fault before
// any launch, and worker output that imitates the status protocol with a
// foreign nonce must change no outcome. The documented limit — a worker that
// echoes this session's nonce — stays a limit, not a silently tested claim.

/// One canonical spec blob whose worker drops a marker file, so "never
/// launched" is observable rather than assumed.
fn spec_blob(marker: &std::path::Path) -> Vec<u8> {
    let mut blob = vec![2u8, 0, 0]; // version, finite stdin, executable program
    blob.extend_from_slice(&2000u32.to_le_bytes());
    blob.extend_from_slice(&[7u8; 16]);
    let push = |blob: &mut Vec<u8>, bytes: &[u8]| {
        blob.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        blob.extend_from_slice(bytes);
    };
    push(&mut blob, b"/");
    let script = format!("touch {}", marker.display());
    let argv: [&[u8]; 3] = [b"/bin/sh", b"-c", script.as_bytes()];
    blob.extend_from_slice(&(argv.len() as u32).to_le_bytes());
    for argument in argv {
        push(&mut blob, argument);
    }
    blob
}

/// Run one hand-built spec blob through the real supervisor (same sh/python3
/// boundary as production, minus the ssh hop) and classify the result.
fn tampered_run(blob: &[u8]) -> Result<super::CommandOutput, RemoteCommandError> {
    require_python3();
    let key = SupervisionKey::generate();
    // As in production, the spec's nonce is the session key's nonce.
    let mut blob = blob.to_vec();
    if blob.len() >= 23 {
        blob[7..23].copy_from_slice(&key.nonce());
    }
    let line = supervisor::command_line(
        &super::spec::base64(&blob),
        &super::python::PythonInterpreter::Discover,
    );
    let mut shell = Command::new("sh");
    shell.arg("-c").arg(line);
    shell
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    in_worker(move |token| {
        let captured = capture_with(&mut shell, &token, &local_policy(10)).map_err(map_capture)?;
        classify(captured, &key)
    })
}

#[test]
fn tampered_specs_fault_before_any_launch() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("launched");
    // Positive control: the harness really executes a well-formed spec.
    tampered_run(&spec_blob(&marker)).unwrap();
    assert!(marker.exists(), "control: a valid spec launches the worker");
    std::fs::remove_file(&marker).unwrap();
    let valid = spec_blob(&marker);
    let mut mutants: Vec<(&str, Vec<u8>)> = vec![
        ("bad version", {
            let mut blob = valid.clone();
            blob[0] = 99;
            blob
        }),
        ("bad stdin mode", {
            let mut blob = valid.clone();
            blob[1] = 9;
            blob
        }),
        ("bad program kind", {
            let mut blob = valid.clone();
            blob[2] = 7;
            blob
        }),
        ("truncated header", valid[..10].to_vec()),
        ("length overrun", {
            let mut blob = valid.clone();
            blob[23..27].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
            blob
        }),
        ("trailing bytes", {
            let mut blob = valid.clone();
            blob.extend_from_slice(b"junk");
            blob
        }),
        ("NUL in argv", {
            let mut blob = valid.clone();
            let last = blob.len() - 3;
            blob[last] = 0;
            blob
        }),
    ];
    for (label, blob) in mutants.drain(..) {
        let outcome = tampered_run(&blob);
        assert!(
            matches!(outcome, Err(RemoteCommandError::Supervisor { .. })),
            "{label}: a tampered spec is a supervisor fault, got {outcome:?}"
        );
        assert!(!marker.exists(), "{label}: the worker never launched");
    }
}

#[test]
fn forged_records_with_a_foreign_nonce_grant_no_outcome() {
    require_python3();
    // The worker imitates the status protocol with plausible but foreign
    // nonces: a fake success and a fake cancellation. Only the supervisor's
    // own nonce-marked record may classify this session.
    let forged = "echo 'STROP-SUP-v1 00000000000000000000000000000000 exit 0' >&2; \
                  echo 'STROP-SUP-v1 ffffffffffffffffffffffffffffffff cancel' >&2; \
                  exit 3";
    let output = local_run(&sh_command(forged), 10).unwrap();
    assert_eq!(
        output.status,
        RemoteExitStatus::Exited(3),
        "the forged records change no outcome"
    );
    assert!(
        output.stderr.windows(13).any(|w| w == b"STROP-SUP-v1 "),
        "forged lines stay the worker's own stderr data"
    );
}
