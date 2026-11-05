use super::*;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use strop_containers::devcontainer::{
    CliOperation, CliProcess, DevcontainerError, EffectKind, JobState, LifecycleStage,
    LockfilePolicy, LogStream, MetadataResolutionConsent, ProcessFailure, ProcessLog,
    ProcessOutput, ProcessRequest, Reconciliation,
};
use strop_core::worker::CancelToken;
use strop_workspace::Filesystem;

mod lifecycle;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "strop-editor-devcontainer-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
    code: Option<i32>,
    stdout: String,
    logs: Vec<(LogStream, Vec<u8>)>,
    failure: Option<ProcessFailure>,
    gate: Option<Gate>,
    entered: Option<std::sync::mpsc::Sender<()>>,
}
impl Reply {
    fn new(code: i32, stdout: String) -> Self {
        Self {
            code: Some(code),
            stdout,
            logs: Vec::new(),
            failure: None,
            gate: None,
            entered: None,
        }
    }
    fn version(version: &str) -> Self {
        Self::new(0, format!("{version}\n"))
    }
    fn json(value: serde_json::Value) -> Self {
        Self::new(0, format!("{value}\n"))
    }
    fn up(id: &str) -> Self {
        Self::json(serde_json::json!({
            "outcome":"success", "containerId":id, "remoteUser":"vscode",
            "remoteWorkspaceFolder":"/workspaces/project"
        }))
    }
    fn blocked(mut self, gate: Gate) -> Self {
        self.gate = Some(gate);
        self
    }
    fn entered(mut self, sender: std::sync::mpsc::Sender<()>) -> Self {
        self.entered = Some(sender);
        self
    }
    fn log(mut self, stream: LogStream, text: &str) -> Self {
        self.logs.push((stream, text.as_bytes().to_vec()));
        self
    }
}

