//! The structural-indent budget (#309): `o` and `Enter` in a 10 MB Rust
//! file, at its top, middle and end, with the syntax tree warm. Each sample
//! is the key until the frame that shows the new line in Insert mode. Like
//! the find scenarios this one fails: a sample past its budget is an error.

use std::io;
use std::time::{Duration, Instant};

use strop_core::frontend_input::Key;
use strop_core::Buffer;

use super::{Drive, Fixture, Op, SETTLE};
use crate::editor::io::OpenIntent;
use crate::editor::{Editor, Mode};

/// `o` or `Enter` to the frame that shows the new line, tree warm.
const OPEN_LINE: Duration = Duration::from_millis(2);

/// About 10 MB of nested Rust: an impl per unit, two methods each.
fn source() -> String {
    let mut text = String::with_capacity(10_100_000);
    let mut index = 0;
    while text.len() < 10_000_000 {
        text.push_str(&format!(
            "impl Thing{index} {{\n    pub fn total(&self, x: u32) -> u32 {{\n        \
             let mut sum = self.a;\n        for k in 0..x {{\n            \
             sum = sum.wrapping_add(k * {index});\n        }}\n        sum\n    }}\n}}\n\n"
        ));
        index += 1;
    }
    text
}

fn key(drive: &mut Drive, keys: &str) -> io::Result<()> {
    for c in keys.chars() {
        drive.key(match c {
            '\r' => Key::Enter,
            '\x1b' => Key::Esc,
            c => Key::Char(c),
        })?;
    }
    Ok(())
}

/// One sample: the key, then turns until the new line is on screen.
fn open_line(drive: &mut Drive, trigger: Key, op: &mut Op) -> io::Result<()> {
    let lines = drive.editor.buf().len_lines();
    let started = Instant::now();
    drive.key(trigger)?;
    drive.wait_until(SETTLE, |editor| {
        editor.mode() == Mode::Insert && editor.buf().len_lines() == lines + 1
    })?;
    op.samples.push(started.elapsed());
    Ok(())
}

pub(super) fn run() -> io::Result<()> {
    let fixture = Fixture::new("indent")?;
    let text = source();
    let path = fixture.write("big.rs", &text)?;
    let mut drive = Drive::new(
        Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
        120,
        40,
    )?;
    drive
        .editor
        .request_open(path, OpenIntent::Switch { readonly: false });
    drive.settle(SETTLE)?;
    let total = drive.editor.buf().len_lines();
    // line 10 is inside the first method; the others land on a `sum`
    // line of a method body, nested three deep
    let targets = [10, (total / 2) / 10 * 10 + 7, (total - 12) / 10 * 10 + 7];

    let mut first = Op::new("first_o");
    let mut below = Op::new("o");
    let mut enter = Op::new("enter");
    let admitted = drive.editor.admission_probe().resolutions;
    // the first `o` after the open, far from the top: no lexical shortcut
    key(&mut drive, &format!(":{}\r", targets[1]))?;
    drive.settle(SETTLE)?;
    open_line(&mut drive, Key::Char('o'), &mut first)?;
    let deferred = drive.editor.admission_probe().resolutions - admitted;
    key(&mut drive, "\x1bu")?;
    drive.settle(SETTLE)?;
    for _ in 0..10 {
        for line in targets {
            key(&mut drive, &format!(":{line}\r"))?;
            drive.settle(SETTLE)?;
            open_line(&mut drive, Key::Char('o'), &mut below)?;
            key(&mut drive, "\x1bu")?;
            key(&mut drive, "A")?;
            drive.settle(SETTLE)?;
            open_line(&mut drive, Key::Enter, &mut enter)?;
            key(&mut drive, "\x1bu")?;
            drive.settle(SETTLE)?;
        }
    }
    if *drive.editor.buf().text() != text {
        return Err(io::Error::other(
            "the indent scenario left the buffer edited",
        ));
    }
    println!(
        "indent ({} lines, {:.1} MB, tree warm)",
        total,
        text.len() as f64 / 1e6
    );
    first.report();
    below.report();
    enter.report();
    if deferred != 0 {
        return Err(io::Error::other(format!(
            "the first `o` started {deferred} resolution job(s)"
        )));
    }
    for op in [&first, &below, &enter] {
        if op.pct(95) > OPEN_LINE.as_secs_f64() * 1000.0 {
            return Err(io::Error::other(format!(
                "indent {} p95 {:.2} ms is over its {} ms budget",
                op.name,
                op.pct(95),
                OPEN_LINE.as_millis()
            )));
        }
    }
    Ok(())
}
