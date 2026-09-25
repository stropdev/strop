//! Executable SftpWire identity/size oracles. Malformed server frames exercise
//! the production codec and selection reader, not source text or argv snapshots.
use super::*;
use crate::{ReadSelection, RemoteReadError, RemoteWindow};
use strop_workspace::RemoteFile;

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

#[test]
fn home_expansion_decodes_the_realpath_name_reply() {
    let mut input = frame(&[PacketKind::Version as u8, 0, 0, 0, 3]);
    let mut name = 1_u32.to_be_bytes().to_vec();
    name.extend(string(b"/home/fixture/project"));
    name.extend(string(b"human longname is not the target"));
    name.extend(0_u32.to_be_bytes()); // missing attributes are valid here
    input.extend(response(PacketKind::Name, 1, &name));
    let expanded = runtime().block_on(async {
        let (mut client, _) = ReadOnlySftp::connect(tokio::io::sink(), &input[..])
            .await
            .unwrap();
        client.expand_path(Path::new("~/project")).await.unwrap()
    });
    assert_eq!(expanded, Path::new("/home/fixture/project"));
}

#[test]
fn home_expansion_refuses_ambiguous_name_counts() {
    for count in [0_u32, 2] {
        let mut input = frame(&[PacketKind::Version as u8, 0, 0, 0, 3]);
        input.extend(response(PacketKind::Name, 1, &count.to_be_bytes()));
        let result = runtime().block_on(async {
            let (mut client, _) = ReadOnlySftp::connect(tokio::io::sink(), &input[..])
                .await
                .unwrap();
            client.expand_path(Path::new("~")).await
        });
        assert_eq!(kind(result), ReadFailureKind::Protocol);
    }
}

// ---------------------------------------------------------------- DeploySftp
// The deploy write session crosses the same codec: a scripted in-memory
// server answers request-for-request and captures exactly what the
// client sent, so request shapes are evidence, not source pins.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

/// One scripted exchange: the server replies to the client's packets in
/// order and records every request payload it received.
struct Scripted {
    runtime: tokio::runtime::Runtime,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    session: DeploySftp<tokio::io::WriteHalf<DuplexStream>, tokio::io::ReadHalf<DuplexStream>>,
    server: tokio::task::JoinHandle<()>,
}

fn attrs_reply(size: u64, uid: u32, gid: u32, mode: u32) -> Vec<u8> {
    let mut attrs = 7_u32.to_be_bytes().to_vec(); // size + uid/gid + permissions
    attrs.extend(size.to_be_bytes());
    attrs.extend(uid.to_be_bytes());
    attrs.extend(gid.to_be_bytes());
    attrs.extend(mode.to_be_bytes());
    attrs
}

fn status_message(id: u32, code: u32, message: &[u8]) -> Vec<u8> {
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend(string(message));
    payload.extend(string(b""));
    response(PacketKind::Status, id, &payload)
}

fn scripted(replies: Vec<Vec<u8>>, extensions: &[&[u8]]) -> Scripted {
    let mut version = vec![PacketKind::Version as u8, 0, 0, 0, 3];
    for name in extensions {
        version.extend(string(name));
        version.extend(string(b""));
    }
    let mut replies: VecDeque<Vec<u8>> = replies.into();
    replies.push_front(frame(&version));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recording = Arc::clone(&requests);
    let runtime = runtime();
    let _context = runtime.enter();
    let (server, client) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let mut server = server;
        let mut length = [0_u8; 4];
        loop {
            if server.read_exact(&mut length).await.is_err() {
                return;
            }
            let length = u32::from_be_bytes(length) as usize;
            assert!(length > 0 && length <= 256 * 1024, "request packet bound");
            let mut payload = vec![0_u8; length];
            if server.read_exact(&mut payload).await.is_err() {
                return;
            }
            recording.lock().unwrap().push(payload);
            let Some(reply) = replies.pop_front() else {
                return;
            };
            if server.write_all(&reply).await.is_err() {
                return;
            }
        }
    });
    let (read, write) = tokio::io::split(client);
    let session = runtime.block_on(DeploySftp::connect(write, read)).unwrap();
    Scripted {
        runtime,
        requests,
        session,
        server,
    }
}

impl Scripted {
    fn request(&self, index: usize) -> Vec<u8> {
        self.requests.lock().unwrap()[index].clone()
    }
    fn finish(self) {
        drop(self.session);
        self.runtime.block_on(self.server).unwrap();
    }
}

#[test]
fn deploy_lstat_decodes_owner_mode_and_size_from_real_attrs() {
    let mut session = scripted(
        vec![response(
            PacketKind::Attrs,
            1,
            &attrs_reply(4096, 1000, 100, 0o100700),
        )],
        &[],
    );
    let attrs = session
        .runtime
        .block_on(session.session.lstat(Path::new("/cache/objects/ab")))
        .unwrap();
    assert_eq!(attrs.size, Some(4096));
    assert_eq!(attrs.uid, Some(1000));
    assert_eq!(attrs.permissions, Some(0o100700));
    // Request one is INIT; request two is LSTAT of the exact path bytes.
    let lstat = session.request(1);
    assert_eq!(lstat[0], PacketKind::Lstat as u8);
    assert!(lstat
        .windows(b"/cache/objects/ab".len())
        .any(|w| w == b"/cache/objects/ab"));
    session.finish();
}

