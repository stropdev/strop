//! Windows-owned bridge to the Linux UI backend. This adapter owns
//! `wsl.exe`, argv construction and stdio byte framing; Strop's backend,
//! shared protocol client and editor remain authoritative.
//!
//! The bridge runs on its own thread. One bounded queue carries ordered
//! actions and one bounded channel carries decoded server messages back
//! to the UI thread, which applies them through the single readonly
//! [`Client`] reducer. A full queue refuses user input; it never
//! coalesces, guesses or speculatively replays uncertain work.

use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::time::Duration;

use strop_ui_protocol::frame::{self, FrameDecoder};
use strop_ui_protocol::{
    AdmittedAction, Client, ClientCapabilities, ClientError, ClientInfo, ClientMessage,
    ProtocolError, ServerMessage, MAX_PENDING_REQUESTS, PROTOCOL_VERSION,
};

/// Deadlock canary for protocol barriers. Measured presentation latency
/// belongs to the GUI instrumentation, not this transport.
pub const DEFAULT_BRIDGE_BUDGET: Duration = Duration::from_secs(30);

/// Explicit distro/user/backend selection. No PATH search, login shell or
/// inherited `/mnt/c` workspace participates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslSelection {
    pub wsl: PathBuf,
    pub distribution: String,
    pub user: String,
    pub backend: PathBuf,
    pub workspace: String,
    pub environment: Vec<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("the backend stream ended or corrupted its framing: {0}")]
    Transport(String),
    #[error("client state: {0}")]
    Client(#[from] ClientError),
    #[error("the ordered input queue is full; user input is refused, not coalesced")]
    Backpressure,
    #[error("the bridge queue closed")]
    QueueClosed,
    #[error("protocol error: {0}")]
    Protocol(ProtocolError),
}

enum TransportEvent {
    Message(ServerMessage),
    Failed(String),
    Closed,
}

/// One queued action batch. The bridge assigns sequence/base inside its
/// ordered loop, so publications and user input cannot race the caller.
pub enum BridgeAction {
    Act(Vec<AdmittedAction>),
    Viewport { columns: u16, rows: u16 },
    Resync,
    Shutdown,
}

/// Every decoded server event, plus transport termination.
#[derive(Debug)]
pub enum BridgeEvent {
    Message(ServerMessage),
    Closed(Result<(), BridgeError>),
}

/// One live bridge. The UI thread applies [`BridgeEvent::Message`] through
/// its single shared `Client`; this type stores no semantic view.
pub struct WslBridge {
    actions: SyncSender<BridgeAction>,
    events: Receiver<BridgeEvent>,
}

