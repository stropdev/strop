//! One captured local launch. Display names and remote-looking paths are not
//! execution authority; the engine selects the namespace before constructing it.
use crate::Error;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;

const MAX_LAUNCH_BYTES: usize = 128 * 1024;
const MAX_LAUNCH_ITEMS: usize = 1024;

#[derive(Debug, Clone)]
pub struct Launch {
    pub program: OsString,
    pub arguments: Vec<OsString>,
    pub directory: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
}

impl Launch {
    pub fn shell(directory: PathBuf, command: Option<OsString>) -> Self {
        let program = std::env::var_os("SHELL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        let arguments = command.map_or_else(Vec::new, |command| vec!["-c".into(), command]);
        let mut environment: Vec<_> = std::env::vars_os()
            .filter(|(name, _)| {
                name != "TERM"
                    && name != "TERM_PROGRAM"
                    && name != "TERM_PROGRAM_VERSION"
                    && name != "COLORTERM"
            })
            .collect();
        environment.extend([
            ("TERM".into(), "xterm-256color".into()),
            ("TERM_PROGRAM".into(), "strop".into()),
            (
                "TERM_PROGRAM_VERSION".into(),
                env!("CARGO_PKG_VERSION").into(),
            ),
            ("COLORTERM".into(), "truecolor".into()),
        ]);
        Self {
            program,
            arguments,
            directory,
            environment,
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.program.is_empty()
            || !self.directory.is_absolute()
            || self.arguments.len() > MAX_LAUNCH_ITEMS
            || self.environment.len() > MAX_LAUNCH_ITEMS
        {
            return Err(Error::Protocol("invalid captured terminal launch".into()));
        }
        let mut bytes = 0usize;
        for value in std::iter::once(self.program.as_os_str())
            .chain(std::iter::once(self.directory.as_os_str()))
            .chain(self.arguments.iter().map(OsString::as_os_str))
            .chain(
                self.environment
                    .iter()
                    .flat_map(|(key, value)| [key.as_os_str(), value.as_os_str()]),
            )
        {
            if value.as_encoded_bytes().contains(&0) {
                return Err(Error::Protocol("NUL in terminal launch context".into()));
            }
            bytes = bytes
                .checked_add(value.as_encoded_bytes().len())
                .ok_or(Error::Capacity("terminal launch size"))?;
            if bytes > MAX_LAUNCH_BYTES {
                return Err(Error::Capacity("terminal launch context"));
            }
        }
        if self
            .environment
            .iter()
            .any(|(key, _)| key.is_empty() || key.as_encoded_bytes().contains(&b'='))
        {
            return Err(Error::Protocol("invalid terminal environment key".into()));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct WireLaunch {
    version: u32,
    program: Vec<u8>,
    arguments: Vec<Vec<u8>>,
    directory: Vec<u8>,
    environment: Vec<(Vec<u8>, Vec<u8>)>,
    pub geometry: crate::model::Geometry,
}

#[cfg(unix)]
impl WireLaunch {
    pub fn encode(launch: &Launch, geometry: crate::model::Geometry) -> Result<Vec<u8>, Error> {
        use std::os::unix::ffi::OsStrExt;
        launch.validate()?;
        if !geometry.valid() {
            return Err(Error::Capacity("terminal geometry"));
        }
        let wire = Self {
            version: crate::protocol::VERSION,
            program: launch.program.as_bytes().to_vec(),
            arguments: launch
                .arguments
                .iter()
                .map(|value| value.as_bytes().to_vec())
                .collect(),
            directory: launch.directory.as_os_str().as_bytes().to_vec(),
            environment: launch
                .environment
                .iter()
                .map(|(key, value)| (key.as_bytes().to_vec(), value.as_bytes().to_vec()))
                .collect(),
            geometry,
        };
        let bytes =
            serde_json::to_vec(&wire).map_err(|error| Error::Protocol(error.to_string()))?;
        if bytes.len() > crate::protocol::MAX_PACKET {
            return Err(Error::Capacity("encoded terminal launch"));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<(Launch, crate::model::Geometry), Error> {
        use std::os::unix::ffi::OsStringExt;
        if bytes.len() > crate::protocol::MAX_PACKET {
            return Err(Error::Capacity("encoded terminal launch"));
        }
        let wire: Self =
            serde_json::from_slice(bytes).map_err(|error| Error::Protocol(error.to_string()))?;
        if wire.version != crate::protocol::VERSION {
            return Err(Error::Protocol(
                "terminal helper protocol version mismatch".into(),
            ));
        }
        if !wire.geometry.valid() {
            return Err(Error::Capacity("terminal geometry"));
        }
        let launch = Launch {
            program: OsString::from_vec(wire.program),
            arguments: wire.arguments.into_iter().map(OsString::from_vec).collect(),
            directory: PathBuf::from(OsString::from_vec(wire.directory)),
            environment: wire
                .environment
                .into_iter()
                .map(|(key, value)| (OsString::from_vec(key), OsString::from_vec(value)))
                .collect(),
        };
        launch.validate()?;
        Ok((launch, wire.geometry))
    }
}
