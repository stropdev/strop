//! Passive Dev Container discovery, typed JSONC, redacted trust review and an
//! explicitly invoked reference-CLI lifecycle adapter.
//!
//! `discover`, `RawConfiguration::parse_jsonc` and `TrustReview::new` are pure
//! with respect to Docker, processes, hooks and the network. The optional CLI
//! is probed only by `ReferenceCli::probe*`; `read_configuration` is itself an
//! explicit CLI/engine operation (the upstream implementation performs a
//! container lookup), and merged metadata additionally requires explicit
//! resolution consent. Attach, start and rebuild are separate owned jobs.

mod cli;
mod config;
mod discovery;
mod error;
mod job;
mod jsonc;
mod model;
mod process;
mod review;

pub use cli::{locate_cli, CliVersion, ReferenceCli, SUPPORTED_VERSIONS};
pub use config::{
    ConfigIssue, ConfigIssueKind, ConfigurationKind, LifecycleCommand, LifecycleStage, MountObject,
    MountSpec, PortSpec, RawBuild, RawConfiguration, SensitiveString, WaitFor,
};
pub use discovery::{
    discover, discover_with_limit, ConfigIdentity, ContentDigest, DiscoveredConfiguration,
    DiscoveryIssue, DiscoveryPrecedence, DiscoveryReport,
};
pub use error::DevcontainerError;
pub use job::{
    CliOperation, JobControl, JobId, JobLog, JobOutcome, JobSnapshot, JobState, OwnedJob,
};
pub use jsonc::DEFAULT_INPUT_LIMIT;
pub use model::{
    AttachRequest, CliConfiguration, CliEngine, ConfigSelection, EngineIdentity,
    EnvironmentContext, LockfilePolicy, MetadataResolutionConsent, OwnershipCoverage,
    OwnershipReceipt, PartialResource, Readiness, RebuildRequest, Reconciliation, StartRequest,
    UserMapping, WorkspaceMapping,
};
pub use process::{
    CliProcess, LogStream, ProcessFailure, ProcessLog, ProcessOutput, ProcessRequest,
    SupervisedCliProcess,
};
pub use review::{
    CredentialKind, EffectDomain, EffectKind, EffectSeverity, TrustEffect, TrustReview,
};

#[cfg(test)]
mod tests;
