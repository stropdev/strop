//! Executable SFTP codec oracles. A scripted in-memory server answers
//! request for request and records exactly what the client sent, so
//! request shapes and pipelining are evidence, not source pins; malformed
//! server frames exercise the production codec and scripts.
use super::super::mutate::{mutate, Mutation};
use super::super::save::{save, Expect, Stamp};
use super::super::session::read_text;
use super::*;
use crate::RemoteError;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use strop_workspace::RemoteFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

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
fn status(id: u32, code: u32) -> Vec<u8> {
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend(string(b"status"));
    payload.extend(string(b""));
    response(PacketKind::Status, id, &payload)
}
/// ATTRS with size, permissions and times.
fn attrs(id: u32, size: u64, mode: u32, mtime: u32) -> Vec<u8> {
    let mut attrs = 13_u32.to_be_bytes().to_vec();
    attrs.extend(size.to_be_bytes());
    attrs.extend(mode.to_be_bytes());
    attrs.extend(mtime.to_be_bytes()); // atime
    attrs.extend(mtime.to_be_bytes());
    response(PacketKind::Attrs, id, &attrs)
}
fn handle(id: u32) -> Vec<u8> {
    response(PacketKind::Handle, id, &string(b"h"))
}
fn data(id: u32, bytes: &[u8]) -> Vec<u8> {
    response(PacketKind::Data, id, &string(bytes))
}
fn kind<T>(result: Result<T, Fault>) -> RemoteFailureKind {
    match result {
        Err(fault) => RemoteError::fault("ssh://fixture/log", fault).kind(),
        Ok(_) => panic!("malformed response accepted"),
    }
}

/// One scripted exchange over an in-memory duplex.
struct Scripted {
    runtime: tokio::runtime::Runtime,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    session: Sftp<tokio::io::WriteHalf<DuplexStream>, tokio::io::ReadHalf<DuplexStream>>,
    server: tokio::task::JoinHandle<()>,
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
    let (server, client) = tokio::io::duplex(1024 * 1024);
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
    let session = runtime.block_on(Sftp::connect(write, read)).unwrap();
    Scripted {
        runtime,
        requests,
        session,
        server,
    }
}

impl Scripted {
    fn kinds(&self) -> Vec<u8> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .skip(1)
            .map(|request| request[0])
            .collect()
    }
    fn request(&self, index: usize) -> Vec<u8> {
        self.requests.lock().unwrap()[index].clone()
    }
    fn read(&mut self, max: u64) -> Result<(String, Stamp), Fault> {
        let file = RemoteFile::parse("ssh://fixture/log").unwrap();
        let stage = std::cell::Cell::new(RemoteStage::Connect);
        self.runtime
            .block_on(read_text(&mut self.session, &file, max, &stage))
    }
    fn save(&mut self, bytes: &[u8], expect: Expect) -> Result<Stamp, Fault> {
        self.runtime
            .block_on(save(&mut self.session, Path::new("/w/log"), bytes, expect))
    }
    fn finish(self) {
        drop(self.session);
        self.runtime.block_on(self.server).unwrap();
    }
}

/// The replies one clean read of `bytes` gets: OPEN+STAT, then READ,
/// the EOF probe, FSTAT and CLOSE.
fn whole(bytes: &[u8], first: u32) -> Vec<Vec<u8>> {
    let size = bytes.len() as u64;
    vec![
        handle(first),
        attrs(first + 1, size, 0o100_644, 7),
        data(first + 2, bytes),
        status(first + 3, 1),
        attrs(first + 4, size, 0o100_644, 7),
        status(first + 5, 0),
    ]
}

const OPEN: u8 = PacketKind::Open as u8;
const STAT: u8 = PacketKind::Stat as u8;
const READ: u8 = PacketKind::Read as u8;
const FSTAT: u8 = PacketKind::Fstat as u8;
const CLOSE: u8 = PacketKind::Close as u8;

