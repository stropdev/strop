//! The validated connection identity: `ssh://[user@]host[:port]`. Private
//! fields leave [`RemoteEndpoint::parse`] as the only constructor, so every
//! value — including deserialized ones — passed the same admission and can
//! never be option-shaped toward the `ssh` argv.

use super::error::AddressError;
use std::num::NonZeroU16;
use std::str::FromStr;

/// A validated `ssh://[user@]host[:port]` endpoint.
///
/// One endpoint names one pooled session. Aliases keep their spelling: this
/// layer does not infer equivalence between OpenSSH configuration targets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteEndpoint {
    user: Option<String>,
    /// Alias/hostname verbatim, or the canonical unbracketed IP form.
    host: String,
    port: Option<NonZeroU16>,
}

impl RemoteEndpoint {
    /// Admit one endpoint-only location: no path is allowed. Pure: no
    /// filesystem, no subprocess.
    pub fn parse(value: &str) -> Result<Self, AddressError> {
        let (endpoint, rest) = parse_authority(value)?;
        if !rest.is_empty() {
            return Err(AddressError::PathOnEndpoint);
        }
        Ok(endpoint)
    }

    /// Split `ssh://[user@]host[:port]` off the front of `value`, returning
    /// the endpoint and the untouched remainder (empty, or a path region).
    pub(super) fn split_authority(value: &str) -> Result<(Self, &str), AddressError> {
        parse_authority(value)
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

    /// The canonical authority spelling `ssh://[user@]host[:port]`.
    fn authority(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::with_capacity("ssh://".len() + self.host.len() + 8);
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
        out
    }
}

impl FromStr for RemoteEndpoint {
    type Err = AddressError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl std::fmt::Display for RemoteEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.authority())
    }
}

/// Serde carries the canonical validated URI — one string, re-parsed on
/// the way in, so stored values can never bypass admission.
impl serde::Serialize for RemoteEndpoint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.authority())
    }
}

impl<'de> serde::Deserialize<'de> for RemoteEndpoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        RemoteEndpoint::parse(&text).map_err(serde::de::Error::custom)
    }
}

