use super::*;
use crate::editor::feedback::{FeedbackKind, FeedbackLifecycle};
use crate::editor::Editor;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use strop_containers::devcontainer::{
    CliProcess, LogStream, ProcessFailure, ProcessLog, ProcessOutput, ProcessRequest,
};
use strop_core::worker::CancelToken;
use strop_core::Buffer;
use strop_workspace::Filesystem;

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "strop-devcontainer-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, text: &str) {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Gate {
    fn release(&self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }
}
struct Reply {
    stdout: String,
    logs: Vec<ProcessLog>,
    gate: Option<Gate>,
    entered: Option<std::sync::mpsc::Sender<()>>,
}
impl Reply {
    fn json(value: serde_json::Value) -> Self {
        Self {
            stdout: format!("{value}\n"),
            logs: Vec::new(),
            gate: None,
            entered: None,
        }
    }
    fn version() -> Self {
        Self {
            stdout: "0.90.0\n".into(),
            logs: Vec::new(),
            gate: None,
            entered: None,
        }
    }
    fn up(id: &str) -> Self {
        Self::json(serde_json::json!({
            "outcome":"success", "containerId":id, "remoteUser":"vscode",
            "remoteWorkspaceFolder":"/workspaces/project"
        }))
    }
    fn blocked(mut self, gate: Gate, entered: std::sync::mpsc::Sender<()>) -> Self {
        self.gate = Some(gate);
        self.entered = Some(entered);
        self
    }
    fn log(mut self, text: &str) -> Self {
        self.logs.push(ProcessLog {
            stream: LogStream::Stderr,
            bytes: text.as_bytes().to_vec(),
        });
        self
    }
}
struct Process {
    replies: Mutex<VecDeque<Reply>>,
}
impl CliProcess for Process {
    fn run(
        &self,
        _request: &ProcessRequest,
        token: &CancelToken,
        log: &mut dyn FnMut(ProcessLog),
    ) -> Result<ProcessOutput, ProcessFailure> {
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted reply");
        for line in reply.logs {
            log(line);
        }
        if let Some(entered) = reply.entered {
            let _ = entered.send(());
        }
        if let Some(gate) = reply.gate {
            let wake = gate.clone();
            token
                .register_cancel_resource(move || {
                    wake.release();
                    Ok(())
                })
                .unwrap();
            let mut released = gate.0 .0.lock().unwrap();
            while !*released && !token.is_cancelled() {
                released = gate.0 .1.wait(released).unwrap();
            }
        }
        Ok(ProcessOutput {
            code: Some(0),
            stdout: reply.stdout.into_bytes(),
            stderr_tail: Vec::new(),
        })
    }
}

fn key(root: &Dir) -> WorkspaceKey {
    WorkspaceKey {
        filesystem: Filesystem::Local,
        root: root.0.clone(),
    }
}
fn settle(editor: &mut Editor) {
    let event = editor
        .devcontainers
        .rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    editor.devcontainers.handle_internal(event);
    editor.handle_devcontainer_events();
}
fn discovered(root: &Dir, json: &str) -> (Editor, WorkspaceKey) {
    root.write(".devcontainer/devcontainer.json", json);
    let mut editor = Editor::new_in(Buffer::from_text("local\n"), root.0.clone());
    let key = key(root);
    editor.devcontainers.discover(key.clone()).unwrap();
    settle(&mut editor);
    (editor, key)
}
fn probe(editor: &mut Editor, key: &WorkspaceKey, root: &Dir, replies: Vec<Reply>) {
    root.write("reference-cli", "fake");
    let identity = editor.devcontainers.selected_configuration(key).unwrap();
    editor.devcontainers.review_options(key, &identity).unwrap();
    editor.devcontainers.approve_review(key).unwrap();
    let mut replies = VecDeque::from(replies);
    replies.push_front(Reply::version());
    editor
        .devcontainers
        .probe_cli_with_runner(
            key,
            Some(root.0.join("reference-cli")),
            Arc::new(Process {
                replies: Mutex::new(replies),
            }),
        )
        .unwrap();
    settle(editor);
}
fn discovery(editor: &Editor, key: &WorkspaceKey) -> u64 {
    editor.devcontainers.discovery_incarnation(key).unwrap()
}
fn start(editor: &mut Editor, key: &WorkspaceKey, kind: OperationKind) {
    let review = editor.devcontainers.review(key).unwrap();
    editor.invoke_devcontainer_action(DevcontainerAction::Lifecycle {
        workspace: key.clone(),
        identity: review.identity,
        engine: review.engine,
        discovery: discovery(editor, key),
        kind,
    });
}
fn dirty_action(editor: &Editor, choice: DirtyTransitionChoice) -> DevcontainerAction {
    DevcontainerAction::Dirty {
        id: editor.devcontainers.pending_transition().unwrap().id,
        choice,
    }
}

