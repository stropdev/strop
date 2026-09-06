//! strop-core: the buffer. A rope, byte-offset positions, edit ops.
//! No UI, no modes, no grammar — the thing everything else edits.

mod buffer;
pub mod diagnostics;
pub mod history;
pub mod id;
pub mod layout;
mod range;
pub mod selection;

pub use buffer::{Buffer, InputEdit};
pub use range::{MotionShape, Range};

#[cfg(test)]
mod persistence_tests;
