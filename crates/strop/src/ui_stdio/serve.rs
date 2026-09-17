//! The serving session (0056 AR09 §8): one editor, one framed client,
//! and the publication rules — bounded, ordered, and truthful about
//! every refusal.
//!
//! Loop shape (input → render never awaits inside the engine; the
//! server is the composition root that owns the waiting):
//!
//! ```text
//! client frame ──reader thread──▶ bounded request channel (256, AR06)
//! engine workers ──▶ the shared bounded AppEvent channel
//!        │                            │
//!        └──── server loop: drain engine, handle one request,
//!              prepare the view, publish snapshot/delta ◀────┘
//! ```
//!
//! Publications key on the backend incarnation + view generation; a
//! pane-set change always ships a full snapshot, otherwise a sparse
//! delta against the last published generation. An action based on
//! anything but the newest published generation is refused
//! (`stale_generation`): ordered control, never optimistic application.

use std::io::{self, Read, Write};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use strop_engine::editor::events::{AppEvent, EventReceiver};
use strop_engine::editor::prepare::ViewGeometry;
use strop_engine::editor::trace::drive::Action;
use strop_engine::editor::Editor;
use strop_ui_protocol::frame::{self, FrameDecoder, FrameError};
use strop_ui_protocol::{
    AckOutcome, ActionLimits, AdmittedAction, BackendInfo, BaseStamp, ClientMessage, EffectRequest,
    ProtocolError, Refusal, ServerCapabilities, ServerMessage, ShutdownReason, ViewSnapshot,
    MAX_PENDING_REQUESTS, MAX_VIEWPORT_CELLS, PROTOCOL_VERSION,
};

use super::snapshot;

/// Idle tick: engine completions (terminal output, pickers, LSP) are
/// published within one tick even while the client is quiet. Requests
/// themselves wake the loop instantly — this is a poll cadence for
/// engine-side progress only, never scheduling evidence.
const IDLE_TICK: Duration = Duration::from_millis(25);

/// What the request reader delivered.
enum Request {
    /// A well-framed body; decoding happens on the server thread so a
    /// bad envelope earns an in-band typed error and the stream lives.
    Body(Vec<u8>),
    /// Frame-level corruption: the stream is poisoned.
    Frame(FrameError),
    /// Clean EOF at a frame boundary.
    Eof,
}

/// Whether the loop keeps serving.
enum Flow {
    Open,
    Close,
}

/// One client session over stdio pipes.
pub struct Session {
    incarnation: u64,
    geometry: ViewGeometry,
    published: Option<ViewSnapshot>,
    applied: u64,
    handshaken: bool,
    /// The newest generation this client was told about in an
    /// acknowledgement or full snapshot — the floor for base admission.
    client_known: u64,
    /// An action or engine event landed since the last publication:
    /// semantic state may have moved at the same view generation.
    pending_observations: bool,
    next_effect: u64,
}

impl Session {
    pub fn new() -> Self {
        Self {
            incarnation: incarnation(),
            geometry: ViewGeometry {
                columns: 100,
                rows: 30,
            },
            published: None,
            applied: 0,
            handshaken: false,
            client_known: 0,
            pending_observations: false,
            next_effect: 0,
        }
    }

