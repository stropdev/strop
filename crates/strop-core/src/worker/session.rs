//! Worker session admission (0058 WK18). The protocol authority
//! calls this exact decision before consulting handle tables. The handshake's
//! session facts are a caller premise; this kernel does not verify the codec.

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

pub fn classify_session(
    current_incarnation: u64,
    incoming_incarnation: u64,
    current_lease: u64,
    incoming_lease: u64,
    closed: bool,
    retiring: bool,
    mutation: bool,
) -> Admission {
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
