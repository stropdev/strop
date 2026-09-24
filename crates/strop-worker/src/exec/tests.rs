//! Seam-level lifecycle tests for the native supervisor: spec admission,
//! launch classification, the kill/leak/reap races and the status-record
//! codec. Real processes, hermetic tempdirs, no network.

use super::record::{mark_line, records};
use super::*;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::time::{Duration, Instant};
use strop_core::worker::CancelToken;

/// A live CancelToken for tests, strop-fs's pattern: a parked worker owns it.
fn with_token<T>(work: impl FnOnce(CancelToken) -> T) -> T {
    let (send, receive) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let owner = strop_core::worker::spawn(
        "exec-test-token",
        |_| {},
        move |token| {
            send.send(token).unwrap();
            let _ = wait.recv();
            strop_core::worker::Outcome::Success(())
        },
    );
    let result = work(receive.recv().unwrap());
    drop(release);
    drop(owner);
    result
}

/// A token that is already cancelled, for the pre-launch refusal.
fn cancelled_token() -> CancelToken {
    let (send, receive) = std::sync::mpsc::channel::<CancelToken>();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let owner = strop_core::worker::spawn(
        "exec-test-cancelled",
        |_| {},
        move |token| {
            send.send(token).unwrap();
            let _ = wait.recv();
            strop_core::worker::Outcome::Success(())
        },
    );
    let token = receive.recv().unwrap();
    owner.cancel(strop_core::worker::CancelReason::Shutdown);
    drop(release);
    token
}

fn sh(script: &str) -> ExecSpec {
    ExecSpec::new(
        OsString::from("sh").as_os_str(),
        vec![OsString::from("-c"), OsString::from(script)],
        Path::new("/"),
    )
    .unwrap()
}

/// A PID is gone: kill(pid, 0) reports ESRCH, or the process is a zombie
/// (state Z in /proc) — killed by the group teardown but awaiting reaping
/// by the namespace init, which is not ours to force (in a container,
/// PID 1 is the shell, which does not reap). Polls briefly so an
/// in-flight teardown can land without a race.
fn pid_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // SAFETY: signal 0 probes existence; it never delivers a signal.
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return true;
        }
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // The state field follows the (possibly space-containing)
            // comm: it is the first token after the final ')'.
            if let Some((_, after)) = stat.rsplit_once(')') {
                if after.split_whitespace().next() == Some("Z") {
                    return true;
                }
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn spec_admission_refuses_unrepresentable_values() {
    let cwd = Path::new("/");
    assert!(matches!(
        ExecSpec::new(OsString::new().as_os_str(), vec![], cwd),
        Err(ExecError::Invalid { .. })
    ));
    assert!(matches!(
        ExecSpec::new(OsString::from("true").as_os_str(), vec![], Path::new("relative/dir")),
        Err(ExecError::Invalid { detail }) if detail.contains("absolute")
    ));
    assert!(matches!(
        ExecSpec::new(
            OsString::from("true").as_os_str(),
            vec![OsString::from("bad\0arg")],
            cwd
        ),
        Err(ExecError::Invalid { detail }) if detail.contains("NUL")
    ));
    let overlong = OsString::from_vec(vec![b'x'; 128 * 1024]);
    assert!(matches!(
        ExecSpec::new(OsString::from("true").as_os_str(), vec![overlong], cwd),
        Err(ExecError::Invalid { detail }) if detail.contains("128 KiB")
    ));
    // The grace clamp matches the remote spec's 600 s cap.
    let spec = sh("true").with_grace(Duration::from_secs(10_000));
    assert_eq!(spec.grace(), Duration::from_secs(600));
}

#[test]
fn byte_exact_argv_and_cwd_survive_exec() {
    with_token(|token| {
        let odd = OsString::from_vec(vec![0xff, b'=', 0x80, b' ', b'x']);
        let dir = tempfile::tempdir().unwrap();
        let spec = ExecSpec::new(
            OsString::from("sh").as_os_str(),
            vec![
                OsString::from("-c"),
                OsString::from(r#"printf %s "$1"; printf :; printf %s "$(pwd -P)""#),
                OsString::from("sh"),
                odd.clone(),
            ],
            dir.path(),
        )
        .unwrap();
        let mut running = launch(&spec, &token).unwrap();
        let mut stdout = running.take_stdout().unwrap();
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        let settlement = running.wait().unwrap();
        assert_eq!(settlement, Settlement::Recorded(StatusRecord::Exited(0)));
        let mut expected = os_bytes(&odd).unwrap();
        expected.push(b':');
        // pwd -P resolves symlinks; compare against the canonical tempdir.
        expected.extend_from_slice(dir.path().canonicalize().unwrap().as_os_str().as_bytes());
        assert_eq!(output, expected);
    });
}

#[test]
fn launch_failures_are_classified_not_disguised_as_exits() {
    with_token(|token| {
        let missing = ExecSpec::new(
            OsString::from("/nonexistent/strop-probe").as_os_str(),
            vec![],
            Path::new("/"),
        )
        .unwrap();
        let Err(ExecError::Launch { diagnostics }) = launch(&missing, &token) else {
            panic!("a missing program must be a launch failure");
        };
        assert!(diagnostics.contains("not-found"), "{diagnostics}");

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("plain-file");
        std::fs::write(&script, "echo hi\n").unwrap();
        let spec = ExecSpec::new(script.as_os_str(), vec![], Path::new("/")).unwrap();
        let Err(ExecError::Launch { diagnostics }) = launch(&spec, &token) else {
            panic!("a non-executable file must be a launch failure");
        };
        assert!(diagnostics.contains("not-executable"), "{diagnostics}");

        let spec = ExecSpec::new(
            OsString::from("true").as_os_str(),
            vec![],
            Path::new("/nonexistent/strop-cwd"),
        )
        .unwrap();
        let Err(ExecError::Launch { diagnostics }) = launch(&spec, &token) else {
            panic!("a missing cwd must be a launch failure");
        };
        assert!(diagnostics.contains("chdir"), "{diagnostics}");
    });
}

#[test]
fn exit_codes_and_signals_are_recorded_data() {
    with_token(|token| {
        let running = launch(&sh("exit 3"), &token).unwrap();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(3))
        );
        let running = launch(&sh("kill -TERM $$"), &token).unwrap();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Signaled(15))
        );
    });
}

