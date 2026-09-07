//! Scripted inputs and native completions use the same channel and recording edge.
use crate::editor::{
    events::AppEvent,
    trace::{drive::Action, seed::Seed},
    Editor,
};
use ratatui::{backend::TestBackend, Terminal};
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

struct Driver<'a> {
    editor: &'a mut Editor,
    terminal: Terminal<TestBackend>,
    events: Receiver<AppEvent>,
}
impl Driver<'_> {
    fn apply(&mut self, action: Action) -> io::Result<()> {
        self.editor
            .recorded_action(action, self.editor.tape.sample_tick())?;
        self.editor.terminal_output.clear();
        Ok(())
    }
    fn drain(&mut self) -> io::Result<()> {
        while let Ok(event) = self.events.try_recv() {
            self.apply(Action::Event(event))?;
        }
        Ok(())
    }
    fn draw(&mut self) -> io::Result<()> {
        if !self.editor.should_quit && !self.editor.docs.is_empty() {
            if std::mem::take(&mut self.editor.needs_repaint) {
                self.terminal.clear()?;
            }
            self.terminal
                .draw(|frame| crate::editor::trace::frame::draw(self.editor, frame, true))?;
        }
        self.editor.tape.healthy()
    }
    fn input(&mut self, event: AppEvent) -> io::Result<()> {
        self.apply(Action::Event(event))?;
        self.drain()?;
        self.draw()
    }
    fn wait(&mut self, duration: Duration, until_idle: bool, draw: bool) -> io::Result<()> {
        let deadline = Instant::now() + duration;
        loop {
            self.drain()?;
            if draw {
                self.draw()?;
            }
            if until_idle && !self.editor.async_pending() {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return if until_idle {
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "editor jobs did not settle",
                    ))
                } else {
                    Ok(())
                };
            }
            match self.events.recv_timeout(remaining) {
                Ok(event) => self.apply(Action::Event(event))?,
                Err(RecvTimeoutError::Timeout) if !until_idle => return Ok(()),
                Err(error) => return Err(io::Error::other(error)),
            }
        }
    }
}

pub fn run_script(
    editor: &mut Editor,
    script: &str,
    cols: u16,
    rows: u16,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut steps = script
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .peekable();
    if let Some(text) = steps.peek().and_then(|line| line.strip_prefix("buffer ")) {
        let text: String = serde_json::from_str(text).map_err(io::Error::other)?;
        let configuration = std::mem::take(&mut editor.config);
        let cwd = editor.cwd.clone();
        *editor = Editor::new_in(strop_core::Buffer::from_text(&text), cwd);
        editor.config = configuration;
        steps.next();
    }
    if editor.tape.observes() {
        editor.tape.seed(&Seed::capture(editor)?)?;
    }
    let (tx, events) = mpsc::channel();
    editor.connect_events(tx);
    let mut driver = Driver {
        editor,
        terminal: Terminal::new(TestBackend::new(cols, rows))?,
        events,
    };
    driver.apply(Action::Start {
        directory_picker: false,
    })?;
    driver.draw()?;
    for line in steps {
        if driver.editor.should_quit {
            break;
        }
        if let Some(keys) = line.strip_prefix("keys ") {
            for key in crate::editor::keys::parse(keys) {
                driver.input(AppEvent::Terminal(key))?;
                if driver.editor.should_quit {
                    break;
                }
            }
        } else if let Some(key) = line.strip_prefix("key ") {
            driver.input(AppEvent::Terminal(
                serde_json::from_str(key).map_err(io::Error::other)?,
            ))?;
        } else if let Some(text) = line.strip_prefix("paste ") {
            driver.input(AppEvent::Paste(
                serde_json::from_str(text).map_err(io::Error::other)?,
            ))?;
        } else if line == "quit-intent" {
            driver.input(AppEvent::QuitIntent)?;
        } else if let Some(size) = line.strip_prefix("resize ") {
            let dimensions: Result<Vec<u16>, _> = size.split_whitespace().map(str::parse).collect();
            let dimensions = dimensions.map_err(io::Error::other)?;
            let [columns, rows] = dimensions.as_slice() else {
                return Err(io::Error::other("resize requires columns and rows"));
            };
            driver.terminal.backend_mut().resize(*columns, *rows);
            driver.input(AppEvent::Resize {
                columns: *columns,
                rows: *rows,
            })?;
        } else if line == "settle" {
            driver.wait(Duration::from_secs(30), true, true)?;
        } else if let Some(duration) = line.strip_prefix("wait ") {
            driver.wait(
                Duration::from_millis(duration.trim().parse().map_err(io::Error::other)?),
                false,
                true,
            )?;
        } else if line == "frame" {
            driver.drain()?;
            driver.draw()?;
            let buffer = driver.terminal.backend().buffer();
            writeln!(
                out,
                "─── frame {}×{}",
                buffer.area.width, buffer.area.height
            )?;
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    write!(out, "{}", buffer[(x, y)].symbol())?;
                }
                writeln!(out)?;
            }
        } else if line == "state" {
            writeln!(out, "─── state {}", super::state_json(driver.editor))?;
        } else {
            return Err(io::Error::other(format!("unknown script command: {line}")));
        }
    }
    driver.apply(Action::Finish)?;
    driver.wait(Duration::from_secs(30), true, false)?;
    driver.editor.tape.finish()?;
    if let Some(error) = driver.editor.io.session_error.take() {
        return Err(io::Error::other(error));
    }
    Ok(())
}
