//! Lifecycle tests for the PTY supervisor: launch classification, exit
//! records, the kill/leak/reap races, resize and the environment overlay.
//! Real processes, hermetic tempdirs, no network.
use super::*;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::Instant;
use strop_core::worker::{CancelHandle, CancelReason};

const SIZE: PtySize = PtySize {
    columns: 80,
    rows: 24,
};

fn launch_in(directory: &Path, program: &str, arguments: &[&[u8]]) -> Launch {
    Launch {
        program: program.into(),
        arguments: arguments
            .iter()
            .map(|argument| OsString::from_vec(argument.to_vec()))
            .collect(),
        directory: directory.to_path_buf(),
        environment: Vec::new(),
    }
}

fn sh(script: &str) -> Launch {
    launch_in(Path::new("/"), "sh", &[b"-c", script.as_bytes()])
}

/// Spawn under a fresh standalone token. The returned master stays open
/// for the test's duration — closing it would hang up the session — and
/// the handle cancels the token.
fn start(launch: &Launch, grace: Duration) -> (File, Running, CancelHandle) {
    let (token, handle) = CancelToken::standalone();
    let pty = spawn(launch, SIZE, grace, &token).unwrap();
    let master = pty.master().try_clone().unwrap();
    (master, pty.into_running(), handle)
}

/// A PID is gone: kill(pid, 0) reports ESRCH, or the process is a zombie
/// awaiting a reaper that is not ours (in a container PID 1 may not
/// reap). Polls briefly so an in-flight teardown can land.
fn pid_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // SAFETY: signal 0 probes existence; it never delivers a signal.
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return true;
        }
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // The state is the first token after the (possibly
            // space-containing) comm's final ')'.
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

/// Read the master until `done` accepts the output or the slave side
/// closes (EIO). Deadline-bounded, so a broken launch fails, not hangs.
fn read_master(master: &File, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
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
                Ok(0) => return collected,
                Ok(count) => collected.extend_from_slice(&chunk[..count]),
                Err(error) if error.raw_os_error() == Some(libc::EIO) => return collected,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("pty read failed: {error}"),
            }
            if done(&collected) {
                return collected;
            }
        }
    }
    panic!(
        "pty output never completed: {:?}",
        String::from_utf8_lossy(&collected)
    );
}

fn read_until(master: &File, needle: &str) -> String {
    let output = read_master(master, |bytes| {
        String::from_utf8_lossy(bytes).contains(needle)
    });
    let text = String::from_utf8_lossy(&output).into_owned();
    assert!(text.contains(needle), "{needle:?} never arrived: {text:?}");
    text
}

fn read_to_end(master: &File) -> Vec<u8> {
    read_master(master, |_| false)
}

#[test]
fn launch_failures_are_typed_not_disguised_as_exits() {
    let (token, _handle) = CancelToken::standalone();
    let missing = launch_in(Path::new("/"), "/nonexistent/strop-probe", &[]);
    let Err(PtyError::Launch { diagnostics }) = spawn(&missing, SIZE, DEFAULT_GRACE, &token) else {
        panic!("a missing program must be a launch failure");
    };
    assert!(diagnostics.contains("not-found"), "{diagnostics}");

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("plain-file");
    std::fs::write(&script, "echo hi\n").unwrap();
    let plain = launch_in(Path::new("/"), script.to_str().unwrap(), &[]);
    let Err(PtyError::Launch { diagnostics }) = spawn(&plain, SIZE, DEFAULT_GRACE, &token) else {
        panic!("a non-executable file must be a launch failure");
    };
    assert!(diagnostics.contains("not-executable"), "{diagnostics}");

    let nowhere = launch_in(Path::new("/nonexistent/strop-cwd"), "true", &[]);
    let Err(PtyError::Launch { diagnostics }) = spawn(&nowhere, SIZE, DEFAULT_GRACE, &token) else {
        panic!("a missing cwd must be a launch failure");
    };
    assert!(diagnostics.contains("chdir"), "{diagnostics}");
}

