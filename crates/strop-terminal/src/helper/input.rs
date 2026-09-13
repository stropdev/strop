use super::io_error;
use crate::{model::Geometry, protocol, Error};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Write},
};

enum Action {
    Write {
        sequence: u64,
        bytes: Vec<u8>,
        offset: usize,
    },
    Resize {
        sequence: u64,
        geometry: Geometry,
    },
}
#[derive(Default)]
pub struct InputQueue {
    actions: VecDeque<Action>,
    retained: usize,
    sequence: u64,
    geometry_revision: u64,
}
impl InputQueue {
    pub fn new(geometry: Geometry) -> Self {
        Self {
            geometry_revision: geometry.revision,
            ..Self::default()
        }
    }
    pub fn clear(&mut self) {
        self.actions.clear();
        self.retained = 0;
    }
    pub fn pending(&self) -> bool {
        !self.actions.is_empty()
    }
    pub fn admit(&mut self, packet: protocol::Packet) -> Result<(), Error> {
        let (sequence, body) = protocol::sequence(&packet.body)?;
        if sequence <= self.sequence {
            return Err(Error::Protocol(
                "non-increasing terminal input sequence".into(),
            ));
        }
        let action = match packet.kind {
            protocol::INPUT => {
                if body.len() > crate::model::MAX_INPUT_BYTES {
                    return Err(Error::Capacity("terminal input"));
                }
                Action::Write {
                    sequence,
                    bytes: packet.body,
                    offset: 8,
                }
            }
            protocol::RESIZE => {
                let geometry: Geometry = serde_json::from_slice(body)
                    .map_err(|error| Error::Protocol(error.to_string()))?;
                if !geometry.valid() || geometry.revision <= self.geometry_revision {
                    return Err(Error::Protocol("stale or invalid terminal geometry".into()));
                }
                Action::Resize { sequence, geometry }
            }
            _ => return Err(Error::Protocol("unexpected terminal helper command".into())),
        };
        let retained = cost(&action);
        if retained > protocol::MAX_QUEUED_BYTES.saturating_sub(self.retained)
            || self.actions.len() >= protocol::MAX_QUEUED_PACKETS
        {
            return Err(Error::InputFull);
        }
        if let Action::Resize { geometry, .. } = &action {
            self.geometry_revision = geometry.revision;
        }
        self.sequence = sequence;
        self.retained += retained;
        self.actions.push_back(action);
        Ok(())
    }
    pub fn flush(&mut self, master: &mut File, writer: &mut protocol::Writer) -> Result<(), Error> {
        for _ in 0..32 {
            let Some(action) = self.actions.front_mut() else {
                return Ok(());
            };
            // An acknowledgment must fit before admitting native effects.
            if writer.data_room() < 128 {
                return Ok(());
            }
            let sequence = match action {
                Action::Write {
                    sequence,
                    bytes,
                    offset,
                } => {
                    if *offset < bytes.len() {
                        match master.write(&bytes[*offset..]) {
                            Ok(0) => return Err(Error::Closed),
                            Ok(count) => *offset += count,
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                return Ok(())
                            }
                            Err(error) => return Err(io_error("write terminal input", error)),
                        }
                        if *offset != bytes.len() {
                            continue;
                        }
                    }
                    *sequence
                }
                Action::Resize { sequence, geometry } => {
                    super::resize(master, *geometry)?;
                    *sequence
                }
            };
            writer.push_sequenced(protocol::ACK, sequence, Vec::new())?;
            if let Some(action) = self.actions.pop_front() {
                self.retained -= cost(&action);
            }
        }
        Ok(())
    }
}
fn cost(action: &Action) -> usize {
    std::mem::size_of::<Action>()
        + match action {
            Action::Write { bytes, .. } => bytes.capacity(),
            Action::Resize { .. } => 0,
        }
}
