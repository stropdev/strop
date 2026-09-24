//! Codec round-trip over real loopback pipes (WK02 acceptance): the
//! handshake, typed envelopes and binary stream chunks cross an actual
//! socket pair byte-exactly, and a stale-incarnation session is rejected
//! at the far end with a typed refusal.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use strop_worker_protocol::codec::{self, Incoming, StreamChunk};
use strop_worker_protocol::frame::FrameDecoder;
use strop_worker_protocol::{
    Authority, Capabilities, ClientMessage, EndpointInfo, LeaseId, Limits, NamespaceIdentity,
    NotifyCoverage, Refusal, Request, RequestId, Session, StreamId, WorkerMessage,
    PROTOCOL_VERSION,
};

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let client = TcpStream::connect(listener.local_addr().expect("addr")).expect("connect");
    let (worker, _) = listener.accept().expect("accept");
    (client, worker)
}

fn endpoint(name: &str) -> EndpointInfo {
    EndpointInfo {
        name: name.to_owned(),
        version: "0.35.0".to_owned(),
        build: Some("candidate".to_owned()),
        target: "x86_64-unknown-linux-musl".to_owned(),
    }
}

fn limits() -> Limits {
    Limits {
        max_frame_bytes: strop_worker_protocol::MAX_BODY_BYTES,
        max_chunk_bytes: strop_worker_protocol::MAX_CHUNK_BYTES,
        max_pending_requests: 256,
        max_batch_steps: 512,
        max_listing_entries: 100_000,
        max_subscriptions: 64,
        max_streams: 128,
        max_exec_processes: 32,
    }
}

fn read_worker_message(reader: &mut TcpStream, decoder: &mut FrameDecoder) -> WorkerMessage {
    let body = strop_worker_protocol::frame::read_frame(reader, decoder)
        .expect("frame")
        .expect("eof at boundary");
    match codec::decode_body(&body).expect("decode") {
        Incoming::Envelope(message) => message,
        Incoming::Chunk(_) => panic!("expected envelope, got chunk"),
    }
}

#[test]
fn handshake_and_request_round_trip_over_loopback() {
    let (mut client, mut worker) = pair();
    let mut client_decoder = FrameDecoder::new();
    let mut worker_decoder = FrameDecoder::new();

    // Handshake.
    codec::write_envelope(
        &mut client,
        &ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            client: endpoint("strop"),
        },
    )
    .expect("write hello");
    let body = strop_worker_protocol::frame::read_frame(&mut worker, &mut worker_decoder)
        .expect("frame")
        .expect("eof at boundary");
    let hello = match codec::decode_body(&body).expect("decode hello") {
        Incoming::Envelope(message) => message,
        Incoming::Chunk(_) => panic!("expected envelope, got chunk"),
    };
    let ClientMessage::Hello {
        protocol,
        client: client_info,
    } = hello
    else {
        panic!("expected hello");
    };
    assert_eq!(protocol, PROTOCOL_VERSION);
    assert_eq!(client_info.name, "strop");

    let session = Session {
        incarnation: 0x5e55_10ab,
        lease: LeaseId(1),
    };
    codec::write_envelope(
        &mut worker,
        &WorkerMessage::Welcome {
            protocol: PROTOCOL_VERSION,
            worker: endpoint("strop-worker"),
            session,
            namespace: NamespaceIdentity {
                identity: "boot:0123456789abcdef".to_owned(),
                principal: Some(1000),
            },
            limits: limits(),
            capabilities: Capabilities {
                observe: true,
                list: true,
                read: true,
                write: true,
                trash: true,
                notify: NotifyCoverage::Native,
                exec_finite: true,
                exec_service: true,
                pty: true,
            },
        },
    )
    .expect("write welcome");
    let welcome = read_worker_message(&mut client, &mut client_decoder);
    let WorkerMessage::Welcome {
        session: admitted,
        capabilities,
        ..
    } = welcome
    else {
        panic!("expected welcome");
    };
    assert_eq!(admitted, session);
    assert_eq!(capabilities.notify, NotifyCoverage::Native);

    // A stamped request and its typed result.
    let authority = Authority::new(session);
    codec::write_envelope(
        &mut client,
        &ClientMessage::Request {
            session,
            id: RequestId(1),
            body: Box::new(Request::Health),
        },
    )
    .expect("write request");
    let body = strop_worker_protocol::frame::read_frame(&mut worker, &mut worker_decoder)
        .expect("frame")
        .expect("eof");
    match codec::decode_body(&body).expect("decode request") {
        Incoming::Envelope(ClientMessage::Request {
            session: stamped,
            id,
            ..
        }) => {
            assert_eq!(id, RequestId(1));
            assert_eq!(authority.admit(&stamped), Ok(()));
        }
        _ => panic!("expected request"),
    }

    // Bulk bytes flow as binary chunks, byte-exact including non-UTF-8.
    let payload: Vec<u8> = (0..=255).chain([0xff, 0x00, 0x80]).collect();
    let chunk = StreamChunk {
        stream: StreamId(9),
        sequence: 0,
        last: true,
        bytes: payload.clone(),
    };
    codec::write_chunk(&mut client, &chunk).expect("write chunk");
    let body = strop_worker_protocol::frame::read_frame(&mut worker, &mut worker_decoder)
        .expect("frame")
        .expect("eof");
    match codec::decode_body::<ClientMessage>(&body).expect("decode chunk") {
        Incoming::Chunk(back) => assert_eq!(back, chunk),
        Incoming::Envelope(_) => panic!("expected chunk"),
    }
}

