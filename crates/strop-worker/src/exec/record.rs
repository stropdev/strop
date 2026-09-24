//! Terminal status records: the supervised target's final state, encoded
//! exactly as the 0036 Python anchor wrote it — one kind byte plus a
//! little-endian u32 — and rendered as nonce-marked lines when it must cross
//! a byte stream shared with target output.
//!
//! The nonce separates this session's records from coincidental target
//! output. It is not a secret and does not authenticate a target that
//! deliberately echoes it; that honesty limit is documented, not fixed.

/// The line prefix every supervisor record carries: `\nSTROP-SUP-v1
/// <nonce-hex> <fields>\n` on the worker's private diagnostics stream.
pub const MARK: &[u8] = b"STROP-SUP-v1";

/// One target's recorded terminal state. The 5-byte form is byte-compatible
/// with the Python anchor's record (`<BI`: kind, then little-endian code), so
/// evidence and decoders written for the remote supervisor still read the
/// native one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRecord {
    /// The target exited with this code. Non-zero codes are data, not
    /// transport failures.
    Exited(u32),
    /// The target died on this signal.
    Signaled(u32),
    /// The target process could not be created; the code is the fork errno.
    LaunchFailed(u32),
}

impl StatusRecord {
    /// The canonical 5-byte record.
    pub fn encode(&self) -> [u8; 5] {
        let (kind, code) = match self {
            Self::Exited(code) => (0_u8, *code),
            Self::Signaled(signal) => (1_u8, *signal),
            Self::LaunchFailed(errno) => (2_u8, *errno),
        };
        let mut out = [0_u8; 5];
        out[0] = kind;
        out[1..].copy_from_slice(&code.to_le_bytes());
        out
    }

    /// Parse one canonical record. Rejects short buffers and unknown kinds.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 5 {
            return None;
        }
        let code = u32::from_le_bytes(bytes[1..5].try_into().ok()?);
        match bytes[0] {
            0 => Some(Self::Exited(code)),
            1 => Some(Self::Signaled(code)),
            2 => Some(Self::LaunchFailed(code)),
            _ => None,
        }
    }

    /// The record's line form after the nonce: `exit 3`, `signal 15`.
    pub fn line(&self) -> String {
        match self {
            Self::Exited(code) => format!("exit {code}"),
            Self::Signaled(signal) => format!("signal {signal}"),
            Self::LaunchFailed(errno) => format!("error worker-fork {errno}"),
        }
    }
}

/// One parsed supervisor status line. `LaunchFailure` outranks a later exit
/// record: the program never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisionOutcome {
    Exited(u32),
    Signaled(u32),
    Cancelled,
    LaunchFailure(String),
    SupervisorError(String),
}

/// Render one nonce-marked record line, exactly the supervisor's framing:
/// a leading newline keeps a partial target line from absorbing the mark.
pub fn mark_line(nonce: &[u8; 16], text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + MARK.len() + 32 + text.len());
    out.push(b'\n');
    out.extend_from_slice(MARK);
    out.push(b' ');
    out.extend_from_slice(hex(nonce).as_bytes());
    out.push(b' ');
    out.extend_from_slice(text.as_bytes());
    out.push(b'\n');
    out
}

/// Extract this session's nonce-marked records from a captured byte stream,
/// in order. Only lines carrying this session's nonce count; a target
/// printing a lookalike line without it is ignored.
pub fn records(stream: &[u8], nonce: &[u8; 16]) -> Vec<SupervisionOutcome> {
    let nonce_hex = hex(nonce);
    let mut found = Vec::new();
    for line in stream.split(|&byte| byte == b'\n') {
        let mut fields = line
            .split(u8::is_ascii_whitespace)
            .filter(|part| !part.is_empty());
        if fields.next() != Some(MARK) {
            continue;
        }
        if fields.next() != Some(nonce_hex.as_bytes()) {
            continue;
        }
        let kind = match fields.next() {
            Some(kind) => kind,
            None => continue,
        };
        let outcome = match kind {
            b"exit" => fields
                .next()
                .and_then(number)
                .map(SupervisionOutcome::Exited),
            b"signal" => fields
                .next()
                .and_then(number)
                .map(SupervisionOutcome::Signaled),
            b"cancel" => Some(SupervisionOutcome::Cancelled),
            b"exec-error" => Some(SupervisionOutcome::LaunchFailure(rest(&mut fields))),
            b"error" => Some(SupervisionOutcome::SupervisorError(rest(&mut fields))),
            _ => None,
        };
        if let Some(outcome) = outcome {
            found.push(outcome);
        }
    }
    found
}

/// The remaining fields of a record line as one space-joined string.
fn rest<'a>(fields: &mut impl Iterator<Item = &'a [u8]>) -> String {
    let mut text = Vec::new();
    for field in fields {
        if !text.is_empty() {
            text.push(b' ');
        }
        text.extend_from_slice(field);
    }
    String::from_utf8_lossy(&text).into_owned()
}

fn number(field: &[u8]) -> Option<u32> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

fn hex(nonce: &[u8; 16]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(32);
    for byte in nonce {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
