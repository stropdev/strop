//! Opening a large file (plan 2026-11-28 D7): a 100 MB log through the
//! production open path, its time, and the process's peak resident
//! memory during the open against what the open document keeps. The
//! rope is built straight from the file, so the peak stays near the
//! settled size; past 1.3× is an error. Linux only, like the release
//! artifact (it reads `/proc/self`).

use std::io;
use std::time::Instant;

use strop_core::Buffer;

use super::{gen_source, Drive, Fixture, Op, SETTLE};
use crate::editor::io::OpenIntent;
use crate::editor::Editor;

const PEAK_OVER_SETTLED: f64 = 1.3;

/// A `/proc/self/status` figure in MB (`VmRSS`, `VmHWM`).
fn status_mb(field: &str) -> io::Result<f64> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let kilobytes = status
        .lines()
        .find_map(|line| line.strip_prefix(field)?.strip_prefix(':'))
        .and_then(|value| value.trim().strip_suffix(" kB")?.parse::<f64>().ok())
        .ok_or_else(|| io::Error::other(format!("/proc/self/status has no {field}")))?;
    Ok(kilobytes / 1024.0)
}

pub(super) fn run() -> io::Result<()> {
    let fixture = Fixture::new("open")?;
    let path = fixture.write("big.log", &gen_source(1_600_000))?;
    let size = std::fs::metadata(&path)?.len() as usize;
    let mut drive = Drive::new(
        Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
        120,
        40,
    )?;
    let before = status_mb("VmRSS")?;
    // the peak counts from here (Linux: "5" resets VmHWM to VmRSS)
    std::fs::write("/proc/self/clear_refs", "5")?;
    let mut open = Op::new("open");
    let started = Instant::now();
    drive
        .editor
        .request_open(path, OpenIntent::Switch { readonly: false });
    drive.wait_until(SETTLE, |editor| editor.buf().len_bytes() == size)?;
    open.samples.push(started.elapsed());
    drive.settle(SETTLE)?;
    let peak = status_mb("VmHWM")? - before;
    let settled = status_mb("VmRSS")? - before;
    println!("open_100mb ({:.1} MB file)", size as f64 / 1e6);
    open.report();
    println!("  memory         peak={peak:>8.1} settled={settled:>8.1} MB over the empty editor");
    if peak > settled * PEAK_OVER_SETTLED {
        return Err(io::Error::other(format!(
            "open peak {peak:.1} MB is over {PEAK_OVER_SETTLED}x the settled {settled:.1} MB"
        )));
    }
    Ok(())
}
