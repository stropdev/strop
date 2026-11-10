//! Tier 1 differential harness (0001 §5.10, 0006, 0095): the same key
//! sequence drives strop (headless) and pinned nvim (`-u NONE -i NONE
//! -n`); final text, cursor, mode, the unnamed register, the lifecycle
//! registers (`"0 "1 "2 "- "a "b`, plan 0092) and 'modified' must agree.
//!
//! The corpus is three lists: `hand` (cases somebody once got wrong), `ex`
//! (the command line) and `generated` (each operator × each motion or text
//! object × count × Normal, Visual and dot-repeat).
//!
//! Deviations are bugs by doctrine (0001 pillar 1). Intended ones live
//! in `divergences::KNOWN` with a reason — the list is capped, and a
//! listed case that agrees again fails the run.
//!
//! Skips (with a notice) when nvim is absent; `STROP_REQUIRE_NVIM=1`
//! turns that skip into a failure — the docker gate installs nvim and
//! sets it, so CI always runs this. `DIFF_ONLY=substring` runs only the
//! cases whose name contains it.

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

mod divergences;
mod ex;
mod generated;
mod hand;

/// A written case: name, initial buffer text, strop-token keys.
type Case = (&'static str, &'static str, &'static str);

/// One case as the runner sees it; generated cases own their strings.
pub struct Run {
    pub name: String,
    pub text: String,
    pub keys: String,
}

fn corpus() -> Vec<Run> {
    let written = |prefix: &str, cases: &[Case]| -> Vec<Run> {
        cases
            .iter()
            .map(|(name, text, keys)| Run {
                name: format!("{prefix}{name}"),
                text: (*text).to_owned(),
                keys: (*keys).to_owned(),
            })
            .collect()
    };
    let mut runs = written("", hand::CASES);
    runs.extend(written("ex/", ex::CASES));
    runs.extend(generated::cases());
    runs
}

/// strop script tokens → raw bytes for nvim's feedkeys(). As in strop's
/// own notation, an unknown `<token>` is literal text.
fn keys_to_bytes(keys: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = keys;
    while let Some(c) = rest.chars().next() {
        if let Some((byte, len)) = token(rest) {
            out.push(byte);
            rest = &rest[len..];
            continue;
        }
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A leading `<name>` token: its byte and its length in the source.
fn token(text: &str) -> Option<(u8, usize)> {
    let end = text.strip_prefix('<')?.find('>')?;
    let byte = match &text[1..=end] {
        "cr" | "enter" => b'\r',
        "esc" => 0x1b,
        "bs" => 0x7f,
        "tab" => b'\t',
        "space" => b' ',
        "lt" => b'<',
        control if control.len() == 3 && control.starts_with("c-") => {
            let key = control.as_bytes()[2];
            if !key.is_ascii_lowercase() {
                return None;
            }
            key - b'a' + 1
        }
        _ => return None,
    };
    Some((byte, end + 2))
}

#[derive(PartialEq, Eq)]
struct State {
    text: String,
    line: usize,
    col: usize,
    mode: String,
    register: String,
    registers: std::collections::BTreeMap<String, String>,
    modified: bool,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "text={:?} cursor={}:{} mode={} register={:?} registers={:?} modified={}",
            self.text, self.line, self.col, self.mode, self.register, self.registers, self.modified
        )
    }
}

/// No real `$HOME`: both editors see only the case's own directory.
fn hermetic(command: &mut Command, dir: &std::path::Path) {
    command
        .current_dir(dir)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data"));
}

/// Neovim's answer, or why there is none (a hung or failed oracle).
fn nvim_state(text: &str, keys: &str, dir: &std::path::Path) -> Result<State, String> {
    let file = dir.join("fixture.txt");
    std::fs::write(&file, text).unwrap();
    std::fs::write(dir.join("keys.bin"), keys_to_bytes(keys)).unwrap();
    let driver = "call feedkeys(join(readfile('keys.bin', 'b'), \"\\n\"), 'xt')";
    let dump = "call writefile(getline(1,'$') + ['MOD:'.&modified, 'MODE:'.mode(), 'POS:'.line('.').':'.col('.'), 'REGS:'.json_encode({'0':getreg('0'),'1':getreg('1'),'2':getreg('2'),'-':getreg('-'),'a':getreg('a'),'b':getreg('b')}), 'REG:'.json_encode(getreg('\"'))], 'dump.txt')";
    let mut command = Command::new("timeout");
    command
        .args(["10", "nvim", "--headless", "-u", "NONE", "-i", "NONE", "-n"])
        .args(["+set noswapfile", "+set shiftwidth=4 expandtab"])
        .arg(format!("+{driver}"))
        .arg(format!("+{dump}"))
        .arg("+qa!")
        .arg("fixture.txt");
    hermetic(&mut command, dir);
    let out = command.output().expect("nvim runs");
    let dumped = std::fs::read_to_string(dir.join("dump.txt")).map_err(|_| {
        format!(
            "no dump (status {}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )
    })?;
    // dump layout: text lines…, then "MOD:0|1", "MODE:m", "POS:L:C", the
    // JSON-encoded lifecycle registers and the unnamed register.
    let mut lines: Vec<&str> = dumped.lines().collect();
    let register = lines.pop().unwrap().strip_prefix("REG:").unwrap();
    let register: String = serde_json::from_str(register).unwrap();
    let registers = lines.pop().unwrap().strip_prefix("REGS:").unwrap();
    let registers = serde_json::from_str(registers).unwrap();
    let pos = lines.pop().unwrap().trim_start_matches("POS:");
    let (line, col) = pos.split_once(':').unwrap();
    let mode = match lines.pop().unwrap().strip_prefix("MODE:").unwrap() {
        "n" => "NORMAL",
        "v" => "VISUAL",
        "V" => "V-LINE",
        "\u{16}" => "V-BLOCK",
        "i" => "INSERT",
        other => other,
    }
    .to_owned();
    let modified = lines.pop().unwrap().strip_prefix("MOD:").unwrap() == "1";
    Ok(State {
        text: lines.join("\n"),
        line: line.parse().unwrap(),
        col: col.parse().unwrap(),
        mode,
        register,
        registers,
        modified,
    })
}

fn strop_state(text: &str, keys: &str, dir: &std::path::Path) -> Result<State, String> {
    let file = dir.join("fixture.txt");
    std::fs::write(&file, text).unwrap();
    let script = dir.join("script.strop");
    // Text is not in the state JSON: `:w` after the dump (so 'modified'
    // is observed before the save) writes the buffer into the fixture.
    writeln!(
        std::fs::File::create(&script).unwrap(),
        "resize 80 23\nkeys {keys}\nsettle 5000\nstate\nkeys <esc>:w<cr>\nsettle 5000"
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_strop"));
    command.arg("--headless").arg(&script).arg("fixture.txt");
    hermetic(&mut command, dir);
    let out = command.output().expect("strop runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = stdout
        .lines()
        .find(|l| l.starts_with("─── state "))
        .ok_or_else(|| {
            format!(
                "no state line; stderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        })?;
    let v: serde_json::Value = serde_json::from_str(json.trim_start_matches("─── state ")).unwrap();
    Ok(State {
        text: std::fs::read_to_string(&file)
            .unwrap()
            .trim_end_matches('\n')
            .to_string(),
        line: v["line"].as_u64().unwrap() as usize,
        col: v["col"].as_u64().unwrap() as usize,
        mode: v["mode"].as_str().unwrap().to_owned(),
        register: v["register"].as_str().unwrap().to_owned(),
        registers: serde_json::from_value(v["registers"].clone()).unwrap(),
        modified: v["dirty"].as_bool().unwrap(),
    })
}

fn have_nvim() -> bool {
    Command::new("nvim")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `Some(report)` when the two editors disagree on this case.
fn divergence(run: &Run) -> Option<String> {
    let dir = tempfile::tempdir().unwrap();
    let strop_dir = dir.path().join("strop");
    let nvim_dir = dir.path().join("nvim");
    std::fs::create_dir_all(&strop_dir).unwrap();
    std::fs::create_dir_all(&nvim_dir).unwrap();
    let nvim = nvim_state(&run.text, &run.keys, &nvim_dir);
    let strop = strop_state(&run.text, &run.keys, &strop_dir);
    if nvim.is_ok() && nvim == strop {
        return None;
    }
    let show = |state: &Result<State, String>| match state {
        Ok(state) => state.to_string(),
        Err(error) => format!("FAILED: {error}"),
    };
    Some(format!(
        "=== {} diverges\n  keys: {:?}\n  nvim:  {}\n  strop: {}",
        run.name,
        run.keys,
        show(&nvim),
        show(&strop)
    ))
}

#[test]
fn differential_vs_nvim() {
    if !have_nvim() {
        // The Docker gate installs nvim and sets STROP_REQUIRE_NVIM=1, so a
        // missing oracle there is a failure, never a quiet pass (0086 F3).
        assert!(
            std::env::var_os("STROP_REQUIRE_NVIM").as_deref() != Some(std::ffi::OsStr::new("1")),
            "differential: STROP_REQUIRE_NVIM=1 but nvim is not runnable"
        );
        // Straight to stderr: libtest captures eprintln!, which would hide it.
        writeln!(
            std::io::stderr().lock(),
            "differential: nvim not installed — skipped (set STROP_REQUIRE_NVIM=1 to require it)"
        )
        .unwrap();
        return;
    }
    let only = std::env::var("DIFF_ONLY").ok();
    let runs: Vec<Run> = corpus()
        .into_iter()
        .filter(|run| only.as_deref().is_none_or(|only| run.name.contains(only)))
        .collect();
    let mut names = std::collections::BTreeSet::new();
    for run in &runs {
        assert!(
            names.insert(run.name.as_str()),
            "duplicate case {}",
            run.name
        );
    }
    if only.is_none() {
        assert!(
            runs.len() >= divergences::MINIMUM_CASES,
            "the corpus shrank to {} cases",
            runs.len()
        );
        assert!(
            divergences::KNOWN.len() <= divergences::MAXIMUM_KNOWN,
            "{} known divergences: fix one before listing another",
            divergences::KNOWN.len()
        );
    }

    // One process pair per case; cases are independent, so they run on
    // every core.
    let next = AtomicUsize::new(0);
    let reports: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, usize::from);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(run) = runs.get(index) else {
                    return;
                };
                if let Some(report) = divergence(run) {
                    reports.lock().unwrap().push((index, report));
                }
            });
        }
    });
    let mut reports = reports.into_inner().unwrap();
    reports.sort();

    let mut failures = Vec::new();
    let mut explained = vec![0usize; divergences::KNOWN.len()];
    for (index, report) in &reports {
        match divergences::reason(&runs[*index].name) {
            Some(known) => explained[known] += 1,
            None => failures.push(report.as_str()),
        }
    }
    writeln!(
        std::io::stderr().lock(),
        "differential: {} cases, {} explained divergences, {} unexplained",
        runs.len(),
        reports.len() - failures.len(),
        failures.len()
    )
    .unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    if only.is_none() {
        for (known, count) in explained.iter().enumerate() {
            assert!(
                *count > 0,
                "{:?} is listed as divergent but every case agrees now — delist it",
                divergences::KNOWN[known].0
            );
        }
    }
}
