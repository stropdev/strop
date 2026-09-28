use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use strop_core::id::{Arena, BufferRevision, DocumentKind};

struct FlushGate(Arc<AtomicBool>);
impl AsyncWrite for FlushGate {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.0.load(Ordering::Acquire) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

#[test]
fn another_frame_and_unflushed_bytes_cannot_release_notification_ownership() {
    let outbound = Arc::new(Outbound::default());
    let ticket = outbound.begin_notification(Method::Change).unwrap();
    let open = Arc::new(AtomicBool::new(true));
    let mut writer = FramedWriter::new(FlushGate(open.clone()), outbound.clone());
    let mut context = Context::from_waker(std::task::Waker::noop());
    let unrelated = frame(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#);
    assert!(matches!(
        Pin::new(&mut writer).poll_write(&mut context, &unrelated),
        Poll::Ready(Ok(_))
    ));
    assert!(matches!(
        Pin::new(&mut writer).poll_flush(&mut context),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(
        outbound.begin_notification(Method::Close),
        Err(WriteRefusal::Pending)
    );

    open.store(false, Ordering::Release);
    let owned = frame(r#"{"jsonrpc":"2.0","method":"textDocument/didChange","params":{}}"#);
    for chunk in owned.chunks(3) {
        assert!(matches!(
            Pin::new(&mut writer).poll_write(&mut context, chunk),
            Poll::Ready(Ok(_))
        ));
    }
    assert!(matches!(
        Pin::new(&mut writer).poll_flush(&mut context),
        Poll::Pending
    ));
    assert_eq!(
        outbound.begin_notification(Method::Close),
        Err(WriteRefusal::Pending)
    );
    open.store(true, Ordering::Release);
    assert!(matches!(
        Pin::new(&mut writer).poll_flush(&mut context),
        Poll::Ready(Ok(()))
    ));
    assert!(outbound.wait(ticket, || {}));
    assert_eq!(outbound.begin_notification(Method::Close), Ok(ticket + 1));
}

#[test]
fn closing_or_aborting_never_attests_an_unwritten_notification() {
    let outbound = Outbound::default();
    let first = outbound.begin_notification(Method::Open).unwrap();
    outbound.abort(first);
    assert!(!outbound.wait(first, || {}));
    let second = outbound.begin_notification(Method::Change).unwrap();
    outbound.close();
    assert!(!outbound.wait(second, || {}));
    assert_eq!(
        outbound.begin_notification(Method::Close),
        Err(WriteRefusal::Closed)
    );
}

fn stamp(request: u64) -> RequestStamp {
    let mut documents: Arena<DocumentKind, ()> = Arena::default();
    RequestStamp {
        request: crate::RequestId::new(request),
        server: crate::ServerId::new(7),
        document: documents.try_insert(()).unwrap(),
        revision: BufferRevision::new(1),
    }
}

#[test]
fn a_split_numeric_identity_is_not_confused_with_the_logical_request_stamp() {
    let outbound = Outbound::default();
    let mut received = outbound.submitted(stamp(53), Method::Completion).unwrap();
    let body = r#"{"jsonrpc":"2.0","method":"textDocument/completion","id":12345,"params":{}}"#;
    let mut frames = Frames::default();
    frames
        .push(
            format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes(),
            &outbound,
        )
        .unwrap();
    let split = body.find("12345").unwrap() + 2;
    frames.push(&body.as_bytes()[..split], &outbound).unwrap();
    assert_eq!(
        received.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    frames.push(&body.as_bytes()[split..], &outbound).unwrap();
    assert_eq!(received.try_recv(), Ok(RpcId::Number(12345)));
}

#[test]
fn reordered_mapping_refuses_instead_of_cancelling_another_request() {
    let outbound = Outbound::default();
    let mut first = outbound.submitted(stamp(1), Method::Completion).unwrap();
    let mut second = outbound.submitted(stamp(2), Method::Resolve).unwrap();
    let wrong = frame(r#"{"jsonrpc":"2.0","id":5,"method":"completionItem/resolve","params":{}}"#);
    assert_eq!(
        Frames::default()
            .push(&wrong, &outbound)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(first.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    assert_eq!(second.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    outbound.close();
    assert_eq!(first.try_recv(), Err(oneshot::error::TryRecvError::Closed));
    assert_eq!(second.try_recv(), Err(oneshot::error::TryRecvError::Closed));
}

#[test]
fn encoded_request_counter_exhaustion_cannot_wrap_into_an_old_owner() {
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/hover","params":{{}}}}"#,
        i32::MAX - 1
    );
    assert_eq!(
        prefix::decode(body.as_bytes()).err().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn retirement_wakes_while_the_native_writer_is_still_blocked() {
    let outbound = Arc::new(Outbound::default());
    let ticket = outbound.begin_notification(Method::Change).unwrap();
    let (retired_tx, retired_rx) = std::sync::mpsc::channel();
    let waiting = outbound.clone();
    let worker = std::thread::spawn(move || {
        waiting.wait(ticket, || {
            retired_tx.send(()).unwrap();
        })
    });
    outbound.wake_retirement();
    let retired = retired_rx.recv_timeout(std::time::Duration::from_secs(5));
    outbound.close();
    assert!(
        !worker.join().unwrap(),
        "retirement is not a frame-write acknowledgment"
    );
    retired.expect("retirement must not wait for the blocked language server");
}
