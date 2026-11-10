//! Thin direct inotify(2) binding (Linux). Four syscalls — `inotify_init1`,
//! `inotify_add_watch`, `inotify_rm_watch`, `read` — plus `poll` for the
//! worker's event loop. No crate sits between us and the kernel queue, so
//! overflow (`IN_Q_OVERFLOW`), queue bounds and watch lifetimes are exactly
//! what the kernel reports, and a musl static build carries no extra weight
//! (`libc` is already a workspace dependency).
//!
//! Raw masks never leave the `notify` module; the wire contract
//! (`strop_worker_protocol::NotifyKind`) is produced by the manager, never
//! by exposing these bits.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Read buffer per cycle: 64 KiB holds a useful slice of a busy queue
/// without unbounded retention; the manager bounds total drain work on top.
pub const READ_BUF_BYTES: usize = 64 * 1024;

/// One raw kernel event, bytes already detached from the read buffer.
/// `name` is empty for events about the watched object itself. `mask` bits
/// are `libc::IN_*` values; only the manager interprets them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    pub wd: i32,
    pub mask: u32,
    pub cookie: u32,
    pub name: Vec<u8>,
}

/// The result of one drain cycle. A read error never discards already
/// parsed events silently: they are returned alongside the error, and the
/// caller invalidates the baseline (`truncated` marks a torn buffer tail,
/// which the same invalidation covers).
#[derive(Debug)]
pub struct ReadOutcome {
    pub events: Vec<RawEvent>,
    pub truncated: bool,
    pub error: Option<io::Error>,
}

/// One inotify instance. Owned per subscription, so its kernel queue (and
/// therefore `IN_Q_OVERFLOW`) is scoped to exactly one subscription's
/// baseline, and teardown is one `close(2)` — cancellation drains promptly
/// with no per-watch dance.
#[derive(Debug)]
pub struct Inotify {
    fd: i32,
    buf: Vec<u8>,
}

impl Inotify {
    /// A nonblocking, close-on-exec instance with a bounded read buffer.
    pub fn init() -> io::Result<Self> {
        // SAFETY: no Rust safety invariant at stake; the returned fd is
        // owned by this value and checked for errors before use.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            buf: vec![0; READ_BUF_BYTES],
        })
    }

    pub fn fd(&self) -> i32 {
        self.fd
    }

    /// Register `path` under `mask`; returns the kernel watch descriptor.
    /// Adding a watch on an inode this instance already watches returns the
    /// existing descriptor with the mask replaced — renamed-directory
    /// re-registration relies on this.
    pub fn add_watch(&mut self, path: &Path, mask: u32) -> io::Result<i32> {
        let bytes = path.as_os_str().as_bytes();
        let c_path = CString::new(bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "path contains an interior NUL")
        })?;
        // SAFETY: `c_path` is a valid NUL-terminated string that outlives
        // the call; `fd` is a live inotify instance owned by `self`.
        let wd = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), mask) };
        if wd == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(wd)
    }

    /// Retire one watch. The kernel queues `IN_IGNORED` for the descriptor
    /// and may reuse it only afterwards; `EINVAL` means it was already
    /// retired (the stale-descriptor race the manager defends against).
    pub fn remove_watch(&mut self, wd: i32) -> io::Result<()> {
        // SAFETY: `fd` is a live inotify instance owned by `self`; a stale
        // `wd` is an ordinary `EINVAL` error, not memory unsafety.
        if unsafe { libc::inotify_rm_watch(self.fd, wd) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Drain the kernel queue until it would block. `max_cycles` bounds one
    /// call's work against a continuously-written queue; leftover events
    /// stay in the kernel queue for the next drain, and a real overrun is
    /// reported by the kernel as `IN_Q_OVERFLOW`, never silently.
    pub fn read_events(&mut self, max_cycles: usize) -> ReadOutcome {
        let mut outcome = ReadOutcome {
            events: Vec::new(),
            truncated: false,
            error: None,
        };
        for _ in 0..max_cycles {
            // SAFETY: `buf` is a live, exclusively-borrowed byte buffer;
            // the kernel writes at most `buf.len()` bytes into it.
            let n = unsafe { libc::read(self.fd, self.buf.as_mut_ptr().cast(), self.buf.len()) };
            if n == -1 {
                let err = io::Error::last_os_error();
                match err.kind() {
                    io::ErrorKind::WouldBlock => break,
                    io::ErrorKind::Interrupted => continue,
                    _ => {
                        outcome.error = Some(err);
                        break;
                    }
                }
            }
            let complete = parse_events(&self.buf[..n as usize], &mut outcome.events);
            outcome.truncated |= !complete;
        }
        outcome
    }
}

impl Drop for Inotify {
    fn drop(&mut self) {
        // SAFETY: `fd` is owned by this value and closed exactly once here;
        // every kernel watch on the instance dies with it.
        unsafe { libc::close(self.fd) };
    }
}

/// Parse one read buffer into events, returning `false` on a torn tail.
/// The `inotify_event` header is four fixed fields; `len` covers the name
/// plus its NUL padding. Bounds-checked slice arithmetic only — no
/// unaligned struct references. The kernel never splits an event across
/// reads of a large-enough buffer, so a torn tail is state corruption:
/// drop it and let the caller invalidate the baseline.
fn parse_events(mut bytes: &[u8], events: &mut Vec<RawEvent>) -> bool {
    const HEADER: usize = 16;
    while bytes.len() >= HEADER {
        let wd = i32::from_ne_bytes(bytes[0..4].try_into().unwrap_or_default());
        let mask = u32::from_ne_bytes(bytes[4..8].try_into().unwrap_or_default());
        let cookie = u32::from_ne_bytes(bytes[8..12].try_into().unwrap_or_default());
        let len = u32::from_ne_bytes(bytes[12..16].try_into().unwrap_or_default()) as usize;
        if bytes.len() < HEADER + len {
            return false;
        }
        let raw_name = &bytes[HEADER..HEADER + len];
        let end = raw_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw_name.len());
        events.push(RawEvent {
            wd,
            mask,
            cookie,
            name: raw_name[..end].to_vec(),
        });
        bytes = &bytes[HEADER + len..];
    }
    bytes.is_empty()
}

/// Block until any of `fds` is readable or `timeout` elapses. Returns
/// `true` when at least one fd reports `POLLIN`/`POLLERR`/`POLLHUP`.
/// An empty `fds` answers `false` immediately — there is nothing to wait on.
pub fn poll(fds: &[i32], timeout: Option<std::time::Duration>) -> io::Result<bool> {
    if fds.is_empty() {
        return Ok(false);
    }
    let millis = match timeout {
        None => -1,
        Some(d) => d.as_millis().try_into().unwrap_or(i32::MAX),
    };
    let mut fds: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    loop {
        // SAFETY: `fds` is a live slice of `pollfd` for the duration of the
        // call; the kernel writes at most `fds.len()` entries back into it.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) };
        if n == -1 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        let any = fds
            .iter()
            .any(|p| p.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0);
        return Ok(any);
    }
}
