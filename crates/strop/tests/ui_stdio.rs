//! 0056 AR09/AR10 closure: the real `strop --ui-stdio` backend over
//! pipes. This repo's lane is WSL — spawning the actual binary over
//! stdio pipes IS the WSL/stdio smoke (no window, GPUI, IME or
//! installer involved). Every test is hermetic: HOME/XDG point at a
//! tempdir, no network, no wall-clock sleeps — the driver's barriers
//! block on protocol messages and fail on budget expiry.

use std::path::Path;
use std::process::{Child, Command, Stdio};

use strop_ui_protocol::frame::{self, FrameDecoder};
use strop_ui_protocol::{
    AdmittedAction, ClientMessage, Driver, DriverError, Refusal, ServerMessage,
};

fn strop() -> &'static str {
    env!("CARGO_BIN_EXE_strop")
}

/// A hermetic backend root: fixture file plus private HOME/XDG.
struct Fixture {
    dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
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

fn env(dir: &Path) -> Vec<(&'static str, std::ffi::OsString)> {
    vec![
        ("HOME", dir.join("home").into_os_string()),
        ("XDG_CONFIG_HOME", dir.join("config").into_os_string()),
        ("XDG_STATE_HOME", dir.join("state").into_os_string()),
        ("STROP_LOG", std::ffi::OsString::new()),
    ]
}

fn spawn(dir: &Path) -> Driver {
    Driver::spawn(Path::new(strop()), dir, &env(dir))
        .unwrap_or_else(|error| panic!("backend handshake: {error}"))
}

fn state(driver: &Driver) -> &serde_json::Value {
    &driver.client().view().expect("a published view").state
}

fn pane_lines(driver: &Driver) -> Vec<String> {
    let view = driver.client().view().expect("a published view");
    view.panes[view.active_pane].lines.clone()
}

/// The open/edit/undo/save journey, asserting the same observable
/// states the headless driver pins (`state_json` rides in every view).
#[test]
fn open_edit_undo_save_journey() {
    let fixture = fixture();
    let notes = fixture.dir.path().join("notes.txt");
    let mut driver = spawn(fixture.dir.path());

    assert_eq!(state(&driver)["mode"], "NORMAL");

    // Open through the admitted input surface (`:e`), not a backdoor.
    // The read lands asynchronously; the barrier is the content itself.
    driver.act_keys(":e notes.txt<cr>").unwrap();
    driver
        .wait_view("file loaded", |view| {
            view.panes
                .iter()
                .any(|pane| pane.lines.first().map(String::as_str) == Some("alpha"))
        })
        .unwrap();
    assert_eq!(state(&driver)["dirty"], false);
    assert_eq!(
        pane_lines(&driver).first().map(String::as_str),
        Some("alpha")
    );

    // Edit: a new line below the first.
    driver.act_keys("odelta one<esc>").unwrap();
    assert!(pane_lines(&driver).iter().any(|line| line == "delta one"));
    assert_eq!(state(&driver)["dirty"], true);

    // Undo restores the exact prior text.
    driver.act_keys("u").unwrap();
    assert!(!pane_lines(&driver).iter().any(|line| line == "delta one"));

    // Save publishes a clean buffer; the disk carries the truth.
    driver.act_keys(":w<cr>").unwrap();
    driver
        .wait_view("saved", |view| view.state["dirty"] == false)
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&notes).unwrap(),
        "alpha\nbeta gamma\nalpha beta\n"
    );

    // Edit + save again: the checked save persists the new bytes.
    driver.act_keys("odelta one<esc>").unwrap();
    driver.act_keys(":w<cr>").unwrap();
    driver
        .wait_view("saved again", |view| view.state["dirty"] == false)
        .unwrap();
    assert!(std::fs::read_to_string(&notes)
        .unwrap()
        .contains("delta one"));

    let status = driver.shutdown().unwrap();
    assert!(status.success(), "orderly shutdown: {status}");
}

