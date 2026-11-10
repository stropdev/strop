//! Spawn-level evidence against real child processes: the initialize
//! payload's workspace fields, the languages.toml config block answering
//! a workspace/configuration pull (0.21.0 field report: pyright saw no
//! workspaceFolders and the config block was inert), and how a server
//! that cannot run, exits or is asked to quit is reported.

use super::*;
use crate::registry;

/// A minimal fake server: records initialize params, answers with empty
/// capabilities, pulls workspace/configuration for section "python",
/// records the answer, then exits — or, given `--stay`, waits for the
/// client's shutdown/exit.
const FAKE_SERVER: &str = r#"
import sys, json, os
def read_msg():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline().decode().strip()
        if not line:
            break
        k, v = line.split(":", 1)
        headers[k.strip()] = v.strip()
    return json.loads(sys.stdin.buffer.read(int(headers["Content-Length"])))
def send(payload):
    body = json.dumps(payload).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
out = os.environ["FAKE_LSP_OUT"]
stay = "--stay" in sys.argv
while True:
    msg = read_msg()
    if msg.get("method") == "initialize":
        with open(out, "w") as f:
            f.write(json.dumps(msg["params"]))
        send({"jsonrpc": "2.0", "id": msg["id"], "result": {"capabilities": {}}})
        send({"jsonrpc": "2.0", "id": 99, "method": "workspace/configuration",
              "params": {"items": [{"section": "python"}]}})
    elif msg.get("id") == 99 and "result" in msg:
        with open(out + ".config", "w") as f:
            f.write(json.dumps(msg["result"]))
        if not stay:
            sys.exit(0)
    elif msg.get("method") == "shutdown":
        send({"jsonrpc": "2.0", "id": msg["id"], "result": None})
    elif msg.get("method") == "exit":
        sys.exit(0)
"#;

#[cfg(unix)]
#[test]
fn initialize_carries_workspace_folders_and_config_answers_pulls() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake_server.py");
    std::fs::write(&script, FAKE_SERVER).unwrap();
    let out = dir.path().join("params.json");
    // --log-content must carry LSP frame bodies (0.21.0 field report):
    // install a Full-content trace and assert on the recorded payload.
    let trace_path = dir.path().join("trace.jsonl");
    // Trace install is process-global: hold the lock for the whole
    // Full-content session so default-policy assertions elsewhere in
    // this crate never observe it.
    let _trace_lock = super::super::trace_io::TRACE_SESSION.lock();
    let trace = strop_trace::start(
        &trace_path,
        strop_trace::TraceOptions {
            content: strop_trace::ContentPolicy::Full,
            limits: strop_trace::Limits::default(),
        },
    )
    .unwrap();
    let config = serde_json::json!({"python": {"analysis": {"extraPaths": ["/opt/bb/lib"]}}});
    let script_arg = script.to_string_lossy().into_owned();
    let spec = registry::ServerSpec {
        name: "fake",
        command: "python3",
        args: std::slice::from_ref(&script_arg),
        install_hint: None,
        init_options: Some(&config),
        project_executable: false,
    };
    let (tx, rx) = channel();
    let client = spawn_local(&spec, dir.path(), &out, tx);
    // Ready after initialize; the fake server then pulls configuration
    // and exits — the client reports the exit (not a quit we asked for)
    // with the process's real status.
    let (ready, failure) = ready_then_failure(&rx);
    assert!(ready, "the fake server completed initialize");
    let hint = failure.expect("the fake server's exit is reported");
    assert!(
        hint.contains("fake exited unexpectedly (exit status: 0)"),
        "the real exit status rides the hint: {hint}"
    );
    drop(client);
    let params: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let folders = params["workspaceFolders"]
        .as_array()
        .expect("workspaceFolders");
    assert_eq!(folders.len(), 1);
    assert!(folders[0]["uri"]
        .as_str()
        .unwrap()
        .ends_with(dir.path().file_name().unwrap().to_str().unwrap()));
    assert_eq!(
        params["capabilities"]["workspace"]["configuration"],
        serde_json::json!(true),
        "the pull channel is advertised"
    );
    let answered: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.with_extension("json.config")).unwrap())
            .unwrap();
    assert_eq!(
        answered,
        serde_json::json!([{"analysis": {"extraPaths": ["/opt/bb/lib"]}}]),
        "the config block answers the section-scoped pull"
    );
    trace.finish().unwrap();
    let log = std::fs::read_to_string(&trace_path).unwrap();
    let init_frame = log
        .lines()
        .filter(|line| line.contains("\"lsp_message\""))
        .find(|line| line.contains("workspaceFolders"))
        .expect("the initialize frame body is in the trace under --log-content");
    assert!(init_frame.contains("\"payload\""));
}

/// The fake server as a local process, its output pointed at `out`
/// through the child's own environment (never this process's).
fn spawn_local(
    spec: &registry::ServerSpec<'_>,
    root: &std::path::Path,
    out: &std::path::Path,
    tx: std::sync::mpsc::Sender<LspEvent>,
) -> Client {
    let mut command = spec.local_command(root);
    command.env("FAKE_LSP_OUT", out);
    Client::spawn(
        spec,
        crate::target::Workspace::Local {
            root: root.to_path_buf(),
        },
        tx,
        command,
    )
    .expect("spawn")
}

