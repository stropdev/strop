//! Frontend command metadata shared by the TUI, native GUI and automation.
//! Dispatch ownership stays in the engine; this data carries only stable
//! identities, user-facing rows and whether the row is dispatchable today.
//!
//! The engine consumes this table as its single keymap source. Native
//! frontends read the same rows for parity/help surfaces; neither builds a
//! second command table from rendering code.

/// The one-char absorber flavors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbsorbKind {
    Replace,
    MarkSet,
    MarkJump,
    Find,
    /// q<a> — toggle macro recording into register a.
    MacroRecord,
    /// @<a> — replay register a's macro.
    MacroPlay,
}

/// Dispatch-independent command category. Engine-specific leaf handlers
/// remain in the engine's adapter over this data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// A direct command or semantic action.
    Leaf,
    /// Alias: expands to these keys through the engine walker.
    Alias,
    /// A grammar motion.
    Motion,
    /// A grammar operator.
    Operator,
    /// An object prefix.
    ObjectPrefix,
    /// A prefix: children follow; which-key renders them.
    Prefix,
    /// Free-text line: a modal text field, not a key sequence.
    TextLine,
    /// One-char absorbers.
    AbsorbChar(AbsorbKind),
    /// Register selection.
    AbsorbRegister,
    /// Implemented by a modal field or Ex host, not the document trie.
    Contextual,
    /// Planned slot: no dispatch yet.
    Soon,
}

/// One binding row: stable identity, notation, description, sections,
/// support state and dispatch category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub keys: &'static str,
    pub desc: &'static str,
    pub sections: &'static [&'static str],
    pub live: bool,
    pub id: &'static str,
    pub kind: CommandKind,
}

pub const SECTIONS: &[&str] = &[
    "normal",
    "visual",
    "insert",
    "leader",
    "git",
    "ex+panes",
    "terminal",
];

include!("commands/bindings.rs");