#[test]
fn packet_bound_is_checked_before_reading_or_allocating_payload() {
    let bytes = (256_u32 * 1024 + 1).to_be_bytes();
    assert_eq!(
        kind(runtime().block_on(Sftp::connect(tokio::io::sink(), &bytes[..]))),
        RemoteFailureKind::Protocol
    );
}

#[test]
fn a_whole_read_is_two_flights_and_returns_the_handle_stamp() {
    let mut session = scripted(whole(b"hello\n", 1), &[]);
    let (text, stamp) = session.read(1024).unwrap();
    assert_eq!(text, "hello\n");
    assert_eq!(stamp, Stamp { size: 6, mtime: 7 });
    assert_eq!(session.kinds(), [OPEN, STAT, READ, READ, FSTAT, CLOSE]);
    session.finish();
}

#[test]
fn a_reply_for_another_request_cannot_supply_a_file_handle() {
    let mut session = scripted(vec![handle(2), attrs(1, 1, 0o100_644, 0)], &[]);
    assert_eq!(kind(session.read(1024)), RemoteFailureKind::Protocol);
}

#[test]
fn the_size_bound_refuses_before_any_byte_moves_and_still_closes() {
    let mut session = scripted(
        vec![handle(1), attrs(2, 4096, 0o100_644, 0), status(3, 0)],
        &[],
    );
    assert_eq!(kind(session.read(1024)), RemoteFailureKind::TooLarge);
    assert_eq!(session.kinds(), [OPEN, STAT, CLOSE]);
    session.finish();
}

#[test]
fn a_directory_is_not_read_as_text() {
    let mut session = scripted(
        vec![handle(1), attrs(2, 4096, 0o040_755, 0), status(3, 0)],
        &[],
    );
    assert_eq!(
        kind(session.read(1 << 20)),
        RemoteFailureKind::NotRegularFile
    );
    session.finish();
}

#[test]
fn a_file_that_grows_or_shrinks_under_the_read_is_read_again() {
    // First attempt: the probe past the end finds more bytes.
    let mut replies = vec![
        handle(1),
        attrs(2, 3, 0o100_644, 7),
        data(3, b"abc"),
        data(4, b"d"),
        attrs(5, 4, 0o100_644, 8),
        status(6, 0),
    ];
    replies.extend(whole(b"abcd", 7));
    let mut session = scripted(replies, &[]);
    assert_eq!(session.read(1024).unwrap().0, "abcd");
    session.finish();

    // A file that keeps ending early is never a partial success.
    let mut replies = Vec::new();
    for attempt in 0..3 {
        let first = 1 + attempt * 6;
        replies.extend([
            handle(first),
            attrs(first + 1, 8, 0o100_644, 7),
            status(first + 2, 1),
            status(first + 3, 1),
            attrs(first + 4, 2, 0o100_644, 7),
            status(first + 5, 0),
        ]);
    }
    let mut session = scripted(replies, &[]);
    assert_eq!(kind(session.read(1024)), RemoteFailureKind::ShortRead);
    session.finish();
}

#[test]
fn a_failed_close_cannot_complete_a_read() {
    let mut replies = whole(b"x", 1);
    replies[5] = status(6, 4);
    let mut session = scripted(replies, &[]);
    assert_eq!(kind(session.read(1024)), RemoteFailureKind::Failure);
    session.finish();
}

#[test]
fn invalid_utf8_is_refused_whole() {
    let mut session = scripted(whole(b"ok\xff", 1), &[]);
    assert_eq!(kind(session.read(1024)), RemoteFailureKind::InvalidUtf8);
    session.finish();
}

#[test]
fn malformed_nested_lengths_and_extra_reply_bytes_are_rejected() {
    let mut replies = whole(b"abc", 1);
    let mut payload = vec![PacketKind::Data as u8];
    payload.extend(3_u32.to_be_bytes());
    payload.extend(99_u32.to_be_bytes());
    payload.extend(b"abc");
    replies[2] = frame(&payload);
    let mut session = scripted(replies, &[]);
    assert_eq!(kind(session.read(1024)), RemoteFailureKind::Protocol);
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
        let mut client = Sftp::connect(tokio::io::sink(), &input[..]).await.unwrap();
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
            let mut client = Sftp::connect(tokio::io::sink(), &input[..]).await.unwrap();
            client.expand_path(Path::new("~")).await
        });
        assert_eq!(kind(result), RemoteFailureKind::Protocol);
    }
}

