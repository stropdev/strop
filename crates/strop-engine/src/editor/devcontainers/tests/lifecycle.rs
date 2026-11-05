use super::*;

#[test]
fn attach_start_rebuild_are_distinct_and_rebuild_supersedes_environment() {
    let root = TestDir::new();
    let (mut state, key) = discovered(&root, r#"{"image":"example"}"#);
    let process = probe(
        &mut state,
        &key,
        &root,
        vec![
            Reply::up(&"a".repeat(64)),
            Reply::up(&"b".repeat(64)),
            Reply::up(&"d".repeat(64)),
            Reply::up(&"c".repeat(64)),
        ],
    );
    state
        .review_options(
            &key,
            &state
                .workspaces
                .get(&key)
                .unwrap()
                .selected
                .clone()
                .unwrap(),
        )
        .unwrap();
    state.approve_review(&key).unwrap();
    state.attach(&key, Some(&"a".repeat(64))).unwrap();
    assert_eq!(
        operation(&settle(&mut state)).operation,
        CliOperation::Attach
    );
    state.start(&key, LockfilePolicy::Frozen).unwrap();
    let started = operation(&settle(&mut state)).context.clone().unwrap();
    let TransitionRequest::Ready(permit) = state.begin_transition(&key, started, false).unwrap()
    else {
        panic!("clean")
    };
    state.commit_transition(*permit).unwrap();
    state.attach(&key, Some(&"d".repeat(64))).unwrap();
    let attached = operation(&settle(&mut state)).context.clone().unwrap();
    let TransitionRequest::Ready(permit) = state.begin_transition(&key, attached, false).unwrap()
    else {
        panic!("clean")
    };
    assert!(
        permit.retire_previous.is_none(),
        "external attach is not replacement ownership"
    );
    state.abort_transition(permit.id).unwrap();
    state.rebuild(&key, LockfilePolicy::Frozen).unwrap();
    let rebuilt = operation(&settle(&mut state)).context.clone().unwrap();
    assert_eq!(rebuilt.supersedes.as_deref(), Some("b".repeat(64).as_str()));
    let TransitionRequest::Ready(permit) = state.begin_transition(&key, rebuilt, false).unwrap()
    else {
        panic!("clean")
    };
    assert_eq!(permit.retire_previous.unwrap().container_id, "b".repeat(64));
    assert_ne!(permit.environment.container_id, "b".repeat(64));
    let requests = process.requests();
    assert!(requests[1]
        .args
        .iter()
        .any(|arg| arg == "--expect-existing-container"));
    assert!(!requests[2]
        .args
        .iter()
        .any(|arg| arg == "--expect-existing-container"));
    assert!(requests[3]
        .args
        .iter()
        .any(|arg| arg == "--expect-existing-container"));
    assert!(requests[4]
        .args
        .iter()
        .any(|arg| arg == "--remove-existing-container"));
}

#[test]
fn rediscovery_and_newer_operations_invalidate_uncommitted_permits() {
    let root = TestDir::new();
    let (mut state, key) = discovered(&root, r#"{"image":"example"}"#);
    probe(
        &mut state,
        &key,
        &root,
        vec![
            Reply::up(&"8".repeat(64)),
            Reply::up(&"7".repeat(64)),
            Reply::up(&"6".repeat(64)),
        ],
    );
    let identity = state
        .workspaces
        .get(&key)
        .unwrap()
        .selected
        .clone()
        .unwrap();
    state.review_options(&key, &identity).unwrap();
    state.approve_review(&key).unwrap();
    state.start(&key, LockfilePolicy::Frozen).unwrap();
    let context = operation(&settle(&mut state)).context.clone().unwrap();
    let TransitionRequest::Ready(permit) = state.begin_transition(&key, context, false).unwrap()
    else {
        panic!("clean")
    };
    state.rediscover(key.clone()).unwrap();
    settle(&mut state);
    assert!(matches!(
        state.commit_transition(*permit),
        Err(DevcontainerStateError::NoPendingTransition)
    ));
    assert!(state.active_environment().is_none());

    state.start(&key, LockfilePolicy::Frozen).unwrap();
    let context = operation(&settle(&mut state)).context.clone().unwrap();
    let TransitionRequest::Ready(permit) = state.begin_transition(&key, context, false).unwrap()
    else {
        panic!("clean")
    };
    state.start(&key, LockfilePolicy::Frozen).unwrap();
    settle(&mut state);
    assert!(matches!(
        state.commit_transition(*permit),
        Err(DevcontainerStateError::NoPendingTransition)
    ));
    assert!(state.active_environment().is_none());
}

#[test]
fn cancellation_is_requested_until_settlement_and_partial_resources_remain_visible() {
    let root = TestDir::new();
    let secret = "cancel-log-secret";
    let (mut state, key) = discovered(
        &root,
        &format!(r#"{{"image":"example","containerEnv":{{"TOKEN":"{secret}"}}}}"#),
    );
    let gate = Gate::default();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    probe(
        &mut state,
        &key,
        &root,
        vec![Reply::up(&"d".repeat(64))
            .log(LogStream::Stderr, &format!("creating with {secret}\n"))
            .blocked(gate)
            .entered(entered_tx)],
    );
    let identity = state
        .workspaces
        .get(&key)
        .unwrap()
        .selected
        .clone()
        .unwrap();
    state.review_options(&key, &identity).unwrap();
    state.approve_review(&key).unwrap();
    state.start(&key, LockfilePolicy::Frozen).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!state.output_buffer(&key).unwrap().text().contains(secret));
    assert!(state.cancel_operation(&key).unwrap());
    assert!(!state.cancel_operation(&key).unwrap());
    assert_eq!(
        state.operation_status(&key).unwrap().phase,
        OperationPhase::CancellationRequested
    );
    let events = settle(&mut state);
    let DevcontainerEvent::OperationSettled(publication) = &events[0] else {
        panic!("settled")
    };
    assert_eq!(publication.phase, OperationPhase::Cancelled);
    let OperationOutcome::Lifecycle(Ok(result)) = &publication.outcome else {
        panic!("job outcome")
    };
    assert_eq!(result.state, JobState::Cancelled);
    assert_eq!(
        result.partial_resource.as_ref().unwrap().container_id,
        "d".repeat(64)
    );
    assert_eq!(
        result.reconciliation,
        Reconciliation::CreationRequiresReconciliation
    );
    assert!(!publication.output.text().contains(secret));
}

#[test]
fn failed_up_retains_reconciliation_evidence_without_changing_root() {
    let root = TestDir::new();
    let (mut state, key) = discovered(&root, r#"{"image":"example"}"#);
    let id = "9".repeat(64);
    probe(
        &mut state,
        &key,
        &root,
        vec![Reply::new(
            1,
            format!(
                "{}\n",
                serde_json::json!({
                    "outcome":"error", "containerId":id.clone(), "description":"creation failed",
                    "didStopContainer":false
                })
            ),
        )],
    );
    let identity = state
        .workspaces
        .get(&key)
        .unwrap()
        .selected
        .clone()
        .unwrap();
    state.review_options(&key, &identity).unwrap();
    state.approve_review(&key).unwrap();
    state.start(&key, LockfilePolicy::Frozen).unwrap();
    let events = settle(&mut state);
    let DevcontainerEvent::OperationSettled(publication) = &events[0] else {
        panic!("failure")
    };
    assert_eq!(publication.phase, OperationPhase::Failed);
    let OperationOutcome::Lifecycle(Ok(result)) = &publication.outcome else {
        panic!("job outcome")
    };
    assert_eq!(result.state, JobState::Failed);
    assert_eq!(result.partial_resource.as_ref().unwrap().container_id, id);
    assert_eq!(
        result.reconciliation,
        Reconciliation::CreationRequiresReconciliation
    );
    assert!(state.active_environment().is_none());
}

#[test]
fn stale_lifecycle_success_never_publishes_to_changed_configuration() {
    let root = TestDir::new();
    let (mut state, key) = discovered(&root, r#"{"image":"old"}"#);
    let gate = Gate::default();
    probe(
        &mut state,
        &key,
        &root,
        vec![Reply::up(&"e".repeat(64)).blocked(gate.clone())],
    );
    let identity = state
        .workspaces
        .get(&key)
        .unwrap()
        .selected
        .clone()
        .unwrap();
    state.review_options(&key, &identity).unwrap();
    state.approve_review(&key).unwrap();
    state.attach(&key, None).unwrap();
    root.write(".devcontainer/devcontainer.json", r#"{"image":"new"}"#);
    state.rediscover(key.clone()).unwrap();
    settle(&mut state);
    root.write(".devcontainer/devcontainer.json", r#"{"image":"old"}"#);
    state.rediscover(key.clone()).unwrap();
    settle(&mut state);
    assert_eq!(
        state.workspaces.get(&key).unwrap().selected.as_ref(),
        Some(&identity)
    );
    gate.release();
    assert!(settle(&mut state).is_empty());
    assert!(
        state.operation_status(&key).is_none(),
        "obsolete A worker cannot become current after A→B→A"
    );
    assert!(matches!(
        state.start(&key, LockfilePolicy::Frozen),
        Err(DevcontainerStateError::ReviewRequired)
    ));
}

#[test]
fn readiness_logs_and_dirty_save_failures_preserve_the_original_environment() {
    let root = TestDir::new();
    let (mut state, key) = discovered(
        &root,
        r#"{
        "image":"example", "waitFor":"onCreateCommand", "postCreateCommand":"echo deferred"
    }"#,
    );
    probe(&mut state, &key, &root, vec![Reply::up(&"f".repeat(64))]);
    let identity = state
        .workspaces
        .get(&key)
        .unwrap()
        .selected
        .clone()
        .unwrap();
    state.review_options(&key, &identity).unwrap();
    state.approve_review(&key).unwrap();
    state.start(&key, LockfilePolicy::Frozen).unwrap();
    let context = operation(&settle(&mut state)).context.clone().unwrap();
    assert_eq!(
        state.pending_deferred_stages(&key).unwrap(),
        [LifecycleStage::PostCreate]
    );

    let TransitionRequest::Dirty(prompt) =
        state.begin_transition(&key, context.clone(), true).unwrap()
    else {
        panic!("dirty")
    };
    assert_eq!(
        state
            .resolve_dirty_transition(DirtyTransitionChoice::Save)
            .unwrap(),
        DirtyTransitionResolution::SaveRequired(prompt.id)
    );
    assert!(matches!(
        state.complete_transition_save(prompt.id, Err("disk full".into())),
        Err(DevcontainerStateError::SaveFailed(_))
    ));
    assert!(state.active_environment().is_none());
    assert_eq!(
        state
            .resolve_dirty_transition(DirtyTransitionChoice::Stay)
            .unwrap(),
        DirtyTransitionResolution::Stayed
    );

    let TransitionRequest::Dirty(_) = state.begin_transition(&key, context.clone(), true).unwrap()
    else {
        panic!("dirty")
    };
    assert_eq!(
        state
            .resolve_dirty_transition(DirtyTransitionChoice::Cancel)
            .unwrap(),
        DirtyTransitionResolution::Cancelled
    );
    let TransitionRequest::Dirty(prompt) =
        state.begin_transition(&key, context.clone(), true).unwrap()
    else {
        panic!("dirty")
    };
    state
        .resolve_dirty_transition(DirtyTransitionChoice::Save)
        .unwrap();
    let permit = state.complete_transition_save(prompt.id, Ok(())).unwrap();
    state.abort_transition(permit.id).unwrap();
    assert!(state.active_environment().is_none());

    let mut wrong = context.clone();
    wrong.workspace.host_root = PathBuf::from("/not/the/workspace");
    assert_eq!(
        state.verified_container_root(&key, &wrong).unwrap().path,
        PathBuf::from("/workspaces/project")
    );
    let mut relative = context.clone();
    relative.workspace.remote_folder = "relative/path".into();
    assert_eq!(
        state.verified_container_root(&key, &relative).unwrap().path,
        PathBuf::from("/workspaces/project")
    );
    let mut altered = context.clone();
    altered.workspace.remote_folder = "/attacker/selected".into();
    assert_eq!(
        state.verified_container_root(&key, &altered).unwrap().path,
        PathBuf::from("/workspaces/project")
    );
    let TransitionRequest::Ready(permit) = state.begin_transition(&key, altered, false).unwrap()
    else {
        panic!("clean")
    };
    assert_eq!(
        permit.environment.workspace.remote_folder,
        "/workspaces/project"
    );
    state.abort_transition(permit.id).unwrap();
    let TransitionRequest::Ready(permit) = state.begin_transition(&key, context, false).unwrap()
    else {
        panic!("clean")
    };
    let active = state.commit_transition(*permit).unwrap();
    assert_eq!(active.root.path, PathBuf::from("/workspaces/project"));
}
