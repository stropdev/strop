//! SFTP v3 read-only wire contract (draft-ietf-secsh-filexfer-02).
//! SSH owns authentication/encryption; this codec owns native filename bytes,
//! bounded packets, request identity, advertised extensions and directory
//! enumeration. No path is ever interpreted by a shell.
use super::error::{Fault, ReadFailureKind, ReadStage};
use crate::address::uri::path_bytes;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;

const MAX_PACKET: usize = 256 * 1024;
const READ_CHUNK: usize = 32 * 1024;
const MAX_SNAPSHOT: u64 = 256 * 1024 * 1024;
/// Hard bound on one directory listing: a page is already packet-bounded;
/// this caps the accumulated result across pages.
pub(super) const MAX_ENTRIES: usize = 100_000;
/// Sanity bound for one entry filename (native bytes).
const MAX_NAME: usize = 4096;

#[derive(Clone, Copy)]
#[repr(u8)]
enum PacketKind {
    Init = 1,
    Version = 2,
    Open = 3,
    Close = 4,
    Read = 5,
    Opendir = 11,
    Readdir = 12,
    Fstat = 8,
    Stat = 17,
    Status = 101,
    Handle = 102,
    Data = 103,
    Name = 104,
    Attrs = 105,
    Extended = 200,
}
#[derive(Clone, Copy)]
struct RequestId(u32);
pub(super) struct FileHandle(Vec<u8>);

/// The attributes this codec consumes: an optional size and an optional
/// POSIX permissions word, both validated for supported flag bits.
pub(super) struct Attrs {
    pub(super) size: Option<u64>,
    pub(super) permissions: Option<u32>,
}

/// Extension names the server advertised in its VERSION reply. Opaque
/// strings; offering is decided by exact byte equality.
pub(crate) struct Advertised(Vec<Vec<u8>>);

impl Advertised {
    pub(super) fn offers(&self, name: &[u8]) -> bool {
        self.0.iter().any(|advertised| advertised == name)
    }
}

/// One directory entry as the server spelled it: the native filename and
/// size and POSIX mode attributes when carried.
pub(super) struct RawEntry {
    pub(super) name: PathBuf,
    pub(super) permissions: Option<u32>,
    pub(super) size: Option<u64>,
}

/// One READDIR page: entries, or the clean end-of-directory status.
pub(super) enum Page {
    Entries(Vec<RawEntry>),
    End,
}

pub(crate) struct ReadOnlySftp<W, R> {
    input: W,
    output: R,
    request: Vec<u8>,
    response: Vec<u8>,
    sequence: RequestId,
}

impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> ReadOnlySftp<W, R> {
    /// Negotiate SFTP v3 and capture the advertised extension names.
    pub(super) async fn connect(input: W, output: R) -> Result<(Self, Advertised), Fault> {
        let mut client = Self {
            input,
            output,
            request: Vec::new(),
            response: Vec::new(),
            sequence: RequestId(0),
        };
        client.request.push(PacketKind::Init as u8);
        client.request.extend_from_slice(&3_u32.to_be_bytes());
        client.exchange(ReadStage::Connect).await?;
        let mut reply = Decoder::new(&client.response, ReadStage::Connect);
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
        Ok((client, Advertised(advertised)))
    }

    fn begin(&mut self, kind: PacketKind, stage: ReadStage) -> Result<RequestId, Fault> {
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
    fn string(&mut self, bytes: &[u8], stage: ReadStage) -> Result<(), Fault> {
        let length =
            u32::try_from(bytes.len()).map_err(|_| protocol(stage, "SFTP string is too long"))?;
        self.request.extend_from_slice(&length.to_be_bytes());
        self.request.extend_from_slice(bytes);
        Ok(())
    }
    async fn exchange(&mut self, stage: ReadStage) -> Result<(), Fault> {
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
        self.input.flush().await.map_err(|e| io_fault(stage, e))?;
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
        stage: ReadStage,
    ) -> Result<Decoder<'_>, Fault> {
        self.exchange(stage).await?;
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
                    1 => ReadFailureKind::ShortRead,
                    2 => ReadFailureKind::NotFound,
                    3 => ReadFailureKind::Permission,
                    20 => ReadFailureKind::NotDirectory,
                    _ => ReadFailureKind::Protocol,
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
    async fn readdir_reply(&mut self, id: RequestId, stage: ReadStage) -> Result<Page, Fault> {
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
                        2 => ReadFailureKind::NotFound,
                        _ => ReadFailureKind::Permission,
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

    pub(super) async fn open(&mut self, path: &Path) -> Result<FileHandle, Fault> {
        let id = self.begin(PacketKind::Open, ReadStage::Open)?;
        self.string(path_bytes(path), ReadStage::Open)?;
        self.request.extend_from_slice(&1_u32.to_be_bytes()); // SSH_FXF_READ only
        self.request.extend_from_slice(&0_u32.to_be_bytes()); // no creation attributes
        let mut reply = self.reply(id, PacketKind::Handle, ReadStage::Open).await?;
        let handle = reply.string()?;
        if handle.is_empty() || handle.len() > 1024 {
            return Err(reply.invalid("invalid SFTP file handle length"));
        }
        let handle = FileHandle(handle.to_vec());
        reply.end()?;
        Ok(handle)
    }

    pub(super) async fn opendir(&mut self, path: &Path) -> Result<FileHandle, Fault> {
        let id = self.begin(PacketKind::Opendir, ReadStage::Open)?;
        self.string(path_bytes(path), ReadStage::Open)?;
        let mut reply = self.reply(id, PacketKind::Handle, ReadStage::Open).await?;
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
        let id = self.begin(PacketKind::Stat, ReadStage::Inspect)?;
        self.string(path_bytes(path), ReadStage::Inspect)?;
        let mut reply = self
            .reply(id, PacketKind::Attrs, ReadStage::Inspect)
            .await?;
        let attrs = parse_attrs(&mut reply)?;
        reply.end()?;
        Ok(attrs)
    }

    pub(super) async fn fstat(&mut self, handle: &FileHandle) -> Result<Attrs, Fault> {
        let id = self.begin(PacketKind::Fstat, ReadStage::Inspect)?;
        self.string(&handle.0, ReadStage::Inspect)?;
        let mut reply = self
            .reply(id, PacketKind::Attrs, ReadStage::Inspect)
            .await?;
        let attrs = parse_attrs(&mut reply)?;
        reply.end()?;
        Ok(attrs)
    }

    /// One page of a directory listing. `End` is the clean terminator.
    pub(super) async fn readdir(&mut self, handle: &FileHandle) -> Result<Page, Fault> {
        let id = self.begin(PacketKind::Readdir, ReadStage::Transfer)?;
        self.string(&handle.0, ReadStage::Transfer)?;
        self.readdir_reply(id, ReadStage::Transfer).await
    }

    /// `expand-path@openssh.com`: the only sanctioned home expansion. The
    /// OpenSSH PROTOCOL §4.9 specifies a REALPATH-format NAME reply,
    /// not EXTENDED_REPLY: count, filename, longname, attributes.
    pub(super) async fn expand_path(&mut self, path: &Path) -> Result<PathBuf, Fault> {
        let id = self.begin(PacketKind::Extended, ReadStage::Open)?;
        self.string(b"expand-path@openssh.com", ReadStage::Open)?;
        self.string(path_bytes(path), ReadStage::Open)?;
        let mut reply = self.reply(id, PacketKind::Name, ReadStage::Open).await?;
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

    /// Read exactly `length` bytes starting at `offset`. Short chunks are
    /// protocol-legal; fewer bytes than requested before a clean end of
    /// file is a short read, and nothing beyond `length` is accepted.
    pub(super) async fn read_at(
        &mut self,
        handle: &FileHandle,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, Fault> {
        let mut snapshot = Vec::with_capacity(length.min(MAX_SNAPSHOT) as usize);
        let mut position = offset;
        while (snapshot.len() as u64) < length {
            let count = READ_CHUNK.min((length - snapshot.len() as u64) as usize);
            let id = self.begin(PacketKind::Read, ReadStage::Transfer)?;
            self.string(&handle.0, ReadStage::Transfer)?;
            self.request.extend_from_slice(&position.to_be_bytes());
            self.request
                .extend_from_slice(&(count as u32).to_be_bytes());
            let mut reply = self
                .reply(id, PacketKind::Data, ReadStage::Transfer)
                .await?;
            let data = reply.string()?;
            if data.len() > count {
                return Err(reply.invalid("SFTP server sent more bytes than requested"));
            }
            if data.is_empty() {
                return Err(Fault::new(
                    ReadStage::Transfer,
                    ReadFailureKind::ShortRead,
                    "SFTP server returned no data before captured end of file",
                ));
            }
            snapshot.extend_from_slice(data);
            position += data.len() as u64;
            reply.end()?;
        }
        Ok(snapshot)
    }

    pub(super) async fn close(&mut self, handle: FileHandle) -> Result<(), Fault> {
        let id = self.begin(PacketKind::Close, ReadStage::Teardown)?;
        self.string(&handle.0, ReadStage::Teardown)?;
        self.reply(id, PacketKind::Status, ReadStage::Teardown)
            .await?
            .end()
    }
}

/// Decode one ATTRS block: size/permissions kept, everything else in the
/// supported flag set skipped by bounds-checked arithmetic.
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
    if flags & 8 != 0 {
        reply.take(8)?;
    } // atime/mtime
    if flags & 0x8000_0000 != 0 {
        let count = reply.number()?;
        for _ in 0..count {
            reply.string()?;
            reply.string()?;
        }
    }
    Ok(Attrs { size, permissions })
}

/// Entry and expansion results are native bytes; platforms without byte
/// filenames get strict UTF-8, never a lossy stand-in.
fn native_name(bytes: &[u8]) -> Result<PathBuf, Fault> {
    crate::address::uri::bytes_to_path(bytes.to_vec())
        .map_err(|error| protocol(ReadStage::Transfer, error.to_string()))
}

fn protocol(stage: ReadStage, detail: impl Into<String>) -> Fault {
    Fault::new(stage, ReadFailureKind::Protocol, detail)
}
fn io_fault(stage: ReadStage, error: std::io::Error) -> Fault {
    if stage == ReadStage::Connect {
        Fault::connect(error.to_string())
    } else {
        Fault::new(stage, ReadFailureKind::Io, error.to_string())
    }
}
struct Decoder<'a> {
    remaining: &'a [u8],
    stage: ReadStage,
}
impl<'a> Decoder<'a> {
    fn new(remaining: &'a [u8], stage: ReadStage) -> Self {
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
