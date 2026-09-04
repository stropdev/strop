//! Shell escapes: `:!cmd` runs and displays, `|cmd` pipes a range
//! through and replaces it (helix's pipe is the better `!`). Every
//! spawn is a job posting onto the event loop (0001 §3) — the input
//! path never waits on a shell.

use strop_core::Buffer;

use super::{Editor, ShellResult};

impl Editor {
    /// `:!cmd`: run `sh -c cmd` in a job; the output buffer opens when
    /// the job lands.
    pub(crate) fn shell_run(&mut self, cmd: &str) {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            self.message = ":! needs a command".into();
            return;
        }
        let tx = self.shell_tx.clone();
        let cwd = self.cwd.clone();
        let job_cmd = cmd.clone();
        std::thread::spawn(move || {
            let output = run_shell(&job_cmd, &cwd, None);
            let _ = tx.send(ShellResult::Display {
                cmd: job_cmd,
                output,
            });
        });
        self.message = format!("sh: {cmd} …");
    }

    /// `|cmd` (visual) or `|cmd` on a normal line: pipe the range
    /// through the command; stdout replaces it (one undo unit).
    pub(crate) fn pipe_run(&mut self, start: usize, end: usize, cmd: &str) {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            self.message = "pipe: needs a command".into();
            return;
        }
        let buffer = self.current;
        let s = start.min(end).min(self.buf().len_bytes());
        let e = end.max(start).min(self.buf().len_bytes());
        let original = self.buf().rope.byte_slice(s..e).to_string();
        let tx = self.shell_tx.clone();
        let cwd = self.cwd.clone();
        let job_cmd = cmd.clone();
        std::thread::spawn(move || {
            let output = run_shell(&job_cmd, &cwd, Some(&original));
            let _ = tx.send(ShellResult::Pipe {
                buffer,
                start,
                end,
                original,
                output,
            });
        });
        self.message = format!("| {cmd} …");
    }

    /// Collect shell results (event-loop tick + headless settle).
    pub fn drain_shell(&mut self) {
        if self.buffers.is_empty() {
            return;
        }
        while let Ok(result) = self.shell_rx.try_recv() {
            match result {
                ShellResult::Display { cmd, output } => {
                    let mut buf = Buffer::from_text(&output);
                    buf.readonly = true;
                    buf.name = Some(format!("sh: {cmd}"));
                    self.buffers.push(buf);
                    self.surfaces.push(None);
                    self.highlighters.push(None);
                    self.current = self.buffers.len() - 1;
                    self.touch_mru(self.current);
                    self.cursor = 0;
                    self.view_top = 0;
                    self.message = format!("sh: {cmd} — q closes");
                }
                ShellResult::Pipe {
                    buffer,
                    start,
                    end,
                    original,
                    output,
                } => {
                    let Some(buf) = self.buffers.get_mut(buffer) else {
                        self.message = "pipe: buffer is gone".into();
                        continue;
                    };
                    if buf.readonly {
                        self.message = "pipe: readonly buffer".into();
                        continue;
                    }
                    // never clobber: the range must still hold what we piped
                    let (s, e) = (start.min(end), end.max(start));
                    let s = s.min(buf.len_bytes());
                    let e = e.min(buf.len_bytes()).max(s);
                    if buf.rope.byte_slice(s..e) != original {
                        self.message = "pipe: text changed under the job — skipped".into();
                        continue;
                    }
                    // linewise ranges keep their newline; charwise gets
                    // the command's trailing newline trimmed
                    let out = if original.ends_with('\n') {
                        output
                    } else {
                        output.strip_suffix('\n').unwrap_or(&output).to_string()
                    };
                    buf.history.begin();
                    buf.delete(strop_core::Range::charwise(s, e));
                    buf.insert(s, &out);
                    buf.history.commit();
                    if buffer == self.current {
                        self.cursor = self.buf().clamp_boundary(s);
                        self.clamp_cursor();
                        self.flash(strop_core::Range::charwise(self.cursor, self.cursor));
                    }
                    self.message = "piped".into();
                }
            }
        }
    }
}

/// Run `sh -c cmd` with optional stdin; returns stdout + stderr (a
/// failed spawn reports itself as output — jobs never panic the loop).
fn run_shell(cmd: &str, cwd: &std::path::Path, stdin_text: Option<&str>) -> String {
    let mut child = match std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("spawn failed: {e}"),
    };
    if let Some(text) = stdin_text {
        use std::io::Write;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
    }
    match child.wait_with_output() {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).to_string();
            if !out.stderr.is_empty() {
                if !text.is_empty() {
                    text.push_str("\n--- stderr ---\n");
                }
                text.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            text
        }
        Err(e) => format!("wait failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bang_opens_output_buffer() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":!echo hello\r");
        // the job is async; deliver it the way the loop would
        for _ in 0..100 {
            e.drain_shell();
            if e.buf().name.as_deref() == Some("sh: echo hello") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(e.buf().name.as_deref(), Some("sh: echo hello"));
        assert!(e.buf().rope.to_string().contains("hello"));
        assert!(e.buf().readonly);
        e.feed_text("q"); // closes like any readonly buffer
        assert_eq!(e.buf().name.as_deref(), None);
    }

    #[test]
    fn visual_pipe_replaces_selection() {
        let mut e = Editor::new(Buffer::from_text("beta\nalpha\n"));
        e.feed_text("Vj"); // select both lines
        e.feed_text("|sort");
        e.feed(crate::editor::Key::Enter);
        for _ in 0..100 {
            e.drain_shell();
            if e.buf().rope == "alpha\nbeta\n" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(e.buf().rope.to_string(), "alpha\nbeta\n");
        // one undo unit for the whole pipe
        e.feed_text("u");
        assert_eq!(e.buf().rope.to_string(), "beta\nalpha\n");
    }
}
