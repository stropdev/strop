//! Content-Length framing admission (0058 WK02). The worker wire decoder
//! (`strop-worker-protocol::frame`) calls these exact decisions; the bounds
//! live here so the policy has one definition.
//!
//! Proved (proofs/Kernel/Frame.lean): each admission is exactly its bound;
//! for a buffer that fits in memory, a frame reported ready has its body
//! slice `[header_end + 4, header_end + 4 + length)` inside the buffer, and
//! none of the arithmetic can overflow.

/// Header bytes accepted before the `\r\n\r\n` terminator must exist.
pub const MAX_HEADER_BYTES: usize = 8192;

/// Body bound: one control envelope, never a file. Bulk payloads are stream
/// chunks.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Header admission: `header_end` is the offset of the terminator, so the
/// header occupies `header_end + 4` bytes. Total for any offset.
pub fn header_admitted(header_end: usize) -> bool {
    header_end < MAX_HEADER_BYTES - 3
}

/// Body admission at header-parse time: refuse before buffering.
pub fn body_admitted(length: usize) -> bool {
    length <= MAX_BODY_BYTES
}

/// Frame completeness for an admitted header and body: enough bytes are
/// buffered to take the terminator and the body. Cannot overflow: both
/// operands are bounded by their admissions.
pub fn frame_ready(header_end: usize, length: usize, buffered: usize) -> bool {
    if !header_admitted(header_end) || !body_admitted(length) {
        return false;
    }
    buffered >= header_end + 4 + length
}
