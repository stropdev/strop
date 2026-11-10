//! SFTP v3 wire contract (draft-ietf-secsh-filexfer-02). SSH owns
//! authentication/encryption; this codec owns native filename bytes,
//! bounded packets, request identity, advertised extensions, directory
//! enumeration and the whole-file reads and saves of `wire_files.rs`.
//! No path is ever interpreted by a shell.
use super::error::{Fault, RemoteFailureKind, RemoteStage};
use std::path::{Path, PathBuf};
use strop_workspace::addr::uri::path_bytes;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[path = "wire_files.rs"]
mod files;
#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;

const MAX_PACKET: usize = 256 * 1024;
const READ_CHUNK: usize = 32 * 1024;
/// Hard bound on one directory listing: a page is already packet-bounded;
/// this caps the accumulated result across pages.
pub(super) const MAX_ENTRIES: usize = 100_000;
/// Sanity bound for one entry filename (native bytes).
const MAX_NAME: usize = 4096;
/// One write request's data ceiling: the packet stays well under
/// MAX_PACKET with the largest legal handle attached.
const WRITE_CHUNK: usize = 192 * 1024;

#[derive(Clone, Copy)]
#[repr(u8)]
enum PacketKind {
    Init = 1,
    Version = 2,
    Open = 3,
    Close = 4,
    Read = 5,
    Write = 6,
    Lstat = 7,
    Fstat = 8,
    Setstat = 9,
    Opendir = 11,
    Readdir = 12,
    Remove = 13,
    Mkdir = 14,
    Rmdir = 15,
    Stat = 17,
    Rename = 18,
    Status = 101,
    Handle = 102,
    Data = 103,
    Name = 104,
    Attrs = 105,
    Extended = 200,
}
#[derive(Clone, Copy)]
struct RequestId(u32);
pub(crate) struct FileHandle(Vec<u8>);

/// The attributes this codec consumes, each validated for supported
/// flag bits: size, the POSIX mode word and the modification time.
pub(crate) struct Attrs {
    pub(crate) size: Option<u64>,
    pub(crate) permissions: Option<u32>,
    /// Whole seconds: SFTP v3 carries no finer modification time.
    pub(crate) mtime: Option<u32>,
}

/// One directory entry as the server spelled it: the native filename and
/// size and POSIX mode attributes when carried.
pub(crate) struct RawEntry {
    pub(crate) name: PathBuf,
    pub(crate) permissions: Option<u32>,
    pub(crate) size: Option<u64>,
}

/// One READDIR page: entries, or the clean end-of-directory status.
pub(super) enum Page {
    Entries(Vec<RawEntry>),
    End,
}

