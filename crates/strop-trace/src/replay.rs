//! The forensic tape: one ordered record stream that makes a full-content
//! trace replayable. The tape is owned by the UI thread and never cloned
//! into workers; the diagnostic logger stays the only disk writer.
//!
//! Record kinds:
//! - `Seed` — the pure startup state a replay is reconstructed from.
//! - `Action` — one external input, service delivery, frame or shutdown
//!   step, stamped with a logical clock, emitted immediately before the
//!   reducer/render work runs.
//! - `Request` — admission of an asynchronous native launch. The owner
//!   (ticket, stamps, loading state) is installed BEFORE the tape sees
//!   the request; the tape only decides whether the native launch runs.
//! - `Call` — a synchronous outside observation, including failures.
//! - `Check` — a state observation both modes must reproduce exactly.
//! - `End` — the deliberate end of the recording.
//!
//! Replay consumes `Request`/`Call`/`Check` synchronously inside the same
//! production code that produced them, so divergence is detected at the
//! exact producer, never patched over. Any failure is sticky: the tape
//! refuses all further work until `healthy()` clears it (it never does).
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    Seed {
        value: Value,
    },
    Action {
        tick: Tick,
        value: Value,
    },
    Request {
        operation: String,
        arguments: Value,
    },
    Call {
        operation: String,
        arguments: Value,
        result: Value,
    },
    Check {
        value: Value,
    },
    End,
}

/// The logical clock actions carry. Monotonic milliseconds order events;
/// unix seconds feed age computations so replay needs no wall clock.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tick {
    pub monotonic_ms: u64,
    pub unix_seconds: i64,
}

enum Mode {
    Live,
    Replay(VecDeque<Node>),
}

pub struct Tape {
    mode: RefCell<Mode>,
    fault: Cell<Option<&'static str>>,
    tick: Cell<Tick>,
    finished: Cell<bool>,
    started: std::time::Instant,
    #[cfg(feature = "test-support")]
    fixture: Option<Fixture>,
}

#[cfg(feature = "test-support")]
type FixtureResponder = dyn Fn(&str, &Value) -> io::Result<Value>;

#[cfg(feature = "test-support")]
struct Fixture {
    nodes: RefCell<Vec<Node>>,
    respond: Box<FixtureResponder>,
}

impl Default for Tape {
    fn default() -> Self {
        Self::live()
    }
}

impl Tape {
    /// A live tape: native launches run, and every record is emitted to the
    /// diagnostic sink while full-content capture is on.
    pub fn live() -> Self {
        Self {
            mode: RefCell::new(Mode::Live),
            fault: Cell::new(None),
            tick: Cell::new(Tick::default()),
            finished: Cell::new(false),
            started: std::time::Instant::now(),
            #[cfg(feature = "test-support")]
            fixture: None,
        }
    }

    /// Alias of [`Tape::live`] matching the plain-constructor call sites
    /// Main wires into `Editor::new_in`.
    pub fn new() -> Self {
        Self::live()
    }

    /// A replay tape over recorded nodes. It never consults the host.
    pub fn replay(nodes: Vec<Node>) -> Self {
        Self {
            mode: RefCell::new(Mode::Replay(nodes.into())),
            ..Self::live()
        }
    }

    /// A hermetic fixture recorder (tests only): `request` suppresses native
    /// launches and `call` is answered by `respond`, so recorded tapes are
    /// built without any process, filesystem or network access.
    #[cfg(feature = "test-support")]
    pub fn fixture(respond: impl Fn(&str, &Value) -> io::Result<Value> + 'static) -> Self {
        Self {
            fixture: Some(Fixture {
                nodes: RefCell::new(Vec::new()),
                respond: Box::new(respond),
            }),
            ..Self::live()
        }
    }

    /// The nodes a fixture recorded, for `Tape::replay` round-trips.
    #[cfg(feature = "test-support")]
    pub fn fixture_nodes(&self) -> Vec<Node> {
        self.fixture
            .as_ref()
            .expect("fixture recorder")
            .nodes
            .borrow()
            .clone()
    }

    #[cfg(feature = "test-support")]
    fn has_fixture(&self) -> bool {
        self.fixture.is_some()
    }

    #[cfg(not(feature = "test-support"))]
    fn has_fixture(&self) -> bool {
        false
    }

    pub fn is_replay(&self) -> bool {
        matches!(&*self.mode.borrow(), Mode::Replay(_))
    }

    /// Expensive state construction is lazy when neither recording nor replaying.
    pub fn observes(&self) -> bool {
        self.is_replay() || self.captures()
    }

