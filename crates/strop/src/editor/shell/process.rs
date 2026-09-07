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
    use std::io::{self, Read, Write};
    use std::process::{Command, ExitStatus, Stdio};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::time::Duration;
    use strop_core::process::OwnedProcess;
    use strop_core::worker::{self, CancelReason, Failure};

    const POLL: Duration = Duration::from_millis(20);
    const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

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
        command_text: &str,
        cwd: &std::path::Path,
        input: Option<String>,
        token: &CancelToken,
    ) -> Outcome<ProcessOutput> {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(command_text)
            .current_dir(cwd)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process = match OwnedProcess::spawn(&mut command, token) {
            Ok(process) => process,
            Err(failure) => {
                return Outcome::Failed {
                    failure,
                    partial: None,
                }
            }
        };
        if token.is_cancelled() {
            if let Err(failure) = process.terminate() {
                return Outcome::Failed {
                    failure,
                    partial: None,
                };
            }
            return Outcome::Cancelled(CancelReason::OwnerClosed);
        }
        let Some(stdout) = process.take_stdout() else {
            return Outcome::failed(FailureKind::Protocol, "missing shell stdout");
        };
        let Some(stderr) = process.take_stderr() else {
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
            let Some(mut stdin) = process.take_stdin() else {
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
                match process.has_exited() {
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
                if let Err(failure) = process.terminate() {
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
                        match process.has_exited() {
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
        if let Err(failure) = process.terminate() {
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
