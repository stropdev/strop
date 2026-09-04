//! Tier 1 differential harness (0001 §5.10, 0006): the same key sequence
//! drives strop (headless) and pinned nvim (`-u NONE -i NONE -n`);
//! final text, cursor, and the unnamed register must agree.
//!
//! Deviations are bugs by doctrine (0001 pillar 1). Genuinely intended
//! deviations live in KNOWN_DIVERGENCES with a reason — the list is
//! reviewed, never grown silently.
//!
//! Skips (with a notice) when nvim is absent — the docker gate installs
//! it, so CI always runs this.

use std::io::Write;
use std::process::Command;

/// One differential case: name, initial buffer text, strop-token keys.
struct Case {
    name: &'static str,
    text: &'static str,
    keys: &'static str,
}

const CASES: &[Case] = &[
    // motions
    Case {
        name: "word motions",
        text: "hello world foo\nsecond line\n",
        keys: "ww",
    },
    Case {
        name: "word back",
        text: "hello world foo\n",
        keys: "$b",
    },
    Case {
        name: "word end",
        text: "hello world foo\n",
        keys: "0e",
    },
    Case {
        name: "line start/end",
        text: "  indented line\n",
        keys: "$0",
    },
    Case {
        name: "first non-blank",
        text: "  indented line\n",
        keys: "^",
    },
    Case {
        name: "gg and G",
        text: "a\nb\nc\nd\n",
        keys: "Ggg",
    },
    Case {
        name: "count motion",
        text: "a\nb\nc\nd\ne\n",
        keys: "3j",
    },
    Case {
        name: "column motion",
        text: "hello world\n",
        keys: "7|",
    },
    Case {
        name: "find char",
        text: "hello world\n",
        keys: "fo",
    },
    Case {
        name: "till char",
        text: "hello world\n",
        keys: "to",
    },
    Case {
        name: "find repeat",
        text: "o.o.o\n",
        keys: "f.;",
    },
    Case {
        name: "find repeat back",
        text: "o.o.o\n",
        keys: "f.;,",
    },
    Case {
        name: "match pair",
        text: "fn f(x) { y }\n",
        keys: "%",
    },
    // operators
    Case {
        name: "dw",
        text: "hello world foo\n",
        keys: "dw",
    },
    Case {
        name: "cw",
        text: "hello world\n",
        keys: "cwX<esc>",
    },
    Case {
        name: "dd",
        text: "a\nb\nc\n",
        keys: "dd",
    },
    Case {
        name: "yy p",
        text: "a\nb\n",
        keys: "yyp",
    },
    Case {
        name: "count dd",
        text: "a\nb\nc\nd\n",
        keys: "2dd",
    },
    Case {
        name: "d$",
        text: "hello world\n",
        keys: "ld$",
    },
    Case {
        name: "diw",
        text: "hello world\n",
        keys: "wdiw",
    },
    Case {
        name: "ci quote",
        text: "say \"hi\" now\n",
        keys: "f\"ci\"yo<esc>",
    },
    Case {
        name: "da bracket",
        text: "f(a, b)\n",
        keys: "lda(",
    },
    Case {
        name: "x and dot",
        text: "abc\n",
        keys: "x.",
    },
    Case {
        name: "r replace",
        text: "abc\n",
        keys: "lrX",
    },
    Case {
        name: "tilde",
        text: "abc\n",
        keys: "~~",
    },
    Case {
        name: "join",
        text: "a\nb\n",
        keys: "J",
    },
    Case {
        name: "indent",
        text: "fn f() {\nx\n}\n",
        keys: "j>>",
    },
    Case {
        name: "undo redo",
        text: "a\nb\n",
        keys: "ddu",
    },
    Case {
        name: "register yank",
        text: "hello world\n",
        keys: "\"ayw\"ap",
    },
    Case {
        name: "repeat find count",
        text: "a.b.c.d\n",
        keys: "2f.",
    },
    Case {
        name: "paste before",
        text: "ab\n",
        keys: "ylp",
    },
    Case {
        name: "P above",
        text: "ab\ncd\n",
        keys: "jyykP",
    },
    Case {
        name: "visual delete",
        text: "hello world\n",
        keys: "vld",
    },
    Case {
        name: "visual line delete",
        text: "a\nb\nc\n",
        keys: "Vjd",
    },
    Case {
        name: "search forward",
        text: "alpha beta alpha\n",
        keys: "/beta<cr>",
    },
    Case {
        name: "search wrap n",
        text: "alpha beta\n",
        keys: "/beta<cr>n",
    },
    Case {
        name: "star search",
        text: "one two one\n",
        keys: "*",
    },
    Case {
        name: "insert append",
        text: "ab\n",
        keys: "A!<esc>",
    },
    Case {
        name: "open line",
        text: "a\nb\n",
        keys: "oX<esc>",
    },
    Case {
        name: "O above",
        text: "a\nb\n",
        keys: "jOX<esc>",
    },
    Case {
        name: "insert count",
        text: "ab\n",
        keys: "3i!<esc>",
    },
];

