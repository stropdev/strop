//! The 1.0 perf gate (0025): `strop --bench [scenario|all]`.
//!
//! Deterministic synthetic data, the production `feed` + render path,
//! `Instant` timing, percentile report. No network, no bench framework.
//! Not a CI assert — wall clock varies by machine; this is the
//! measurement surface that any perf claim or regression check re-runs
//! and diffs.
//!
//! Two scenario families:
//! - Buffer scenarios drive `Editor::feed` + `headless::frame_string`
//!   on in-memory text.
//! - Stress scenarios (picker streams, project replace, cancel/reopen,
//!   stale-worker drops) need the async services, so they generate
//!   deterministic fixtures in a tempdir and drive the SAME event loop
//!   the headless driver uses: one `AppEvent` channel, the shared
//!   handlers, `async_pending` as the settle barrier — no competing
//!   editor loop, no sleeps for correctness.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use strop_core::Buffer;
use strop_picker::Kind;

use crate::editor::events::{AppEvent, EVENTS_PER_TURN, TURN_BUDGET};
use crate::editor::io::OpenIntent;
use crate::editor::{Editor, Key};

/// Settle budgets are backstops, not timing: a settle that outlives
/// them is a hung worker, surfaced as an error — never a sleep.
const SETTLE: Duration = Duration::from_secs(300);

/// Source-shaped line of ~80 bytes, deterministic.
fn gen_source(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 82);
    for i in 0..lines {
        s.push_str(&format!(
            "fn work_{i:07}(x: i32) -> i32 {{ let v = x * {}; v + {} }} // pad\n",
            i % 97,
            i % 13
        ));
    }
    s
}

/// One measured op: name + collected samples.
struct Op {
    name: &'static str,
    samples: Vec<Duration>,
}

impl Op {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            samples: vec![],
        }
    }

    fn time(&mut self, f: impl FnOnce()) {
        let t0 = Instant::now();
        f();
        self.samples.push(t0.elapsed());
    }

    fn try_time(&mut self, f: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        let t0 = Instant::now();
        f()?;
        self.samples.push(t0.elapsed());
        Ok(())
    }

    fn pct(&self, p: usize) -> f64 {
        let mut v: Vec<Duration> = self.samples.clone();
        v.sort();
        let idx = (p * v.len().saturating_sub(1)) / 100;
        v[idx].as_secs_f64() * 1000.0
    }

    fn report(&self) {
        println!(
            "  {:<14} n={:<4} p50={:>8.2} p95={:>8.2} p99={:>8.2} max={:>8.2} ms",
            self.name,
            self.samples.len(),
            self.pct(50),
            self.pct(95),
            self.pct(99),
            self.pct(100)
        );
    }
}

