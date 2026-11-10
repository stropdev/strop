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

    /// Open, prove a length within `max`, read the whole file with
    /// pipelined requests and close. The attributes come from the open
    /// handle, so they describe exactly the bytes returned.
    pub(crate) async fn read_whole(
        &mut self,
        path: &Path,
        max: u64,
    ) -> Result<(Vec<u8>, DeployAttrs), Fault> {
        let handle = self.client.open(path).await?;
        let outcome = async {
            let attrs = self.fstat_full(&handle).await?;
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
            let bytes = self.read_pipelined(&handle, size).await?;
            Ok((bytes, attrs))
        }
        .await;
        self.settle(handle, outcome).await
    }

    async fn fstat_full(&mut self, handle: &FileHandle) -> Result<DeployAttrs, Fault> {
        let id = self.client.begin(PacketKind::Fstat, ReadStage::Inspect)?;
        self.client.string(&handle.0, ReadStage::Inspect)?;
        let mut reply = self
            .client
            .reply(id, PacketKind::Attrs, ReadStage::Inspect)
            .await?;
        let attrs = parse_deploy_attrs(&mut reply)?;
        reply.end()?;
        Ok(attrs)
    }

    async fn read_pipelined(&mut self, handle: &FileHandle, size: u64) -> Result<Vec<u8>, Fault> {
        let stage = ReadStage::Transfer;
        let length = usize::try_from(size)
            .map_err(|_| Fault::new(stage, ReadFailureKind::TooLarge, "file exceeds memory"))?;
        let mut bytes = vec![0_u8; length];
        let mut pending: VecDeque<(RequestId, usize, usize)> = VecDeque::new();
        let mut next = 0_usize;
        loop {
            while pending.len() < WINDOW && next < length {
                let count = READ_CHUNK.min(length - next);
                pending.push_back((self.send_read(handle, next, count).await?, next, count));
                next += count;
            }
            let Some((id, offset, count)) = pending.pop_front() else {
                return Ok(bytes);
            };
            self.client.receive(stage).await?;
            let mut reply = self.client.decode(id, PacketKind::Data, stage)?;
            let data = reply.string()?;
            if data.is_empty() || data.len() > count {
                return Err(Fault::new(
                    stage,
                    ReadFailureKind::ShortRead,
                    "the file changed length while it was read",
                ));
            }
            bytes[offset..offset + data.len()].copy_from_slice(data);
            let got = data.len();
            reply.end()?;
            // A short chunk is protocol-legal: ask for the rest, which
            // queues behind every reply already in flight.
            if got < count {
                let rest = (
                    self.send_read(handle, offset + got, count - got).await?,
                    offset + got,
                    count - got,
                );
                pending.push_back(rest);
            }
        }
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

    /// Create `path` (private, truncated) holding exactly `bytes` with
    /// permission bits `mode`: one round trip for the handle, then every
    /// write, the close and the mode change pipelined behind it. The
    /// first failure is reported after every reply is consumed, so the
    /// session stays in step.
    pub(crate) async fn upload(
        &mut self,
        path: &Path,
        bytes: &[u8],
        mode: u32,
    ) -> Result<(), Fault> {
        let stage = ReadStage::Transfer;
        let handle = self.open_write(path).await?;
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
