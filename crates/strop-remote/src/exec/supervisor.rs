//! The remote process supervisor: one fixed, embedded Python 3 program
//! that owns the remote lifecycle, plus the local code that talks to it.
//!
//! No helper is installed remotely and nothing is written to the remote
//! filesystem. The login shell selects a compatible interpreter, then
//! `exec`s this fixed source with the base64 command spec. The bounded
//! selector is shared by Git and LSP; every later lifecycle decision
//! belongs to Python.
//!
//! ## Topology
//!
//! ```text
//! sshd
//!   └─ supervisor             python3; owns SSH stdin, relay, cancel
//!        └─ anchor            setsid(); PGID = anchor PID; ignores TERM
//!             └─ worker       native exec with byte argv
//! ```
//!
//! The anchor pins the process-group identity: after `setsid()` its PID
//! is the PGID of the worker and any descendants. It posts a READY byte
//! before creating the worker and stays (live, or unreaped zombie —
//! either keeps the PID allocated) until the supervisor has finished
//! signaling, so `killpg` can never target a reused PGID.
//!
//! ## Ownership story (what is guaranteed, and honestly what is not)
//!
//! - **Local child:** the caller spawns the returned
//!   `std::process::Command` through `strop_core::process::OwnedProcess`
//!   in its own process group; the token hook SIGKILLs that group on
//!   cancellation and the PID is revoked before it is reaped.
//! - **Remote group:** SSH stdin is the lifetime lease. Local close
//!   (drop of the stdin writer, or death of local ssh) makes sshd close
//!   the supervisor's stdin; the supervisor then SIGTERMs the anchor's
//!   group, waits the bounded grace, SIGKILLs it, and only then reaps
//!   the anchor. A worker that exited normally still gets this cleanup,
//!   because a finite git command can leave descendants behind.
//! - **Normal exit:** the anchor `waitpid`s the worker and posts a
//!   fixed-size status record; the supervisor exits with the worker's
//!   own code (which rides ssh's exit-status channel) after writing a
//!   nonce-marked status line to stderr. Recorded worker status beats a
//!   concurrent hangup/cancel; a 300 ms last-chance drain lets an
//!   in-flight record win when the lease closes right after a graceful
//!   exit, so LSP shutdown responses are not misreported as
//!   cancellation.
//! - **Limits (stated, not papered over):** if the network partitions
//!   before sshd notices the dead local client, remote cleanup waits
//!   for sshd's own disconnection detection (server-side client-alive
//!   configuration bounds this). Descendants that call `setsid()`
//!   themselves can escape a process-group kill; only a cgroup or
//!   service manager contains those. A remote `SIGKILL`, host failure
//!   or supervisor crash can strand a worker — nothing stdio-based can
//!   promise otherwise. The status nonce stops accidental collisions
//!   with worker output, not a worker that deliberately echoes it.
//! - **Backpressure:** the relay keeps a bounded buffer and stops
//!   reading SSH stdin when full; `poll` is asked for zero events then,
//!   which still reports `POLLHUP`/`POLLERR`, so lease close is visible
//!   even under backpressure. A worker that closes its own stdin stops
//!   the relay without killing the worker (its shutdown response may
//!   still be in flight).
//!
//! ## Remote prerequisites
//!
//! A POSIX-compatible login shell, OpenSSH stdio without a PTY, and
//! Python 3.8+ with `fork`, `setsid`, `poll`, `killpg` and native byte
//! execution. The bootstrap probes Python 3 names on PATH, or the explicit
//! `STROP_REMOTE_PYTHON` interpreter only; unavailable/incompatible Python
//! is a typed prerequisite error. Remote stdin must be a pipe whose `poll` reports `POLLHUP`
//! when its last writer closes even with unread bytes buffered (Linux
//! sshd behavior).

use crate::exec::SupervisionOutcome;

/// The fixed supervisor program. ASCII only, Python 3 stdlib only.
const SOURCE: &str = r#"
import os, sys, time, base64, errno, select, signal, struct

