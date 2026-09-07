//! Validated remote-file locations (0034 "Implementation boundaries" 2):
//! `ssh://[user@]host[:port]/absolute/path` as a pure URI domain.
//!
//! Parsing is the only place remote identity is textual. Every field is
//! admitted here — before any OpenSSH subprocess exists — so a
//! [`RemoteFile`] can never carry an option-shaped endpoint, a password,
//! a query, a fragment or a control byte toward `ssh` argv. The bounded
//! grammar needs no URL crate: aliases and IPv4 reuse `std` parsing,
//! bracketed IPv6 reuses `std` validation. Paths decode to native
//! filename bytes — Unix keeps arbitrary non-NUL bytes, other platforms
//! require UTF-8 rather than a lossy stand-in. `Display` renders the
//! canonical URI (necessary percent escapes only) and serde round-trips
//! through that same string, so deserialized values are re-validated,
//! never trusted fields. Identity is the decoded endpoint: spellings
//! that decode to the same bytes, port or address are one value, while a
//! decoded path is never trimmed or normalized — `.`, `..` and `//`
//! belong to the remote host.

use std::num::NonZeroU16;
use std::path::{Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A validated `ssh://[user@]host[:port]/absolute/path` location.
///
/// Private fields leave [`RemoteFile::parse`] as the only constructor, so
/// every value — including deserialized ones — passed the same admission.
#[derive(Debug, Clone, Eq)]
pub struct RemoteFile {
    user: Option<String>,
    /// Alias/hostname verbatim, or the canonical unbracketed IP form.
    host: String,
    port: Option<NonZeroU16>,
    path: PathBuf,
}

impl PartialEq for RemoteFile {
    fn eq(&self, other: &Self) -> bool {
        (&self.user, &self.host, self.port, self.path.as_os_str())
            == (&other.user, &other.host, other.port, other.path.as_os_str())
    }
}
impl std::hash::Hash for RemoteFile {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (&self.user, &self.host, self.port, self.path.as_os_str()).hash(state);
    }
}

/// Why a textual remote location was refused. Every variant is a pure
/// admission failure; none implies a subprocess was ever started.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("remote locations use the form `ssh://[user@]host[:port]/absolute/path`")]
    NotSshUri,
    #[error(
        "SSH locations cannot carry a password (`user:pass@`); authenticate with keys or an agent instead"
    )]
    PasswordInUri,
    #[error("empty username before `@`")]
    EmptyUser,
    #[error("host is empty")]
    EmptyHost,
    #[error("port must be 1-65535 written in digits, e.g. `:2222`")]
    InvalidPort,
    #[error(
        "refusing an option-shaped username/host (leading `-` or any `=`): SSH would read it as an option"
    )]
    OptionShapedEndpoint,
    #[error("IPv6 hosts need brackets, e.g. `ssh://[2001:db8::1]:22/var/log/app.log`")]
    UnbracketedIpv6,
    #[error("bracketed IPv6 host is missing its closing `]`")]
    UnclosedIpv6,
    #[error("only `:port` may follow a bracketed IPv6 host")]
    JunkAfterIpv6,
    #[error("{literal:?} is not a valid IPv6 address")]
    InvalidIpv6 { literal: String },
    #[error("{literal:?} looks like an IPv4 address but is not a valid one")]
    InvalidIpv4 { literal: String },
    #[error(
        "remote path is missing: the location must end in an absolute path like `/var/log/app.log`"
    )]
    AbsentPath,
    #[error(
        "SSH locations carry no query string or fragment; percent-encode those bytes in the path instead (`?` -> `%3F`, `#` -> `%23`)"
    )]
    QueryOrFragment,
    #[error("percent escapes need two hexadecimal digits, e.g. `%20`")]
    MalformedPercentEscape,
    #[error("path cannot contain a NUL byte")]
    NulInPath,
    #[error("path contains a byte that must be percent-encoded (a space is `%20`)")]
    UnencodedPathByte,
    #[cfg(not(unix))]
    #[error("the decoded path bytes are not representable as a native filename on this platform")]
    UnrepresentablePath,
    #[error("username/host may only contain ASCII letters, digits and `-._+`")]
    InvalidAuthorityCharacter,
}

