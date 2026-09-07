//! strop-core: the buffer. A rope, byte-offset positions, edit ops.
//! No UI, no modes, no grammar — the thing everything else edits.

mod buffer;
pub mod diagnostics;
pub mod history;
pub mod id;
pub mod layout;
pub mod path_serde;
pub mod process;
mod range;
pub mod selection;
pub mod worker;

pub use buffer::{
    Buffer, BufferSeed, Change, ChangeOrigin, EditError, HistoryMove, InputEdit,
    PreparedReplacements, Replacement, SaveReceipt, SaveRequest, SystemEdit, UserEdit,
};
pub use range::{MotionShape, Range};

#[cfg(test)]
mod persistence_tests;
