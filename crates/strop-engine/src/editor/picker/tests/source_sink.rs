#![cfg(strop_loom)]

use super::super::*;
use crate::editor::events::{AppEvent, TryRecvError, MAX_SEMANTIC_EVENTS};
use strop_core::worker::{Outcome, Ticket, WorkerId};
use strop_picker::PickerMsg;

#[test]
fn loom_source_sink_finished_survives_a_full_lane() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(|| {
        let ticket = Ticket {
            request: WorkerId::new(1),
            key: PickerKey {
                picker: PickerId(WorkerId::new(2)),
                cwd: "/".into(),
            },
        };
        let (tx, rx) = crate::editor::events::channel();
        let sink = Editor::app_source_sink(tx.clone(), ticket.clone());
        for _ in 0..MAX_SEMANTIC_EVENTS {
            tx.send(AppEvent::QuitIntent).unwrap();
        }
        assert!(tx.send(AppEvent::QuitIntent).is_err());

        let producer =
            loom::thread::spawn(move || sink.send(PickerMsg::Finished(Outcome::Success(()))));
        drop(tx);
        let mut quit_events = 0usize;
        let mut finished = None;
        loop {
            match rx.try_recv() {
                Ok(AppEvent::QuitIntent) => quit_events += 1,
                Ok(AppEvent::Picker(event)) => finished = Some(event),
                Ok(_) => panic!("unexpected event"),
                Err(TryRecvError::Empty) => loom::thread::yield_now(),
                Err(TryRecvError::Disconnected) => break,
            }
        }
        assert!(
            producer.join().unwrap(),
            "the source must see terminal delivery accepted"
        );
        assert_eq!(quit_events, MAX_SEMANTIC_EVENTS);
        let finished = finished.expect("Finished was stranded");
        assert_eq!(finished.ticket, ticket);
        assert!(matches!(
            &finished.msg,
            PickerMsg::Finished(Outcome::Success(()))
        ));
    });
}
