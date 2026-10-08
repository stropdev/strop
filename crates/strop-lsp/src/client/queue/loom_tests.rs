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

fn request() -> WireJob {
    let mut arena: strop_core::id::Arena<strop_core::id::DocumentKind, ()> =
        strop_core::id::Arena::default();
    let document = arena.try_insert(()).unwrap();
    WireJob::Request(PendingRequest {
        stamp: crate::protocol::RequestStamp {
            request: crate::protocol::RequestId::new(0),
            server: ServerId::new(1),
            document,
            revision: strop_core::id::BufferRevision::new(0),
        },
        input: crate::protocol::RequestInput {
            document,
            revision: strop_core::id::BufferRevision::new(0),
            path: std::path::PathBuf::from("/workspace/a.rs"),
            line: strop_core::id::LineIndex::new(0),
            byte_col: strop_core::id::ByteColumn::new(0),
            line_text: crate::FrozenLine::from(""),
            kind: crate::protocol::RequestKind::Hover,
            rename_to: None,
            rename_files: None,
        },
        tab_width: None,
    })
}

/// One loom thread's share of the sender count: drop discipline is
/// what WireTx::drop does.
fn release(shared: &Shared) {
    let mut state = lock_state(shared);
    state.senders = state.senders.saturating_sub(1);
    if state.senders == 0 {
        drop(state);
        shared.available.notify_all();
    }
}

/// Versions allocate in admission order (production: next_version
/// under the sync lock that also admits), so the campaign pulls the
/// version and admits under one instrumented lock.
fn admit_versioned(
    alloc: &loom::sync::Mutex<i32>,
    shared: &Shared,
    job: impl FnOnce(i32) -> WireJob,
) -> (Admission, i32) {
    let mut next = alloc.lock().unwrap_or_else(|p| p.into_inner());
    *next += 1;
    let version = *next;
    let outcome = admit(shared, job(version));
    drop(next);
    (outcome, version)
}

#[test]
fn loom_fifo_barrier_drain_disconnect() {
    loom::model(|| {
        let shared = SharedArc::new(Shared {
            state: QueueLock::new(QueueState {
                senders: 2,
                accepting: true,
                ..QueueState::default()
            }),
            available: QueueCondvar::new(),
        });
        let alloc = loom::sync::Arc::new(loom::sync::Mutex::new(0));
        let producer_shared = shared.clone();
        let producer_alloc = alloc.clone();
        let producer = loom::thread::spawn(move || {
            let mut admissions = Vec::new();
            for text in ["one", "two", "three"] {
                admissions.push(admit_versioned(&producer_alloc, &producer_shared, |v| {
                    change("a.rs", v, text)
                }));
            }
            release(&producer_shared);
            admissions
        });
        let request_admission = admit(&shared, request());
        let (change_admission, after_request) =
            admit_versioned(&alloc, &shared, |v| change("a.rs", v, "four"));
        release(&shared);
        let mut drained = Vec::new();
        let mut retired = Vec::new();
        while let Some(job) = next_job(&shared, &mut retired) {
            drained.push(job);
        }
        let theirs = producer.join().unwrap();
        let admissions: Vec<Admission> = theirs
            .iter()
            .map(|(a, _)| *a)
            .chain([request_admission, change_admission])
            .collect();
        assert!(
            admissions
                .iter()
                .all(|a| *a == Admission::Admitted || *a == Admission::Coalesced),
            "nothing refuses within the shrunken bound: {admissions:?}"
        );
        let admitted = admissions
            .iter()
            .filter(|a| **a == Admission::Admitted)
            .count();
        assert_eq!(
            drained.len(),
            admitted,
            "every admitted job drained before disconnect; \
                 coalesced ones replaced their slot"
        );
        let mut last_version = 0;
        let mut request_seen = false;
        for job in &drained {
            match job {
                WireJob::Open { version, .. } | WireJob::Change { version, .. } => {
                    let v = version.get();
                    assert!(v > last_version, "versions strictly increase");
                    if request_seen {
                        assert!(
                            v >= after_request,
                            "a pre-request version never drains after \
                                 the barrier"
                        );
                    } else {
                        assert!(
                            v < after_request,
                            "a post-request version never drains before \
                                 the barrier"
                        );
                    }
                    last_version = v;
                }
                WireJob::Request(_) | WireJob::Completion(_) => request_seen = true,
                WireJob::Close { .. }
                | WireJob::WorkspaceSymbols { .. }
                | WireJob::DidRenameFiles { .. } => {}
            }
        }
        assert!(request_seen, "the request drained");
        assert_eq!(last_version, 4, "the newest snapshot reached the wire");
    });
}
