//! Same-source framing arithmetic (0058 WK18/WK19). The byte boundary's
//! decoder (`strop-worker-protocol::frame`) calls these exact decisions;
//! the bounds live here so the policy has one definition and the wire
//! codec re-exports it. The kernels are total for any header offset that
//! can come out of a real buffer search (bounded by `isize::MAX`, which
//! is also the `Vec` length ceiling), so the arithmetic cannot wrap.

use vstd::prelude::*;

verus! {

/// Header bytes accepted before the terminator must exist (the shared
/// decoder bound). Mirrors the wire codec's published constant.
pub const MAX_HEADER_BYTES: usize = 8192;

/// Body bound: one control envelope, never a file. Bulk payloads are
/// stream chunks.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The most an in-memory search offset can be: every real buffer is a
/// `Vec`, whose length is bounded by `isize::MAX`.
pub const MAX_BUFFER_OFFSET: usize = usize::MAX / 2;

/// A complete header (including its `\r\n\r\n` terminator) fits the
/// header bound.
pub open spec fn header_fits(header_end: usize) -> bool {
    header_end + 4 <= MAX_HEADER_BYTES
}

/// A declared body length fits the body bound.
pub open spec fn body_fits(length: usize) -> bool {
    length <= MAX_BODY_BYTES
}

/// The buffered bytes cover the whole frame: terminator plus body.
pub open spec fn frame_complete(header_end: usize, length: usize, buffered: usize) -> bool {
    buffered >= header_end + 4 + length
}

/// Header admission: `header_end` is the offset of the terminator, so the
/// header occupies `header_end + 4` bytes.
pub fn header_admitted(header_end: usize) -> (fits: bool)
    requires header_end <= MAX_BUFFER_OFFSET,
    ensures fits == header_fits(header_end),
{
    header_end + 4 <= MAX_HEADER_BYTES
}

/// Body admission at header-parse time: refuse before buffering.
pub fn body_admitted(length: usize) -> (fits: bool)
    ensures fits == body_fits(length),
{
    length <= MAX_BODY_BYTES
}

/// Frame completeness: enough bytes are buffered to take the body.
pub fn frame_ready(header_end: usize, length: usize, buffered: usize) -> (complete: bool)
    requires
        header_end <= MAX_BUFFER_OFFSET,
        header_fits(header_end),
        body_fits(length),
        buffered <= MAX_BUFFER_OFFSET,
    ensures complete == frame_complete(header_end, length, buffered),
{
    buffered >= header_end + 4 + length
}

/// The load-bearing framing theorem: a frame the decoder reports ready
/// yields slice indices strictly inside the buffer — the take in
/// `FrameDecoder::next_frame` cannot go out of bounds or wrap.
proof fn ready_frame_slices_in_bounds(
    header_end: usize,
    length: usize,
    buffered: usize,
)
    requires
        header_end <= MAX_BUFFER_OFFSET,
        header_fits(header_end),
        body_fits(length),
        buffered <= MAX_BUFFER_OFFSET,
        frame_complete(header_end, length, buffered),
    ensures
        header_end + 4 + length <= buffered,
        header_end + 4 <= buffered,
        (header_end + 4) + length <= buffered,
{
}

} // verus!
