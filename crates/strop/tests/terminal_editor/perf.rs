use super::*;
use std::fs::File;
use std::io;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub(crate) fn benchmark_binary() -> (std::path::PathBuf, String, u64) {
    use sha2::{Digest, Sha256};

    let binary = std::env::var_os("STROP_BENCH_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_strop")));
    let mut artifact = File::open(&binary).unwrap();
    let mut hasher = Sha256::new();
    let mut bytes = [0; 65_536];
    loop {
        let count = artifact.read(&mut bytes).unwrap();
        if count == 0 {
            break;
        }
        hasher.update(&bytes[..count]);
    }
    let sha256 = format!("{:x}", hasher.finalize());
    (binary, sha256, artifact.metadata().unwrap().len())
}

pub(crate) fn benchmark_percentiles(raw_ms: &[f64]) -> serde_json::Value {
    let mut ordered = raw_ms.to_vec();
    ordered.sort_by(f64::total_cmp);
    let percentile = |n: usize| ordered[(n * ordered.len()).div_ceil(100) - 1];
    serde_json::json!({
        "p50": percentile(50),
        "p95": percentile(95),
        "p99": percentile(99),
        "max": ordered[ordered.len() - 1],
    })
}

/// Opt-in native performance observation. Each sample waits until a real PTY's
/// decoded cell grid shows the edit, rather than stopping at an input ACK or
/// an internal semantic view. The same harness can drive a clean baseline
/// artifact through STROP_BENCH_BINARY without changing the shipped editor.

#[test]
#[ignore = "native TUI performance measurement; run explicitly with --ignored --nocapture"]
fn native_terminal_input_to_painted_frame_samples() {
    let (binary, binary_sha256, binary_bytes) = benchmark_binary();

    let directory = tempfile::tempdir().unwrap();
    let mut notes = io::BufWriter::new(File::create(directory.path().join("notes.txt")).unwrap());
    for line in 0..10_000 {
        if line != 0 {
            notes.write_all(b"\n").unwrap();
        }
        write!(notes, "line {line:05} worker frame fixture").unwrap();
    }
    notes.flush().unwrap();
    drop(notes);
    let trace = directory.path().join("no-capture.jsonl");
    let mut tui = Tui::spawn(directory.path(), &trace, &binary, false);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":e notes.txt\r");
    tui.until(|screen| screen.contains("line 00000 worker frame fixture"));
    tui.send(b":5000\rA");
    tui.until(|screen| {
        screen.contains("INSERT") && screen.contains("line 04999 worker frame fixture")
    });

    let warmup = 8;
    let iterations = 64;
    let mut raw_ms = Vec::with_capacity(iterations);
    let mut expected = String::from("line 04999 worker frame fixture");
    for index in 0..(warmup + iterations) {
        expected.push('x');
        let started = Instant::now();
        tui.send(b"x");
        tui.until_within(Duration::from_secs(15), |screen| {
            numbered_line(screen, &expected)
        });
        if index >= warmup {
            raw_ms.push((started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0);
        }
    }
    tui.send(b"\x1b:qa!\r");
    assert!(tui.wait_exit().success(), "TUI did not exit cleanly");

    let input_to_grid_ms = benchmark_percentiles(&raw_ms);
    let report = serde_json::json!({
        "binary_sha256": binary_sha256,
        "binary_bytes": binary_bytes,
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "fixture": {
            "lines": 10_000,
            "geometry": [120, 30],
            "input": "one committed character at line 5000",
            "capture": false,
        },
        "method": "PTY key write through VT100 decoded and verified TUI cell-grid paint",
        "warmup_requests": warmup,
        "measured_requests": iterations,
        "raw_ms": raw_ms,
        "input_to_grid_ms": input_to_grid_ms,
    });
    println!("STROP_TUI_BENCH={report}");
}

/// One real worker-leased terminal, repeatedly flooded with 256 shell
/// output lines per two-key request. Timing ends only when the unique
/// final marker is painted on the decoded host cell grid. The request
/// contains no marker text, so input echo cannot impersonate output.
#[test]
#[ignore = "native terminal output-load measurement; run explicitly with --ignored --nocapture"]
fn native_terminal_output_under_load_samples() {
    use std::fmt::Write as _;

    let (binary, binary_sha256, binary_bytes) = benchmark_binary();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("producer.sh"),
        r#"#!/bin/sh
printf 'BENCH-READY\n'
n=0
while IFS= read -r request; do
    i=0
    while [ "$i" -lt 256 ]; do
        printf 'L%04d\n' "$i"
        i=$((i+1))
    done
    printf 'OUTPUT-END-%03d\n' "$n"
    n=$((n+1))
done
"#,
    )
    .unwrap();
    let trace = directory.path().join("no-capture.jsonl");
    let mut tui = Tui::spawn(directory.path(), &trace, &binary, false);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":terminal /bin/sh producer.sh\r");
    tui.until(|screen| screen.contains("TERMINAL") && line(screen, "BENCH-READY"));

    let warmup = 8;
    let iterations = 64;
    let mut raw_ms = Vec::with_capacity(iterations);
    let mut marker = String::with_capacity(32);
    for index in 0..(warmup + iterations) {
        marker.clear();
        write!(&mut marker, "OUTPUT-END-{index:03}").unwrap();
        let started = Instant::now();
        tui.send(b"x\r");
        tui.until_within(Duration::from_secs(30), |screen| line(screen, &marker));
        if index >= warmup {
            raw_ms.push((started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0);
        }
    }
    tui.send(b"\x1c\x0e:terminal-stop\r");
    tui.until(|screen| screen.contains("terminal ended") || screen.contains("terminal exited"));
    tui.send(b":qa\r");
    assert!(tui.wait_exit().success(), "loaded TUI did not exit cleanly");

    let output_to_grid_ms = benchmark_percentiles(&raw_ms);
    let report = serde_json::json!({
        "binary_sha256": binary_sha256,
        "binary_bytes": binary_bytes,
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "fixture": {
            "output_lines_per_request": 256,
            "geometry": [120, 30],
            "input": "one two-key line to a worker-leased shell PTY",
            "capture": false,
        },
        "method": "two-key PTY input through 256 output lines to VT100 decoded and verified final grid marker",
        "warmup_requests": warmup,
        "measured_requests": iterations,
        "raw_ms": raw_ms,
        "output_to_grid_ms": output_to_grid_ms,
    });
    println!("STROP_TERMINAL_LOAD_BENCH={report}");
}