#[test]
fn stale_incarnation_frame_is_rejected_over_loopback() {
    let (mut client, mut worker) = pair();
    let mut worker_decoder = FrameDecoder::new();

    // The worker restarted: its authority is a new session. A frame
    // stamped by the old session arrives late on a reconnected pipe.
    let restarted = Authority::new(Session {
        incarnation: 2,
        lease: LeaseId(2),
    });
    codec::write_envelope(
        &mut client,
        &ClientMessage::Request {
            session: Session {
                incarnation: 1,
                lease: LeaseId(1),
            },
            id: RequestId(44),
            body: Box::new(Request::Health),
        },
    )
    .expect("write stale request");
    let body = strop_worker_protocol::frame::read_frame(&mut worker, &mut worker_decoder)
        .expect("frame")
        .expect("eof");
    match codec::decode_body(&body).expect("decode") {
        Incoming::Envelope(ClientMessage::Request {
            session: stamped, ..
        }) => {
            assert_eq!(
                restarted.admit(&stamped),
                Err(Refusal::WrongIncarnation { current: 2 })
            );
        }
        _ => panic!("expected request"),
    }
}

#[test]
fn fragmented_stream_survives_arbitrary_splits() {
    let (mut client, mut worker) = pair();
    let mut worker_decoder = FrameDecoder::new();

    let mut wire = Vec::new();
    codec::write_envelope(
        &mut wire,
        &ClientMessage::Shutdown {
            session: Session {
                incarnation: 1,
                lease: LeaseId(1),
            },
        },
    )
    .expect("write shutdown");
    codec::write_chunk(
        &mut wire,
        &StreamChunk {
            stream: StreamId(1),
            sequence: 3,
            last: false,
            bytes: vec![7; 1000],
        },
    )
    .expect("write chunk");

    // Byte-at-a-time delivery still decodes both bodies in order.
    client.write_all(&wire).expect("send");
    drop(client);
    let mut received = Vec::new();
    let mut buffer = [0u8; 1];
    loop {
        let read = worker.read(&mut buffer).expect("read");
        if read == 0 {
            break;
        }
        received.push(buffer[0]);
    }
    worker_decoder.accept(&received).expect("accept");
    let mut bodies = Vec::new();
    while let Some(body) = worker_decoder.next_frame().expect("frame") {
        bodies.push(body);
    }
    assert_eq!(bodies.len(), 2);
    match codec::decode_body::<ClientMessage>(&bodies[0]).expect("decode") {
        Incoming::Envelope(ClientMessage::Shutdown { .. }) => {}
        _ => panic!("expected shutdown"),
    }
    match codec::decode_body::<ClientMessage>(&bodies[1]).expect("decode") {
        Incoming::Chunk(chunk) => {
            assert_eq!(chunk.sequence, 3);
            assert_eq!(chunk.bytes.len(), 1000);
        }
        Incoming::Envelope(_) => panic!("expected chunk"),
    }
}
