//! The wire description of one supervised remote process: a small
//! canonical binary blob, base64-wrapped so it can ride a single argv
//! element through the remote login shell unchanged.
//!
//! Encoding is transport, not secrecy: the blob names cwd and argv in
//! the clear (base64) and will appear in remote process listings. It
//! exists so that native POSIX byte strings — spaces, newlines, quotes,
//! non-UTF-8 filenames — cross the shell boundary as inert data, never
//! as executable text. Decoding and execution happen in the fixed
//! Python supervisor via `os.chdir(bytes)` / `os.execvpe` with byte
//! argv; nothing is ever interpolated into shell syntax.
//!
//! Layout (little-endian): version `u8`, mode `u8` (0 finite, 1
//! relayed), grace milliseconds `u32`, 16 raw nonce bytes, then length-
//! prefixed (`u32`) cwd and `argc`-counted length-prefixed argv
//! elements. Every string is native POSIX bytes with no NUL.

use crate::exec::{RemoteCommandError, StdinMode};
use std::path::Path;

pub(super) const VERSION: u8 = 1;

/// Linux caps a single argv element at 128 KiB (`MAX_ARG_STRLEN`); the
/// base64 wrapper plus quoting headroom keeps us comfortably below.
pub(super) const MAX_ENCODED: usize = 96 * 1024;

/// TERM→KILL grace the remote supervisor applies on cancellation, in
/// milliseconds. Locally owned so policy changes never need a remote
/// update.
pub(super) const GRACE_MS: u32 = 2_000;

#[derive(Debug)]
pub(super) struct Spec {
    pub(super) mode: StdinMode,
    pub(super) grace_ms: u32,
    pub(super) nonce: [u8; 16],
    pub(super) cwd: Vec<u8>,
    pub(super) argv: Vec<Vec<u8>>,
}

/// Native POSIX bytes for a remote path/argument. Unix keeps arbitrary
/// non-NUL bytes; other platforms require UTF-8 rather than a lossy
/// stand-in that could name the wrong remote file.
pub(super) fn os_bytes(value: &std::ffi::OsStr) -> Result<Vec<u8>, RemoteCommandError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(value.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        value
            .to_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| RemoteCommandError::Invalid {
                detail: "argument is not representable as remote POSIX bytes on this platform"
                    .into(),
            })
    }
}

impl Spec {
    /// Validate and encode one command description. Rejects NUL bytes,
    /// empty programs, non-absolute cwd and argv that cannot fit a
    /// remote command line.
    pub(super) fn encode(
        mode: StdinMode,
        nonce: [u8; 16],
        program: &std::ffi::OsStr,
        args: &[std::ffi::OsString],
        cwd: &Path,
    ) -> Result<Self, RemoteCommandError> {
        let mut argv = Vec::with_capacity(args.len() + 1);
        let program = os_bytes(program)?;
        if program.is_empty() {
            return Err(RemoteCommandError::Invalid {
                detail: "program is empty".into(),
            });
        }
        argv.push(program);
        for argument in args {
            argv.push(os_bytes(argument)?);
        }
        if argv.iter().any(|argument| argument.contains(&0)) {
            return Err(RemoteCommandError::Invalid {
                detail: "program and arguments cannot contain a NUL byte".into(),
            });
        }
        let cwd = os_bytes(cwd.as_os_str())?;
        if cwd.contains(&0) {
            return Err(RemoteCommandError::Invalid {
                detail: "working directory cannot contain a NUL byte".into(),
            });
        }
        if !cwd.starts_with(b"/") {
            return Err(RemoteCommandError::Invalid {
                detail: format!(
                    "remote working directory must be absolute, not {:?}",
                    String::from_utf8_lossy(&cwd)
                ),
            });
        }
        Ok(Self {
            mode,
            grace_ms: GRACE_MS,
            nonce,
            cwd,
            argv,
        })
    }

    /// Canonical binary form.
    pub(super) fn bytes(&self) -> Vec<u8> {
        let mut blob = Vec::with_capacity(6 + 16 + 4 + self.cwd.len() + 4 * self.argv.len());
        blob.push(VERSION);
        blob.push(match self.mode {
            StdinMode::Finite => 0,
            StdinMode::Relayed => 1,
        });
        blob.extend_from_slice(&self.grace_ms.to_le_bytes());
        blob.extend_from_slice(&self.nonce);
        blob.extend_from_slice(&(self.cwd.len() as u32).to_le_bytes());
        blob.extend_from_slice(&self.cwd);
        blob.extend_from_slice(&(self.argv.len() as u32).to_le_bytes());
        for argument in &self.argv {
            blob.extend_from_slice(&(argument.len() as u32).to_le_bytes());
            blob.extend_from_slice(argument);
        }
        blob
    }

