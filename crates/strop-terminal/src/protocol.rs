//! Local helper framing. This is not a remote execution protocol. Raw PTY bytes
//! never become shell text or an unbounded application-event payload.
use crate::Error;
use std::collections::VecDeque;
use std::io::{self, IoSlice, Read, Write};

pub const VERSION: u32 = 1;
pub const MAX_PACKET: usize = crate::model::MAX_INPUT_BYTES + 128;
pub const MAX_QUEUED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_QUEUED_PACKETS: usize = 256;
const CONTROL_RESERVE: usize = 16 * 1024;

pub const LAUNCH: u8 = 1;
pub const INPUT: u8 = 2;
pub const RESIZE: u8 = 3;
pub const STOP: u8 = 4;
pub const READY: u8 = 16;
pub const OUTPUT: u8 = 17;
pub const ACK: u8 = 18;
pub const EXITED: u8 = 19;
pub const FAILED: u8 = 20;

pub struct Packet {
    pub kind: u8,
    pub body: Vec<u8>,
}

struct Pending {
    header: [u8; 13],
    header_len: usize,
    body: Vec<u8>,
    written: usize,
    retained: usize,
}

#[derive(Default)]
pub struct Writer {
    packets: VecDeque<Pending>,
    bytes: usize,
}
impl Writer {
    pub fn push(&mut self, kind: u8, body: Vec<u8>) -> Result<(), Error> {
        self.push_inner(kind, None, body)
    }

    pub fn push_sequenced(&mut self, kind: u8, sequence: u64, body: Vec<u8>) -> Result<(), Error> {
        self.push_inner(kind, Some(sequence), body)
    }

    fn push_inner(&mut self, kind: u8, sequence: Option<u64>, body: Vec<u8>) -> Result<(), Error> {
        let prefix = if sequence.is_some() { 8 } else { 0 };
        if body.len().saturating_add(prefix) > MAX_PACKET {
            return Err(Error::Capacity("terminal helper packet"));
        }
        let retained = body
            .capacity()
            .saturating_add(std::mem::size_of::<Pending>());
        let limit = if kind == OUTPUT {
            MAX_QUEUED_BYTES - CONTROL_RESERVE
        } else {
            MAX_QUEUED_BYTES
        };
        let packet_limit = if kind == OUTPUT {
            MAX_QUEUED_PACKETS - 8
        } else {
            MAX_QUEUED_PACKETS
        };
        if retained > limit.saturating_sub(self.bytes) || self.packets.len() >= packet_limit {
            return Err(Error::InputFull);
        }
        let mut header = [0; 13];
        header[..4].copy_from_slice(&((body.len() + prefix) as u32).to_le_bytes());
        header[4] = kind;
        if let Some(sequence) = sequence {
            header[5..13].copy_from_slice(&sequence.to_le_bytes());
        }
        self.packets.push_back(Pending {
            header,
            header_len: 5 + prefix,
            body,
            written: 0,
            retained,
        });
        self.bytes += retained;
        Ok(())
    }

    pub fn data_room(&self) -> usize {
        if self.packets.len() >= MAX_QUEUED_PACKETS - 8 {
            return 0;
        }
        (MAX_QUEUED_BYTES - CONTROL_RESERVE)
            .saturating_sub(self.bytes)
            .saturating_sub(std::mem::size_of::<Pending>())
    }

    pub fn pending(&self) -> bool {
        !self.packets.is_empty()
    }

    pub fn flush(&mut self, stream: &mut impl Write) -> Result<(), Error> {
        while let Some(packet) = self.packets.front_mut() {
            let written = if packet.written < packet.header_len {
                stream.write_vectored(&[
                    IoSlice::new(&packet.header[packet.written..packet.header_len]),
                    IoSlice::new(&packet.body),
                ])
            } else {
                stream.write(&packet.body[packet.written - packet.header_len..])
            };
            match written {
                Ok(0) => return Err(Error::Closed),
                Ok(count) => {
                    packet.written += count;
                    if packet.written == packet.body.len() + packet.header_len {
                        self.bytes -= packet.retained;
                        self.packets.pop_front();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => {
                    return Err(Error::Io {
                        operation: "write helper channel",
                        detail: error.to_string(),
                    })
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Reader {
    header: [u8; 5],
    header_read: usize,
    body: Vec<u8>,
    body_read: usize,
    eof: bool,
}
impl Reader {
    pub fn eof(&self) -> bool {
        self.eof
    }

    pub fn next(&mut self, stream: &mut impl Read) -> Result<Option<Packet>, Error> {
        loop {
            if self.eof {
                return Ok(None);
            }
            if self.header_read < 5 {
                match stream.read(&mut self.header[self.header_read..]) {
                    Ok(0) if self.header_read == 0 => {
                        self.eof = true;
                        return Ok(None);
                    }
                    Ok(0) => {
                        self.eof = true;
                        return Err(Error::Protocol("truncated terminal packet header".into()));
                    }
                    Ok(count) => self.header_read += count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                    Err(error) => {
                        return Err(Error::Io {
                            operation: "read helper channel",
                            detail: error.to_string(),
                        })
                    }
                }
                if self.header_read < 5 {
                    continue;
                }
                let length = u32::from_le_bytes([
                    self.header[0],
                    self.header[1],
                    self.header[2],
                    self.header[3],
                ]) as usize;
                if length > MAX_PACKET {
                    return Err(Error::Capacity("terminal helper packet"));
                }
                self.body.resize(length, 0);
            }
            if self.body_read < self.body.len() {
                match stream.read(&mut self.body[self.body_read..]) {
                    Ok(0) => {
                        self.eof = true;
                        return Err(Error::Protocol("truncated terminal packet body".into()));
                    }
                    Ok(count) => self.body_read += count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                    Err(error) => {
                        return Err(Error::Io {
                            operation: "read helper body",
                            detail: error.to_string(),
                        })
                    }
                }
                if self.body_read < self.body.len() {
                    continue;
                }
            }
            let packet = Packet {
                kind: self.header[4],
                body: std::mem::take(&mut self.body),
            };
            self.header_read = 0;
            self.body_read = 0;
            return Ok(Some(packet));
        }
    }
}

pub fn sequence(body: &[u8]) -> Result<(u64, &[u8]), Error> {
    let bytes: [u8; 8] = body
        .get(..8)
        .ok_or_else(|| Error::Protocol("missing terminal sequence".into()))?
        .try_into()
        .map_err(|_| Error::Protocol("invalid terminal sequence".into()))?;
    Ok((u64::from_le_bytes(bytes), &body[8..]))
}

#[cfg(test)]
mod tests;
