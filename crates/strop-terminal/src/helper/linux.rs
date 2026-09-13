use super::io_error;
use crate::Error;
use rustix::process::{pidfd_open, pidfd_send_signal, Pid, PidfdFlags, Signal};
use std::io::Read;

pub struct Session {
    id: i32,
}
impl Session {
    pub fn new(_master: &std::fs::File) -> Result<Self, Error> {
        let id = std::process::id() as i32;
        let pid =
            Pid::from_raw(id).ok_or_else(|| Error::Protocol("invalid helper identity".into()))?;
        pidfd_open(pid, PidfdFlags::empty())
            .map_err(|error| Error::Unavailable(format!("Linux pidfd supervision: {error}")))?;
        let own = inspect(id)?
            .ok_or_else(|| Error::Unavailable("/proc session inspection unavailable".into()))?;
        if own.session != id {
            return Err(Error::Protocol("helper is not its session anchor".into()));
        }
        // SAFETY: the dedicated helper is single-threaded and owns its process
        // policy. Adopting orphaned descendants lets it reap its background jobs.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
            return Err(io_error(
                "enable terminal descendant reaping",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(Self { id })
    }

    /// Each signal uses a pidfd; numeric directory names only discover candidates.
    /// Keeping this helper alive reserves the private SID throughout the scan.
    pub fn sweep(&self, signal: Option<Signal>) -> Result<usize, Error> {
        let mut live = 0;
        for entry in std::fs::read_dir("/proc")
            .map_err(|error| io_error("enumerate terminal session", error))?
        {
            let entry = entry.map_err(|error| io_error("enumerate terminal member", error))?;
            let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
            else {
                continue;
            };
            if id == self.id {
                continue;
            }
            let Some(before) = inspect(id)? else { continue };
            if before.session != self.id || matches!(before.state, b'Z' | b'X') {
                continue;
            }
            let Some(pid) = Pid::from_raw(id) else {
                continue;
            };
            let handle = match pidfd_open(pid, PidfdFlags::empty()) {
                Ok(handle) => handle,
                Err(rustix::io::Errno::SRCH) => continue,
                Err(error) => {
                    return Err(Error::Unavailable(format!(
                        "open terminal member pidfd: {error}"
                    )))
                }
            };
            let Some(after) = inspect(id)? else { continue };
            if after.session != self.id
                || after.start != before.start
                || matches!(after.state, b'Z' | b'X')
            {
                continue;
            }
            live += 1;
            if let Some(signal) = signal {
                match pidfd_send_signal(&handle, signal) {
                    Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                    Err(error) => {
                        return Err(Error::Unavailable(format!(
                            "signal terminal member: {error}"
                        )))
                    }
                }
            }
        }
        Ok(live)
    }
}

struct Identity {
    session: i32,
    start: u64,
    state: u8,
}
fn inspect(pid: i32) -> Result<Option<Identity>, Error> {
    let file = match std::fs::File::open(format!("/proc/{pid}/stat")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("inspect terminal member", error)),
    };
    let mut bytes = Vec::with_capacity(4096);
    file.take(4096)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read terminal member identity", error))?;
    let end = bytes
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or_else(|| Error::Protocol("invalid proc stat command field".into()))?;
    let fields = std::str::from_utf8(&bytes[end + 1..])
        .map_err(|_| Error::Protocol("invalid proc stat identity fields".into()))?;
    let mut fields = fields.split_ascii_whitespace();
    let state = fields
        .next()
        .and_then(|field| field.as_bytes().first())
        .copied();
    let session = fields.nth(2).and_then(|field| field.parse().ok());
    let start = fields.nth(15).and_then(|field| field.parse().ok());
    match (state, session, start) {
        (Some(state), Some(session), Some(start)) => Ok(Some(Identity {
            state,
            session,
            start,
        })),
        _ => Err(Error::Protocol("incomplete proc stat identity".into())),
    }
}
