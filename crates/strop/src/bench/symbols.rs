//! The symbols budgets (0099 §7). Like the find scenarios, this one
//! fails: an operation past its budget is an error.
//!
//! A generated Rust file of 15,400 lines and 2,800 declarations, with no
//! language server — the syntax outline, so the scenario is hermetic. It
//! is driven through the recorded-action path the terminal and the
//! headless driver use, because that is where the after-event work that
//! fetches symbols ahead of the key runs.

use std::io;
use std::time::{Duration, Instant};

use strop_core::frontend_input::Key;
use strop_core::Buffer;
use strop_picker::Kind;

use super::{Drive, Fixture, Op, SETTLE};
use crate::editor::events::AppEvent;
use crate::editor::io::OpenIntent;
use crate::editor::trace::drive::Action;
use crate::editor::Editor;

const OPEN_KEPT: Duration = Duration::from_millis(10);
const KEY_DURING_BUILD: Duration = Duration::from_millis(5);

/// 700 × (a struct, an impl with two methods, a free function).
fn source() -> String {
    let mut text = String::new();
    for index in 0..700 {
        text.push_str(&format!(
            "pub struct Thing{index} {{\n    pub a: u32,\n    pub b: String,\n}}\n\n\
             impl Thing{index} {{\n    pub fn new(a: u32) -> Self {{\n        Self {{ a, b: String::new() }}\n    }}\n    \
             pub fn total(&self, x: u32) -> u32 {{\n        let mut sum = self.a;\n        for k in 0..x {{\n            \
             sum = sum.wrapping_add(k * {index});\n        }}\n        sum\n    }}\n}}\n\n\
             pub fn free_{index}(x: u32) -> u32 {{\n    x + {index}\n}}\n\n"
        ));
    }
    text
}

/// Every event goes through the recorded-action path, the one the
/// terminal loop uses: the after-event work runs for job results too,
/// not only for keys.
fn apply(drive: &mut Drive, event: AppEvent) -> io::Result<()> {
    let tick = drive.editor.tape().sample_tick();
    drive.editor.recorded_action(Action::Event(event), tick)
}

/// One key: the event, whatever is already queued, a frame.
fn turn(drive: &mut Drive, key: Key) -> io::Result<()> {
    apply(drive, AppEvent::EditorKey(key))?;
    while let Ok(event) = drive.events.try_recv() {
        apply(drive, event)?;
    }
    drive.draw()
}

/// Handle events as they arrive until `done`; parks on the channel.
fn pump(drive: &mut Drive, done: impl Fn(&Editor) -> bool) -> io::Result<()> {
    let deadline = Instant::now() + SETTLE;
    loop {
        while let Ok(event) = drive.events.try_recv() {
            apply(drive, event)?;
        }
        drive.draw()?;
        if done(&drive.editor) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        match drive.events.recv_timeout(remaining) {
            Ok(event) => apply(drive, event)?,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the symbols scenario did not settle",
                ))
            }
        }
    }
}

fn settled(editor: &Editor) -> bool {
    !editor.async_pending()
}

fn symbols_listed(editor: &Editor) -> bool {
    editor.picker().is_some_and(|glue| {
        glue.picker.kind == Kind::Symbols
            && glue.rank_pending.is_none()
            && !glue.picker.rows.is_empty()
    })
}

pub(super) fn run() -> io::Result<()> {
    let fixture = Fixture::new("symbols")?;
    let path = fixture.write("big.rs", &source())?;
    let mut drive = Drive::new(
        Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
        120,
        40,
    )?;
    drive
        .editor
        .request_open(path, OpenIntent::Switch { readonly: false });
    drive.settle(SETTLE)?;

    let mut build = Op::new("build");
    let mut open_kept = Op::new("open_kept");
    let mut open_after_edit = Op::new("open_edited");
    let mut key_during = Op::new("key_in_build");

    // the first turn after the open starts the build
    build.try_time(|| {
        turn(&mut drive, Key::Esc)?;
        pump(&mut drive, settled)
    })?;

    for _ in 0..20 {
        let started = Instant::now();
        turn(&mut drive, Key::Char(' '))?;
        turn(&mut drive, Key::Char('s'))?;
        pump(&mut drive, symbols_listed)?;
        open_kept.samples.push(started.elapsed());
        pump(&mut drive, settled)?;
        turn(&mut drive, Key::Esc)?;
        turn(&mut drive, Key::Esc)?;
        if drive.editor.picker().is_some() {
            return Err(io::Error::other("the symbols picker did not close"));
        }
    }

    for _ in 0..10 {
        // an edit: the kept list is for another revision now, and the
        // turn that ends the edit starts a new build
        turn(&mut drive, Key::Char('x'))?;
        // (a key can be deferred behind path resolution; wait for the
        // build it causes rather than assuming the same turn)
        pump(&mut drive, |editor| editor.async_pending())?;
        // a key while that build runs must not wait for it
        key_during.try_time(|| turn(&mut drive, Key::Char('l')))?;
        if !drive.editor.async_pending() {
            return Err(io::Error::other(
                "the build ended before the key that was to overlap it",
            ));
        }
        // a press now adopts the build: rows when it lands
        let started = Instant::now();
        turn(&mut drive, Key::Char(' '))?;
        turn(&mut drive, Key::Char('s'))?;
        pump(&mut drive, symbols_listed)?;
        open_after_edit.samples.push(started.elapsed());
        pump(&mut drive, settled)?;
        turn(&mut drive, Key::Esc)?;
        turn(&mut drive, Key::Esc)?;
    }

    println!("symbols (15,400 lines, 2,800 declarations, syntax outline)");
    for op in [&build, &open_kept, &key_during, &open_after_edit] {
        op.report();
    }
    super::find::within(&open_kept, OPEN_KEPT)?;
    super::find::within(&key_during, KEY_DURING_BUILD)
}
