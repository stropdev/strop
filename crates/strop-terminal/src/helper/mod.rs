//! Same-binary, single-threaded native PTY supervisor. Never run in the editor
//! process: this entrypoint owns a controlling session until descendants drain.
mod input;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod native;
use crate::{launch::WireLaunch, model::Phase, protocol, Error};
use input::InputQueue;
#[cfg(target_os = "linux")]
use linux::Session;
#[cfg(target_os = "macos")]
use macos::Session;
use native::resize;
use rustix::process::Signal;
use std::{
    fs::File,
    io::{self, Read},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::ExitStatusExt},
    },
    process::Child,
    time::{Duration, Instant},
};

/// Enter only at process startup, before threads or ordinary CLI initialization.
///
/// # Safety
/// The caller transfers sole ownership of the inherited descriptor to this helper.
pub unsafe fn run_inherited(fd: i32) -> Result<(), Error> {
    use std::os::fd::FromRawFd;
    // SAFETY: fcntl validates an integer without borrowing or dereferencing it.
    if fd < 3 || unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
        return Err(Error::Protocol(
            "invalid inherited helper descriptor".into(),
        ));
    }
    // SAFETY: caller transfers unique ownership; fcntl verified the fd is open.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)
        .map_err(|error| io_error("protect helper descriptor", error.into()))?;
    run(fd)
}

pub fn run(fd: OwnedFd) -> Result<(), Error> {
    let mut channel = native::channel(fd)?;
    let mut final_sent = false;
    let result = run_channel(&mut channel, &mut final_sent);
    if let Err(error) = &result {
        if !final_sent {
            let mut writer = protocol::Writer::default();
            writer.push(protocol::FAILED, error.to_string().into_bytes())?;
            finish(
                &mut channel,
                &mut writer,
                Phase::Exited {
                    code: None,
                    signal: None,
                },
                &mut final_sent,
            )?;
        }
    }
    result
}

fn run_channel(channel: &mut UnixStream, final_sent: &mut bool) -> Result<(), Error> {
    let mut reader = protocol::Reader::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    let packet = loop {
        if let Some(packet) = command(&mut reader, channel)? {
            break packet;
        }
        if reader.eof() {
            return finish(
                channel,
                &mut protocol::Writer::default(),
                Phase::Exited {
                    code: None,
                    signal: None,
                },
                final_sent,
            );
        }
        if Instant::now() >= deadline {
            return Err(Error::Unavailable(
                "helper launch handshake timed out".into(),
            ));
        }
        std::thread::park_timeout(Duration::from_millis(2));
    };
    if packet.kind != protocol::LAUNCH {
        return Err(Error::Protocol(
            "helper requires launch as its first record".into(),
        ));
    }
    let (launch, geometry) = WireLaunch::decode(&packet.body)?;
    let (mut master, slave) = native::pair(geometry)?;
    let session = Session::new(&master)?;
    let mut child = native::launch(&launch, &slave)?;
    drop(slave);
    let mut writer = protocol::Writer::default();
    let mut inputs = InputQueue::new(geometry);
    let outcome = writer
        .push(protocol::READY, protocol::VERSION.to_le_bytes().to_vec())
        .and_then(|()| {
            pump(
                channel,
                &mut reader,
                &mut writer,
                &mut inputs,
                &mut master,
                &child,
                &session,
            )
        });
    // Errors revoke input but never bypass native ownership. Keep the SID anchor
    // alive even if cleanup is temporarily unavailable; never signal a reused PID.
    if let Err(error) = &outcome {
        let _ = writer.push(protocol::FAILED, error.to_string().into_bytes());
        inputs.clear();
        loop {
            match session.sweep(Some(Signal::KILL)) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => {
                    // Direct-child fallback retains its unreaped PID reservation.
                    // The session anchor still remains alive for descendant retry.
                    let _ = child.kill();
                }
            }
            let _ = drain(&mut master, &mut writer, true);
            let _ = writer.flush(channel);
            std::thread::park_timeout(Duration::from_millis(20));
        }
    }
    let status = child
        .wait()
        .map_err(|error| io_error("reap terminal program", error))?;
    reap_adopted()?;
    let phase = Phase::Exited {
        code: status.code(),
        signal: status.signal(),
    };
    finish(channel, &mut writer, phase, final_sent)?;
    outcome
}

