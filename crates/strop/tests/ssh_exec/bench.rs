//! Remote latency over plain `ssh` (plan 0090 §7): the editor's own
//! paths — `RemoteClient` open and save, `rg` and `git` as exec channels,
//! a terminal keystroke echo through `ssh -tt` on a local PTY, and a first
//! connect — against a real sshd. A measurement, not an assert: opt in
//! with `STROP_SSH_EXEC_BENCH=1` (plus `STROP_REQUIRE_SSH_TESTS=1`); the
//! compose `ssh-bench` service runs it in release, `DELAY_MS` adding
//! netem delay each way. Prints a markdown table and the raw samples as
//! `STROP_SSH_EXEC_BENCH=<json>`.
use super::*;
use std::ffi::OsStr;
use std::io::{BufRead as _, Write as _};
use std::sync::mpsc;
use strop_remote::exec::ExecHost;
use strop_remote::{Expect, RemoteClient};
use strop_workspace::RemoteFile;

const WARMUP: usize = 3;
const PATTERN: &str = "RemoteEndpoint";
const BLAMED: &str = "crates/strop-remote/src/ssh.rs";
const STALL: Duration = Duration::from_secs(30);

struct Row {
    name: &'static str,
    samples: Vec<f64>,
}

fn percentile(samples: &[f64], at: usize) -> f64 {
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    ordered[(at * ordered.len()).div_ceil(100).max(1) - 1]
}

fn ms(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0
}

fn git(workspace: &Path, args: &[&str]) {
    successful(
        Command::new("git")
            .args(["-c", "user.name=bench", "-c", "user.email=bench@invalid"])
            .args(args)
            .current_dir(workspace),
    );
}

/// The repository sources as a searchable workspace, a 1 MB text file,
/// and a file with 30 commits of history.
fn workspace(root: &Path) -> PathBuf {
    let workspace = root.join("ws");
    std::fs::create_dir(&workspace).unwrap();
    successful(
        Command::new("cp")
            .arg("-R")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
            .arg(workspace.join("crates")),
    );
    let mut text = String::new();
    let mut line = 0;
    while text.len() < 1 << 20 {
        text.push_str(&format!(
            "{line:07} the quick brown fox jumps over the lazy dog {}\n",
            "x".repeat(line % 40)
        ));
        line += 1;
    }
    text.truncate(1 << 20);
    for name in ["big.txt", "save.txt"] {
        std::fs::write(workspace.join(name), &text).unwrap();
    }
    git(&workspace, &["init", "-q"]);
    git(&workspace, &["add", "-A"]);
    git(&workspace, &["commit", "-qm", "import"]);
    for revision in 0..30 {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(workspace.join(BLAMED))
            .unwrap();
        writeln!(file, "// revision {revision}").unwrap();
        git(
            &workspace,
            &["commit", "-qam", &format!("revision {revision}")],
        );
    }
    workspace
}

fn os<'a>(words: &[&'a str]) -> Vec<&'a OsStr> {
    words.iter().map(|word| OsStr::new(*word)).collect()
}

fn finite(host: &ExecHost, argv: &[&str], cwd: &Path) -> f64 {
    let started = Instant::now();
    let output = host.command(&os(argv), Some(cwd)).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!output.stdout.is_empty());
    ms(started)
}

/// Pump a blocking reader onto a channel so every wait is bounded.
fn pump(mut reader: impl std::io::Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    receiver
}

/// Accumulate output until `seen` holds; a stall names what arrived.
fn wait_for(receiver: &mpsc::Receiver<Vec<u8>>, seen: impl Fn(&[u8]) -> bool) {
    let mut output = Vec::new();
    while !seen(&output) {
        match receiver.recv_timeout(STALL) {
            Ok(bytes) => output.extend(bytes),
            Err(_) => panic!(
                "terminal output stalled after {:?}",
                String::from_utf8_lossy(&output)
            ),
        }
    }
}