impl RemoteFile {
    /// Admit one textual location. Pure: no filesystem, no subprocess.
    pub fn parse(value: &str) -> Result<Self, AddressError> {
        let rest = value
            .strip_prefix("ssh://")
            .ok_or(AddressError::NotSshUri)?;
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (user, host_port) = split_user(&rest[..authority_end])?;
        let (host, port) = split_host(host_port)?;
        let tail = &rest[authority_end..];
        let path_text = match tail.as_bytes().first() {
            None => return Err(AddressError::AbsentPath),
            Some(b'/') => tail,
            Some(_) => return Err(AddressError::QueryOrFragment),
        };
        if path_text.contains(['?', '#']) {
            return Err(AddressError::QueryOrFragment);
        }
        let path = decode_path(path_text)?;
        Ok(Self {
            user,
            host,
            port,
            path,
        })
    }

    /// Hostname, alias, or the canonical unbracketed IPv4/IPv6 form.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    pub fn port(&self) -> Option<NonZeroU16> {
        self.port
    }

    /// The decoded remote path. A remote POSIX path, not a local one.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The canonical URI: this is the identity replayed, traced and shown.
    fn canonical(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::with_capacity(
            "ssh://".len()
                + self.host.len()
                + self.user.as_deref().map_or(0, str::len)
                + 6
                + path_bytes(&self.path).len().saturating_mul(3),
        );
        out.push_str("ssh://");
        if let Some(user) = &self.user {
            out.push_str(user);
            out.push('@');
        }
        if self.host.contains(':') {
            out.push('[');
            out.push_str(&self.host);
            out.push(']');
        } else {
            out.push_str(&self.host);
        }
        if let Some(port) = self.port {
            let _ = write!(out, ":{port}");
        }
        for &byte in path_bytes(&self.path) {
            if is_raw_path_byte(byte) {
                out.push(byte as char);
            } else {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0F) as usize] as char);
            }
        }
        out
    }
}

impl std::fmt::Display for RemoteFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

/// Serde carries the canonical validated URI — one string, re-parsed on
/// the way in, so stored values can never bypass admission.
impl Serialize for RemoteFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> Deserialize<'de> for RemoteFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        RemoteFile::parse(&text).map_err(D::Error::custom)
    }
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn split_user(authority: &str) -> Result<(Option<String>, &str), AddressError> {
    let Some(at) = authority.find('@') else {
        return Ok((None, authority));
    };
    let (user, host_port) = (&authority[..at], &authority[at + 1..]);
    if host_port.contains('@') {
        return Err(AddressError::InvalidAuthorityCharacter);
    }
    if user.is_empty() {
        return Err(AddressError::EmptyUser);
    }
    if user.contains(':') {
        return Err(AddressError::PasswordInUri);
    }
    check_endpoint_token(user)?;
    Ok((Some(user.to_owned()), host_port))
}

fn split_host(host_port: &str) -> Result<(String, Option<NonZeroU16>), AddressError> {
    if let Some(bracketed) = host_port.strip_prefix('[') {
        let close = bracketed.find(']').ok_or(AddressError::UnclosedIpv6)?;
        let inner = &bracketed[..close];
        let after = &bracketed[close + 1..];
        let port = if after.is_empty() {
            None
        } else if let Some(digits) = after.strip_prefix(':') {
            Some(parse_port(digits)?)
        } else {
            return Err(AddressError::JunkAfterIpv6);
        };
        let address: std::net::Ipv6Addr = inner.parse().map_err(|_| AddressError::InvalidIpv6 {
            literal: inner.to_owned(),
        })?;
        return Ok((address.to_string(), port));
    }
    if host_port.matches(':').count() > 1 {
        return Err(AddressError::UnbracketedIpv6);
    }
    let (host, port) = match host_port.split_once(':') {
        Some((host, digits)) => (host, Some(parse_port(digits)?)),
        None => (host_port, None),
    };
    if host.is_empty() {
        return Err(AddressError::EmptyHost);
    }
    // Digits-and-dots is IPv4-shaped: admit it only as a real address,
    // so `01.2.3.4` or `999.1.1.1` cannot reach getaddrinfo as an alias.
    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        let address: std::net::Ipv4Addr = host.parse().map_err(|_| AddressError::InvalidIpv4 {
            literal: host.to_owned(),
        })?;
        return Ok((address.to_string(), port));
    }
    check_endpoint_token(host)?;
    Ok((host.to_owned(), port))
}

fn check_endpoint_token(token: &str) -> Result<(), AddressError> {
    if token.starts_with('-') || token.contains('=') {
        return Err(AddressError::OptionShapedEndpoint);
    }
    if !token.bytes().all(is_endpoint_byte) {
        return Err(AddressError::InvalidAuthorityCharacter);
    }
    Ok(())
}

