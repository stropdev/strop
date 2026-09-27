//! strop-core: the buffer. A rope, byte-offset positions, edit ops.
//! No UI, no modes, no grammar — the thing everything else edits.

mod buffer;
pub mod cohortguard;
pub mod diagnostics;
pub mod editmap;
pub mod frontend_input;
pub mod history;
pub mod id;
pub mod languages;
pub mod layout;
pub mod mutguard;
pub mod path_serde;
pub mod process;
pub mod projectguard;
mod range;
pub mod searchguard;
pub mod selection;
pub mod theme;
pub mod viewguard;
pub mod worker;

pub use buffer::{
    Buffer, BufferSeed, Change, ChangeOrigin, EditError, HistoryMove, InputEdit,
    PreparedReplacements, ReadonlyReason, Replacement, SavePlan, SaveReceipt, SaveRequest,
    SystemEdit, UserEdit,
};
pub use range::{MotionShape, Range};

#[cfg(test)]
mod persistence_tests;