/// One SFTP v3 session over a byte stream pair.
pub(crate) struct Sftp<W, R> {
    input: W,
    output: R,
    request: Vec<u8>,
    response: Vec<u8>,
    sequence: RequestId,
    /// Extension names the server advertised in its VERSION reply.
    /// Opaque strings; offering is decided by exact byte equality.
    advertised: Vec<Vec<u8>>,
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> Sftp<W, R> {
    /// Negotiate SFTP v3 and capture the advertised extension names.
    pub(crate) async fn connect(input: W, output: R) -> Result<Self, Fault> {
        let mut client = Self {
            input,
            output,
            request: Vec::new(),
            response: Vec::new(),
            sequence: RequestId(0),
            advertised: Vec::new(),
        };
        client.request.push(PacketKind::Init as u8);
        client.request.extend_from_slice(&3_u32.to_be_bytes());
        client.exchange(RemoteStage::Connect).await?;
        let mut reply = Decoder::new(&client.response, RemoteStage::Connect);
        if reply.take(1)? != [PacketKind::Version as u8] || reply.number()? != 3 {
            return Err(reply.invalid("server must support SFTP version 3"));
        }
        let mut advertised = Vec::new();
        // Extension names and values are opaque strings; framing still
        // matters, and only names are retained.
        while !reply.remaining.is_empty() {
            let name = reply.string()?.to_vec();
            reply.string()?;
            advertised.push(name);
        }
        client.advertised = advertised;
        Ok(client)
    }

    pub(crate) fn offers(&self, name: &[u8]) -> bool {
        self.advertised.iter().any(|advertised| advertised == name)
    }

    fn begin(&mut self, kind: PacketKind, stage: RemoteStage) -> Result<RequestId, Fault> {
        self.sequence.0 = self
            .sequence
            .0
            .checked_add(1)
            .ok_or_else(|| protocol(stage, "SFTP request identity exhausted"))?;
        self.request.clear();
        self.request.push(kind as u8);
        self.request
            .extend_from_slice(&self.sequence.0.to_be_bytes());
        Ok(self.sequence)
    }
    fn string(&mut self, bytes: &[u8], stage: RemoteStage) -> Result<(), Fault> {
        let length =
            u32::try_from(bytes.len()).map_err(|_| protocol(stage, "SFTP string is too long"))?;
        self.request.extend_from_slice(&length.to_be_bytes());
        self.request.extend_from_slice(bytes);
        Ok(())
    }
    async fn exchange(&mut self, stage: RemoteStage) -> Result<(), Fault> {
        self.send(stage).await?;
        self.receive(stage).await
    }
    /// Write the framed request without awaiting its reply (pipelining
    /// sends several before [`Self::receive`]-ing them in order).
    async fn send(&mut self, stage: RemoteStage) -> Result<(), Fault> {
        if self.request.len() > MAX_PACKET {
            return Err(protocol(stage, "SFTP request exceeds packet limit"));
        }
        self.input
            .write_all(&(self.request.len() as u32).to_be_bytes())
            .await
            .map_err(|e| io_fault(stage, e))?;
        self.input
            .write_all(&self.request)
            .await
            .map_err(|e| io_fault(stage, e))?;
        self.input.flush().await.map_err(|e| io_fault(stage, e))
    }
    /// Read one framed response into the response buffer.
    async fn receive(&mut self, stage: RemoteStage) -> Result<(), Fault> {
        let length = self
            .output
            .read_u32()
            .await
            .map_err(|e| io_fault(stage, e))? as usize;
        if length == 0 || length > MAX_PACKET {
            return Err(protocol(
                stage,
                format!("SFTP response length {length} exceeds framing bounds"),
            ));
        }
        self.response.resize(length, 0);
        self.output
            .read_exact(&mut self.response)
            .await
            .map_err(|e| io_fault(stage, e))?;
        Ok(())
    }
    async fn reply(
        &mut self,
        id: RequestId,
        expected: PacketKind,
        stage: RemoteStage,
    ) -> Result<Decoder<'_>, Fault> {
        self.exchange(stage).await?;
        self.decode(id, expected, stage)
    }
    /// Validate the received response as the reply to `id`: an error
    /// status becomes a classified fault, anything but `expected` a
    /// protocol fault.
    fn decode(
        &self,
        id: RequestId,
        expected: PacketKind,
        stage: RemoteStage,
    ) -> Result<Decoder<'_>, Fault> {
        let mut reply = Decoder::new(&self.response, stage);
        let kind = reply.take(1)?[0];
        if reply.number()? != id.0 {
            return Err(reply.invalid("SFTP response belongs to a different request"));
        }
        if kind == PacketKind::Status as u8 {
            let code = reply.number()?;
            let message = reply.string()?;
            reply.string()?; // language tag
            reply.end()?;
            if code != 0 {
                let kind = match code {
                    1 => RemoteFailureKind::ShortRead,
                    2 => RemoteFailureKind::NotFound,
                    3 => RemoteFailureKind::Permission,
                    4 => RemoteFailureKind::Failure,
                    20 => RemoteFailureKind::NotDirectory,
                    _ => RemoteFailureKind::Protocol,
                };
                return Err(Fault::new(
                    stage,
                    kind,
                    format!("SFTP status {code}: {}", String::from_utf8_lossy(message)),
                ));
            }
        }
        if kind != expected as u8 {
            return Err(reply.invalid(format!("unexpected SFTP response type {kind}")));
        }
        Ok(reply)
    }

    /// The REaddir exchange: SSH_FX_EOF ends the listing cleanly instead of
    /// being a short read. Returns the decoded ATTRS-less page or `End`.
    async fn readdir_reply(&mut self, id: RequestId, stage: RemoteStage) -> Result<Page, Fault> {
        self.exchange(stage).await?;
        let mut reply = Decoder::new(&self.response, stage);
        let kind = reply.take(1)?[0];
        if reply.number()? != id.0 {
            return Err(reply.invalid("SFTP response belongs to a different request"));
        }
        if kind == PacketKind::Status as u8 {
            let code = reply.number()?;
            reply.string()?;
            reply.string()?;
            reply.end()?;
            return match code {
                1 => Ok(Page::End),
                2 | 3 => Err(Fault::new(
                    stage,
                    match code {
                        2 => RemoteFailureKind::NotFound,
                        _ => RemoteFailureKind::Permission,
                    },
                    format!("SFTP status {code} while listing"),
                )),
                _ => Err(reply.invalid(format!("unexpected SFTP status {code} while listing"))),
            };
        }
        if kind != PacketKind::Name as u8 {
            return Err(reply.invalid(format!("unexpected SFTP response type {kind}")));
        }
        let count = reply.number()? as usize;
        if count == 0 || count > MAX_ENTRIES {
            return Err(reply.invalid("SFTP NAME count outside sane bounds"));
        }
        let mut entries = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let name = reply.string()?;
            if name.is_empty() || name.len() > MAX_NAME || name.contains(&0) || name.contains(&b'/')
            {
                return Err(reply.invalid("SFTP entry name outside sane bounds"));
            }
            reply.string()?; // longname: human text, ignored
            let attrs = parse_attrs(&mut reply)?;
            entries.push(RawEntry {
                name: native_name(name)?,
                permissions: attrs.permissions,
                size: attrs.size,
            });
        }
        reply.end()?;
        Ok(Page::Entries(entries))
    }

    pub(super) async fn opendir(&mut self, path: &Path) -> Result<FileHandle, Fault> {
        let id = self.begin(PacketKind::Opendir, RemoteStage::Open)?;
        self.string(path_bytes(path), RemoteStage::Open)?;
        let mut reply = self
            .reply(id, PacketKind::Handle, RemoteStage::Open)
            .await?;
        let handle = reply.string()?;
        if handle.is_empty() || handle.len() > 1024 {
            return Err(reply.invalid("invalid SFTP directory handle length"));
        }
        let handle = FileHandle(handle.to_vec());
        reply.end()?;
        Ok(handle)
    }

    /// Path-based attributes, without any handle: how `open` decides a
    /// file-or-directory target before opening anything.
    pub(super) async fn stat(&mut self, path: &Path) -> Result<Attrs, Fault> {
        let id = self.begin(PacketKind::Stat, RemoteStage::Inspect)?;
        self.string(path_bytes(path), RemoteStage::Inspect)?;
        let mut reply = self
            .reply(id, PacketKind::Attrs, RemoteStage::Inspect)
            .await?;
        let attrs = parse_attrs(&mut reply)?;
        reply.end()?;
        Ok(attrs)
    }

    /// One page of a directory listing. `End` is the clean terminator.
    pub(super) async fn readdir(&mut self, handle: &FileHandle) -> Result<Page, Fault> {
        let id = self.begin(PacketKind::Readdir, RemoteStage::Transfer)?;
        self.string(&handle.0, RemoteStage::Transfer)?;
        self.readdir_reply(id, RemoteStage::Transfer).await
    }

    /// `expand-path@openssh.com`: the only sanctioned home expansion. The
    /// OpenSSH PROTOCOL §4.9 specifies a REALPATH-format NAME reply,
    /// not EXTENDED_REPLY: count, filename, longname, attributes.
    pub(super) async fn expand_path(&mut self, path: &Path) -> Result<PathBuf, Fault> {
        let id = self.begin(PacketKind::Extended, RemoteStage::Open)?;
        self.string(b"expand-path@openssh.com", RemoteStage::Open)?;
        self.string(path_bytes(path), RemoteStage::Open)?;
        let mut reply = self.reply(id, PacketKind::Name, RemoteStage::Open).await?;
        if reply.number()? != 1 {
            return Err(reply.invalid("path expansion must return exactly one name"));
        }
        let expanded = reply.string()?;
        if expanded.is_empty() || expanded.len() > MAX_NAME || expanded.contains(&0) {
            return Err(reply.invalid("expanded path outside sane bounds"));
        }
        let expanded = native_name(expanded)?;
        reply.string()?; // human longname is not the path identity
        parse_attrs(&mut reply)?;
        reply.end()?;
        Ok(expanded)
    }

    pub(super) async fn close(&mut self, handle: FileHandle) -> Result<(), Fault> {
        let id = self.begin(PacketKind::Close, RemoteStage::Teardown)?;
        self.string(&handle.0, RemoteStage::Teardown)?;
        self.reply(id, PacketKind::Status, RemoteStage::Teardown)
            .await?
            .end()
    }
    /// Append one ATTRS block carrying only permission bits (mkdir,
    /// open-create and setstat all send exactly this shape).
    fn attrs_permissions(&mut self, mode: u32) {
        self.request.extend_from_slice(&4_u32.to_be_bytes());
        self.request.extend_from_slice(&mode.to_be_bytes());
    }
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> Sftp<W, R> {
    /// Symlink-conscious attributes (SSH_FXP_LSTAT never follows the
    /// final component).
    pub(crate) async fn lstat(&mut self, path: &Path) -> Result<Attrs, Fault> {
        let id = self.begin(PacketKind::Lstat, RemoteStage::Inspect)?;
        self.string(path_bytes(path), RemoteStage::Inspect)?;
        let mut reply = self
            .reply(id, PacketKind::Attrs, RemoteStage::Inspect)
            .await?;
        let attrs = parse_attrs(&mut reply)?;
        reply.end()?;
        Ok(attrs)
    }

    /// SFTP v3 rename, which refuses an existing destination.
    pub(crate) async fn rename(&mut self, from: &Path, to: &Path) -> Result<(), Fault> {
        let stage = RemoteStage::Transfer;
        let id = self.begin(PacketKind::Rename, stage)?;
        self.string(path_bytes(from), stage)?;
        self.string(path_bytes(to), stage)?;
        self.reply(id, PacketKind::Status, stage).await?.end()
    }

    /// Remove one file. Never recursive.
    pub(crate) async fn remove(&mut self, path: &Path) -> Result<(), Fault> {
        let id = self.begin(PacketKind::Remove, RemoteStage::Teardown)?;
        self.string(path_bytes(path), RemoteStage::Teardown)?;
        self.reply(id, PacketKind::Status, RemoteStage::Teardown)
            .await?
            .end()
    }
}

