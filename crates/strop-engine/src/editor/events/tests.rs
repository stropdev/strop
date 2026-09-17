//! Bounded-transport and settle-contract tests (0056 AR06): coalescing
//! policy per stream class, visible refusal, native-reader backpressure,
//! lane fairness under sustained output, and finite-work vs service
//! liveness at shutdown.
use super::*;
use strop_core::worker::WorkerId;
use strop_terminal::model::SessionId;

fn session(n: u64) -> SessionId {
    SessionId::from_request(WorkerId::new(n))
}

fn key(c: char) -> AppEvent {
    AppEvent::EditorKey(Key::Char(c))
}

fn drain(rx: &EventReceiver) -> Vec<AppEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn wakes(events: &[AppEvent]) -> Vec<SessionId> {
    events
        .iter()
        .filter_map(|event| match event {
            AppEvent::TerminalUpdate(session) => Some(*session),
            _ => None,
        })
        .collect()
}

fn pastes(events: &[AppEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            AppEvent::Paste(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn terminal_wakes_coalesce_per_session_without_losing_sessions() {
    let (tx, rx) = channel();
    for _ in 0..1000 {
        tx.send(AppEvent::TerminalUpdate(session(1))).unwrap();
    }
    tx.send(AppEvent::TerminalUpdate(session(2))).unwrap();
    tx.send(AppEvent::TerminalUpdate(session(1))).unwrap();
    // A flood for one session retains one wake; FIFO of first arrival.
    let events = drain(&rx);
    assert_eq!(wakes(&events), vec![session(1), session(2)]);
    assert_eq!(events.len(), 2);
    assert!(tx.take_refusals().is_empty());
}

#[test]
fn resize_and_focus_are_latest_wins() {
    let (tx, rx) = channel();
    for columns in 80..90u16 {
        tx.send(AppEvent::Resize { columns, rows: 24 }).unwrap();
    }
    tx.send(AppEvent::Focus(true)).unwrap();
    tx.send(AppEvent::Focus(false)).unwrap();
    let events = drain(&rx);
    assert_eq!(events.len(), 2);
    assert!(
        matches!(
            events[0],
            AppEvent::Resize {
                columns: 89,
                rows: 24
            }
        ),
        "only the latest resize survives"
    );
    assert!(matches!(events[1], AppEvent::Focus(false)));
}

#[test]
fn semantic_events_are_never_coalesced_or_reordered() {
    let (tx, rx) = channel();
    for i in 0..500 {
        tx.send(AppEvent::Paste(format!("chunk-{i}"))).unwrap();
    }
    let events = drain(&rx);
    assert_eq!(events.len(), 500);
    let expected: Vec<String> = (0..500).map(|i| format!("chunk-{i}")).collect();
    assert_eq!(
        pastes(&events),
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );
}

#[test]
fn sustained_output_flood_stays_bounded_and_input_is_never_lost() {
    // The AR06 fairness case: unbounded VT output (wake hints) churning
    // alongside paste traffic. Hints coalesce; every paste survives in
    // order; no send is refused.
    let (tx, rx) = channel();
    for i in 0..500 {
        tx.send(AppEvent::TerminalUpdate(session(7))).unwrap();
        tx.send(AppEvent::Paste(format!("paste-{i}"))).unwrap();
    }
    assert!(tx.take_refusals().is_empty());
    let events = drain(&rx);
    assert_eq!(
        wakes(&events),
        vec![session(7)],
        "a sustained output flood retains one wake"
    );
    let expected: Vec<String> = (0..500).map(|i| format!("paste-{i}")).collect();
    assert_eq!(
        pastes(&events),
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "every paste survives a sustained output flood, in order"
    );
}

#[test]
fn lanes_alternate_so_neither_class_starves() {
    let (tx, rx) = channel();
    for i in 0..5u64 {
        tx.send(AppEvent::TerminalUpdate(session(i + 1))).unwrap();
        tx.send(AppEvent::Paste(format!("p{i}"))).unwrap();
    }
    let events = drain(&rx);
    // Strict alternation while both lanes are pending.
    let hints: Vec<bool> = events
        .iter()
        .map(|event| matches!(event, AppEvent::TerminalUpdate(_)))
        .collect();
    assert_eq!(
        hints,
        vec![true, false, true, false, true, false, true, false, true, false]
    );
}

#[test]
fn a_full_semantic_lane_refuses_visibly_and_recovers_after_drain() {
    let (tx, rx) = channel();
    for _ in 0..MAX_SEMANTIC_EVENTS {
        tx.send(key('l')).unwrap();
    }
    // The next semantic event is refused: handed back to the producer
    // AND recorded for the UI.
    assert!(tx.send(key('l')).is_err());
    assert_eq!(
        tx.take_refusals(),
        vec![AdmissionRefusal {
            class: "editor key",
            bytes: 0
        }]
    );
    // Hints still admit while the semantic lane is full.
    tx.send(AppEvent::Resize {
        columns: 120,
        rows: 40,
    })
    .unwrap();
    // Draining frees room; the same event now admits.
    assert_eq!(drain(&rx).len(), MAX_SEMANTIC_EVENTS + 1);
    tx.send(key('l')).unwrap();
}

#[test]
fn paste_bytes_are_bounded_but_a_lone_oversize_paste_is_never_stranded() {
    let (tx, rx) = channel();
    let half = "x".repeat(MAX_QUEUED_PASTE_BYTES / 2);
    tx.send(AppEvent::Paste(half.clone())).unwrap();
    tx.send(AppEvent::Paste(half)).unwrap();
    // The lane is at the byte bound: a further paste is refused.
    assert!(tx
        .send(AppEvent::Paste("one byte too many".into()))
        .is_err());
    let refusals = rx.take_refusals();
    assert_eq!(
        refusals,
        vec![AdmissionRefusal {
            class: "paste",
            bytes: "one byte too many".len(),
        }]
    );
    // Everything admitted is delivered intact, and an oversize paste
    // into an EMPTY lane still admits: input is backpressured, never
    // silently dropped.
    assert_eq!(drain(&rx).len(), 2);
    let huge = "y".repeat(MAX_QUEUED_PASTE_BYTES * 2);
    tx.send(AppEvent::Paste(huge)).unwrap();
    let events = drain(&rx);
    assert_eq!(pastes(&events).len(), 1);
    assert_eq!(pastes(&events)[0].len(), MAX_QUEUED_PASTE_BYTES * 2);
}

#[test]
fn send_blocking_backpressures_the_reader_until_the_lane_drains() {
    let (tx, rx) = channel();
    for i in 0..MAX_SEMANTIC_EVENTS {
        tx.send(AppEvent::Paste(format!("{i}"))).unwrap();
    }
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        tx.send_blocking(AppEvent::Paste("last".into())).unwrap();
        done_tx.send(()).unwrap();
    });
    // Free exactly one slot: the blocked reader completes.
    assert!(rx.try_recv().is_ok());
    done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    reader.join().unwrap();
    // The backpressured paste lands after every earlier one.
    let events = drain(&rx);
    assert_eq!(events.len(), MAX_SEMANTIC_EVENTS);
    assert_eq!(pastes(&events).last().copied(), Some("last"));
}

#[test]
fn send_blocking_unblocks_when_the_receiver_closes() {
    let (tx, rx) = channel();
    for _ in 0..MAX_SEMANTIC_EVENTS {
        tx.send(key('l')).unwrap();
    }
    let reader = std::thread::spawn(move || tx.send_blocking(key('l')));
    drop(rx);
    assert!(reader.join().unwrap().is_err());
}

#[test]
fn disconnect_is_observed_only_after_every_admitted_event_drains() {
    let (tx, rx) = channel();
    tx.send(AppEvent::Paste("kept".into())).unwrap();
    let other = tx.clone();
    drop(tx);
    // A live clone keeps the channel open: drained-empty reads are
    // Empty, and the admitted event survives its original sender.
    let events = drain(&rx);
    assert_eq!(pastes(&events), vec!["kept"]);
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    drop(other);
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    assert!(matches!(
        rx.recv_timeout(std::time::Duration::from_millis(10)),
        Err(RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn recv_timeout_distinguishes_timeout_from_disconnect() {
    let (tx, rx) = channel();
    assert!(matches!(
        rx.recv_timeout(std::time::Duration::from_millis(10)),
        Err(RecvTimeoutError::Timeout)
    ));
    drop(tx);
    assert!(matches!(
        rx.recv_timeout(std::time::Duration::from_millis(10)),
        Err(RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn refused_admission_is_surfaced_in_the_editor_message() {
    let mut e = Editor::new(strop_core::Buffer::from_text("x\n"));
    e.session_policy = crate::session::SessionPolicy::Disabled;
    let (tx, rx) = channel();
    let filler = tx.clone();
    e.connect_events(tx);
    for _ in 0..MAX_SEMANTIC_EVENTS {
        filler.send(key('l')).unwrap();
    }
    assert!(filler.send(key('l')).is_err());
    // Handling the next event reports the refusal.
    let event = rx.try_recv().unwrap();
    e.handle_app_event(event);
    assert!(
        e.message
            .contains("event queue full — refused: 1×editor key"),
        "{}",
        e.message
    );
}

#[test]
fn shutdown_settles_owned_work_without_waiting_for_service_liveness() {
    let mut e = Editor::new(strop_core::Buffer::from_text("x\n"));
    e.session_policy = crate::session::SessionPolicy::Disabled;
    // A server stuck in start/handshake is finite owned work...
    e.lsp_servers.push(crate::editor::lsp::LspServer {
        id: strop_lsp::ServerId::new(6),
        client: None,
        rx: std::sync::mpsc::channel().1,
        ready: false,
    });
    assert!(e.async_pending(), "start/handshake counts as pending work");
    // ...but shutdown quiesces and closes: settle must not wait for the
    // service to become ready or to exit.
    e.finish_background_work();
    assert!(
        !e.async_pending(),
        "finishing stops waiting on service readiness"
    );
    assert!(e.take_shutdown_error().is_none());
}