    /// Serve until orderly shutdown, editor quit, link loss, or a
    /// protocol violation. Returns Err only for I/O failures on the
    /// pipe itself and for an engine settle that outlives its bound.
    pub fn serve<R: Read + Send + 'static, W: Write>(
        mut self,
        editor: &mut Editor,
        events: &EventReceiver,
        reader: R,
        writer: W,
    ) -> io::Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel(MAX_PENDING_REQUESTS);
        std::thread::spawn(move || read_requests(reader, tx));
        let mut out = io::BufWriter::new(writer);
        loop {
            self.drain_events(editor, events)?;
            // A drained completion can close the last document: the quit
            // boundary precedes any publication (preparing a view with
            // no panes is not a state the engine has).
            if editor.should_quit() || !editor.has_documents() {
                self.finish(editor, events)?;
                self.send(
                    &mut out,
                    &ServerMessage::Bye {
                        reason: ShutdownReason::Quit,
                    },
                )?;
                return Ok(());
            }
            self.publish(editor, &mut out)?;
            let request = match rx.recv_timeout(IDLE_TICK) {
                Ok(request) => request,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                // The reader only dies after delivering Eof/Frame; a bare
                // disconnect is defensive — treat it as link loss.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Request::Eof,
            };
            match request {
                Request::Body(body) => match serde_json::from_slice::<ClientMessage>(&body) {
                    Ok(message) => {
                        if let Flow::Close = self.handle(editor, events, message, &mut out)? {
                            return Ok(());
                        }
                    }
                    Err(error) => self.send(
                        &mut out,
                        &ServerMessage::Error {
                            seq: None,
                            error: ProtocolError::Decode {
                                message: error.to_string(),
                            },
                        },
                    )?,
                },
                Request::Frame(error) => {
                    // The next boundary is unknowable: report in-band,
                    // then close. Never resynchronize by guessing.
                    let _ = self.send(
                        &mut out,
                        &ServerMessage::Error {
                            seq: None,
                            error: ProtocolError::Frame {
                                message: error.to_string(),
                            },
                        },
                    );
                    let _ = self.send(
                        &mut out,
                        &ServerMessage::Bye {
                            reason: ShutdownReason::ProtocolViolation,
                        },
                    );
                    return Ok(());
                }
                Request::Eof => {
                    self.finish(editor, events)?;
                    let _ = self.send(
                        &mut out,
                        &ServerMessage::Bye {
                            reason: ShutdownReason::Disconnect,
                        },
                    );
                    return Ok(());
                }
            }
        }
    }

    fn handle<W: Write>(
        &mut self,
        editor: &mut Editor,
        events: &EventReceiver,
        message: ClientMessage,
        out: &mut io::BufWriter<W>,
    ) -> io::Result<Flow> {
        match message {
            ClientMessage::Hello {
                protocol,
                client: _,
                capabilities: _,
            } => self.hello(editor, protocol, out),
            ClientMessage::Act { seq, base, actions } => {
                if !self.handshaken {
                    return self.unexpected(out, "act before hello");
                }
                self.act(editor, events, seq, base, actions, out)?;
                Ok(Flow::Open)
            }
            ClientMessage::Viewport { seq, columns, rows } => {
                if !self.handshaken {
                    return self.unexpected(out, "viewport before hello");
                }
                self.viewport(editor, events, seq, columns, rows, out)?;
                Ok(Flow::Open)
            }
            ClientMessage::Resync { seq } => {
                if !self.handshaken {
                    return self.unexpected(out, "resync before hello");
                }
                self.resync(editor, seq, out)?;
                Ok(Flow::Open)
            }
            ClientMessage::EffectResult { .. } => {
                // Effect outcomes are terminal records (AR08): the engine
                // staged the request fire-and-forget, exactly like the
                // TUI's escape write. Nothing to deliver back.
                Ok(Flow::Open)
            }
            ClientMessage::Shutdown { seq } => {
                if !self.handshaken {
                    return self.unexpected(out, "shutdown before hello");
                }
                self.send(
                    &mut *out,
                    &ServerMessage::Ack {
                        seq,
                        outcome: AckOutcome::Applied {
                            applied: self.applied,
                            generation: self.published.as_ref().map_or(0, |v| v.generation),
                        },
                    },
                )?;
                self.finish(editor, events)?;
                self.send(
                    &mut *out,
                    &ServerMessage::Bye {
                        reason: ShutdownReason::Requested,
                    },
                )?;
                Ok(Flow::Close)
            }
        }
    }

    fn hello<W: Write>(
        &mut self,
        editor: &mut Editor,
        protocol: u32,
        out: &mut io::BufWriter<W>,
    ) -> io::Result<Flow> {
        if self.handshaken {
            return self.unexpected(out, "duplicate hello");
        }
        if protocol != PROTOCOL_VERSION {
            self.send(
                &mut *out,
                &ServerMessage::Error {
                    seq: None,
                    error: ProtocolError::Version {
                        supported: PROTOCOL_VERSION,
                        offered: protocol,
                    },
                },
            )?;
            self.send(
                &mut *out,
                &ServerMessage::Bye {
                    reason: ShutdownReason::ProtocolViolation,
                },
            )?;
            return Ok(Flow::Close);
        }
        self.handshaken = true;
        self.send(
            &mut *out,
            &ServerMessage::Welcome {
                protocol: PROTOCOL_VERSION,
                backend: BackendInfo {
                    name: "strop".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    build: None,
                    incarnation: self.incarnation,
                },
                limits: ActionLimits {
                    max_frame_bytes: frame::MAX_BODY_BYTES,
                    max_pending_requests: MAX_PENDING_REQUESTS,
                    max_viewport_cells: MAX_VIEWPORT_CELLS,
                },
                capabilities: ServerCapabilities {
                    terminals: true,
                    workspace_search: true,
                    filesystem: true,
                    clipboard_write: true,
                },
            },
        )?;
        self.publish(editor, out)?;
        Ok(Flow::Open)
    }

    /// Admitted actions on a checked base, through the SAME admitted
    /// engine path (recorded actions over `AppEvent`) the TUI and the
    /// headless driver use. The acknowledgement follows the engine's
    /// answer, never precedes it.
    fn act<W: Write>(
        &mut self,
        editor: &mut Editor,
        events: &EventReceiver,
        seq: u64,
        base: BaseStamp,
        actions: Vec<AdmittedAction>,
        out: &mut io::BufWriter<W>,
    ) -> io::Result<()> {
        if let Err(refusal) = self.admit(base) {
            return self.send(
                out,
                &ServerMessage::Ack {
                    seq,
                    outcome: AckOutcome::Refused { refusal },
                },
            );
        }
        let mut failure = None;
        for action in actions {
            self.pending_observations = true;
            let tick = editor.tape().sample_tick();
            if let Err(error) = editor.recorded_action(Action::Event(app_event(action)), tick) {
                failure = Some(error);
                break;
            }
            self.drain_events(editor, events)?;
            self.emit_effects(editor, out)?;
        }
        // The quit boundary: an action that closed the last document is
        // acked against the last published generation; the loop's quit
        // check then finishes and says bye. Preparing a view with no
        // panes is not a state the engine has.
        let quitting = editor.should_quit() || !editor.has_documents();
        let generation = if quitting {
            self.published.as_ref().map_or(0, |view| view.generation)
        } else {
            editor.prepare_view(self.geometry).generation
        };
        let outcome = match failure {
            None => {
                self.applied += 1;
                self.client_known = self.client_known.max(generation);
                AckOutcome::Applied {
                    applied: self.applied,
                    generation,
                }
            }
            Some(error) => AckOutcome::Refused {
                refusal: Refusal::Engine {
                    message: error.to_string(),
                },
            },
        };
        self.send(&mut *out, &ServerMessage::Ack { seq, outcome })?;
        if quitting {
            return Ok(());
        }
        self.publish(editor, out)
    }
    /// Base admission (AR09): an action may build on any generation of
    /// THIS incarnation at or above the floor the client provably knows
    /// (the last acked/snapshot generation) — asynchronous publications
    /// (terminal output, picker streaming) must never make ordinary
    /// input racy, exactly as a TUI user's keys are not refused when
    /// output lands mid-keystroke. A base BELOW the floor means the
    /// client ignored information it was given: its cache is
    /// disconnected (a missed delta), so the action is refused and the
    /// client resyncs instead of diverging silently. A base beyond the
    /// current generation is client confusion, refused as well.
    fn admit(&self, base: BaseStamp) -> Result<(), Refusal> {
        if base.incarnation != self.incarnation {
            return Err(Refusal::WrongIncarnation {
                current: self.incarnation,
            });
        }
        let current = self.published.as_ref().map_or(0, |view| view.generation);
        if base.generation > current {
            return Err(Refusal::FutureGeneration { current });
        }
        if base.generation < self.client_known {
            return Err(Refusal::StaleGeneration { current });
        }
        Ok(())
    }

    /// Viewport interest: the geometry preparation runs against,
    /// delivered to the engine as the same resize a TUI sends.
    fn viewport<W: Write>(
        &mut self,
        editor: &mut Editor,
        events: &EventReceiver,
        seq: u64,
        columns: u16,
        rows: u16,
        out: &mut io::BufWriter<W>,
    ) -> io::Result<()> {
        let cells = u32::from(columns) * u32::from(rows);
        let outcome = if columns == 0 || rows == 0 || cells > MAX_VIEWPORT_CELLS {
            AckOutcome::Refused {
                refusal: Refusal::Limit {
                    message: format!(
                        "viewport {columns}x{rows} exceeds the {MAX_VIEWPORT_CELLS}-cell bound"
                    ),
                },
            }
        } else {
            self.geometry = ViewGeometry { columns, rows };
            let tick = editor.tape().sample_tick();
            editor.recorded_action(Action::Event(AppEvent::Resize { columns, rows }), tick)?;
            self.drain_events(editor, events)?;
            self.applied += 1;
            let generation = editor.prepare_view(self.geometry).generation;
            self.client_known = self.client_known.max(generation);
            AckOutcome::Applied {
                applied: self.applied,
                generation,
            }
        };
        self.send(&mut *out, &ServerMessage::Ack { seq, outcome })?;
        self.publish(editor, out)
    }

    /// Explicit resynchronization: the complete current snapshot, the
    /// only recovery from a dropped delta.
    fn resync<W: Write>(
        &mut self,
        editor: &mut Editor,
        seq: u64,
        out: &mut io::BufWriter<W>,
    ) -> io::Result<()> {
        let generation = editor.prepare_view(self.geometry).generation;
        self.client_known = self.client_known.max(generation);
        self.send(
            &mut *out,
            &ServerMessage::Ack {
                seq,
                outcome: AckOutcome::Applied {
                    applied: self.applied,
                    generation,
                },
            },
        )?;
        let view = snapshot::build(editor, generation);
        self.published = Some(view.clone());
        self.send(
            out,
            &ServerMessage::Snapshot {
                incarnation: self.incarnation,
                view,
            },
        )
    }

    /// Publish when the view moved: a delta against the last
    /// publication, or a snapshot when the pane set changed (or nothing
    /// was published yet). The trigger is the accepted preparation's
    /// generation OR any applied action/event since the last
    /// publication — semantic state (mode, message, dirty, picker) can
    /// move without the preparation stamp (a save completion flips
    /// `dirty` at the same buffer revision). A same-generation delta is
    /// the truthful shape for that; an unchanged observation sends
    /// nothing at all.
    fn publish<W: Write>(
        &mut self,
        editor: &mut Editor,
        out: &mut io::BufWriter<W>,
    ) -> io::Result<()> {
        if !self.handshaken {
            return Ok(());
        }
        let generation = editor.prepare_view(self.geometry).generation;
        let moved = self
            .published
            .as_ref()
            .is_none_or(|view| view.generation != generation);
        if !moved && !self.pending_observations {
            return Ok(());
        }
        self.pending_observations = false;
        let view = snapshot::build(editor, generation);
        if !moved {
            if let Some(previous) = &self.published {
                if previous.panes == view.panes
                    && previous.state == view.state
                    && previous.geometry == view.geometry
                    && previous.active_pane == view.active_pane
                {
                    return Ok(());
                }
            }
        }
        let message = match &self.published {
            Some(previous) if previous.panes.len() == view.panes.len() => ServerMessage::Delta {
                incarnation: self.incarnation,
                delta: snapshot::diff(previous, &view),
            },
            _ => ServerMessage::Snapshot {
                incarnation: self.incarnation,
                view: view.clone(),
            },
        };
        if matches!(message, ServerMessage::Snapshot { .. }) {
            self.client_known = self.client_known.max(generation);
        }
        self.published = Some(view);
        self.send(out, &message)
    }

    /// The engine stages OSC52 clipboard payloads for its frontend; over
    /// this protocol the client is it (AR08 host effect → result).
    fn emit_effects<W: Write>(
        &mut self,
        editor: &mut Editor,
        out: &mut io::BufWriter<W>,
    ) -> io::Result<()> {
        for text in editor.take_terminal_output() {
            let id = self.next_effect;
            self.next_effect += 1;
            self.send(
                &mut *out,
                &ServerMessage::Effect {
                    id,
                    effect: EffectRequest::ClipboardWrite { text },
                },
            )?;
        }
        Ok(())
    }

    /// Bounded engine-event drain, the headless driver's turn shape:
    /// EVENTS_PER_TURN per pass under the turn budget.
    fn drain_events(&mut self, editor: &mut Editor, events: &EventReceiver) -> io::Result<()> {
        let started = Instant::now();
        for _ in 0..strop_engine::editor::events::EVENTS_PER_TURN {
            let Ok(event) = events.try_recv() else {
                break;
            };
            let tick = editor.tape().sample_tick();
            self.pending_observations = true;
            editor.recorded_action(Action::Event(event), tick)?;
            if started.elapsed() >= strop_engine::editor::events::TURN_BUDGET {
                break;
            }
        }
        Ok(())
    }

    /// Deliberate shutdown: finish background work, then settle bounded
    /// (the headless jobs barrier: the bound fails deadlocks, it never
    /// budgets legitimate work).
    fn finish(&mut self, editor: &mut Editor, events: &EventReceiver) -> io::Result<()> {
        let tick = editor.tape().sample_tick();
        editor.recorded_action(Action::Finish, tick)?;
        let budget = std::env::var_os("STROP_JOBS_BUDGET_MS")
            .and_then(|raw| raw.to_str()?.parse::<u64>().ok())
            .map_or(Duration::from_secs(120), Duration::from_millis);
        let deadline = Instant::now() + budget;
        loop {
            self.drain_events(editor, events)?;
            if !editor.async_pending() {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "editor jobs did not settle",
                ));
            }
            match events.recv_timeout(remaining) {
                Ok(event) => {
                    let tick = editor.tape().sample_tick();
                    editor.recorded_action(Action::Event(event), tick)?;
                }
                Err(strop_engine::editor::events::RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "editor jobs did not settle",
                    ));
                }
                Err(strop_engine::editor::events::RecvTimeoutError::Disconnected) => {
                    return Ok(());
                }
            }
        }
    }

    fn unexpected<W: Write>(&mut self, out: &mut io::BufWriter<W>, what: &str) -> io::Result<Flow> {
        self.send(
            out,
            &ServerMessage::Error {
                seq: None,
                error: ProtocolError::Unexpected {
                    message: what.into(),
                },
            },
        )?;
        Ok(Flow::Open)
    }

    fn send<W: Write>(
        &mut self,
        out: &mut io::BufWriter<W>,
        message: &ServerMessage,
    ) -> io::Result<()> {
        frame::write_message(&mut *out, message)
    }
}
/// One u64 per backend process: publications and action bases key on it,
/// so a restarted backend can never resolve a stale client's state.
fn incarnation() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ (u64::from(std::process::id()).rotate_left(32))
}