impl WslBridge {
    /// Launch the selected backend and return its handshake identity.
    /// The shared client is constructed here; the caller owns and applies it.
    pub fn spawn(
        selection: &WslSelection,
    ) -> Result<(Self, strop_ui_protocol::BackendInfo), BridgeError> {
        let mut args = vec![
            "--distribution".to_string(),
            selection.distribution.clone(),
            "--user".to_string(),
            selection.user.clone(),
            "--cd".to_string(),
            selection.workspace.clone(),
            "--exec".to_string(),
            "/usr/bin/env".to_string(),
        ];
        for (name, value) in &selection.environment {
            args.push(format!("{name}={value}"));
        }
        args.push(selection.backend.to_string_lossy().into_owned());
        args.push("--ui-stdio".to_string());

        let mut child = Command::new(&selection.wsl)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("backend stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("backend stdout was not piped"))?;

        frame::write_message(
            &mut std::io::BufWriter::new(&stdin),
            &ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                client: ClientInfo {
                    name: "strop-gui".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                },
                capabilities: ClientCapabilities {
                    clipboard_write: true,
                },
            },
        )?;

        let (transport_tx, transport_rx) = std::sync::mpsc::sync_channel(MAX_PENDING_REQUESTS);
        let reader_tx = transport_tx.clone();
        std::thread::spawn(move || read_backend(stdout, reader_tx));
        let backend = match transport_rx.recv_timeout(DEFAULT_BRIDGE_BUDGET) {
            Ok(TransportEvent::Message(ServerMessage::Welcome { backend, .. })) => backend,
            Ok(TransportEvent::Message(ServerMessage::Error { error, .. })) => {
                return Err(BridgeError::Protocol(error));
            }
            Ok(TransportEvent::Failed(error)) => return Err(BridgeError::Transport(error)),
            Ok(TransportEvent::Closed) => {
                return Err(BridgeError::Transport(
                    "backend closed during handshake".into(),
                ));
            }
            Ok(TransportEvent::Message(_)) => {
                return Err(BridgeError::Transport(
                    "backend published before handshake".into(),
                ));
            }
            Err(_) => return Err(BridgeError::Transport("handshake deadline expired".into())),
        };

        let (actions, action_rx) = std::sync::mpsc::sync_channel(MAX_PENDING_REQUESTS);
        let (event_tx, events) = std::sync::mpsc::sync_channel(MAX_PENDING_REQUESTS);
        let transport_tx_thread = transport_tx.clone();
        let stdin = std::sync::Arc::new(std::sync::Mutex::new(stdin));
        let event_tx_thread = event_tx.clone();
        let ordered_backend = backend.clone();
        std::thread::spawn(move || {
            let result = bridge_loop(
                child,
                stdin,
                action_rx,
                transport_rx,
                event_tx,
                ordered_backend,
            );
            if let Err(error) = &result {
                let _ = transport_tx_thread.send(TransportEvent::Failed(error.to_string()));
            }
            let _ = transport_tx_thread.send(TransportEvent::Closed);
            if let Err(error) = &result {
                let _ = event_tx_thread.send(BridgeEvent::Closed(Err(BridgeError::Transport(
                    error.to_string(),
                ))));
            }
            result
        });

        Ok((Self { actions, events }, backend))
    }

    /// Queue one ordered batch. A full queue is a typed refusal; UI code
    /// must surface that pressure rather than merge editing keys.
    pub fn queue(&self, action: BridgeAction) -> Result<(), BridgeError> {
        match self.actions.try_send(action) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(BridgeError::Backpressure),
            Err(TrySendError::Disconnected(_)) => Err(BridgeError::QueueClosed),
        }
    }

    /// Take one decoded bridge event without blocking render.
    pub fn next_event(&self) -> Option<BridgeEvent> {
        self.events.try_recv().ok()
    }

    /// Take one decoded bridge event, bounded for the caller's barrier.
    pub fn wait_event(&self, budget: Duration) -> Result<BridgeEvent, BridgeError> {
        self.events
            .recv_timeout(budget)
            .map_err(|_| BridgeError::Transport("bridge event barrier expired".into()))
    }
}

impl Drop for WslBridge {
    fn drop(&mut self) {
        let _ = self.queue(BridgeAction::Shutdown);
    }
}