    /// Only the live adapter samples the host clock. Fixtures and replay use
    /// their explicit tick; reducers consume now() and never consult wall time.
    pub fn sample_tick(&self) -> Tick {
        if self.is_replay() || self.has_fixture() {
            return self.now();
        }
        Tick {
            monotonic_ms: u64::try_from(self.started.elapsed().as_millis())
                .unwrap_or(u64::MAX)
                .max(self.now().monotonic_ms),
            unix_seconds: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| {
                    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
                }),
        }
    }

    pub fn now(&self) -> Tick {
        self.tick.get()
    }

    pub fn set_tick(&self, tick: Tick) -> io::Result<()> {
        if tick.monotonic_ms < self.tick.get().monotonic_ms {
            return self.fail("clock moved backwards");
        }
        self.tick.set(tick);
        Ok(())
    }

    fn fail<T>(&self, message: &'static str) -> io::Result<T> {
        if self.fault.get().is_none() {
            self.fault.set(Some(message));
        }
        Err(io::Error::other(message))
    }

    /// The first failure stays put until the tape is dropped.
    pub fn healthy(&self) -> io::Result<()> {
        match self.fault.get() {
            Some(message) => Err(io::Error::other(message)),
            None => Ok(()),
        }
    }

    fn pop(&self) -> io::Result<Node> {
        self.healthy()?;
        let node = match &mut *self.mode.borrow_mut() {
            Mode::Replay(nodes) => nodes.pop_front(),
            Mode::Live => None,
        };
        node.ok_or_else(|| {
            if self.fault.get().is_none() {
                self.fault.set(Some("unexpected end of replay"));
            }
            io::Error::other("unexpected end of replay")
        })
    }

    fn captures(&self) -> bool {
        if self.has_fixture() {
            return true;
        }
        crate::capture_content()
    }

    fn emit(&self, node: &Node) {
        #[cfg(feature = "test-support")]
        if let Some(fixture) = &self.fixture {
            fixture.nodes.borrow_mut().push(node.clone());
            return;
        }
        if crate::capture_content() {
            crate::record(crate::EventKind::Replay, node);
        }
    }

    /// Serialize through a bounded encoder sized to the assembled-value
    /// bound: admission chunks whatever exceeds one record, and only a
    /// value beyond a whole capture still degrades the session.
    fn value<T: Serialize>(&self, value: &T) -> io::Result<Value> {
        let mut bytes = crate::bounded::Bytes::new(crate::chunk::MAX_VALUE_BYTES);
        if serde_json::to_writer(&mut bytes, value).is_err() {
            if self.is_replay() || self.has_fixture() {
                return self.fail("replay value exceeds capture bound");
            }
            // Live capture degrades honestly: the forensic stream is marked
            // incomplete and the session continues as a plain diagnostic log.
            crate::mark_incomplete("forensic value exceeds cap or cannot serialize");
            return Ok(Value::Null);
        }
        match serde_json::from_slice(&bytes.into_vec()) {
            Ok(value) => Ok(value),
            Err(_) => self.fail("encoded replay value cannot decode"),
        }
    }

    fn decode<T: DeserializeOwned>(&self, value: Value) -> io::Result<T> {
        serde_json::from_value(value).map_err(|_| {
            if self.fault.get().is_none() {
                self.fault.set(Some("replay payload does not decode"));
            }
            io::Error::other("replay payload does not decode")
        })
    }

    /// Record the pure startup state. Live only, exactly once, before any
    /// `Action`.
    pub fn seed<S: Serialize>(&self, seed: &S) -> io::Result<()> {
        if self.is_replay() {
            return self.fail("live seed entered replay");
        }
        if self.finished.get() {
            return self.fail("recording finished");
        }
        if self.captures() {
            self.emit(&Node::Seed {
                value: self.value(seed)?,
            });
        }
        Ok(())
    }

    /// Replay-side seed consumption; must be the first record.
    pub fn take_seed<S: DeserializeOwned>(&self) -> io::Result<S> {
        match self.pop()? {
            Node::Seed { value } => self.decode(value),
            _ => self.fail("replay must start with seed"),
        }
    }

    /// Live drivers call this immediately BEFORE reducer or render work.
    pub fn action<A: Serialize>(&self, tick: Tick, action: &A) -> io::Result<()> {
        if self.is_replay() {
            return self.fail("live action entered replay");
        }
        if self.finished.get() {
            return self.fail("recording finished");
        }
        self.set_tick(tick)?;
        if self.captures() {
            self.emit(&Node::Action {
                tick,
                value: self.value(action)?,
            });
        }
        Ok(())
    }

    /// Replay driver: the next external action, or `None` at a clean `End`.
    /// Any unconsumed observation left in place is a hard failure.
    pub fn next<A: DeserializeOwned>(&self) -> io::Result<Option<A>> {
        match self.pop()? {
            Node::Action { tick, value } => {
                self.set_tick(tick)?;
                self.decode(value).map(Some)
            }
            Node::End => {
                let empty = matches!(&*self.mode.borrow(), Mode::Replay(nodes) if nodes.is_empty());
                if !empty {
                    return self.fail("records after replay end");
                }
                Ok(None)
            }
            _ => self.fail("unconsumed replay observation"),
        }
    }

    /// Asynchronous native launch admission. The identical request owner
    /// must ALREADY be installed; `true` grants the launch (live modes),
    /// `false` means a recorded request matched and the native side is
    /// suppressed. Replay never falls back to the host on mismatch.
    pub fn request<A: Serialize>(&self, operation: &str, arguments: &A) -> io::Result<bool> {
        self.healthy()?;
        if self.finished.get() && !self.is_replay() {
            return self.fail("recording finished");
        }
        if !self.is_replay() && !self.captures() {
            return Ok(true);
        }
        let arguments = self.value(arguments)?;
        #[cfg(feature = "test-support")]
        if self.fixture.is_some() {
            self.emit(&Node::Request {
                operation: operation.into(),
                arguments,
            });
            return Ok(false);
        }
        if self.is_replay() {
            match self.pop()? {
                Node::Request {
                    operation: expected,
                    arguments: wanted,
                } if expected == operation && wanted == arguments => Ok(false),
                _ => self.fail("request identity or arguments diverged"),
            }
        } else {
            self.emit(&Node::Request {
                operation: operation.into(),
                arguments,
            });
            Ok(true)
        }
    }

    /// Synchronous outside observation. The closure must contain the
    /// ENTIRE native access including preflight (config/path probes);
    /// replay returns the recorded result, including `Err` shapes, and
    /// never invokes it.
    pub fn call<A: Serialize, R: Serialize + DeserializeOwned>(
        &self,
        operation: &str,
        arguments: &A,
        native: impl FnOnce() -> R,
    ) -> io::Result<R> {
        self.healthy()?;
        if self.finished.get() && !self.is_replay() {
            return self.fail("recording finished");
        }
        if !self.is_replay() && !self.captures() {
            return Ok(native());
        }
        let arguments = self.value(arguments)?;
        #[cfg(feature = "test-support")]
        if let Some(fixture) = &self.fixture {
            let result = match (fixture.respond)(operation, &arguments) {
                Ok(result) => result,
                Err(error) => {
                    if self.fault.get().is_none() {
                        self.fault.set(Some("fixture observation missing"));
                    }
                    return Err(error);
                }
            };
            self.emit(&Node::Call {
                operation: operation.into(),
                arguments,
                result: result.clone(),
            });
            return self.decode(result);
        }
        if self.is_replay() {
            match self.pop()? {
                Node::Call {
                    operation: expected,
                    arguments: wanted,
                    result,
                } if expected == operation && wanted == arguments => self.decode(result),
                _ => self.fail("synchronous service call diverged"),
            }
        } else {
            let result = native();
            self.emit(&Node::Call {
                operation: operation.into(),
                arguments,
                result: self.value(&result)?,
            });
            Ok(result)
        }
    }

    /// Both modes must produce this observation bit-for-bit.
    pub fn check<C: Serialize>(&self, state: &C) -> io::Result<()> {
        self.healthy()?;
        if self.finished.get() && !self.is_replay() {
            return self.fail("recording finished");
        }
        if !self.is_replay() && !self.captures() {
            return Ok(());
        }
        let value = self.value(state)?;
        if self.is_replay() {
            match self.pop()? {
                Node::Check { value: expected } if value == expected => Ok(()),
                _ => self.fail("editor state diverged"),
            }
        } else {
            self.emit(&Node::Check { value });
            Ok(())
        }
    }

    /// Deliberate recording end. Live only; a replayed tape ends itself.
    pub fn finish(&self) -> io::Result<()> {
        self.healthy()?;
        if self.is_replay() {
            return self.fail("live finish entered replay");
        }
        if self.finished.get() {
            return self.fail("recording finished");
        }
        self.finished.set(true);
        self.emit(&Node::End);
        Ok(())
    }
}