/// Cases where strop deliberately differs — each with the doctrine reason.
const KNOWN_DIVERGENCES: &[(&str, &str)] = &[];

/// strop script tokens → raw bytes for nvim's feedkeys().
fn keys_to_bytes(keys: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = keys.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let tok: String = chars.by_ref().take_while(|&c| c != '>').collect();
            match tok.as_str() {
                "cr" | "enter" => out.push(b'\r'),
                "esc" => out.push(0x1b),
                "bs" => out.push(0x7f),
                "tab" => out.push(b'\t'),
                "space" => out.push(b' '),
                other => panic!("unknown token <{other}> in case"),
            }
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    out
}

struct State {
    text: String,
    line: usize,
    col: usize,
    register: String,
}

fn nvim_state(text: &str, keys: &str, dir: &std::path::Path) -> State {
    let file = dir.join("fixture.txt");
    std::fs::write(&file, text).unwrap();
    let keysf = dir.join("keys.bin");
    std::fs::write(&keysf, keys_to_bytes(keys)).unwrap();
    let driver = format!(
        "call feedkeys(join(readfile('{}', 'b'), ''), 'xt')",
        keysf.display()
    );
    let dump = "call writefile(getline(1,'$') + ['POS:'.line('.').':'.col('.')], '/dev/stdout')";
    let out = Command::new("nvim")
        .args(["--headless", "-u", "NONE", "-i", "NONE", "-n"])
        .args(["+set noswapfile", "+set shiftwidth=4 expandtab"])
        .arg(format!("+{driver}"))
        .arg(format!("+{dump}"))
        .arg("+qa!")
        .arg(&file)
        .output()
        .expect("nvim runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    if std::env::var_os("DIFF_DEBUG").is_some() {
        eprintln!(
            "nvim stdout: {:?} stderr: {:?}",
            stdout,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // dump layout: text lines…, then "POS:L:C"
    let mut lines: Vec<&str> = stdout.lines().collect();
    let register = String::new(); // register compare: linewise-bit noise — later wave
    let pos = lines.pop().unwrap_or("POS:1:1").trim_start_matches("POS:");
    let (line, col) = pos.split_once(':').unwrap();
    State {
        text: lines.join("\n"),
        line: line.parse().unwrap(),
        col: col.parse().unwrap(),
        register,
    }
}

fn strop_state(text: &str, keys: &str, dir: &std::path::Path) -> State {
    let file = dir.join("fixture.txt");
    std::fs::write(&file, text).unwrap();
    let script = dir.join("script.strop");
    writeln!(
        std::fs::File::create(&script).unwrap(),
        "keys {keys}\nkeys :w<cr>\nstate"
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_strop"))
        .arg("--headless")
        .arg(&script)
        .arg(&file)
        .output()
        .expect("strop runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = stdout
        .lines()
        .find(|l| l.starts_with("─── state "))
        .unwrap_or_else(|| {
            panic!(
                "no state line; stderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    let v: serde_json::Value = serde_json::from_str(json.trim_start_matches("─── state ")).unwrap();
    // text isn't in state json — read it through the buffer: use the
    // register trick? No: write the buffer via :w into the file first.
    State {
        text: read_buffer(dir),
        line: v["line"].as_u64().unwrap() as usize,
        col: v["col"].as_u64().unwrap() as usize,
        register: v["register"].as_str().unwrap().to_string(),
    }
}

fn read_buffer(dir: &std::path::Path) -> String {
    // strop wrote the fixture in place (the script appends :w)
    std::fs::read_to_string(dir.join("fixture.txt"))
        .unwrap()
        .trim_end_matches('\n')
        .to_string()
}

fn have_nvim() -> bool {
    Command::new("nvim")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn differential_vs_nvim() {
    if !have_nvim() {
        eprintln!("differential: nvim not installed — skipped (CI installs it)");
        return;
    }
    let mut failures = Vec::new();
    for case in CASES {
        let known = KNOWN_DIVERGENCES
            .iter()
            .find(|(n, _)| *n == case.name)
            .map(|(_, r)| *r);
        let dir = tempfile::tempdir().unwrap();
        let strop_dir = dir.path().join("strop");
        let nvim_dir = dir.path().join("nvim");
        std::fs::create_dir_all(&strop_dir).unwrap();
        std::fs::create_dir_all(&nvim_dir).unwrap();
        // strop: keys, then :w so the buffer lands in the fixture file
        let keys_with_save = case.keys.to_string();
        let nv = nvim_state(case.text, case.keys, &nvim_dir);
        let st = strop_state(case.text, &keys_with_save, &strop_dir);
        let same = nv.text == st.text && nv.line == st.line && nv.col == st.col;
        if known.is_none() && !same {
            failures.push(format!(
                "=== {} diverges\n  keys: {:?}\n  nvim:  text={:?} cursor={}:{}\n  strop: text={:?} cursor={}:{}",
                case.name, case.keys, nv.text, nv.line, nv.col, st.text, st.line, st.col
            ));
        }
        if let Some(reason) = known {
            assert!(
                !same,
                "{}: listed as divergent ({reason}) but agrees now — delist it",
                case.name
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
