//! Typed byte-domain selection and window metadata. Every quantity here is
//! bytes on the remote file — never lines, characters or "units" — and the
//! constructors are the only way to build one, so an invalid length or a
//! raw `u64` confused for an offset cannot reach a read.

use serde::{Deserialize, Serialize};

/// A byte offset into a remote file. Untyped in memory, typed at the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteOffset(u64);

impl RemoteOffset {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why a [`ReadLimit`] was refused: the byte-domain bounds exist so one
/// snapshot can never ask for an unbounded or empty allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("read lengths must be between 1 byte and {} bytes", ReadLimit::MAX)]
pub struct ReadLimitError;

/// A checked read length in bytes: positive and at most [`ReadLimit::MAX`],
/// the in-memory snapshot cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct ReadLimit(u64);

impl ReadLimit {
    /// Largest window one snapshot may allocate (256 MiB).
    pub const MAX: u64 = 256 * 1024 * 1024;
    /// The default tail window (256 KiB), also follow's initial window.
    pub const DEFAULT_TAIL: Self = Self(256 * 1024);

    /// Admit one length; zero and anything above [`ReadLimit::MAX`] are
    /// refused before any transfer or allocation.
    pub fn new(value: u64) -> Result<Self, ReadLimitError> {
        if value == 0 || value > Self::MAX {
            Err(ReadLimitError)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}
impl TryFrom<u64> for ReadLimit {
    type Error = ReadLimitError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<ReadLimit> for u64 {
    fn from(value: ReadLimit) -> Self {
        value.get()
    }
}

/// A whole-file size in bytes, as captured at inspection time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteSize(u64);

impl RemoteSize {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Which bytes one read wants.
///
/// `Full` is `[0, inspected_size)`. `Range` starts at `start` and carries at
/// most `length` bytes, clamped to the inspected end. `Tail` names the last
/// `length` bytes (the whole file when it is shorter). A selection resolved
/// against the inspected size yields a [`RemoteWindow`]; growth after
/// inspection is not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReadSelection {
    Full,
    Range {
        start: RemoteOffset,
        length: ReadLimit,
    },
    Tail(ReadLimit),
}

/// The bytes a snapshot actually covers: where the content starts, how many
/// bytes it holds, and how large the whole file was when inspected. The
/// buffer contains exactly the window — nothing more is implied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RemoteWindow {
    start: RemoteOffset,
    length: RemoteSize,
    file_size: RemoteSize,
}

impl RemoteWindow {
    /// Resolve a selection against an inspected size. Every branch is
    /// clamped with checked arithmetic; an empty result is represented, not
    /// refused.
    pub fn resolve(selection: &ReadSelection, file_size: RemoteSize) -> Self {
        let size = file_size.get();
        let (start, length) = match selection {
            ReadSelection::Full => (0, size),
            ReadSelection::Range { start, length } => {
                let start = start.get().min(size);
                (start, length.get().min(size - start))
            }
            ReadSelection::Tail(length) => {
                let start = size.saturating_sub(length.get());
                (start, size - start)
            }
        };
        Self {
            start: RemoteOffset::new(start),
            length: RemoteSize::new(length),
            file_size,
        }
    }

    /// Where the covered bytes begin.
    pub fn start(&self) -> RemoteOffset {
        self.start
    }

    /// How many bytes the snapshot holds.
    pub fn length(&self) -> RemoteSize {
        self.length
    }

    /// The whole-file size captured at inspection time.
    pub fn file_size(&self) -> RemoteSize {
        self.file_size
    }

    /// True only when the snapshot covers the entire inspected file.
    pub fn is_complete(&self) -> bool {
        self.start.get() == 0 && self.length == self.file_size
    }

    /// True when this window ends at the inspected end of file — the
    /// position a tail/follow window must keep.
    pub(super) fn reaches_eof(&self) -> bool {
        self.start.get() + self.length.get() == self.file_size.get()
    }

    /// The same window narrowed to the boundary-aligned bytes actually
    /// held: the start advances by `front` and the length becomes `kept`.
    pub(super) fn narrowed(self, front: u64, kept: u64) -> Self {
        Self {
            start: RemoteOffset::new(self.start.get() + front),
            length: RemoteSize::new(kept),
            file_size: self.file_size,
        }
    }
}

/// The boundary-aligned sub-range of `bytes` that holds only complete UTF-8
/// sequences at its edges: a window may start or end mid-sequence, and the
/// snapshot trims those partial edges rather than emitting invalid text or
/// dropping the whole window. Interior invalid bytes are left in place —
/// UTF-8 validation still fails honestly on them.
pub(super) fn utf8_boundary_range(bytes: &[u8]) -> (usize, usize) {
    let mut start = 0;
    // A leading partial sequence is at most three continuation bytes.
    while start < bytes.len() && start < 3 && is_continuation(bytes[start]) {
        start += 1;
    }
    let mut end = bytes.len();
    // Find the start of the last sequence and drop it if it is cut short.
    let last = bytes[start..]
        .iter()
        .rposition(|&byte| !is_continuation(byte))
        .map(|position| start + position);
    if let Some(lead) = last {
        let expected = sequence_length(bytes[lead]);
        if lead + expected > end {
            end = lead;
        }
    }
    (start, end)
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

/// Length of a sequence from its leading byte; 1 for anything that is not a
/// valid leading byte (validation reports those separately).
fn sequence_length(lead: u8) -> usize {
    match lead {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(selection: &ReadSelection, size: u64) -> RemoteWindow {
        RemoteWindow::resolve(selection, RemoteSize::new(size))
    }

    #[test]
    fn limits_are_checked_at_construction() {
        assert!(ReadLimit::new(1).is_ok());
        assert_eq!(ReadLimit::new(512).unwrap().get(), 512);
        assert_eq!(
            ReadLimit::new(ReadLimit::MAX).unwrap().get(),
            ReadLimit::MAX
        );
        assert!(ReadLimit::new(0).is_err());
        assert!(ReadLimit::new(ReadLimit::MAX + 1).is_err());
    }

    #[test]
    fn full_covers_everything() {
        let whole = window(&ReadSelection::Full, 4096);
        assert_eq!(whole.start().get(), 0);
        assert_eq!(whole.length().get(), 4096);
        assert_eq!(whole.file_size().get(), 4096);
        assert!(whole.is_complete());
        assert!(whole.reaches_eof());
        let empty_file = window(&ReadSelection::Full, 0);
        assert!(empty_file.is_complete());
        assert_eq!(empty_file.length().get(), 0);
    }

    #[test]
    fn ranges_clamp_to_the_inspected_end() {
        let selection = ReadSelection::Range {
            start: RemoteOffset::new(100),
            length: ReadLimit::new(50).unwrap(),
        };
        let clamped = window(&selection, 120);
        assert_eq!(clamped.start().get(), 100);
        assert_eq!(clamped.length().get(), 20);
        assert!(!clamped.is_complete());
        assert!(clamped.reaches_eof());
        // A start beyond EOF clamps to an empty window at EOF.
        let past = window(
            &ReadSelection::Range {
                start: RemoteOffset::new(500),
                length: ReadLimit::new(10).unwrap(),
            },
            100,
        );
        assert_eq!(past.start().get(), 100);
        assert_eq!(past.length().get(), 0);
        // A zero-length result is represented, never refused.
        let zero = window(
            &ReadSelection::Range {
                start: RemoteOffset::new(10),
                length: ReadLimit::new(1).unwrap(),
            },
            10,
        );
        assert_eq!(zero.length().get(), 0);
        assert_eq!(zero.start().get(), 10);
    }

    #[test]
    fn tails_stop_at_the_start_of_file() {
        let selection = ReadSelection::Tail(ReadLimit::new(100).unwrap());
        let tail = window(&selection, 1000);
        assert_eq!(tail.start().get(), 900);
        assert_eq!(tail.length().get(), 100);
        assert!(tail.reaches_eof());
        assert!(!tail.is_complete());
        let short = window(&selection, 40);
        assert_eq!(short.start().get(), 0);
        assert_eq!(short.length().get(), 40);
        assert!(short.is_complete());
        let empty = window(
            &ReadSelection::Tail(ReadLimit::new(ReadLimit::MAX).unwrap()),
            0,
        );
        assert_eq!(empty.length().get(), 0);
    }

    #[test]
    fn deserialization_cannot_bypass_read_admission() {
        assert!(serde_json::from_str::<ReadLimit>("0").is_err());
        assert!(serde_json::from_str::<ReadLimit>(&((ReadLimit::MAX + 1).to_string())).is_err());
    }
}