fn parse_authority(value: &str) -> Result<(RemoteEndpoint, &str), AddressError> {
    let rest = value
        .strip_prefix("ssh://")
        .ok_or(AddressError::NotSshUri)?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (user, host_port) = split_user(&rest[..authority_end])?;
    let (host, port) = split_host(host_port)?;
    Ok((RemoteEndpoint { user, host, port }, &rest[authority_end..]))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn ok(uri: &str) -> RemoteEndpoint {
        RemoteEndpoint::parse(uri).unwrap_or_else(|error| panic!("expected parse: {uri}: {error}"))
    }

    fn refused(uri: &str) -> AddressError {
        RemoteEndpoint::parse(uri).unwrap_err()
    }

    #[test]
    fn alias_with_user_and_port() {
        let endpoint = ok("ssh://dev@bbgithub:2222");
        assert_eq!(endpoint.user(), Some("dev"));
        assert_eq!(endpoint.host(), "bbgithub");
        assert_eq!(endpoint.port(), NonZeroU16::new(2222));
        assert_eq!(endpoint.to_string(), "ssh://dev@bbgithub:2222");
    }

    #[test]
    fn a_path_is_refused_on_an_endpoint() {
        assert!(matches!(
            refused("ssh://host/var/log"),
            AddressError::PathOnEndpoint
        ));
    }

    #[test]
    fn aliases_keep_their_spelling() {
        assert_ne!(ok("ssh://Box"), ok("ssh://box"));
        assert_eq!(ok("ssh://dev_01.a").host(), "dev_01.a");
    }

    #[test]
    fn ipv6_brackets_canonicalize_and_roundtrip() {
        let endpoint = ok("ssh://root@[2001:0db8:0000::0001]:22");
        assert_eq!(endpoint.host(), "2001:db8::1");
        assert_eq!(endpoint.to_string(), "ssh://root@[2001:db8::1]:22");
        assert_eq!(endpoint, ok("ssh://root@[2001:0db8:0:0:0:0:0:1]:0022"));
        let bare = ok("ssh://[::1]");
        assert_eq!(bare.host(), "::1");
        assert_eq!(bare.port(), None);
        assert_eq!(bare.to_string(), "ssh://[::1]");
    }

    #[test]
    fn ipv6_hostility() {
        for (uri, expected) in [
            ("ssh://fe80::1", AddressError::UnbracketedIpv6),
            ("ssh://user@::1", AddressError::UnbracketedIpv6),
            ("ssh://[::1", AddressError::UnclosedIpv6),
            ("ssh://[::1]extra", AddressError::JunkAfterIpv6),
        ] {
            assert_eq!(refused(uri), expected, "{uri}");
        }
        assert!(matches!(
            refused("ssh://[zz]"),
            AddressError::InvalidIpv6 { .. }
        ));
    }

    #[test]
    fn ipv4_shapes_must_be_addresses() {
        assert_eq!(ok("ssh://127.0.0.1").host(), "127.0.0.1");
        for uri in [
            "ssh://999.1.1.1",
            "ssh://1.2.3.4.5",
            "ssh://1.2.3",
            "ssh://0",
        ] {
            assert!(
                matches!(refused(uri), AddressError::InvalidIpv4 { .. }),
                "{uri}"
            );
        }
    }

    #[test]
    fn port_boundaries() {
        assert_eq!(ok("ssh://h:022").port(), NonZeroU16::new(22));
        for uri in [
            "ssh://h:0",
            "ssh://h:",
            "ssh://h:65536",
            "ssh://h:notaport",
            "ssh://h:+2",
            "ssh://h: 2",
        ] {
            assert!(matches!(refused(uri), AddressError::InvalidPort), "{uri}");
        }
    }

    #[test]
    fn hostile_authorities() {
        assert!(matches!(
            refused("ssh://user:secret@host"),
            AddressError::PasswordInUri
        ));
        assert!(matches!(
            refused("ssh://-oProxyCommand=evil@host"),
            AddressError::OptionShapedEndpoint
        ));
        assert!(matches!(
            refused("ssh://name=-x@host"),
            AddressError::OptionShapedEndpoint
        ));
        assert!(matches!(
            refused("ssh://-flag"),
            AddressError::OptionShapedEndpoint
        ));
        assert!(matches!(refused("ssh://@host"), AddressError::EmptyUser));
        assert!(matches!(refused("ssh://h@"), AddressError::EmptyHost));
        assert!(matches!(refused("ssh://"), AddressError::EmptyHost));
        assert!(matches!(
            refused("ssh://a@b@c"),
            AddressError::InvalidAuthorityCharacter
        ));
        assert!(matches!(
            refused("ssh://ho%st"),
            AddressError::InvalidAuthorityCharacter
        ));
        assert!(matches!(
            refused("ssh://ho st"),
            AddressError::InvalidAuthorityCharacter
        ));
        for uri in ["host", "scp://host", "", "SSH://host"] {
            assert!(matches!(refused(uri), AddressError::NotSshUri), "{uri}");
        }
    }

    #[test]
    fn identity_and_hashing_follow_the_decoded_value() {
        assert_eq!(ok("ssh://h:022"), ok("ssh://h:22"));
        assert_eq!(ok("ssh://[::1]"), ok("ssh://[0:0:0:0:0:0:0:1]"));
        let mut seen = HashSet::new();
        seen.insert(ok("ssh://h:022"));
        seen.insert(ok("ssh://h:22"));
        seen.insert(ok("ssh://[::1]"));
        seen.insert(ok("ssh://[0::1]"));
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn serde_roundtrips_the_canonical_uri() {
        let endpoint = ok("ssh://dev@box:2222");
        let json = serde_json::to_string(&endpoint).expect("serialize");
        assert_eq!(json, "\"ssh://dev@box:2222\"");
        assert_eq!(
            serde_json::from_str::<RemoteEndpoint>(&json).expect("deserialize"),
            endpoint
        );
        assert!(serde_json::from_str::<RemoteEndpoint>("\"ssh://h/x\"").is_err());
    }
}
