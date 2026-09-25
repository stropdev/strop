//! VF19 native mutant seams (plans/0057 §9, verification/mutants.json):
//! compiled only under `--cfg strop_mutant`, never in release artifacts
//! (the `strop_loom` precedent). `STROP_MUTANT` names the armed mutant;
//! anything else — including the cfg set with no selection — is the
//! honest build. Each constant is one registered mutant; the registry
//! checker executes the named kill test against the armed build.

/// `events::forward` treats a full-lane admission refusal as terminal,
/// dropping the event and dying — the UiSessionModel storm defect.
pub const FORWARD_DROP_REFUSED: &str = "forward-drop-refused";

/// Whether `name` is the armed mutant for this process.
pub fn active(name: &str) -> bool {
    std::env::var_os("STROP_MUTANT").is_some_and(|armed| armed == name)
}
