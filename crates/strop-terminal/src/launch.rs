//! One captured launch intent. Display names and remote-looking paths are not
//! execution authority; the engine selects the namespace's worker before
//! constructing it, and the worker's exec admission owns the actual spawn.
use crate::Error;
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

    /// A shell launch for a non-local namespace (0058 WK12): never the
    /// editor's environment — a local capture forwarded to a remote host
    /// would leak it. The remote gets the terminal identity variables and
    /// nothing else; its own profile supplies the rest.
    pub fn shell_remote(directory: PathBuf, command: Option<OsString>) -> Self {
        let arguments = command.map_or_else(Vec::new, |command| vec!["-c".into(), command]);
        let environment = vec![
            ("TERM".into(), "xterm-256color".into()),
            ("TERM_PROGRAM".into(), "strop".into()),
            (
                "TERM_PROGRAM_VERSION".into(),
                env!("CARGO_PKG_VERSION").into(),
            ),
            ("COLORTERM".into(), "truecolor".into()),
        ];
        Self {
            program: "/bin/sh".into(),
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