/// The Boolean query language (0063 §3) through the workspace Search
/// surface (`space /`), over real pipes, with revision barriers instead
/// of sleeps.
#[test]
fn boolean_search_journey() {
    let fixture = fixture();
    let mut driver = spawn(fixture.dir.path());

    driver.act_keys("<space>/").unwrap();
    assert_eq!(state(&driver)["picker"], true);

    driver.act_keys("alpha AND beta").unwrap();
    driver
        .wait_view("search results", |view| {
            view.state["picker_streaming"] == false
                && view.state["picker_items"]
                    .as_u64()
                    .is_some_and(|items| items >= 1)
        })
        .unwrap();
    // The conjunction's two hit spans both sit on "alpha beta" —
    // "beta gamma" (one atom) and a literal reading match nothing.
    assert_eq!(state(&driver)["picker_items"], 2);
    // Opening the hit loads the file asynchronously, then lands on the
    // matched line: the barrier is the landing itself.
    driver.act_keys("<cr>").unwrap();
    driver
        .wait_view("hit open", |view| {
            view.state["picker"] == false && view.state["line"] == 3
        })
        .unwrap();
    assert!(pane_lines(&driver).iter().any(|line| line == "alpha beta"));

    let status = driver.shutdown().unwrap();
    assert!(status.success());
}

/// Viewport interest rides the same resize a TUI delivers; the cell
/// bound refuses typed; the OSC52 clipboard write arrives as a host
/// effect request (AR08) and is answered.
#[test]
fn viewport_bounds_and_clipboard_effect() {
    let fixture = fixture();
    let mut driver = spawn(fixture.dir.path());
    driver.act_keys(":e notes.txt<cr>").unwrap();
    driver
        .wait_view("file loaded", |view| {
            view.panes
                .iter()
                .any(|pane| pane.lines.first().map(String::as_str) == Some("alpha"))
        })
        .unwrap();

    // Viewport interest → the same admitted resize → new geometry.
    driver.viewport(120, 40).unwrap();
    let view = driver.client().view().unwrap();
    assert_eq!(view.geometry.columns, 120);
    assert_eq!(view.geometry.rows, 40);

    // The AR06 cell bound refuses typed, not clamped.
    let error = driver.viewport(2000, 1000).unwrap_err();
    assert!(
        matches!(
            error,
            DriverError::Refused(strop_ui_protocol::Refusal::Limit { .. })
        ),
        "oversized viewport: {error}"
    );

    // Yank to the `+` register stages an OSC52 payload; over the
    // protocol the client is the host-effect target.
    driver.act_keys("\"+yy").unwrap();
    assert_eq!(
        driver.effects(),
        &[(
            0,
            strop_ui_protocol::EffectRequest::ClipboardWrite {
                text: "alpha\n".into()
            }
        )],
        "the clipboard write crosses as a host effect"
    );

    let status = driver.shutdown().unwrap();
    assert!(status.success());
}