#[test]
fn offer_dismiss_is_stable_and_rediscovery_invalidates_actions() {
    let root = Dir::new();
    let (mut editor, key) = discovered(&root, r#"{"image":"one"}"#);
    let record = editor
        .feedback()
        .iter()
        .find(|record| record.kind == FeedbackKind::DevcontainerOffer)
        .unwrap();
    let id = record.id();
    let occurrences = record.occurrences();
    let stale = record.actions[0].clone();
    editor.invoke_feedback_action(id, 3);
    editor.handle_devcontainer_events();
    let record = editor
        .feedback()
        .iter()
        .find(|record| record.id() == id)
        .unwrap();
    assert_eq!(record.lifecycle(), FeedbackLifecycle::Dismissed);
    assert_eq!(record.occurrences(), occurrences);
    assert!(
        editor.devcontainers.offer(&key).is_some(),
        "dismissal does not suppress the decision"
    );

    root.write(
        ".devcontainer/devcontainer.json",
        "// changed\n{\"image\":\"one\"}",
    );
    editor.devcontainers.rediscover(key.clone()).unwrap();
    settle(&mut editor);
    let crate::editor::feedback::TypedAction::Devcontainer(action) = stale else {
        panic!("typed review")
    };
    assert!(!editor.devcontainer_action_valid(&action));
    let current = editor
        .feedback()
        .iter()
        .find(|record| {
            record.kind == FeedbackKind::DevcontainerOffer
                && record.lifecycle() == FeedbackLifecycle::Active
        })
        .unwrap();
    let crate::editor::feedback::TypedAction::Devcontainer(suppress) = current.actions[2].clone()
    else {
        panic!("suppress")
    };
    editor.invoke_devcontainer_action(suppress);
    assert!(editor.devcontainers.offer(&key).is_none());
}

#[test]
fn review_is_a_redacted_real_buffer_with_keyboard_actions_and_missing_cli_explanation() {
    let root = Dir::new();
    let secret = "strop-ui-secret";
    let (mut editor, key) = discovered(
        &root,
        &format!(
            r#"{{"name":"review","image":"one","remoteUser":"vscode","workspaceFolder":"/workspaces/project","initializeCommand":"echo {secret}","containerEnv":{{"TOKEN":"{secret}"}}}}"#
        ),
    );
    editor.invoke_devcontainer_action(DevcontainerAction::Review {
        workspace: key.clone(),
        identity: editor.devcontainers.selected_configuration(&key).unwrap(),
        discovery: discovery(&editor, &key),
    });
    assert!(editor.buf().readonly);
    let text = editor.buf().text().to_string();
    assert!(text.contains("Provider: image") && text.contains("Remote user: vscode"));
    assert!(text.contains("HostInitializeCommand") && !text.contains(secret));
    editor.open_filesystem_actions();
    let rows = &editor.picker.as_ref().unwrap().picker.items;
    assert!(rows
        .iter()
        .any(|row| row.text.contains("Approve this digest")));
    assert!(rows
        .iter()
        .any(|row| row.text.contains("Probe the optional reference CLI")));
    editor.close_picker();

    editor
        .devcontainers
        .probe_cli(&key, Some(root.0.join("missing")))
        .unwrap();
    settle(&mut editor);
    let record = editor
        .feedback()
        .iter()
        .find(|record| {
            record.kind == FeedbackKind::DevcontainerOperation
                && record.summary().contains("failed")
        })
        .unwrap();
    assert!(record.detail().contains("No installation was attempted"));
    assert!(editor
        .devcontainers
        .review_text(&key)
        .contains("0.87.0 through 0.90.x"));
}

#[test]
fn merged_read_is_explicit_and_opens_a_real_configuration_buffer() {
    let root = Dir::new();
    let (mut editor, key) = discovered(&root, r#"{"image":"one"}"#);
    probe(
        &mut editor,
        &key,
        &root,
        vec![Reply::json(serde_json::json!({
            "configuration":{"image":"one"},
            "mergedConfiguration":{"image":"one","remoteUser":"resolved"}
        }))],
    );
    let review = editor.devcontainers.review(&key).unwrap();
    editor.invoke_devcontainer_action(DevcontainerAction::ReadConfiguration {
        workspace: key.clone(),
        identity: review.identity,
        engine: review.engine,
        discovery: discovery(&editor, &key),
        merged: true,
    });
    settle(&mut editor);
    assert!(editor.buf().readonly);
    let text = editor.buf().text().to_string();
    assert!(text.contains("merged configuration") && text.contains("resolved"));
}

#[test]
fn cancellation_and_output_remain_operation_scoped_until_settlement() {
    let root = Dir::new();
    let (mut editor, key) = discovered(
        &root,
        r#"{"image":"one","containerEnv":{"TOKEN":"strop-log-secret"}}"#,
    );
    let gate = Gate::default();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    probe(
        &mut editor,
        &key,
        &root,
        vec![Reply::up(&"a".repeat(64))
            .log("strop-log-secret\n")
            .blocked(gate, entered_tx)],
    );
    start(&mut editor, &key, OperationKind::Start);
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let operation = editor.devcontainers.operation_status(&key).unwrap().key.id;
    editor.invoke_devcontainer_action(DevcontainerAction::CancelOperation {
        workspace: key.clone(),
        discovery: discovery(&editor, &key),
        operation,
    });
    assert_eq!(
        editor.devcontainers.operation_status(&key).unwrap().phase,
        OperationPhase::CancellationRequested
    );
    editor.invoke_devcontainer_action(DevcontainerAction::InspectOutput {
        workspace: key.clone(),
        discovery: discovery(&editor, &key),
        operation,
    });
    assert!(editor.buf().readonly);
    assert!(editor
        .buf()
        .text()
        .to_string()
        .contains("Cancellation requested: true"));
    settle(&mut editor);
    let output = editor.devcontainers.operation_text(&key).unwrap();
    assert!(output.contains("Cancelled") && !output.contains("strop-log-secret"));
}

#[test]
fn dirty_stay_and_save_failure_preserve_original_work() {
    let root = Dir::new();
    let (mut editor, key) = discovered(&root, r#"{"image":"one"}"#);
    probe(
        &mut editor,
        &key,
        &root,
        vec![Reply::up(&"a".repeat(64)), Reply::up(&"b".repeat(64))],
    );
    editor.feed_text("Aunsaved");
    start(&mut editor, &key, OperationKind::Start);
    settle(&mut editor);
    assert!(editor.devcontainers.pending_transition().is_some());
    let stay = dirty_action(&editor, DirtyTransitionChoice::Stay);
    editor.invoke_devcontainer_action(stay);
    assert!(editor.buf().is_dirty() && editor.devcontainers.active_environment().is_none());

    start(&mut editor, &key, OperationKind::Start);
    settle(&mut editor);
    let save = dirty_action(&editor, DirtyTransitionChoice::Save);
    editor.invoke_devcontainer_action(save);
    assert!(
        editor.buf().is_dirty(),
        "scratch without a save path remains untouched"
    );
    assert!(editor.devcontainers.active_environment().is_none());
    assert!(
        editor.devcontainers.pending_transition().is_some(),
        "failure leaves an explicit Stay/Cancel choice"
    );
}

#[test]
fn successful_save_enters_exact_verified_root_and_rebuild_retires_old_editor_binding() {
    let root = Dir::new();
    root.write("file.txt", "before\n");
    let (mut editor, key) = discovered(&root, r#"{"image":"one"}"#);
    probe(
        &mut editor,
        &key,
        &root,
        vec![Reply::up(&"a".repeat(64)), Reply::up(&"b".repeat(64))],
    );
    let document = editor.open_fixture(&root.0.join("file.txt")).unwrap();
    editor.feed_text("Oafter<esc>");
    start(&mut editor, &key, OperationKind::Start);
    settle(&mut editor);
    let save = dirty_action(&editor, DirtyTransitionChoice::Save);
    editor.invoke_devcontainer_action(save);
    editor.wait_io().unwrap();
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(|_, _| {
        Err(std::io::Error::other("native container attach forbidden"))
    }));
    editor.handle_devcontainer_events();
    assert!(!editor.doc(document).buf.is_dirty());
    let active = editor.devcontainers.active_environment().unwrap().clone();
    assert_eq!(active.root.path, Path::new("/workspaces/project"));
    assert_eq!(active.identity.container_id, "a".repeat(64));
    assert!(editor.workspaces.resolve(&active.root).is_some());
    let crate::editor::containers::ContainerJob::Attach { id, path, .. } =
        &editor.containers.pending.as_ref().unwrap().key.job
    else {
        panic!("attach")
    };
    assert_eq!(id, &"a".repeat(64));
    assert_eq!(path, Path::new("/workspaces/project"));

    editor.containers.attached.insert(
        "a".repeat(64),
        strop_containers::ContainerIdentity {
            id: "a".repeat(64),
            name: "old".into(),
            image: "one".into(),
            started_at: "old".into(),
            user: String::new(),
            workdir: "/workspaces/project".into(),
            env: Vec::new(),
        },
    );
    start(&mut editor, &key, OperationKind::Rebuild);
    settle(&mut editor);
    assert_eq!(
        editor
            .devcontainers
            .active_environment()
            .unwrap()
            .identity
            .container_id,
        "b".repeat(64)
    );
    assert!(!editor.containers.attached.contains_key(&"a".repeat(64)));
}

#[test]
fn discovery_of_a_workspace_without_configurations_stays_silent() {
    // 0088 U13: no devcontainer is the ordinary case — no notice, no
    // `Space a` advertisement for an empty result.
    let root = Dir::new();
    let mut editor = Editor::new_in(Buffer::from_text("local\n"), root.0.clone());
    editor.devcontainers.discover(key(&root)).unwrap();
    settle(&mut editor);
    assert!(
        editor.feedback().iter().all(|record| !matches!(
            record.kind,
            FeedbackKind::DevcontainerOperation | FeedbackKind::DevcontainerOffer
        )),
        "{:?}",
        editor
            .feedback()
            .iter()
            .map(|record| record.summary())
            .collect::<Vec<_>>()
    );
    assert!(editor.presented_feedback().is_none());
}

fn no_replay_frame(_: &mut Editor, _: u16, _: u16, _: bool) -> std::io::Result<()> {
    Ok(())
}

#[test]
fn recorded_discovery_completion_replays_without_native_work() {
    use crate::editor::events::AppEvent;
    use crate::editor::trace::drive::{self, Action};
    use crate::editor::trace::seed::Seed;
    use std::rc::Rc;
    use strop_trace::replay::{Node, Tape};

    let root = Dir::new();
    // A discovery issue is what makes the completion speak (U13).
    root.write(".devcontainer/devcontainer.json", "{ not json");
    let mut editor = Editor::new_in(Buffer::from_text("local\n"), root.0.clone());
    editor.config.completion.enabled = false;
    editor.tape = Rc::new(Tape::fixture(|operation, _| match operation {
        "analysis.start" => Ok(serde_json::json!({"Ok": null})),
        _ => Err(std::io::Error::other("unexpected native observation")),
    }));
    editor.tape.seed(&Seed::capture(&editor).unwrap()).unwrap();
    editor.devcontainers.discover(key(&root)).unwrap();
    let event = editor
        .devcontainers
        .rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    editor.devcontainers.handle_internal(event);
    let mut tick = editor.tape.now();
    tick.monotonic_ms += 1;
    editor
        .recorded_action(Action::Event(AppEvent::Devcontainer), tick)
        .unwrap();

    let expected = editor
        .feedback()
        .iter()
        .find(|record| {
            record.kind == FeedbackKind::DevcontainerOperation
                && record.summary() == "Dev container discovery completed"
        })
        .map(|record| (record.summary().to_owned(), record.detail().to_owned()))
        .unwrap();
    editor.tape.finish().unwrap();
    let nodes = editor.tape.fixture_nodes();
    assert!(nodes.iter().any(|node| matches!(
        node,
        Node::Action { value, .. }
            if value["Event"]["DevcontainerSettled"].is_array()
    )));

    let replayed = drive::replay(nodes, no_replay_frame).unwrap();
    let actual = replayed
        .feedback()
        .iter()
        .find(|record| record.kind == FeedbackKind::DevcontainerOperation)
        .map(|record| (record.summary().to_owned(), record.detail().to_owned()))
        .unwrap();
    assert_eq!(actual, expected);
    assert!(replayed.devcontainers.workspaces.is_empty());
    assert!(!replayed.devcontainers.pending());
}
