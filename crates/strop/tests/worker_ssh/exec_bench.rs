//! Plan 0090 §5 go/no-go: the same editor operations through today's
//! deployed worker and through plain `ssh` exec + SFTP on one master,
//! against the same real sshd, interleaved sample by sample. A
//! measurement, not an assert: opt in with `STROP_SSH_EXEC_BENCH=1`
//! (plus the fixture's `STROP_REQUIRE_SSH_TESTS=1`), run in release so
//! both the worker and the editor side are the shipped code. Prints a
//! markdown table and the raw samples as `STROP_SSH_EXEC_BENCH=<json>`.
use super::*;
use std::ffi::OsStr;
use std::io::{BufRead as _, Read as _, Write as _};
use std::sync::mpsc;
use strop_remote::exec::{ExecHost, Expect, RemoteFiles};
use strop_worker_client::{ExecEvent, ExecHandle, StreamEvent};
use strop_worker_protocol::{ExecSpec, ExitStatus, PtyGeometry};

const WARMUP: usize = 3;
const PATTERN: &str = "RemoteEndpoint";
const BLAMED: &str = "crates/strop-remote/src/ssh.rs";
const STALL: Duration = Duration::from_secs(30);

/// The §5 pass rule for one row.
#[derive(Clone, Copy)]
enum Gate {
    /// exec median and p95 each within worker's plus this many ms.
    Added(f64),
    /// exec p95 within worker p95 plus this many ms.
    AddedP95(f64),
    /// exec median below worker median.
    Faster,
    /// Context only.
    Info,
}

struct Row {
    name: &'static str,
    gate: Gate,
    worker: Vec<f64>,
    exec: Vec<f64>,
}

fn percentile(samples: &[f64], at: usize) -> f64 {
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    ordered[(at * ordered.len()).div_ceil(100).max(1) - 1]
}

fn ms(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0
}

impl Row {
    fn new(name: &'static str, gate: Gate) -> Self {
        Self {
            name,
            gate,
            worker: Vec::new(),
            exec: Vec::new(),
        }
    }

    fn verdict(&self) -> &'static str {
        let (w50, w95) = (percentile(&self.worker, 50), percentile(&self.worker, 95));
        let (e50, e95) = (percentile(&self.exec, 50), percentile(&self.exec, 95));
        let pass = match self.gate {
            Gate::Added(budget) => e50 <= w50 + budget && e95 <= w95 + budget,
            Gate::AddedP95(budget) => e95 <= w95 + budget,
            Gate::Faster => e50 < w50,
            Gate::Info => return "—",
        };
        if pass {
            "PASS"
        } else {
            "FAIL"
        }
    }

    fn markdown(&self) -> String {
        let budget = match self.gate {
            Gate::Added(budget) => format!("+{budget} ms"),
            Gate::AddedP95(budget) => format!("+{budget} ms p95"),
            Gate::Faster => "faster".into(),
            Gate::Info => "—".into(),
        };
        format!(
            "| {} | {:.1} / {:.1} | {:.1} / {:.1} | {:+.1} / {:+.1} | {} | {} |",
            self.name,
            percentile(&self.worker, 50),
            percentile(&self.worker, 95),
            percentile(&self.exec, 50),
            percentile(&self.exec, 95),
            percentile(&self.exec, 50) - percentile(&self.worker, 50),
            percentile(&self.exec, 95) - percentile(&self.worker, 95),
            budget,
            self.verdict()
        )
    }
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
    for name in ["big.txt", "save-worker.txt", "save-exec.txt"] {
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

fn spec(program: &str, argv: &[&str], cwd: &Path, service: bool) -> ExecSpec {
    ExecSpec {
        program: program.as_bytes().to_vec(),
        argv: argv.iter().map(|word| word.as_bytes().to_vec()).collect(),
        cwd: cwd.as_os_str().as_encoded_bytes().to_vec(),
        env: Vec::new(),
        service,
        pty: None,
    }
}

fn os<'a>(words: &[&'a str]) -> Vec<&'a OsStr> {
    words.iter().map(|word| OsStr::new(*word)).collect()
}

/// Read `lines` until one is a `rg --json` match; return its latency,
/// then drain the rest.
fn first_match(started: Instant, reader: impl std::io::Read) -> f64 {
    let mut lines = std::io::BufReader::new(reader).lines();
    let first = loop {
        let line = lines.next().expect("rg reports a match").unwrap();
        if line.contains("\"type\":\"match\"") {
            break ms(started);
        }
    };
    for line in lines {
        line.unwrap();
    }
    first
}

