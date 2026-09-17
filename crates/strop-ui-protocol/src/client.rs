//! The pure client state machine (0056 AR10): a readonly cache of the
//! backend's published view plus the resynchronization rules. No IO, no
//! process, no clock — the stdio transport lives in [`crate::driver`].
//!
//! Rules (AR09 §8):
//! - A snapshot is valid against any prior state; it clears poisoning.
//! - A delta is valid only against its `base`; anything else — a dropped
//!   or reordered delta, a changed pane set, a foreign incarnation —
//!   poisons the client until an explicit resync.
//! - A poisoned client stops accepting actions. It retains its last
//!   known view (known outcomes are never fabricated away) and recovers
//!   only through a complete current snapshot.

use crate::message::{
    BackendInfo, EffectRequest, ServerMessage, ShutdownReason, ViewDelta, ViewSnapshot,
};
use crate::BaseStamp;

/// Why the client needs a complete current snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncReason {
    /// A delta's base is not the applied generation (a publication was
    /// dropped or reordered on the wire).
    DeltaBaseMismatch { expected: u64, found: u64 },
    /// A delta arrived from another backend incarnation.
    IncarnationChanged { current: u64 },
    /// The link closed; no further publications will arrive.
    LinkLost,
}

/// What one applied server message changed.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    /// The view moved to this generation (snapshot or delta).
    View { generation: u64 },
    /// The client is poisoned; resync before acting again.
    ResyncRequired { reason: ResyncReason },
    /// The backend requests a host effect.
    Effect { id: u64, effect: EffectRequest },
    /// The backend is exiting.
    Closed { reason: ShutdownReason },
}

/// Client-side misuse that no resync can fix.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("the client is poisoned; resync before acting")]
    Poisoned,
    #[error("no view has been published yet")]
    NoView,
}

/// The readonly view cache and its resynchronization state.
#[derive(Debug)]
pub struct Client {
    incarnation: u64,
    view: Option<ViewSnapshot>,
    poisoned: Option<ResyncReason>,
}

impl Client {
    /// Construct from the handshake's backend identity.
    pub fn new(backend: &BackendInfo) -> Self {
        Self {
            incarnation: backend.incarnation,
            view: None,
            poisoned: None,
        }
    }

    /// The applied view generation (0 before the first publication).
    pub fn generation(&self) -> u64 {
        self.view.as_ref().map_or(0, |view| view.generation)
    }

    /// The cached view, if one has been published.
    pub fn view(&self) -> Option<&ViewSnapshot> {
        self.view.as_ref()
    }

    /// Why the client is poisoned, if it is.
    pub fn poisoned(&self) -> Option<ResyncReason> {
        self.poisoned
    }

    /// The base stamp for the next action. Poisoned clients and clients
    /// that never saw a publication may not act (AR09: no optimistic
    /// replay of uncertain state).
    pub fn base_stamp(&self) -> Result<BaseStamp, ClientError> {
        if self.poisoned.is_some() {
            return Err(ClientError::Poisoned);
        }
        let view = self.view.as_ref().ok_or(ClientError::NoView)?;
        Ok(BaseStamp {
            incarnation: self.incarnation,
            generation: view.generation,
        })
    }

    /// Apply one server message to the cache. Control messages (`ack`,
    /// `error`) carry no view state and are the transport's business;
    /// they return no events here.
    pub fn apply(&mut self, message: &ServerMessage) -> Vec<ClientEvent> {
        match message {
            ServerMessage::Snapshot { incarnation, view } => {
                let mut events = Vec::new();
                if *incarnation != self.incarnation {
                    // The backend restarted: its snapshot is the truth,
                    // but the identity change is observable.
                    events.push(ClientEvent::ResyncRequired {
                        reason: ResyncReason::IncarnationChanged {
                            current: *incarnation,
                        },
                    });
                    self.incarnation = *incarnation;
                }
                self.poisoned = None;
                let generation = view.generation;
                self.view = Some(view.clone());
                events.push(ClientEvent::View { generation });
                events
            }
            ServerMessage::Delta { incarnation, delta } => self.apply_delta(*incarnation, delta),
            ServerMessage::Effect { id, effect } => vec![ClientEvent::Effect {
                id: *id,
                effect: effect.clone(),
            }],
            ServerMessage::Bye { reason } => {
                self.poisoned = Some(ResyncReason::LinkLost);
                vec![ClientEvent::Closed { reason: *reason }]
            }
            ServerMessage::Welcome { backend, .. } => {
                if backend.incarnation != self.incarnation {
                    self.incarnation = backend.incarnation;
                    self.view = None;
                    self.poisoned = None;
                    vec![ClientEvent::ResyncRequired {
                        reason: ResyncReason::IncarnationChanged {
                            current: backend.incarnation,
                        },
                    }]
                } else {
                    Vec::new()
                }
            }
            ServerMessage::Ack { .. } | ServerMessage::Error { .. } => Vec::new(),
        }
    }