/// The quit intent (ctrl-c's admitted form) is the editor's own
/// decision: a clean scratch session quits through it.
#[test]
fn quit_intent_exits_orderly() {
    let fixture = fixture();
    let mut driver = spawn(fixture.dir.path());
    driver.act(vec![AdmittedAction::QuitIntent]).unwrap();
    let status = driver.wait_exit().unwrap();
    assert!(status.success(), "quit intent: {status}");
}
/// A terminal session through the protocol: launch, child input, live
/// output projection, exit — the 0065 modal terminal states observable
/// in the semantic view.
#[test]
fn terminal_journey() {
    let fixture = fixture();
    let mut driver = spawn(fixture.dir.path());

    driver
        .act_keys(":terminal cat<cr>")
        .unwrap_or_else(|error| panic!("terminal launch: {error} — stderr: {}", driver.stderr()));
    driver
        .wait_view("terminal running", |view| {
            view.state["terminal"]["phase"] == "running"
        })
        .unwrap();
    assert_eq!(state(&driver)["mode"], "TERMINAL");

    // Child input echoes through the PTY and lands in the projection.
    driver
        .act_keys("hi there<cr>")
        .unwrap_or_else(|error| panic!("child input: {error} — stderr: {}", driver.stderr()));
    driver
        .wait_view("terminal echo", |view| {
            view.panes
                .iter()
                .any(|pane| pane.lines.iter().any(|line| line.contains("hi there")))
        })
        .unwrap_or_else(|error| panic!("echo: {error} — stderr: {}", driver.stderr()));

    // ctrl-d ends cat; the exit is observable, the output retained.
    driver
        .act(vec![AdmittedAction::Input(
            strop_core::frontend_input::Input::Key(ctrl('d')),
        )])
        .unwrap_or_else(|error| panic!("ctrl-d: {error} — stderr: {}", driver.stderr()));
    driver
        .wait_view("terminal exit", |view| {
            view.state["terminal"]["phase"] == "exited"
        })
        .unwrap_or_else(|error| panic!("exit: {error} — stderr: {}", driver.stderr()));

    // ctrl-\ ctrl-n enters editor Normal mode (0065); :q closes.
    driver
        .act_keys("<c-\\><c-n>")
        .unwrap_or_else(|error| panic!("leave input: {error} — stderr: {}", driver.stderr()));
    assert_eq!(state(&driver)["mode"], "NORMAL");
    driver
        .act_keys(":q<cr>")
        .unwrap_or_else(|error| panic!("close: {error} — stderr: {}", driver.stderr()));
    let status = driver
        .wait_exit()
        .unwrap_or_else(|error| panic!("wait exit: {error}"));
    assert!(status.success(), "quit through the grammar: {status}");
}

fn ctrl(ch: char) -> strop_core::frontend_input::KeyEvent {
    let mut key =
        strop_core::frontend_input::KeyEvent::press(strop_core::frontend_input::KeyCode::Char(ch));
    key.modifiers.control = true;
    key
}