/// A deterministic fixture tree under a fresh tempdir; removed on drop.
/// Content is a pure function of the scenario — only the location is
/// per-run (pid), and a stale tree from a killed run is reclaimed.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("strop-bench-{name}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn write(&self, name: &str, text: &str) -> io::Result<PathBuf> {
        let path = self.dir.join(name);
        std::fs::write(&path, text)?;
        Ok(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The headless driver's loop (0006/0018) as a bench harness: one
/// `AppEvent` channel via `connect_events`, the shared per-event
/// handlers, fairness-bounded turns, and `async_pending` as the settle
/// barrier. Stress scenarios measure the production engine/render
/// path, not a parallel reimplementation.
struct Drive {
    editor: Editor,
    terminal: Terminal<TestBackend>,
    events: Receiver<AppEvent>,
}

impl Drive {
    fn new(mut editor: Editor, cols: u16, rows: u16) -> io::Result<Self> {
        editor.session_policy = crate::session::SessionPolicy::Disabled;
        let (tx, events) = crate::editor::events::channel();
        editor.connect_events(tx);
        let mut drive = Self {
            editor,
            terminal: Terminal::new(TestBackend::new(cols, rows))?,
            events,
        };
        drive.draw()?;
        Ok(drive)
    }

    /// One external event through the shared handler, then a
    /// fairness-bounded turn and a frame — the headless `input` step.
    fn input(&mut self, event: AppEvent) -> io::Result<()> {
        self.editor.handle_app_event(event);
        self.drain();
        self.draw()
    }

    fn key(&mut self, key: Key) -> io::Result<()> {
        self.input(AppEvent::Terminal(key))
    }

    /// One fairness-bounded turn of queued events (EVENTS_PER_TURN /
    /// TURN_BUDGET, as the live loop and the headless drain apply them).
    fn drain(&mut self) {
        let started = Instant::now();
        for _ in 0..EVENTS_PER_TURN {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            self.editor.handle_app_event(event);
            if started.elapsed() >= TURN_BUDGET {
                break;
            }
        }
    }

    fn draw(&mut self) -> io::Result<()> {
        if !self.editor.should_quit && !self.editor.docs.is_empty() {
            if std::mem::take(&mut self.editor.needs_repaint) {
                self.terminal.clear()?;
            }
            self.terminal.draw(|frame| {
                crate::render::frame_capture::draw(&mut self.editor, frame, false);
            })?;
        }
        Ok(())
    }

    /// Pump until the editor reports no outstanding finite work — the
    /// headless `wait jobs` barrier.
    fn settle(&mut self, budget: Duration) -> io::Result<()> {
        self.wait_until(budget, |editor| !editor.async_pending())
    }

    /// Pump events, drawing between turns, until `done` observes the
    /// editor state or the budget runs out. Parking uses the channel's
    /// owner wake, never a poll sleep.
    fn wait_until(&mut self, budget: Duration, done: impl Fn(&Editor) -> bool) -> io::Result<()> {
        let deadline = Instant::now().checked_add(budget).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "wait duration exceeds the clock range",
            )
        })?;
        loop {
            self.drain();
            self.draw()?;
            if done(&self.editor) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "editor jobs did not settle",
                ));
            }
            match self.events.recv_timeout(remaining) {
                Ok(event) => self.editor.handle_app_event(event),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "editor jobs did not settle",
                    ));
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

/// A picker stream that settled with a sticky error means the scenario
/// measured a failure path (rg missing, dead worker) — refuse to
/// report timings for it.
fn picker_settled_ok(drive: &Drive) -> io::Result<()> {
    match drive
        .editor
        .picker
        .as_ref()
        .and_then(|g| g.picker.error.as_ref())
    {
        Some(error) => Err(io::Error::other(format!("picker stream failed: {error}"))),
        None => Ok(()),
    }
}

/// One buffer-size scenario: open, a 100-keystroke insert session at the
/// middle line, 10× dd, 10× u.
fn bench_buffer(lines: usize, label: &str) {
    let text = gen_source(lines);
    let mut open = Op::new("open");
    let mut session = Op::new("insert_100");
    let mut dd = Op::new("dd");
    let mut undo = Op::new("u");
    for _ in 0..3 {
        let t0 = Instant::now();
        let mut e = Editor::new(Buffer::from_text(&text));
        open.samples.push(t0.elapsed());
        e.feed_text(&format!(":{}\r", lines / 2));
        session.time(|| {
            e.feed_text("i");
            e.feed_text(&"x".repeat(100));
            e.feed(Key::Esc);
        });
        for _ in 0..10 {
            dd.time(|| e.feed_text("dd"));
        }
        for _ in 0..10 {
            undo.time(|| e.feed_text("u"));
        }
    }
    println!("{label} ({lines} lines, {:.1} MB)", text.len() as f64 / 1e6);
    open.report();
    session.report();
    dd.report();
    undo.report();
}

/// 100k-line navigation: G, gg, 500j, and a worst-case search (needle
/// only on the last line).
fn bench_nav_100k() {
    let mut text = gen_source(100_000);
    text.push_str("needle_zzz\n");
    let mut e = Editor::new(Buffer::from_text(&text));
    let mut g = Op::new("G");
    let mut gg = Op::new("gg");
    let mut j500 = Op::new("500j");
    let mut search = Op::new("/needle");
    for _ in 0..3 {
        g.time(|| e.feed_text("G"));
        gg.time(|| e.feed_text("gg"));
        j500.time(|| e.feed_text("500j"));
        search.time(|| e.feed_text("/needle_zzz\r"));
    }
    println!("nav_100k (100001 lines, {:.1} MB)", text.len() as f64 / 1e6);
    g.report();
    gg.report();
    j500.report();
    search.report();
}

