//! Whole-file SFTP operations for editing over a multiplexed SSH
//! connection (plan 0090 §5): pipelined reads and writes, attributes with
//! the modification time, symlink-following stat and complete listings.
//!
//! Pipelining matters off loopback: one request per round trip turns a
//! 1 MB open into 32 round trips. Requests are sent up to a window ahead
//! and replies are consumed in request order — OpenSSH's `sftp-server`
//! answers in order, and a server that reorders gets an honest protocol
//! fault from [`ReadOnlySftp::decode`], never a misplaced chunk.
use super::{
    parse_deploy_attrs, DeployAttrs, DeploySftp, Fault, FileHandle, PacketKind, Page, RawEntry,
    ReadFailureKind, ReadStage, RequestId, MAX_ENTRIES, READ_CHUNK, WRITE_CHUNK,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use strop_workspace::addr::uri::path_bytes;
use tokio::io::{AsyncRead, AsyncWrite};

/// Requests in flight per transfer: 1 MiB of reads or 1.5 MiB of
/// writes, inside OpenSSH's 2 MiB session channel window.
const WINDOW: usize = 32;
const WRITE_WINDOW: usize = 8;
/// Re-reads of a file that changed while it was read.
const READ_ATTEMPTS: usize = 2;

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> DeploySftp<W, R> {
    /// Whether `rename` atomically replaces an existing destination.
    pub(crate) fn overwrites_on_rename(&self) -> bool {
        self.posix_rename
    }

    /// Symlink-following attributes with the modification time.
    pub(crate) async fn stat_full(&mut self, path: &Path) -> Result<DeployAttrs, Fault> {
        let id = self.client.begin(PacketKind::Stat, ReadStage::Inspect)?;
        self.client.string(path_bytes(path), ReadStage::Inspect)?;
        let mut reply = self
            .client
            .reply(id, PacketKind::Attrs, ReadStage::Inspect)
            .await?;
        let attrs = parse_deploy_attrs(&mut reply)?;
        reply.end()?;
        Ok(attrs)
    }

    /// The canonical path (`expand-path@openssh.com`: `~` and symlinks
    /// resolved by the server).
    pub(crate) async fn canonical(&mut self, path: &Path) -> Result<PathBuf, Fault> {
        self.client.expand_path(path).await
    }

    /// The whole file within `max` bytes, in two round trips: OPEN and
    /// STAT pipelined, then every READ, an end-of-file probe, FSTAT and
    /// CLOSE pipelined behind them. The attributes are the open handle's
    /// (they describe exactly the bytes returned); a file that changes
    /// while it is read is read again, a bounded number of times.
    pub(crate) async fn read_whole(
        &mut self,
        path: &Path,
        max: u64,
    ) -> Result<(Vec<u8>, DeployAttrs), Fault> {
        let mut attempt = 0;
        loop {
            match self.read_once(path, max).await? {
                Some(read) => return Ok(read),
                None if attempt < READ_ATTEMPTS => attempt += 1,
                None => {
                    return Err(Fault::new(
                        ReadStage::Transfer,
                        ReadFailureKind::ShortRead,
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
    ) -> Result<Option<(Vec<u8>, DeployAttrs)>, Fault> {
        let open = self.client.begin(PacketKind::Open, ReadStage::Open)?;
        self.client.string(path_bytes(path), ReadStage::Open)?;
        self.request_flags(0x01); // SSH_FXF_READ
        self.client.request.extend_from_slice(&0_u32.to_be_bytes());
        self.client.send(ReadStage::Open).await?;
        let stat = self.client.begin(PacketKind::Stat, ReadStage::Inspect)?;
        self.client.string(path_bytes(path), ReadStage::Inspect)?;
        self.client.send(ReadStage::Inspect).await?;
        self.client.receive(ReadStage::Open).await?;
        let handle = self.decode_handle(open, ReadStage::Open);
        self.client.receive(ReadStage::Inspect).await?;
        let attrs = self
            .client
            .decode(stat, PacketKind::Attrs, ReadStage::Inspect)
            .and_then(|mut reply| {
                let attrs = parse_deploy_attrs(&mut reply)?;
                reply.end()?;
                Ok(attrs)
            });
        let handle = handle?;
        let size = attrs.and_then(|attrs| {
            let size = attrs.size.ok_or_else(|| {
                Fault::new(
                    ReadStage::Inspect,
                    ReadFailureKind::UnknownLength,
                    "the server reported no file length",
                )
            })?;
            if size > max {
                return Err(Fault::new(
                    ReadStage::Inspect,
                    ReadFailureKind::TooLarge,
                    format!("{size} bytes exceeds the {max}-byte bound"),
                ));
            }
            Ok((size, attrs))
        });
        let (size, stated) = match size {
            Ok(size) => size,
            Err(primary) => return self.settle(handle, Err(primary)).await,
        };
        let (bytes, opened) = self.read_through(handle, size).await?;
        let unchanged = opened.size == Some(size) && opened.mtime == stated.mtime;
        Ok(bytes.filter(|_| unchanged).map(|bytes| (bytes, opened)))
    }

    fn decode_handle(&self, id: RequestId, stage: ReadStage) -> Result<FileHandle, Fault> {
        let mut reply = self.client.decode(id, PacketKind::Handle, stage)?;
        let handle = reply.string()?;
        if handle.is_empty() || handle.len() > 1024 {
            return Err(reply.invalid("invalid SFTP file handle length"));
        }
        let handle = FileHandle(handle.to_vec());
        reply.end()?;
        Ok(handle)
    }

    /// Read `size` bytes of `handle` with up to [`WINDOW`] requests in
    /// flight, then probe one byte past the end, FSTAT and CLOSE in the
    /// same flight. Every reply is consumed even after a failure so the
    /// session stays in step. The bytes are `None` when the file was
    /// shorter or longer than `size`: it changed under the read.
    async fn read_through(
        &mut self,
        handle: FileHandle,
        size: u64,
    ) -> Result<(Option<Vec<u8>>, DeployAttrs), Fault> {
        let stage = ReadStage::Transfer;
        let length = usize::try_from(size)
            .map_err(|_| Fault::new(stage, ReadFailureKind::TooLarge, "file exceeds memory"))?;
        let mut bytes = vec![0_u8; length];
        let mut pending: VecDeque<(RequestId, usize, usize)> = VecDeque::new();
        let mut next = 0_usize;
        let mut changed = false;
        let mut failure: Option<Fault> = None;
        loop {
            while pending.len() < WINDOW && next < length && !changed {
                let count = READ_CHUNK.min(length - next);
                pending.push_back((self.send_read(&handle, next, count).await?, next, count));
                next += count;
            }
            let Some((id, offset, count)) = pending.pop_front() else {
                break;
            };
            self.client.receive(stage).await?;
            let reply = self.client.decode(id, PacketKind::Data, stage);
            let got = match reply.and_then(|mut reply| {
                let data = reply.string()?;
                if data.len() > count {
                    return Err(reply.invalid("SFTP server sent more bytes than requested"));
                }
                bytes[offset..offset + data.len()].copy_from_slice(data);
                reply.end()?;
                Ok(data.len())
            }) {
                Ok(0) => {
                    changed = true;
                    continue;
                }
                Ok(got) => got,
                Err(fault) if fault.disposition().0 == ReadFailureKind::ShortRead => {
                    changed = true; // end of file before `size`
                    continue;
                }
                Err(fault) => {
                    failure.get_or_insert(fault);
                    continue;
                }
            };
            // A short chunk is protocol-legal: ask for the rest, which
            // queues behind every reply already in flight.
            if got < count && !changed && failure.is_none() {
                let rest = offset + got;
                pending.push_back((
                    self.send_read(&handle, rest, count - got).await?,
                    rest,
                    count - got,
                ));
            }
        }
        let probe = self.send_read(&handle, length, 1).await?;
        let fstat = self.client.begin(PacketKind::Fstat, stage)?;
        self.client.string(&handle.0, stage)?;
        self.client.send(stage).await?;
        let close = self.client.begin(PacketKind::Close, ReadStage::Teardown)?;
        self.client.string(&handle.0, ReadStage::Teardown)?;
        self.client.send(ReadStage::Teardown).await?;
        self.client.receive(stage).await?;
        match self.client.decode(probe, PacketKind::Data, stage) {
            Ok(_) => changed = true, // bytes past `size`: it grew
            Err(fault) if fault.disposition().0 == ReadFailureKind::ShortRead => {}
            Err(fault) => {
                failure.get_or_insert(fault);
            }
        }
        self.client.receive(stage).await?;
        let attrs = self
            .client
            .decode(fstat, PacketKind::Attrs, stage)
            .and_then(|mut reply| {
                let attrs = parse_deploy_attrs(&mut reply)?;
                reply.end()?;
                Ok(attrs)
            });
        self.client.receive(ReadStage::Teardown).await?;
        let closed = self
            .client
            .decode(close, PacketKind::Status, ReadStage::Teardown)
            .and_then(|reply| reply.end());
        if let Some(fault) = failure {
            return Err(match closed {
                Ok(()) => fault,
                Err(cleanup) => fault.with_cleanup(cleanup),
            });
        }
        closed?;
        Ok(((!changed).then_some(bytes), attrs?))
    }

    async fn send_read(
        &mut self,
        handle: &FileHandle,
        offset: usize,
        count: usize,
    ) -> Result<RequestId, Fault> {
        let stage = ReadStage::Transfer;
        let id = self.client.begin(PacketKind::Read, stage)?;
        self.client.string(&handle.0, stage)?;
        self.client
            .request
            .extend_from_slice(&(offset as u64).to_be_bytes());
        self.client
            .request
            .extend_from_slice(&(count as u32).to_be_bytes());
        self.client.send(stage).await?;
        Ok(id)
    }

    /// LSTAT `path` and create `temporary` (private, truncated) in one
    /// round trip: the save's first flight. The outer error is a broken
    /// session; the inner results are the two replies.
    pub(crate) async fn inspect_and_create(
        &mut self,
        path: &Path,
        temporary: &Path,
    ) -> Result<(Result<DeployAttrs, Fault>, Result<FileHandle, Fault>), Fault> {
        let lstat = self.client.begin(PacketKind::Lstat, ReadStage::Inspect)?;
        self.client.string(path_bytes(path), ReadStage::Inspect)?;
        self.client.send(ReadStage::Inspect).await?;
        let open = self.send_open_write(temporary).await?;
        self.client.receive(ReadStage::Inspect).await?;
        let attrs = self
            .client
            .decode(lstat, PacketKind::Attrs, ReadStage::Inspect)
            .and_then(|mut reply| {
                let attrs = parse_deploy_attrs(&mut reply)?;
                reply.end()?;
                Ok(attrs)
            });
        self.client.receive(ReadStage::Open).await?;
        Ok((attrs, self.decode_handle(open, ReadStage::Open)))
    }

    async fn send_open_write(&mut self, path: &Path) -> Result<RequestId, Fault> {
        let id = self.client.begin(PacketKind::Open, ReadStage::Open)?;
        self.client.string(path_bytes(path), ReadStage::Open)?;
        // SSH_FXF_WRITE | SSH_FXF_CREAT | SSH_FXF_TRUNC
        self.request_flags(0x02 | 0x08 | 0x10);
        self.client.attrs_permissions(0o600);
        self.client.send(ReadStage::Open).await?;
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
        self.client.receive(ReadStage::Open).await?;
        let handle = self.decode_handle(open, ReadStage::Open)?;
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
        let stage = ReadStage::Transfer;
        let mut pending: VecDeque<RequestId> = VecDeque::new();
        let mut failure = None;
        for (index, chunk) in bytes.chunks(WRITE_CHUNK).enumerate() {
            if pending.len() == WRITE_WINDOW {
                self.status(pending.pop_front(), &mut failure).await?;
            }
            let id = self.client.begin(PacketKind::Write, stage)?;
            self.client.string(&handle.0, stage)?;
            let offset = (index * WRITE_CHUNK) as u64;
            self.client.request.extend_from_slice(&offset.to_be_bytes());
            self.client.string(chunk, stage)?;
            self.client.send(stage).await?;
            pending.push_back(id);
        }
        let id = self.client.begin(PacketKind::Close, stage)?;
        self.client.string(&handle.0, stage)?;
        self.client.send(stage).await?;
        pending.push_back(id);
        let id = self.client.begin(PacketKind::Setstat, stage)?;
        self.client.string(path_bytes(path), stage)?;
        self.client.attrs_permissions(mode);
        self.client.send(stage).await?;
        pending.push_back(id);
        while !pending.is_empty() {
            self.status(pending.pop_front(), &mut failure).await?;
        }
        failure.map_or(Ok(()), Err)
    }

    /// Close the open `handle` of `path` and remove `path`, pipelined:
    /// a staged file that will not be published.
    pub(crate) async fn discard(&mut self, handle: FileHandle, path: &Path) -> Result<(), Fault> {
        let stage = ReadStage::Teardown;
        let close = self.client.begin(PacketKind::Close, stage)?;
        self.client.string(&handle.0, stage)?;
        self.client.send(stage).await?;
        let remove = self.client.begin(PacketKind::Remove, stage)?;
        self.client.string(path_bytes(path), stage)?;
        self.client.send(stage).await?;
        let mut failure = None;
        self.status(Some(close), &mut failure).await?;
        self.status(Some(remove), &mut failure).await?;
        failure.map_or(Ok(()), Err)
    }

    /// Consume one STATUS reply, keeping the first failure.
    async fn status(
        &mut self,
        id: Option<RequestId>,
        failure: &mut Option<Fault>,
    ) -> Result<(), Fault> {
        let Some(id) = id else { return Ok(()) };
        let stage = ReadStage::Transfer;
        self.client.receive(stage).await?;
        if let Err(fault) = self
            .client
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
    pub(crate) async fn publish(&mut self, from: &Path, to: &Path) -> Result<DeployAttrs, Fault> {
        let stage = ReadStage::Transfer;
        debug_assert!(self.posix_rename, "publish needs posix-rename");
        let renamed = self.client.begin(PacketKind::Extended, stage)?;
        self.client.string(b"posix-rename@openssh.com", stage)?;
        self.client.string(path_bytes(from), stage)?;
        self.client.string(path_bytes(to), stage)?;
        self.client.send(stage).await?;
        let stat = self.client.begin(PacketKind::Lstat, stage)?;
        self.client.string(path_bytes(to), stage)?;
        self.client.send(stage).await?;
        self.client.receive(stage).await?;
        let renamed = self
            .client
            .decode(renamed, PacketKind::Status, stage)
            .and_then(|reply| reply.end());
        self.client.receive(stage).await?;
        let mut reply = self.client.decode(stat, PacketKind::Attrs, stage)?;
        let attrs = parse_deploy_attrs(&mut reply)?;
        reply.end()?;
        renamed.map(|()| attrs)
    }

    /// Every entry of one directory except `.` and `..`, bounded by
    /// [`MAX_ENTRIES`].
    pub(crate) async fn list_entries(&mut self, path: &Path) -> Result<Vec<RawEntry>, Fault> {
        let handle = self.client.opendir(path).await?;
        let outcome = async {
            let mut listed = Vec::new();
            loop {
                match self.client.readdir(&handle).await? {
                    Page::Entries(entries) => {
                        listed.extend(entries.into_iter().filter(|entry| {
                            entry.name.as_os_str() != "." && entry.name.as_os_str() != ".."
                        }));
                        if listed.len() > MAX_ENTRIES {
                            return Err(Fault::new(
                                ReadStage::Transfer,
                                ReadFailureKind::TooManyEntries,
                                "directory exceeds the bounded listing cap",
                            ));
                        }
                    }
                    Page::End => return Ok(listed),
                }
            }
        }
        .await;
        self.settle(handle, outcome).await
    }

    /// Close `handle` after `outcome`: a failed close after a failure
    /// keeps the primary diagnosis and poisons the connection.
    async fn settle<T>(
        &mut self,
        handle: FileHandle,
        outcome: Result<T, Fault>,
    ) -> Result<T, Fault> {
        match outcome {
            Ok(value) => {
                self.client.close(handle).await?;
                Ok(value)
            }
            Err(primary) => match self.client.close(handle).await {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(primary.with_cleanup(cleanup)),
            },
        }
    }
}
