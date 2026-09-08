//! Executable SftpWire identity/size oracles. Malformed server frames exercise
//! the production codec and selection reader, not source text or argv snapshots.
use super::*;
use crate::{ReadSelection, RemoteFile, RemoteReadError, RemoteWindow};

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
    let mut attrs = 5_u32.to_be_bytes().to_vec();
    attrs.extend(size.to_be_bytes());
    attrs.extend(0o100644_u32.to_be_bytes());
    input.extend(response(PacketKind::Attrs, 2, &attrs));
    input
}
fn status(id: u32, code: u32) -> Vec<u8> {
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend(string(b"status"));
    payload.extend(string(b""));
    response(PacketKind::Status, id, &payload)
}
fn kind<T>(result: Result<T, Fault>) -> ReadFailureKind {
    match result {
        Err(fault) => RemoteReadError::fault("ssh://fixture/log", fault).kind(),
        Ok(_) => panic!("malformed response accepted"),
    }
}
fn read_session(
    bytes: &[u8],
    selection: ReadSelection,
) -> Result<(String, RemoteWindow), ReadFailureKind> {
    runtime()
        .block_on(async {
            let (mut client, _) = ReadOnlySftp::connect(tokio::io::sink(), bytes).await?;
            let file = RemoteFile::parse("ssh://fixture/log").unwrap();
            let stage = std::cell::Cell::new(ReadStage::Connect);
            super::super::session::read_selection(&mut client, &file, &selection, &stage).await
        })
        .map_err(|fault| RemoteReadError::fault("ssh://fixture/log", fault).kind())
}

#[test]
fn packet_bound_is_checked_before_reading_or_allocating_payload() {
    let bytes = ((MAX_PACKET + 1) as u32).to_be_bytes();
    assert_eq!(
        kind(runtime().block_on(ReadOnlySftp::connect(tokio::io::sink(), &bytes[..]))),
        ReadFailureKind::Protocol
    );
}
#[test]
fn a_reply_for_another_request_cannot_supply_a_file_handle() {
    let mut bytes = frame(&[PacketKind::Version as u8, 0, 0, 0, 3]);
    bytes.extend(response(PacketKind::Handle, 9, &string(b"stale")));
    assert_eq!(
        read_session(&bytes, ReadSelection::Full),
        Err(ReadFailureKind::Protocol)
    );
}
#[test]
fn memory_cap_applies_to_the_window_not_the_remote_file() {
    let size = MAX_SNAPSHOT + 1024;
    let mut too_large = initial(size);
    too_large.extend(status(3, 0));
    assert_eq!(
        read_session(&too_large, ReadSelection::Full),
        Err(ReadFailureKind::TooLarge)
    );
    let mut tail = initial(size);
    tail.extend(response(PacketKind::Data, 3, &string(b"end")));
    tail.extend(status(4, 0));
    let (text, window) = read_session(
        &tail,
        ReadSelection::Tail(crate::ReadLimit::new(3).unwrap()),
    )
    .unwrap();
    assert_eq!(text, "end");
    assert_eq!(window.start().get(), size - 3);
    assert_eq!(window.length().get(), 3);
    assert!(!window.is_complete());
}
#[test]
fn a_server_cannot_append_beyond_the_captured_snapshot_length() {
    let mut bytes = initial(1);
    bytes.extend(response(PacketKind::Data, 3, &string(b"ab")));
    assert_eq!(
        read_session(&bytes, ReadSelection::Full),
        Err(ReadFailureKind::Protocol)
    );
}
#[test]
fn early_eof_never_becomes_a_successful_partial_snapshot() {
    let mut bytes = initial(2);
    bytes.extend(status(3, 1));
    assert_eq!(
        read_session(&bytes, ReadSelection::Full),
        Err(ReadFailureKind::ShortRead)
    );
}
#[test]
fn malformed_nested_lengths_and_extra_reply_bytes_are_rejected() {
    let mut bytes = frame(&[PacketKind::Version as u8, 0, 0, 0, 3]);
    bytes.extend(response(PacketKind::Handle, 1, &u32::MAX.to_be_bytes()));
    assert_eq!(
        read_session(&bytes, ReadSelection::Full),
        Err(ReadFailureKind::Protocol)
    );
    let mut bytes = initial(1);
    let mut data = string(b"a");
    data.push(0);
    bytes.extend(response(PacketKind::Data, 3, &data));
    assert_eq!(
        read_session(&bytes, ReadSelection::Full),
        Err(ReadFailureKind::Protocol)
    );
}
#[test]
fn failed_close_cannot_complete_a_snapshot() {
    let mut bytes = initial(0);
    bytes.extend(status(3, 4));
    assert_eq!(
        read_session(&bytes, ReadSelection::Full),
        Err(ReadFailureKind::Protocol)
    );
}
#[test]
fn invalid_utf8_is_not_trimmed_from_a_whole_file() {
    for content in [&[0xff][..], &[0x80, 0x80, 0x80, 0x80], &[0xe8, 0xaa]] {
        let mut bytes = initial(content.len() as u64);
        bytes.extend(response(PacketKind::Data, 3, &string(content)));
        bytes.extend(status(4, 0));
        assert_eq!(
            read_session(&bytes, ReadSelection::Full),
            Err(ReadFailureKind::InvalidUtf8)
        );
    }
}
#[test]
fn range_edges_trim_only_cut_utf8_and_report_the_actual_bytes() {
    let content = "日abc語".as_bytes();
    let mut bytes = initial(content.len() as u64);
    bytes.extend(response(PacketKind::Data, 3, &string(&content[1..8])));
    bytes.extend(status(4, 0));
    let (text, window) = read_session(
        &bytes,
        ReadSelection::Range {
            start: crate::RemoteOffset::new(1),
            length: crate::ReadLimit::new(7).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(text, "abc");
    assert_eq!(window.start().get(), 3);
    assert_eq!(window.length().get(), 3);
}
