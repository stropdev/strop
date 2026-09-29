//! WSL discovery and explicit first-open selection. The GUI may enumerate
//! facts; it must not infer consent, mutate distro state or pick a shell
//! banner-corrupted startup path.
//!
//! `wsl.exe --list --quiet` is decoded as UTF-16LE. Entries are parsed into
//! typed distro names, never NUL-stripped globally and never matched by
//! localized verbose status text.

use std::process::{Command, Output};

use super::bridge::{BridgeError, WslSelection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslDistro {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WslState {
    /// WSL is usable and installed distributions were enumerated.
    Available(Vec<WslDistro>),
    /// WSL exists but has no installed user distribution.
    NoDistribution,
    /// WSL is unavailable or its command failed. The exact stderr is a
    /// diagnostic, not permission to guess.
    Unavailable(String),
}

pub fn enumerate(wsl: &std::path::Path) -> Result<WslState, BridgeError> {
    let output = Command::new(wsl).args(["--list", "--quiet"]).output()?;
    classify(output)
}

fn classify(output: Output) -> Result<WslState, BridgeError> {
    if !output.status.success() {
        return Ok(WslState::Unavailable(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let text = String::from_utf16(&decode_utf16le(&output.stdout))
        .map_err(|_| BridgeError::Transport("WSL enumeration is not UTF-16LE".into()))?;
    let mut distros: Vec<WslDistro> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|name| WslDistro {
            name: name.trim_start_matches("* ").to_owned(),
        })
        .collect();
    if distros.is_empty() {
        Ok(WslState::NoDistribution)
    } else {
        distros.sort_by(|left, right| left.name.cmp(&right.name));
        distros.dedup_by(|left, right| left.name == right.name);
        Ok(WslState::Available(distros))
    }
}

fn decode_utf16le(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect()
}

/// Construct a first-open launch only from explicit user-selected distro,
/// user, workspace and verified backend facts. The environment is exact
/// key/value data; no login shell interpolation participates.
pub fn select(
    distro: &WslDistro,
    user: &str,
    workspace: &str,
    backend: &std::path::Path,
) -> Result<WslSelection, BridgeError> {
    if distro.name.is_empty() || user.is_empty() || !workspace.starts_with('/') {
        return Err(BridgeError::Transport(
            "first-open selection lacks an explicit distro, user or Linux workspace".into(),
        ));
    }
    let backend = backend.to_owned();
    if !backend.is_absolute() {
        return Err(BridgeError::Transport(
            "first-open backend must be an absolute Linux path".into(),
        ));
    }
    Ok(WslSelection {
        wsl: std::path::PathBuf::from("wsl.exe"),
        distribution: distro.name.clone(),
        user: user.to_owned(),
        backend,
        workspace: workspace.to_owned(),
        environment: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn quiet_utf16_distros_are_typed_without_nul_stripping() {
        let output = Output {
            status: Default::default(),
            stdout: utf16("Ubuntu\r\nDebian\r\n"),
            stderr: Vec::new(),
        };
        assert_eq!(
            classify(output).unwrap(),
            WslState::Available(vec![
                WslDistro {
                    name: "Debian".into()
                },
                WslDistro {
                    name: "Ubuntu".into()
                },
            ])
        );
    }

    #[test]
    fn empty_enumeration_is_explicit_not_a_guessed_fallback() {
        let output = Output {
            status: Default::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        assert_eq!(classify(output).unwrap(), WslState::NoDistribution);
    }

    #[test]
    fn first_open_requires_absolute_linux_backend_and_workspace() {
        let distro = WslDistro {
            name: "Ubuntu".into(),
        };
        assert!(select(
            &distro,
            "tarek",
            "/home/tarek",
            std::path::Path::new("/opt/strop/strop")
        )
        .is_ok());
        assert!(select(
            &distro,
            "tarek",
            "C:\\Users",
            std::path::Path::new("/opt/strop/strop")
        )
        .is_err());
        assert!(select(
            &distro,
            "tarek",
            "/home/tarek",
            std::path::Path::new("strop")
        )
        .is_err());
    }
}