/// 1k cursors: 999 stacked cursors, then one cascaded insert.
fn bench_cursors_1k() {
    let lines: String = (0..1200).map(|i| format!("foo work_{i}\n")).collect();
    let mut e = Editor::new(Buffer::from_text(&lines));
    let mut add = Op::new("add_cursor");
    for _ in 0..999 {
        add.time(|| e.feed_text(" c"));
    }
    let mut edit = Op::new("cascade_iZ");
    edit.time(|| {
        e.feed_text("i");
        e.feed_text("Z");
        e.feed(Key::Esc);
    });
    println!("cursors_1k (1200 lines)");
    add.report();
    edit.report();
}

/// Input-to-frame: 500 insert-mode keystrokes, each followed by a full
/// 120×40 frame render. The user-perceived latency number.
fn bench_input_frame() {
    let text = gen_source(10_000);
    let mut e = Editor::new(Buffer::from_text(&text));
    e.feed_text(":5000\r");
    e.feed_text("i");
    let mut frame = Op::new("key+frame");
    for i in 0..500 {
        let c = (b'a' + (i % 26) as u8) as char;
        frame.time(|| {
            e.feed(Key::Char(c));
            let _ = crate::headless::frame_string(&mut e, 120, 40);
        });
    }
    e.feed(Key::Esc);
    println!("input_frame (10k lines, 120x40)");
    frame.report();
}

/// Exactly 1 MiB, one line, no newline: source-shaped words with a
/// single MARKZ sentinel at ~90% for the worst-case find.
fn gen_long_line() -> String {
    const MIB: usize = 1_048_576;
    let mut text = String::with_capacity(MIB + 64);
    let mut i = 0;
    while text.len() < 900_000 {
        text.push_str(&format!(
            "let value_{i:07} = compute(input_{:02}); ",
            i % 89
        ));
        i += 1;
    }
    text.push_str("MARKZ ");
    while text.len() < MIB {
        text.push_str(&format!("tail_{i:07} = v + {:02}; ", i % 31));
        i += 1;
    }
    text.truncate(MIB);
    text
}

/// A 1 MiB single-line buffer: end/home motions, a worst-case char
/// find, word motion, and a frame rendered with the cursor at the far
/// end of the line (the horizontal-trim stress).
fn bench_line_1mb() {
    let text = gen_long_line();
    let mut open = Op::new("open");
    let mut end = Op::new("$");
    let mut frame = Op::new("frame_end");
    let mut home = Op::new("0");
    let mut find = Op::new("fZ");
    let mut words = Op::new("500w");
    for _ in 0..3 {
        let t0 = Instant::now();
        let mut e = Editor::new(Buffer::from_text(&text));
        open.samples.push(t0.elapsed());
        end.time(|| e.feed_text("$"));
        frame.time(|| {
            let _ = crate::headless::frame_string(&mut e, 120, 40);
        });
        home.time(|| e.feed_text("0"));
        find.time(|| e.feed_text("fZ"));
        words.time(|| e.feed_text("500w"));
    }
    println!("line_1mb (1 line, {:.1} MB)", text.len() as f64 / 1e6);
    open.report();
    end.report();
    frame.report();
    home.report();
    find.report();
    words.report();
}

