//! Shared harness for the `strop --ui-stdio` closure tests: the hermetic
//! fixture, the driver spawn, the view accessors and the minimal raw
//! client for malformed/partial/synthetic-drop traffic the (correct)
//! Driver never produces.

use std::path::Path;
use std::process::{Child, Command, Stdio};

use strop_ui_protocol::frame::{self, FrameDecoder};
use strop_ui_protocol::{ClientMessage, Driver, ServerMessage};

pub(crate) fn strop() -> &'static str {
    env!("CARGO_BIN_EXE_strop")
}

/// A hermetic backend root: fixture file plus private HOME/XDG.
pub(crate) struct Fixture {
    pub(crate) dir: tempfile::TempDir,
}

pub(crate) fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    std::fs::write(
        dir.path().join("notes.txt"),
        "alpha\nbeta gamma\nalpha beta\n",
    )
    .unwrap();
    Fixture { dir }
}

pub(crate) fn env(dir: &Path) -> Vec<(&'static str, std::ffi::OsString)> {
    vec![
        ("HOME", dir.join("home").into_os_string()),
        ("XDG_CONFIG_HOME", dir.join("config").into_os_string()),
        ("XDG_STATE_HOME", dir.join("state").into_os_string()),
        ("STROP_LOG", std::ffi::OsString::new()),
    ]
}

pub(crate) fn spawn(dir: &Path) -> Driver {
    Driver::spawn(Path::new(strop()), dir, &env(dir))
        .unwrap_or_else(|error| panic!("backend handshake: {error}"))
}

pub(crate) fn state(driver: &Driver) -> &serde_json::Value {
    &driver.client().view().expect("a published view").state
}

pub(crate) fn pane_lines(driver: &Driver) -> Vec<String> {
    let view = driver.client().view().expect("a published view");
    view.panes[view.active_pane].lines.clone()
}

pub(crate) fn ctrl(ch: char) -> strop_core::frontend_input::KeyEvent {
    let mut key =
        strop_core::frontend_input::KeyEvent::press(strop_core::frontend_input::KeyCode::Char(ch));
    key.modifiers.control = true;
    key
}

/// A minimal raw client for malformed/partial traffic: the Driver
/// (correctly) never produces such bytes.
pub(crate) struct Raw {
    child: Child,
    pub(crate) stdin: Option<std::process::ChildStdin>,
    stdout: std::process::ChildStdout,
    decoder: FrameDecoder,
}

impl Raw {
    pub(crate) fn spawn(dir: &Path) -> Self {
        let mut command = Command::new(strop());
        command
            .arg("--ui-stdio")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in env(dir) {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        Self {
            stdin: Some(child.stdin.take().unwrap()),
            stdout: child.stdout.take().unwrap(),
            child,
            decoder: FrameDecoder::new(),
        }
    }

    pub(crate) fn send(&mut self, message: &ClientMessage) {
        frame::write_message(self.stdin.as_mut().unwrap(), message).unwrap();
    }

    pub(crate) fn send_bytes(&mut self, bytes: &[u8]) {
        use std::io::Write;
        self.stdin.as_mut().unwrap().write_all(bytes).unwrap();
        self.stdin.as_mut().unwrap().flush().unwrap();
    }

    /// Bounded read: the budget is a hang canary (the headless
    /// `jobs_budget` precedent) — expiry fails the test, never budgets
    /// legitimate work.
    pub(crate) fn recv(&mut self) -> ServerMessage {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            if let Some(frame) = self.decoder.next_frame().unwrap() {
                return serde_json::from_slice(&frame).unwrap();
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "backend response hang");
            let mut pollfd = libc::pollfd {
                fd: std::os::unix::io::AsRawFd::as_raw_fd(&self.stdout),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pollfd, 1, remaining.as_millis() as libc::c_int) };
            assert!(ready >= 0, "poll failed");
            if ready == 0 {
                panic!("backend response hang after 120s");
            }
            let mut chunk = [0u8; 8192];
            let read = std::io::Read::read(&mut self.stdout, &mut chunk).unwrap();
            if read == 0 {
                panic!("the backend closed without a bye");
            }
            self.decoder.accept(&chunk[..read]).unwrap();
        }
    }

    /// The next control message: asynchronous view publications
    /// (semantic state can move at the same generation) are skipped.
    pub(crate) fn recv_control(&mut self) -> ServerMessage {
        loop {
            match self.recv() {
                ServerMessage::Snapshot { .. } | ServerMessage::Delta { .. } => {}
                control => return control,
            }
        }
    }

    /// Hello → Welcome; returns the backend incarnation.
    pub(crate) fn hello(&mut self) -> u64 {
        self.send(&ClientMessage::Hello {
            protocol: strop_ui_protocol::PROTOCOL_VERSION,
            client: strop_ui_protocol::ClientInfo {
                name: "raw-test".into(),
                version: "0.0".into(),
            },
            capabilities: strop_ui_protocol::ClientCapabilities {
                clipboard_write: true,
            },
        });
        self.recv_welcome()
    }

    pub(crate) fn recv_welcome(&mut self) -> u64 {
        match self.recv() {
            ServerMessage::Welcome { backend, .. } => backend.incarnation,
            other => panic!("expected welcome: {other:?}"),
        }
    }

    pub(crate) fn recv_ack(&mut self, seq: u64) {
        match self.recv_control() {
            ServerMessage::Ack { seq: seen, .. } => assert_eq!(seen, seq),
            other => panic!("expected ack {seq}: {other:?}"),
        }
    }

    pub(crate) fn recv_snapshot(&mut self) -> ServerMessage {
        loop {
            match self.recv() {
                snapshot @ ServerMessage::Snapshot { .. } => return snapshot,
                ServerMessage::Delta { .. } => {}
                other => panic!("expected snapshot: {other:?}"),
            }
        }
    }

    pub(crate) fn wait(&mut self) -> std::process::ExitStatus {
        // Bounded reap: the backend exits right after its bye.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(std::time::Instant::now() < deadline, "backend exit hang");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
