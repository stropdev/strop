//! Spawn-level protocol evidence against a real child process: the
//! initialize payload's workspace fields, and the languages.toml config
//! block answering a workspace/configuration pull (0.21.0 field report:
//! pyright saw no workspaceFolders and the config block was inert).

use super::*;
use crate::registry;

/// A minimal fake server: records initialize params, answers with empty
/// capabilities, pulls workspace/configuration for section "python",
/// records the answer, exits.
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
        send({"jsonrpc": "2.0", "method": "exit"})
        sys.exit(0)
    elif msg.get("method") == "shutdown":
        send({"jsonrpc": "2.0", "id": msg["id"], "result": None})
    elif msg.get("method") == "exit":
        sys.exit(0)
"#;

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
    // The child inherits this process's environment; point the fake
    // server's output at the fixture before spawning.
    std::env::set_var("FAKE_LSP_OUT", &out);
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
    let client = Client::spawn(
        &spec,
        crate::target::Workspace::Local {
            root: dir.path().to_path_buf(),
        },
        tx,
    )
    .expect("spawn");
    // Ready after initialize; the fake server then pulls configuration
    // and exits — the client reports the exit (not a quit we asked for).
    let mut ready = false;
    let mut exited = false;
    for event in rx.iter() {
        match event {
            LspEvent::Ready { .. } => ready = true,
            LspEvent::Failed { .. } => {
                exited = true;
                break;
            }
            _ => {}
        }
    }
    assert!(ready, "the fake server completed initialize");
    assert!(exited, "the fake server exited after its pull");
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