fn is_endpoint_byte(byte: u8) -> bool {
    // OpenSSH config can interpolate %h/%r in ProxyCommand/Match exec. Keep
    // shell metacharacters out even when an older ssh accepts them as argv.
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'+')
}

fn parse_port(digits: &str) -> Result<NonZeroU16, AddressError> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AddressError::InvalidPort);
    }
    match digits.parse::<u16>() {
        Ok(value) => NonZeroU16::new(value).ok_or(AddressError::InvalidPort),
        Err(_) => Err(AddressError::InvalidPort),
    }
}

/// Decode the path region to native filename bytes. Raw bytes outside the
/// URI path grammar are refused (percent-encode them); raw non-ASCII
/// UTF-8 passes through as data. Remote metacharacters are never special.
fn decode_path(text: &str) -> Result<PathBuf, AddressError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'%' {
            let high = bytes.get(i + 1).copied().filter(|b| b.is_ascii_hexdigit());
            let low = bytes.get(i + 2).copied().filter(|b| b.is_ascii_hexdigit());
            match (high, low) {
                (Some(high), Some(low)) => {
                    out.push((hex_value(high) << 4) | hex_value(low));
                    i += 3;
                }
                _ => return Err(AddressError::MalformedPercentEscape),
            }
        } else if byte == 0 {
            return Err(AddressError::NulInPath);
        } else if is_raw_path_byte(byte) || byte >= 0x80 {
            out.push(byte);
            i += 1;
        } else {
            return Err(AddressError::UnencodedPathByte);
        }
    }
    if out.contains(&0) {
        return Err(AddressError::NulInPath);
    }
    bytes_to_path(out)
}

/// Caller checked `is_ascii_hexdigit`.
fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => unreachable!("hex_value called on a non-hex byte"),
    }
}

/// RFC 3986 pchar plus `/`: bytes a canonical URI carries raw.
fn is_raw_path_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':'
            | b'@'
            | b'/'
    )
}

#[cfg(unix)]
fn bytes_to_path(bytes: Vec<u8>) -> Result<PathBuf, AddressError> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

