use super::mixed_fixture::{Fixture, Usage};
use super::perf::{benchmark_binary, benchmark_percentiles};
use super::Tui;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

const RUNS: usize = 20;
const QUALIFIED: &str = "agent_generated_0000";

fn start(f: &Fixture, binary: &Path) -> Tui {
    let mut tui = f.spawn(binary);
    tui.until(|s| s.contains("NORMAL"));
    tui.send(format!(":e {}\r", f.large).as_bytes());
    tui.until(|s| s.contains("pub fn measured_symbol()"));
    tui
}

fn finish(mut tui: Tui) {
    tui.send(b":qa!\r");
    tui.expect_clean_exit("mixed symbol benchmark editor");
}

fn gate(tui: &mut Tui, dir: &Path, name: &str) {
    let signal = dir.join(format!("{name}.waiting"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !signal.exists() {
        assert!(
            Instant::now() < deadline,
            "no {name} request: {}",
            tui.screen.screen().contents()
        );
        tui.until_soft(Duration::from_millis(5), |_| false);
    }
}

fn configure(f: &Fixture, gate: &Path) {
    let python = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("python3"))
        .find(|p| p.is_file())
        .expect("python3");
    let server = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mixed_bench_server.py"
    );
    fs::write(f.config().join("languages.toml"), format!(
        "[language-server.rust-analyzer]\ncommand = {python:?}\nargs = [{server:?}, {gate:?}, {:?}]\n", f.work.join(f.large)
    )).unwrap();
}

fn header<'a>(screen: &'a str, title: &str) -> Option<&'a str> {
    screen
        .lines()
        .find(|s| s.contains('╭') && s.contains(title))
}

// Only the left-hand result list counts, not a name in the preview/query/backdrop.
fn rows(screen: &str) -> impl Iterator<Item = (&str, usize, bool)> {
    screen.lines().filter_map(|line| {
        let list = line.split_once('│')?.1.split('│').next()?.trim();
        let selected = list.starts_with('▌');
        let mut words = list.trim_start_matches('▌').split_whitespace();
        if words.next()? != "fn" {
            return None;
        }
        let name = words.next()?;
        let line = words.find_map(|w| w.strip_prefix(':')?.parse().ok())?;
        Some((name, line, selected))
    })
}

// The terminal header includes an ellipsis while either the stream or ranking
// is active. For a ready workspace the syntax total alone isn't completion:
// the distinct qualified LSP result must ALSO have landed for this query.
fn settled(screen: &str, title: &str) -> Option<usize> {
    let h = header(screen, title)?;
    if h.contains('…') {
        return None;
    }
    h.split_whitespace()
        .rev()
        .find_map(|s| s.replace(',', "").parse().ok())
}

fn syntax_row(name: &str, line: usize, edited: bool) -> bool {
    let shift = usize::from(edited);
    match name {
        "measured_symbol" => line == 1 + shift,
        "edited_outline" => edited && line == 1,
        _ => name
            .strip_prefix("generated_")
            .and_then(|n| n.parse::<usize>().ok())
            .is_some_and(|n| n < 2800 && line == n + 2 + shift),
    }
}

