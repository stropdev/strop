use std::{
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub(crate) struct Tui {
    pub(crate) child: Child,
    pub(crate) master: File,
    pub(crate) screen: vt100::Parser,
    pub(crate) trace: std::path::PathBuf,
}
impl Tui {
    pub(crate) fn start(directory: &std::path::Path, trace: &std::path::Path) -> Self {
        Self::spawn(
            directory,
            trace,
            std::path::Path::new(env!("CARGO_BIN_EXE_strop")),
            true,
        )
    }

    pub(crate) fn spawn(
        directory: &std::path::Path,
        trace: &std::path::Path,
        binary: &std::path::Path,
        capture: bool,
    ) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let mut size = std::mem::MaybeUninit::new(libc::winsize {
            ws_row: 30,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        });
        // SAFETY: valid output pointers and an initialized, writable
        // winsize for Linux's const and macOS's mutable openpty APIs.
        // This dedicated integration binary has no concurrent fork.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut::<libc::termios>(),
                    size.as_mut_ptr(),
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
        let mut command = Command::new(binary);
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
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        if capture {
            command
                .args(["--log-file"])
                .arg(trace)
                .arg("--log-terminal-content");
        }
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
    pub(crate) fn recent_trace(&self) -> String {
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

    /// This fixture explicitly opted into terminal content capture.
    /// Show only consented key actions on a byte mismatch; source input
    /// stays private in ordinary editor sessions.
    pub(crate) fn recent_input_trace(&self) -> String {
        std::fs::read_to_string(&self.trace)
            .unwrap_or_default()
            .lines()
            .rev()
            .filter(|line| line.contains("\"kind\":\"action\"") && line.contains("\"Input\""))
            .take(24)
            .map(|line| line.chars().take(700).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn poll(&self, events: libc::c_short, deadline: Instant) {
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
            "terminal poll timed out:\n{}\nrecent trace:\n{}\nrecent input:\n{}",
            self.screen.screen().contents(),
            self.recent_trace(),
            self.recent_input_trace()
        );
    }
    /// Soft poll: false on deadline instead of asserting (retry loops).
    pub(crate) fn poll_soft(&self, events: libc::c_short, deadline: Instant) -> bool {
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
    pub(crate) fn until_soft(
        &mut self,
        budget: Duration,
        predicate: impl Fn(&str) -> bool,
    ) -> Option<String> {
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
                // Linux reports EIO (rather than EOF) after the last
                // PTY slave closes. This soft wait treats a closed
                // output stream as completion; wait_exit still checks
                // the child's actual exit status separately.
                Err(error) if error.raw_os_error() == Some(libc::EIO) => return None,
                Err(error) => panic!("terminal read: {error}"),
            }
        }
    }
    /// Reap only this fixture's editor. Keep draining its PTY while
    /// waiting, so the child cannot block on a full output pipe;
    /// expiry names the visible owner and recent consented trace.
    pub(crate) fn wait_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "terminal editor did not exit: {}\nrecent trace:\n{}",
                self.screen.screen().contents(),
                self.recent_trace()
            );
            let _ = self.until_soft(Duration::from_millis(16), |_| false);
        }
    }

    pub(crate) fn send(&mut self, mut bytes: &[u8]) {
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
    pub(crate) fn until(&mut self, predicate: impl Fn(&str) -> bool) -> String {
        // This session carries a consented terminal capture throughout:
        // per-update frame records and snapshot searches legitimately cost
        // seconds on slow debug runners. A minute still distinguishes a
        // slow drain from a hang.
        self.until_within(Duration::from_secs(60), predicate)
    }
    /// Slow runners process a consented capture's flood of full-frame
    /// records on the editor thread; a minute still distinguishes a drain
    /// from a hang.
    pub(crate) fn until_within(
        &mut self,
        budget: Duration,
        predicate: impl Fn(&str) -> bool,
    ) -> String {
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
pub(crate) fn numbered_line(screen: &str, expected: &str) -> bool {
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
pub(crate) fn strip_track(row: &str) -> &str {
    let trimmed = row.trim();
    trimmed
        .strip_suffix(['\u{2502}', '\u{25ae}', '\u{258e}'])
        .map_or(trimmed, str::trim_end)
}
pub(crate) fn line(screen: &str, expected: &str) -> bool {
    screen.lines().any(|line| strip_track(line) == expected)
}
/// The text of one pane column range (0064 §1 geometry-aware split
/// checks; the divider and track glyphs stay out of the slice).
pub(crate) fn cells(row: &str, from: usize, to: usize) -> String {
    row.chars().skip(from).take(to - from).collect::<String>()
}