fn pump(
    channel: &mut UnixStream,
    reader: &mut protocol::Reader,
    writer: &mut protocol::Writer,
    inputs: &mut InputQueue,
    master: &mut File,
    child: &Child,
    session: &Session,
) -> Result<(), Error> {
    let mut closing = None;
    // Darwin reports the last slave closure as POLLHUP on the controller
    // instead of an EOF/EIO read (Linux's report), so the hangup seen by the
    // transport poll is sticky evidence the session can never produce more.
    let mut hangup = false;
    loop {
        for _ in 0..32 {
            let Some(packet) = command(reader, channel)? else {
                break;
            };
            if packet.kind == protocol::STOP {
                if !packet.body.is_empty() {
                    return Err(Error::Protocol("nonempty terminal stop record".into()));
                }
                closing.get_or_insert_with(Instant::now);
                inputs.clear();
            } else if closing.is_none() {
                inputs.admit(packet)?;
            }
        }
        if reader.eof()
            || strop_core::process::child_has_exited(child)
                .map_err(|error| Error::Protocol(error.message))?
        {
            closing.get_or_insert_with(Instant::now);
            inputs.clear();
        }
        writer.flush(channel)?;
        if closing.is_none() {
            inputs.flush(master, writer)?;
        }
        let eof = hangup || drain(master, writer, false)?;
        if let Some(start) = closing {
            let signal = if start.elapsed() < Duration::from_millis(250) {
                Signal::TERM
            } else {
                Signal::KILL
            };
            let live = session.sweep(Some(signal))?;
            if live == 0 && eof {
                return Ok(());
            }
        }
        hangup |= wait_io(
            channel,
            master,
            !reader.eof(),
            !eof && writer.data_room() > 0,
            writer.pending(),
            closing.is_none() && inputs.pending(),
            if closing.is_some() { 20 } else { 250 },
        )?;
    }
}

fn drain(master: &mut File, writer: &mut protocol::Writer, discard: bool) -> Result<bool, Error> {
    let mut bytes = [0; 8192];
    for _ in 0..8 {
        let room = if discard {
            bytes.len()
        } else {
            writer.data_room().min(bytes.len())
        };
        if room == 0 {
            return Ok(false);
        }
        match master.read(&mut bytes[..room]) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if !discard {
                    writer.push(protocol::OUTPUT, bytes[..count].to_vec())?;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            // Linux reports last-slave closure as EIO rather than a zero read.
            Err(error) if error.raw_os_error() == Some(libc::EIO) => return Ok(true),
            Err(error) => return Err(io_error("read terminal output", error)),
        }
    }
    Ok(false)
}
fn io_error(operation: &'static str, error: io::Error) -> Error {
    Error::Io {
        operation,
        detail: error.to_string(),
    }
}

fn reap_adopted() -> Result<(), Error> {
    loop {
        let mut status = 0;
        // SAFETY: waitpid(-1) can reap only this helper's children. The root has
        // already been reaped and session members are dead; no signaling follows.
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid == 0 {
            return Ok(());
        }
        if pid > 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ECHILD) {
            return Ok(());
        }
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(io_error("reap terminal descendants", error));
        }
    }
}

fn wait_io(
    channel: &UnixStream,
    master: &File,
    read_channel: bool,
    read_master: bool,
    write_channel: bool,
    write_master: bool,
    timeout_ms: i32,
) -> Result<bool, Error> {
    use std::os::fd::AsRawFd;
    let interests = |read, write| {
        (if read { libc::POLLIN } else { 0 }) | (if write { libc::POLLOUT } else { 0 })
    };
    let channel_events = interests(read_channel, write_channel);
    let master_events = interests(read_master, write_master);
    let mut descriptors = [
        libc::pollfd {
            fd: if channel_events == 0 {
                -1
            } else {
                channel.as_raw_fd()
            },
            events: channel_events,
            revents: 0,
        },
        libc::pollfd {
            fd: if master_events == 0 {
                -1
            } else {
                master.as_raw_fd()
            },
            events: master_events,
            revents: 0,
        },
    ];
    // SAFETY: both descriptors are owned throughout poll; the array is writable.
    let result = unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            timeout_ms,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(io_error("wait for terminal transport", error));
        }
    }
    // POLLHUP/POLLERR on the controller is the only last-slave-closure notice
    // Darwin's transport gives; the read-EIO report is a Linux quirk. The
    // descriptor is skipped only under output backpressure, which the lease
    // owner always drains to unblock.
    Ok(descriptors[1].revents & (libc::POLLHUP | libc::POLLERR) != 0)
}

fn command(
    reader: &mut protocol::Reader,
    channel: &mut UnixStream,
) -> Result<Option<protocol::Packet>, Error> {
    match reader.next(channel) {
        // Closing the lease revokes even a partly transmitted command. It was
        // never admitted to the PTY; output-side truncation remains an error.
        Err(_) if reader.eof() => Ok(None),
        result => result,
    }
}

fn finish(
    channel: &mut UnixStream,
    writer: &mut protocol::Writer,
    phase: Phase,
    final_sent: &mut bool,
) -> Result<(), Error> {
    writer.push(
        protocol::EXITED,
        serde_json::to_vec(&phase).map_err(|error| Error::Protocol(error.to_string()))?,
    )?;
    *final_sent = true;
    while writer.pending() {
        writer.flush(channel)?;
        if writer.pending() {
            std::thread::park_timeout(Duration::from_millis(2));
        }
    }
    Ok(())
}
