//! Scripted inputs and native completions use the same channel and recording edge.
use super::directives::{self, DirectiveKind};
use crate::editor::{
    events::AppEvent,
    trace::{drive::Action, seed::Seed},
    Editor,
};
use ratatui::{backend::TestBackend, Terminal};
use std::io::{self, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

struct Driver<'a> {
    editor: &'a mut Editor,
    terminal: Terminal<TestBackend>,
    events: Receiver<AppEvent>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum WaitTarget {
    Delay,
    Jobs,
    Input,
}
impl WaitTarget {
    fn pending(self, editor: &Editor) -> bool {
        match self {
            Self::Delay => true,
            Self::Jobs => editor.async_pending(),
            Self::Input => editor.resolution.pending(),
        }
    }
}
impl Driver<'_> {
    fn apply(&mut self, action: Action) -> io::Result<()> {
        self.editor
            .recorded_action(action, self.editor.tape.sample_tick())?;
        self.editor.terminal_output.clear();
        Ok(())
    }
    fn drain(&mut self) -> io::Result<()> {
        let started = Instant::now();
        for _ in 0..crate::editor::events::EVENTS_PER_TURN {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            self.apply(Action::Event(event))?;
            if started.elapsed() >= crate::editor::events::TURN_BUDGET {
                break;
            }
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
    fn wait(&mut self, duration: Duration, target: WaitTarget, draw: bool) -> io::Result<()> {
        let deadline = Instant::now().checked_add(duration).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "wait duration exceeds the clock range",
            )
        })?;
        loop {
            self.drain()?;
            if draw {
                self.draw()?;
            }
            if target != WaitTarget::Delay && !target.pending(self.editor) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return if target != WaitTarget::Delay {
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
                Err(RecvTimeoutError::Timeout) => {
                    return if target != WaitTarget::Delay {
                        Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "editor jobs did not settle",
                        ))
                    } else {
                        Ok(())
                    };
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "editor event channel disconnected",
                    ));
                }
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
    open: Option<crate::editor::trace::drive::StartupOpen>,
) -> io::Result<()> {
    let mut steps = script
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .peekable();
    if let Some((DirectiveKind::Buffer, text)) = steps
        .peek()
        .map(|line| directives::parse(line))
        .transpose()?
    {
        let text: String = serde_json::from_str(text).map_err(io::Error::other)?;
        let configuration = std::mem::take(&mut editor.config);
        let cwd = editor.cwd.clone();
        let state_dir = editor.state_dir.take();
        let message = std::mem::take(&mut editor.message);
        *editor = Editor::new_in(strop_core::Buffer::from_text(&text), cwd);
        editor.config = configuration;
        editor.state_dir = state_dir;
        editor.message = message;
        steps.next();
    }
    editor.session_policy = crate::session::SessionPolicy::Disabled;
    if editor.tape.observes() {
        editor.tape.seed(&Seed::capture(editor)?)?;
    }
    let (tx, events) = crate::editor::events::channel();
    editor.connect_events(tx);
    let mut driver = Driver {
        editor,
        terminal: Terminal::new(TestBackend::new(cols, rows))?,
        events,
    };
    driver.apply(Action::Start {
        directory_picker: false,
        open,
    })?;
    driver.draw()?;
    for line in steps {
        if driver.editor.should_quit {
            break;
        }
        let (kind, arguments) = directives::parse(line)?;
        match kind {
            DirectiveKind::Buffer => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "buffer must be the first directive",
                ));
            }
            DirectiveKind::Keys => {
                for key in crate::editor::keys::parse(arguments) {
                    driver.input(AppEvent::Terminal(key))?;
                    if driver.editor.should_quit {
                        break;
                    }
                }
                if driver.editor.resolution.pending() {
                    driver.wait(Duration::from_secs(30), WaitTarget::Input, true)?;
                }
            }
            DirectiveKind::Key => {
                driver.input(AppEvent::Terminal(
                    serde_json::from_str(arguments).map_err(io::Error::other)?,
                ))?;
                if driver.editor.resolution.pending() {
                    driver.wait(Duration::from_secs(30), WaitTarget::Input, true)?;
                }
            }
            DirectiveKind::Paste => {
                driver.input(AppEvent::Paste(
                    serde_json::from_str(arguments).map_err(io::Error::other)?,
                ))?;
                if driver.editor.resolution.pending() {
                    driver.wait(Duration::from_secs(30), WaitTarget::Input, true)?;
                }
            }
            DirectiveKind::QuitIntent => driver.input(AppEvent::QuitIntent)?,
            DirectiveKind::Resize => {
                let dimensions = arguments
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<Vec<u16>, _>>()
                    .map_err(io::Error::other)?;
                let [columns, rows] = dimensions.as_slice() else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "resize requires columns and rows",
                    ));
                };
                driver.terminal.backend_mut().resize(*columns, *rows);
                driver.input(AppEvent::Resize {
                    columns: *columns,
                    rows: *rows,
                })?;
            }
            DirectiveKind::Settle => {
                driver.wait(
                    directives::duration(arguments, Some(Duration::from_secs(30)))?,
                    WaitTarget::Jobs,
                    true,
                )?;
            }
            DirectiveKind::Wait => {
                driver.wait(
                    directives::duration(arguments, None)?,
                    WaitTarget::Delay,
                    true,
                )?;
            }
            DirectiveKind::Frame => {
                driver.drain()?;
                driver.draw()?;
                let buffer = driver.terminal.backend().buffer();
                writeln!(
                    out,
                    "─── frame {}×{}",
                    buffer.area.width, buffer.area.height
                )?;
                for y in 0..buffer.area.height {
                    for symbol in super::row_symbols(buffer, y) {
                        write!(out, "{symbol}")?;
                    }
                    writeln!(out)?;
                }
            }
            DirectiveKind::State => {
                writeln!(out, "─── state {}", super::state_json(driver.editor))?
            }
        }
    }
    driver.apply(Action::Finish)?;
    driver.wait(Duration::from_secs(30), WaitTarget::Jobs, false)?;
    driver.editor.tape.finish()?;
    if let Some(error) = driver.editor.io.session_error.take() {
        return Err(io::Error::other(error));
    }
    Ok(())
}
