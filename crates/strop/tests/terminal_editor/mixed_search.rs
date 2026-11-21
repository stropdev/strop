//! Real PTY observation of project search and its file-finder control. A whole
//! term is one bracketed paste: no scans for prefixes in the measured request.
use super::mixed_fixture::{Fixture, Usage};
use super::perf::{benchmark_binary, benchmark_percentiles};
use super::Tui;
use serde_json::{json, Value};
use std::cell::Cell;
use std::time::Instant;

const RUNS: usize = 20;
const RARE_PATH: &str = "bulk/d37/p19.txt";
const SHARED_PATHS: [&str; 3] = [
    "notes.txt",
    "team-a/service/src/main.cpp",
    "team-b/service/src/main.py",
];

#[derive(Clone, Copy, PartialEq)]
enum Case {
    Rare,
    Absent,
    Shared,
    Common,
    Files,
}

impl Case {
    fn query(self, f: &Fixture) -> &str {
        match self {
            Self::Rare => f.rare,
            Self::Absent => "absent076",
            Self::Shared => f.common,
            Self::Common => "widespread076",
            Self::Files => "notes.txt",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rare => "rare",
            Self::Absent => "absent",
            Self::Shared => "mixed_root_collision",
            Self::Common => "common",
            Self::Files => "file_control",
        }
    }

    fn title(self) -> &'static str {
        if self == Self::Files {
            "╭ files "
        } else {
            "╭ search "
        }
    }

    // Explicit terminal count: a streaming tally ends in `…`, not ` ╮`.
    // Finder counts all 4,108 eligible files, including the nested .ignore.
    fn terminal(self) -> &'static str {
        match self {
            Self::Rare => " 1 hit · 1 file ╮",
            Self::Absent => " 0 hits · 0 files ╮",
            Self::Shared => " 3 hits · 3 files ╮",
            Self::Common => " 4,096 hits · 4,096 files ╮",
            Self::Files => " 1 of 4,108 ╮",
        }
    }

    fn valid_hit(self, path: &str, line: &str) -> bool {
        match self {
            Self::Rare => path == RARE_PATH && line == "65",
            Self::Shared => SHARED_PATHS.contains(&path) && line == "1",
            Self::Common => {
                // Exactly bulk/d00..63/p00..63.txt, one hit per file on line 64.
                let Some((d, p)) = path.strip_prefix("bulk/d").and_then(|s| s.split_once("/p"))
                else {
                    return false;
                };
                let index =
                    |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()) && s < "64";
                line == "64" && index(d) && p.strip_suffix(".txt").is_some_and(index)
            }
            Self::Absent | Self::Files => false,
        }
    }
}

// Only the results column. The field, source preview and backdrop can all
// contain the same path/text and none of those makes a result selectable.
fn column(screen: &str) -> impl Iterator<Item = &str> {
    screen
        .lines()
        .filter_map(|line| line.split('│').nth(1))
        .map(str::trim)
}

fn hit_paths<'a>(
    screen: &'a str,
    case: Case,
    query: &'a str,
    selected_only: bool,
) -> impl Iterator<Item = &'a str> {
    column(screen)
        .zip(column(screen).skip(1))
        .filter_map(move |(heading, hit)| {
            let path = heading.split_whitespace().next()?;
            if selected_only && !hit.starts_with('▌') {
                return None;
            }
            let text = hit.trim_start_matches('▌').trim_start();
            let (line, snippet) = text.split_once(' ')?;
            (case.valid_hit(path, line) && snippet.contains(query)).then_some(path)
        })
}

fn selected<'a>(screen: &'a str, case: Case, query: &'a str) -> Option<&'a str> {
    if case == Case::Files {
        column(screen)
            .any(|row| {
                row.strip_prefix('▌')
                    .is_some_and(|name| name.trim() == "notes.txt")
            })
            .then_some("notes.txt")
    } else {
        hit_paths(screen, case, query, true).next()
    }
}