struct FakeProcess {
    requests: Mutex<Vec<ProcessRequest>>,
    replies: Mutex<VecDeque<Reply>>,
}
impl FakeProcess {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            replies: Mutex::new(replies.into()),
        }
    }
    fn requests(&self) -> Vec<ProcessRequest> {
        self.requests.lock().unwrap().clone()
    }
}
impl CliProcess for FakeProcess {
    fn run(
        &self,
        request: &ProcessRequest,
        token: &CancelToken,
        log: &mut dyn FnMut(ProcessLog),
    ) -> Result<ProcessOutput, ProcessFailure> {
        self.requests.lock().unwrap().push(request.clone());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted reply");
        for (stream, bytes) in reply.logs {
            log(ProcessLog { stream, bytes });
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
        if let Some(failure) = reply.failure {
            return Err(failure);
        }
        Ok(ProcessOutput {
            code: reply.code,
            stdout: reply.stdout.into_bytes(),
            stderr_tail: Vec::new(),
        })
    }
}

fn key(root: &TestDir) -> WorkspaceKey {
    WorkspaceKey {
        filesystem: Filesystem::Local,
        root: root.path().to_path_buf(),
    }
}
fn settle(state: &mut DevcontainerState) -> Vec<DevcontainerEvent> {
    let event = state
        .rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker settlement");
    state.handle_internal(event);
    state.poll()
}
fn discovered(root: &TestDir, json: &str) -> (DevcontainerState, WorkspaceKey) {
    root.write(".devcontainer/devcontainer.json", json);
    let mut state = DevcontainerState::default();
    let key = key(root);
    state.discover(key.clone()).unwrap();
    settle(&mut state);
    (state, key)
}
fn probe(
    state: &mut DevcontainerState,
    key: &WorkspaceKey,
    root: &TestDir,
    mut replies: Vec<Reply>,
) -> Arc<FakeProcess> {
    root.write("reference-cli", "fake process boundary");
    replies.insert(0, Reply::version("0.90.0"));
    let process = Arc::new(FakeProcess::new(replies));
    state
        .probe_cli_with_runner(
            key,
            Some(root.path().join("reference-cli")),
            process.clone(),
        )
        .unwrap();
    settle(state);
    process
}
fn operation(events: &[DevcontainerEvent]) -> &OperationResult {
    let DevcontainerEvent::OperationSettled(publication) = &events[0] else {
        panic!("operation event")
    };
    let OperationOutcome::Lifecycle(Ok(result)) = &publication.outcome else {
        panic!("lifecycle success")
    };
    result
}

#[test]
fn discovery_refuses_nonlocal_workspace_namespaces() {
    let mut state = DevcontainerState::default();
    let workspace = WorkspaceKey {
        filesystem: Filesystem::Container(
            strop_workspace::ContainerId::canonical("a".repeat(64)).unwrap(),
        ),
        root: PathBuf::from("/workspace"),
    };
    assert!(matches!(
        state.discover(workspace),
        Err(DevcontainerStateError::NonLocalWorkspace)
    ));
}

#[test]
fn discovery_is_idempotent_ordered_and_digest_scoped() {
    let root = TestDir::new();
    root.write(
        ".devcontainer/devcontainer.json",
        r#"{"name":"primary","image":"one"}"#,
    );
    root.write(
        ".devcontainer/named/devcontainer.json",
        r#"{"name":"named","image":"two"}"#,
    );
    let key = key(&root);
    let mut state = DevcontainerState::default();
    assert!(matches!(
        state.discover(key.clone()),
        Ok(DiscoveryDisposition::Started(_))
    ));
    assert_eq!(
        state.discover(key.clone()).unwrap(),
        DiscoveryDisposition::AlreadySettledOrRunning
    );
    let events = settle(&mut state);
    let DevcontainerEvent::DiscoverySettled {
        result: Ok(summary),
        offer: Some(offer),
        ..
    } = &events[0]
    else {
        panic!("discovery offer")
    };
    assert_eq!(summary.configurations, 2);
    assert_eq!(offer.name.as_deref(), Some("primary"));
    let primary = offer.identity.clone();
    let named = state.configurations(&key).unwrap()[1].identity.clone();
    assert!(state.offer(&key).is_some(), "frames do not consume offers");

    state.later(&key, &primary).unwrap();
    assert_eq!(state.offer(&key).unwrap().identity, named);
    state.suppress_workspace(&key).unwrap();
    assert!(state.offer(&key).is_none());

    root.write(
        ".devcontainer/devcontainer.json",
        "// changed\n{\"name\":\"primary\",\"image\":\"one\"}",
    );
    state.rediscover(key.clone()).unwrap();
    settle(&mut state);
    assert!(
        state.offer(&key).is_some(),
        "changed digest invalidates suppression"
    );
    assert_ne!(state.offer(&key).unwrap().identity, primary);
}

#[test]
fn malicious_hooks_are_pure_review_data_and_multiple_configs_remain_selectable() {
    let root = TestDir::new();
    let marker = root.path().join("marker");
    let secret = "sk-editor-devcontainer-secret";
    root.write(".devcontainer/one/devcontainer.json", &format!(
        r#"{{"name":"one","image":"example","initializeCommand":"touch {}","postCreateCommand":"echo {secret}","containerEnv":{{"TOKEN":"{secret}"}}}}"#,
        marker.display()
    ));
    root.write(
        ".devcontainer/two/devcontainer.json",
        r#"{"name":"two","image":"other"}"#,
    );
    let key = key(&root);
    let mut state = DevcontainerState::default();
    state.discover(key.clone()).unwrap();
    settle(&mut state);
    let configurations = state.configurations(&key).unwrap();
    let second = configurations[1].identity.clone();
    let review = state.review_options(&key, &second).unwrap();
    assert_eq!(review.identity, second);
    let first = state.configurations(&key).unwrap()[0].identity.clone();
    let review = state.review_options(&key, &first).unwrap();
    assert!(review
        .trust
        .effects
        .iter()
        .any(|effect| effect.kind == EffectKind::HostInitializeCommand));
    assert!(!marker.exists());
    let exposed = format!("{:?}\n{}", review.trust, review.trust);
    assert!(!exposed.contains(secret));
    assert!(!exposed.contains("touch "));
}

#[test]
fn missing_and_unsupported_cli_are_typed_without_installation() {
    let root = TestDir::new();
    let (mut state, key) = discovered(&root, r#"{"image":"example"}"#);
    state
        .probe_cli(&key, Some(root.path().join("missing")))
        .unwrap();
    let events = settle(&mut state);
    let DevcontainerEvent::OperationSettled(publication) = &events[0] else {
        panic!("probe")
    };
    assert!(matches!(
        publication.outcome,
        OperationOutcome::Probe(Err(OperationError::Devcontainer(
            DevcontainerError::CliNotFound
        )))
    ));

    root.write("reference-cli", "fake");
    let process = Arc::new(FakeProcess::new(vec![Reply::version("0.86.0")]));
    state
        .probe_cli_with_runner(&key, Some(root.path().join("reference-cli")), process)
        .unwrap();
    settle(&mut state);
    assert!(state.review(&key).is_none());
    let CliAvailability::Unavailable(OperationError::Devcontainer(
        DevcontainerError::UnsupportedVersion { .. },
    )) = state.workspaces.get(&key).unwrap().cli.availability()
    else {
        panic!("unsupported version")
    };
}

#[test]
fn explicit_read_and_consented_merged_read_remain_separate() {
    let root = TestDir::new();
    let (mut state, key) = discovered(&root, r#"{"name":"local","image":"example"}"#);
    let process = probe(
        &mut state,
        &key,
        &root,
        vec![
            Reply::json(serde_json::json!({"configuration":{"name":"read","image":"example"}})),
            Reply::json(serde_json::json!({
                "configuration":{"name":"read","image":"example"},
                "mergedConfiguration":{"name":"merged","image":"example"}
            })),
        ],
    );
    state.read_configuration(&key).unwrap();
    settle(&mut state);
    assert_eq!(
        state
            .read_configuration_value(&key)
            .unwrap()
            .unwrap()
            .name
            .as_deref(),
        Some("read")
    );
    assert!(state.merged_configuration_value(&key).unwrap().is_none());
    state
        .read_merged_configuration(&key, MetadataResolutionConsent::explicit())
        .unwrap();
    settle(&mut state);
    assert_eq!(
        state
            .merged_configuration_value(&key)
            .unwrap()
            .unwrap()
            .name
            .as_deref(),
        Some("merged")
    );
    let requests = process.requests();
    assert!(!requests[1]
        .args
        .iter()
        .any(|arg| arg == "--include-merged-configuration"));
    assert!(requests[2]
        .args
        .iter()
        .any(|arg| arg == "--include-merged-configuration"));
}

#[test]
fn rediscovery_resets_an_obsolete_inflight_read() {
    let root = TestDir::new();
    let (mut state, key) = discovered(&root, r#"{"image":"example"}"#);
    let gate = Gate::default();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    probe(
        &mut state,
        &key,
        &root,
        vec![Reply::json(serde_json::json!({
            "configuration":{"image":"example"}
        }))
        .blocked(gate.clone())
        .entered(entered_tx)],
    );
    state.read_configuration(&key).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    state.rediscover(key.clone()).unwrap();
    settle(&mut state);
    assert_eq!(
        state.workspaces.get(&key).unwrap().read.availability(),
        ConfigurationReadAvailability::NotRequested
    );
    gate.release();
    assert!(settle(&mut state).is_empty());
    assert_eq!(
        state.workspaces.get(&key).unwrap().read.availability(),
        ConfigurationReadAvailability::NotRequested
    );
}

#[test]
fn settled_event_serde_round_trips_every_outcome_without_unredacting_output() {
    let root = TestDir::new();
    let secret = "settled-event-secret";
    root.write(
        ".devcontainer/devcontainer.json",
        &format!(r#"{{"image":"example","containerEnv":{{"TOKEN":"{secret}"}}}}"#),
    );
    root.write("reference-cli", "fake process boundary");
    let mut state = DevcontainerState::default();
    let key = key(&root);
    let process = Arc::new(FakeProcess::new(vec![
        Reply::version("0.90.0"),
        Reply::json(serde_json::json!({"configuration":{"image":"example"}})),
        Reply::up(&"a".repeat(64)).log(LogStream::Stderr, &format!("secret: {secret}\n")),
    ]));
    let mut events = Vec::new();

    state.discover(key.clone()).unwrap();
    events.extend(settle(&mut state));
    state
        .probe_cli_with_runner(&key, Some(root.path().join("reference-cli")), process)
        .unwrap();
    events.extend(settle(&mut state));
    state.read_configuration(&key).unwrap();
    events.extend(settle(&mut state));
    let identity = state.selected_configuration(&key).unwrap();
    state.review_options(&key, &identity).unwrap();
    state.approve_review(&key).unwrap();
    state.start(&key, LockfilePolicy::Frozen).unwrap();
    events.extend(settle(&mut state));
    events.push(DevcontainerEvent::DiscoverySettled {
        workspace: key,
        incarnation: 99,
        result: Err(OperationError::Devcontainer(
            DevcontainerError::UnsupportedVersion {
                found: "0.1.0".into(),
                supported: "0.87.0 through 0.90.x".into(),
            },
        )),
        offer: None,
    });

    assert_eq!(events.len(), 5);
    assert!(matches!(
        events[0],
        DevcontainerEvent::DiscoverySettled { .. }
    ));
    assert!(matches!(
        &events[1],
        DevcontainerEvent::OperationSettled(publication)
            if matches!(publication.outcome, OperationOutcome::Probe(Ok(_)))
    ));
    assert!(matches!(
        &events[2],
        DevcontainerEvent::OperationSettled(publication)
            if matches!(publication.outcome, OperationOutcome::Read { result: Ok(()), .. })
    ));
    assert!(matches!(
        &events[3],
        DevcontainerEvent::OperationSettled(publication)
            if matches!(publication.outcome, OperationOutcome::Lifecycle(Ok(_)))
    ));
    for event in events {
        let wire = serde_json::to_value(&event).unwrap();
        assert!(!wire.to_string().contains(secret));
        let decoded: DevcontainerEvent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }
}
