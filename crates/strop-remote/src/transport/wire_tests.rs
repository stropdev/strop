//! Executable counterparts of SftpWire's identity and size invariants. These
//! inject malformed server frames, not implementation/argv snapshots.
use super::*;
use crate::{ReadFailureKind, RemoteReadError};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}
fn response(kind: PacketKind, id: u32, data: &[u8]) -> Vec<u8> {
    let mut payload = vec![kind as u8];
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(data);
    frame(&payload)
}
fn string(data: &[u8]) -> Vec<u8> {
    let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(data);
    bytes
}
fn initial(size: u64) -> Vec<u8> {
    let mut input = frame(&[PacketKind::Version as u8, 0, 0, 0, 3]);
    input.extend(response(PacketKind::Handle, 1, &string(b"handle")));
    let mut attrs = 5_u32.to_be_bytes().to_vec(); // size + permissions
    attrs.extend(size.to_be_bytes());
    attrs.extend(0o100644_u32.to_be_bytes());
    input.extend(response(PacketKind::Attrs, 2, &attrs));
    input
}
fn kind<T>(result: Result<T, Fault>) -> ReadFailureKind {
    match result {
        Err(fault) => RemoteReadError::fault("ssh://fixture/log", fault).kind(),
        Ok(_) => panic!("malformed response was accepted"),
    }
}

#[test]
fn packet_bound_is_checked_before_reading_or_allocating_payload() {
    let bytes = ((MAX_PACKET + 1) as u32).to_be_bytes();
    let result = runtime().block_on(ReadOnlySftp::connect(tokio::io::sink(), &bytes[..]));
    // Missing payload would be I/O failure if the cap weren't enforced first.
    assert_eq!(kind(result), ReadFailureKind::Protocol);
}

#[test]
fn a_reply_for_another_request_cannot_supply_a_file_handle() {
    let mut bytes = frame(&[PacketKind::Version as u8, 0, 0, 0, 3]);
    bytes.extend(response(PacketKind::Handle, 9, &string(b"stale")));
    runtime().block_on(async {
        let mut client = ReadOnlySftp::connect(tokio::io::sink(), bytes.as_slice())
            .await
            .unwrap();
        assert_eq!(
            kind(client.open(Path::new("/log")).await),
            ReadFailureKind::Protocol
        );
    });
}

#[test]
fn snapshot_length_is_validated_before_content_allocation() {
    let bytes = initial(MAX_SNAPSHOT + 1);
    runtime().block_on(async {
        let mut client = ReadOnlySftp::connect(tokio::io::sink(), bytes.as_slice())
            .await
            .unwrap();
        let handle = client.open(Path::new("/log")).await.unwrap();
        assert_eq!(
            kind(client.inspect(&handle).await),
            ReadFailureKind::TooLarge
        );
    });
}

#[test]
fn a_server_cannot_append_beyond_the_captured_snapshot_length() {
    let mut bytes = initial(1);
    bytes.extend(response(PacketKind::Data, 3, &string(b"ab")));
    runtime().block_on(async {
        let mut client = ReadOnlySftp::connect(tokio::io::sink(), bytes.as_slice())
            .await
            .unwrap();
        let handle = client.open(Path::new("/log")).await.unwrap();
        let length = client.inspect(&handle).await.unwrap();
        assert_eq!(
            kind(client.read(&handle, length).await),
            ReadFailureKind::Protocol
        );
    });
}

#[test]
fn early_eof_never_becomes_a_successful_partial_snapshot() {
    let mut bytes = initial(2);
    let mut status = 1_u32.to_be_bytes().to_vec();
    status.extend(string(b"EOF"));
    status.extend(string(b""));
    bytes.extend(response(PacketKind::Status, 3, &status));
    runtime().block_on(async {
        let mut client = ReadOnlySftp::connect(tokio::io::sink(), bytes.as_slice())
            .await
            .unwrap();
        let handle = client.open(Path::new("/log")).await.unwrap();
        let length = client.inspect(&handle).await.unwrap();
        assert_eq!(
            kind(client.read(&handle, length).await),
            ReadFailureKind::ShortRead
        );
    });
}

#[test]
fn malformed_nested_lengths_and_duplicate_reply_bytes_are_rejected() {
    let mut bytes = frame(&[PacketKind::Version as u8, 0, 0, 0, 3]);
    bytes.extend(response(PacketKind::Handle, 1, &u32::MAX.to_be_bytes()));
    runtime().block_on(async {
        let mut client = ReadOnlySftp::connect(tokio::io::sink(), bytes.as_slice())
            .await
            .unwrap();
        assert_eq!(
            kind(client.open(Path::new("/log")).await),
            ReadFailureKind::Protocol
        );
    });
    let mut bytes = initial(1);
    let mut data = string(b"a");
    data.push(0); // cannot silently ignore bytes after a DATA payload
    bytes.extend(response(PacketKind::Data, 3, &data));
    runtime().block_on(async {
        let mut client = ReadOnlySftp::connect(tokio::io::sink(), bytes.as_slice())
            .await
            .unwrap();
        let handle = client.open(Path::new("/log")).await.unwrap();
        let length = client.inspect(&handle).await.unwrap();
        assert_eq!(
            kind(client.read(&handle, length).await),
            ReadFailureKind::Protocol
        );
    });
}

fn completed_session(bytes: Vec<u8>) -> Result<String, ReadFailureKind> {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = strop_core::worker::spawn(
        "session-oracle",
        move |outcome| {
            tx.send(outcome).unwrap();
        },
        move |token| {
            let result = runtime()
                .block_on(super::super::session::run(
                    tokio::io::sink(),
                    bytes.as_slice(),
                    Path::new("/log"),
                    &token,
                ))
                .map_err(|fault| RemoteReadError::fault("ssh://fixture/log", fault).kind());
            strop_core::worker::Outcome::Success(result)
        },
    );
    let result = rx.recv().unwrap();
    drop(worker);
    match result {
        strop_core::worker::Outcome::Success(result) => result,
        other => panic!("session worker failed: {other:?}"),
    }
}

#[test]
fn failed_close_cannot_complete_a_snapshot() {
    let mut bytes = initial(0);
    let mut status = 4_u32.to_be_bytes().to_vec();
    status.extend(string(b"close failed"));
    status.extend(string(b""));
    bytes.extend(response(PacketKind::Status, 3, &status));
    assert_eq!(completed_session(bytes), Err(ReadFailureKind::Protocol));
}

#[test]
fn invalid_utf8_cannot_be_replaced_with_a_successful_lossy_snapshot() {
    let mut bytes = initial(1);
    bytes.extend(response(PacketKind::Data, 3, &string(&[0xff])));
    let mut status = 0_u32.to_be_bytes().to_vec();
    status.extend(string(b""));
    status.extend(string(b""));
    bytes.extend(response(PacketKind::Status, 4, &status));
    assert_eq!(completed_session(bytes), Err(ReadFailureKind::InvalidUtf8));
}