#[test]
fn byte_exact_argv_and_cwd_survive_exec() {
    let odd = vec![0xff, b'=', 0x80, b' ', b'x'];
    let dir = tempfile::tempdir().unwrap();
    let launch = launch_in(
        dir.path(),
        "sh",
        &[
            b"-c",
            br#"printf %s "$1"; printf :; printf %s "$(pwd -P)""#,
            b"sh",
            &odd,
        ],
    );
    let (master, running, _handle) = start(&launch, DEFAULT_GRACE);
    let output = read_to_end(&master);
    assert_eq!(
        running.wait().unwrap(),
        Settlement::Recorded(StatusRecord::Exited(0))
    );
    let mut expected = odd;
    expected.push(b':');
    // pwd -P resolves symlinks; compare against the canonical tempdir.
    expected.extend_from_slice(dir.path().canonicalize().unwrap().as_os_str().as_bytes());
    assert_eq!(output, expected);
}

#[test]
fn exit_codes_and_signals_are_recorded_data() {
    let (_master, running, _handle) = start(&sh("exit 3"), DEFAULT_GRACE);
    assert_eq!(
        running.wait().unwrap(),
        Settlement::Recorded(StatusRecord::Exited(3))
    );
    let (_master, running, _handle) = start(&sh("kill -TERM $$"), DEFAULT_GRACE);
    assert_eq!(
        running.wait().unwrap(),
        Settlement::Recorded(StatusRecord::Signaled(15))
    );
}

#[test]
fn pty_launch_owns_a_controlling_terminal_at_its_geometry() {
    let size = PtySize {
        columns: 37,
        rows: 9,
    };
    let (token, _handle) = CancelToken::standalone();
    let pty = spawn(
        &sh("[ -t 0 ] && [ -t 1 ] || exit 90; stty size; read value; printf 'RESULT:%s\\n' \"$value\"; exit 7"),
        size,
        DEFAULT_GRACE,
        &token,
    )
    .unwrap();
    let master = pty.master().try_clone().unwrap();
    let running = pty.into_running();
    read_until(&master, "9 37");
    (&master).write_all(b"hello\n").unwrap();
    read_until(&master, "RESULT:hello");
    assert_eq!(
        running.wait().unwrap(),
        Settlement::Recorded(StatusRecord::Exited(7))
    );
}

#[test]
fn resize_applies_the_new_winsize_in_order() {
    let directory = tempfile::tempdir().unwrap();
    let fifo = directory.path().join("gate");
    let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: `path` is a valid C string naming a fresh tempdir entry.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let script = format!(
        "stty size; cat \"{}\" >/dev/null; stty size",
        fifo.display()
    );
    let (master, running, _handle) = start(&sh(&script), DEFAULT_GRACE);
    read_until(&master, "24 80");
    resize(
        &master,
        PtySize {
            columns: 37,
            rows: 9,
        },
    )
    .unwrap();
    // Opening blocks until the child is inside `cat`: the ordering handshake.
    std::fs::write(&fifo, b"go").unwrap();
    read_until(&master, "9 37");
    assert_eq!(
        running.wait().unwrap(),
        Settlement::Recorded(StatusRecord::Exited(0))
    );
}

#[test]
fn normal_exit_kills_leftover_descendants_before_reaping() {
    // The grandchild shares the child's process group (no job control in
    // a non-interactive sh); the child exits at once, leaving it behind.
    let (master, running, _handle) = start(
        &sh("(trap '' HUP; exec sleep 300) & echo \"PID:$!\"; exit 0"),
        DEFAULT_GRACE,
    );
    let output = read_until(&master, "\n");
    let grandchild: u32 = output
        .split("PID:")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        running.wait().unwrap(),
        Settlement::Recorded(StatusRecord::Exited(0))
    );
    assert!(
        pid_gone(grandchild),
        "descendant survived the group teardown"
    );
}

