//! The history budgets (0101 §5.1): the log and a file's history on a
//! repository of 150,000 commits with a merge every 50, once strop has
//! built its private commit-graph. Past a budget is an error.
//!
//! Needs `git` (the repository is written with `git fast-import`). Driven
//! through the recorded-action path, where the after-event work that
//! starts the graph build runs.

use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use strop_core::frontend_input::Key;
use strop_core::Buffer;

use super::{Drive, Fixture, Op, SETTLE};
use crate::editor::events::AppEvent;
use crate::editor::io::OpenIntent;
use crate::editor::trace::drive::Action;
use crate::editor::Editor;
use crate::editor::Surface;

const COMMITS: usize = 150_000;
const FILES: usize = 2_000;
const MERGE_EVERY: usize = 50;
/// The file whose history is read: touched 75 times.
const FILE: &str = "src/d00/f0007.txt";

const LOG_WITH_GRAPH: Duration = Duration::from_millis(50);
const FILE_FIRST_ROWS: Duration = Duration::from_millis(60);

/// The fast-import stream: one file changed per commit, round robin,
/// committer dates 600 s apart, a one-commit side branch merged every
/// [`MERGE_EVERY`]th commit (the shape plan 0101 §1 measured).
fn history() -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMITS * 220);
    let mut mark = 0;
    let mut last = None;
    for n in 0..COMMITS {
        let when = 1_500_000_000 + n * 600;
        let path = format!("src/d{:02}/f{:04}.txt", n % FILES / 50, n % FILES);
        let body = format!("line {n}\n");
        let mut side = None;
        if n > 0 && n % MERGE_EVERY == 0 {
            mark += 1;
            side = Some(mark);
            let msg = format!("side {n}\n");
            let content = format!("side {n}\n");
            out.extend_from_slice(
                format!(
                    "commit refs/heads/side\nmark :{mark}\n\
                     author A U Thor <a@example.com> {} +0000\n\
                     committer C O Mitter <c@example.com> {} +0000\n\
                     data {}\n{msg}from :{}\n\
                     M 100644 inline side/s{}.txt\ndata {}\n{content}\n",
                    when - 60,
                    when - 60,
                    msg.len(),
                    last.unwrap_or(0),
                    n % 97,
                    content.len(),
                )
                .as_bytes(),
            );
        }
        mark += 1;
        let msg = format!("commit {n}: touch {path}\n");
        out.extend_from_slice(
            format!(
                "commit refs/heads/main\nmark :{mark}\n\
                 author Author {} <a{}@example.com> {when} +0000\n\
                 committer C O Mitter <c@example.com> {when} +0000\n\
                 data {}\n{msg}",
                n % 37,
                n % 37,
                msg.len(),
            )
            .as_bytes(),
        );
        if let Some(last) = last {
            out.extend_from_slice(format!("from :{last}\n").as_bytes());
        }
        if let Some(side) = side {
            out.extend_from_slice(format!("merge :{side}\n").as_bytes());
        }
        out.extend_from_slice(
            format!("M 100644 inline {path}\ndata {}\n{body}\n", body.len()).as_bytes(),
        );
        last = Some(mark);
    }
    out
}

fn git(dir: &std::path::Path, args: &[&str], stdin: Option<&[u8]>) -> io::Result<()> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .spawn()?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(bytes)?;
    }
    if child.wait()?.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("git {args:?} failed")))
    }
}

fn apply(drive: &mut Drive, event: AppEvent) -> io::Result<()> {
    let tick = drive.editor.tape().sample_tick();
    drive.editor.recorded_action(Action::Event(event), tick)
}

/// Keys, each with whatever is queued and a frame.
fn keys(drive: &mut Drive, keys: &[Key]) -> io::Result<()> {
    for key in keys {
        apply(drive, AppEvent::EditorKey(*key))?;
        while let Ok(event) = drive.events.try_recv() {
            apply(drive, event)?;
        }
        drive.draw()?;
    }
    Ok(())
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
                    "the history scenario did not settle",
                ))
            }
        }
    }
}

fn log_rows(editor: &Editor) -> bool {
    matches!(editor.surface(), Some(Surface::CommitLog { rows, .. }) if !rows.is_empty())
}

fn log_complete(editor: &Editor) -> bool {
    log_rows(editor) && !editor.log_loading()
}

/// One press of `chord`: the time to the first rows on screen and to the
/// whole list; then the surface is closed.
fn press(drive: &mut Drive, chord: char) -> io::Result<(Duration, Duration)> {
    let started = Instant::now();
    keys(drive, &[Key::Char(' '), Key::Char('g'), Key::Char(chord)])?;
    pump(drive, log_rows)?;
    let first = started.elapsed();
    pump(drive, log_complete)?;
    let complete = started.elapsed();
    keys(drive, &[Key::Char('q')])?;
    if drive.editor.surface().is_some() {
        return Err(io::Error::other("the log surface did not close"));
    }
    Ok((first, complete))
}

pub(super) fn run() -> io::Result<()> {
    let fixture = Fixture::new("history")?;
    let repo = fixture.dir.join("repo");
    std::fs::create_dir_all(&repo)?;
    let generated = Instant::now();
    git(&repo, &["init", "-q", "-b", "main"], None)?;
    git(&repo, &["fast-import", "--quiet"], Some(&history()))?;
    git(&repo, &["reset", "-q", "--hard", "main"], None)?;
    let generated = generated.elapsed();

    let mut editor = Editor::new_in(Buffer::from_text(""), repo.clone());
    // the first presses run with no graph: no state directory yet
    editor.set_state_dir(None);
    let mut drive = Drive::new(editor, 120, 40)?;
    drive
        .editor
        .request_open(repo.join(FILE), OpenIntent::Switch { readonly: false });
    drive.settle(SETTLE)?;
    pump(&mut drive, |editor| editor.git().is_some())?;

    let mut log_plain = Op::new("log_no_graph");
    let mut file_plain = Op::new("file_no_graph");
    for _ in 0..3 {
        log_plain.samples.push(press(&mut drive, 'l')?.1);
        file_plain.samples.push(press(&mut drive, 'h')?.1);
    }

    // a state directory: the next slow press builds the graph
    drive.editor.set_state_dir(Some(fixture.dir.join("state")));
    press(&mut drive, 'l')?;
    let mut build = Op::new("graph_build");
    build.try_time(|| pump(&mut drive, |editor| !editor.async_pending()))?;
    if !fixture.dir.join("state/strop/graphs").is_dir() {
        return Err(io::Error::other("no commit-graph was built"));
    }

    let mut log = Op::new("log");
    let mut file_first = Op::new("file_first");
    let mut file_complete = Op::new("file_complete");
    for _ in 0..20 {
        log.samples.push(press(&mut drive, 'l')?.0);
        let (first, complete) = press(&mut drive, 'h')?;
        file_first.samples.push(first);
        file_complete.samples.push(complete);
    }

    println!(
        "history ({COMMITS} commits, a merge every {MERGE_EVERY}, generated in {:.1} s)",
        generated.as_secs_f64()
    );
    for op in [
        &log_plain,
        &file_plain,
        &build,
        &log,
        &file_first,
        &file_complete,
    ] {
        op.report();
    }
    super::find::within(&log, LOG_WITH_GRAPH)?;
    super::find::within(&file_first, FILE_FIRST_ROWS)
}