/// One local PTY pair (controller, peer) at 80x24.
fn local_pty() -> (std::fs::File, std::fs::File) {
    use std::os::fd::FromRawFd as _;
    let (mut controller, mut peer) = (-1, -1);
    let size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty writes two fresh descriptors into the out
    // pointers; name and termios are optional (null) and the winsize is
    // a live local. std has no PTY API.
    let status = unsafe {
        libc::openpty(
            &mut controller,
            &mut peer,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    };
    assert_eq!(status, 0, "openpty: {}", std::io::Error::last_os_error());
    // SAFETY: both descriptors were just returned by openpty and are
    // owned by nothing else; each File takes sole ownership.
    unsafe {
        (
            std::fs::File::from_raw_fd(controller),
            std::fs::File::from_raw_fd(peer),
        )
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn remote_latency_over_real_sshd() {
    if std::env::var_os("STROP_SSH_EXEC_BENCH").as_deref() != Some(OsStr::new("1")) {
        writeln!(
            std::io::stderr().lock(),
            "skipped remote latency benchmark: set STROP_SSH_EXEC_BENCH=1 (and STROP_REQUIRE_SSH_TESTS=1) to run it"
        )
        .unwrap();
        return;
    }
    let Some(scope) = serial() else { return };
    let fixture = &scope.fixture;
    let samples: usize = std::env::var("STROP_SSH_EXEC_BENCH_N")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    let (token, _owner) = token();
    let ws = workspace(fixture.root());
    let host_endpoint = endpoint("fixture");

    // First connect: master authentication, the SFTP channel and one
    // listing; the master is stopped between samples outside the timer.
    let mut connect = Row {
        name: "first connect (auth + SFTP + list)",
        samples: Vec::new(),
    };
    let listed: strop_workspace::RemoteLocation =
        RemoteFile::from_path(host_endpoint.clone(), ws.clone())
            .unwrap()
            .into();
    for index in 0..(WARMUP + samples.min(10)) {
        let client = RemoteClient::with_control_dir(fixture.control());
        let started = Instant::now();
        client.list(&listed, &token).unwrap();
        let elapsed = ms(started);
        if index >= WARMUP {
            connect.samples.push(elapsed);
        }
        client.disconnect_all().unwrap();
    }

    let client = RemoteClient::with_control_dir(fixture.control());
    let host = client.host(&host_endpoint, &token).unwrap();
    let file = |name: &str| RemoteFile::from_path(host_endpoint.clone(), ws.join(name)).unwrap();
    let mut open = Row {
        name: "open a 1 MB file",
        samples: Vec::new(),
    };
    let mut save = Row {
        name: "save a 1 MB file (changed-on-disk checked)",
        samples: Vec::new(),
    };
    let mut search = Row {
        name: "rg --json first result",
        samples: Vec::new(),
    };
    let mut log = Row {
        name: "git log -n 50 -- file",
        samples: Vec::new(),
    };
    let mut blame = Row {
        name: "git blame --porcelain file",
        samples: Vec::new(),
    };
    let mut stamp = client
        .read(&file("save.txt").into(), 1 << 28, &token)
        .unwrap()
        .stamp;
    for index in 0..(WARMUP + samples) {
        let keep = index >= WARMUP;
        let started = Instant::now();
        let loaded = client
            .read(&file("big.txt").into(), 1 << 28, &token)
            .unwrap();
        let elapsed = ms(started);
        assert_eq!(loaded.buffer.len_bytes(), 1 << 20);
        if keep {
            open.samples.push(elapsed);
        }
        // Rewrite in place with a fresh size each time: the stamp must
        // move even within one whole second.
        let content = vec![b'a' + (index % 26) as u8; (1 << 20) - index];
        let started = Instant::now();
        stamp = client
            .save(&file("save.txt"), content, Expect::Unchanged(stamp), &token)
            .unwrap();
        if keep {
            save.samples.push(ms(started));
        }
        let started = Instant::now();
        let mut child = host
            .command(&os(&["rg", "--json", PATTERN, "."]), Some(&ws))
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut lines = std::io::BufReader::new(stdout).lines();
        let first = loop {
            let line = lines.next().expect("rg reports a match").unwrap();
            if line.contains("\"type\":\"match\"") {
                break ms(started);
            }
        };
        for line in lines {
            line.unwrap();
        }
        assert!(child.wait().unwrap().success());
        if keep {
            search.samples.push(first);
        }
        let elapsed = finite(
            &host,
            &["git", "log", "-n", "50", "--format=%H %s", "--", BLAMED],
            &ws,
        );
        if keep {
            log.samples.push(elapsed);
        }
        let elapsed = finite(&host, &["git", "blame", "--porcelain", "--", BLAMED], &ws);
        if keep {
            blame.samples.push(elapsed);
        }
    }

    // `:terminal` is `ssh -tt` on a local PTY: one keystroke until its
    // echo returns. The line is cancelled outside the timer before it
    // can wrap.
    let mut echo = Row {
        name: ":terminal keystroke echo",
        samples: Vec::new(),
    };
    let (local, peer) = local_pty();
    let mut shell = host
        .terminal(&os(&["env", "PS1=$ ", "/bin/sh"]), Some(&ws))
        .stdin(Stdio::from(peer.try_clone().unwrap()))
        .stdout(Stdio::from(peer.try_clone().unwrap()))
        .stderr(Stdio::from(peer))
        .spawn()
        .unwrap();
    let mut shell_in = local.try_clone().unwrap();
    let shell_out = pump(local);
    wait_for(&shell_out, |seen| contains(seen, b"$ "));
    shell_in.write_all(b"echo READY\n").unwrap();
    wait_for(&shell_out, |seen| contains(seen, b"\nREADY"));
    for index in 0..(WARMUP + samples) {
        let started = Instant::now();
        shell_in.write_all(b"x").unwrap();
        wait_for(&shell_out, |seen| seen.contains(&b'x'));
        if index >= WARMUP {
            echo.samples.push(ms(started));
        }
        if index % 16 == 15 || index + 1 == WARMUP + samples {
            shell_in.write_all(b"\x03").unwrap();
            wait_for(&shell_out, |seen| contains(seen, b"$ "));
        }
    }
    shell_in.write_all(b"exit 0\n").unwrap();
    assert!(shell.wait().unwrap().success());
    client.disconnect_all().unwrap();

    let rows = [open, save, search, log, blame, echo, connect];
    let mut table = String::from("| Operation | p50 / p95 ms | N |\n|---|---|---|\n");
    for row in &rows {
        table.push_str(&format!(
            "| {} | {:.1} / {:.1} | {} |\n",
            row.name,
            percentile(&row.samples, 50),
            percentile(&row.samples, 95),
            row.samples.len()
        ));
    }
    let report = serde_json::json!({
        "warmup": WARMUP,
        "rows": rows.iter().map(|row| serde_json::json!({
            "operation": row.name,
            "ms": row.samples,
        })).collect::<Vec<_>>(),
    });
    println!("\n{table}");
    println!("STROP_SSH_EXEC_BENCH={report}");
}