/// Whether `Ready` arrived, then the first `Failed` hint (bounded wait).
fn ready_then_failure(rx: &Receiver<LspEvent>) -> (bool, Option<String>) {
    let mut ready = false;
    while let Ok(event) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
        match event {
            LspEvent::Ready { .. } => ready = true,
            LspEvent::Failed { hint, .. } => return (ready, Some(hint)),
            _ => {}
        }
    }
    (ready, None)
}

/// Every `Failed` hint of a client whose server leaves on its own:
/// joined first, so nothing is still in flight.
fn failures(client: Client, rx: &Receiver<LspEvent>) -> Vec<String> {
    client.wait(std::time::Duration::from_secs(20));
    rx.try_iter()
        .filter_map(|event| match event {
            LspEvent::Failed { hint, .. } => Some(hint),
            _ => None,
        })
        .collect()
}

fn spec_for<'a>(command: &'a str, args: &'a [String]) -> registry::ServerSpec<'a> {
    registry::ServerSpec {
        name: "fake",
        command,
        args,
        install_hint: Some("install the server"),
        init_options: None,
        project_executable: false,
    }
}

fn remote(root: &std::path::Path) -> crate::target::Workspace {
    crate::target::Workspace::Remote {
        endpoint: strop_workspace::RemoteEndpoint::parse("ssh://box.example").unwrap(),
        root: root.to_path_buf(),
    }
}

/// A local program that does not exist is a `Failed` event naming the
/// command, the workspace and the OS reason — raised on the client's
/// thread, never a panic or silence.
#[test]
fn missing_local_program_fails_with_a_hint_naming_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let spec = spec_for("strop-no-such-server-binary", &[]);
    let (tx, rx) = channel();
    let workspace = crate::target::Workspace::Local {
        root: dir.path().to_path_buf(),
    };
    let client = Client::spawn(&spec, workspace, tx, spec.local_command(dir.path()))
        .expect("spawn returns; the failure is an event");
    let hint = ready_then_failure(&rx)
        .1
        .expect("the launch failure is reported");
    let expected = format!(
        "cannot run `strop-no-such-server-binary` on {}: ",
        dir.path().display()
    );
    assert!(hint.starts_with(&expected), "{hint}");
    assert!(hint.ends_with("— install the server"), "{hint}");
    drop(client);
}

/// The remote shape: `ssh` runs the server through the remote shell, so
/// a missing program is exit 127 with the shell's complaint on stderr.
/// That is still "cannot run", not a crash.
#[cfg(unix)]
#[test]
fn remote_command_not_found_is_a_cannot_run_failure() {
    let dir = tempfile::tempdir().unwrap();
    let spec = spec_for("strop-missing-remote", &[]);
    let mut command = std::process::Command::new("sh");
    command.args([
        "-c",
        "echo 'sh: 1: strop-missing-remote: not found' >&2; exit 127",
    ]);
    let (tx, rx) = channel();
    let client = Client::spawn(&spec, remote(dir.path()), tx, command).expect("spawn");
    let hints = failures(client, &rx);
    let expected = format!(
        "cannot run `strop-missing-remote` on ssh://box.example{}: \
         sh: 1: strop-missing-remote: not found — install the server",
        dir.path().display()
    );
    assert_eq!(hints, [expected], "one failure, the real one");
}

/// Exit 255 from the remote command is `ssh` failing, and says so.
#[cfg(unix)]
#[test]
fn remote_ssh_failure_is_named_in_the_exit() {
    let dir = tempfile::tempdir().unwrap();
    let spec = spec_for("fake-server", &[]);
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "echo 'Connection refused' >&2; exit 255"]);
    let (tx, rx) = channel();
    let client = Client::spawn(&spec, remote(dir.path()), tx, command).expect("spawn");
    let hints = failures(client, &rx);
    assert!(
        hints.iter().any(|hint| hint
            .starts_with("fake exited unexpectedly (ssh failed, exit status: 255)")
            && hint.contains("; stderr: Connection refused")),
        "{hints:?}"
    );
}

/// A quit the client asked for is not a failure: shutdown/exit reach the
/// server, it leaves, and no `Failed` event follows.
#[cfg(unix)]
#[test]
fn requested_quit_reports_no_failure() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake_server.py");
    std::fs::write(&script, FAKE_SERVER).unwrap();
    let out = dir.path().join("params.json");
    let args = [script.to_string_lossy().into_owned(), "--stay".to_string()];
    let spec = spec_for("python3", &args);
    let (tx, rx) = channel();
    let client = spawn_local(&spec, dir.path(), &out, tx);
    let mut ready = false;
    while let Ok(event) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
        match event {
            LspEvent::Ready { .. } => {
                ready = true;
                break;
            }
            LspEvent::Failed { hint, .. } => panic!("failed before quit: {hint}"),
            _ => {}
        }
    }
    assert!(ready, "the fake server completed initialize");
    client.shutdown();
    client.wait(std::time::Duration::from_secs(20));
    let failed: Vec<_> = rx
        .try_iter()
        .filter_map(|event| match event {
            LspEvent::Failed { hint, .. } => Some(hint),
            _ => None,
        })
        .collect();
    assert!(
        failed.is_empty(),
        "a requested quit is not a failure: {failed:?}"
    );
}
