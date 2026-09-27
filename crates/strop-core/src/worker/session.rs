//! Same-source worker session admission (0058 WK18). The protocol authority
//! calls this exact decision before consulting handle tables. The handshake's
//! session facts are a caller premise; this kernel does not verify the codec.

use vstd::prelude::*;

verus! {

/// First mismatch wins; retirement refuses new mutations but allows
/// admitted outcomes to drain until the session closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Live,
    WrongIncarnation,
    WrongLease,
    Closed,
    Retiring,
}

pub open spec fn session_permits(
    current_incarnation: u64,
    incoming_incarnation: u64,
    current_lease: u64,
    incoming_lease: u64,
    closed: bool,
    retiring: bool,
    mutation: bool,
) -> bool {
    current_incarnation == incoming_incarnation
        && current_lease == incoming_lease
        && !closed && !(retiring && mutation)
}

pub fn classify_session(
    current_incarnation: u64,
    incoming_incarnation: u64,
    current_lease: u64,
    incoming_lease: u64,
    closed: bool,
    retiring: bool,
    mutation: bool,
) -> (decision: Admission)
    ensures
        (decision == Admission::Live) == session_permits(
            current_incarnation, incoming_incarnation, current_lease,
            incoming_lease, closed, retiring, mutation,
        ),
        (decision == Admission::WrongIncarnation) ==
            (current_incarnation != incoming_incarnation),
        (decision == Admission::WrongLease) ==
            (current_incarnation == incoming_incarnation
            && current_lease != incoming_lease),
        (decision == Admission::Closed) ==
            (current_incarnation == incoming_incarnation
            && current_lease == incoming_lease && closed),
        (decision == Admission::Retiring) ==
            (current_incarnation == incoming_incarnation
            && current_lease == incoming_lease && !closed && retiring && mutation),
{
    if current_incarnation != incoming_incarnation {
        Admission::WrongIncarnation
    } else if current_lease != incoming_lease {
        Admission::WrongLease
    } else if closed {
        Admission::Closed
    } else if retiring && mutation {
        Admission::Retiring
    } else {
        Admission::Live
    }
}

/// A restarted worker cannot admit an old frame even if its numeric
/// lease happens to be reused.
proof fn stale_incarnation_never_admitted(
    current_incarnation: u64,
    incoming_incarnation: u64,
    current_lease: u64,
    incoming_lease: u64,
    closed: bool,
    retiring: bool,
    mutation: bool,
)
    requires current_incarnation != incoming_incarnation,
    ensures !session_permits(
        current_incarnation, incoming_incarnation, current_lease, incoming_lease,
        closed, retiring, mutation,
    ),
{
}

}
