//! Queue, budget and cancellation behavior under production synchronization.

use super::*;
use strop_worker_protocol::ResultOutcome;

fn budgets() -> Budgets {
    Budgets {
        max_pending: 4,
        control_reserve: 2,
        max_reads: 1,
        max_execs: 1,
        data_chunks: 2,
        control_frames: 3,
    }
}

fn slot() -> (CancelToken, CancelHandle) {
    CancelToken::standalone()
}

fn chunk(stream: u64, sequence: u64, last: bool) -> StreamChunk {
    StreamChunk {
        stream: StreamId(stream),
        sequence,
        last,
        bytes: Vec::new(),
    }
}

fn control() -> WorkerMessage {
    WorkerMessage::Result {
        id: RequestId(7),
        outcome: ResultOutcome::Healthy,
    }
}

#[test]
fn control_reserve_admits_control_when_standard_is_full() {
    let scheduler = Scheduler::new(budgets());
    // Two standard requests fill the non-reserved slots (4 - 2).
    let (_t1, h1) = slot();
    let (_t2, h2) = slot();
    scheduler
        .admit(RequestId(1), RequestClass::Standard, h1)
        .unwrap();
    scheduler
        .admit(RequestId(2), RequestClass::Standard, h2)
        .unwrap();
    // A third standard/bulk request is a typed refusal...
    let (_t3, h3) = slot();
    let refused = scheduler.admit(RequestId(3), RequestClass::Standard, h3);
    assert!(matches!(refused, Err(Refusal::Busy { .. })));
    // ...but control still admits into the reserve.
    let (_t4, h4) = slot();
    scheduler
        .admit(RequestId(4), RequestClass::Control, h4)
        .unwrap();
    assert_eq!(scheduler.outstanding(), 3);
}

#[test]
fn bulk_reads_draw_from_their_own_budget() {
    let scheduler = Scheduler::new(budgets());
    let (_t1, h1) = slot();
    scheduler
        .admit(RequestId(1), RequestClass::Bulk, h1)
        .unwrap();
    let (_t2, h2) = slot();
    let refused = scheduler.admit(RequestId(2), RequestClass::Bulk, h2);
    assert!(matches!(refused, Err(Refusal::Busy { .. })));
    // A settled read frees the budget for the next one.
    scheduler.settle(RequestId(1));
    let (_t3, h3) = slot();
    scheduler
        .admit(RequestId(3), RequestClass::Bulk, h3)
        .unwrap();
}

#[test]
fn exec_slots_are_atomic_and_release_exactly() {
    let scheduler = Scheduler::new(budgets());
    scheduler.acquire_exec().unwrap();
    assert!(matches!(
        scheduler.acquire_exec(),
        Err(Refusal::Busy { .. })
    ));
    scheduler.release_exec();
    scheduler.acquire_exec().unwrap();
}

#[test]
fn control_pops_before_queued_data_and_data_stays_fifo() {
    let scheduler = Scheduler::new(budgets());
    scheduler.push_chunk(chunk(2, 0, false), None);
    scheduler.push_chunk(chunk(2, 1, false), None);
    scheduler.push_control(control());
    // The control frame overtakes already-queued data (cross-stream
    // reordering is legal; per-stream order is not touched).
    assert!(matches!(scheduler.pop(), Some(Outbound::Control(_))));
    assert!(matches!(
        scheduler.pop(),
        Some(Outbound::Chunk(c)) if c.sequence == 0
    ));
    assert!(matches!(
        scheduler.pop(),
        Some(Outbound::Chunk(c)) if c.sequence == 1
    ));
}

#[test]
fn exit_barrier_waits_for_written_bytes_not_just_a_popped_chunk() {
    use std::sync::Arc;
    let scheduler = Arc::new(Scheduler::new(budgets()));
    let stop = Arc::new(AtomicBool::new(false));
    scheduler.push_chunk(chunk(2, 0, true), None);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let waiter = {
        let scheduler = Arc::clone(&scheduler);
        std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            done_tx.send(scheduler.flush_data(&stop)).unwrap();
        })
    };
    ready_rx.recv().unwrap();
    assert!(matches!(scheduler.pop(), Some(Outbound::Chunk(c)) if c.last));
    assert!(
        done_rx.try_recv().is_err(),
        "pop is not a completed wire write"
    );
    scheduler.chunk_written();
    assert!(done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap());
    waiter.join().unwrap();
}