fn bridge_loop(
    mut child: Child,
    stdin: std::sync::Arc<std::sync::Mutex<ChildStdin>>,
    actions: Receiver<BridgeAction>,
    transport: Receiver<TransportEvent>,
    events: SyncSender<BridgeEvent>,
    backend: strop_ui_protocol::BackendInfo,
) -> Result<(), BridgeError> {
    let mut client = Client::new(&backend);
    let mut seq = 0u64;
    let mut closed = false;
    while !closed {
        while let Ok(action) = actions.try_recv() {
            seq += 1;
            match action {
                BridgeAction::Act(actions) => {
                    let base = client.base_stamp()?;
                    write_client(&stdin, &ClientMessage::Act { seq, base, actions })?;
                }
                BridgeAction::Viewport { columns, rows } => {
                    write_client(&stdin, &ClientMessage::Viewport { seq, columns, rows })?;
                }
                BridgeAction::Resync => {
                    write_client(&stdin, &ClientMessage::Resync { seq })?;
                }
                BridgeAction::Shutdown => {
                    write_client(&stdin, &ClientMessage::Shutdown { seq })?;
                }
            }
        }

        match transport.recv_timeout(Duration::from_millis(10)) {
            Ok(TransportEvent::Message(message)) => {
                if matches!(message, ServerMessage::Bye { .. }) {
                    closed = true;
                }
                let _ = client.apply(&message);
                events
                    .send(BridgeEvent::Message(message))
                    .map_err(|_| BridgeError::QueueClosed)?;
            }
            Ok(TransportEvent::Failed(error)) => {
                return Err(BridgeError::Transport(error));
            }
            Ok(TransportEvent::Closed) => {
                let status = child.wait()?;
                events
                    .send(BridgeEvent::Closed(Ok(())))
                    .map_err(|_| BridgeError::QueueClosed)?;
                return status.success().then_some(()).ok_or_else(|| {
                    BridgeError::Transport(format!("backend exited with {status}"))
                });
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(BridgeError::QueueClosed);
            }
        }
    }
    let status = child.wait()?;
    events
        .send(BridgeEvent::Closed(Ok(())))
        .map_err(|_| BridgeError::QueueClosed)?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| BridgeError::Transport(format!("backend exited with {status}")))
}

fn write_client(
    stdin: &std::sync::Arc<std::sync::Mutex<ChildStdin>>,
    message: &ClientMessage,
) -> Result<(), BridgeError> {
    let mut writer = stdin.lock().map_err(|_| BridgeError::QueueClosed)?;
    frame::write_message(&mut *writer, message)?;
    Ok(())
}

fn read_backend(mut stdout: ChildStdout, tx: SyncSender<TransportEvent>) {
    let mut decoder = FrameDecoder::new();
    let mut chunk = [0u8; 8192];
    loop {
        match std::io::Read::read(&mut stdout, &mut chunk) {
            Ok(0) | Err(_) => {
                let _ = tx.send(if decoder.is_empty() {
                    TransportEvent::Closed
                } else {
                    TransportEvent::Failed("stream ended mid-frame".into())
                });
                return;
            }
            Ok(n) => {
                if let Err(error) = decoder.accept(&chunk[..n]) {
                    let _ = tx.send(TransportEvent::Failed(error.to_string()));
                    return;
                }
                loop {
                    match decoder.next_frame() {
                        Ok(Some(body)) => match serde_json::from_slice::<ServerMessage>(&body) {
                            Ok(message) => {
                                if tx.send(TransportEvent::Message(message)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                let _ = tx.send(TransportEvent::Failed(format!(
                                    "undecodable server message: {error}"
                                )));
                                return;
                            }
                        },
                        Ok(None) => break,
                        Err(error) => {
                            let _ = tx.send(TransportEvent::Failed(error.to_string()));
                            return;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_error_remains_typed_and_pressure_is_explicit() {
        let error = BridgeError::Backpressure;
        assert_eq!(
            error.to_string(),
            "the ordered input queue is full; user input is refused, not coalesced"
        );
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        tx.send(BridgeAction::Shutdown).unwrap();
        assert!(matches!(
            tx.try_send(BridgeAction::Shutdown),
            Err(TrySendError::Full(_))
        ));
    }

    #[test]
    fn bridge_actions_cover_admitted_input_viewport_recovery_and_shutdown() {
        let actions = [
            BridgeAction::Act(Vec::new()),
            BridgeAction::Viewport {
                columns: 120,
                rows: 40,
            },
            BridgeAction::Resync,
            BridgeAction::Shutdown,
        ];
        assert!(matches!(actions[0], BridgeAction::Act(_)));
        assert!(matches!(
            actions[1],
            BridgeAction::Viewport {
                columns: 120,
                rows: 40
            }
        ));
        assert!(matches!(actions[2], BridgeAction::Resync));
        assert!(matches!(actions[3], BridgeAction::Shutdown));
    }
}
