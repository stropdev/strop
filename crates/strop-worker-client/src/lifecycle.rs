//! One lease's connection factory, reconnect slot and final retirement.

use std::path::PathBuf;
use std::sync::{mpsc::Sender, Arc};

use parking_lot::Mutex;
use strop_worker_protocol::Event;

use crate::connection::{self, Conn};
use crate::{ClientError, StreamNotifier, Transport};

/// How a connection is (re)created. Process transports carry a child to
/// reap; factory transports (the in-process test seam and deployed
/// remote/container workers) cross the same codec over caller-provided
/// pipes. Each connector also pins the target triple the worker's
/// handshake must report: this build's own, or the admitted endpoint's
/// for a deployed worker (WK07/WK08).
pub(super) enum Connector {
    /// `current_exe --worker-stdio`: the matching installed executable.
    Installed,
    /// An explicit artifact path (administrator-provisioned or test);
    /// the handshake validates its identity exactly as for `Installed`.
    Program(PathBuf),
    /// Caller-provided duplex plus the endpoint target the handshake
    /// must report (this build's triple for the in-process test seam).
    Deployed {
        expected_target: String,
        factory: Box<dyn Fn() -> std::io::Result<Transport> + Send + Sync>,
    },
}

impl Connector {
    pub(super) fn expected_target(&self) -> String {
        match self {
            Self::Installed | Self::Program(_) => strop_worker_protocol::TARGET_TRIPLE.to_string(),
            Self::Deployed {
                expected_target, ..
            } => expected_target.clone(),
        }
    }

    pub(super) fn connect(&self) -> Result<Transport, ClientError> {
        match self {
            Self::Installed => {
                let program = std::env::current_exe()
                    .map_err(|error| ClientError::Spawn(format!("locate current exe: {error}")))?;
                connection::spawn_worker(&program)
            }
            Self::Program(path) => connection::spawn_worker(path),
            Self::Deployed { factory, .. } => factory()
                .map_err(|error| ClientError::Spawn(format!("in-process transport: {error}"))),
        }
    }
}

pub(super) struct Shared {
    pub(super) connector: Connector,
    pub(super) slot: Mutex<Option<Arc<Conn>>>,
    /// Serializes (re)connection so one death never spawns two workers.
    pub(super) connecting: Mutex<()>,
    pub(super) events: Mutex<Option<Sender<Event>>>,
    /// The stream-arrival hook (WK12), re-installed on every
    /// (re)connection like the event sink.
    pub(super) stream_notifier: Mutex<Option<StreamNotifier>>,
}

impl Shared {
    pub(super) fn new(connector: Connector) -> Self {
        Self {
            connector,
            slot: Mutex::new(None),
            connecting: Mutex::new(()),
            events: Mutex::new(None),
            stream_notifier: Mutex::new(None),
        }
    }
}

impl Drop for Shared {
    /// The last owner closed: the worker is retired (shutdown handshake,
    /// bounded wait, reap). Never a daemon left behind.
    fn drop(&mut self) {
        if let Some(conn) = self.slot.lock().take() {
            conn.retire();
        }
    }
}