#[test]
fn a_recorded_exit_beats_the_cancel_that_raced_it() {
    let (_master, running, _handle) = start(&sh("exit 7"), DEFAULT_GRACE);
    // Wait until the exit has settled before revoking: the last-chance
    // drain must find the record.
    let deadline = Instant::now() + Duration::from_secs(5);
    while running.poll_record().unwrap().is_none() {
        assert!(Instant::now() < deadline, "child never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        running.revoke().unwrap(),
        Settlement::Recorded(StatusRecord::Exited(7))
    );
}

#[test]
fn revoke_escalates_term_grace_kill_and_reaps_last() {
    let (_master, running, _handle) = start(
        &sh("trap '' TERM; while :; do sleep 1; done"),
        Duration::from_millis(100),
    );
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
    assert!(pid_gone(pid), "a TERM-immune child survived the KILL");
}

#[test]
fn a_term_cooperative_exit_is_still_revoked_not_recorded() {
    let (_master, running, _handle) = start(
        &sh("trap 'exit 42' TERM; while :; do sleep 1; done"),
        Duration::from_millis(50),
    );
    assert_eq!(running.revoke().unwrap(), Settlement::Revoked);
}

#[test]
fn revoke_tears_down_the_whole_session() {
    let (master, running, _handle) = start(
        &sh("(trap '' HUP TERM; printf 'BG:%s\\n' $$; exec sleep 600) & read value; exit 0"),
        Duration::from_millis(50),
    );
    let output = read_until(&master, "BG:");
    assert_eq!(running.revoke().unwrap(), Settlement::Revoked);
    let member: u32 = output
        .split("BG:")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .parse()
        .unwrap();
    assert!(pid_gone(member), "background member survived revocation");
}

#[test]
fn cancelling_the_token_revokes_a_waiting_child() {
    let (_master, running, handle) = start(&sh("sleep 300"), Duration::from_millis(50));
    let pid = running.id();
    let waiter = std::thread::spawn(move || running.wait());
    handle.cancel(CancelReason::OwnerClosed);
    assert_eq!(waiter.join().unwrap().unwrap(), Settlement::Revoked);
    assert!(pid_gone(pid), "the latch never tore the group down");
}

#[test]
fn a_cancelled_token_never_spawns() {
    let (token, handle) = CancelToken::standalone();
    handle.cancel(CancelReason::OwnerClosed);
    assert_eq!(
        spawn(&sh("exit 0"), SIZE, DEFAULT_GRACE, &token).err(),
        Some(PtyError::Cancelled)
    );
}

#[test]
fn dropping_the_child_tears_it_down() {
    let (_master, running, _handle) = start(&sh("sleep 300"), Duration::from_millis(50));
    let pid = running.id();
    drop(running);
    assert!(pid_gone(pid), "drop left the child running");
}

/// The overlay reaches the child byte-exactly, on top of the inherited
/// environment, and never leaks into another launch.
#[test]
fn environment_overlay_applies_and_stays_scoped() {
    let mut launch = sh(
        r#"printf %s "$STROP_OVERLAY" | od -An -tx1 | tr -d ' \n'; printf :; test -n "$PATH" && printf inherited"#,
    );
    launch.environment = vec![(
        "STROP_OVERLAY".into(),
        OsString::from_vec(vec![0x66, 0x80, 0x67]),
    )];
    let (master, running, _handle) = start(&launch, DEFAULT_GRACE);
    assert_eq!(read_to_end(&master), b"668067:inherited");
    running.wait().unwrap();

    let (master, running, _handle) =
        start(&sh(r#"printf %s "${STROP_OVERLAY-unset}""#), DEFAULT_GRACE);
    assert_eq!(read_to_end(&master), b"unset");
    running.wait().unwrap();
}

#[test]
fn unrepresentable_launches_are_refused_before_spawning() {
    let (token, _handle) = CancelToken::standalone();
    let empty = launch_in(Path::new("/"), "", &[]);
    assert_eq!(
        spawn(&empty, SIZE, DEFAULT_GRACE, &token).err(),
        Some(PtyError::Invalid("empty program"))
    );
    let mut nul = sh("true");
    nul.directory = PathBuf::from(OsString::from_vec(b"/tmp\0x".to_vec()));
    assert!(matches!(
        spawn(&nul, SIZE, DEFAULT_GRACE, &token),
        Err(PtyError::Invalid(_))
    ));
}