MARK = b'STROP-SUP-v1'
RELAY_CAP = 4 * 1024 * 1024
CANCELLED = 251
FAULT = 250

def _clip(value, limit=180):
    data = repr(value).encode('ascii', 'replace')[:limit]
    return data.replace(b'\n', b' ').replace(b'\r', b' ')

def _note(nonce, text):
    try:
        os.write(2, b'\n' + MARK + b' ' + nonce + b' ' + text + b'\n')
    except OSError:
        pass

def _plain(text):
    try:
        os.write(2, b'STROP-ERR ' + text + b'\n')
    except OSError:
        pass

def _parse_spec(blob):
    if len(blob) < 29:
        raise ValueError('short spec')
    version, mode, program, grace = struct.unpack_from('<BBBI', blob, 0)
    if version != 2:
        raise ValueError('version %d' % version)
    if mode > 1:
        raise ValueError('mode %d' % mode)
    if program not in (0, 1):
        raise ValueError('program kind %d' % program)
    off = 7
    nonce = blob[off:off + 16]
    if len(nonce) != 16:
        raise ValueError('nonce')
    off += 16
    def take(at):
        (size,) = struct.unpack_from('<I', blob, at)
        at += 4
        if size > len(blob) - at:
            raise ValueError('length overrun')
        return blob[at:at + size], at + size
    cwd, off = take(off)
    (argc,) = struct.unpack_from('<I', blob, off)
    off += 4
    if argc == 0 or argc > 4096:
        raise ValueError('argc %d' % argc)
    argv = []
    for _ in range(argc):
        item, off = take(off)
        argv.append(item)
    if off != len(blob):
        raise ValueError('trailing bytes')
    if not cwd.startswith(b'/'):
        raise ValueError('cwd not absolute')
    if b'\x00' in cwd or any(b'\x00' in item for item in argv):
        raise ValueError('NUL in spec')
    if program == 1:
        argv = [os.fsencode(sys.executable), b'-I', b'-S'] + argv
    elif not argv[0]:
        raise ValueError('empty executable')
    if grace > 600000:
        grace = 600000
    return mode, grace, nonce, cwd, argv

def _worker(mode, relay_r, cwd, argv, nonce):
    try:
        for name in (signal.SIGHUP, signal.SIGINT, signal.SIGPIPE,
                     signal.SIGTERM):
            signal.signal(name, signal.SIG_DFL)
        try:
            signal.pthread_sigmask(signal.SIG_SETMASK, set())
        except (AttributeError, OSError, ValueError):
            pass
        if mode == 1:
            os.dup2(relay_r, 0)
        else:
            null = os.open('/dev/null', os.O_RDONLY)
            os.dup2(null, 0)
            os.close(null)
        try:
            os.chdir(cwd)
        except OSError as exc:
            _note(nonce, b'exec-error chdir ' + _clip(exc))
            os._exit(125)
        try:
            os.execvpe(argv[0], argv, os.environ)
        except FileNotFoundError:
            _note(nonce, b'exec-error not-found ' + _clip(argv[0]))
            os._exit(127)
        except PermissionError:
            _note(nonce, b'exec-error not-executable ' + _clip(argv[0]))
            os._exit(126)
        except OSError as exc:
            _note(nonce, b'exec-error os ' + _clip(exc))
            os._exit(126)
        _note(nonce, b'exec-error unknown')
        os._exit(126)
    except BaseException as exc:
        _plain(b'worker-fatal ' + _clip(exc))
        os._exit(FAULT)

