//! The 1.0 perf gate (0025): `strop --bench [scenario|all]`.
//!
//! Deterministic synthetic data, the production `feed` + render path,
//! `Instant` timing, percentile report. No fs, no network, no bench
//! framework. Not a CI assert — wall clock varies by machine; this is
//! the measurement surface that any perf claim or regression check
//! re-runs and diffs.

use std::time::{Duration, Instant};

use strop_core::Buffer;

use crate::editor::{Editor, Key};

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
    fn time(&mut self, f: impl FnOnce()) {
        let t0 = Instant::now();
        f();
        self.samples.push(t0.elapsed());
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

/// One buffer-size scenario: open, a 100-keystroke insert session at the
/// middle line, 10× dd, 10× u.
fn bench_buffer(lines: usize, label: &str) {
    let text = gen_source(lines);
    let mut open = Op {
        name: "open",
        samples: vec![],
    };
    let mut session = Op {
        name: "insert_100",
        samples: vec![],
    };
    let mut dd = Op {
        name: "dd",
        samples: vec![],
    };
    let mut undo = Op {
        name: "u",
        samples: vec![],
    };
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
    let mut g = Op {
        name: "G",
        samples: vec![],
    };
    let mut gg = Op {
        name: "gg",
        samples: vec![],
    };
    let mut j500 = Op {
        name: "500j",
        samples: vec![],
    };
    let mut search = Op {
        name: "/needle",
        samples: vec![],
    };
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
    let mut add = Op {
        name: "add_cursor",
        samples: vec![],
    };
    for _ in 0..999 {
        add.time(|| e.feed_text(" c"));
    }
    let mut edit = Op {
        name: "cascade_iZ",
        samples: vec![],
    };
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
    let mut frame = Op {
        name: "key+frame",
        samples: vec![],
    };
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

pub fn run(which: &str) {
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
}
