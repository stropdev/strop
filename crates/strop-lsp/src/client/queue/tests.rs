//! Admission-policy unit tests: coalescing legality, barrier
//! preservation and the count/byte bounds. These exercise the queue
//! state directly; the worker is a plain FIFO drain over it.
use super::*;

fn uri(name: &str) -> lt::Url {
    lt::Url::parse(&format!("file:///workspace/{name}")).unwrap()
}

fn change(name: &str, version: i32, text: &str) -> WireJob {
    WireJob::Change {
        uri: uri(name),
        version: WireVersion::new(version),
        text: Rope::from_str(text),
    }
}

fn document_id() -> strop_core::id::DocumentId {
    let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
        strop_core::id::Arena::default();
    arena.try_insert(()).unwrap()
}

fn request() -> WireJob {
    WireJob::Request(PendingRequest {
        stamp: crate::protocol::RequestStamp {
            request: crate::protocol::RequestId::new(0),
            server: ServerId::new(1),
            document: document_id(),
            revision: strop_core::id::BufferRevision::new(0),
        },
        input: crate::protocol::RequestInput {
            document: document_id(),
            revision: strop_core::id::BufferRevision::new(0),
            path: std::path::PathBuf::from("/workspace/a.rs"),
            line: strop_core::id::LineIndex::new(0),
            byte_col: strop_core::id::ByteColumn::new(0),
            line_text: crate::FrozenLine::from(""),
            kind: crate::protocol::RequestKind::Hover,
            rename_to: None,
        },
        tab_width: None,
    })
}

fn queue() -> Shared {
    Shared {
        state: QueueLock::new(QueueState {
            senders: 1,
            accepting: true,
            ..QueueState::default()
        }),
        available: QueueCondvar::new(),
    }
}

fn queued_versions(shared: &Shared, name: &str) -> Vec<i32> {
    lock_state(shared)
        .jobs
        .iter()
        .filter_map(|job| match job {
            WireJob::Change {
                uri: u, version, ..
            } if *u == uri(name) => Some(version.get()),
            _ => None,
        })
        .collect()
}

#[test]
fn superseded_unsent_changes_coalesce_without_a_barrier() {
    let shared = queue();
    assert_eq!(
        admit(&shared, change("a.rs", 1, "one")),
        Admission::Admitted
    );
    assert_eq!(
        admit(&shared, change("b.rs", 1, "other")),
        Admission::Admitted
    );
    // No barrier between: the newest snapshot replaces the oldest
    // unsent one for the same document, in place.
    assert_eq!(
        admit(&shared, change("a.rs", 2, "two")),
        Admission::Coalesced
    );
    assert_eq!(queued_versions(&shared, "a.rs"), vec![2]);
    assert_eq!(queued_versions(&shared, "b.rs"), vec![1]);
    let state = lock_state(&shared);
    assert_eq!(state.jobs.len(), 2);
    assert_eq!(
        state.snapshot_bytes,
        Rope::from_str("two").len_bytes() + Rope::from_str("other").len_bytes()
    );
}

#[test]
fn an_admitted_request_bars_coalescing_the_version_it_observes() {
    let shared = queue();
    admit(&shared, change("a.rs", 1, "one"));
    admit(&shared, request());
    // The request is answered against version 1: version 1 must
    // reach the wire, so version 2 queues behind it.
    assert_eq!(
        admit(&shared, change("a.rs", 2, "two")),
        Admission::Admitted
    );
    assert_eq!(queued_versions(&shared, "a.rs"), vec![1, 2]);
    // Versions 1 and 2 are BOTH pre-request... no: only version 2
    // trails the request. A third change may coalesce into 2.
    assert_eq!(
        admit(&shared, change("a.rs", 3, "three")),
        Admission::Coalesced
    );
    assert_eq!(queued_versions(&shared, "a.rs"), vec![1, 3]);
}

#[test]
fn a_close_for_the_document_bars_coalescing_across_it() {
    let shared = queue();
    admit(&shared, change("a.rs", 1, "one"));
    admit(&shared, WireJob::Close { uri: uri("a.rs") });
    // A change after the close (reopen churn) must not merge past
    // the lifecycle barrier.
    assert_eq!(
        admit(&shared, change("a.rs", 2, "two")),
        Admission::Admitted
    );
    assert_eq!(queued_versions(&shared, "a.rs"), vec![1, 2]);
    assert_eq!(lock_state(&shared).jobs.len(), 3);
}