def _anchor(status_w, status_r, gate_r, gate_w, relay_r, relay_w,
            mode, cwd, argv, nonce):
    try:
        for name in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
            signal.signal(name, signal.SIG_IGN)
        os.setsid()
        os.close(status_r)
        os.close(gate_w)
        if relay_w is not None:
            os.close(relay_w)
        os.write(status_w, b'R')
        gate = os.read(gate_r, 1)
        if gate != b'G':
            os._exit(0)
        os.close(gate_r)
        try:
            pid = os.fork()
        except OSError as exc:
            os.write(status_w, struct.pack('<BI', 2, exc.errno or 0))
            os._exit(0)
        if pid == 0:
            _worker(mode, relay_r, cwd, argv, nonce)
        if relay_r is not None:
            os.close(relay_r)
        while True:
            try:
                _, status = os.waitpid(pid, 0)
                break
            except InterruptedError:
                continue
            except OSError:
                status = 1
                break
        if os.WIFEXITED(status):
            record = struct.pack('<BI', 0, os.WEXITSTATUS(status))
        elif os.WIFSIGNALED(status):
            record = struct.pack('<BI', 1, os.WTERMSIG(status))
        else:
            record = struct.pack('<BI', 2, 0)
        try:
            os.write(status_w, record)
        except OSError:
            pass
        os._exit(0)
    except BaseException as exc:
        _plain(b'anchor-fatal ' + _clip(exc))
        os._exit(FAULT)

def _kill_group(pid, sig, ready):
    try:
        if ready:
            os.killpg(pid, sig)
        else:
            os.kill(pid, sig)  # no GO was sent: only the reserved child exists
        return True
    except ProcessLookupError:
        return True
    except OSError:
        return False

def _sleep_grace(milliseconds):
    remaining = milliseconds / 1000.0
    while remaining > 0:
        pause = 0.05 if remaining > 0.05 else remaining
        time.sleep(pause)
        remaining -= pause