#[test]
fn normal_exit_kills_leftover_descendants_before_reaping() {
    with_token(|token| {
        // The grandchild shares the target's process group (no job control in
        // a non-interactive sh); the target exits immediately, leaving it.
        // The group teardown must kill it before the target is reaped.
        let mut running = launch(&sh("sleep 300 & echo $!; exit 0"), &token).unwrap();
        // Read only the PID line: the grandchild inherits stdout, so reading
        // to EOF would block until the leftover dies — exactly what settle
        // must do first. One line is enough.
        let stdout = running.take_stdout().unwrap();
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        let grandchild: u32 = line.trim().parse().unwrap();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(0))
        );
        assert!(
            pid_gone(grandchild),
            "descendant survived the group teardown"
        );
    });
}

#[test]
fn a_recorded_exit_beats_the_cancel_that_raced_it() {
    with_token(|token| {
        let running = launch(&sh("exit 7"), &token).unwrap();
        // Wait until the exit has definitely settled before revoking: the
        // last-chance drain must find the record.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if running.poll_record().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "target never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            running.revoke().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(7))
        );
    });
}

#[test]
fn revoke_escalates_term_grace_kill_and_reaps_last() {
    with_token(|token| {
        let spec =
            sh("trap '' TERM; while :; do sleep 1; done").with_grace(Duration::from_millis(100));
        let running = launch(&spec, &token).unwrap();
        let pid = running.id();
        let started = Instant::now();
        assert_eq!(running.revoke().unwrap(), Settlement::Revoked);
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(100),
            "grace was skipped: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "escalation took too long: {elapsed:?}"
        );
        assert!(pid_gone(pid), "a TERM-immune target survived the KILL");
    });
}

