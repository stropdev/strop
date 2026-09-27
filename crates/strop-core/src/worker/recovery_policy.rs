//! Same-source recovery and terminal-outcome kernels (0058 WK18/WK19).
//! The worker's recovered-Store gate and the effect resolver call these
//! exact decisions; namespace/principal observations are premises the
//! kernels trust and never re-derive.

use vstd::prelude::*;

verus! {

/// Recovered verification is observation only: the namespace must be
/// attested (not the "unattested" placeholder) and must still be the
/// worker's own. Only a receipt whose outcome is Unconfirmed can take
/// this path. The caller supplies the observed facts.
pub open spec fn recovery_permits(
    namespace_attested: bool,
    namespace_matches: bool,
    attempt_unconfirmed: bool,
) -> bool {
    namespace_attested && namespace_matches && attempt_unconfirmed
}

/// The gate `serve::fs::verify_recovered` consults before delegating to
/// the durable Store verifier.
pub fn recovery_admitted(
    namespace_attested: bool,
    namespace_matches: bool,
    attempt_unconfirmed: bool,
) -> (admitted: bool)
    ensures admitted == recovery_permits(
        namespace_attested, namespace_matches, attempt_unconfirmed,
    ),
{
    namespace_attested && namespace_matches && attempt_unconfirmed
}

/// A namespace or principal change observed on reconnect is a typed
/// conflict, never a verified recovery.
proof fn namespace_change_never_verifies(
    namespace_attested: bool,
    namespace_matches: bool,
    attempt_unconfirmed: bool,
)
    requires !namespace_attested || !namespace_matches,
    ensures !recovery_permits(
        namespace_attested, namespace_matches, attempt_unconfirmed,
    ),
{
}

/// A confirmed attempt never re-enters the recovered path: verification
/// happens once per uncertainty.
proof fn confirmed_attempt_never_recovers(
    namespace_attested: bool,
    namespace_matches: bool,
    attempt_unconfirmed: bool,
)
    requires !attempt_unconfirmed,
    ensures !recovery_permits(
        namespace_attested, namespace_matches, attempt_unconfirmed,
    ),
{
}

/// Terminal delivery: an observed outcome is delivered once no
/// cancellation cycle is mid-flight. Cancellation requests stop
/// resources; they never replace or drop an observed mutation receipt —
/// the receipt stays queued until the cycle completes.
pub open spec fn delivered(observed: bool, cancelling: bool) -> bool {
    observed && !cancelling
}

/// The emit decision `worker::effect` consults in its resolver.
pub fn ready_to_deliver(observed: bool, cancelling: bool) -> (deliver: bool)
    ensures deliver == delivered(observed, cancelling),
{
    observed && !cancelling
}

/// Cancellation alone never discards an observed outcome: it defers
/// delivery until the cycle finishes.
proof fn cancellation_defers_but_never_drops(
    observed: bool,
    cancelling: bool,
    later_cancelling: bool,
)
    requires observed && cancelling && !later_cancelling,
    ensures delivered(observed, later_cancelling),
{
}

} // verus!
