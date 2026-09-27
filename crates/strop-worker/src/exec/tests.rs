//! Seam-level lifecycle tests for the native supervisor: spec admission,
//! launch classification, the kill/leak/reap races and the status-record
//! codec. Real processes, hermetic tempdirs, no network.

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

/// Read until `needle` appears in the PTY master's output (deadline-
/// bounded, so a broken launch fails the test instead of hanging).
fn pty_read_until(master: &std::fs::File, needle: &str) -> String {
    use std::os::fd::AsRawFd;
    let mut collected = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let mut descriptors = [libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: the test owns the live master and the pollfd storage.
        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 1, 100) };
        if ready > 0 {
            let mut chunk = [0_u8; 4096];
            match (&*master).read(&mut chunk) {
                Ok(count) => collected.extend_from_slice(&chunk[..count]),
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("pty read failed: {error}"),
            }
            if String::from_utf8_lossy(&collected).contains(needle) {
                return String::from_utf8_lossy(&collected).into_owned();
            }
        }
    }
    panic!(
        "pty output never contained {needle:?}: {:?}",
        String::from_utf8_lossy(&collected)
    );
}

#[test]
fn pty_launch_owns_a_controlling_terminal_and_classifies_exit() {
    with_token(|token| {
        let size = PtySize {
            columns: 37,
            rows: 9,
        };
        let pty = launch_pty(
            &sh("[ -t 0 ] && [ -t 1 ] || exit 90; stty size; read value; printf 'RESULT:%s\\n' \"$value\"; exit 7"),
            size,
            &token,
        )
        .unwrap();
        let master = pty.master().try_clone().unwrap();
        // The admitted geometry is the terminal's winsize.
        assert!(pty_read_until(&master, "9 37").contains("9 37"));
        pty.master().write_all(b"hello\n").unwrap();
        assert!(pty_read_until(&master, "RESULT:hello").contains("RESULT:hello"));
        let running = pty.into_running();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(7))
        );
    });
}

#[test]
fn pty_launch_failures_are_typed_not_disguised_as_exits() {
    with_token(|token| {
        let missing = ExecSpec::new(
            OsString::from("sh").as_os_str(),
            vec![OsString::from("-c"), OsString::from("exit 0")],
            Path::new("/definitely/missing"),
        )
        .unwrap();
        let error = launch_pty(
            &missing,
            PtySize {
                columns: 80,
                rows: 24,
            },
            &token,
        )
        .unwrap_err();
        assert!(
            matches!(error, ExecError::Launch { ref diagnostics } if diagnostics.contains("chdir")),
            "{error}"
        );
    });
}

#[test]
fn pty_resize_applies_the_new_winsize_in_order() {
    with_token(|token| {
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("gate");
        // SAFETY: creating a FIFO at a fresh tempdir path cannot clash.
        let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let script = format!(
            "stty size; cat \"{}\" >/dev/null; stty size",
            fifo.display()
        );
        let size = PtySize {
            columns: 80,
            rows: 24,
        };
        let pty = launch_pty(
            &ExecSpec::new(
                OsString::from("sh").as_os_str(),
                vec![OsString::from("-c"), OsString::from(script)],
                directory.path(),
            )
            .unwrap(),
            size,
            &token,
        )
        .unwrap();
        let master = pty.master().try_clone().unwrap();
        assert!(pty_read_until(&master, "24 80").contains("24 80"));
        resize_pty(
            pty.master(),
            PtySize {
                columns: 37,
                rows: 9,
            },
        )
        .unwrap();
        // Open after the child is known to be inside `cat`: open blocks
        // until both ends arrive, which is the ordering handshake.
        std::fs::write(&fifo, b"go").unwrap();
        assert!(pty_read_until(&master, "9 37").contains("9 37"));
        let running = pty.into_running();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(0))
        );
    });
}

#[test]
fn pty_revoke_tears_down_the_whole_session() {
    with_token(|token| {
        let pty = launch_pty(
            &sh("(trap '' HUP TERM; printf 'BG:%s\\n' $$; exec sleep 600) & read value; exit 0"),
            PtySize {
                columns: 80,
                rows: 24,
            },
            &token,
        )
        .unwrap();
        let master = pty.master().try_clone().unwrap();
        let output = pty_read_until(&master, "BG:");
        let running = pty.into_running();
        assert_eq!(running.revoke().unwrap(), Settlement::Revoked);
        // The group teardown kills the trapped background member too.
        let child: u32 = output
            .split("BG:")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(pid_gone(child), "background member survived revocation");
    });
}

// ---- environment overlays (0058 WK10) --------------------------------------

/// The admitted overlay reaches the child byte-exactly, on top of the
/// inherited environment, and never leaks into another exec.
#[test]
fn env_overlay_applies_and_stays_scoped() {
    with_token(|token| {
        let spec = sh(r#"printf %s "$STROP_WK10"; printf :; test -n "$PATH" && printf inherited"#)
            .with_env_overlay(vec![(b"STROP_WK10".to_vec(), b"present".to_vec())])
            .unwrap();
        let mut running = launch(&spec, &token).unwrap();
        let mut stdout = running.take_stdout().unwrap();
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(0))
        );
        assert_eq!(output, b"present:inherited");

        // A second exec without the overlay never sees the variable.
        let spec = sh(r#"printf %s "${STROP_WK10-unset}""#);
        let mut running = launch(&spec, &token).unwrap();
        let mut stdout = running.take_stdout().unwrap();
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(0))
        );
        assert_eq!(output, b"unset");
    });
}

/// Non-UTF8 values cross the overlay unchanged.
#[test]
fn env_overlay_keeps_native_bytes() {
    with_token(|token| {
        let spec = sh(r#"printf %s "$STROP_WK10" | od -An -tx1 | tr -d ' \n'"#)
            .with_env_overlay(vec![
                (b"STROP_WK10".to_vec(), vec![0x66, 0x80, 0x67]),
                (b"STROP_EMPTY".to_vec(), Vec::new()),
            ])
            .unwrap();
        let mut running = launch(&spec, &token).unwrap();
        let mut stdout = running.take_stdout().unwrap();
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        assert_eq!(
            running.wait().unwrap(),
            Settlement::Recorded(StatusRecord::Exited(0))
        );
        assert_eq!(output, b"668067");
    });
}

/// Malformed and oversized overlays are typed admission refusals.
#[test]
fn env_overlay_refusals_are_typed() {
    let invalid = |vars: Vec<(Vec<u8>, Vec<u8>)>| {
        sh("true")
            .with_env_overlay(vars)
            .expect_err("overlay must be refused")
    };
    assert!(matches!(
        invalid(vec![(Vec::new(), b"v".to_vec())]),
        ExecError::Invalid { .. }
    ));
    assert!(matches!(
        invalid(vec![(b"NAME=VALUE".to_vec(), b"v".to_vec())]),
        ExecError::Invalid { .. }
    ));
    assert!(matches!(
        invalid(vec![(b"NAME".to_vec(), vec![0])]),
        ExecError::Invalid { .. }
    ));
    assert!(matches!(
        invalid(vec![(b"NAME".to_vec(), vec![b'x'; 128 * 1024])]),
        ExecError::Invalid { .. }
    ));
    let too_many: Vec<(Vec<u8>, Vec<u8>)> = (0..257)
        .map(|index| (format!("STROP_N{index}").into_bytes(), b"v".to_vec()))
        .collect();
    assert!(matches!(invalid(too_many), ExecError::Invalid { .. }));
}
