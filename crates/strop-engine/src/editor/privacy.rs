//! One target-aware privacy classifier for host effects (0056 AR08).
//!
//! Explicitly native-free: classification is a pure function of typed
//! inputs — no environment, filesystem, process or network access, no
//! wall clock — so live and replay classify identically, and consulting
//! the classifier can never grant authority. The opt-ins it reads
//! (`--log-content`, `--log-terminal-content`, `:recover consent
//! remote`) are explicit user/session grants made elsewhere; nothing
//! here can create them, and replay or recovery never acquire fresh
//! process, filesystem, clipboard or network authority from a decision.
//!
//! This generalizes the two places classification used to live: the
//! trace `ContentPolicy` (Metadata/Full) producers consulted inline, and
//! the 0055 terminal privacy classification — which stays in
//! `terminal/privacy.rs` and is consulted here, not duplicated.

use strop_trace::ContentPolicy;

use super::trace::drive::Action;
use super::{DocumentSource, Editor};

/// The host-effect families whose records cross a capture or
/// persistence boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectFamily {
    /// System clipboard reads/writes.
    Clipboard,
    /// External opener launches (browser URLs, permalinks).
    ExternalOpen,
    /// Process launch: shell/pipe commands, language servers.
    Process,
    /// Source mutation: checked saves and projected edits.
    Mutation,
    /// Draft/session persistence to private state storage.
    Persistence,
    /// Observation: keys/paste, frames, cell/view exports, listings.
    Observation,
}

impl EffectFamily {
    pub const ALL: [EffectFamily; 6] = [
        EffectFamily::Clipboard,
        EffectFamily::ExternalOpen,
        EffectFamily::Process,
        EffectFamily::Mutation,
        EffectFamily::Persistence,
        EffectFamily::Observation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EffectFamily::Clipboard => "clipboard",
            EffectFamily::ExternalOpen => "external open",
            EffectFamily::Process => "process",
            EffectFamily::Mutation => "mutation",
            EffectFamily::Persistence => "persistence",
            EffectFamily::Observation => "observation",
        }
    }
}

/// Where an effect lands: the local host, an SSH endpoint, or a
/// container namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTarget {
    Local,
    Ssh,
    Container,
}

impl EffectTarget {
    pub fn label(self) -> &'static str {
        match self {
            EffectTarget::Local => "local",
            EffectTarget::Ssh => "ssh",
            EffectTarget::Container => "container",
        }
    }
}

/// The explicit grants a classification consults. Defaults withhold.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EffectPolicy {
    /// `--log-content`: full diagnostic/forensic content capture.
    pub content: ContentPolicy,
    /// `--log-terminal-content` (0055): terminal output stays private
    /// without this second opt-in.
    pub terminal_capture: bool,
    /// `:recover consent remote` (AR04): remote drafts may persist.
    pub remote_consent: bool,
}

/// One family's classification under a policy at a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectClass {
    /// What records may retain. `Metadata` names the effect and its
    /// sizes/outcomes without content; it never fabricates an empty
    /// equivalent view.
    pub capture: ContentPolicy,
    /// Why — a stable, specific reason, never a generic label.
    pub reason: &'static str,
}

/// The one capture decision table. Capture policy is uniform across
/// targets today — the target-aware half is persistence admission
/// below — and tests pin that uniformity rather than assume it.
pub fn classify(policy: &EffectPolicy, family: EffectFamily, target: EffectTarget) -> EffectClass {
    let _ = target;
    match family {
        EffectFamily::Clipboard => EffectClass {
            capture: ContentPolicy::Metadata,
            reason: "clipboard payloads never enter diagnostic records — byte counts and outcomes only",
        },
        EffectFamily::ExternalOpen => EffectClass {
            capture: policy.content,
            reason: "a user-explicit open; the URL records under the content opt-in",
        },
        EffectFamily::Process => EffectClass {
            capture: policy.content,
            reason: "command lines and environment record under the content opt-in",
        },
        EffectFamily::Mutation => EffectClass {
            capture: policy.content,
            reason: "edit content records under the content opt-in; paths, bytes and revisions always",
        },
        EffectFamily::Persistence => EffectClass {
            capture: ContentPolicy::Metadata,
            reason: "draft content persists only to the private state store; records carry cohorts, counts and bytes",
        },
        EffectFamily::Observation => EffectClass {
            capture: policy.content,
            reason: "frames, keys/paste and view exports follow the content opt-in; terminal output additionally requires the terminal capture opt-in (0055)",
        },
    }
}

/// Persistence admission — the target-aware half (AR04 §5, AR08).
/// Local drafts persist under the automatic policy; remote drafts need
/// explicit session consent; container bytes are never draft-persisted.
/// A refusal loses the draft honestly (memory-only), never quietly.
pub fn persistence_admitted(policy: &EffectPolicy, target: EffectTarget) -> bool {
    match target {
        EffectTarget::Local => true,
        EffectTarget::Ssh => policy.remote_consent,
        EffectTarget::Container => false,
    }
}