#[test]
fn deploy_write_sequence_sends_exact_flags_offsets_and_modes() {
    let mut session = scripted(
        vec![
            status(1, 0),
            response(PacketKind::Handle, 2, &string(b"h")),
            status(3, 0),
            status(4, 0),
        ],
        &[],
    );
    let bytes = b"worker bytes".as_slice();
    session.runtime.block_on(async {
        session
            .session
            .mkdir(Path::new("/cache/strop-worker"), 0o700)
            .await
            .unwrap();
        let handle = session
            .session
            .open_write(Path::new("/cache/staging/aa"))
            .await
            .unwrap();
        session.session.write_at(&handle, 0, bytes).await.unwrap();
        session.session.close(handle).await.unwrap();
    });
    let mkdir = session.request(1);
    assert_eq!(mkdir[0], PacketKind::Mkdir as u8);
    assert!(mkdir.ends_with(&0o700_u32.to_be_bytes()));
    let open = session.request(2);
    assert_eq!(open[0], PacketKind::Open as u8);
    // SSH_FXF_WRITE|CREAT|TRUNC then private create mode.
    assert!(open.windows(4).any(|w| w == 0x1a_u32.to_be_bytes()));
    assert!(open.ends_with(&0o600_u32.to_be_bytes()));
    let write = session.request(3);
    assert_eq!(write[0], PacketKind::Write as u8);
    assert!(write.ends_with(bytes));
    session.finish();
}

#[test]
fn deploy_rename_prefers_posix_rename_only_when_advertised() {
    // With the extension advertised: an Extended request, never v3 rename.
    let mut session = scripted(vec![status(1, 0)], &[b"posix-rename@openssh.com"]);
    session
        .runtime
        .block_on(session.session.rename(
            Path::new("/cache/staging/aa"),
            Path::new("/cache/objects/bb"),
        ))
        .unwrap();
    let extended = session.request(1);
    assert_eq!(extended[0], PacketKind::Extended as u8);
    assert!(extended
        .windows(b"posix-rename@openssh.com".len())
        .any(|w| w == b"posix-rename@openssh.com"));
    session.finish();

    // Without it: the plain v3 rename.
    let mut session = scripted(vec![status(1, 0)], &[]);
    session
        .runtime
        .block_on(session.session.rename(
            Path::new("/cache/staging/aa"),
            Path::new("/cache/objects/bb"),
        ))
        .unwrap();
    assert_eq!(session.request(1)[0], PacketKind::Rename as u8);
    session.finish();
}

#[test]
fn deploy_failures_are_typed_and_a_failed_close_poisons() {
    // Permission on mkdir is a typed refusal, never a silent success.
    let mut session = scripted(vec![status_message(1, 3, b"Permission denied")], &[]);
    let result = session
        .runtime
        .block_on(session.session.mkdir(Path::new("/root/nope"), 0o700));
    assert_eq!(kind(result), ReadFailureKind::Permission);
    session.finish();

    // A failed close after a good write poisons the connection.
    let mut session = scripted(
        vec![
            response(PacketKind::Handle, 1, &string(b"h")),
            status_message(2, 4, b"no space left on device"),
            status_message(3, 4, b"disk quota exceeded"),
        ],
        &[],
    );
    let fault = session
        .runtime
        .block_on(async {
            let handle = session
                .session
                .open_write(Path::new("/cache/staging/aa"))
                .await
                .unwrap();
            let outcome = session.session.write_at(&handle, 0, b"x").await.map(|_| ());
            match outcome {
                Ok(()) => session.session.close(handle).await,
                Err(primary) => match session.session.close(handle).await {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(primary.with_cleanup(cleanup)),
                },
            }
        })
        .unwrap_err();
    let (_, poisoned) = fault.disposition();
    assert!(poisoned, "a failed close poisons the connection");
    session.finish();
}

#[test]
fn deploy_read_back_enforces_the_length_bound_and_still_closes() {
    let mut session = scripted(
        vec![
            response(PacketKind::Handle, 1, &string(b"h")),
            response(
                PacketKind::Attrs,
                2,
                &attrs_reply(4096, 1000, 1000, 0o100500),
            ),
            status(3, 0),
        ],
        &[],
    );
    let result = session.runtime.block_on(
        session
            .session
            .read_file(Path::new("/cache/objects/ab"), 1024),
    );
    assert_eq!(kind(result), ReadFailureKind::TooLarge);
    // The close ran even on the bound failure (request four is Close).
    assert_eq!(session.request(3)[0], PacketKind::Close as u8);
    session.finish();
}

#[test]
fn deploy_listing_skips_dot_entries_and_bounds_names() {
    // One NAME page carrying ".", ".." and a real entry, then EOF.
    let mut name = 3_u32.to_be_bytes().to_vec();
    for entry in [".", "..", "0123abcd.json"] {
        name.extend(string(entry.as_bytes()));
        name.extend(string(b"human longname"));
        name.extend(0_u32.to_be_bytes()); // no attrs
    }
    let mut session = scripted(
        vec![
            response(PacketKind::Handle, 1, &string(b"d")),
            response(PacketKind::Name, 2, &name),
            status(3, 1), // SSH_FX_EOF ends the listing
            status(4, 0), // close
        ],
        &[],
    );
    let names = session
        .runtime
        .block_on(session.session.list_names(Path::new("/cache/receipts")))
        .unwrap();
    assert_eq!(names, vec!["0123abcd.json".to_string()]);
    session.finish();
}