    /// Mark the link lost (transport EOF/error). The last known view is
    /// retained; actions stop until an explicit recovery.
    pub fn link_lost(&mut self) {
        self.poisoned = Some(ResyncReason::LinkLost);
    }

    fn apply_delta(&mut self, incarnation: u64, delta: &ViewDelta) -> Vec<ClientEvent> {
        let poison = |client: &mut Self, reason| {
            client.poisoned = Some(reason);
            vec![ClientEvent::ResyncRequired { reason }]
        };
        if incarnation != self.incarnation {
            return poison(
                self,
                ResyncReason::IncarnationChanged {
                    current: incarnation,
                },
            );
        }
        if self.poisoned.is_some() {
            // Already poisoned: only a snapshot recovers.
            return Vec::new();
        }
        // Validate against immutable facts first; the mutable fold runs
        // only after every poison check has passed.
        let Some((generation, pane_count)) = self
            .view
            .as_ref()
            .map(|view| (view.generation, view.panes.len()))
        else {
            return poison(
                self,
                ResyncReason::DeltaBaseMismatch {
                    expected: 0,
                    found: delta.base,
                },
            );
        };
        if delta.base != generation || delta.panes.len() != pane_count {
            // A delta applies only onto its exact base, aligned with the
            // base's pane set.
            return poison(
                self,
                ResyncReason::DeltaBaseMismatch {
                    expected: generation,
                    found: delta.base,
                },
            );
        }
        let Some(view) = self.view.as_mut() else {
            unreachable!("the view was present for validation above");
        };
        for (pane, change) in view.panes.iter_mut().zip(&delta.panes) {
            if let crate::message::PaneDelta::Changed(changed) = change {
                *pane = changed.clone();
            }
        }
        if let Some(geometry) = delta.geometry {
            view.geometry = geometry;
        }
        if let Some(active_pane) = delta.active_pane {
            view.active_pane = active_pane;
        }
        if let Some(state) = &delta.state {
            view.state = state.clone();
        }
        view.generation = delta.generation;
        vec![ClientEvent::View {
            generation: delta.generation,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Geometry, PaneDelta, PaneSnapshot, ViewBounds};

    fn backend(incarnation: u64) -> BackendInfo {
        BackendInfo {
            name: "strop".into(),
            version: "0.0.0".into(),
            build: None,
            incarnation,
        }
    }

    fn pane(lines: &[&str]) -> PaneSnapshot {
        PaneSnapshot {
            document: serde_json::from_value(serde_json::json!({"slot":0,"generation":0})).unwrap(),
            revision: strop_core::id::BufferRevision::new(0),
            bounds: ViewBounds::Complete,
            cursor: 0,
            view_top: 0,
            hscroll: 0,
            terminal_input: false,
            overlays: false,
            rect: Default::default(),
            budget: Default::default(),
            window_top: 0,
            lines: lines.iter().map(|line| line.to_string()).collect(),
        }
    }

    fn snapshot(generation: u64, lines: &[&str]) -> ViewSnapshot {
        ViewSnapshot {
            generation,
            geometry: Geometry {
                columns: 80,
                rows: 24,
            },
            active_pane: 0,
            panes: vec![pane(lines)],
            state: serde_json::json!({"mode":"NORMAL"}),
        }
    }

    fn delta(base: u64, generation: u64, lines: &[&str]) -> ViewDelta {
        ViewDelta {
            base,
            generation,
            geometry: None,
            active_pane: None,
            panes: vec![PaneDelta::Changed(pane(lines))],
            state: None,
        }
    }

    #[test]
    fn snapshot_then_deltas_apply_in_order() {
        let mut client = Client::new(&backend(1));
        client.apply(&ServerMessage::Snapshot {
            incarnation: 1,
            view: snapshot(1, &["a"]),
        });
        assert_eq!(client.generation(), 1);
        client.apply(&ServerMessage::Delta {
            incarnation: 1,
            delta: delta(1, 2, &["ab"]),
        });
        assert_eq!(client.generation(), 2);
        assert_eq!(client.view().unwrap().panes[0].lines, ["ab"]);
        assert!(client.poisoned().is_none());
        assert_eq!(
            client.base_stamp().unwrap(),
            BaseStamp {
                incarnation: 1,
                generation: 2
            }
        );
    }

    #[test]
    fn a_dropped_delta_poisons_until_resync() {
        let mut client = Client::new(&backend(1));
        client.apply(&ServerMessage::Snapshot {
            incarnation: 1,
            view: snapshot(1, &["a"]),
        });
        // The 1→2 delta is dropped; 2→3 arrives against an unknown base.
        let events = client.apply(&ServerMessage::Delta {
            incarnation: 1,
            delta: delta(2, 3, &["abc"]),
        });
        assert_eq!(
            events,
            vec![ClientEvent::ResyncRequired {
                reason: ResyncReason::DeltaBaseMismatch {
                    expected: 1,
                    found: 2
                }
            }]
        );
        assert_eq!(client.generation(), 1, "never applied onto a wrong base");
        assert_eq!(
            client.base_stamp().unwrap_err(),
            ClientError::Poisoned,
            "actions stop after the drop"
        );
        // Further deltas cannot unpoison; only a snapshot can.
        client.apply(&ServerMessage::Delta {
            incarnation: 1,
            delta: delta(3, 4, &["abcd"]),
        });
        assert!(client.poisoned().is_some());
        client.apply(&ServerMessage::Snapshot {
            incarnation: 1,
            view: snapshot(4, &["abcd"]),
        });
        assert!(client.poisoned().is_none());
        assert_eq!(client.view().unwrap().panes[0].lines, ["abcd"]);
    }

    #[test]
    fn a_foreign_incarnation_poisons_deltas() {
        let mut client = Client::new(&backend(1));
        client.apply(&ServerMessage::Snapshot {
            incarnation: 1,
            view: snapshot(1, &["a"]),
        });
        let events = client.apply(&ServerMessage::Delta {
            incarnation: 2,
            delta: delta(1, 2, &["ab"]),
        });
        assert_eq!(
            events,
            vec![ClientEvent::ResyncRequired {
                reason: ResyncReason::IncarnationChanged { current: 2 }
            }]
        );
        assert_eq!(client.generation(), 1);
    }

    #[test]
    fn a_pane_set_mismatch_poisons() {
        let mut client = Client::new(&backend(1));
        client.apply(&ServerMessage::Snapshot {
            incarnation: 1,
            view: snapshot(1, &["a"]),
        });
        let mut two_panes = delta(1, 2, &["ab"]);
        two_panes.panes.push(PaneDelta::Unchanged);
        client.apply(&ServerMessage::Delta {
            incarnation: 1,
            delta: two_panes,
        });
        assert!(client.poisoned().is_some());
        assert_eq!(client.generation(), 1);
    }

    #[test]
    fn bye_stops_actions_but_retains_the_known_view() {
        let mut client = Client::new(&backend(1));
        client.apply(&ServerMessage::Snapshot {
            incarnation: 1,
            view: snapshot(1, &["a"]),
        });
        client.apply(&ServerMessage::Bye {
            reason: ShutdownReason::Requested,
        });
        assert_eq!(client.poisoned(), Some(ResyncReason::LinkLost));
        assert_eq!(
            client.view().unwrap().panes[0].lines,
            ["a"],
            "known outcomes are retained"
        );
        assert_eq!(client.base_stamp().unwrap_err(), ClientError::Poisoned);
    }

    /// Property (seeded, deterministic): over random publication
    /// sequences with random drops, the client applies a delta only onto
    /// its exact base, poisons on the first gap, stays poisoned through
    /// further deltas, and a snapshot always recovers to the published
    /// truth.
    #[test]
    fn random_publication_sequences_never_diverge() {
        // Knuth LCG, the repo's deterministic-generator precedent.
        struct Lcg(u64);
        impl Lcg {
            fn next(&mut self) -> u64 {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                self.0 >> 33
            }
            fn below(&mut self, n: u64) -> u64 {
                self.next() % n
            }
        }

        for seed in 0..64u64 {
            let mut rand = Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
            let mut client = Client::new(&backend(7));
            // The reference model: the truth the backend published.
            let mut truth: Option<(u64, Vec<String>)> = None;
            let mut poisoned_reference = true; // no view yet
            for step in 0..40u64 {
                let generation = step / 2 + 1;
                let text = format!("s{seed}g{generation}");
                let message = if step % 2 == 0 || truth.is_none() {
                    truth = Some((generation, vec![text.clone()]));
                    poisoned_reference = false;
                    ServerMessage::Snapshot {
                        incarnation: 7,
                        view: snapshot(generation, &[&text]),
                    }
                } else {
                    let (base, _) = truth.clone().unwrap();
                    truth = Some((generation, vec![text.clone()]));
                    // Randomly drop the publication (client never sees it)
                    // or deliver a delta against the published base.
                    if rand.below(3) == 0 {
                        poisoned_reference = true;
                        continue; // dropped on the wire
                    }
                    ServerMessage::Delta {
                        incarnation: 7,
                        delta: delta(base, generation, &[&text]),
                    }
                };
                client.apply(&message);
                match (&client.view().cloned(), &truth) {
                    (Some(view), Some((_, lines))) if !poisoned_reference => {
                        assert_eq!(&view.panes[0].lines, lines, "seed {seed} step {step}");
                        assert!(client.poisoned().is_none());
                    }
                    _ => {
                        if poisoned_reference {
                            assert!(
                                client.poisoned().is_some() || client.view().is_none(),
                                "seed {seed} step {step}: a gap must poison"
                            );
                        }
                    }
                }
                // Invariant at every step: the cached generation is always
                // one the backend actually published, reached by an intact
                // chain from a snapshot.
                if let Some(view) = client.view() {
                    assert!(view.generation <= generation);
                    if client.poisoned().is_none() {
                        assert_eq!(Some(view.generation), truth.as_ref().map(|t| t.0));
                    }
                }
            }
        }
    }
}
