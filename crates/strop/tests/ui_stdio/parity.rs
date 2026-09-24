//! 0057 VF14 parity journeys (the WSL lane's stdio closure): the claims
//! of `specs/UiSession.tla` exercised against the real backend — a
//! mid-journey resync after a synthetic drop, stale-action refusal on
//! both sides of the wire, the clipboard effect round-trip exactly once
//! per admitted request, the wrong-version refusal, and orderly shutdown
//! with work in flight.

use strop_ui_protocol::{
    AckOutcome, AdmittedAction, BaseStamp, Client, ClientCapabilities, ClientError, ClientEvent,
    ClientInfo, ClientMessage, ProtocolError, Refusal, ResyncReason, ServerMessage, ShutdownReason,
};

use crate::harness::{fixture, spawn, state, Raw};

/// One scripted key sequence as a raw admitted-action batch (the same
/// engine script tokens the headless driver consumes).
fn act_message(seq: u64, base: BaseStamp, keys: &str) -> ClientMessage {
    let actions: Vec<AdmittedAction> = strop_engine::editor::keys::parse(keys)
        .map(|key| AdmittedAction::Input(strop_core::frontend_input::Input::Key(key)))
        .collect();
    ClientMessage::Act { seq, base, actions }
}

/// Mid-journey resync after a synthetic drop (UiSession.tla's
/// PoisonedUntilSnapshot / PoisonedNeverActs): the pure client rides the
/// real wire, one delta is deliberately dropped, the next delta poisons
/// the cache, a poisoned cache cannot produce a base stamp, the stale
/// base is refused server-side, and the explicit resync's complete
/// current snapshot re-establishes the base.
#[test]
fn mid_journey_drop_poisons_and_resync_recovers() {
    let fixture = fixture();
    let mut raw = Raw::spawn(fixture.dir.path());
    let incarnation = raw.hello();
    let mut client = Client::new(
        // The welcome's backend identity is the client constructor's
        // input; only the incarnation is load-bearing here.
        &strop_ui_protocol::BackendInfo {
            name: "strop".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            build: None,
            incarnation,
        },
    );
    let snapshot = raw.recv_snapshot();
    client.apply(&snapshot);
    assert!(client.view().is_some(), "the initial publication landed");

    // Journey: open through the admitted surface, then edit — pumping
    // every publication into the pure client until each barrier lands.
    let base = client.base_stamp().unwrap();
    raw.send(&act_message(1, base, ":e notes.txt<cr>"));
    loop {
        let loaded = client.view().is_some_and(|view| {
            view.panes
                .iter()
                .any(|pane| pane.lines.first().map(String::as_str) == Some("alpha"))
        });
        if loaded {
            break;
        }
        let message = raw.recv();
        if let ServerMessage::Ack { seq: 1, outcome } = &message {
            assert!(
                matches!(outcome, AckOutcome::Applied { .. }),
                "the open is admitted on the snapshot's base: {outcome:?}"
            );
        }
        client.apply(&message);
    }
    let base = client.base_stamp().unwrap();
    raw.send(&act_message(2, base, "odelta one<esc>"));
    loop {
        let edited = client.view().is_some_and(|view| {
            view.panes
                .iter()
                .any(|pane| pane.lines.iter().any(|line| line == "delta one"))
        });
        if edited {
            break;
        }
        let message = raw.recv();
        if let ServerMessage::Ack { seq: 2, outcome } = &message {
            assert!(
                matches!(outcome, AckOutcome::Applied { .. }),
                "the edit is admitted: {outcome:?}"
            );
        }
        client.apply(&message);
    }

    // The synthetic drop: an undo plus a viewport interest each publish
    // an ordered delta (content moved; geometry moved). Every delta up
    // to the viewport's acknowledgement is buffered client-side; the
    // buffer is then replayed with its first entry withheld — the
    // poison rule (client.rs: a delta applies only onto its exact base)
    // must fire. The undo's generation moves past every pre-undo stamp,
    // so either a replayed delta finds a base gap, or the withheld delta
    // itself no longer fits the advanced cache.
    let pre_gen = client.generation();
    let base = client.base_stamp().unwrap();
    raw.send(&act_message(3, base, "u"));
    raw.send(&ClientMessage::Viewport {
        seq: 4,
        columns: 90,
        rows: 24,
    });
    let mut buffered: Vec<ServerMessage> = Vec::new();
    let mut acked = false;
    // The undo's publication is certain (content moved); the viewport's
    // follows its ack in the same server iteration (geometry moved).
    while !(acked && buffered.len() >= 2) {
        let message = raw.recv();
        match &message {
            ServerMessage::Delta { .. } => buffered.push(message),
            ServerMessage::Ack { seq: 4, outcome } => {
                assert!(
                    matches!(outcome, AckOutcome::Applied { .. }),
                    "the viewport interest is admitted: {outcome:?}"
                );
                acked = true;
            }
            _ => {
                client.apply(&message);
            }
        }
    }
    let withheld = buffered.remove(0);
    let mut poison = None;
    for message in buffered {
        for event in client.apply(&message) {
            if let ClientEvent::ResyncRequired { reason } = event {
                poison = Some(reason);
            }
        }
    }
    if poison.is_none() {
        // Every replayed delta chained cleanly: the withheld delta was a
        // same-generation observation. It no longer fits the advanced
        // cache — applying it now must poison.
        for event in client.apply(&withheld) {
            if let ClientEvent::ResyncRequired { reason } = event {
                poison = Some(reason);
            }
        }
    }
    assert!(
        matches!(poison, Some(ResyncReason::DeltaBaseMismatch { .. })),
        "the withheld delta poisons with a base mismatch: {poison:?}"
    );

    // Stale-action refusal, both sides: the poisoned cache refuses to
    // produce a base stamp (client.rs), and a base naming the pre-drop
    // generation — below the acknowledged floor, whichever replay path
    // poisoned — is refused typed by the server (serve.rs admit).
    assert_eq!(client.base_stamp(), Err(ClientError::Poisoned));
    let stale = pre_gen;
    raw.send(&ClientMessage::Act {
        seq: 5,
        base: BaseStamp {
            incarnation,
            generation: stale,
        },
        actions: vec![],
    });
    assert!(
        matches!(
            raw.recv_control(),
            ServerMessage::Ack {
                seq: 5,
                outcome: AckOutcome::Refused {
                    refusal: Refusal::StaleGeneration { .. }
                }
            }
        ),
        "the stale mid-journey base is refused, not applied"
    );

    // The explicit resync: the complete current snapshot clears the
    // poison and re-establishes the base; actions resume on it.
    raw.send(&ClientMessage::Resync { seq: 6 });
    raw.recv_ack(6);
    let snapshot = raw.recv_snapshot();
    client.apply(&snapshot);
    assert!(client.poisoned().is_none(), "the snapshot clears poisoning");
    let base = client.base_stamp().unwrap();
    raw.send(&ClientMessage::Act {
        seq: 7,
        base,
        actions: vec![],
    });
    assert!(
        matches!(
            raw.recv_control(),
            ServerMessage::Ack {
                seq: 7,
                outcome: AckOutcome::Applied { .. }
            }
        ),
        "the resynced base admits actions again"
    );
    raw.send(&ClientMessage::Shutdown { seq: 8 });
    raw.recv_ack(8);
    assert!(matches!(
        raw.recv_control(),
        ServerMessage::Bye {
            reason: ShutdownReason::Requested
        }
    ));
    assert!(raw.wait().success());
}

