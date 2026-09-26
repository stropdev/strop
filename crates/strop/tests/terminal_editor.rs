#![cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Tui {
    child: Child,
    master: File,
    screen: vt100::Parser,
    trace: std::path::PathBuf,
}
impl Tui {
    fn start(directory: &std::path::Path, trace: &std::path::Path) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let size = libc::winsize {
            ws_row: 30,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: valid output pointers and winsize. This dedicated integration
        // binary has one test/launcher; no concurrent fork can inherit the pair.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            },
            0
        );
        // SAFETY: successful openpty returned two uniquely owned descriptors.
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            // SAFETY: owned descriptors stay open through configuration.
            assert_ne!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                -1
            );
        }
        // SAFETY: the owned master remains live; nonblocking I/O uses poll below.
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_ne!(flags, -1);
        assert_ne!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            -1
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_strop"));
        command
            .env_clear()
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
            )
            .current_dir(directory)
            .env("HOME", directory)
            .env("XDG_CONFIG_HOME", directory.join("config"))
            .env("XDG_STATE_HOME", directory.join("state"))
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm-256color")
            .env_remove("STROP_LOG")
            .args(["--log-file"])
            .arg(trace)
            .arg("--log-terminal-content")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: pre_exec performs only async-signal-safe session/tty syscalls;
        // stdin is the child-owned duplicate of this test's PTY slave.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Self {
            child: command.spawn().unwrap(),
            master,
            screen: vt100::Parser::new(30, 120, 0),
            trace: trace.to_path_buf(),
        }
    }
    fn recent_trace(&self) -> String {
        std::fs::read_to_string(&self.trace)
            .unwrap_or_default()
            .lines()
            .rev()
            .filter(|line| {
                line.contains("terminal.update")
                    || line.contains("terminal.start")
                    || line.contains("\"event\":\"error\"")
                    || line.contains("\"event\":\"panic\"")
            })
            .map(|line| line.chars().take(1200).collect::<String>())
            .take(12)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn poll(&self, events: libc::c_short, deadline: Instant) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "terminal deadline");
        let mut descriptor = libc::pollfd {
            fd: self.master.as_raw_fd(),
            events,
            revents: 0,
        };
        // SAFETY: owned descriptor and initialized writable pollfd storage.
        let ready = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                left.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        assert!(
            ready > 0 || io::Error::last_os_error().kind() == io::ErrorKind::Interrupted,
            "terminal poll timed out:\n{}\nrecent trace:\n{}",
            self.screen.screen().contents(),
            self.recent_trace()
        );
    }
    /// Soft poll: false on deadline instead of asserting (retry loops).
    fn poll_soft(&self, events: libc::c_short, deadline: Instant) -> bool {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        let mut descriptor = libc::pollfd {
            fd: self.master.as_raw_fd(),
            events,
            revents: 0,
        };
        // SAFETY: owned descriptor and initialized writable pollfd storage.
        let ready = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                left.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        ready > 0
    }
    /// Soft wait: None on budget expiry instead of asserting (retry loops).
    fn until_soft(&mut self, budget: Duration, predicate: impl Fn(&str) -> bool) -> Option<String> {
        let deadline = Instant::now() + budget;
        loop {
            let screen = self.screen.screen().contents();
            if predicate(&screen) {
                return Some(screen);
            }
            if Instant::now() >= deadline {
                return None;
            }
            let mut bytes = [0; 8192];
            match self.master.read(&mut bytes) {
                Ok(0) => return None,
                Ok(count) => self.screen.process(&bytes[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !self.poll_soft(libc::POLLIN, deadline) {
                        return None;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => panic!("terminal read: {error}"),
            }
        }
    }
    fn send(&mut self, mut bytes: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !bytes.is_empty() {
            match self.master.write(bytes) {
                Ok(0) => panic!("terminal write closed"),
                Ok(count) => bytes = &bytes[count..],
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.poll(libc::POLLOUT, deadline)
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => panic!("terminal write: {error}"),
            }
        }
    }
    fn until(&mut self, predicate: impl Fn(&str) -> bool) -> String {
        // This session carries a consented terminal capture throughout:
        // per-update frame records and snapshot searches legitimately cost
        // seconds on slow debug runners. A minute still distinguishes a
        // slow drain from a hang.
        self.until_within(Duration::from_secs(60), predicate)
    }
    /// Slow runners process a consented capture's flood of full-frame
    /// records on the editor thread; a minute still distinguishes a drain
    /// from a hang.
    fn until_within(&mut self, budget: Duration, predicate: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + budget;
        loop {
            let screen = self.screen.screen().contents();
            if predicate(&screen) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "terminal condition timed out: {screen}"
            );
            let mut bytes = [0; 8192];
            match self.master.read(&mut bytes) {
                Ok(0) => panic!("terminal read closed: {screen}"),
                Ok(count) => self.screen.process(&bytes[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.poll(libc::POLLIN, deadline)
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => panic!("terminal read: {error}: {screen}"),
            }
        }
    }
}

/// Inspection-mode rows carry the numbered gutter (0065): match the
/// post-number text, the same shape the split-geometry checks use.
fn numbered_line(screen: &str, expected: &str) -> bool {
    screen.lines().any(|row| {
        strip_track(row)
            .trim_start()
            .split_once(' ')
            .is_some_and(|(number, text)| {
                number.bytes().all(|byte| byte.is_ascii_digit()) && text.trim() == expected
            })
    })
}
impl Drop for Tui {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
/// 0064 §1: every pane reserves its last column for the scrollbar
/// track (`│`/`▮`/`▎`), so full-row comparisons ignore that cell.
fn strip_track(row: &str) -> &str {
    let trimmed = row.trim();
    trimmed
        .strip_suffix(['\u{2502}', '\u{25ae}', '\u{258e}'])
        .map_or(trimmed, str::trim_end)
}
fn line(screen: &str, expected: &str) -> bool {
    screen.lines().any(|line| strip_track(line) == expected)
}
/// The text of one pane column range (0064 §1 geometry-aware split
/// checks; the divider and track glyphs stay out of the slice).
fn cells(row: &str, from: usize, to: usize) -> String {
    row.chars().skip(from).take(to - from).collect::<String>()
}

#[test]
fn real_terminal_input_consent_quit_and_execution_free_replay() {
    let directory = tempfile::tempdir().unwrap();
    let trace = directory.path().join("terminal.jsonl");
    std::fs::write(
        directory.path().join("pager.txt"),
        (0..100)
            .map(|index| format!("PAGER-LINE-{index:03}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let mut tui = Tui::start(directory.path(), &trace);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":terminal env PS1='STROP-PTY> ' /bin/sh -i\r");
    tui.until(|screen| screen.contains("TERMINAL") && screen.contains("STROP-PTY>"));
    tui.send(b"printf x >> run-count; printf 'PTY-INPUT-OK\\n'\r");
    tui.until(|screen| line(screen, "PTY-INPUT-OK"));
    tui.send(b"printf '\\033[?2004lPASTE-READY\\n'; read first; read second; printf 'PASTE:%s|%s\\n' \"$first\" \"$second\"\r");
    tui.until(|screen| line(screen, "PASTE-READY"));
    tui.send(b"\x1b[200~alpha\nbeta\n\x1b[201~");
    let held = tui.until(|screen| screen.contains("paste held"));
    assert!(!line(&held, "alpha") && !line(&held, "beta"));
    tui.send(b"\x1c\x0e");
    tui.until(|screen| screen.contains("NORMAL") && screen.contains("snapshot"));
    tui.send(b":terminal-paste\r");
    tui.until(|screen| line(screen, "PASTE:alpha|beta") && screen.contains("TERMINAL"));
    tui.send(b"nvim --clean nested.txt\r");
    tui.until(|screen| {
        screen.contains("nested.txt") && screen.lines().any(|row| row.starts_with('~'))
    });
    tui.send(b"iNESTED-EDITOR-OK");
    tui.until(|screen| screen.contains("-- INSERT --"));
    tui.send(b"\x1b");
    tui.until(|screen| !screen.contains("-- INSERT --") && screen.contains("NESTED-EDITOR-OK"));
    tui.send(b":wq\r");
    tui.until(|screen| screen.contains("STROP-PTY>") && screen.contains("TERMINAL"));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("nested.txt")).unwrap(),
        "NESTED-EDITOR-OK\n"
    );
    tui.send(b"less pager.txt\r");
    tui.until(|screen| {
        screen.contains("PAGER-LINE-000")
            && screen.contains("pager.txt")
            && !screen.contains("STROP-PTY>")
    });
    tui.send(b"q");
    tui.until(|screen| screen.contains("STROP-PTY>") && screen.contains("TERMINAL"));
    tui.send(b"printf '\\033[2J\\033[HINTERRUPT-READY\\n'; cat\r");
    tui.until(|screen| line(screen, "INTERRUPT-READY"));
    tui.send(b"\x03");
    tui.until(|screen| screen.contains("STROP-PTY>") && screen.contains("TERMINAL"));
    // (raw Esc/Alt byte fidelity is asserted exactly by the od step below)
    tui.send(b"printf 'ESC-REMAINED\\n'\r");
    tui.send(b"stty raw -echo; printf 'BYTE-READY\\n'; dd bs=1 count=8 2>/dev/null | od -An -tx1; stty sane; printf '\\r\\nBYTE-DONE\\n'\r");
    tui.until(|screen| line(screen, "BYTE-READY"));
    tui.send(b"\x12\x1bx\x1b[15~");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "12 1b 78 1b 5b 31 35 7e"
        })
    });
    tui.until(|screen| line(screen, "BYTE-DONE"));
    // The prefix grammar's pass-through contract (0055 §12, literal
    // escape-prefix recovery): `Ctrl-W .` delivers the literal 0x17, and a
    // nonmatching `Ctrl-\` or `Ctrl-W` follow-up forwards the prefix byte
    // plus the key, in order — nothing the user typed may disappear.
    tui.send(b"stty raw -echo; printf 'PREFIX-READY\\n'; dd bs=1 count=5 2>/dev/null | od -An -tx1; stty sane; printf '\\r\\nPREFIX-DONE\\n'\r");
    tui.until(|screen| line(screen, "PREFIX-READY"));
    tui.send(b"\x17.\x1cz\x17z");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "17 1c 7a 17 7a"
        })
    });
    tui.until(|screen| line(screen, "PREFIX-DONE"));
    // Application versus normal cursor-key mode (0055 §12): the child's
    // DECCKM toggle re-aims the same Up key through the mode-aware
    // encoder — SS3 (ESC O A) in application mode, CSI (ESC [ A) after
    // the reset. Byte-asserted, not assumed from the engine choice.
    tui.send(b"stty raw -echo; printf '\\033[?1hMODE-APP\\n'; dd bs=1 count=3 2>/dev/null | od -An -tx1; printf '\\033[?1lMODE-NORM\\n'; dd bs=1 count=3 2>/dev/null | od -An -tx1; stty sane; printf '\\r\\nMODE-DONE\\n'\r");
    tui.until(|screen| line(screen, "MODE-APP"));
    tui.send(b"\x1b[A");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "1b 4f 41"
        })
    });
    tui.until(|screen| line(screen, "MODE-NORM"));
    tui.send(b"\x1b[A");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "1b 5b 41"
        })
    });
    tui.until(|screen| line(screen, "MODE-DONE"));
    // Sustained output (0055 §12): while a sixteen-KiB `yes` stream floods
    // the PTY, the editor still admits input and renders — the mode escape
    // lands mid-flood — and the bounded history drops the flood's head:
    // FLOOD-START cannot survive, FLOOD-DONE does, and the shell prompt
    // returns. Markers are line-anchored so the echoed command cannot
    // satisfy them. The session must ALSO stay fully capturable: frames
    // travel as cell runs, so the flood cannot exhaust the capture bound
    // and degrade the trace.
    tui.send(b"printf 'FLOOD-START\\n'; yes | head -c 16384; printf '\\r\\nFLOOD-DONE\\n'\r");
    tui.send(b"\x1c\x0e");
    // The capture's per-update frame records queue ahead of this escape on
    // slow runners; the escape still lands in order — the mid-flood input
    // contract. The pinned-view proof is the mode chip: the transient
    // "snapshot" message is cleared by the very next update while the
    // flood streams.
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("TERMINAL"));
    tui.send(b"i");
    let settled = tui.until(|screen| line(screen, "FLOOD-DONE") && screen.contains("STROP-PTY>"));
    assert!(
        !settled.contains("FLOOD-START"),
        "bounded history must drop the flood head"
    );
    tui.send(b"mkfifo pause; (exec 3<>pause; printf '\\r\\nINSPECTION-READY\\n'; read go <&3; printf '\\r\\nASYNC-INSPECTION-OUTPUT\\n') &\r");
    tui.until(|screen| line(screen, "INSPECTION-READY"));
    tui.send(b"\x1c\x0e");
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("TERMINAL"));
    File::options()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(directory.path().join("pause"))
        .unwrap()
        .write_all(b"go\n")
        .unwrap();
    let frozen = tui.until(|screen| screen.contains("new output"));
    assert!(!line(&frozen, "ASYNC-INSPECTION-OUTPUT"));
    // The pinned view never drags to new output (0065): refresh installs
    // the latest published frame, and a search inside the pinned buffer
    // is the honest probe that the deferred line landed — the caret
    // follows the match, revealing it. Frames may still be arriving at
    // the first refresh, so iterate the documented signal→refresh loop
    // like a user, bounded.
    let mut installed = false;
    for _ in 0..5 {
        tui.send(b":terminal-refresh\r");
        tui.until(|s| s.contains("refreshed") || s.contains("already shows"));
        tui.send(b"/ASYNC-INSPECTION-OUTPUT\r");
        if tui
            .until_soft(Duration::from_secs(5), |s| {
                numbered_line(s, "ASYNC-INSPECTION-OUTPUT")
            })
            .is_some()
        {
            installed = true;
            break;
        }
        // The refreshed frame predated the deferred line; let the
        // remaining frames settle, then refresh again.
        let _ = tui.until_soft(Duration::from_secs(2), |_| false);
    }
    assert!(
        installed,
        "terminal-refresh installs the deferred output into the pinned view"
    );
    tui.send(b"gg/^INSPECTION-READY\ryy:e yank-target.txt\r");
    tui.until(|screen| screen.contains("yank-target.txt") && !screen.contains("terminal #"));
    tui.send(b"p");
    tui.until(|screen| !screen.contains("terminal #") && numbered_line(screen, "INSPECTION-READY"));
    tui.send(b":q!\r");
    tui.until(|screen| screen.contains("NORMAL") && screen.contains("terminal #"));
    tui.send(b":vs\ri");
    // 0064 §1 geometry: left pane cols 0..58, its track 58, divider 59,
    // right pane content 60..119, right track 119 — compare by range,
    // never by the divider glyph (the track shares it).
    let split = tui.until(|screen| {
        screen
            .lines()
            .any(|row| cells(row, 60, 119).trim() == "ASYNC-INSPECTION-OUTPUT")
    });
    assert!(!split
        .lines()
        .any(|row| cells(row, 0, 58).trim() == "ASYNC-INSPECTION-OUTPUT"));
    // From terminal input, the t_CTRL-W grammar moves panes without the
    // child seeing a byte: focus lands on the left editor pane (NORMAL),
    // then returns to the terminal pane (TERMINAL) still owning input.
    tui.send(b"\x17l");
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("TERMINAL"));
    tui.send(b"\x17h");
    tui.until(|screen| screen.contains("TERMINAL"));
    tui.send(b"printf 'SIZE:'; stty size\r");
    tui.until(|screen| {
        // The 60-column right pane reserves one track column (0064 §1),
        // so the child's grid is 28x59.
        screen
            .lines()
            .any(|row| cells(row, 60, 119).trim() == "SIZE:28 59")
    });
    tui.send(b"\x1c\x0e:q\ri");
    tui.until(|screen| {
        // split closed: no divider remains; the only reserved cell is
        // the single pane's own track at the last column (0064 §1).
        screen.contains("TERMINAL")
            && !screen
                .lines()
                .take(29)
                .any(|row| row.chars().take(119).any(|cell| cell == '\u{2502}'))
    });
    tui.send(b"\x1c\x0e:qa\r");
    tui.until(|screen| screen.contains("terminal sessions are running"));
    tui.send(b":terminal-stop\r");
    tui.until(|screen| screen.contains("terminal ended") || screen.contains("terminal exited"));
    tui.send(b":qa\r");
    assert!(tui.child.wait().unwrap().success());
    // The flood never degraded the capture: the file says so itself.
    let terminal = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .last()
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .unwrap_or_default();
    assert_eq!(
        terminal["event"], "trace_end",
        "trace must end with its marker"
    );
    assert_eq!(
        terminal["fields"]["complete"], true,
        "flooded capture stayed complete: {terminal}"
    );
    let before = std::fs::read(directory.path().join("run-count")).unwrap();
    assert_eq!(before, b"x");
    let replay = Command::new(env!("CARGO_BIN_EXE_strop"))
        .arg("--replay")
        .arg(&trace)
        .env("HOME", directory.path())
        .env("SHELL", "/unavailable-during-replay")
        .env_remove("STROP_LOG")
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("run-count")).unwrap(),
        before
    );
}