fn worker_finite(worker: &Worker, token: &strop_core::worker::CancelToken, spec: ExecSpec) -> f64 {
    let started = Instant::now();
    let mut handle: ExecHandle = worker.exec(token, spec).unwrap();
    let mut output = Vec::new();
    handle.stdout().read_to_end(&mut output).unwrap();
    assert_eq!(handle.wait_exit(), ExitStatus::Exit(0));
    assert!(!output.is_empty());
    ms(started)
}

fn exec_finite(host: &ExecHost, argv: &[&str], cwd: &Path) -> f64 {
    let started = Instant::now();
    let output = host.command(&os(argv), Some(cwd)).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!output.stdout.is_empty());
    ms(started)
}

fn lsp_frame() -> Vec<u8> {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///ws/crates/strop-remote/src/ssh.rs"},"position":{"line":40,"character":12}}}"#;
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
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
fn wait_for(side: &str, next: &mut dyn FnMut() -> Option<Vec<u8>>, seen: impl Fn(&[u8]) -> bool) {
    let mut output = Vec::new();
    while !seen(&output) {
        match next() {
            Some(bytes) => output.extend(bytes),
            None => panic!(
                "{side} terminal output stalled after {:?}",
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

/// Remove the deployed object and receipt so the next deploy uploads.
fn evict(provider: &SftpDeployProvider, object: &strop_worker_deploy::deploy::VerifiedObject) {
    let layout = strop_worker_deploy::cache::resolve(provider).unwrap();
    for _ in 0..1024 {
        if provider.list(&layout.leases_dir()).unwrap().is_empty() {
            break;
        }
    }
    provider.remove(&layout.receipt(&object.sha256)).unwrap();
    provider.remove(&object.path).unwrap();
}

#[test]
fn worker_versus_exec_latency_over_real_sshd() {
    if std::env::var_os("STROP_SSH_EXEC_BENCH").as_deref() != Some(OsStr::new("1")) {
        writeln!(
            std::io::stderr().lock(),
            "skipped worker-versus-exec benchmark: set STROP_SSH_EXEC_BENCH=1 (and STROP_REQUIRE_SSH_TESTS=1) to run it"
        )
        .unwrap();
        return;
    }
    let Some(host_fixture) = serial() else {
        panic!("the benchmark needs the sshd fixture: set STROP_REQUIRE_SSH_TESTS=1");
    };
    let fixture = &host_fixture.fixture;
    let samples: usize = std::env::var("STROP_SSH_EXEC_BENCH_N")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    // Cold connects upload the whole artifact; under injected delay
    // each takes tens of seconds, so their count is separate.
    let connects: usize = std::env::var("STROP_SSH_EXEC_BENCH_CONNECTS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(samples);
    let connect_runs = if connects == 0 { 0 } else { WARMUP + connects };
    let host = endpoint("fixture");
    let control = fixture.root().join("ctl");
    let (token, _owner) = token();
    let ws = workspace(fixture.root());
    let artifact_bytes = std::fs::metadata(&fixture.artifact).unwrap().len();

    // First connect to a fresh host: the worker uploads and verifies its
    // artifact, launches and lists; exec authenticates a master, opens
    // SFTP and lists. Cache eviction and master teardown happen
    // outside the timer.
    let mut cold = Row::new("first connect, fresh host", Gate::Faster);
    let mut warm = Row::new("reconnect (worker already cached)", Gate::Info);
    let mut deployed = None;
    for index in 0..connect_runs {
        if let Some((provider, object)) = deployed.take() {
            evict(&provider, &object);
        }
        let started = Instant::now();
        let facts = discover(&host, &token);
        let (provider, ready) = deploy_worker(
            fixture,
            &host,
            &facts,
            Consent::Granted {
                action: "exec-bench".into(),
            },
            &token,
        );
        assert_eq!(ready.origin, DeployOrigin::Uploaded);
        let worker = worker_transport::worker(&host, &ready.object.path, &facts.target);
        worker.health(&token).unwrap();
        worker
            .list(&token, ResourceLocation::local(ws.clone()))
            .unwrap();
        let worker_ms = ms(started);
        worker.shutdown().unwrap();

        let started = Instant::now();
        let exec = ExecHost::connect(&host, &control).unwrap();
        let mut files = exec.files().unwrap();
        files.list(&ws).unwrap();
        let exec_ms = ms(started);
        drop(files);
        exec.disconnect().unwrap();
        writeln!(
            std::io::stderr().lock(),
            "first connect {index}: worker {worker_ms} ms, exec {exec_ms} ms"
        )
        .unwrap();
        if index >= WARMUP {
            cold.worker.push(worker_ms);
            cold.exec.push(exec_ms);
        }
        deployed = Some((provider, ready.object));
    }
    let (_provider, object) = deployed.unwrap_or_else(|| {
        let facts = discover(&host, &token);
        let (provider, ready) = deploy_worker(
            fixture,
            &host,
            &facts,
            Consent::Granted {
                action: "exec-bench".into(),
            },
            &token,
        );
        (provider, ready.object)
    });
    for index in 0..connect_runs {
        let started = Instant::now();
        let facts = discover(&host, &token);
        let (_provider, ready) = deploy_worker(fixture, &host, &facts, Consent::Absent, &token);
        assert_eq!(ready.origin, DeployOrigin::Reused);
        let worker = worker_transport::worker(&host, &ready.object.path, &facts.target);
        worker.health(&token).unwrap();
        worker
            .list(&token, ResourceLocation::local(ws.clone()))
            .unwrap();
        let worker_ms = ms(started);
        worker.shutdown().unwrap();
        let started = Instant::now();
        let exec = ExecHost::connect(&host, &control).unwrap();
        exec.files().unwrap().list(&ws).unwrap();
        let exec_ms = ms(started);
        exec.disconnect().unwrap();
        if index >= WARMUP {
            warm.worker.push(worker_ms);
            warm.exec.push(exec_ms);
        }
    }

    // Warm: one live worker, one live master and SFTP session.
    let facts = discover(&host, &token);
    let worker = worker_transport::worker(&host, &object.path, &facts.target);
    worker.health(&token).unwrap();
    let exec = ExecHost::connect(&host, &control).unwrap();
    let mut files: RemoteFiles = exec.files().unwrap();

    let mut open = Row::new("open a 1 MB file", Gate::Added(100.0));
    let mut save = Row::new("save a 1 MB file", Gate::Added(100.0));
    let mut search = Row::new("rg --json first result", Gate::Added(150.0));
    let mut search_all = Row::new("rg --json complete", Gate::Info);
    let mut log = Row::new("git log -n 50 -- file", Gate::Added(150.0));
    let mut blame = Row::new("git blame --porcelain file", Gate::Added(150.0));
    let big = ws.join("big.txt");
    let save_worker = ws.join("save-worker.txt");
    let save_exec = ws.join("save-exec.txt");
    let variants: Vec<Vec<u8>> = (0..2)
        .map(|variant| {
            let mut bytes = std::fs::read(&big).unwrap();
            bytes[0] = b'A' + variant;
            bytes
        })
        .collect();
    let mut stamp = files.stat(&save_exec).unwrap().unwrap();
    let rg = ["rg", "--json", PATTERN, "."];
    let git_log = [
        "git",
        "log",
        "-n",
        "50",
        "--format=%H%x00%an%x00%at%x00%s",
        "--",
        BLAMED,
    ];
    let git_blame = ["git", "blame", "--porcelain", "--", BLAMED];
    let program = |name: &str| format!("/usr/bin/{name}");
    for index in 0..(WARMUP + samples) {
        let keep = index >= WARMUP;
        // open
        let started = Instant::now();
        let mut payload = worker
            .read(&token, ResourceLocation::local(big.clone()), 0, None)
            .unwrap();
        let mut bytes = Vec::new();
        payload.read_to_end(&mut bytes).unwrap();
        let worker_ms = ms(started);
        assert_eq!(bytes.len(), 1 << 20);
        let started = Instant::now();
        let loaded = files.read(&big, 1 << 28).unwrap();
        let exec_ms = ms(started);
        assert_eq!(loaded.bytes, bytes);
        if keep {
            open.worker.push(worker_ms);
            open.exec.push(exec_ms);
        }

        // save: the worker's Store intent with an mtime baseline, the
        // exec path's stamped temp+rename.
        let content = &variants[index % 2];
        let metadata = std::fs::metadata(&save_worker).unwrap();
        let baseline = strop_workspace::FileTime {
            seconds: metadata.mtime(),
            nanos: metadata.mtime_nsec() as u32,
        };
        let started = Instant::now();
        let intent = OperationIntent {
            kind: OperationKind::Store,
            source: None,
            destination: Some(ResourceLocation::local(save_worker.clone())),
            copy_version: strop_workspace::operation::CopyVersion::Stored,
            expected_content: Some({
                use sha2::Digest as _;
                sha2::Sha256::digest(content).into()
            }),
            store: Some(strop_workspace::operation::StorePolicy {
                baseline: Some(baseline),
                baseline_object: None,
                baseline_attributes: None,
                force: false,
                expect_absent: false,
                displayed: None,
            }),
        };
        let (steps, refused, session) = worker.prepare(&token, vec![intent], None).unwrap();
        assert!(refused.is_empty(), "{refused:?}");
        let receipts = worker.apply(&token, session, steps, Some(content)).unwrap();
        let worker_ms = ms(started);
        assert!(receipts[0].outcome.is_committed(), "{receipts:?}");
        let started = Instant::now();
        stamp = files
            .write(&save_exec, content, Expect::Unchanged(stamp))
            .unwrap();
        let exec_ms = ms(started);
        assert_eq!(&std::fs::read(&save_exec).unwrap(), content);
        if keep {
            save.worker.push(worker_ms);
            save.exec.push(exec_ms);
        }

        // rg: time to the first match, and to completion.
        let started = Instant::now();
        let mut handle = worker
            .exec(&token, spec(&program("rg"), &rg[1..], &ws, false))
            .unwrap();
        let worker_first = first_match(started, handle.stdout());
        assert_eq!(handle.wait_exit(), ExitStatus::Exit(0));
        let worker_all = ms(started);
        let started = Instant::now();
        let mut child = exec.exec(&os(&rg), Some(&ws)).unwrap();
        let exec_first = first_match(started, child.stdout.take().unwrap());
        assert!(child.wait().unwrap().success());
        let exec_all = ms(started);
        if keep {
            search.worker.push(worker_first);
            search.exec.push(exec_first);
            search_all.worker.push(worker_all);
            search_all.exec.push(exec_all);
        }

        // git log / blame to completion.
        let worker_ms = worker_finite(
            &worker,
            &token,
            spec(&program("git"), &git_log[1..], &ws, false),
        );
        let exec_ms = exec_finite(&exec, &git_log, &ws);
        if keep {
            log.worker.push(worker_ms);
            log.exec.push(exec_ms);
        }
        let worker_ms = worker_finite(
            &worker,
            &token,
            spec(&program("git"), &git_blame[1..], &ws, false),
        );
        let exec_ms = exec_finite(&exec, &git_blame, &ws);
        if keep {
            blame.worker.push(worker_ms);
            blame.exec.push(exec_ms);
        }
    }

    // LSP transport: a stdio server that echoes each framed request
    // (`cat`), so the round trip is the transport alone. Attach is the
    // spawn-to-first-reply time.
    let frame = lsp_frame();
    let mut attach = Row::new("LSP attach (spawn → first reply)", Gate::Info);
    let mut hover = Row::new("LSP round trip after attach", Gate::Added(50.0));
    for _ in 0..(WARMUP + samples) {
        let started = Instant::now();
        let handle = worker
            .exec(&token, spec("/bin/cat", &[], &ws, true))
            .unwrap();
        let (_exec, mut stdin, mut stdout, _stderr, exit) = handle.into_parts();
        let mut echoed = vec![0_u8; frame.len()];
        stdin.write(&frame, false).unwrap();
        stdout.read_exact(&mut echoed).unwrap();
        attach.worker.push(ms(started));
        assert_eq!(echoed, frame);
        let started = Instant::now();
        let mut child = exec.exec(&os(&["cat"]), Some(&ws)).unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        input.write_all(&frame).unwrap();
        output.read_exact(&mut echoed).unwrap();
        attach.exec.push(ms(started));
        assert_eq!(echoed, frame);
        if attach.exec.len() == WARMUP + samples {
            for index in 0..(WARMUP + samples) {
                let started = Instant::now();
                stdin.write(&frame, false).unwrap();
                stdout.read_exact(&mut echoed).unwrap();
                let worker_ms = ms(started);
                let started = Instant::now();
                input.write_all(&frame).unwrap();
                output.read_exact(&mut echoed).unwrap();
                let exec_ms = ms(started);
                if index >= WARMUP {
                    hover.worker.push(worker_ms);
                    hover.exec.push(exec_ms);
                }
            }
        }
        stdin.write(&[], true).unwrap();
        assert_eq!(exit.wait(), ExitStatus::Exit(0));
        drop(input);
        assert!(child.wait().unwrap().success());
    }
    attach.worker.drain(..WARMUP);
    attach.exec.drain(..WARMUP);

    // `:terminal` keystroke echo: one byte into an interactive /bin/sh
    // on a remote PTY until it comes back. A fixed prompt keeps the
    // workspace path (and any `x` in it) out of the output; the line is
    // cancelled outside the timer before it can wrap.
    let mut echo = Row::new(":terminal keystroke echo", Gate::AddedP95(20.0));
    let mut pty_spec = spec("/usr/bin/env", &["PS1=$ ", "/bin/sh"], &ws, false);
    pty_spec.pty = Some(PtyGeometry {
        columns: 80,
        rows: 24,
    });
    let mut session = worker.exec_pty(&token, pty_spec).unwrap();
    // The editor runs `ssh -tt` on a local PTY (that is how resizes and
    // control characters reach the remote one); so does the benchmark.
    let (local, peer) = local_pty();
    let mut shell = exec
        .terminal(&os(&["env", "PS1=$ ", "/bin/sh"]), Some(&ws))
        .stdin(Stdio::from(peer.try_clone().unwrap()))
        .stdout(Stdio::from(peer.try_clone().unwrap()))
        .stderr(Stdio::from(peer))
        .spawn()
        .unwrap();
    let mut shell_in = local.try_clone().unwrap();
    let shell_out = pump(local);
    let worker_out = |session: &strop_worker_client::PtySession| loop {
        match session.recv_output_timeout(STALL).ok()? {
            StreamEvent::Chunk(chunk) => return Some(chunk.bytes),
            StreamEvent::Resized => {}
            StreamEvent::Failed(reason) => panic!("worker PTY failed: {reason}"),
        }
    };
    let exec_out = || shell_out.recv_timeout(STALL).ok();
    // Typeahead sent before the shell configures its line editor can be
    // flushed, so wait for the first prompt on both sides.
    wait_for("worker prompt", &mut || worker_out(&session), |seen| {
        contains(seen, b"$ ")
    });
    wait_for("exec prompt", &mut || exec_out(), |seen| {
        contains(seen, b"$ ")
    });
    session.feed(b"echo READY\n").unwrap();
    wait_for("worker READY", &mut || worker_out(&session), |seen| {
        contains(seen, b"\nREADY")
    });
    shell_in.write_all(b"echo READY\n").unwrap();
    wait_for("exec READY", &mut || exec_out(), |seen| {
        contains(seen, b"\nREADY")
    });
    for index in 0..(WARMUP + samples) {
        let started = Instant::now();
        session.feed(b"x").unwrap();
        wait_for("worker echo", &mut || worker_out(&session), |seen| {
            seen.contains(&b'x')
        });
        let worker_ms = ms(started);
        let started = Instant::now();
        shell_in.write_all(b"x").unwrap();
        wait_for("exec echo", &mut || exec_out(), |seen| seen.contains(&b'x'));
        let exec_ms = ms(started);
        if index >= WARMUP {
            echo.worker.push(worker_ms);
            echo.exec.push(exec_ms);
        }
        if index % 16 == 15 || index + 1 == WARMUP + samples {
            session.feed(b"\x03").unwrap();
            wait_for("worker cancel", &mut || worker_out(&session), |seen| {
                contains(seen, b"$ ")
            });
            shell_in.write_all(b"\x03").unwrap();
            wait_for("exec cancel", &mut || exec_out(), |seen| {
                contains(seen, b"$ ")
            });
        }
    }
    session.feed(b"exit 0\n").unwrap();
    shell_in.write_all(b"exit 0\n").unwrap();
    assert!(shell.wait().unwrap().success());
    loop {
        match session
            .events()
            .recv_timeout(STALL)
            .expect("worker PTY exit")
        {
            ExecEvent::Exit(status) => break assert_eq!(status, ExitStatus::Exit(0)),
            ExecEvent::Input { .. } => {}
        }
    }
    drop(files);
    exec.disconnect().unwrap();
    worker.shutdown().unwrap();

    let rows = [
        open, save, search, search_all, log, blame, attach, hover, echo, cold, warm,
    ];
    let mut table = String::from(
        "| Operation | worker p50 / p95 ms | exec p50 / p95 ms | Δ p50 / p95 ms | Gate | Verdict |\n|---|---|---|---|---|---|\n",
    );
    for row in rows.iter().filter(|row| !row.exec.is_empty()) {
        table.push_str(&row.markdown());
        table.push('\n');
    }
    let failed = rows
        .iter()
        .filter(|row| !row.exec.is_empty())
        .filter(|row| row.verdict() == "FAIL")
        .count();
    let report = serde_json::json!({
        "samples": samples,
        "connects": connects,
        "warmup": WARMUP,
        "artifact_bytes": artifact_bytes,
        "rows": rows.iter().filter(|row| !row.exec.is_empty()).map(|row| serde_json::json!({
            "operation": row.name,
            "verdict": row.verdict(),
            "worker_ms": row.worker,
            "exec_ms": row.exec,
        })).collect::<Vec<_>>(),
    });
    println!(
        "\n{table}\nworker artifact {artifact_bytes} bytes; N={samples} after {WARMUP} warmup; gate rows failed: {failed}\n"
    );
    println!("STROP_SSH_EXEC_BENCH={report}");
}
