//! strop ring-0 kernel (plans/0086): pure decisions over plain values.
//!
//! Everything here is plain safe Rust shaped for the Charon/Aeneas
//! translation: no IO, no floats, no `dyn`, no dependencies, `no_std` plus
//! `alloc`, no closures or iterator adaptors in proved code. The `proofs`
//! compose gate translates this crate to Lean 4 (`proofs/Extracted/`,
//! committed and drift-checked) and proves the theorems in `proofs/Kernel/`
//! over that translation. Proofs are about the shipped code: there is no
//! hand-written Lean copy of these functions.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod editmap;
pub mod frame;
pub mod generation;