/// Parity (AR10): one key script driven through the headless driver and
/// through the protocol must publish the same logical states.
#[test]
fn protocol_states_match_the_headless_driver() {
    let fixture = fixture();
    let steps = [":e notes.txt<cr>", "odelta one<esc>", "u", "gg", "G", "0w"];
    // Headless: the scripted driver prints `─── state {json}` per step.
    // The open's read lands asynchronously on BOTH sides: the script
    // settles (the headless jobs barrier), the protocol barriers on the
    // content itself — then both observe the same quiesced state.
    let script = steps
        .iter()
        .map(|keys| {
            if keys.starts_with(':') {
                format!("keys {keys}\nsettle\nstate\n")
            } else {
                format!("keys {keys}\nstate\n")
            }
        })
        .collect::<String>();
    let script_path = fixture.dir.path().join("journey.stropscript");
    std::fs::write(&script_path, &script).unwrap();
    let output = Command::new(strop())
        .arg("--headless")
        .arg(&script_path)
        .current_dir(fixture.dir.path())
        .envs(env(fixture.dir.path()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "headless run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let headless: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("─── state "))
        .map(|json| serde_json::from_str(json).unwrap())
        .collect();
    assert_eq!(headless.len(), steps.len(), "every step pinned a state");

    // Protocol: the same admitted keys, the same observation.
    let mut driver = spawn(fixture.dir.path());
    for (keys, expected) in steps.iter().zip(&headless) {
        driver.act_keys(keys).unwrap();
        if keys.starts_with(':') {
            // The asynchronous open: barrier on the landed content, the
            // protocol's form of the script's `settle`.
            driver
                .wait_view("file loaded", |view| {
                    view.panes
                        .iter()
                        .any(|pane| pane.lines.first().map(String::as_str) == Some("alpha"))
                })
                .unwrap();
        }
        assert_eq!(
            state(&driver),
            expected,
            "protocol state diverges from headless after {keys:?}"
        );
    }
    let status = driver.shutdown().unwrap();
    assert!(status.success());
}

/// Malformed traffic (AR09): garbage framing and unknown envelopes earn
/// typed errors — never a panic, never a silent divergence — and a
/// well-framed stream survives a bad envelope.
#[test]
fn malformed_frames_get_typed_errors() {
    // Frame-level garbage poisons the stream: typed error + bye.
    let garbage = fixture();
    let mut raw = Raw::spawn(garbage.dir.path());
    raw.send_bytes(b"this is not a frame\r\n\r\n");
    let error = raw.recv();
    assert!(
        matches!(
            &error,
            ServerMessage::Error {
                error: strop_ui_protocol::ProtocolError::Frame { .. },
                ..
            }
        ),
        "garbage framing earns a typed frame error: {error:?}"
    );
    let bye = raw.recv();
    assert!(
        matches!(
            bye,
            ServerMessage::Bye {
                reason: strop_ui_protocol::ShutdownReason::ProtocolViolation
            }
        ),
        "a poisoned stream closes with a typed reason: {bye:?}"
    );
    assert!(raw.wait().success());

    // An oversized declaration is refused at the header (AR06 bounds).
    let oversized = fixture();
    let mut raw = Raw::spawn(oversized.dir.path());
    raw.send_bytes(format!("Content-Length: {}\r\n\r\n", frame::MAX_BODY_BYTES + 1).as_bytes());
    let error = raw.recv();
    assert!(
        matches!(&error, ServerMessage::Error { error: strop_ui_protocol::ProtocolError::Frame { message }, .. } if message.contains("exceeds")),
        "oversized body is a typed refusal: {error:?}"
    );
    assert!(raw.wait().success());

    // A partial frame at EOF is a truncated stream — a typed protocol
    // violation, not a clean disconnect.
    let truncated = fixture();
    let mut raw = Raw::spawn(truncated.dir.path());
    raw.send_bytes(b"Content-Length: 50\r\n\r\n{\"type\":");
    drop(raw.stdin.take());
    let error = raw.recv();
    assert!(
        matches!(&error, ServerMessage::Error { error: strop_ui_protocol::ProtocolError::Frame { message }, .. } if message.contains("mid-frame")),
        "truncation is typed: {error:?}"
    );
    assert!(matches!(
        raw.recv(),
        ServerMessage::Bye {
            reason: strop_ui_protocol::ShutdownReason::ProtocolViolation
        }
    ));
    assert!(raw.wait().success());

    // A well-framed unknown envelope is a decode error; the stream lives.
    let decode = fixture();
    let mut raw = Raw::spawn(decode.dir.path());
    raw.hello();
    raw.recv_snapshot();
    raw.send_bytes(&{
        let mut framed = Vec::new();
        frame::write_frame(&mut framed, br#"{"type":"teleport"}"#).unwrap();
        framed
    });
    let error = raw.recv_control();
    assert!(
        matches!(
            &error,
            ServerMessage::Error {
                error: strop_ui_protocol::ProtocolError::Decode { .. },
                ..
            }
        ),
        "a bad envelope is typed: {error:?}"
    );
    // The framing stayed aligned: the stream still serves.
    raw.send(&ClientMessage::Resync { seq: 1 });
    raw.recv_ack(1);
    raw.recv_snapshot();
    raw.send(&ClientMessage::Shutdown { seq: 2 });
    raw.recv_ack(2);
    assert!(matches!(raw.recv_control(), ServerMessage::Bye { .. }));
    assert!(raw.wait().success());
}

/// Stale-generation actions are refused with the current generation;
/// an explicit resync is the recovery (AR09: no blind application).
#[test]
fn stale_generations_refuse_and_resync_recovers() {
    let fixture = fixture();
    let mut raw = Raw::spawn(fixture.dir.path());
    let incarnation = raw.hello();
    raw.recv_snapshot();

    // Base generation 0 predates the initial publication.
    raw.send(&ClientMessage::Act {
        seq: 10,
        base: strop_ui_protocol::BaseStamp {
            incarnation,
            generation: 0,
        },
        actions: vec![AdmittedAction::Input(
            strop_core::frontend_input::Input::Text("x".into()),
        )],
    });
    let refusal = raw.recv_control();
    assert!(
        matches!(
            refusal,
            ServerMessage::Ack {
                seq: 10,
                outcome: strop_ui_protocol::AckOutcome::Refused {
                    refusal: Refusal::StaleGeneration { .. }
                }
            }
        ),
        "a stale base is refused, not applied: {refusal:?}"
    );
    raw.send(&ClientMessage::Act {
        seq: 11,
        base: strop_ui_protocol::BaseStamp {
            incarnation: incarnation ^ 1,
            generation: 1,
        },
        actions: vec![],
    });
    assert!(matches!(
        raw.recv_control(),
        ServerMessage::Ack {
            seq: 11,
            outcome: strop_ui_protocol::AckOutcome::Refused {
                refusal: Refusal::WrongIncarnation { .. }
            }
        }
    ));
    // Explicit resync: the complete current snapshot.
    raw.send(&ClientMessage::Resync { seq: 12 });
    raw.recv_ack(12);
    let generation = match raw.recv_snapshot() {
        ServerMessage::Snapshot {
            incarnation: seen,
            view,
        } => {
            assert_eq!(seen, incarnation, "the same backend answers");
            view.generation
        }
        other => panic!("resync ships a complete current snapshot: {other:?}"),
    };
    // And the base the snapshot re-established admits actions again.
    raw.send(&ClientMessage::Act {
        seq: 13,
        base: strop_ui_protocol::BaseStamp {
            incarnation,
            generation,
        },
        actions: vec![],
    });
    assert!(matches!(
        raw.recv_control(),
        ServerMessage::Ack {
            seq: 13,
            outcome: strop_ui_protocol::AckOutcome::Applied { .. }
        }
    ));
    raw.send(&ClientMessage::Shutdown { seq: 14 });
    raw.recv_ack(14);
    assert!(matches!(raw.recv_control(), ServerMessage::Bye { .. }));
    assert!(raw.wait().success());
}

/// Unicode content/quoting and partial IO: a hello delivered byte by
/// byte, a unicode client name, a unicode path, unicode text.
#[test]
fn unicode_and_partial_io() {
    let fixture = fixture();
    let path = fixture.dir.path().join("héllo wörld.txt");
    std::fs::write(&path, "日本語の行\n").unwrap();
    let mut raw = Raw::spawn(fixture.dir.path());

    let hello = serde_json::to_vec(&ClientMessage::Hello {
        protocol: strop_ui_protocol::PROTOCOL_VERSION,
        client: strop_ui_protocol::ClientInfo {
            name: "smörgås-クライアント".into(),
            version: "0.1".into(),
        },
        capabilities: strop_ui_protocol::ClientCapabilities {
            clipboard_write: true,
        },
    })
    .unwrap();
    let mut framed = Vec::new();
    frame::write_frame(&mut framed, &hello).unwrap();
    // Partial IO: one byte at a time, no sleeps — pipe writes are the
    // schedule.
    for byte in framed {
        raw.send_bytes(&[byte]);
    }
    raw.recv_welcome();
    raw.recv_snapshot();
    raw.send(&ClientMessage::Shutdown { seq: 1 });
    raw.recv_ack(1);
    assert!(matches!(raw.recv_control(), ServerMessage::Bye { .. }));
    assert!(raw.wait().success());

    // Unicode path and content through the admitted surface.
    let mut driver = spawn(fixture.dir.path());
    driver.act_keys(":e héllo wörld.txt<cr>").unwrap();
    driver
        .wait_view("unicode file loaded", |view| {
            view.panes
                .iter()
                .any(|pane| pane.lines.first().map(String::as_str) == Some("日本語の行"))
        })
        .unwrap();
    assert_eq!(
        pane_lines(&driver).first().map(String::as_str),
        Some("日本語の行")
    );
    driver.act_keys("ogrüße 世界<esc>").unwrap();
    assert!(pane_lines(&driver).iter().any(|line| line == "grüße 世界"));
    let status = driver.shutdown().unwrap();
    assert!(status.success());
}

/// Orderly lifecycle: client EOF drains and exits cleanly (AR04
/// disconnect policy: checkpoint/drain, no orphan work).
#[test]
fn client_eof_shuts_down_orderly() {
    let fixture = fixture();
    let mut raw = Raw::spawn(fixture.dir.path());
    raw.hello();
    raw.recv_snapshot();
    drop(raw.stdin.take());
    // The backend notices the closed link, finishes and says goodbye.
    loop {
        match raw.recv() {
            ServerMessage::Bye {
                reason: strop_ui_protocol::ShutdownReason::Disconnect,
            } => break,
            ServerMessage::Bye { reason } => panic!("unexpected bye: {reason:?}"),
            _ => {}
        }
    }
    assert!(raw.wait().success(), "EOF is an orderly disconnect");
}

/// A minimal raw client for malformed/partial traffic: the Driver
/// (correctly) never produces such bytes.
struct Raw {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: std::process::ChildStdout,
    decoder: FrameDecoder,
}

impl Raw {
    fn spawn(dir: &Path) -> Self {
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

    fn send(&mut self, message: &ClientMessage) {
        frame::write_message(self.stdin.as_mut().unwrap(), message).unwrap();
    }

    fn send_bytes(&mut self, bytes: &[u8]) {
        use std::io::Write;
        self.stdin.as_mut().unwrap().write_all(bytes).unwrap();
        self.stdin.as_mut().unwrap().flush().unwrap();
    }

    /// Bounded read: the budget is a hang canary (the headless
    /// `jobs_budget` precedent) — expiry fails the test, never budgets
    /// legitimate work.
    fn recv(&mut self) -> ServerMessage {
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
    fn recv_control(&mut self) -> ServerMessage {
        loop {
            match self.recv() {
                ServerMessage::Snapshot { .. } | ServerMessage::Delta { .. } => {}
                control => return control,
            }
        }
    }

    /// Hello → Welcome; returns the backend incarnation.
    fn hello(&mut self) -> u64 {
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

    fn recv_welcome(&mut self) -> u64 {
        match self.recv() {
            ServerMessage::Welcome { backend, .. } => backend.incarnation,
            other => panic!("expected welcome: {other:?}"),
        }
    }

    fn recv_ack(&mut self, seq: u64) {
        match self.recv_control() {
            ServerMessage::Ack { seq: seen, .. } => assert_eq!(seen, seq),
            other => panic!("expected ack {seq}: {other:?}"),
        }
    }

    fn recv_snapshot(&mut self) -> ServerMessage {
        loop {
            match self.recv() {
                snapshot @ ServerMessage::Snapshot { .. } => return snapshot,
                ServerMessage::Delta { .. } => {}
                other => panic!("expected snapshot: {other:?}"),
            }
        }
    }

    fn wait(&mut self) -> std::process::ExitStatus {
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

/// The driver's explicit resync barrier over pipes: a complete current
/// snapshot re-establishes the base and actions resume on it. (Dropped
/// deltas are the pure client's property — strop-ui-protocol's seeded
/// generator — and the refusal itself is pinned raw above; the Driver
/// never produces stale bases on its own.)
#[test]
fn driver_resync_reestablishes_the_base() {
    let fixture = fixture();
    let mut driver = spawn(fixture.dir.path());
    driver.act_keys(":e notes.txt<cr>").unwrap();
    let generation = driver.resync().unwrap();
    assert_eq!(generation, driver.client().generation());
    assert!(driver.client().poisoned().is_none());
    // The re-established base admits actions again.
    driver.act_keys("G").unwrap();
    assert_eq!(state(&driver)["line"], 3);
    let status = driver.shutdown().unwrap();
    assert!(status.success());
}
