//! Recovery and terminal-outcome decisions (0058 WK18/WK19).
//! The worker's recovered-Store gate and the effect resolver call these
//! exact decisions; namespace/principal observations are premises the
//! kernels trust and never re-derive.

/// The gate `serve::fs::verify_recovered` consults before delegating to
/// the durable Store verifier.
pub fn recovery_admitted(
    namespace_attested: bool,
    namespace_matches: bool,
    attempt_unconfirmed: bool,
) -> bool {
    namespace_attested && namespace_matches && attempt_unconfirmed
}

/// The emit decision `worker::effect` consults in its resolver.
pub fn ready_to_deliver(observed: bool, cancelling: bool) -> bool {
    observed && !cancelling
}