/// 100k-result picker stream: one grep request over a generated tree,
/// settled through the real event loop, then a frame with the full
/// result list loaded.
fn bench_picker_100k() -> io::Result<()> {
    let fixture = Fixture::new("picker-100k")?;
    // 200 files x 500 hits = 100k streamed items.
    for f in 0..200 {
        let mut text = String::with_capacity(500 * 64);
        for i in 0..500 {
            text.push_str(&format!(
                "let needle_hit_{f:03}_{i:03} = compute(x); // padding pad\n"
            ));
        }
        fixture.write(&format!("src_{f:03}.txt"), &text)?;
    }
    let mut drive = Drive::new(
        Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
        120,
        40,
    )?;
    let mut stream = Op::new("stream+settle");
    let mut frame = Op::new("frame_full");
    for _ in 0..3 {
        drive.editor.open_picker(Kind::Grep);
        stream.try_time(|| {
            drive.input(AppEvent::Paste("needle_hit_".into()))?;
            drive.settle(SETTLE)
        })?;
        picker_settled_ok(&drive)?;
        frame.try_time(|| drive.draw())?;
        drive.editor.close_picker();
    }
    println!("picker_100k (200 files x 500 hits)");
    stream.report();
    frame.report();
    Ok(())
}

/// Measure the actual Search -> Review -> Apply -> Save cutover separately.
/// Repeated fresh editor instances include cold source loading; on-disk
/// witnesses prevent reporting a fast refusal/no-op as successful replacement.
fn bench_replace_project() -> io::Result<()> {
    const FILES: usize = 300;
    const HITS: usize = 50;
    const RUNS: usize = 8;
    let fixture = Fixture::new("replace-project")?;
    let mut search = Op::new("search+settle");
    let mut review = Op::new("review+settle");
    let mut apply = Op::new("apply+frame");
    let mut save = Op::new("save+settle");
    let mut retire = Op::new("retire");
    for _ in 0..RUNS {
        for f in 0..FILES {
            let mut text = String::with_capacity(HITS * 64);
            for i in 0..HITS {
                text.push_str(&format!(
                    "// needle_todo {f:03}/{i:03} padding padding padding\n"
                ));
            }
            fixture.write(&format!("mod_{f:03}.txt"), &text)?;
        }
        let mut drive = Drive::new(
            Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
            120,
            40,
        )?;
        drive.editor.open_picker(Kind::Replace);
        search.try_time(|| {
            drive.input(AppEvent::Paste("needle_todo".into()))?;
            drive.settle(SETTLE)
        })?;
        picker_settled_ok(&drive)?;
        review.try_time(|| {
            drive.key(Key::Tab)?;
            drive.input(AppEvent::Paste("fixed_done".into()))?;
            drive.key(Key::Enter)?;
            drive.settle(SETTLE)
        })?;
        apply.try_time(|| {
            for character in ":apply-change".chars() {
                drive.key(Key::Char(character))?;
            }
            drive.key(Key::Enter)
        })?;
        save.try_time(|| {
            for character in ":save-change".chars() {
                drive.key(Key::Char(character))?;
            }
            drive.key(Key::Enter)?;
            drive.settle(SETTLE)
        })?;
        for f in 0..FILES {
            let text = std::fs::read_to_string(fixture.dir.join(format!("mod_{f:03}.txt")))?;
            if text.matches("fixed_done").count() != HITS || text.contains("needle_todo") {
                return Err(io::Error::other(format!(
                    "replacement/save did not reach mod_{f:03}.txt"
                )));
            }
        }
        retire.time(|| drop(drive));
    }
    println!("replace_project ({FILES} files x {HITS} hits; {RUNS} fresh runs)");
    search.report();
    review.report();
    apply.report();
    save.report();
    retire.report();
    Ok(())
}