#[test]
fn a_term_cooperative_exit_is_still_revoked_not_recorded() {
    with_token(|token| {
        // The Python supervisor drains before sending TERM and never checks
        // again after it: an exit that lands on the TERM is a revocation,
        // not a recorded exit 42.
        let spec = sh("trap 'exit 42' TERM; while :; do sleep 1; done")
            .with_grace(Duration::from_millis(50));
        let running = launch(&spec, &token).unwrap();
        assert_eq!(running.revoke().unwrap(), Settlement::Revoked);
    });
}

#[test]
fn half_close_is_not_revocation() {
    with_token(|token| {
        let spec = ExecSpec::new(OsString::from("cat").as_os_str(), vec![], Path::new("/"))
            .unwrap()
            .relayed_stdin();
        let mut running = launch(&spec, &token).unwrap();
        let mut stdin = running.take_stdin().unwrap();
        stdin.write_all(b"half-close payload\n").unwrap();
        drop(stdin); // half-close: EOF. The target must finish normally.
        let mut stdout = running.take_stdout().unwrap();
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(0))
        );
        assert_eq!(output, b"half-close payload\n");
    });
}

#[test]
fn the_cancel_latch_revokes_through_the_token() {
    let (send, receive) = std::sync::mpsc::channel();
    let owner = strop_core::worker::spawn(
        "exec-test-latch",
        |_| {},
        move |token| {
            let spec = sh("sleep 300").with_grace(Duration::from_millis(50));
            let running = launch(&spec, &token).unwrap();
            send.send(running.id()).unwrap();
            let outcome = match running.wait() {
                Ok(outcome) => outcome,
                Err(error) => panic!("wait failed: {error}"),
            };
            strop_core::worker::Outcome::Success(outcome)
        },
    );
    let pid = receive.recv().unwrap();
    owner.cancel(strop_core::worker::CancelReason::Shutdown);
    assert!(pid_gone(pid), "the latch never tore the group down");
}

#[test]
fn a_cancelled_token_never_spawns() {
    let token = cancelled_token();
    let Err(ExecError::Cancelled) = launch(&sh("exit 0"), &token) else {
        panic!("a cancelled lease must refuse the launch");
    };
}

#[test]
fn dropping_abandons_the_lease_and_tears_down() {
    with_token(|token| {
        let spec = sh("sleep 300").with_grace(Duration::from_millis(50));
        let running = launch(&spec, &token).unwrap();
        let pid = running.id();
        drop(running);
        assert!(pid_gone(pid), "drop left the target running");
    });
}

#[test]
fn status_records_are_byte_compatible_with_the_anchor() {
    assert_eq!(StatusRecord::Exited(3).encode(), [0, 3, 0, 0, 0]);
    assert_eq!(StatusRecord::Signaled(15).encode(), [1, 15, 0, 0, 0]);
    assert_eq!(StatusRecord::LaunchFailed(2).encode(), [2, 2, 0, 0, 0]);
    for record in [
        StatusRecord::Exited(3),
        StatusRecord::Signaled(15),
        StatusRecord::LaunchFailed(2),
    ] {
        assert_eq!(StatusRecord::decode(&record.encode()), Some(record));
    }
    assert_eq!(StatusRecord::decode(&[0, 3]), None);
    assert_eq!(StatusRecord::decode(&[9, 0, 0, 0, 0]), None);
}

#[test]
fn nonce_marked_lines_parse_only_for_their_session() {
    let nonce = [0xab_u8; 16];
    let other = [0xcd_u8; 16];
    let mut stream = Vec::new();
    stream.extend_from_slice(b"target's own output\n");
    stream.extend_from_slice(&mark_line(&nonce, "exit 3"));
    stream.extend_from_slice(&mark_line(&other, "exit 4"));
    stream.extend_from_slice(b"STROP-SUP-v1 nothex exit 9\n");
    assert_eq!(
        records(&stream, &nonce),
        vec![SupervisionOutcome::Exited(3)]
    );
    // The rendered frame keeps the exact supervisor framing.
    assert_eq!(
        mark_line(&[0_u8; 16], "cancel"),
        b"\nSTROP-SUP-v1 00000000000000000000000000000000 cancel\n".to_vec()
    );
}