/// Decode one ATTRS block: size, mode and mtime kept, everything else in
/// the supported flag set skipped by bounds-checked arithmetic.
fn parse_attrs(reply: &mut Decoder<'_>) -> Result<Attrs, Fault> {
    let flags = reply.number()?;
    if flags & !0x8000_000f != 0 {
        return Err(reply.invalid("unsupported SFTP v3 attribute flags"));
    }
    let size = if flags & 1 != 0 {
        Some(reply.wide_number()?)
    } else {
        None
    };
    if flags & 2 != 0 {
        reply.take(8)?;
    } // uid/gid
    let permissions = if flags & 4 != 0 {
        Some(reply.number()?)
    } else {
        None
    };
    let mtime = if flags & 8 != 0 {
        reply.number()?; // atime
        Some(reply.number()?)
    } else {
        None
    };
    if flags & 0x8000_0000 != 0 {
        let count = reply.number()?;
        for _ in 0..count {
            reply.string()?;
            reply.string()?;
        }
    }
    Ok(Attrs {
        size,
        permissions,
        mtime,
    })
}

/// Entry and expansion results are native bytes; platforms without byte
/// filenames get strict UTF-8, never a lossy stand-in.
fn native_name(bytes: &[u8]) -> Result<PathBuf, Fault> {
    strop_workspace::addr::uri::bytes_to_path(bytes.to_vec())
        .map_err(|error| protocol(RemoteStage::Transfer, error.to_string()))
}

