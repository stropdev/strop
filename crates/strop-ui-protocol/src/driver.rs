//! The Rust-first stdio driver (0056 AR10): spawns `strop --ui-stdio`
//! over pipes and exposes deterministic schedules — every operation is a
//! message barrier (acknowledgement, generation, predicate), never an
//! arbitrary sleep. The driver holds no editing model; all view truth
//! lives in [`crate::client::Client`].
//!
//! Determinism contract: a budget is a hang canary, not timing evidence
//! (the headless driver's `jobs_budget` precedent). The happy path
//! returns the moment the barrier lands; expiry is always a failure.

use std::io;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, Instant};

use crate::client::{Client, ClientEvent};
use crate::frame::{self, FrameDecoder};
use crate::message::{
    AckOutcome, AdmittedAction, ClientCapabilities, ClientInfo, ClientMessage, EffectOutcome,
    EffectRequest, ProtocolError, Refusal, ServerMessage, ShutdownReason, ViewSnapshot,
    PROTOCOL_VERSION,
};
use crate::{ClientError, MAX_PENDING_REQUESTS};

/// Default barrier budget: generous like the headless hang canary — the
/// bound exists only to fail deadlocks.
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(120);

/// How the driver failed. Refusals and protocol errors are typed data,
/// never strings parsed out of a stream.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("refused: {0}")]
    Refused(Refusal),
    #[error("protocol error: {0}")]
    Protocol(ProtocolError),
    #[error("the backend closed ({reason:?}) while waiting for {waiting}")]
    BackendClosed {
        reason: Option<ShutdownReason>,
        waiting: &'static str,
    },
    #[error("timed out waiting for {waiting} (last applied generation {last_generation}, state {last_state})")]
    Timeout {
        waiting: &'static str,
        last_generation: u64,
        /// The cached semantic state at expiry — a barrier failure must
        /// show what the backend actually published.
        last_state: String,
    },
    #[error("client state: {0}")]
    Client(#[from] ClientError),
    #[error("the backend sent undecodable bytes: {0}")]
    Transport(String),
}

/// What the backend's reader thread delivered.
enum Transport {
    Message(ServerMessage),
    Failed(String),
    Closed,
}

/// A control message a waiter may be looking for; view/effect traffic
/// is consumed inside the pump.
enum Pumped {
    Ack(u64, AckOutcome),
    Error(ProtocolError),
    Closed(ShutdownReason),
}

/// A live `strop --ui-stdio` backend and the protocol client state.
pub struct Driver {
    child: Child,
    stdin: io::BufWriter<std::process::ChildStdin>,
    rx: Receiver<Transport>,
    stderr: std::sync::Arc<std::sync::Mutex<String>>,
    client: Client,
    seq: u64,
    budget: Duration,
    closed: Option<ShutdownReason>,
    effects: Vec<(u64, EffectRequest)>,
}

impl Driver {
    /// Spawn the backend and complete the handshake barrier: hello →
    /// welcome → the initial snapshot. `env` entries overlay the
    /// inherited environment (tests pin HOME/XDG to a tempdir; the real
    /// $HOME is never consulted).
    pub fn spawn(
        backend: &Path,
        cwd: &Path,
        env: &[(&str, std::ffi::OsString)],
    ) -> Result<Self, DriverError> {
        Self::spawn_args(backend, &["--ui-stdio"], cwd, env)
    }

    /// As [`Driver::spawn`] with explicit argv (unicode/quoting smoke).
    pub fn spawn_args(
        backend: &Path,
        args: &[&str],
        cwd: &Path,
        env: &[(&str, std::ffi::OsString)],
    ) -> Result<Self, DriverError> {
        let mut command = Command::new(backend);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("backend stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("backend stdout was not piped"))?;
        let mut stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("backend stderr was not piped"))?;

        let (tx, rx) = std::sync::mpsc::sync_channel(MAX_PENDING_REQUESTS);
        std::thread::spawn(move || read_backend(stdout, tx));
        let stderr = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        std::thread::spawn({
            let captured = stderr.clone();
            move || {
                // Bounded diagnostics capture: 64 KiB is ample for a
                // failure report; the bound keeps a noisy backend from
                // growing memory without limit.
                let mut chunk = [0u8; 4096];
                loop {
                    match io::Read::read(&mut stderr_pipe, &mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if let Ok(mut captured) = captured.lock() {
                                const CAP: usize = 64 * 1024;
                                let room = CAP.saturating_sub(captured.len());
                                let take = n.min(room);
                                captured.push_str(&String::from_utf8_lossy(&chunk[..take]));
                            }
                        }
                    }
                }
            }
        });

