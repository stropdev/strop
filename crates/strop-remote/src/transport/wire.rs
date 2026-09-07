//! SFTP v3 read-only wire contract (draft-ietf-secsh-filexfer-02).
//! SSH owns authentication/encryption; this codec owns native filename bytes,
//! bounded packets and request identity. No path is interpreted by a shell.
use super::error::{Fault, ReadFailureKind, ReadStage};
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;

const MAX_PACKET: usize = 256 * 1024;
const READ_CHUNK: usize = 32 * 1024;
const MAX_SNAPSHOT: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy)]
#[repr(u8)]
enum PacketKind {
    Init = 1,
    Version = 2,
    Open = 3,
    Close = 4,
    Read = 5,
    Fstat = 8,
    Status = 101,
    Handle = 102,
    Data = 103,
    Attrs = 105,
}
#[derive(Clone, Copy)]
struct RequestId(u32);
pub(super) struct FileHandle(Vec<u8>);
pub(super) struct SnapshotSize(u64);

pub(super) struct ReadOnlySftp<W, R> {
    input: W,
    output: R,
    request: Vec<u8>,
    response: Vec<u8>,
    sequence: RequestId,
}
impl<W: AsyncWrite + Unpin, R: AsyncRead + Unpin> ReadOnlySftp<W, R> {
    pub(super) async fn connect(input: W, output: R) -> Result<Self, Fault> {
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
        // Extension names and values are opaque strings; framing still matters.
        while !reply.remaining.is_empty() {
            reply.string()?;
            reply.string()?;
        }
        Ok(client)
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

    pub(super) async fn open(&mut self, path: &Path) -> Result<FileHandle, Fault> {
        use std::os::unix::ffi::OsStrExt;
        let id = self.begin(PacketKind::Open, ReadStage::Open)?;
        self.string(path.as_os_str().as_bytes(), ReadStage::Open)?;
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

    pub(super) async fn inspect(&mut self, handle: &FileHandle) -> Result<SnapshotSize, Fault> {
        let id = self.begin(PacketKind::Fstat, ReadStage::Inspect)?;
        self.string(&handle.0, ReadStage::Inspect)?;
        let mut reply = self
            .reply(id, PacketKind::Attrs, ReadStage::Inspect)
            .await?;
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
        reply.end()?;
        if !permissions.is_some_and(|mode| mode & 0xf000 == 0x8000) {
            return Err(Fault::new(
                ReadStage::Inspect,
                ReadFailureKind::NotRegularFile,
                "opened handle is not a proven regular file",
            ));
        }
        let size = size.ok_or_else(|| {
            Fault::new(
                ReadStage::Inspect,
                ReadFailureKind::UnknownLength,
                "server reported no file length",
            )
        })?;
        if size > MAX_SNAPSHOT {
            return Err(Fault::new(
                ReadStage::Inspect,
                ReadFailureKind::TooLarge,
                format!("{size} bytes exceeds the {MAX_SNAPSHOT} byte snapshot cap"),
            ));
        }
        Ok(SnapshotSize(size))
    }

    pub(super) async fn read(
        &mut self,
        handle: &FileHandle,
        size: SnapshotSize,
    ) -> Result<Vec<u8>, Fault> {
        let mut snapshot = Vec::with_capacity(size.0 as usize);
        while (snapshot.len() as u64) < size.0 {
            let count = READ_CHUNK.min(size.0 as usize - snapshot.len());
            let id = self.begin(PacketKind::Read, ReadStage::Transfer)?;
            self.string(&handle.0, ReadStage::Transfer)?;
            self.request
                .extend_from_slice(&(snapshot.len() as u64).to_be_bytes());
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
