//! Real SFTP subsystem over private pipes: reuse, queued cancellation and final
//! lease cleanup. No network, authentication configuration, HOME or sleeps.
use super::*;
use crate::test_support::in_worker;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use strop_core::worker::{self, CancelReason, Outcome};

fn subsystem() -> Option<PathBuf> {
    let path = [
        "/usr/lib/openssh/sftp-server",
        "/usr/lib/ssh/sftp-server",
        "/usr/libexec/sftp-server",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file());
    assert!(
        path.is_some() || std::env::var_os("STROP_REQUIRE_SSH_TESTS").is_none(),
        "the required SFTP subsystem is missing"
    );
    path
}
fn queued(
    lease: &ConnectionLease,
    token: &CancelToken,
    path: &Path,
) -> Receiver<Result<Reply, RemoteReadError>> {
    let (reply, receiver) = std::sync::mpsc::channel();
    lease
        .inner
        .jobs
        .send(Job {
            lease: lease.clone(),
            cancel: token.clone(),
            reply,
            request: Request::Read {
                location: RemoteFile::from_path(lease.endpoint().clone(), path.to_owned())
                    .unwrap()
                    .into(),
                selection: ReadSelection::Full,
            },
        })
        .unwrap_or_else(|_| panic!("actor admission closed"));
    receiver
}
fn expect_text(receiver: Receiver<Result<Reply, RemoteReadError>>, expected: &str) {
    match receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap()
    {
        Reply::Snapshot(snapshot) => {
            assert_eq!(snapshot.buffer.text(), expected);
            assert!(snapshot.buffer.readonly);
        }
        _ => panic!("read did not return a snapshot"),
    }
}

#[test]
fn queued_cancel_preserves_shared_stream_and_last_lease_reaps() {
    let Some(subsystem) = subsystem() else {
        return;
    };
    in_worker(move |token| {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("log");
        std::fs::write(&path, "native SFTP content\n").unwrap();
        let (jobs, receiver) = std::sync::mpsc::sync_channel(QUEUE_CAPACITY);
        let stop = Arc::new(StopSignal::new());
        let status = Arc::new(StatusCell::default());
        let inner = Arc::new(EndpointHandle {
            endpoint: RemoteEndpoint::parse("ssh://fixture").unwrap(),
            jobs,
            stop: stop.clone(),
            status: status.clone(),
        });
        let lease = ConnectionLease { inner };
        let starts = Arc::new(AtomicUsize::new(0));
        let spawned = starts.clone();
        let actor = Actor {
            receiver,
            stop: stop.clone(),
            status,
            spawner: Box::new(move || {
                spawned.fetch_add(1, Ordering::SeqCst);
                let mut command = Command::new(&subsystem);
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                command
            }),
        };
        // A real worker's cancellation token, cancelled while its job is queued.
        let (tokens, token_rx) = std::sync::mpsc::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let cancelled = worker::spawn(
            "queued-cancel-oracle",
            |_| {},
            move |cancel| {
                tokens.send(cancel).unwrap();
                let _ = waiting.recv();
                Outcome::Success(())
            },
        );
        let cancelled_token = token_rx.recv().unwrap();
        let first = queued(&lease, &token, &path);
        let skipped = queued(&lease, &cancelled_token, &path);
        let third = queued(&lease, &token, &path);
        cancelled.cancel(CancelReason::Superseded);
        release.send(()).unwrap();
        let thread = std::thread::spawn(move || actor_loop(actor));
        expect_text(first, "native SFTP content\n");
        assert!(
            matches!(skipped.recv_timeout(Duration::from_secs(10)).unwrap(), Err(error) if error.is_cancellation())
        );
        expect_text(third, "native SFTP content\n");
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "queued cancellation must not force reauthentication"
        );
        assert!(lease.is_connected());
        drop(lease);
        stop.wait().unwrap();
        thread.join().unwrap();
    });
}
