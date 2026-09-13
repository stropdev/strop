//! The URI byte grammar shared by every address form: percent decoding,
//! native filename bytes and the canonical escape rules. Nothing here
//! interprets a path — metacharacters are data, and a `~` byte is just a
//! byte; whether it means "home" is decided by the *raw* spelling in
//! [`super::location`], never by the decoded value.

use super::error::AddressError;
use std::path::PathBuf;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Decode a URI path region to native filename bytes. Raw bytes outside the
/// path grammar are refused (percent-encode them); raw non-ASCII UTF-8
/// passes through as data. Remote metacharacters are never special.
/// Decoding alone grants no authority and does not require an absolute path.
pub fn decode_path(text: &str) -> Result<PathBuf, AddressError> {
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

/// Append the canonical URI spelling of native path bytes: only necessary
/// percent escapes, everything the grammar allows stays raw.
pub(crate) fn push_escaped(out: &mut String, bytes: &[u8]) {
    use std::fmt::Write as _;
    for &byte in bytes {
        if is_raw_path_byte(byte) {
            out.push(byte as char);
        } else {
            let _ = write!(
                out,
                "%{}{}",
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 0x0F) as usize] as char
            );
        }
    }
}

/// Lossless URI path spelling for resource references and native Trash metadata.
/// This encoder does not grant resource authority; admission still validates paths.
pub fn encode_path(path: &std::path::Path) -> String {
    let mut value = String::new();
    push_escaped(&mut value, path_bytes(path));
    value
}

/// Caller checked `is_ascii_hexdigit`.
pub(super) fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => unreachable!("hex_value called on a non-hex byte"),
    }
}

/// RFC 3986 pchar plus `/`: bytes a canonical URI carries raw.
pub(super) fn is_raw_path_byte(byte: u8) -> bool {
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

/// Drop one leading `/` from a decoded absolute path, yielding the bytes a
/// home query carries (`/~/log` -> `~/log`). The caller proved the slash.
pub(super) fn strip_leading_slash(path: PathBuf) -> PathBuf {
    let bytes = path_bytes(&path);
    debug_assert_eq!(bytes.first(), Some(&b'/'));
    let stripped = bytes[1..].to_vec();
    bytes_to_path(stripped).expect("rebuilding a decoded path cannot fail")
}

#[cfg(unix)]
pub fn bytes_to_path(bytes: Vec<u8>) -> Result<PathBuf, AddressError> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(unix)]
pub fn path_bytes(path: &std::path::Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

/// Platforms without byte filenames get strict UTF-8, never a lossy
/// replacement character that could name the wrong remote file.
#[cfg(not(unix))]
pub fn bytes_to_path(bytes: Vec<u8>) -> Result<PathBuf, AddressError> {
    let text = String::from_utf8(bytes).map_err(|_| AddressError::UnrepresentablePath)?;
    Ok(PathBuf::from(text))
}

#[cfg(not(unix))]
pub fn path_bytes(path: &std::path::Path) -> &[u8] {
    // Built only from strict UTF-8 here, so this is the exact path text.
    path.as_os_str().as_encoded_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn decoding_refuses_malformed_escapes_and_keeps_data_bytes() {
        assert_eq!(decode_path("/a%20b%2Fc").unwrap(), Path::new("/a b/c"));
        assert!(matches!(
            decode_path("/%zz"),
            Err(AddressError::MalformedPercentEscape)
        ));
        assert!(matches!(decode_path("/%00"), Err(AddressError::NulInPath)));
    }

    #[test]
    fn escaping_reproduces_only_necessary_escapes() {
        let mut out = String::new();
        push_escaped(&mut out, b"/var log/a#b%c?d");
        assert_eq!(out, "/var%20log/a%23b%25c%3Fd");
    }
}
