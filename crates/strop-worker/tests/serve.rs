//! Raw-protocol tests against `strop_worker::serve::run` (WK04): scripted
//! frames exercise the session edges a client library never produces —
//! pre-handshake noise, a second hello, stale session stamps, and
//! disconnect — over real pipes and the real codec.

use strop_worker_protocol::codec::{self, Incoming};
use strop_worker_protocol::frame::{self, FrameDecoder};
use strop_worker_protocol::{
    ClientMessage, EndpointInfo, ProtocolError, Refusal, Request, RequestId, WorkerMessage,
    PROTOCOL_VERSION,
};

fn endpoint() -> EndpointInfo {
    EndpointInfo {
        name: "test".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        build: None,
        target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
    }
}

struct Scripted {
    writer: std::io::PipeWriter,
    reader: std::io::PipeReader,
    decoder: FrameDecoder,
    server: Option<std::thread::JoinHandle<()>>,
}

impl Scripted {
    fn start() -> Self {
        let (client_read, worker_write) = std::io::pipe().unwrap();
        let (worker_read, client_write) = std::io::pipe().unwrap();
        let server = std::thread::spawn(move || {
            strop_worker::serve::run(worker_read, worker_write).unwrap();
        });
        let mut scripted = Self {
            writer: client_write,
            reader: client_read,
            decoder: FrameDecoder::default(),
            server: Some(server),
        };
        scripted.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            client: endpoint(),
        });
        scripted
    }

    fn send(&mut self, message: &ClientMessage) {
        codec::write_envelope(&mut self.writer, message).unwrap();
    }

    fn send_body(&mut self, body: &[u8]) {
        frame::write_frame(&mut self.writer, body).unwrap();
    }

    fn recv(&mut self) -> WorkerMessage {
        let body = frame::read_frame(&mut self.reader, &mut self.decoder)
            .unwrap()
            .unwrap();
        match codec::decode_body(&body).unwrap() {
            Incoming::Envelope(message) => message,
            Incoming::Chunk(_) => panic!("unexpected chunk"),
        }
    }

    fn welcome(mut self) -> (Self, strop_worker_protocol::Session) {
        match self.recv() {
            WorkerMessage::Welcome { session, .. } => (self, session),
            other => panic!("expected welcome, got {other:?}"),
        }
    }

    fn finish(mut self) {
        drop(self.writer);
        self.server.take().unwrap().join().unwrap();
    }
}

#[test]
fn eof_before_hello_exits_cleanly() {
    let (client_read, worker_write) = std::io::pipe().unwrap();
    let (worker_read, client_write) = std::io::pipe().unwrap();
    let server = std::thread::spawn(move || strop_worker::serve::run(worker_read, worker_write));
    // Closing the client's writer is the disconnect: EOF at a frame
    // boundary is a clean worker exit.
    drop(client_write);
    server.join().unwrap().unwrap();
    drop(client_read);
}

#[test]
fn noise_before_hello_is_a_typed_error() {
    let mut scripted = Scripted::start_raw();
    scripted.recv_error_is(|error| matches!(error, ProtocolError::Unexpected { .. }));
}

impl Scripted {
    fn start_raw() -> Self {
        let (client_read, worker_write) = std::io::pipe().unwrap();
        let (worker_read, client_write) = std::io::pipe().unwrap();
        let server = std::thread::spawn(move || {
            strop_worker::serve::run(worker_read, worker_write).unwrap();
        });
        let mut scripted = Self {
            writer: client_write,
            reader: client_read,
            decoder: FrameDecoder::default(),
            server: Some(server),
        };
        // A request where the hello belongs.
        let request = ClientMessage::Request {
            session: strop_worker_protocol::Session {
                incarnation: 1,
                lease: strop_worker_protocol::LeaseId(1),
            },
            id: RequestId(0),
            body: Box::new(Request::Health),
        };
        scripted.send(&request);
        scripted
    }

    fn recv_error_is(&mut self, check: impl Fn(&ProtocolError) -> bool) {
        match self.recv() {
            WorkerMessage::Error { error, .. } => assert!(check(&error), "unexpected {error:?}"),
            other => panic!("expected error, got {other:?}"),
        }
    }
}

#[test]
fn a_second_hello_is_refused_and_closes() {
    let (mut scripted, _session) = Scripted::start().welcome();
    scripted.send(&ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        client: endpoint(),
    });
    scripted.recv_error_is(|error| matches!(error, ProtocolError::Unexpected { .. }));
    scripted.finish();
}

#[test]
fn a_stale_incarnation_is_a_typed_refusal() {
    let (mut scripted, session) = Scripted::start().welcome();
    let stale = strop_worker_protocol::Session {
        incarnation: session.incarnation.wrapping_add(1),
        lease: session.lease,
    };
    scripted.send(&ClientMessage::Request {
        session: stale,
        id: RequestId(7),
        body: Box::new(Request::Health),
    });
    match scripted.recv() {
        WorkerMessage::Result {
            id,
            outcome:
                strop_worker_protocol::ResultOutcome::Refused {
                    refusal: Refusal::WrongIncarnation { current },
                },
        } => {
            assert_eq!(id, RequestId(7));
            assert_eq!(current, session.incarnation);
        }
        other => panic!("expected wrong-incarnation refusal, got {other:?}"),
    }
    // The live session still answers afterwards.
    scripted.send(&ClientMessage::Request {
        session,
        id: RequestId(8),
        body: Box::new(Request::Health),
    });
    assert!(matches!(
        scripted.recv(),
        WorkerMessage::Result {
            outcome: strop_worker_protocol::ResultOutcome::Healthy,
            ..
        }
    ));
    scripted.finish();
}

#[test]
fn garbage_body_is_a_decode_error_and_close() {
    let (mut scripted, _session) = Scripted::start().welcome();
    scripted.send_body(b"\x00{not json");
    scripted.recv_error_is(|error| matches!(error, ProtocolError::Decode { .. }));
    scripted.finish();
}
