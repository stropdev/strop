//! Concurrent bounded shell I/O with synchronous cancellation signalling.
//! Unix children lead a private process group. The leader is never reaped
//! while a group-signalling capability exists, even when descendants keep
//! its pipes open. Cancellation never waits for a process or joins a thread.

use super::jobs::ProcessOutput;
use strop_core::worker::{CancelToken, FailureKind, Outcome};

#[cfg(not(unix))]
pub(super) fn run_shell(
    _command: &str,
    _cwd: &std::path::Path,
    _input: Option<String>,
    _token: &CancelToken,
) -> Outcome<ProcessOutput> {
    Outcome::failed(
        FailureKind::Unavailable,
        "shell process-group supervision requires Unix",
    )
}

#[cfg(unix)]
pub(super) fn run_shell(
    command: &str,
    cwd: &std::path::Path,
    input: Option<String>,
    token: &CancelToken,
) -> Outcome<ProcessOutput> {
    unix::run(command, cwd, input, token)
}

#[cfg(unix)]
mod unix {
    use super::*;
    use parking_lot::Mutex;
    use std::io::{self, Read, Write};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::{
        mpsc::{channel, RecvTimeoutError},
        Arc,
    };
    use std::time::Duration;
    use strop_core::worker::{self, CancelReason, Failure};

    const POLL: Duration = Duration::from_millis(20);
    const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