const LSTAT: u8 = PacketKind::Lstat as u8;
const WRITE: u8 = PacketKind::Write as u8;
const SETSTAT: u8 = PacketKind::Setstat as u8;
const EXTENDED: u8 = PacketKind::Extended as u8;
const REMOVE: u8 = PacketKind::Remove as u8;
const RENAME: u8 = PacketKind::Rename as u8;

#[test]
fn a_save_is_three_flights_keeps_the_mode_and_returns_the_new_stamp() {
    let mut session = scripted(
        vec![
            attrs(1, 3, 0o100_640, 7),
            handle(2),
            status(3, 0),
            status(4, 0),
            status(5, 0),
            status(6, 0),
            attrs(7, 5, 0o100_640, 9),
        ],
        &[b"posix-rename@openssh.com"],
    );
    let stamp = session
        .save(b"fresh", Expect::Unchanged(Stamp { size: 3, mtime: 7 }))
        .unwrap();
    assert_eq!(stamp, Stamp { size: 5, mtime: 9 });
    assert_eq!(
        session.kinds(),
        [LSTAT, OPEN, WRITE, CLOSE, SETSTAT, EXTENDED, LSTAT]
    );
    // The temporary is created private, then given the original's mode.
    assert!(session.request(2).ends_with(&0o600_u32.to_be_bytes()));
    assert!(session.request(5).ends_with(&0o640_u32.to_be_bytes()));
    assert!(session.request(3).ends_with(b"fresh"));
    session.finish();
}

#[test]
fn a_file_changed_since_it_was_read_is_refused_and_the_temporary_removed() {
    let mut session = scripted(
        vec![
            attrs(1, 4, 0o100_644, 8),
            handle(2),
            status(3, 0),
            status(4, 0),
        ],
        &[b"posix-rename@openssh.com"],
    );
    let refused = session.save(b"mine", Expect::Unchanged(Stamp { size: 3, mtime: 7 }));
    assert_eq!(kind(refused), RemoteFailureKind::Changed);
    assert_eq!(session.kinds(), [LSTAT, OPEN, CLOSE, REMOVE]);
    session.finish();
}

#[test]
fn save_as_never_replaces_an_existing_file() {
    let mut session = scripted(
        vec![
            attrs(1, 4, 0o100_644, 8),
            handle(2),
            status(3, 0),
            status(4, 0),
        ],
        &[],
    );
    assert_eq!(
        kind(session.save(b"x", Expect::Absent)),
        RemoteFailureKind::Exists
    );
    session.finish();
}

#[test]
fn without_posix_rename_a_save_removes_then_renames() {
    let mut session = scripted(
        vec![
            attrs(1, 3, 0o100_644, 7),
            handle(2),
            status(3, 0),
            status(4, 0),
            status(5, 0),
            status(6, 0),
            status(7, 0),
            attrs(8, 1, 0o100_644, 9),
        ],
        &[],
    );
    session.save(b"x", Expect::Any).unwrap();
    assert_eq!(
        session.kinds(),
        [LSTAT, OPEN, WRITE, CLOSE, SETSTAT, REMOVE, RENAME, LSTAT]
    );
    session.finish();
}

#[test]
fn a_failed_write_is_typed_and_removes_the_temporary() {
    let mut session = scripted(
        vec![
            status(1, 2), // nothing at the destination yet
            handle(2),
            status(3, 4),
            status(4, 0),
            status(5, 0),
            status(6, 0),
        ],
        &[b"posix-rename@openssh.com"],
    );
    assert_eq!(
        kind(session.save(b"x", Expect::Absent)),
        RemoteFailureKind::Failure
    );
    assert_eq!(
        session.kinds(),
        [LSTAT, OPEN, WRITE, CLOSE, SETSTAT, REMOVE]
    );
    session.finish();
}

