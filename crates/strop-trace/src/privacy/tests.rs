use crate::*;

#[test]
fn metadata_scope_withholds_payload_and_restores_after_unwinding() {
    let _session = super::SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scoped.jsonl");
    let session = start(
        &path,
        TraceOptions {
            content: ContentPolicy::Full,
            ..TraceOptions::default()
        },
    )
    .unwrap();
    let log = |secret: &'static str| {
        record(
            EventKind::Mutation,
            &serde_json::json!({"bytes":secret.len(),"text":capture_content().then_some(secret)}),
        )
    };
    without_content(|| {
        log("outer-private");
        without_content(|| log("nested-private"));
        log("still-private");
    });
    let panic = std::panic::catch_unwind(|| {
        without_content(|| {
            log("unwinding-private");
            panic!("exercise scope restoration");
        })
    });
    assert!(panic.is_err());
    log("public-after-scope");
    session.finish().unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        !text.contains("outer-private")
            && !text.contains("nested-private")
            && !text.contains("still-private")
            && !text.contains("unwinding-private")
    );
    assert!(text.contains("public-after-scope"));
    assert!(text.contains("\"bytes\":13"));
}

#[test]
fn opaque_capture_is_valid_diagnostics_but_cannot_replay_missing_private_actions() {
    let _session = super::SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opaque.jsonl");
    let session = start(
        &path,
        TraceOptions {
            content: ContentPolicy::Full,
            ..TraceOptions::default()
        },
    )
    .unwrap();
    record(
        EventKind::SessionStart,
        &serde_json::json!({"full_content":true}),
    );
    let tape = replay::Tape::live();
    tape.seed(&serde_json::json!({"documents":1})).unwrap();
    tape.omit_content("terminal").unwrap();
    tape.action(
        replay::Tick::default(),
        &serde_json::json!({"input":"private-key-payload"}),
    )
    .unwrap();
    tape.check(&serde_json::json!({"cells":"private-grid-payload"}))
        .unwrap();
    assert_eq!(
        tape.call("terminal.admission", &"private-command", || 41u32)
            .unwrap(),
        41
    );
    tape.finish().unwrap();
    record(EventKind::State, &serde_json::json!({"sessions":1}));
    session.finish().unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        !text.contains("private-key-payload")
            && !text.contains("private-grid-payload")
            && !text.contains("private-command")
    );
    assert!(text.contains("\"kind\":\"opaque\"") && text.contains("\"sessions\":1"));
    assert!(export::scan(text.as_bytes(), |_| Ok(())).unwrap());
    assert!(export::replay_nodes(text.as_bytes()).is_err());
    let replayed = replay::Tape::replay(vec![replay::Node::Opaque {
        scope: "terminal".into(),
    }]);
    assert!(replayed
        .call::<_, ()>("native", &(), || panic!("replay must not execute"))
        .is_err());
}

#[cfg(feature = "test-support")]
#[test]
fn omitted_fixture_content_cannot_enable_native_requests_or_calls() {
    let tape = replay::Tape::fixture(|operation, _| {
        if operation == "observe" {
            Ok(serde_json::json!(7))
        } else {
            Err(std::io::Error::other("unexpected fixture observation"))
        }
    });
    tape.seed(&()).unwrap();
    tape.omit_content("terminal").unwrap();
    assert!(!tape.request("spawn", &"private-command").unwrap());
    let observed: u32 = tape
        .call("observe", &"private-argument", || {
            panic!("fixture performed native work")
        })
        .unwrap();
    assert_eq!(observed, 7);
    tape.finish().unwrap();
    let nodes = serde_json::to_string(&tape.fixture_nodes()).unwrap();
    assert!(!nodes.contains("private-command") && !nodes.contains("private-argument"));
}
