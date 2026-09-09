//! Why a textual remote address was refused. Every variant is a pure
//! admission failure; none implies a subprocess was ever started.

/// Why a textual remote address was refused.
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
    #[error(
        "an endpoint carries no path: this value names files, so parse it as a location or file instead"
    )]
    PathOnEndpoint,
    #[error(
        "the path begins with `~` (a home query), which is not a canonical file; a session must expand it first"
    )]
    UnresolvedHome,
    #[error("a canonical remote path is absolute: it must start with `/`")]
    RelativePath,
}
