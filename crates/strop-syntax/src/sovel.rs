//! Statically linked Sovel presentation grammar (plan 0070). The parser
//! tables are generated from `grammars/sovel/grammar.js` by the pinned
//! tree-sitter-cli 0.25.10 recipe (`--abi 15`) and committed; `build.rs`
//! compiles them into this crate. Experimental syntax highlighting only —
//! the tree is not proof of source validity.
use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_sovel() -> *const ();
}

/// The Sovel language, exposed through the same `LanguageFn` bridge the
/// published grammar crates use.
///
// SAFETY: `build.rs` compiles the committed `grammars/sovel/src/parser.c`
// (ABI 15) into this very crate, so `tree_sitter_sovel` always resolves to
// the generated `TSLanguage` static matching the linked tree-sitter 0.25
// runtime — no dlopen, no version skew is possible. The returned pointer
// borrows a `'static` parser table that is never mutated, which safe Rust
// cannot express across the C ABI; this is the whole FFI boundary.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_sovel) };
