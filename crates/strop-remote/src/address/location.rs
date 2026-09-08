//! What a user actually types: a remote location that may still be
//! unresolved. Only the *raw* URI spelling decides whether a leading `~`
//! component is a home query (`/~/log`) or a literal tilde directory
//! (`/%7E/log`) — the decoded bytes alone cannot, and a session must never
//! guess. Home queries stay unresolved here until a negotiated
//! `expand-path@openssh.com` exchange publishes the canonical file.

use super::endpoint::RemoteEndpoint;
use super::error::AddressError;
use super::file::RemoteFile;
use super::uri::{self, path_bytes};
use std::path::{Path, PathBuf};

/// A remote location as entered: either an already-canonical file, or an
/// unresolved home-relative query on one endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteLocation(LocationInner);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum LocationInner {
    File(RemoteFile),
    Home {
        endpoint: RemoteEndpoint,
        /// `~/log`, `~alice/log` — bytes exactly as decoded, tilde included.
        relative: PathBuf,
    },
}

impl RemoteLocation {
    /// Admit one textual location. A raw `~` first component yields the
    /// unresolved home form; everything else must decode to a canonical
    /// absolute file. Pure: no filesystem, no subprocess, no home guessing.
    pub fn parse(value: &str) -> Result<Self, AddressError> {
        let (endpoint, tail) = RemoteEndpoint::split_authority(value)?;
        match tail.as_bytes().first() {
            None => return Err(AddressError::AbsentPath),
            Some(b'/') => {}
            Some(_) => return Err(AddressError::QueryOrFragment),
        }
        if tail.contains(['?', '#']) {
            return Err(AddressError::QueryOrFragment);
        }
        let home = tail.as_bytes().get(1) == Some(&b'~');
        let path = uri::decode_path(tail)?;
        let inner = if home {
            LocationInner::Home {
                endpoint,
                relative: uri::strip_leading_slash(path),
            }
        } else {
            LocationInner::File(RemoteFile::from_path(endpoint, path)?)
        };
        Ok(Self(inner))
    }

    /// The endpoint this location addresses — the pooled-session identity
    /// even before the path is resolved.
    pub fn endpoint(&self) -> &RemoteEndpoint {
        match &self.0 {
            LocationInner::File(file) => file.endpoint(),
            LocationInner::Home { endpoint, .. } => endpoint,
        }
    }

    /// The canonical file, when this location is already absolute and
    /// resolved. A home query returns `None`: only a session that negotiated
    /// `expand-path@openssh.com` may publish its canonical identity.
    pub fn absolute_file(&self) -> Option<&RemoteFile> {
        match &self.0 {
            LocationInner::File(file) => Some(file),
            LocationInner::Home { .. } => None,
        }
    }

    /// The unresolved home-relative bytes (`~/log`), exactly as decoded.
    pub(crate) fn home_relative(&self) -> Option<&Path> {
        match &self.0 {
            LocationInner::File(_) => None,
            LocationInner::Home { relative, .. } => Some(relative),
        }
    }

    fn canonical(&self) -> String {
        match &self.0 {
            LocationInner::File(file) => file.to_string(),
            LocationInner::Home { endpoint, relative } => {
                let bytes = path_bytes(relative);
                let mut out = String::with_capacity(endpoint.to_string().len() + bytes.len() * 3);
                out.push_str(&endpoint.to_string());
                // The raw `~` is what marks a home query; later `~` bytes
                // inside the path are ordinary data.
                out.push('/');
                if bytes.first() == Some(&b'~') {
                    out.push('~');
                    uri::push_escaped(&mut out, &bytes[1..]);
                } else {
                    uri::push_escaped(&mut out, bytes);
                }
                out
            }
        }
    }
}

impl From<RemoteFile> for RemoteLocation {
    fn from(file: RemoteFile) -> Self {
        Self(LocationInner::File(file))
    }
}

impl std::fmt::Display for RemoteLocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.canonical())
    }
}

/// Serde carries the canonical validated URI — one string, re-parsed on
/// the way in, so stored values can never bypass admission.
impl serde::Serialize for RemoteLocation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.canonical())
    }
}

impl<'de> serde::Deserialize<'de> for RemoteLocation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        RemoteLocation::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ok(uri: &str) -> RemoteLocation {
        RemoteLocation::parse(uri).unwrap_or_else(|e| panic!("expected parse: {uri}: {e}"))
    }

    #[test]
    fn absolute_locations_expose_their_canonical_file() {
        let location = ok("ssh://dev@box:2222/var/log/app.log");
        let file = location.absolute_file().expect("absolute");
        assert_eq!(file.path(), Path::new("/var/log/app.log"));
        assert_eq!(location.endpoint().host(), "box");
        assert_eq!(location.to_string(), "ssh://dev@box:2222/var/log/app.log");
        assert_eq!(
            ok("ssh://h/").absolute_file().unwrap().path(),
            Path::new("/")
        );
    }

    #[test]
    fn home_queries_stay_unresolved_and_round_trip() {
        for uri in ["ssh://h/~/log/app.log", "ssh://h/~", "ssh://h/~alice/x"] {
            let location = ok(uri);
            assert!(location.absolute_file().is_none(), "{uri}");
            assert_eq!(location.to_string(), uri);
            assert_eq!(ok(&location.to_string()), location);
        }
        assert_eq!(
            ok("ssh://h/~/log/app.log").home_relative(),
            Some(Path::new("~/log/app.log"))
        );
        assert_eq!(
            ok("ssh://h/~alice/x").home_relative(),
            Some(Path::new("~alice/x"))
        );
    }

    #[test]
    fn a_literal_escaped_tilde_is_not_a_home_query() {
        let literal = ok("ssh://h/%7E/log/app.log");
        let file = literal.absolute_file().expect("literal tilde is canonical");
        assert_eq!(file.path(), Path::new("/~/log/app.log"));
        // Identity survives the round trip without ever becoming `~`.
        assert_eq!(literal.to_string(), "ssh://h/%7E/log/app.log");
        let home = ok("ssh://h/~/log/app.log");
        assert_ne!(literal, home);
        assert!(home.absolute_file().is_none());
        // A `~` beyond the first component is data, not a home marker.
        let inner = ok("ssh://h/var/~tmp/x");
        assert_eq!(
            inner.absolute_file().unwrap().path(),
            Path::new("/var/~tmp/x")
        );
    }

    #[test]
    fn files_convert_into_locations() {
        let file = RemoteFile::parse("ssh://h/var%20log/a").unwrap();
        let location = RemoteLocation::from(file.clone());
        assert_eq!(location.absolute_file(), Some(&file));
        assert_eq!(location, ok("ssh://h/var%20log/a"));
    }

    #[test]
    fn admission_failures_match_the_file_grammar() {
        for uri in ["ssh://host", "scp://host/x", "", "SSH://h/x"] {
            assert!(RemoteLocation::parse(uri).is_err(), "{uri}");
        }
        assert!(matches!(
            RemoteLocation::parse("ssh://h/a?b"),
            Err(AddressError::QueryOrFragment)
        ));
        assert!(matches!(
            RemoteLocation::parse("ssh://h/%zz"),
            Err(AddressError::MalformedPercentEscape)
        ));
    }

    #[test]
    fn serde_roundtrips_both_forms() {
        for uri in ["ssh://dev@box:2222/var%20log/a.log", "ssh://h/~/log"] {
            let location = ok(uri);
            let json = serde_json::to_string(&location).expect("serialize");
            assert_eq!(
                serde_json::from_str::<RemoteLocation>(&json).expect("deserialize"),
                location
            );
        }
    }
}
