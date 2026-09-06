//! Plain-substring search helpers (prototype; 0001 §2.5's transpiled
//! regex lands with the real search layer).

use strop_core::Buffer;

/// Search backward for `pat` before `from` (prototype: plain substring).
pub fn search_backward(buf: &Buffer, from: usize, pat: &str) -> Option<usize> {
    // stale cascade positions clamp, not panic
    let from = from.min(buf.len_bytes());
    if from == 0 {
        return None;
    }
    let text = buf.rope.byte_slice(..from).to_string();
    text.rfind(pat)
}

pub fn search_forward(buf: &Buffer, from: usize, pat: &str) -> Option<usize> {
    let text = buf.rope.byte_slice(from.min(buf.len_bytes())..).to_string();
    text.find(pat).map(|i| from + i)
}

/// All matches of `pat` (incsearch highlight).
pub fn search_all(buf: &Buffer, pat: &str) -> Vec<usize> {
    if pat.is_empty() {
        return vec![];
    }
    // prototype: materializes; §2.5 promises rope-chunk search before M0 ships
    let text = buf.rope.to_string();
    text.match_indices(pat).map(|(i, _)| i).collect()
}
