//! CLI locations retain native filenames and convert one-based lines once.
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::files::FileTarget;
use strop_core::id::LineIndex;
use strop_workspace::RemoteLocation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileLocation {
    pub path: FileTarget,
    pub line: Option<LineIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocationError {
    InvalidLine,
    MissingPath,
    DuplicateLine,
    Remote(strop_workspace::AddressError),
}

impl std::fmt::Display for LocationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLine => "line must be a positive integer within the platform range",
            Self::MissingPath => "a line location requires a file path",
            Self::DuplicateLine => "specify the initial line only once (+LINE or FILE:LINE)",
            Self::Remote(error) => return error.fmt(formatter),
        })
    }
}
impl std::error::Error for LocationError {}

impl FileLocation {
    pub fn parse(value: OsString, literal: bool) -> Result<Self, LocationError> {
        if !literal {
            if let Some(uri) = value.to_str().filter(|text| text.starts_with("ssh://")) {
                // FILE[:LINE] parity with local files: a trailing
                // :digits after the last '/' is the line, so a pasted
                // grep result opens where it points. The authority's
                // :port is before the first '/', never ambiguous.
                let (uri, line) = match uri.rfind('/') {
                    Some(slash) => match uri[slash + 1..].rfind(':') {
                        Some(colon) => {
                            let digits = &uri[slash + 1 + colon + 1..];
                            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                                (&uri[..slash + 1 + colon], Some(parse_line(digits)?))
                            } else {
                                (uri, None)
                            }
                        }
                        None => (uri, None),
                    },
                    None => (uri, None),
                };
                return Ok(Self {
                    path: FileTarget::Remote(
                        RemoteLocation::parse(uri).map_err(LocationError::Remote)?,
                    ),
                    line,
                });
            }
            if let Some((path, line)) = suffix(&value)? {
                return Ok(Self {
                    path: FileTarget::Local(path),
                    line: Some(line),
                });
            }
        }
        Ok(Self {
            path: FileTarget::Local(value.into()),
            line: None,
        })
    }

    pub fn with_line(mut self, line: Option<LineIndex>) -> Result<Self, LocationError> {
        if line.is_some() && self.line.is_some() {
            return Err(LocationError::DuplicateLine);
        }
        self.line = self.line.or(line);
        Ok(self)
    }
}

pub(crate) fn parse_line(text: &str) -> Result<LineIndex, LocationError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(LocationError::InvalidLine);
    }
    let line = text
        .parse::<usize>()
        .map_err(|_| LocationError::InvalidLine)?;
    line.checked_sub(1)
        .map(LineIndex::new)
        .ok_or(LocationError::InvalidLine)
}

#[cfg(unix)]
fn suffix(value: &OsStr) -> Result<Option<(PathBuf, LineIndex)>, LocationError> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = value.as_bytes();
    let Some(colon) = bytes.iter().rposition(|&byte| byte == b':') else {
        return Ok(None);
    };
    let digits = &bytes[colon + 1..];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    if colon == 0 {
        return Err(LocationError::MissingPath);
    }
    let line = parse_line(std::str::from_utf8(digits).map_err(|_| LocationError::InvalidLine)?)?;
    Ok(Some((
        PathBuf::from(OsStr::from_bytes(&bytes[..colon])),
        line,
    )))
}

#[cfg(windows)]
fn suffix(value: &OsStr) -> Result<Option<(PathBuf, LineIndex)>, LocationError> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units: Vec<u16> = value.encode_wide().collect();
    let Some(colon) = units.iter().rposition(|&unit| unit == u16::from(b':')) else {
        return Ok(None);
    };
    let digits = &units[colon + 1..];
    if digits.is_empty()
        || !digits
            .iter()
            .all(|&unit| (u16::from(b'0')..=u16::from(b'9')).contains(&unit))
    {
        return Ok(None);
    }
    if colon == 0 {
        return Err(LocationError::MissingPath);
    }
    let text: String = digits.iter().map(|&unit| char::from(unit as u8)).collect();
    Ok(Some((
        PathBuf::from(OsString::from_wide(&units[..colon])),
        parse_line(&text)?,
    )))
}

#[cfg(not(any(unix, windows)))]
fn suffix(value: &OsStr) -> Result<Option<(PathBuf, LineIndex)>, LocationError> {
    let Some(text) = value.to_str() else {
        return Ok(None);
    };
    let Some((path, digits)) = text.rsplit_once(':') else {
        return Ok(None);
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(None);
    }
    if path.is_empty() {
        return Err(LocationError::MissingPath);
    }
    Ok(Some((path.into(), parse_line(digits)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_and_literal_operands_have_distinct_meanings() {
        let location = FileLocation::parse("dir with spaces/file.txt:3".into(), false).unwrap();
        assert_eq!(
            location.path.local_path(),
            Some(std::path::Path::new("dir with spaces/file.txt"))
        );
        assert_eq!(location.line, Some(LineIndex::new(2)));
        let literal = FileLocation::parse("file.txt:3".into(), true).unwrap();
        assert_eq!(
            literal.path.local_path(),
            Some(std::path::Path::new("file.txt:3"))
        );
        assert_eq!(literal.line, None);
    }

    #[test]
    fn invalid_or_ambiguous_locations_fail_before_opening() {
        assert_eq!(parse_line("0"), Err(LocationError::InvalidLine));
        assert_eq!(
            parse_line("999999999999999999999999999999"),
            Err(LocationError::InvalidLine)
        );
        assert_eq!(
            FileLocation::parse(":3".into(), false),
            Err(LocationError::MissingPath)
        );
        let location = FileLocation::parse("f:3".into(), false).unwrap();
        assert_eq!(
            location.with_line(Some(LineIndex::new(4))),
            Err(LocationError::DuplicateLine)
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_filename_bytes_survive_location_suffix_removal() {
        use std::os::unix::ffi::OsStrExt;
        let location =
            FileLocation::parse(OsStr::from_bytes(b"\xff.rs:3").to_owned(), false).unwrap();
        assert_eq!(
            location.path.local_path().unwrap().as_os_str().as_bytes(),
            b"\xff.rs"
        );
        assert_eq!(location.line, Some(LineIndex::new(2)));
    }

    #[test]
    fn ssh_urls_accept_a_line_suffix_like_local_files() {
        // pasting a grep result opens where it points; the authority's
        // :port precedes the first '/', never ambiguous with the suffix
        let location = FileLocation::parse("ssh://host/path/f.txt:3".into(), false).unwrap();
        assert!(matches!(location.path, FileTarget::Remote(_)));
        assert_eq!(location.line, Some(LineIndex::new(2)));
        let FileTarget::Remote(remote) = location.path else {
            unreachable!()
        };
        assert_eq!(remote.to_string(), "ssh://host/path/f.txt");
        // user@host:port with a line
        let location =
            FileLocation::parse("ssh://user@host:2222/path/f.txt:10".into(), false).unwrap();
        assert_eq!(location.line, Some(LineIndex::new(9)));
        // no numeric suffix: the path keeps its colons
        let location = FileLocation::parse("ssh://host/path/f:txt".into(), false).unwrap();
        assert_eq!(location.line, None);
        let FileTarget::Remote(remote) = location.path else {
            unreachable!()
        };
        assert_eq!(remote.to_string(), "ssh://host/path/f:txt");
    }
}