        // The client state needs the incarnation, known after Welcome.
        let mut driver = Self {
            child,
            stdin: io::BufWriter::new(stdin),
            rx,
            stderr,
            client: Client::new(&crate::message::BackendInfo {
                name: String::new(),
                version: String::new(),
                build: None,
                incarnation: 0,
            }),
            seq: 0,
            budget: DEFAULT_BUDGET,
            closed: None,
            effects: Vec::new(),
        };
        driver.handshake()?;
        Ok(driver)
    }

    fn handshake(&mut self) -> Result<(), DriverError> {
        self.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            client: ClientInfo {
                name: "strop-ui-protocol-driver".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            capabilities: ClientCapabilities {
                clipboard_write: true,
            },
        })?;
        let deadline = self.deadline();
        // Barrier: welcome, then the initial snapshot — the client may
        // not act before its first publication exists.
        while self.client.view().is_none() {
            if let Some(pumped) = self.pump("the handshake", deadline)? {
                match pumped {
                    Pumped::Error(error) => return Err(DriverError::Protocol(error)),
                    Pumped::Closed(reason) => {
                        return Err(DriverError::BackendClosed {
                            reason: Some(reason),
                            waiting: "the handshake",
                        })
                    }
                    Pumped::Ack(..) => {}
                }
            }
        }
        Ok(())
    }

    /// Override the barrier budget (hang canary; not timing evidence).
    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    /// The readonly view cache.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Host effect requests observed so far (answered `Applied`).
    pub fn effects(&self) -> &[(u64, EffectRequest)] {
        &self.effects
    }

    /// Captured bounded stderr diagnostics.
    pub fn stderr(&self) -> String {
        self.stderr
            .lock()
            .map(|captured| captured.clone())
            .unwrap_or_else(|_| String::new())
    }

    /// Admitted actions on the current base; the barrier is the
    /// acknowledgement plus the view reaching the acknowledged
    /// generation. A `stale_generation` refusal means a publication was
    /// missed: resync explicitly, then retry — the driver never replays
    /// optimistically.
    pub fn act(&mut self, actions: Vec<AdmittedAction>) -> Result<u64, DriverError> {
        let base = self.client.base_stamp()?;
        let seq = self.next_seq();
        self.send(&ClientMessage::Act { seq, base, actions })?;
        let deadline = self.deadline();
        let generation = match self.wait_ack(seq, "the action acknowledgement", deadline)? {
            AckOutcome::Applied { generation, .. } => generation,
            AckOutcome::Refused { refusal } => return Err(DriverError::Refused(refusal)),
        };
        self.wait_generation_atleast(generation, "the action's publication", deadline)?;
        Ok(generation)
    }

    /// One scripted key sequence (`ihello<esc>`), decoded by the
    /// engine's own script-token parser — the same tokens the headless
    /// driver's `keys` directive consumes.
    pub fn act_keys(&mut self, keys: &str) -> Result<u64, DriverError> {
        let actions: Vec<AdmittedAction> = strop_engine::editor::keys::parse(keys)
            .map(|key| AdmittedAction::Input(strop_core::frontend_input::Input::Key(key)))
            .collect();
        self.act(actions)
    }

    /// One committed-text input (IME-style), a single admitted action.
    pub fn act_text(&mut self, text: &str) -> Result<u64, DriverError> {
        self.act(vec![AdmittedAction::Input(
            strop_core::frontend_input::Input::Text(text.to_string()),
        )])
    }

    /// One bracketed paste, a single admitted action.
    pub fn act_paste(&mut self, text: &str) -> Result<u64, DriverError> {
        self.act(vec![AdmittedAction::Paste(text.to_string())])
    }

    /// Declare viewport interest; the engine sees the same resize a TUI
    /// would deliver. Barriered like an action.
    pub fn viewport(&mut self, columns: u16, rows: u16) -> Result<u64, DriverError> {
        let seq = self.next_seq();
        self.send(&ClientMessage::Viewport { seq, columns, rows })?;
        let deadline = self.deadline();
        let generation = match self.wait_ack(seq, "the viewport acknowledgement", deadline)? {
            AckOutcome::Applied { generation, .. } => generation,
            AckOutcome::Refused { refusal } => return Err(DriverError::Refused(refusal)),
        };
        self.wait_generation_atleast(generation, "the viewport's publication", deadline)?;
        Ok(generation)
    }

    /// Explicit recovery from a dropped delta or a poisoned client:
    /// request a complete current snapshot and barrier on it.
    pub fn resync(&mut self) -> Result<u64, DriverError> {
        let seq = self.next_seq();
        self.send(&ClientMessage::Resync { seq })?;
        let deadline = self.deadline();
        let generation = match self.wait_ack(seq, "the resync acknowledgement", deadline)? {
            AckOutcome::Applied { generation, .. } => generation,
            AckOutcome::Refused { refusal } => return Err(DriverError::Refused(refusal)),
        };
        loop {
            if self.client.poisoned().is_none() && self.client.generation() >= generation {
                return Ok(self.client.generation());
            }
            if let Some(pumped) = self.pump("the resync snapshot", deadline)? {
                match pumped {
                    Pumped::Error(error) => return Err(DriverError::Protocol(error)),
                    Pumped::Closed(reason) => {
                        return Err(DriverError::BackendClosed {
                            reason: Some(reason),
                            waiting: "the resync snapshot",
                        })
                    }
                    Pumped::Ack(..) => {}
                }
            }
        }
    }

    /// Barrier: the applied view reaches `generation`.
    pub fn wait_generation(&mut self, generation: u64) -> Result<u64, DriverError> {
        let deadline = self.deadline();
        self.wait_generation_atleast(generation, "a view generation", deadline)?;
        Ok(self.client.generation())
    }

    /// Barrier: the cached view satisfies `predicate`. Returns the
    /// matching snapshot's generation.
    pub fn wait_view(
        &mut self,
        waiting: &'static str,
        predicate: impl Fn(&ViewSnapshot) -> bool,
    ) -> Result<u64, DriverError> {
        let deadline = self.deadline();
        loop {
            if let Some(view) = self.client.view() {
                if predicate(view) {
                    return Ok(view.generation);
                }
            }
            if let Some(pumped) = self.pump(waiting, deadline)? {
                match pumped {
                    Pumped::Error(error) => return Err(DriverError::Protocol(error)),
                    Pumped::Closed(reason) => {
                        return Err(DriverError::BackendClosed {
                            reason: Some(reason),
                            waiting,
                        })
                    }
                    Pumped::Ack(..) => {}
                }
            }
        }
    }

    /// Authorized orderly shutdown: `shutdown` → `bye` → backend exit.
    /// Returns the backend's exit status.
    pub fn shutdown(mut self) -> Result<ExitStatus, DriverError> {
        let seq = self.next_seq();
        let deadline = self.deadline();
        self.send(&ClientMessage::Shutdown { seq })?;
        loop {
            match self.pump("bye", deadline)? {
                Some(Pumped::Closed(_)) => break,
                Some(Pumped::Error(error)) => return Err(DriverError::Protocol(error)),
                Some(Pumped::Ack(..)) | None => {}
            }
            if self.closed.is_some() {
                break;
            }
        }
        self.reap(deadline)
    }

    /// The backend's exit status once the link closed on its own
    /// (editor quit, EOF). Barriered like `shutdown`.
    pub fn wait_exit(mut self) -> Result<ExitStatus, DriverError> {
        let deadline = self.deadline();
        while self.closed.is_none() {
            let _ = self.pump("backend exit", deadline)?;
        }
        self.reap(deadline)
    }

    fn reap(&mut self, deadline: Instant) -> Result<ExitStatus, DriverError> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                return Err(self.timeout("backend exit"));
            }
            // Reap poll: process mechanics, not scheduling evidence.
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn timeout(&self, waiting: &'static str) -> DriverError {
        DriverError::Timeout {
            waiting,
            last_generation: self.client.generation(),
            last_state: self
                .client
                .view()
                .map(|view| view.state.to_string())
                .unwrap_or_else(|| "<no view>".into()),
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn deadline(&self) -> Instant {
        Instant::now() + self.budget
    }

    fn send(&mut self, message: &ClientMessage) -> Result<(), DriverError> {
        frame::write_message(&mut self.stdin, message).map_err(DriverError::Io)
    }

    fn wait_ack(
        &mut self,
        seq: u64,
        waiting: &'static str,
        deadline: Instant,
    ) -> Result<AckOutcome, DriverError> {
        loop {
            match self.pump(waiting, deadline)? {
                Some(Pumped::Ack(seen, outcome)) if seen == seq => return Ok(outcome),
                Some(Pumped::Error(error)) => return Err(DriverError::Protocol(error)),
                Some(Pumped::Closed(reason)) => {
                    return Err(DriverError::BackendClosed {
                        reason: Some(reason),
                        waiting,
                    })
                }
                _ => {}
            }
        }
    }

    fn wait_generation_atleast(
        &mut self,
        generation: u64,
        waiting: &'static str,
        deadline: Instant,
    ) -> Result<(), DriverError> {
        while self.client.generation() < generation {
            if let Some(pumped) = self.pump(waiting, deadline)? {
                match pumped {
                    Pumped::Error(error) => return Err(DriverError::Protocol(error)),
                    Pumped::Closed(reason) => {
                        return Err(DriverError::BackendClosed {
                            reason: Some(reason),
                            waiting,
                        })
                    }
                    Pumped::Ack(..) => {}
                }
            }
        }
        Ok(())
    }

    /// Receive one backend delivery within the deadline. View and
    /// effect traffic is applied/answered inside; control messages are
    /// returned for the waiters.
    fn pump(
        &mut self,
        waiting: &'static str,
        deadline: Instant,
    ) -> Result<Option<Pumped>, DriverError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(self.timeout(waiting));
        }
        let transport = match self.rx.recv_timeout(remaining) {
            Ok(transport) => transport,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(self.timeout(waiting)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.client.link_lost();
                return Err(DriverError::BackendClosed {
                    reason: self.closed,
                    waiting,
                });
            }
        };
        let message = match transport {
            Transport::Message(message) => message,
            Transport::Failed(error) => {
                self.client.link_lost();
                return Err(DriverError::Transport(error));
            }
            Transport::Closed => {
                self.client.link_lost();
                return Err(DriverError::BackendClosed {
                    reason: self.closed,
                    waiting,
                });
            }
        };
        if let ServerMessage::Bye { reason } = &message {
            self.closed = Some(*reason);
            // The cache poisons (link lost) AND the waiters observe the
            // close — both, never one without the other.
            self.client.apply(&message);
            return Ok(Some(Pumped::Closed(*reason)));
        }
        if let ServerMessage::Error { error, .. } = &message {
            return Ok(Some(Pumped::Error(error.clone())));
        }
        if let ServerMessage::Ack { seq, outcome } = &message {
            return Ok(Some(Pumped::Ack(*seq, outcome.clone())));
        }
        for event in self.client.apply(&message) {
            if let ClientEvent::Effect { id, effect } = event {
                self.effects.push((id, effect));
                self.send(&ClientMessage::EffectResult {
                    id,
                    outcome: EffectOutcome::Applied,
                })?;
            }
        }
        Ok(None)
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        if self.closed.is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// The backend reader: frames → decoded messages, typed failures, or a
/// clean close. Send failure means the driver went away; exit quietly.
fn read_backend(mut stdout: std::process::ChildStdout, tx: SyncSender<Transport>) {
    let mut decoder = FrameDecoder::new();
    let mut chunk = [0u8; 8192];
    loop {
        match io::Read::read(&mut stdout, &mut chunk) {
            Ok(0) | Err(_) => {
                // A partial frame at EOF means the backend died
                // mid-write — a transport failure, not a clean close.
                let _ = tx.send(if decoder.is_empty() {
                    Transport::Closed
                } else {
                    Transport::Failed("stream ended mid-frame".into())
                });
                return;
            }
            Ok(n) => {
                if let Err(error) = decoder.accept(&chunk[..n]) {
                    let _ = tx.send(Transport::Failed(error.to_string()));
                    return;
                }
                loop {
                    match decoder.next_frame() {
                        Ok(Some(body)) => match serde_json::from_slice::<ServerMessage>(&body) {
                            Ok(message) => {
                                if tx.send(Transport::Message(message)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                let _ = tx.send(Transport::Failed(format!(
                                    "undecodable server message: {error}"
                                )));
                                return;
                            }
                        },
                        Ok(None) => break,
                        Err(error) => {
                            let _ = tx.send(Transport::Failed(error.to_string()));
                            return;
                        }
                    }
                }
            }
        }
    }
}