def main():
    try:
        blob = base64.b64decode(sys.argv[1].encode('ascii'), validate=True)
        mode, grace, nonce, cwd, argv = _parse_spec(blob)
    except Exception as exc:
        _plain(b'bad-spec ' + _clip(exc))
        os._exit(FAULT)
    nonce_hex = nonce.hex().encode('ascii')
    cancelled = [False]
    def latch(signum, frame):
        cancelled[0] = True
    for name in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
        signal.signal(name, latch)
    status_r, status_w = os.pipe()
    gate_r, gate_w = os.pipe()
    relay_r = relay_w = None
    if mode == 1:
        relay_r, relay_w = os.pipe()
        os.set_blocking(relay_w, False)
    try:
        anchor = os.fork()
    except OSError as exc:
        _note(nonce_hex, b'error fork-anchor ' + _clip(exc))
        os._exit(FAULT)
    if anchor == 0:
        _anchor(status_w, status_r, gate_r, gate_w, relay_r, relay_w,
                mode, cwd, argv, nonce_hex)
        return
    os.close(status_w)
    os.close(gate_r)
    if relay_r is not None:
        os.close(relay_r)
    poll = select.poll()
    poll.register(status_r, select.POLLIN)
    poll.register(0, 0)  # HUP is observable even before the launch gate
    acc = b''
    failure = None
    hangup = False
    relay_dead = False
    buffered = bytearray()
    while failure is None and not cancelled[0] and not hangup and len(acc) < 1:
        for fd, event in poll.poll(200):
            if fd == 0 and event & (select.POLLHUP | select.POLLERR):
                hangup = True
            if fd == status_r and event & (select.POLLIN | select.POLLHUP
                                           | select.POLLERR):
                data = os.read(status_r, 64)
                if not data:
                    failure = b'anchor-exited-before-ready'
                else:
                    acc += data
    if failure is None and len(acc) >= 1:
        if acc[:1] != b'R':
            failure = b'anchor-protocol'
        elif not cancelled[0] and not hangup:
            if any(fd == 0 and event & (select.POLLHUP | select.POLLERR) for fd, event in poll.poll(0)):
                hangup = True
        if failure is None and not cancelled[0] and not hangup:
            try:
                os.write(gate_w, b'G')
            except OSError as exc:
                failure = b'gate-write ' + _clip(exc)
    os.close(gate_w)
    if failure is None:
        poll.modify(0, select.POLLIN if mode == 0 else 0)
        if relay_w is not None:
            poll.register(relay_w, 0)
    while (failure is None and not hangup and not cancelled[0]
           and len(acc) < 6):
        if mode == 1:
            poll.modify(0, select.POLLIN if not relay_dead and len(buffered) < RELAY_CAP else 0)
            if not relay_dead:
                poll.modify(relay_w, select.POLLOUT if buffered else 0)
        for fd, event in poll.poll(200):
            if (failure is not None or hangup or cancelled[0]
                    or len(acc) >= 6):
                break
            if fd == 0 and event & (select.POLLIN | select.POLLHUP
                                    | select.POLLERR):
                if event & (select.POLLHUP | select.POLLERR):
                    hangup = True
                    break
                capacity = min(65536, RELAY_CAP - len(buffered)) if mode == 1 else 65536
                data = os.read(0, capacity)
                if not data:
                    hangup = True
                elif mode == 1 and not relay_dead:
                    buffered.extend(data)
            elif fd == relay_w:
                if event & (select.POLLERR | select.POLLHUP):
                    relay_dead = True
                    buffered = bytearray()
                    poll.unregister(relay_w)
                elif event & select.POLLOUT and buffered:
                    try:
                        written = os.write(relay_w, bytes(buffered[:65536]))
                        del buffered[:written]
                    except OSError as exc:
                        if exc.errno in (errno.EPIPE, errno.EACCES,
                                         errno.EIO):
                            relay_dead = True
                            buffered = bytearray()
                            poll.unregister(relay_w)
                        elif exc.errno != errno.EAGAIN:
                            failure = b'relay-write'
            elif fd == status_r and event & (select.POLLIN | select.POLLHUP
                                             | select.POLLERR):
                data = os.read(status_r, 64)
                if data:
                    acc += data
                elif len(acc) < 6:
                    failure = b'anchor-exited-early'
    if (failure is None and len(acc) < 6 and (hangup or cancelled[0])):
        drain = select.poll()
        drain.register(status_r, select.POLLIN)
        if drain.poll(300):
            acc += os.read(status_r, 64)
    outcome = None
    if failure is None and len(acc) >= 6:
        kind, code = struct.unpack_from('<BI', acc, 1)
        if kind == 0:
            outcome = (0, code)
        elif kind == 1:
            outcome = (1, code)
        else:
            failure = b'worker-fork'
    if relay_w is not None:
        try:
            os.close(relay_w)
        except OSError:
            pass
    if outcome is not None:
        killed = _kill_group(anchor, signal.SIGKILL, acc[:1] == b'R')
    else:
        _kill_group(anchor, signal.SIGTERM, acc[:1] == b'R')
        _sleep_grace(grace)
        killed = _kill_group(anchor, signal.SIGKILL, acc[:1] == b'R')
    if not killed:
        _note(nonce_hex, b'error kill-group')
        os._exit(FAULT)
    while True:
        try:
            os.waitpid(anchor, 0)
            break
        except InterruptedError:
            continue
        except OSError as exc:
            _note(nonce_hex, b'error reap-anchor ' + _clip(exc))
            os._exit(FAULT)
    if outcome is not None:
        if outcome[0] == 0:
            _note(nonce_hex, b'exit %d' % outcome[1])
            os._exit(outcome[1])
        _note(nonce_hex, b'signal %d' % outcome[1])
        code = 128 + outcome[1]
        os._exit(code if code < FAULT else FAULT - 1)
    if hangup or cancelled[0]:
        _note(nonce_hex, b'cancel')
        os._exit(CANCELLED)
    _note(nonce_hex, b'error ' + (failure or b'unknown'))
    os._exit(FAULT)

main()
os._exit(250)
"#;

/// POSIX single-quote escaping for one remote command argument.
///
/// OpenSSH remote command arguments are parsed by the remote login
/// shell, not passed as a native argv: the only safe way to carry a
/// literal string through that boundary is single quotes, with the one
/// impossible byte (the quote itself) spliced as `'\''`. UTF-8 executable
/// paths remain intact; native command argv stays base64 in the spec.
pub(super) fn shell_single_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for ch in text.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// The one line the remote login shell executes. The `exec` replaces
/// the shell, so the supervisor is sshd's direct child and no shell
/// lingers holding the pipes.
pub(super) fn command_line(
    encoded_spec: &str,
    python: &super::python::PythonInterpreter,
) -> String {
    python.bootstrap(SOURCE, encoded_spec)
}