    /// Base64 of the canonical form, or a typed refusal when the
    /// command cannot ride one remote command line.
    pub(super) fn encoded(&self) -> Result<String, RemoteCommandError> {
        let text = base64(&self.bytes());
        if text.len() > MAX_ENCODED {
            return Err(RemoteCommandError::ArgvTooLarge { bytes: text.len() });
        }
        Ok(text)
    }
}

/// `base64.b64decode(..., validate=True)`, so no non-canonical spelling
/// is ever accepted.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub(super) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |index: usize| -> u8 { chunk.get(index).copied().unwrap_or(0) };
        let triple = ((byte(0) as u32) << 16) | ((byte(1) as u32) << 8) | byte(2) as u32;
        let characters = [
            ALPHABET[(triple >> 18) as usize & 0x3f],
            ALPHABET[(triple >> 12) as usize & 0x3f],
            if chunk.len() > 1 {
                ALPHABET[(triple >> 6) as usize & 0x3f]
            } else {
                b'='
            },
            if chunk.len() > 2 {
                ALPHABET[triple as usize & 0x3f]
            } else {
                b'='
            },
        ];
        out.push(characters[0] as char);
        out.push(characters[1] as char);
        out.push(characters[2] as char);
        out.push(characters[3] as char);
    }
    out
}

/// 128 bits from the process-seeded SipHash keys. The nonce separates
/// the supervisor's status records from coincidental worker output; it
/// is not a secret (it travels inside the spec, visible in remote
/// process listings) and does not protect against a worker that
/// deliberately echoes it.
pub(super) fn nonce() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut out = [0u8; 16];
    let mut left = std::collections::hash_map::RandomState::new().build_hasher();
    left.write_u64(0x5354_524f_505f_4c46); // "STROP_LF"
    out[..8].copy_from_slice(&left.finish().to_le_bytes());
    let mut right = std::collections::hash_map::RandomState::new().build_hasher();
    right.write_u64(0x5355_5045_5256_4953); // "SUPERVIS"
    out[8..].copy_from_slice(&right.finish().to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn spec(argv: &[&str], cwd: &str) -> Spec {
        Spec::encode(
            StdinMode::Finite,
            [7u8; 16],
            std::ffi::OsStr::new(argv[0]),
            &argv[1..]
                .iter()
                .map(|a| OsString::from(a.to_owned()))
                .collect::<Vec<_>>(),
            &PathBuf::from(cwd),
        )
        .unwrap()
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn canonical_bytes_have_a_stable_layout() {
        let bytes = spec(&["git", "log"], "/srv").bytes();
        assert_eq!(bytes[0], 1, "version");
        assert_eq!(bytes[1], 0, "finite mode");
        assert_eq!(
            u32::from_le_bytes(bytes[2..6].try_into().unwrap()),
            GRACE_MS
        );
        assert_eq!(&bytes[6..22], &[7u8; 16], "nonce");
        let mut at = 22;
        let cwd_len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        assert_eq!(&bytes[at..at + cwd_len], b"/srv");
        at += cwd_len;
        let argc = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(argc, 2);
    }

    #[test]
    fn relayed_mode_is_encoded_as_one() {
        let relayed = Spec::encode(
            StdinMode::Relayed,
            nonce(),
            std::ffi::OsStr::new("server"),
            &[],
            &PathBuf::from("/"),
        )
        .unwrap();
        assert_eq!(relayed.bytes()[1], 1);
    }

    #[test]
    #[cfg(unix)]
    fn nul_bytes_are_refused() {
        use std::os::unix::ffi::OsStringExt;
        let poisoned = OsString::from_vec(vec![b'a', 0, b'b']);
        let error = Spec::encode(
            StdinMode::Finite,
            nonce(),
            std::ffi::OsStr::new("git"),
            &[poisoned],
            &PathBuf::from("/"),
        )
        .unwrap_err();
        assert!(matches!(error, RemoteCommandError::Invalid { .. }));
    }

    #[test]
    fn non_absolute_cwd_is_refused() {
        let error = Spec::encode(
            StdinMode::Finite,
            nonce(),
            std::ffi::OsStr::new("git"),
            &[],
            &PathBuf::from("relative/path"),
        )
        .unwrap_err();
        assert!(matches!(error, RemoteCommandError::Invalid { .. }));
    }

    #[test]
    fn oversize_argv_is_a_typed_refusal() {
        let big = "x".repeat(200 * 1024);
        let error = spec(&["git", &big], "/").encoded().unwrap_err();
        assert!(matches!(
            error,
            RemoteCommandError::ArgvTooLarge { bytes: _ }
        ));
    }

    #[test]
    fn nonces_between_calls_differ() {
        assert_ne!(nonce(), nonce());
    }
}