#[test]
fn the_job_count_bound_refuses_visibly() {
    let shared = queue();
    for i in 0..MAX_QUEUED_JOBS {
        let admitted = admit(
            &shared,
            WireJob::Close {
                uri: uri(&format!("{i}.rs")),
            },
        );
        assert_eq!(admitted, Admission::Admitted);
    }
    assert_eq!(
        admit(
            &shared,
            WireJob::Close {
                uri: uri("overflow.rs"),
            },
        ),
        Admission::Refused
    );
}

#[test]
fn the_snapshot_byte_bound_refuses_changes_but_not_lifecycle() {
    let shared = queue();
    // A lone oversize snapshot into an empty queue admits (the bound
    // limits retention, not single-message size).
    let big = "x".repeat(MAX_QUEUED_SNAPSHOT_BYTES + 1);
    assert_eq!(
        admit(&shared, change("big.rs", 1, &big)),
        Admission::Admitted
    );
    // With a snapshot retained, any further change exceeds the byte
    // bound and is refused...
    assert_eq!(admit(&shared, change("b.rs", 1, "b")), Admission::Refused);
    // ...but coalescing the retained snapshot itself is never
    // byte-gated: replacement cannot grow retention past one
    // snapshot per document.
    assert_eq!(
        admit(&shared, change("big.rs", 2, "small")),
        Admission::Coalesced
    );
    // Lifecycle frames and requests still flow with the lane at
    // the byte bound (they carry no snapshots).
    assert_eq!(
        admit(&shared, WireJob::Close { uri: uri("big.rs") },),
        Admission::Admitted
    );
    assert_eq!(admit(&shared, request()), Admission::Admitted);
    // The close is a barrier for its document: a later change must
    // not merge past it — both versions stay queued in order.
    assert_eq!(
        admit(&shared, change("big.rs", 3, "later")),
        Admission::Admitted
    );
    assert_eq!(queued_versions(&shared, "big.rs"), vec![2, 3]);
}

#[test]
fn coalescing_tracks_bytes_so_the_bound_reflects_what_is_retained() {
    let shared = queue();
    let first = "a".repeat(MAX_QUEUED_SNAPSHOT_BYTES - 4);
    admit(&shared, change("a.rs", 1, &first));
    // The live queue releases its old byte charge; the superseded rope
    // remains in the separately bounded worker-retirement lane.
    assert_eq!(
        admit(&shared, change("a.rs", 2, "tiny")),
        Admission::Coalesced
    );
    assert_eq!(
        admit(&shared, change("b.rs", 1, "fits now")),
        Admission::Admitted
    );
}

#[test]
fn repeated_coalescing_is_bounded_until_worker_retirement_returns_capacity() {
    let shared = queue();
    admit(&shared, change("a.rs", 1, "initial"));
    for version in 2..=MAX_RETIRED_SNAPSHOTS as i32 + 1 {
        assert_eq!(
            admit(&shared, change("a.rs", version, "next")),
            Admission::Coalesced
        );
    }
    let newest = MAX_RETIRED_SNAPSHOTS as i32 + 2;
    assert_eq!(
        admit(&shared, change("a.rs", newest, "not admitted")),
        Admission::Refused
    );
    assert_eq!(queued_versions(&shared, "a.rs"), vec![newest - 1]);
    let mut retired = Vec::with_capacity(MAX_RETIRED_SNAPSHOTS);
    retire_snapshots(&shared, &mut retired);
    assert_eq!(
        admit(&shared, change("a.rs", newest, "admitted")),
        Admission::Coalesced
    );
    assert_eq!(queued_versions(&shared, "a.rs"), vec![newest]);
}

#[test]
fn coalescing_cannot_grow_a_multi_document_queue_beyond_its_byte_bound() {
    let shared = queue();
    admit(&shared, change("a.rs", 1, "tiny"));
    admit(
        &shared,
        change("b.rs", 1, &"b".repeat(MAX_QUEUED_SNAPSHOT_BYTES - 4)),
    );
    assert_eq!(
        admit(&shared, change("a.rs", 2, "would grow")),
        Admission::Refused
    );
    assert_eq!(queued_versions(&shared, "a.rs"), vec![1]);
    assert_eq!(
        lock_state(&shared).snapshot_bytes,
        MAX_QUEUED_SNAPSHOT_BYTES
    );
    assert_eq!(
        admit(&shared, WireJob::Close { uri: uri("a.rs") }),
        Admission::Admitted
    );
}