/// The admitted wire action → the shared `AppEvent`: the SAME admitted
/// engine surface the TUI/headless drive, never a parallel table.
fn app_event(action: AdmittedAction) -> AppEvent {
    match action {
        AdmittedAction::Input(input) => AppEvent::Input(input),
        AdmittedAction::EditorKey(key) => AppEvent::EditorKey(key),
        AdmittedAction::Paste(text) => AppEvent::Paste(text),
        AdmittedAction::Resize { columns, rows } => AppEvent::Resize { columns, rows },
        AdmittedAction::QuitIntent => AppEvent::QuitIntent,
        AdmittedAction::Focus(focused) => AppEvent::Focus(focused),
    }
}

/// The request reader: bytes → bounded frames → bodies. All waiting is
/// the blocking pipe read; every violation is typed and terminal.
fn read_requests<R: Read>(mut reader: R, tx: SyncSender<Request>) {
    let mut decoder = FrameDecoder::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => {
                // Clean only at a frame boundary; a partial frame at EOF
                // is a truncated stream, reported typed before closing.
                let _ = tx.send(if decoder.is_empty() {
                    Request::Eof
                } else {
                    Request::Frame(FrameError::Truncated)
                });
                return;
            }
            Ok(n) => {
                if let Err(error) = decoder.accept(&chunk[..n]) {
                    let _ = tx.send(Request::Frame(error));
                    return;
                }
                loop {
                    match decoder.next_frame() {
                        Ok(Some(body)) => {
                            if tx.send(Request::Body(body)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            let _ = tx.send(Request::Frame(error));
                            return;
                        }
                    }
                }
            }
        }
    }
}
