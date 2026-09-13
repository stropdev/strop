use super::io_error;
use crate::{launch::Launch, model::Geometry, Error};
use std::{
    fs::{File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::{fs::OpenOptionsExt, net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
};

pub fn channel(fd: OwnedFd) -> Result<UnixStream, Error> {
    if rustix::net::sockopt::socket_type(&fd)
        .map_err(|error| io_error("inspect helper channel", error.into()))?
        != rustix::net::SocketType::STREAM
    {
        return Err(Error::Protocol(
            "terminal helper requires a private stream socket".into(),
        ));
    }
    let stream = UnixStream::from(fd);
    stream
        .peer_addr()
        .map_err(|error| io_error("inspect helper peer", error))?;
    #[cfg(target_os = "linux")]
    {
        let credentials = rustix::net::sockopt::socket_peercred(&stream)
            .map_err(|error| io_error("read helper credentials", error.into()))?;
        if credentials.uid != rustix::process::geteuid()
            || Some(credentials.pid) != rustix::process::getppid()
        {
            return Err(Error::Protocol(
                "terminal helper peer is not its parent owner".into(),
            ));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: socket is owned and both credential output pointers are valid.
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(io_error(
                "read helper credentials",
                io::Error::last_os_error(),
            ));
        }
        if uid != rustix::process::geteuid().as_raw() {
            return Err(Error::Protocol(
                "terminal helper peer has a different owner".into(),
            ));
        }
    }
    stream
        .set_nonblocking(true)
        .map_err(|error| io_error("configure helper channel", error))?;
    Ok(stream)
}

pub fn pair(geometry: Geometry) -> Result<(File, File), Error> {
    // This entrypoint runs before creating any threads. Reset inherited ignored
    // dispositions; openpty/grantpt require no SIGCHLD handler during creation.
    for signal in [
        libc::SIGCHLD,
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGQUIT,
        libc::SIGTERM,
        libc::SIGTSTP,
        libc::SIGTTIN,
        libc::SIGTTOU,
    ] {
        // SAFETY: single-threaded helper owns its process signal dispositions.
        if unsafe { libc::signal(signal, libc::SIG_DFL) } == libc::SIG_ERR {
            return Err(io_error("reset helper signals", io::Error::last_os_error()));
        }
    }
    rustix::process::setsid()
        .map_err(|error| io_error("establish terminal session", error.into()))?;
    let size = rustix::termios::Winsize {
        ws_row: geometry.rows,
        ws_col: geometry.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pair = rustix_openpty::openpty(None, Some(&size))
        .map_err(|error| io_error("open PTY", error.into()))?;
    let master = File::from(pair.controller);
    let slave = File::from(pair.user);
    // SAFETY: a new session with no controlling tty takes its owned PTY slave.
    if unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSCTTY as _, 0) } != 0 {
        return Err(io_error(
            "establish controlling terminal",
            io::Error::last_os_error(),
        ));
    }
    let flags = rustix::fs::fcntl_getfl(&master)
        .map_err(|error| io_error("read PTY flags", error.into()))?;
    rustix::fs::fcntl_setfl(&master, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|error| io_error("configure PTY", error.into()))?;
    Ok((master, slave))
}

pub fn launch(launch: &Launch, slave: &File) -> Result<Child, Error> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(&launch.directory)
        .map_err(|error| io_error("open captured terminal directory", error))?;
    let directory_fd = directory.as_raw_fd();
    let slave_fd = slave.as_raw_fd();
    let mut command = Command::new(&launch.program);
    // SAFETY: the helper must survive closing its own controlling PTY. The child
    // restores SIGHUP before exec so application job-control semantics are intact.
    if unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) } == libc::SIG_ERR {
        return Err(io_error(
            "protect terminal session anchor",
            io::Error::last_os_error(),
        ));
    }
    command
        .args(&launch.arguments)
        .env_clear()
        .envs(launch.environment.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::from(
            slave
                .try_clone()
                .map_err(|error| io_error("clone PTY stdin", error))?,
        ))
        .stdout(Stdio::from(
            slave
                .try_clone()
                .map_err(|error| io_error("clone PTY stdout", error))?,
        ))
        .stderr(Stdio::from(
            slave
                .try_clone()
                .map_err(|error| io_error("clone PTY stderr", error))?,
        ))
        .process_group(0);
    // SAFETY: pre_exec calls only async-signal-safe syscalls, using descriptors
    // held alive through spawn. fchdir pins the admitted directory across rename.
    // The child takes foreground ownership before exec/read, avoiding SIGTTIN.
    unsafe {
        command.pre_exec(move || {
            if libc::signal(libc::SIGHUP, libc::SIG_DFL) == libc::SIG_ERR {
                return Err(io::Error::last_os_error());
            }
            if libc::fchdir(directory_fd) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut set = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTTOU);
            let mut old = std::mem::zeroed();
            if libc::sigprocmask(libc::SIG_BLOCK, &set, &mut old) != 0 {
                return Err(io::Error::last_os_error());
            }
            let foreground = libc::tcsetpgrp(slave_fd, libc::getpid());
            let foreground_error = io::Error::last_os_error();
            if libc::sigprocmask(libc::SIG_SETMASK, &old, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
            if foreground != 0 {
                return Err(foreground_error);
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|error| io_error("spawn terminal program", error))
}

pub fn resize(master: &File, geometry: Geometry) -> Result<(), Error> {
    let size = libc::winsize {
        ws_row: geometry.rows,
        ws_col: geometry.columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: owned master and initialized winsize for this synchronous ioctl.
    if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) } != 0 {
        return Err(io_error("resize PTY", io::Error::last_os_error()));
    }
    Ok(())
}