fn protocol(stage: RemoteStage, detail: impl Into<String>) -> Fault {
    Fault::new(stage, RemoteFailureKind::Protocol, detail)
}
fn io_fault(stage: RemoteStage, error: std::io::Error) -> Fault {
    if stage == RemoteStage::Connect {
        Fault::connect(error.to_string())
    } else {
        Fault::new(stage, RemoteFailureKind::Io, error.to_string())
    }
}
struct Decoder<'a> {
    remaining: &'a [u8],
    stage: RemoteStage,
}
impl<'a> Decoder<'a> {
    fn new(remaining: &'a [u8], stage: RemoteStage) -> Self {
        Self { remaining, stage }
    }
    fn invalid(&self, detail: impl Into<String>) -> Fault {
        protocol(self.stage, detail)
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], Fault> {
        if length > self.remaining.len() {
            return Err(self.invalid("truncated SFTP response"));
        }
        let (value, rest) = self.remaining.split_at(length);
        self.remaining = rest;
        Ok(value)
    }
    fn number(&mut self) -> Result<u32, Fault> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn wide_number(&mut self) -> Result<u64, Fault> {
        let high = u64::from(self.number()?);
        Ok((high << 32) | u64::from(self.number()?))
    }
    fn string(&mut self) -> Result<&'a [u8], Fault> {
        let length = self.number()? as usize;
        self.take(length)
    }
    fn end(&self) -> Result<(), Fault> {
        if self.remaining.is_empty() {
            Ok(())
        } else {
            Err(self.invalid("unexpected trailing SFTP response bytes"))
        }
    }
}