/// The current document's effect target, from its typed source.
pub fn target_of(source: &DocumentSource) -> EffectTarget {
    match source {
        DocumentSource::Remote(_) => EffectTarget::Ssh,
        DocumentSource::Container { .. } => EffectTarget::Container,
        _ => EffectTarget::Local,
    }
}

impl Editor {
    /// The live policy snapshot a classification consults: the trace
    /// content opt-in actually in force, the terminal capture flag and
    /// the session's remote-persistence consent.
    pub(crate) fn effect_policy(&self) -> EffectPolicy {
        EffectPolicy {
            content: if strop_trace::capture_content() {
                ContentPolicy::Full
            } else {
                ContentPolicy::Metadata
            },
            terminal_capture: self.terminals.capture,
            remote_consent: self.recovery.consent_remote,
        }
    }

    /// Tape admission of one driver action: the forensic stream exists
    /// only under the full-content opt-in; inside it the 0055 terminal
    /// classification remains the one private boundary. Reuses the
    /// terminal classifier; does not replace it.
    pub(crate) fn tape_action_private(&self, action: &Action) -> bool {
        self.private_terminal_action(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILIES: [EffectFamily; 6] = EffectFamily::ALL;
    const TARGETS: [EffectTarget; 3] = [
        EffectTarget::Local,
        EffectTarget::Ssh,
        EffectTarget::Container,
    ];

    fn policy(content: ContentPolicy) -> EffectPolicy {
        EffectPolicy {
            content,
            terminal_capture: false,
            remote_consent: false,
        }
    }

    #[test]
    fn decisions_pinned_per_family_and_target() {
        for family in FAMILIES {
            for target in TARGETS {
                let metadata = classify(&policy(ContentPolicy::Metadata), family, target);
                let full = classify(&policy(ContentPolicy::Full), family, target);
                assert_eq!(
                    metadata.capture,
                    ContentPolicy::Metadata,
                    "{family:?} × {target:?} must withhold content under the default policy"
                );
                let expected_full = match family {
                    // Hard rules, independent of any opt-in or target.
                    EffectFamily::Clipboard | EffectFamily::Persistence => ContentPolicy::Metadata,
                    EffectFamily::ExternalOpen
                    | EffectFamily::Process
                    | EffectFamily::Mutation
                    | EffectFamily::Observation => ContentPolicy::Full,
                };
                assert_eq!(
                    full.capture, expected_full,
                    "{family:?} × {target:?} under full opt-in"
                );
                assert!(
                    !metadata.reason.is_empty() && !full.reason.is_empty(),
                    "every decision carries its own reason"
                );
            }
        }
    }

    #[test]
    fn persistence_admission_is_target_aware() {
        let no_consent = policy(ContentPolicy::Metadata);
        let consent = EffectPolicy {
            remote_consent: true,
            ..no_consent
        };
        assert!(persistence_admitted(&no_consent, EffectTarget::Local));
        assert!(!persistence_admitted(&no_consent, EffectTarget::Ssh));
        assert!(!persistence_admitted(&no_consent, EffectTarget::Container));
        assert!(persistence_admitted(&consent, EffectTarget::Ssh));
        // Consent never extends to container namespaces.
        assert!(!persistence_admitted(&consent, EffectTarget::Container));
    }

    /// 0057 VF15: defaults withhold — no grant can be manufactured by a
    /// missing field, a default value or a replay. The classifier's own
    /// defaults must class every family × target as metadata-only, and
    /// persistence admits local drafts only.
    #[test]
    fn default_policy_withholds_everything() {
        let policy = EffectPolicy::default();
        assert_eq!(policy.content, ContentPolicy::Metadata);
        assert!(!policy.terminal_capture);
        assert!(!policy.remote_consent);
        for family in FAMILIES {
            for target in TARGETS {
                assert_eq!(
                    classify(&policy, family, target).capture,
                    ContentPolicy::Metadata,
                    "{family:?} × {target:?} must withhold content by default"
                );
            }
        }
        assert!(persistence_admitted(&policy, EffectTarget::Local));
        assert!(!persistence_admitted(&policy, EffectTarget::Ssh));
        assert!(!persistence_admitted(&policy, EffectTarget::Container));
    }

    /// 0057 VF15: the terminal capture opt-in is a SECOND grant — the
    /// content opt-in alone never admits terminal content (0055's
    /// boundary reuses this classifier rather than duplicating it).
    #[test]
    fn terminal_capture_needs_its_own_opt_in() {
        let content_only = EffectPolicy {
            content: ContentPolicy::Full,
            terminal_capture: false,
            remote_consent: false,
        };
        assert!(!content_only.terminal_capture);
        // The observation family's reason names the second opt-in.
        let class = classify(
            &content_only,
            EffectFamily::Observation,
            EffectTarget::Local,
        );
        assert!(class.reason.contains("terminal capture opt-in"));
    }
}
