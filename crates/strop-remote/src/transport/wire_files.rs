//! Whole-file SFTP reads and saves for editing over a multiplexed SSH
//! connection (plan 0090 §5), pipelined to a fixed number of round trips.
//!
//! Pipelining matters off loopback: one request per round trip turns a
//! 1 MB open into 32 round trips. Requests are sent up to a window ahead
//! and replies are consumed in request order — OpenSSH's `sftp-server`
//! answers in order, and a server that reorders gets an honest protocol
//! fault from [`Sftp::decode`], never a misplaced chunk.
use super::{
    parse_attrs, Attrs, Fault, FileHandle, PacketKind, RemoteFailureKind, RemoteStage, RequestId,
    Sftp, READ_CHUNK, WRITE_CHUNK,
};
use std::collections::VecDeque;
use std::path::Path;
use strop_workspace::addr::uri::path_bytes;
use tokio::io::{AsyncRead, AsyncWrite};

/// Requests in flight per transfer: 1 MiB of reads or 1.5 MiB of
/// writes, inside OpenSSH's 2 MiB session channel window.
const WINDOW: usize = 32;
const WRITE_WINDOW: usize = 8;
/// Re-reads of a file that changed while it was read.
const READ_ATTEMPTS: usize = 2;

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> Sftp<W, R> {
    /// Whether `rename` atomically replaces an existing destination.
    pub(crate) fn overwrites_on_rename(&self) -> bool {
        self.offers(b"posix-rename@openssh.com")
    }

    /// The whole file within `max` bytes, in two round trips for a file
    /// within the read window: OPEN and STAT pipelined, then every READ,
    /// an end-of-file probe, FSTAT and CLOSE in one flight. The attributes are the open handle's
    /// (they describe exactly the bytes returned); a file that changes
    /// while it is read is read again, a bounded number of times.
    pub(crate) async fn read_whole(
        &mut self,
        path: &Path,
        max: u64,
    ) -> Result<(Vec<u8>, Attrs), Fault> {
        let mut attempt = 0;
        loop {
            match self.read_once(path, max).await? {
                Some(read) => return Ok(read),
                None if attempt < READ_ATTEMPTS => attempt += 1,
                None => {
                    return Err(Fault::new(
                        RemoteStage::Transfer,
                        RemoteFailureKind::ShortRead,
                        "the file kept changing while it was read",
                    ))
                }
            }
        }
    }

    /// One pipelined read; `None` when the file changed under it.
    async fn read_once(
        &mut self,
        path: &Path,
        max: u64,
    ) -> Result<Option<(Vec<u8>, Attrs)>, Fault> {
        let open = self.begin(PacketKind::Open, RemoteStage::Open)?;
        self.string(path_bytes(path), RemoteStage::Open)?;
        self.request.extend_from_slice(&0x01_u32.to_be_bytes()); // SSH_FXF_READ
        self.request.extend_from_slice(&0_u32.to_be_bytes());
        self.send(RemoteStage::Open).await?;
        let stat = self.begin(PacketKind::Stat, RemoteStage::Inspect)?;
        self.string(path_bytes(path), RemoteStage::Inspect)?;
        self.send(RemoteStage::Inspect).await?;
        self.receive(RemoteStage::Open).await?;
        let handle = self.decode_handle(open, RemoteStage::Open);
        self.receive(RemoteStage::Inspect).await?;
        let attrs = self
            .decode(stat, PacketKind::Attrs, RemoteStage::Inspect)
            .and_then(|mut reply| {
                let attrs = parse_attrs(&mut reply)?;
                reply.end()?;
                Ok(attrs)
            });
        let handle = handle?;
        let size = attrs.and_then(|attrs| {
            let size = attrs.size.ok_or_else(|| {
                Fault::new(
                    RemoteStage::Inspect,
                    RemoteFailureKind::UnknownLength,
                    "the server reported no file length",
                )
            })?;
            if size > max {
                return Err(Fault::new(
                    RemoteStage::Inspect,
                    RemoteFailureKind::TooLarge,
                    format!("{size} bytes exceeds the {max}-byte bound"),
                ));
            }
            if attrs
                .permissions
                .is_some_and(|mode| mode & 0o170_000 != 0o100_000)
            {
                return Err(Fault::new(
                    RemoteStage::Inspect,
                    RemoteFailureKind::NotRegularFile,
                    "not a regular file",
                ));
            }
            Ok((size, attrs))
        });
        let (size, stated) = match size {
            Ok(size) => size,
            Err(primary) => {
                return Err(match self.close(handle).await {
                    Ok(()) => primary,
                    Err(cleanup) => primary.with_cleanup(cleanup),
                })
            }
        };
        let (bytes, opened) = self.read_through(handle, size).await?;
        let unchanged = opened.size == Some(size) && opened.mtime == stated.mtime;
        Ok(bytes.filter(|_| unchanged).map(|bytes| (bytes, opened)))
    }

    fn decode_handle(&self, id: RequestId, stage: RemoteStage) -> Result<FileHandle, Fault> {
        let mut reply = self.decode(id, PacketKind::Handle, stage)?;
        let handle = reply.string()?;
        if handle.is_empty() || handle.len() > 1024 {
            return Err(reply.invalid("invalid SFTP file handle length"));
        }
        let handle = FileHandle(handle.to_vec());
        reply.end()?;
        Ok(handle)
    }

    /// Read `size` bytes of `handle` with up to [`WINDOW`] requests in
    /// flight; the moment the last read is sent, one byte past the end,
    /// FSTAT and CLOSE follow in the same flight, so a file within the
    /// window is one round trip. Every reply is consumed even after a
    /// failure so the session stays in step. The bytes are `None` when
    /// the file was shorter or longer than `size` (it changed under the
    /// read) or a short chunk arrived after the tail was already sent.
    async fn read_through(
        &mut self,
        handle: FileHandle,
        size: u64,
    ) -> Result<(Option<Vec<u8>>, Attrs), Fault> {
        let stage = RemoteStage::Transfer;
        let length = usize::try_from(size)
            .map_err(|_| Fault::new(stage, RemoteFailureKind::TooLarge, "file exceeds memory"))?;
        let mut bytes = vec![0_u8; length];
        let mut pending: VecDeque<Flight> = VecDeque::new();
        let mut next = 0_usize;
        let mut tail_sent = false;
        let mut changed = false;
        let mut failure: Option<Fault> = None;
        let mut attrs: Option<Result<Attrs, Fault>> = None;
        let mut closed: Option<Result<(), Fault>> = None;
        loop {
            while !tail_sent && pending.len() < WINDOW && next < length && !changed {
                let count = READ_CHUNK.min(length - next);
                let id = self.send_read(&handle, next, count).await?;
                pending.push_back(Flight::Read {
                    id,
                    offset: next,
                    count,
                });
                next += count;
            }
            if !tail_sent && (next >= length || changed) {
                pending.push_back(Flight::Probe(self.send_read(&handle, length, 1).await?));
                let fstat = self.begin(PacketKind::Fstat, stage)?;
                self.string(&handle.0, stage)?;
                self.send(stage).await?;
                pending.push_back(Flight::Fstat(fstat));
                let close = self.begin(PacketKind::Close, RemoteStage::Teardown)?;
                self.string(&handle.0, RemoteStage::Teardown)?;
                self.send(RemoteStage::Teardown).await?;
                pending.push_back(Flight::Close(close));
                tail_sent = true;
            }
            let Some(flight) = pending.pop_front() else {
                break;
            };
            self.receive(stage).await?;
            match flight {
                Flight::Read { id, offset, count } => {
                    let reply = self.decode(id, PacketKind::Data, stage);
                    match reply.and_then(|mut reply| {
                        let data = reply.string()?;
                        if data.len() > count {
                            return Err(reply.invalid("SFTP server sent more bytes than requested"));
                        }
                        bytes[offset..offset + data.len()].copy_from_slice(data);
                        reply.end()?;
                        Ok(data.len())
                    }) {
                        Ok(got) if got == count => {}
                        // A short chunk is protocol-legal: ask for the
                        // rest while the tail is not yet in flight.
                        Ok(got) if got > 0 && !tail_sent && !changed => {
                            let rest = offset + got;
                            let id = self.send_read(&handle, rest, count - got).await?;
                            pending.push_back(Flight::Read {
                                id,
                                offset: rest,
                                count: count - got,
                            });
                        }
                        Ok(_) => changed = true,
                        // End of file before `size`.
                        Err(fault) if fault.disposition().0 == RemoteFailureKind::ShortRead => {
                            changed = true
                        }
                        Err(fault) => {
                            failure.get_or_insert(fault);
                        }
                    }
                }
                Flight::Probe(id) => match self.decode(id, PacketKind::Data, stage) {
                    Ok(_) => changed = true, // bytes past `size`: it grew
                    Err(fault) if fault.disposition().0 == RemoteFailureKind::ShortRead => {}
                    Err(fault) => {
                        failure.get_or_insert(fault);
                    }
                },
                Flight::Fstat(id) => {
                    attrs = Some(self.decode(id, PacketKind::Attrs, stage).and_then(
                        |mut reply| {
                            let attrs = parse_attrs(&mut reply)?;
                            reply.end()?;
                            Ok(attrs)
                        },
                    ));
                }
                Flight::Close(id) => {
                    closed = Some(
                        self.decode(id, PacketKind::Status, RemoteStage::Teardown)
                            .and_then(|reply| reply.end()),
                    );
                }
            }
        }
        let missing = || Fault::new(stage, RemoteFailureKind::Protocol, "read tail was not sent");
        let closed = closed.unwrap_or_else(|| Err(missing()));
        if let Some(fault) = failure {
            return Err(match closed {
                Ok(()) => fault,
                Err(cleanup) => fault.with_cleanup(cleanup),
            });
        }
        closed?;
        let attrs = attrs.unwrap_or_else(|| Err(missing()))?;
        Ok(((!changed).then_some(bytes), attrs))
    }

    async fn send_read(
        &mut self,
        handle: &FileHandle,
        offset: usize,
        count: usize,
    ) -> Result<RequestId, Fault> {
        let stage = RemoteStage::Transfer;
        let id = self.begin(PacketKind::Read, stage)?;
        self.string(&handle.0, stage)?;
        self.request
            .extend_from_slice(&(offset as u64).to_be_bytes());
        self.request
            .extend_from_slice(&(count as u32).to_be_bytes());
        self.send(stage).await?;
        Ok(id)
    }

    /// LSTAT `path` and create `temporary` (private, truncated) in one
    /// round trip: the save's first flight. The outer error is a broken
    /// session; the inner results are the two replies.
    pub(crate) async fn inspect_and_create(
        &mut self,
        path: &Path,
        temporary: &Path,
    ) -> Result<(Result<Attrs, Fault>, Result<FileHandle, Fault>), Fault> {
        let lstat = self.begin(PacketKind::Lstat, RemoteStage::Inspect)?;
        self.string(path_bytes(path), RemoteStage::Inspect)?;
        self.send(RemoteStage::Inspect).await?;
        let open = self.send_open_write(temporary).await?;
        self.receive(RemoteStage::Inspect).await?;
        let attrs = self
            .decode(lstat, PacketKind::Attrs, RemoteStage::Inspect)
            .and_then(|mut reply| {
                let attrs = parse_attrs(&mut reply)?;
                reply.end()?;
                Ok(attrs)
            });
        self.receive(RemoteStage::Open).await?;
        Ok((attrs, self.decode_handle(open, RemoteStage::Open)))
    }

    async fn send_open_write(&mut self, path: &Path) -> Result<RequestId, Fault> {
        let id = self.begin(PacketKind::Open, RemoteStage::Open)?;
        self.string(path_bytes(path), RemoteStage::Open)?;
        // SSH_FXF_WRITE | SSH_FXF_CREAT | SSH_FXF_TRUNC
        self.request
            .extend_from_slice(&(0x02_u32 | 0x08 | 0x10).to_be_bytes());
        self.attrs_permissions(0o600);
        self.send(RemoteStage::Open).await?;
        Ok(id)
    }

    /// Create `path` (private, truncated) holding exactly `bytes` with
    /// permission bits `mode`.
    pub(crate) async fn upload(
        &mut self,
        path: &Path,
        bytes: &[u8],
        mode: u32,
    ) -> Result<(), Fault> {
        let open = self.send_open_write(path).await?;
        self.receive(RemoteStage::Open).await?;
        let handle = self.decode_handle(open, RemoteStage::Open)?;
        self.upload_into(handle, path, bytes, mode).await
    }

    /// Fill the open `handle` of `path` with exactly `bytes`, close it
    /// and set permission bits `mode`, every request pipelined. The
    /// first failure is reported after every reply is consumed, so the
    /// session stays in step.
    pub(crate) async fn upload_into(
        &mut self,
        handle: FileHandle,
        path: &Path,
        bytes: &[u8],
        mode: u32,
    ) -> Result<(), Fault> {
        let stage = RemoteStage::Transfer;
        let mut pending: VecDeque<RequestId> = VecDeque::new();
        let mut failure = None;
        for (index, chunk) in bytes.chunks(WRITE_CHUNK).enumerate() {
            if pending.len() == WRITE_WINDOW {
                self.status(pending.pop_front(), &mut failure).await?;
            }
            let id = self.begin(PacketKind::Write, stage)?;
            self.string(&handle.0, stage)?;
            let offset = (index * WRITE_CHUNK) as u64;
            self.request.extend_from_slice(&offset.to_be_bytes());
            self.string(chunk, stage)?;
            self.send(stage).await?;
            pending.push_back(id);
        }
        let id = self.begin(PacketKind::Close, stage)?;
        self.string(&handle.0, stage)?;
        self.send(stage).await?;
        pending.push_back(id);
        let id = self.begin(PacketKind::Setstat, stage)?;
        self.string(path_bytes(path), stage)?;
        self.attrs_permissions(mode);
        self.send(stage).await?;
        pending.push_back(id);
        while !pending.is_empty() {
            self.status(pending.pop_front(), &mut failure).await?;
        }
        failure.map_or(Ok(()), Err)
    }

    /// Close the open `handle` of `path` and remove `path`, pipelined:
    /// a staged file that will not be published.
    pub(crate) async fn discard(&mut self, handle: FileHandle, path: &Path) -> Result<(), Fault> {
        let stage = RemoteStage::Teardown;
        let close = self.begin(PacketKind::Close, stage)?;
        self.string(&handle.0, stage)?;
        self.send(stage).await?;
        let remove = self.begin(PacketKind::Remove, stage)?;
        self.string(path_bytes(path), stage)?;
        self.send(stage).await?;
        let mut failure = None;
        self.status(Some(close), &mut failure).await?;
        self.status(Some(remove), &mut failure).await?;
        failure.map_or(Ok(()), Err)
    }

    /// One directory with permission bits `mode`; an existing name is
    /// refused by the server.
    pub(crate) async fn mkdir(&mut self, path: &Path, mode: u32) -> Result<(), Fault> {
        let stage = RemoteStage::Transfer;
        let id = self.begin(PacketKind::Mkdir, stage)?;
        self.string(path_bytes(path), stage)?;
        self.attrs_permissions(mode);
        self.reply(id, PacketKind::Status, stage).await?.end()
    }

    /// One empty directory.
    pub(crate) async fn rmdir(&mut self, path: &Path) -> Result<(), Fault> {
        let stage = RemoteStage::Transfer;
        let id = self.begin(PacketKind::Rmdir, stage)?;
        self.string(path_bytes(path), stage)?;
        self.reply(id, PacketKind::Status, stage).await?.end()
    }

    /// Create `path` only if nothing is there (`O_EXCL`), private until
    /// its mode is set.
    pub(crate) async fn open_exclusive(&mut self, path: &Path) -> Result<FileHandle, Fault> {
        let stage = RemoteStage::Open;
        let id = self.begin(PacketKind::Open, stage)?;
        self.string(path_bytes(path), stage)?;
        // SSH_FXF_WRITE | SSH_FXF_CREAT | SSH_FXF_EXCL
        self.request
            .extend_from_slice(&(0x02_u32 | 0x08 | 0x20).to_be_bytes());
        self.attrs_permissions(0o600);
        self.send(stage).await?;
        self.receive(stage).await?;
        self.decode_handle(id, stage)
    }

    /// Consume one STATUS reply, keeping the first failure.
    async fn status(
        &mut self,
        id: Option<RequestId>,
        failure: &mut Option<Fault>,
    ) -> Result<(), Fault> {
        let Some(id) = id else { return Ok(()) };
        let stage = RemoteStage::Transfer;
        self.receive(stage).await?;
        if let Err(fault) = self
            .decode(id, PacketKind::Status, stage)
            .and_then(|reply| reply.end())
        {
            failure.get_or_insert(fault);
        }
        Ok(())
    }

    /// `posix-rename@openssh.com` of `from` over `to`, with the
    /// published file's attributes in the same round trip. Only for a
    /// server that [`Self::overwrites_on_rename`].
    pub(crate) async fn publish(&mut self, from: &Path, to: &Path) -> Result<Attrs, Fault> {
        let stage = RemoteStage::Transfer;
        debug_assert!(self.overwrites_on_rename(), "publish needs posix-rename");
        let renamed = self.begin(PacketKind::Extended, stage)?;
        self.string(b"posix-rename@openssh.com", stage)?;
        self.string(path_bytes(from), stage)?;
        self.string(path_bytes(to), stage)?;
        self.send(stage).await?;
        let stat = self.begin(PacketKind::Lstat, stage)?;
        self.string(path_bytes(to), stage)?;
        self.send(stage).await?;
        self.receive(stage).await?;
        let renamed = self
            .decode(renamed, PacketKind::Status, stage)
            .and_then(|reply| reply.end());
        self.receive(stage).await?;
        let mut reply = self.decode(stat, PacketKind::Attrs, stage)?;
        let attrs = parse_attrs(&mut reply)?;
        reply.end()?;
        renamed.map(|()| attrs)
    }
}

/// One request of a whole-file read in flight, answered in order.
enum Flight {
    Read {
        id: RequestId,
        offset: usize,
        count: usize,
    },
    Probe(RequestId),
    Fstat(RequestId),
    Close(RequestId),
}
