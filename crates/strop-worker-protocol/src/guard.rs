//! Worker-side freshness authority (0058 WK02, prepared-authority
//! semantics). Prepared authority belongs to its admitted context: a
//! restarted worker cannot accept an old live handle or write permit,
//! and connection loss never replays a mutation.
//!
//! [`Authority`] is the pure decision — the worker's session loop calls
//! it on every stamped envelope before admission. It owns no handles
//! itself; handle tables key on the session, so a rejected session can
//! never reach one.

use crate::id::{Session, Subscription};
use crate::message::Refusal;

/// The freshness decisions for one live worker session.
#[derive(Debug, Clone)]
pub struct Authority {
    session: Session,
    retiring: bool,
    closed: bool,
}

impl Authority {
    /// The authority established by the handshake: the fresh session
    /// pair this worker process will admit.
    pub fn new(session: Session) -> Self {
        Self {
            session,
            retiring: false,
            closed: false,
        }
    }

    /// The admitted session, for stamping results/events.
    pub fn session(&self) -> Session {
        self.session
    }

    /// Admit any stamped envelope: incarnation and lease must be this
    /// session's. Order matters — a wrong incarnation reports the
    /// current one so a reconnected client can re-handshake; it never
    /// falls through to lease comparison with foreign state.
    pub fn admit(&self, stamped: &Session) -> Result<(), Refusal> {
        if stamped.incarnation != self.session.incarnation {
            return Err(Refusal::WrongIncarnation {
                current: self.session.incarnation,
            });
        }
        if stamped.lease != self.session.lease {
            return Err(Refusal::WrongLease);
        }
        if self.closed {
            return Err(Refusal::Closed);
        }
        Ok(())
    }

    /// Admit a mutation-class request: after `quiesce` the worker drains
    /// admitted work but prepares/applies nothing new.
    pub fn admit_mutation(&self, stamped: &Session) -> Result<(), Refusal> {
        self.admit(stamped)?;
        if self.retiring {
            return Err(Refusal::Retiring);
        }
        Ok(())
    }

    /// Admit a subscription identity against its current generation.
    /// Reinstallation after loss/overflow bumps the generation; old
    /// identities are stale even inside the live session.
    pub fn admit_subscription(
        &self,
        subscription: &Subscription,
        current_generation: u64,
    ) -> Result<(), Refusal> {
        if subscription.generation != current_generation {
            return Err(Refusal::StaleSubscription {
                current: current_generation,
            });
        }
        Ok(())
    }

    /// Begin retirement: new mutations are refused, outcomes still drain.
    pub fn quiesce(&mut self) {
        self.retiring = true;
    }

    /// Close: no further requests admitted at all.
    pub fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::LeaseId;

    fn authority() -> Authority {
        Authority::new(Session {
            incarnation: 7,
            lease: LeaseId(3),
        })
    }

    fn session() -> Session {
        authority().session()
    }

    #[test]
    fn live_session_is_admitted() {
        assert_eq!(authority().admit(&session()), Ok(()));
    }

    #[test]
    fn stale_incarnation_is_rejected() {
        // A restarted worker (incarnation 8) receives a frame stamped by
        // the old session (incarnation 7): typed rejection, current
        // incarnation reported, no handle lookup ever happens.
        let restarted = Authority::new(Session {
            incarnation: 8,
            lease: LeaseId(4),
        });
        assert_eq!(
            restarted.admit(&session()),
            Err(Refusal::WrongIncarnation { current: 8 })
        );
    }

    #[test]
    fn foreign_lease_is_rejected() {
        let stamped = Session {
            lease: LeaseId(99),
            ..session()
        };
        assert_eq!(authority().admit(&stamped), Err(Refusal::WrongLease));
    }

    #[test]
    fn quiesce_refuses_mutations_but_admits_drain() {
        let mut authority = authority();
        authority.quiesce();
        assert_eq!(authority.admit(&session()), Ok(()));
        assert_eq!(authority.admit_mutation(&session()), Err(Refusal::Retiring));
    }

    #[test]
    fn close_refuses_everything() {
        let mut authority = authority();
        authority.close();
        assert_eq!(authority.admit(&session()), Err(Refusal::Closed));
    }

    #[test]
    fn stale_subscription_generation_is_rejected() {
        let subscription = Subscription {
            id: 5,
            generation: 1,
        };
        assert_eq!(authority().admit_subscription(&subscription, 1), Ok(()));
        assert_eq!(
            authority().admit_subscription(&subscription, 2),
            Err(Refusal::StaleSubscription { current: 2 })
        );
    }
}
