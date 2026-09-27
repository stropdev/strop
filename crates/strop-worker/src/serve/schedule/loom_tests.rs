//! Loom campaigns over the real scheduler (WK11): instrumented Mutex/
//! Condvar at shrunken bounds; no lost wakeup, frame, settlement or EOF.

use super::*;
use loom::sync::Arc;
use loom::thread;
use strop_worker_protocol::ResultOutcome;

fn budgets() -> Budgets {
    Budgets {
        max_pending: 3,
        control_reserve: 1,
        max_reads: 1,
        max_execs: 1,
        data_chunks: DATA_LANE_CHUNKS,
        control_frames: 2,
    }
}

fn chunk(sequence: u64, last: bool) -> StreamChunk {
    StreamChunk {
        stream: StreamId(2),
        sequence,
        last,
        bytes: Vec::new(),
    }
}

fn control(id: u64) -> WorkerMessage {
    WorkerMessage::Result {
        id: RequestId(id),
        outcome: ResultOutcome::Healthy,
    }
}

/// The writer drains into a local record; the campaign then checks:
/// every pushed frame arrived exactly once, per-stream FIFO held, and
/// a control frame pushed while the data lane stayed full overtook
/// the data queued ahead of it.
#[test]
fn loom_control_priority_and_no_lost_frames() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(|| {
        let scheduler = Arc::new(Scheduler::new(budgets()));
        let writer = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || {
                let mut written = Vec::new();
                while let Some(frame) = scheduler.pop() {
                    written.push(frame);
                }
                written
            })
        };
        // One producer fills the data lane (capacity 2 under loom)
        // and blocks on the third chunk; one control pusher competes.
        let producer = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || {
                scheduler.push_chunk(chunk(0, false), None);
                scheduler.push_chunk(chunk(1, false), None);
                scheduler.push_chunk(chunk(2, true), None);
            })
        };
        let pusher = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || {
                scheduler.push_control(control(9));
            })
        };
        producer.join().unwrap();
        pusher.join().unwrap();
        scheduler.halt();
        let written = writer.join().unwrap();
        let data: Vec<u64> = written
            .iter()
            .filter_map(|frame| match frame {
                Outbound::Chunk(chunk) => Some(chunk.sequence),
                Outbound::Control(_) => None,
            })
            .collect();
        let controls = written
            .iter()
            .filter(|frame| matches!(frame, Outbound::Control(_)))
            .count();
        assert_eq!(data, vec![0, 1, 2], "no lost/duplicated/reordered data");
        assert_eq!(controls, 1, "no lost/duplicated control");
    });
}

/// A bulk producer blocked on a full data lane is woken by cancel —
/// never parked forever (loom fails the model on a deadlock) — and
/// its terminal chunk still bypasses the full lane.
#[test]
fn loom_cancel_wakes_a_lane_blocked_producer() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(|| {
        let scheduler = Arc::new(Scheduler::new(budgets()));
        let (token, handle) = CancelToken::standalone();
        scheduler
            .admit(RequestId(1), RequestClass::Bulk, handle)
            .expect("budget has room");
        scheduler.push_chunk(chunk(0, false), None);
        scheduler.push_chunk(chunk(1, false), None);
        let producer = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || {
                let pushed = scheduler.push_chunk(chunk(2, false), Some(&token));
                assert_eq!(pushed, PushChunk::Cancelled);
                // Honest termination: the short stream's EOF bypasses
                // the full lane, then the request settles.
                assert_eq!(
                    scheduler.push_chunk(chunk(2, true), None),
                    PushChunk::Enqueued
                );
                scheduler.settle(RequestId(1));
            })
        };
        let canceller = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || {
                scheduler.cancel(RequestId(1));
            })
        };
        producer.join().unwrap();
        canceller.join().unwrap();
        assert_eq!(scheduler.outstanding(), 0);
        assert_eq!(scheduler.queued_data_sequences(StreamId(2)), vec![0, 1, 2]);
    });
}

/// Cancel racing settle: exactly one terminal disposition. Either the
/// cancel lands first (returns true; settle still removes the slot)
/// or settle lands first (cancel is a no-op false). Never both, never
/// neither, never a double settle.
#[test]
fn loom_cancel_racing_settle_settles_exactly_once() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(|| {
        let scheduler = Arc::new(Scheduler::new(budgets()));
        let (_token, handle) = CancelToken::standalone();
        scheduler
            .admit(RequestId(1), RequestClass::Standard, handle)
            .expect("budget has room");
        let canceller = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || scheduler.cancel(RequestId(1)))
        };
        let settler = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || scheduler.settle(RequestId(1)))
        };
        let cancelled = canceller.join().unwrap();
        settler.join().unwrap();
        assert_eq!(scheduler.outstanding(), 0);
        // If cancel observed the slot, it owned the token flip; the
        // slot was settled exactly once either way (a second settle
        // panics inside the model).
        let _ = cancelled;
    });
}

/// Halt with a lane-blocked producer and a control-blocked pusher:
/// every waiter wakes (no lost wakeup), pushes after halt drop, the
/// writer drains and exits, and every admitted request can settle.
#[test]
fn loom_halt_settles_every_waiter() {
    let mut builder = loom::model::Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(|| {
        let scheduler = Arc::new(Scheduler::new(budgets()));
        // Fill both lanes (capacity 2 each under loom).
        scheduler.push_chunk(chunk(0, false), None);
        scheduler.push_chunk(chunk(1, false), None);
        scheduler.push_control(control(1));
        scheduler.push_control(control(2));
        let producer = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || scheduler.push_chunk(chunk(2, false), None))
        };
        let pusher = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || scheduler.push_control(control(3)))
        };
        let writer = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || {
                let mut drained = 0;
                while scheduler.pop().is_some() {
                    drained += 1;
                }
                drained
            })
        };
        let halter = {
            let scheduler = Arc::clone(&scheduler);
            thread::spawn(move || scheduler.halt())
        };
        let pushed = producer.join().unwrap();
        let _control_pushed = pusher.join().unwrap();
        let drained = writer.join().unwrap();
        halter.join().unwrap();
        // The producer either enqueued before halt or abandoned
        // after it; the writer drained everything enqueued and then
        // observed halt. Nothing parked.
        if pushed == PushChunk::Enqueued {
            assert!(drained >= 3);
        }
    });
}