fn has_query(screen: &str, case: Case, query: &str) -> bool {
    let Some(header) = screen.lines().next() else {
        return false;
    };
    header.contains(case.title())
        && column(screen).next().is_some_and(|field| {
            field
                .trim_start_matches(['▮', '❯'])
                .trim_start()
                .strip_prefix(if case == Case::Files { "" } else { "Find " })
                .is_some_and(|value| value.trim_end_matches('▏') == query)
        })
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

struct Sample {
    first: Option<f64>,
    complete: f64,
    usage: Value,
}

fn measure(tui: &mut Tui, f: &Fixture, case: Case) -> (Sample, String) {
    let query = case.query(f);
    let keys = format!(
        " {}\x1b[200~{query}\x1b[201~",
        if case == Case::Files { 'f' } else { '/' }
    );
    let first = Cell::new(None);
    let before = Usage::read(tui);
    let start = Instant::now();
    tui.send(keys.as_bytes());
    let screen = tui.until(|screen| {
        if !has_query(screen, case, query) {
            return false;
        }
        let has_row = selected(screen, case, query).is_some();
        if first.get().is_none() && has_row {
            first.set(Some(elapsed_ms(start)));
        }
        screen
            .lines()
            .next()
            .unwrap()
            .trim_end()
            .ends_with(case.terminal())
            && match case {
                Case::Absent => {
                    column(screen).any(|r| r == "No matches “absent076”")
                        && !column(screen).any(|r| r.starts_with('▌'))
                }
                // A PTY read may split a single paint after its count, before
                // its last row. Require the three native targets on this grid.
                Case::Shared => {
                    has_row
                        && SHARED_PATHS.iter().all(|&path| {
                            hit_paths(screen, case, query, false).any(|shown| shown == path)
                        })
                }
                _ => has_row,
            }
    });
    let complete = elapsed_ms(start);
    let usage = Usage::read(tui).report_since(&before);
    (
        Sample {
            first: first.get(),
            complete,
            usage,
        },
        screen,
    )
}

fn dismiss(tui: &mut Tui, case: Case) {
    // Search retains its field and results. Clear it before dismissing so the
    // warm repeat cannot time a retained surface or launch an extra old scan.
    tui.send(b"\x15\x1b");
    tui.until(|s| column(s).next().is_some_and(|field| field.starts_with('▮')));
    tui.send(b"\x1b");
    tui.until(|s| !s.contains(case.title()) && s.contains("NORMAL"));
}

fn accept(tui: &mut Tui, f: &Fixture, case: Case, screen: &str) {
    let path = selected(screen, case, case.query(f)).unwrap();
    let (line, text) = match case {
        Case::Rare => (65, f.rare),
        Case::Common => (64, "ordinary project fixture line widespread076"),
        Case::Files => (1, f.common),
        Case::Shared => (
            1,
            match path {
                "notes.txt" => f.common,
                "team-a/service/src/main.cpp" => "void render() {} // sharedneedle076",
                "team-b/service/src/main.py" => "def render(): pass # sharedneedle076",
                _ => unreachable!(),
            },
        ),
        Case::Absent => unreachable!(),
    };
    let col = if case == Case::Files {
        1
    } else {
        text.find(case.query(f)).unwrap() + 1
    };
    let position = format!("{line}:{col}");
    tui.send(b"\r");
    tui.until(|s| {
        !s.contains(case.title())
            && super::numbered_line(s, text)
            && s.lines().last().is_some_and(|status| {
                status.contains("NORMAL")
                    && status.split_whitespace().any(|item| item == path)
                    && status.trim_end().ends_with(&position)
            })
    });
}

fn summary(samples: &[Sample]) -> Value {
    let first: Vec<_> = samples.iter().filter_map(|s| s.first).collect();
    let complete: Vec<_> = samples.iter().map(|s| s.complete).collect();
    json!({
        "first_ms": (!first.is_empty()).then(|| benchmark_percentiles(&first)),
        "complete_ms": benchmark_percentiles(&complete),
        "raw": samples.iter().map(|s| json!({"first_ms":s.first,"complete_ms":s.complete,"usage":s.usage})).collect::<Vec<_>>(),
    })
}

#[test]
#[ignore = "release-binary mixed-project PTY baseline; run explicitly with --ignored --nocapture"]
fn mixed_project_search_samples() {
    let (binary, sha, _) = benchmark_binary();
    let f = Fixture::new();
    for case in [
        Case::Rare,
        Case::Absent,
        Case::Shared,
        Case::Common,
        Case::Files,
    ] {
        let mut cold = Vec::with_capacity(RUNS);
        let mut warm = Vec::with_capacity(RUNS);
        for iteration in 0..RUNS {
            let mut tui = f.spawn(&binary);
            tui.until(|s| s.contains("NORMAL"));
            let (sample, _) = measure(&mut tui, &f, case);
            cold.push(sample);
            dismiss(&mut tui, case);
            let (sample, mut screen) = measure(&mut tui, &f, case);
            warm.push(sample);
            if case == Case::Absent {
                // The retained search was dismissed in field-normal mode.
                tui.send(b"\x1b");
                tui.until(|s| !s.contains(case.title()) && s.contains("NORMAL"));
            } else {
                // Exercise every collision target, not just the best one.
                if case == Case::Shared {
                    let target = SHARED_PATHS[iteration % SHARED_PATHS.len()];
                    for _ in 0..2 {
                        let current = selected(&screen, case, f.common).unwrap().to_owned();
                        if current == target {
                            break;
                        }
                        tui.send(b"\x0e"); // Ctrl-N
                        screen = tui
                            .until(|s| selected(s, case, f.common).is_some_and(|p| p != current));
                    }
                    assert_eq!(selected(&screen, case, f.common), Some(target));
                }
                accept(&mut tui, &f, case, &screen);
            }
            tui.send(b":qa!\r");
            tui.expect_clean_exit("mixed search editor failed");
        }
        println!(
            "STROP_MIXED_SEARCH={}",
            json!({
                "case":case.name(),"binary_sha256":sha,"query":case.query(&f),"runs":RUNS,
                "cold_process":summary(&cold),"warm_same_process":summary(&warm),
            })
        );
    }
}

#[test]
#[ignore = "release-binary mixed-project input under search load; run explicitly with --ignored --nocapture"]
fn mixed_project_search_cancellation_samples() {
    let (binary, sha, _) = benchmark_binary();
    let f = Fixture::new();
    let replacement = format!("\x15\x1b[200~{}\x1b[201~", f.rare);
    let mut input = Vec::with_capacity(RUNS);
    let mut complete = Vec::with_capacity(RUNS);
    let mut usage = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let mut tui = f.spawn(&binary);
        tui.until(|s| s.contains("NORMAL"));
        let before = Usage::read(&tui);
        tui.send(b" /\x1b[200~ordinary project\x1b[201~");
        // The exact query's streaming count, not a capped/settled report,
        // is the input barrier. The first ranked batch need not be painted
        // before the cap; waiting for that row would miss active searches.
        tui.until(|s| {
            has_query(s, Case::Rare, "ordinary project")
                && s.lines().next().unwrap().trim_end().ends_with("… ╮")
        });
        let start = Instant::now();
        tui.send(replacement.as_bytes());
        tui.until(|s| {
            has_query(s, Case::Rare, f.rare) && !column(s).any(|r| r.contains("ordinary project"))
        });
        input.push(elapsed_ms(start));
        let screen = tui.until(|s| {
            has_query(s, Case::Rare, f.rare)
                && s.lines()
                    .next()
                    .unwrap()
                    .trim_end()
                    .ends_with(Case::Rare.terminal())
                && selected(s, Case::Rare, f.rare) == Some(RARE_PATH)
        });
        complete.push(elapsed_ms(start));
        usage.push(Usage::read(&tui).report_since(&before));
        accept(&mut tui, &f, Case::Rare, &screen);
        tui.send(b":qa!\r");
        tui.expect_clean_exit("cancelled search editor failed");
    }
    println!(
        "STROP_MIXED_SEARCH_CANCEL={}",
        json!({
            "binary_sha256":sha,"runs":RUNS,"superseding_query_to_paint_ms":benchmark_percentiles(&input),
            "superseding_query_to_complete_ms":benchmark_percentiles(&complete),
            "raw_paint_ms":input,"raw_complete_ms":complete,"usage_including_old_query":usage,
        })
    );
}
