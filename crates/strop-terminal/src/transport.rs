//! Where one session's PTY runs (0090 §4): in this process for a local
//! directory, on the namespace's admitted worker for a remote one. The
//! client drives both through one surface; the two differ only in how
//! a resize is ordered against output and in how the actor waits.
use crate::{local::LocalPty, model::Geometry, Error};
use std::{os::unix::net::UnixDatagram, sync::mpsc::TryRecvError};
use strop_worker_client::{ExecEvent, PtySession, StreamEvent, Worker};
use strop_worker_protocol::PtyGeometry;

/// The caller's choice of where the child runs.
pub enum Target {
    /// The editor process, on a fresh in-process PTY.
    Local,
    /// An admitted worker lease (0058 WK12).
    Worker(Worker),
}

pub(crate) enum Transport {
    Local(LocalPty),
    Worker(PtySession),
}

impl Transport {
    /// Input bytes; the wire sequence returned is acknowledged on
    /// [`ExecEvent::Input`] once delivered.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<u64, Error> {
        match self {
            Self::Local(pty) => pty.feed(bytes),
            Self::Worker(pty) => pty
                .feed(bytes)
                .map_err(|error| Error::Unavailable(error.to_string())),
        }
    }

    /// Apply a geometry. `true` means an ordered [`StreamEvent::Resized`]
    /// marker will follow on the output stream and the VT must flip
    /// there; `false` means the ioctl landed now and the VT flips now.
    pub(crate) fn resize(&mut self, geometry: Geometry) -> Result<bool, Error> {
        match self {
            Self::Local(pty) => pty.resize(geometry).map(|()| false),
            Self::Worker(pty) => pty
                .resize(PtyGeometry {
                    columns: geometry.columns,
                    rows: geometry.rows,
                })
                .map(|()| true)
                .map_err(|error| Error::Unavailable(error.to_string())),
        }
    }

    pub(crate) fn terminate(&mut self) -> Result<(), Error> {
        match self {
            Self::Local(pty) => {
                pty.terminate();
                Ok(())
            }
            Self::Worker(pty) => pty
                .terminate()
                .map_err(|error| Error::Unavailable(error.to_string())),
        }
    }

    pub(crate) fn try_output(&mut self) -> Result<StreamEvent, TryRecvError> {
        match self {
            Self::Local(pty) => {
                // Retained input gets its chance on every turn.
                if pty.flush().is_err() {
                    return Err(TryRecvError::Disconnected);
                }
                pty.try_output()
            }
            Self::Worker(pty) => pty.try_output(),
        }
    }

    pub(crate) fn try_event(&self) -> Result<ExecEvent, TryRecvError> {
        match self {
            Self::Local(pty) => pty.events().try_recv(),
            Self::Worker(pty) => pty.events().try_recv(),
        }
    }

    /// Park until a wake (an intent or an output nudge), or until a local
    /// master can take retained input. The timeout only bounds wake-loss
    /// recovery; ordinary output arrives by nudge.
    pub(crate) fn wait(&self, wake: &UnixDatagram) -> Result<(), Error> {
        use std::os::fd::AsRawFd;
        let mut descriptors = [
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: -1,
                events: libc::POLLOUT,
                revents: 0,
            },
        ];
        if let Self::Local(pty) = self {
            if pty.has_pending_input() {
                descriptors[1].fd = pty.master_fd();
            }
        }
        // SAFETY: the service owns both descriptors and the pollfd storage
        // for the duration of the call.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, 250) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(Error::Io {
                    operation: "wait for terminal input",
                    detail: error.to_string(),
                });
            }
        }
        Ok(())
    }
}