/// The clipboard effect round-trip (UiSession.tla's EffectExactlyOnce):
/// each admitted yank stages one OSC52 payload, each crosses as exactly
/// one host effect with a monotone id, the driver answers each
/// `Applied`, and the session stays responsive afterwards.
#[test]
fn clipboard_effects_round_trip_exactly_once() {
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

    driver.act_keys("\"+yy").unwrap();
    driver.act_keys("\"+yy").unwrap();
    assert_eq!(
        driver.effects(),
        &[
            (
                0,
                strop_ui_protocol::EffectRequest::ClipboardWrite {
                    text: "alpha\n".into()
                }
            ),
            (
                1,
                strop_ui_protocol::EffectRequest::ClipboardWrite {
                    text: "alpha\n".into()
                }
            )
        ],
        "one effect per admitted yank: monotone ids, no duplicate, none unstaged"
    );

    // The round-trips answered, the journey continues on the same base.
    driver.act_keys("G").unwrap();
    assert_eq!(state(&driver)["line"], 3);
    let status = driver.shutdown().unwrap();
    assert!(status.success());
}

/// A wrong-version hello is a typed refusal and a terminal bye (AR09:
/// the version handshake never guesses), the WSL lane's startup
/// negotiation case.
#[test]
fn wrong_version_hello_is_typed_and_terminal() {
    let fixture = fixture();
    let mut raw = Raw::spawn(fixture.dir.path());
    raw.send(&ClientMessage::Hello {
        protocol: strop_ui_protocol::PROTOCOL_VERSION + 1,
        client: ClientInfo {
            name: "parity".into(),
            version: "0.1".into(),
        },
        capabilities: ClientCapabilities {
            clipboard_write: true,
        },
    });
    assert!(
        matches!(
            raw.recv(),
            ServerMessage::Error {
                error: ProtocolError::Version { supported, offered },
                ..
            } if supported == strop_ui_protocol::PROTOCOL_VERSION
                && offered == strop_ui_protocol::PROTOCOL_VERSION + 1
        ),
        "the offered version is refused typed"
    );
    assert!(matches!(
        raw.recv(),
        ServerMessage::Bye {
            reason: ShutdownReason::ProtocolViolation
        }
    ));
    assert!(raw.wait().success());
}

/// Orderly shutdown with work in flight (UiSession.tla's
/// ByeAfterFinalAck / NoPostByePublication): the worker-served read has
/// not necessarily landed when the shutdown is authorized — finish()
/// settles the engine's jobs (the headless jobs barrier, never a sleep),
/// the final ack precedes the bye, and the exit is clean. The journey is
/// scheduling-independent: whether the read lands before or after the
/// shutdown, the outcome is the same orderly close.
#[test]
fn shutdown_drains_in_flight_work() {
    let fixture = fixture();
    let mut driver = spawn(fixture.dir.path());
    // The read lands asynchronously on the worker (WK04); the act
    // barrier is the acknowledgement, not the content.
    driver.act_keys(":e notes.txt<cr>").unwrap();
    // No settle: the shutdown is authorized with the read in flight.
    let status = driver.shutdown().unwrap();
    assert!(status.success(), "shutdown with in-flight work: {status}");

    // And the stronger ordering: a second session where the read has
    // landed and an edit is dirty — shutdown still acks then byes, and
    // the editor's own checkpoint policy owns the unsaved bytes.
    let mut driver = spawn(fixture.dir.path());
    driver.act_keys(":e notes.txt<cr>").unwrap();
    driver
        .wait_view("file loaded", |view| {
            view.panes
                .iter()
                .any(|pane| pane.lines.first().map(String::as_str) == Some("alpha"))
        })
        .unwrap();
    driver.act_keys("odelta one<esc>").unwrap();
    assert_eq!(state(&driver)["dirty"], true);
    let status = driver.shutdown().unwrap();
    assert!(status.success(), "shutdown with a dirty buffer: {status}");
}
