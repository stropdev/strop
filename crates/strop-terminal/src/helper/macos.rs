use super::io_error;
use crate::Error;
use rustix::process::Signal;
use std::{fs::File, io, os::fd::AsRawFd};

// Darwin sys/proc_info.h: list only members attached to this controlling tty.
const PROC_TTY_ONLY: u32 = 3;
const MAX_MEMBERS: usize = 16384;
pub struct Session {
    master: File,
    id: u32,
    device: u32,
}
impl Session {
    pub fn new(master: &File) -> Result<Self, Error> {
        let id = std::process::id();
        let own = inspect(id as i32)?
            .ok_or_else(|| Error::Unavailable("Darwin process inspection unavailable".into()))?;
        if own.pbi_pgid != id || own.e_tdev == u32::MAX {
            return Err(Error::Protocol(
                "helper has no private controlling terminal".into(),
            ));
        }
        Ok(Self {
            master: master
                .try_clone()
                .map_err(|error| io_error("retain controlling PTY", error))?,
            id,
            device: own.e_tdev,
        })
    }
    pub fn sweep(&self, signal: Option<Signal>) -> Result<usize, Error> {
        let mut pids = vec![0i32; MAX_MEMBERS];
        let capacity = std::mem::size_of_val(pids.as_slice());
        // SAFETY: the writable array has precisely capacity initialized bytes.
        let bytes = unsafe {
            libc::proc_listpids(
                PROC_TTY_ONLY,
                self.device,
                pids.as_mut_ptr().cast(),
                capacity as i32,
            )
        };
        if bytes < 0 {
            return Err(io_error(
                "enumerate terminal members",
                io::Error::last_os_error(),
            ));
        }
        if bytes as usize >= capacity {
            return Err(Error::Capacity("Darwin terminal session member scan"));
        }
        if bytes as usize % std::mem::size_of::<i32>() != 0 {
            return Err(Error::Protocol("misaligned Darwin process list".into()));
        }
        let mut live = 0;
        for &pid in &pids[..bytes as usize / std::mem::size_of::<i32>()] {
            if pid <= 0 || pid as u32 == self.id {
                continue;
            }
            let Some(info) = inspect(pid)? else { continue };
            // Darwin SZOMB == 5. Exited processes retain identity, not live jobs.
            if info.e_tdev != self.device || info.pbi_pgid == self.id || info.pbi_status == 5 {
                continue;
            }
            live += 1;
            if let Some(signal) = signal {
                self.signal_group(info.pbi_pgid as i32, signal)?;
            }
        }
        Ok(live)
    }
    fn signal_group(&self, group: i32, signal: Signal) -> Result<(), Error> {
        // SAFETY: initialized signal sets are confined to this single-threaded
        // helper. Blocking SIGTTOU permits restoring its tty foreground owner.
        unsafe {
            let mut set = std::mem::zeroed();
            let mut old = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTTOU);
            if libc::sigprocmask(libc::SIG_BLOCK, &set, &mut old) != 0 {
                return Err(io_error(
                    "block helper job-control signal",
                    io::Error::last_os_error(),
                ));
            }
            // TIOCSPGRP checks membership in this controlling terminal's session.
            // TIOCSIG then signals through the same owned tty, not a reusable PID.
            let result = if libc::tcsetpgrp(self.master.as_raw_fd(), group) != 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ESRCH | libc::EPERM | libc::EINVAL)
                ) {
                    Ok(())
                } else {
                    Err(io_error("select terminal process group", error))
                }
            } else if libc::ioctl(
                self.master.as_raw_fd(),
                libc::TIOCSIG as libc::c_ulong,
                signal.as_raw(),
            ) != 0
            {
                Err(io_error(
                    "signal owned terminal group",
                    io::Error::last_os_error(),
                ))
            } else {
                Ok(())
            };
            let restore = libc::sigprocmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
            if restore != 0 {
                return Err(io_error(
                    "restore helper signal mask",
                    io::Error::last_os_error(),
                ));
            }
            result
        }
    }
}
fn inspect(pid: i32) -> Result<Option<libc::proc_bsdinfo>, Error> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let capacity = std::mem::size_of::<libc::proc_bsdinfo>();
    // SAFETY: output points at properly aligned storage of the documented size.
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            capacity as i32,
        )
    };
    if bytes == capacity as i32 {
        // SAFETY: a full successful record initialized every field.
        return Ok(Some(unsafe { info.assume_init() }));
    }
    let error = io::Error::last_os_error();
    if bytes == 0 && error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(None);
    }
    Err(io_error("inspect terminal member", error))
}