struct Samples(BTreeMap<&'static str, Vec<Value>>);

impl Samples {
    fn measure(
        &mut self,
        scenario: &'static str,
        tui: &mut Tui,
        workspace: bool,
        valid: impl Fn(&str, usize) -> bool,
        total: Option<usize>,
        release: Option<&Path>,
    ) -> usize {
        let title = if workspace {
            " workspace symbols "
        } else {
            " symbols "
        };
        let before = Usage::read(tui);
        let start = Instant::now();
        tui.send(if workspace { b" S" } else { b" s" });
        if let Some(path) = release {
            fs::write(path, []).unwrap();
        }
        let usable = |s: &str| header(s, title).is_some() && rows(s).any(|(n, l, _)| valid(n, l));
        let complete =
            |s: &str| usable(s) && settled(s, title).is_some_and(|n| total.is_none_or(|t| t == n));
        let first_frame = tui.until(usable);
        let first = start.elapsed().as_secs_f64() * 1000.;
        let (last_frame, last) = if complete(&first_frame) {
            (first_frame, first)
        } else {
            let frame = tui.until(complete);
            (frame, start.elapsed().as_secs_f64() * 1000.)
        };
        let count = settled(&last_frame, title).unwrap();
        self.0.entry(scenario).or_default().push(json!({
            "first_ms": first, "complete_ms": last, "total_rows": count,
            "usage": Usage::read(tui).report_since(&before),
        }));
        count
    }

    fn report(&self) -> Value {
        self.0.iter().map(|(key, samples)| {
            let stats = |field: &str| benchmark_percentiles(&samples.iter().map(|v| v[field].as_f64().unwrap()).collect::<Vec<_>>());
            (key.to_string(), json!({"raw": samples, "first_ms": stats("first_ms"), "complete_ms": stats("complete_ms")}))
        }).collect::<serde_json::Map<_,_>>().into()
    }
}

fn query(tui: &mut Tui, name: &str) {
    tui.send(format!("\x1b[200~{name}\x1b[201~").as_bytes());
}

fn accept(tui: &mut Tui, name: &str, line: usize, file: &str, token: &str, col: usize) {
    query(tui, name);
    tui.until(|s| rows(s).any(|(n, l, selected)| selected && n == name && l == line));
    tui.send(b"\r");
    let pos = format!("{line}:{col}");
    tui.until(|s| {
        header(s, " symbols ").is_none()
            && s.contains(token)
            && s.lines()
                .last()
                .is_some_and(|m| m.contains(file) && m.split_whitespace().any(|w| w == pos))
    });
}

fn workspace_row(name: &str, line: usize) -> bool {
    syntax_row(name, line, false)
        || matches!(
            (name, line),
            ("reporter_symbol" | "render", 1) | ("AgentMain" | "SecondSymbol", 2) | (QUALIFIED, 2)
        )
}

fn close_workspace(tui: &mut Tui) {
    tui.send(b"\x1b\x1b");
    tui.until(|s| header(s, " workspace symbols ").is_none());
}

fn cancel(tui: &mut Tui, gate_dir: &Path) {
    tui.send(b" S");
    tui.until(|s| header(s, " workspace symbols ").is_some());
    query(tui, "retired");
    gate(tui, gate_dir, "retired");
    tui.send(b"\x15");
    query(tui, "proof");
    // Prove that the editor saw the successor *before* releasing the retired reply.
    tui.until(|s| s.lines().any(|l| l.contains('❯') && l.contains("proof")));
    fs::write(gate_dir.join("retired.release"), []).unwrap();
    let s = tui.until(|s| {
        settled(s, " workspace symbols ").is_some()
            && rows(s).any(|(n, _, _)| n == "proof::generated_0000")
    });
    assert!(
        !s.contains("generated_0000_retired"),
        "retired generation leaked: {s}"
    );
    close_workspace(tui);
}

#[test]
#[ignore = "#262 unchanged release mixed-workspace symbol baseline; Linux PTY and python3"]
fn mixed_symbol_samples() {
    let (binary, sha256, bytes) = benchmark_binary();
    let mut samples = Samples(BTreeMap::new());
    let f = Fixture::new();
    for _ in 0..RUNS {
        let config = f.config().join("languages.toml");
        if config.exists() {
            fs::remove_file(config).unwrap();
        }
        let mut doc = start(&f, &binary);
        for case in ["document_open", "document_warm"] {
            samples.measure(
                case,
                &mut doc,
                false,
                |n, l| syntax_row(n, l, false),
                Some(2801),
                None,
            );
            accept(
                &mut doc,
                "measured_symbol",
                1,
                "lib.rs",
                "pub fn measured_symbol()",
                8,
            );
        }
        doc.send(b"ggOfn edited_outline() {}");
        doc.until(|s| s.contains("INSERT") && s.contains("fn edited_outline() {}"));
        doc.send(b"\x1b");
        doc.until(|s| s.contains("NORMAL"));
        samples.measure(
            "document_after_edit",
            &mut doc,
            false,
            |n, l| syntax_row(n, l, true),
            Some(2802),
            None,
        );
        accept(
            &mut doc,
            "edited_outline",
            1,
            "lib.rs",
            "fn edited_outline()",
            4,
        );
        finish(doc);

        let mut ws = start(&f, &binary);
        let mut expected = None;
        for case in ["workspace_fallback", "workspace_fallback_warm"] {
            let count = samples.measure(case, &mut ws, true, workspace_row, expected, None);
            assert!(count > 2801);
            expected = Some(count);
            if case == "workspace_fallback" {
                close_workspace(&mut ws);
            } else {
                accept(
                    &mut ws,
                    "reporter_symbol",
                    1,
                    "lib.rs",
                    "pub fn reporter_symbol()",
                    8,
                );
            }
        }
        finish(ws);

        let gates = tempfile::tempdir().unwrap();
        configure(&f, gates.path());
        let mut ready = start(&f, &binary);
        gate(&mut ready, gates.path(), "document");
        for (case, release) in [
            (
                "document_server_ready",
                Some(gates.path().join("document.release")),
            ),
            ("document_server_warm", None),
        ] {
            samples.measure(
                case,
                &mut ready,
                false,
                |n, l| n == "generated_0000" && l == 2,
                Some(1),
                release.as_deref(),
            );
            accept(
                &mut ready,
                "generated_0000",
                2,
                "lib.rs",
                "pub fn generated_0000()",
                8,
            );
        }
        for case in ["workspace_ready", "workspace_ready_warm"] {
            samples.measure(
                case,
                &mut ready,
                true,
                workspace_row,
                expected.map(|n| n + 1),
                None,
            );
            if case == "workspace_ready" {
                close_workspace(&mut ready);
            } else {
                accept(
                    &mut ready,
                    QUALIFIED,
                    2,
                    "lib.rs",
                    "pub fn generated_0000()",
                    8,
                );
            }
        }
        cancel(&mut ready, gates.path());
        finish(ready);
    }
    let report = json!({"binary_sha256":sha256,"binary_bytes":bytes,"runs":RUNS,"provider_delay_ms":80,"samples":samples.report()});
    println!("STROP_MIXED_SYMBOLS_BENCH={report}");
}