/// Platforms without byte filenames get strict UTF-8, never a lossy
/// replacement character that could name the wrong remote file.
#[cfg(not(unix))]
fn bytes_to_path(bytes: Vec<u8>) -> Result<PathBuf, AddressError> {
    let text = String::from_utf8(bytes).map_err(|_| AddressError::UnrepresentablePath)?;
    Ok(PathBuf::from(text))
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    // Built only from strict UTF-8 here, so this is the exact path text.
    path.as_os_str().as_encoded_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(uri: &str) -> RemoteFile {
        RemoteFile::parse(uri).unwrap_or_else(|error| panic!("expected parse: {uri}: {error}"))
    }

    fn refused(uri: &str) -> AddressError {
        RemoteFile::parse(uri).unwrap_err()
    }

    #[test]
    fn alias_with_user_and_port() {
        let file = ok("ssh://dev@bbgithub:2222/var/log/app.log");
        assert_eq!(file.user(), Some("dev"));
        assert_eq!(file.host(), "bbgithub");
        assert_eq!(file.port(), NonZeroU16::new(2222));
        assert_eq!(file.path(), Path::new("/var/log/app.log"));
        assert_eq!(file.to_string(), "ssh://dev@bbgithub:2222/var/log/app.log");
    }

    #[test]
    fn aliases_keep_their_spelling() {
        // Preserve the chosen alias spelling; this identity layer does not
        // infer equivalence between OpenSSH configuration targets.
        assert_ne!(ok("ssh://Box/x"), ok("ssh://box/x"));
        assert_eq!(ok("ssh://dev_01.a/x").host(), "dev_01.a");
    }

    #[test]
    fn ipv6_brackets_canonicalize_and_roundtrip() {
        let file = ok("ssh://root@[2001:0db8:0000::0001]:22/var/log/a.log");
        assert_eq!(file.user(), Some("root"));
        assert_eq!(file.host(), "2001:db8::1");
        assert_eq!(file.port(), NonZeroU16::new(22));
        assert_eq!(
            file.to_string(),
            "ssh://root@[2001:db8::1]:22/var/log/a.log"
        );
        assert_eq!(
            file,
            ok("ssh://root@[2001:db8:0:0:0:0:0:1]:0022/var/log/a.log")
        );
        let bare = ok("ssh://[::1]/x");
        assert_eq!(bare.host(), "::1");
        assert_eq!(bare.port(), None);
        assert_eq!(bare.to_string(), "ssh://[::1]/x");
    }

    #[test]
    fn ipv6_hostility() {
        assert!(matches!(
            refused("ssh://fe80::1/x"),
            AddressError::UnbracketedIpv6
        ));
        assert!(matches!(
            refused("ssh://user@::1/x"),
            AddressError::UnbracketedIpv6
        ));
        assert!(matches!(
            refused("ssh://[::1/x"),
            AddressError::UnclosedIpv6
        ));
        assert!(matches!(
            refused("ssh://[::1]extra/x"),
            AddressError::JunkAfterIpv6
        ));
        assert!(matches!(
            refused("ssh://[zz]/x"),
            AddressError::InvalidIpv6 { .. }
        ));
    }

    #[test]
    fn ipv4_shapes_must_be_addresses() {
        assert_eq!(ok("ssh://127.0.0.1/x").host(), "127.0.0.1");
        for uri in [
            "ssh://999.1.1.1/x",
            "ssh://1.2.3.4.5/x",
            "ssh://1.2.3/x",
            "ssh://0/x",
        ] {
            assert!(
                matches!(refused(uri), AddressError::InvalidIpv4 { .. }),
                "{uri}"
            );
        }
    }

    #[test]
    fn port_boundaries() {
        assert_eq!(ok("ssh://h:022/x").port(), NonZeroU16::new(22));
        for uri in [
            "ssh://h:0/x",
            "ssh://h:/x",
            "ssh://h:65536/x",
            "ssh://h:notaport/x",
            "ssh://h:+2/x",
            "ssh://h: 2/x",
        ] {
            assert!(matches!(refused(uri), AddressError::InvalidPort), "{uri}");
        }
    }

    #[test]
    fn hostile_authorities() {
        assert!(matches!(
            refused("ssh://user:secret@host/x"),
            AddressError::PasswordInUri
        ));
        assert!(matches!(
            refused("ssh://-oProxyCommand=evil@host/x"),
            AddressError::OptionShapedEndpoint
        ));
        assert!(matches!(
            refused("ssh://name=-x@host/x"),
            AddressError::OptionShapedEndpoint
        ));
        assert!(matches!(
            refused("ssh://-flag/x"),
            AddressError::OptionShapedEndpoint
        ));
        assert!(matches!(refused("ssh://@host/x"), AddressError::EmptyUser));
        assert!(matches!(refused("ssh://h@/x"), AddressError::EmptyHost));
        assert!(matches!(refused("ssh:///x"), AddressError::EmptyHost));
        assert!(matches!(
            refused("ssh://a@b@c/x"),
            AddressError::InvalidAuthorityCharacter
        ));
        assert!(matches!(
            refused("ssh://ho%st/x"),
            AddressError::InvalidAuthorityCharacter
        ));
        assert!(matches!(
            refused("ssh://ho st/x"),
            AddressError::InvalidAuthorityCharacter
        ));
        for uri in ["host/x", "scp://host/x", "", "SSH://host/x"] {
            assert!(matches!(refused(uri), AddressError::NotSshUri), "{uri}");
        }
    }

    #[test]
    fn absent_path_is_refused() {
        for uri in ["ssh://host", "ssh://host:22"] {
            assert!(matches!(refused(uri), AddressError::AbsentPath), "{uri}");
        }
    }

    #[test]
    fn query_and_fragment_are_refused() {
        for uri in [
            "ssh://h/a?b",
            "ssh://h/a#b",
            "ssh://h?q",
            "ssh://h#f",
            "ssh://h/a%3Fb?c",
        ] {
            assert!(
                matches!(refused(uri), AddressError::QueryOrFragment),
                "{uri}"
            );
        }
    }

    #[test]
    fn percent_decoding_and_canonical_display() {
        let file = ok("ssh://h/var%20log/a%23b%25c%3Fd");
        assert_eq!(file.path(), Path::new("/var log/a#b%c?d"));
        assert_eq!(file.to_string(), "ssh://h/var%20log/a%23b%25c%3Fd");
        assert_eq!(file, ok(&file.to_string()));
    }

    #[test]
    fn malformed_percent_and_nul() {
        for uri in [
            "ssh://h/%",
            "ssh://h/%2",
            "ssh://h/%G1",
            "ssh://h/a%zz",
            "ssh://h/a%2G",
        ] {
            assert!(
                matches!(refused(uri), AddressError::MalformedPercentEscape),
                "{uri}"
            );
        }
        assert!(matches!(refused("ssh://h/%00"), AddressError::NulInPath));
        assert!(matches!(
            refused("ssh://h/a\u{0}b"),
            AddressError::NulInPath
        ));
    }

    #[test]
    fn shell_metacharacters_are_path_data() {
        let uri = "ssh://h/log/$x&(rm);'q'*+,.~!@:x";
        let file = ok(uri);
        assert_eq!(file.path(), Path::new("/log/$x&(rm);'q'*+,.~!@:x"));
        assert_eq!(file.to_string(), uri);
    }

    #[test]
    fn unencodable_raw_bytes_are_refused() {
        for uri in [
            "ssh://h/a b",
            "ssh://h/a`b",
            "ssh://h/a\"b",
            "ssh://h/a\x01b",
            "ssh://h/a\x7fb",
            "ssh://h/a\\b",
        ] {
            assert!(
                matches!(refused(uri), AddressError::UnencodedPathByte),
                "{uri}"
            );
        }
    }

    #[test]
    fn unicode_path_data_roundtrips_through_escapes() {
        let file = ok("ssh://h/日本語-légère.log");
        assert_eq!(file.path(), Path::new("/日本語-légère.log"));
        let displayed = file.to_string();
        assert_eq!(
            displayed,
            "ssh://h/%E6%97%A5%E6%9C%AC%E8%AA%9E-l%C3%A9g%C3%A8re.log"
        );
        assert_eq!(ok(&displayed), file);
    }

    #[test]
    fn paths_are_never_trimmed_or_normalized() {
        let file = ok("ssh://h/a/../b//c/./d/");
        assert_eq!(file.path(), Path::new("/a/../b//c/./d/"));
        assert_eq!(file.to_string(), "ssh://h/a/../b//c/./d/");
        assert_eq!(ok("ssh://h/").path(), Path::new("/"));
    }

    #[test]
    fn identity_follows_the_decoded_endpoint() {
        assert_eq!(ok("ssh://h/%61%62"), ok("ssh://h/ab"));
        assert_eq!(ok("ssh://h/a%2Fb"), ok("ssh://h/a/b"));
        assert_eq!(ok("ssh://h:022/x"), ok("ssh://h:22/x"));
        assert_eq!(ok("ssh://[::1]/x"), ok("ssh://[0:0:0:0:0:0:0:1]/x"));
        assert_ne!(ok("ssh://h/log"), ok("ssh://h/log/"));
        assert_ne!(ok("ssh://h/a/b"), ok("ssh://h/a//b"));
        let mut seen = std::collections::HashSet::new();
        seen.insert(ok("ssh://h:022/x"));
        seen.insert(ok("ssh://h:22/x"));
        seen.insert(ok("ssh://[::1]/x"));
        seen.insert(ok("ssh://[0::1]/x"));
        assert_eq!(seen.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn native_filename_bytes_roundtrip() {
        use std::os::unix::ffi::OsStrExt;
        let file = ok("ssh://h/l%FCg%FF");
        assert_eq!(file.path().as_os_str().as_bytes(), b"/l\xFCg\xFF");
        assert_eq!(file.to_string(), "ssh://h/l%FCg%FF");
        assert_eq!(ok(&file.to_string()), file);
    }

    #[cfg(not(unix))]
    #[test]
    fn unrepresentable_native_bytes_are_refused() {
        assert!(matches!(
            refused("ssh://h/l%FCg"),
            AddressError::UnrepresentablePath
        ));
    }

    #[test]
    fn serde_roundtrips_the_canonical_uri() {
        let file = ok("ssh://dev@box:2222/var%20log/a.log");
        let json = serde_json::to_string(&file).expect("serialize");
        assert_eq!(json, "\"ssh://dev@box:2222/var%20log/a.log\"");
        assert_eq!(
            serde_json::from_str::<RemoteFile>(&json).expect("deserialize"),
            file
        );
    }

    #[test]
    fn serde_rejects_invalid_values() {
        assert!(serde_json::from_str::<RemoteFile>("\"ssh://h/%zz\"").is_err());
        assert!(serde_json::from_str::<RemoteFile>("\"/local/path\"").is_err());
        // Field-shaped values cannot bypass the URI admission either.
        let shaped = serde_json::json!({"host": "h", "path": "/x"});
        assert!(serde_json::from_value::<RemoteFile>(shaped).is_err());
    }
}