/// Repeated cancel/reopen of a large file: start a load, supersede it
/// before it lands (the real impatience path — `request_target`
/// cancels the pending navigation and its late completion is dropped),
/// then settle the replacement. A plain worker open per iteration is
/// the reference.
fn bench_reopen_cancel() -> io::Result<()> {
    let fixture = Fixture::new("reopen-cancel")?;
    for i in 0..3 {
        for side in ["cancelled", "opened", "plain"] {
            fixture.write(&format!("big_{i}_{side}.txt"), &gen_source(100_000))?;
        }
    }
    let mut drive = Drive::new(
        Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
        120,
        40,
    )?;
    let mut cancel = Op::new("cancel+open");
    let mut open = Op::new("open");
    for i in 0..3 {
        drive.editor.request_open(
            fixture.dir.join(format!("big_{i}_cancelled.txt")),
            OpenIntent::Switch { readonly: false },
        );
        cancel.try_time(|| {
            drive.editor.request_open(
                fixture.dir.join(format!("big_{i}_opened.txt")),
                OpenIntent::Switch { readonly: false },
            );
            drive.settle(SETTLE)
        })?;
        open.try_time(|| {
            drive.editor.request_open(
                fixture.dir.join(format!("big_{i}_plain.txt")),
                OpenIntent::Switch { readonly: false },
            );
            drive.settle(SETTLE)
        })?;
    }
    println!("reopen_cancel (8 MB file, cancel mid-load + reopen)");
    cancel.report();
    open.report();
    Ok(())
}

/// Dropping a large worker result while newer work is active: prime a
/// 50k-item stream, then edit the query — every keystroke revokes the
/// in-flight stream, its queued batches drain through ticket rejection
/// (never the model), and the replacement stream delivers underneath.
fn bench_drop_stale() -> io::Result<()> {
    let fixture = Fixture::new("drop-stale")?;
    // 50k alpha + 50k beta hits interleaved across 200 files.
    for f in 0..200 {
        let mut text = String::with_capacity(500 * 64);
        for i in 0..500 {
            let side = if (f + i) % 2 == 0 { "alpha" } else { "beta" };
            text.push_str(&format!(
                "needle_{side} hit {f:03}/{i:03} padding padding\n"
            ));
        }
        fixture.write(&format!("mix_{f:03}.txt"), &text)?;
    }
    let mut drive = Drive::new(
        Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
        120,
        40,
    )?;
    let mut prime = Op::new("prime_10k");
    let mut supersede = Op::new("supersede+settle");
    for _ in 0..3 {
        drive.editor.open_picker(Kind::Grep);
        prime.try_time(|| {
            drive.input(AppEvent::Paste("needle_alpha".into()))?;
            // Consume until 10k items have landed; the rest of the 50k
            // is still in flight — that tail is what gets dropped.
            drive.wait_until(SETTLE, |e| {
                e.picker
                    .as_ref()
                    .is_none_or(|g| !g.picker.streaming || g.picker.items.len() >= 10_000)
            })
        })?;
        picker_settled_ok(&drive)?;
        supersede.try_time(|| {
            // "needle_alpha" → "needle_" (one revoke+respawn per
            // keystroke) → "needle_beta"; the alpha backlog drains
            // through ticket rejection while the beta stream delivers.
            for _ in 0..5 {
                drive.key(Key::Backspace)?;
            }
            drive.input(AppEvent::Paste("beta".into()))?;
            drive.settle(SETTLE)
        })?;
        picker_settled_ok(&drive)?;
        drive.editor.close_picker();
    }
    println!("drop_stale (50k-item stream revoked mid-flight)");
    prime.report();
    supersede.report();
    Ok(())
}

pub fn run(which: &str) -> io::Result<()> {
    let all = which == "all";
    if all || which.starts_with("buffer_") {
        for (lines, label) in [
            (12_500, "buffer_1mb"),
            (125_000, "buffer_10mb"),
            (1_250_000, "buffer_100mb"),
        ] {
            if all || which == label {
                bench_buffer(lines, label);
            }
        }
    }
    if all || which == "nav_100k" {
        bench_nav_100k();
    }
    if all || which == "cursors_1k" {
        bench_cursors_1k();
    }
    if all || which == "input_frame" {
        bench_input_frame();
    }
    if all || which == "picker_100k" {
        bench_picker_100k()?;
    }
    if all || which == "line_1mb" {
        bench_line_1mb();
    }
    if all || which == "replace_project" {
        bench_replace_project()?;
    }
    if all || which == "reopen_cancel" {
        bench_reopen_cancel()?;
    }
    if all || which == "drop_stale" {
        bench_drop_stale()?;
    }
    Ok(())
}