const MKDIR: u8 = PacketKind::Mkdir as u8;
const RMDIR: u8 = PacketKind::Rmdir as u8;

impl Scripted {
    fn mutate(&mut self, mutation: Mutation) -> Result<Option<super::super::RemoteAttrs>, Fault> {
        self.runtime.block_on(mutate(&mut self.session, &mutation))
    }
}

#[test]
fn a_created_file_is_exclusive_and_takes_its_mode_after_the_bytes() {
    let mut session = scripted(
        vec![
            handle(1),
            status(2, 0),
            status(3, 0),
            status(4, 0),
            attrs(5, 3, 0o100_644, 9),
        ],
        &[],
    );
    let after = session
        .mutate(Mutation::CreateFile {
            path: "/w/new.txt".into(),
            bytes: b"new".to_vec(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(after.size, Some(3));
    assert_eq!(session.kinds(), [OPEN, WRITE, CLOSE, SETSTAT, LSTAT]);
    // SSH_FXF_WRITE | SSH_FXF_CREAT | SSH_FXF_EXCL, created private.
    let open = session.request(1);
    assert!(
        open.windows(4).any(|w| w == 0x2a_u32.to_be_bytes()),
        "{open:?}"
    );
    assert!(open.ends_with(&0o600_u32.to_be_bytes()));
    assert!(session.request(4).ends_with(&0o644_u32.to_be_bytes()));
    session.finish();
}

#[test]
fn an_occupied_name_refuses_creation_and_nothing_is_written() {
    let mut session = scripted(vec![status(1, 4)], &[]);
    let refused = session.mutate(Mutation::CreateFile {
        path: "/w/taken.txt".into(),
        bytes: b"x".to_vec(),
    });
    assert_eq!(kind(refused), RemoteFailureKind::Failure);
    assert_eq!(session.kinds(), [OPEN]);
    session.finish();
}

#[test]
fn directories_are_made_and_removed_with_their_own_requests() {
    let mut session = scripted(
        vec![status(1, 0), attrs(2, 4096, 0o040_755, 9), status(3, 0)],
        &[],
    );
    let made = session
        .mutate(Mutation::CreateDirectory {
            path: "/w/dir".into(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(made.kind, crate::RemoteEntryKind::Directory);
    assert!(session.request(1).ends_with(&0o755_u32.to_be_bytes()));
    assert!(session
        .mutate(Mutation::Remove {
            path: "/w/dir".into(),
            directory: true,
        })
        .unwrap()
        .is_none());
    assert_eq!(session.kinds(), [MKDIR, LSTAT, RMDIR]);
    session.finish();
}

#[test]
fn rename_never_uses_the_replacing_extension() {
    let mut session = scripted(
        vec![status(1, 0), attrs(2, 1, 0o100_644, 9)],
        &[b"posix-rename@openssh.com"],
    );
    session
        .mutate(Mutation::Rename {
            from: "/w/a".into(),
            to: "/w/b".into(),
        })
        .unwrap();
    assert_eq!(session.kinds(), [RENAME, LSTAT]);
    session.finish();
}

#[test]
fn a_copy_reads_the_source_and_creates_the_copy_with_its_mode() {
    let mut replies = whole(b"abc", 1);
    replies[1] = attrs(2, 3, 0o100_750, 7);
    replies[4] = attrs(5, 3, 0o100_750, 7);
    replies.extend([
        handle(7),
        status(8, 0),
        status(9, 0),
        status(10, 0),
        attrs(11, 3, 0o100_750, 9),
    ]);
    let mut session = scripted(replies, &[]);
    session
        .mutate(Mutation::Copy {
            from: "/w/a".into(),
            to: "/w/b".into(),
            max: 1024,
        })
        .unwrap();
    assert_eq!(
        session.kinds(),
        [OPEN, STAT, READ, READ, FSTAT, CLOSE, OPEN, WRITE, CLOSE, SETSTAT, LSTAT]
    );
    assert!(session.request(8).ends_with(b"abc"));
    assert!(session.request(10).ends_with(&0o750_u32.to_be_bytes()));
    session.finish();
}