/// Extract the nonce-marked supervisor records from captured stderr.
/// Only lines carrying this session's nonce count; a worker printing a
/// lookalike line without it is ignored (one that echoes the nonce is
/// not — an honesty limit, not a secret).
pub(super) fn records(stderr: &[u8], nonce_hex: &str) -> Vec<SupervisionOutcome> {
    let mut found = Vec::new();
    for line in stderr.split(|&byte| byte == b'\n') {
        let mut fields = line
            .split(u8::is_ascii_whitespace)
            .filter(|part| !part.is_empty());
        if fields.next() != Some(b"STROP-SUP-v1".as_slice()) {
            continue;
        }
        if fields.next() != Some(nonce_hex.as_bytes()) {
            continue;
        }
        let kind = match fields.next() {
            Some(kind) => kind,
            None => continue,
        };
        let outcome = match kind {
            b"exit" => fields
                .next()
                .and_then(number)
                .map(SupervisionOutcome::Exited),
            b"signal" => fields
                .next()
                .and_then(number)
                .map(SupervisionOutcome::Signaled),
            b"cancel" => Some(SupervisionOutcome::Cancelled),
            b"exec-error" => Some(SupervisionOutcome::LaunchFailure(rest(&mut fields))),
            b"error" => Some(SupervisionOutcome::SupervisorError(rest(&mut fields))),
            _ => None,
        };
        if let Some(outcome) = outcome {
            found.push(outcome);
        }
    }
    found
}

/// The remaining fields of a record line as one space-joined string.
fn rest<'a>(fields: &mut impl Iterator<Item = &'a [u8]>) -> String {
    let mut text = Vec::new();
    for field in fields {
        if !text.is_empty() {
            text.push(b' ');
        }
        text.extend_from_slice(field);
    }
    String::from_utf8_lossy(&text).into_owned()
}

fn number(field: &[u8]) -> Option<u32> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stderr_of(lines: &[&str]) -> Vec<u8> {
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<Vec<_>>()
            .concat()
            .into_bytes()
    }

    #[test]
    #[cfg(unix)]
    fn shell_quoting_survives_every_metacharacter() {
        let tricky = "it's \"quoted\" $(rm -rf /) `x` \\n; | & < > \t";
        let quoted = shell_single_quote(tricky);
        // Feed it through a real POSIX sh: what sh sees must be the original.
        let script = format!("printf %s {}", quoted);
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .output()
            .expect("sh runs");
        assert!(status.status.success());
        assert_eq!(String::from_utf8_lossy(&status.stdout), tricky);
    }

    #[test]
    fn records_accept_only_this_sessions_nonce() {
        let stderr = stderr_of(&[
            "STROP-SUP-v1 00112233445566778899aabbccddeeff exit 3",
            "STROP-SUP-v1 ffeeddccbbaa99887766554433221100 exit 4",
            "git: 'status' is not a git command",
        ]);
        let found = records(&stderr, "00112233445566778899aabbccddeeff");
        assert_eq!(found, vec![SupervisionOutcome::Exited(3)]);
    }

    #[test]
    fn records_keep_launch_and_cancel_kinds() {
        let stderr = stderr_of(&[
            "STROP-SUP-v1 n exec-error not-found b'git'",
            "STROP-SUP-v1 n exit 127",
        ]);
        let found = records(&stderr, "n");
        assert!(matches!(found[0], SupervisionOutcome::LaunchFailure(_)));
        assert_eq!(found[1], SupervisionOutcome::Exited(127));
        assert_eq!(
            records(&stderr_of(&["STROP-SUP-v1 n cancel"]), "n"),
            vec![SupervisionOutcome::Cancelled]
        );
        assert!(matches!(
            records(&stderr_of(&["STROP-SUP-v1 n error relay-write"]), "n")[0],
            SupervisionOutcome::SupervisorError(_)
        ));
    }
}