#[test]
fn opening_envelope_precedes_its_first_chunk_under_control_pressure() {
    let mut limits = budgets();
    limits.control_frames = 9;
    let scheduler = Scheduler::new(limits);
    for _ in 0..8 {
        assert!(scheduler.push_control(control()));
    }
    assert!(scheduler.push_control(WorkerMessage::Result {
        id: RequestId(9),
        outcome: ResultOutcome::ExecStarted {
            exec: strop_worker_protocol::ExecId(1),
            stdin: None,
            stdout: StreamId(4),
            stderr: StreamId(6),
        },
    }));
    scheduler.push_chunk(chunk(4, 0, false), None);
    for _ in 0..9 {
        assert!(matches!(scheduler.pop(), Some(Outbound::Control(_))));
    }
    assert!(matches!(scheduler.pop(), Some(Outbound::Chunk(c)) if c.stream == StreamId(4)));
}

#[test]
fn terminal_chunks_never_queue_behind_a_full_data_lane() {
    let scheduler = Scheduler::new(budgets());
    scheduler.push_chunk(chunk(2, 0, false), None);
    scheduler.push_chunk(chunk(2, 1, false), None);
    assert_eq!(scheduler.lanes().1, 2);
    // The lane is full; the stream's EOF still lands.
    scheduler.push_chunk(chunk(2, 2, true), None);
    assert_eq!(scheduler.queued_data_sequences(StreamId(2)), vec![0, 1, 2]);
}

#[test]
fn cancel_abandons_a_full_lane_push() {
    use std::sync::Arc;
    let scheduler = Arc::new(Scheduler::new(budgets()));
    let (token, handle) = slot();
    scheduler
        .admit(RequestId(1), RequestClass::Bulk, handle)
        .unwrap();
    scheduler.push_chunk(chunk(2, 0, false), None);
    scheduler.push_chunk(chunk(2, 1, false), None);
    let producer = {
        let scheduler = Arc::clone(&scheduler);
        std::thread::spawn(move || scheduler.push_chunk(chunk(2, 2, false), Some(&token)))
    };
    // Cancel may race with the producer's park; either order must
    // abandon the full-lane push and allow an honest stream ending.
    assert!(scheduler.cancel(RequestId(1)));
    assert_eq!(producer.join().unwrap(), PushChunk::Cancelled);
    scheduler.push_chunk(chunk(2, 2, true), None);
    assert_eq!(scheduler.queued_data_sequences(StreamId(2)), vec![0, 1, 2]);
    scheduler.settle(RequestId(1));
    assert_eq!(scheduler.outstanding(), 0);
}

#[test]
fn halt_abandons_a_full_lane_push() {
    use std::sync::Arc;
    let scheduler = Arc::new(Scheduler::new(budgets()));
    scheduler.push_chunk(chunk(2, 0, false), None);
    scheduler.push_chunk(chunk(2, 1, false), None);
    let producer = {
        let scheduler = Arc::clone(&scheduler);
        std::thread::spawn(move || scheduler.push_chunk(chunk(2, 2, false), None))
    };
    scheduler.halt();
    assert_eq!(producer.join().unwrap(), PushChunk::Halted);
    assert!(!scheduler.push_control(control()));
    // The writer drains what was queued, then exits.
    assert!(matches!(scheduler.pop(), Some(Outbound::Chunk(_))));
    assert!(matches!(scheduler.pop(), Some(Outbound::Chunk(_))));
    assert!(scheduler.pop().is_none());
}

#[test]
fn settle_of_an_unknown_id_is_a_bug_not_a_race() {
    let scheduler = Scheduler::new(budgets());
    let (_t, handle) = slot();
    scheduler
        .admit(RequestId(1), RequestClass::Standard, handle)
        .unwrap();
    scheduler.settle(RequestId(1));
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scheduler.settle(RequestId(1));
    }));
    assert!(panicked.is_err());
}