    #[derive(Default)]
    struct Group {
        pid: Option<libc::pid_t>,
        cancelled: bool,
    }
    impl Group {
        // Called only under the capability mutex. The child owner cannot reap
        // until it has removed pid under this same mutex.
        fn signal(&self) -> Result<(), Failure> {
            if let Some(pid) = self.pid {
                // SAFETY: pid is a positive, owned, unreaped child PID whose
                // group was created by process_group(0). Negation targets only
                // that group. std has no process-group signalling operation.
                if unsafe { libc::kill(-pid, libc::SIGKILL) } == -1 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(Failure::new(
                            FailureKind::Io,
                            format!("kill shell group: {error}"),
                        ));
                    }
                }
            }
            Ok(())
        }
        fn cancel(&mut self) -> Result<(), Failure> {
            self.cancelled = true;
            self.signal()
        }
    }

    struct OwnedChild {
        child: Child,
        group: Arc<Mutex<Group>>,
        reaped: bool,
    }
    impl OwnedChild {
        fn kill(&mut self) -> Result<(), Failure> {
            self.group.lock().signal()
        }

        fn exited(&self) -> Result<bool, Failure> {
            // SAFETY: zero is a valid initial siginfo_t representation; waitid
            // writes it before access. P_PID selects our child, WNOWAIT retains
            // the zombie/PID reservation, and WNOHANG never blocks. Unlike
            // Child::try_wait, this does NOT reap. Linux and macOS document
            // si_pid == 0 when no requested state change is available.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            loop {
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        self.child.id() as libc::id_t,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                if result == 0 {
                    // SAFETY: successful waitid initialized the siginfo layout.
                    return Ok(unsafe { info.si_pid() } != 0);
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(Failure::new(
                        FailureKind::Wait,
                        format!("observe shell exit: {error}"),
                    ));
                }
            }
        }
        fn wait(&mut self) -> Result<ExitStatus, Failure> {
            // Revoke before reaping. A detached cancellation callback may run
            // afterwards but will see None, never a recycled process-group ID.
            self.group.lock().pid = None;
            loop {
                match self.child.wait() {
                    Ok(status) => {
                        self.reaped = true;
                        return Ok(status);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(Failure::new(FailureKind::Wait, error.to_string())),
                }
            }
        }
    }
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if !self.reaped {
                let _ = self.kill();
                // Safe direct-child fallback if group signalling failed.
                let _ = self.child.kill();
                let _ = self.wait();
            }
        }
    }
    struct Hook<'a>(&'a CancelToken);
    impl Drop for Hook<'_> {
        fn drop(&mut self) {
            self.0.clear_cancel_resource();
        }
    }

    enum IoDone {
        Input(Outcome<()>),
        Stdout(Outcome<Vec<u8>>),
        Stderr(Outcome<Vec<u8>>),
    }
    #[derive(Default)]
    struct Drain {
        stdout: Option<Vec<u8>>,
        stderr: Option<Vec<u8>>,
        input_done: bool,
        failure: Option<Failure>,
    }
    fn collect<T: Default>(outcome: Outcome<T>, failure: &mut Option<Failure>) -> T {
        match outcome {
            Outcome::Success(value) => value,
            Outcome::Failed {
                failure: error,
                partial,
            } => {
                failure.get_or_insert(error);
                partial.unwrap_or_default()
            }
            Outcome::Cancelled(_) => {
                failure.get_or_insert_with(|| {
                    Failure::new(FailureKind::Disconnected, "shell stream cancelled")
                });
                T::default()
            }
        }
    }
    impl Drain {
        fn step(&mut self, event: IoDone) {
            match event {
                IoDone::Input(result) => {
                    self.input_done = true;
                    collect(result, &mut self.failure);
                }
                IoDone::Stdout(result) => self.stdout = Some(collect(result, &mut self.failure)),
                IoDone::Stderr(result) => self.stderr = Some(collect(result, &mut self.failure)),
            }
        }
        fn settled(&self) -> bool {
            self.input_done && self.stdout.is_some() && self.stderr.is_some()
        }
        fn finish(self, status: Option<ExitStatus>, cancelled: bool) -> Outcome<ProcessOutput> {
            let mut failure = self.failure;
            let mut decode = |bytes: Vec<u8>| match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => {
                    failure.get_or_insert_with(|| {
                        Failure::new(FailureKind::Io, "shell output is not UTF-8")
                    });
                    String::from_utf8_lossy(error.as_bytes()).into_owned()
                }
            };
            let output = ProcessOutput {
                stdout: decode(self.stdout.unwrap_or_default()),
                stderr: decode(self.stderr.unwrap_or_default()),
            };
            if let Some(failure) = failure {
                return Outcome::Failed {
                    failure,
                    partial: Some(output),
                };
            }
            if cancelled {
                return Outcome::Cancelled(CancelReason::OwnerClosed);
            }
            match status {
                Some(status) if status.success() => Outcome::Success(output),
                Some(status) => Outcome::Failed {
                    failure: Failure::new(FailureKind::Exit, format!("shell exited with {status}")),
                    partial: Some(output),
                },
                None => Outcome::Failed {
                    failure: Failure::new(FailureKind::Wait, "shell wait failed"),
                    partial: Some(output),
                },
            }
        }
    }
    fn reader(
        mut pipe: impl Read + Send + 'static,
        emit: impl FnOnce(Outcome<Vec<u8>>) + Send + 'static,
    ) -> worker::CancelHandle {
        worker::spawn("shell-read", emit, move |_| {
            let mut bytes = Vec::new();
            let mut buffer = [0; 16384];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => return Outcome::Success(bytes),
                    Ok(count) => {
                        let keep = count.min(OUTPUT_LIMIT - bytes.len());
                        bytes.extend_from_slice(&buffer[..keep]);
                        if keep != count {
                            return Outcome::Failed {
                                failure: Failure::new(
                                    FailureKind::Io,
                                    "shell output limit exceeded",
                                ),
                                partial: Some(bytes),
                            };
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        return Outcome::Failed {
                            failure: Failure::new(FailureKind::Io, error.to_string()),
                            partial: Some(bytes),
                        }
                    }
                }
            }
        })
    }

    pub(super) fn run(
        command: &str,
        cwd: &std::path::Path,
        input: Option<String>,
        token: &CancelToken,
    ) -> Outcome<ProcessOutput> {
        let group = Arc::new(Mutex::new(Group::default()));
        let callback_group = group.clone();
        if let Err(failure) = token.register_cancel_resource(move || callback_group.lock().cancel())
        {
            return Outcome::Failed {
                failure,
                partial: None,
            };
        }
        // Declared first: child RAII cleanup precedes hook removal on every exit.
        let _hook = Hook(token);
        if token.is_cancelled() {
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        let spawned = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .process_group(0)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let child = match spawned {
            Ok(child) => child,
            Err(error) => {
                return Outcome::failed(FailureKind::Spawn, format!("spawn failed: {error}"))
            }
        };
        let mut process = OwnedChild {
            child,
            group,
            reaped: false,
        };
        let publish = {
            let mut group = process.group.lock();
            group.pid = Some(process.child.id() as libc::pid_t);
            // Covers cancellation before/during spawn and before publication.
            if group.cancelled || token.is_cancelled() {
                group.cancel()
            } else {
                Ok(())
            }
        };
        if let Err(failure) = publish {
            return Outcome::Failed {
                failure,
                partial: None,
            };
        }
        if token.is_cancelled() {
            if let Err(failure) = process.kill() {
                return Outcome::Failed {
                    failure,
                    partial: None,
                };
            }
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        let Some(stdout) = process.child.stdout.take() else {
            return Outcome::failed(FailureKind::Protocol, "missing shell stdout");
        };
        let Some(stderr) = process.child.stderr.take() else {
            return Outcome::failed(FailureKind::Protocol, "missing shell stderr");
        };
        let (tx, rx) = channel();
        let out = reader(stdout, {
            let tx = tx.clone();
            move |r| {
                let _ = tx.send(IoDone::Stdout(r));
            }
        });
        let err = reader(stderr, {
            let tx = tx.clone();
            move |r| {
                let _ = tx.send(IoDone::Stderr(r));
            }
        });
        let writer = if let Some(text) = input {
            let Some(mut stdin) = process.child.stdin.take() else {
                return Outcome::failed(FailureKind::Protocol, "missing shell stdin");
            };
            Some(worker::spawn(
                "shell-write",
                {
                    let tx = tx.clone();
                    move |r| {
                        let _ = tx.send(IoDone::Input(r));
                    }
                },
                move |_| {
                    match stdin.write_all(text.as_bytes()) {
                        Ok(()) => Outcome::Success(()),
                        // Early consumers are judged by the child exit status.
                        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                            Outcome::Success(())
                        }
                        Err(error) => {
                            Outcome::failed(FailureKind::Io, format!("stdin write failed: {error}"))
                        }
                    }
                },
            ))
        } else {
            let _ = tx.send(IoDone::Input(Outcome::Success(())));
            None
        };
        drop(tx);
        let _streams = (out, err, writer);
        let mut drain = Drain::default();
        let mut exited = false;
        let mut signalled = false;
        loop {
            if !exited {
                match process.exited() {
                    Ok(value) => exited = value,
                    Err(failure) => {
                        drain.failure.get_or_insert(failure);
                        break;
                    }
                }
            }
            // A finished leader must not leave background children holding the
            // readers forever. Its zombie still reserves the PGID here.
            if !signalled && (exited || drain.failure.is_some() || token.is_cancelled()) {
                if let Err(failure) = process.kill() {
                    drain.failure.get_or_insert(failure);
                    break;
                }
                signalled = true;
            }
            if exited && drain.settled() {
                break;
            }
            match rx.recv_timeout(POLL) {
                Ok(event) => drain.step(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) if drain.settled() => {
                    // Pipes can close before the process exits. Avoid spinning
                    // on a disconnected receiver; this wait is supervisor-only.
                    if !exited {
                        match process.exited() {
                            Ok(true) => exited = true,
                            Ok(false) => {
                                // Keep cancellation capability until exit is
                                // observed, not until std::wait reaps it.
                                std::thread::park_timeout(POLL);
                                continue;
                            }
                            Err(failure) => {
                                drain.failure.get_or_insert(failure);
                            }
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    drain.failure.get_or_insert_with(|| {
                        Failure::new(FailureKind::Disconnected, "shell streams disconnected")
                    });
                    break;
                }
            }
        }
        if let Err(failure) = process.kill() {
            drain.failure.get_or_insert(failure);
        }
        let status = match process.wait() {
            Ok(status) => Some(status),
            Err(failure) => {
                drain.failure.get_or_insert(failure);
                None
            }
        };
        drain.finish(status, token.is_cancelled())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::path::Path;

        fn run(command: &'static str, input: Option<String>) -> Outcome<ProcessOutput> {
            let (tx, rx) = channel();
            let handle = worker::spawn(
                "shell-test",
                move |r| {
                    tx.send(r).unwrap();
                },
                move |token| super::run(command, Path::new("."), input, &token),
            );
            let result = rx.recv().unwrap();
            drop(handle);
            result
        }
        #[test]
        fn concurrent_io_and_early_broken_pipe() {
            let text = "payload line\n".repeat(65536);
            match run("cat", Some(text.clone())) {
                Outcome::Success(out) => assert_eq!(out.stdout, text),
                other => panic!("{other:?}"),
            }
            match run("head -c 16", Some("A".repeat(1024 * 1024))) {
                Outcome::Success(out) => assert_eq!(out.stdout, "A".repeat(16)),
                other => panic!("{other:?}"),
            }
        }
        #[test]
        fn nonzero_exit_keeps_output_and_output_is_bounded() {
            assert!(matches!(run("printf kept; exit 3", None),
                Outcome::Failed { failure, partial: Some(out) }
                if failure.kind == FailureKind::Exit && out.stdout == "kept"));
            assert!(
                matches!(run("yes", None), Outcome::Failed { failure, partial: Some(out) }
                if failure.kind == FailureKind::Io && out.stdout.len() <= OUTPUT_LIMIT)
            );
        }

        // A test subprocess connects to a Unix socket then blocks on its read
        // side, inheriting sh's stdout/stderr. No timer coordinates readiness.
        #[test]
        fn descendant_helper() {
            let Ok(path) = std::env::var("STROP_PROCESS_TEST_SOCKET") else {
                return;
            };
            let mut socket = UnixStream::connect(path).unwrap();
            socket.write_all(b"R").unwrap();
            let mut byte = [0];
            let _ = socket.read(&mut byte);
        }
        #[test]
        fn cancellation_kills_non_exec_shell_descendant_then_supervisor_progresses() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("process.sock");
            let listener = UnixListener::bind(&path).unwrap();
            fn quote(value: &str) -> String {
                format!("'{}'", value.replace('\'', "'\\''"))
            }
            let exe = std::env::current_exe().unwrap();
            // Background + wait forces sh to remain a separate process.
            let command = format!(
                "STROP_PROCESS_TEST_SOCKET={} {} descendant_helper --nocapture & wait",
                quote(path.to_str().unwrap()),
                quote(exe.to_str().unwrap())
            );
            let (tx, rx) = channel();
            let (done_tx, done_rx) = channel();
            let handle = worker::spawn(
                "shell-cancel-test",
                move |r| {
                    tx.send(r).unwrap();
                },
                move |token| {
                    let result = super::run(&command, Path::new("."), None, &token);
                    done_tx.send(()).unwrap();
                    result
                },
            );
            let (mut socket, _) = listener.accept().unwrap();
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            assert_eq!(byte, *b"R");
            handle.cancel(CancelReason::Dismissed);
            assert!(matches!(
                rx.recv().unwrap(),
                Outcome::Cancelled(CancelReason::Dismissed)
            ));
            assert_eq!(socket.read(&mut byte).unwrap(), 0);
            done_rx.recv().unwrap();
            assert!(rx.recv().is_err());
            std::fs::remove_file(path).unwrap();
        }
    }
}
