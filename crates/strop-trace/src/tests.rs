use super::*;

#[test]
fn trace_lifecycle_is_exclusive_ordered_and_durable() {
    assert!(!enabled());
    record_with(EventKind::Input, || -> () {
        panic!("disabled producer was evaluated")
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let session = start(&path, TraceOptions::default()).unwrap();
    assert!(matches!(
        start(&path, TraceOptions::default()),
        Err(TraceError::AlreadyActive)
    ));
    std::thread::scope(|scope| {
        for producer in 0..4 {
            scope.spawn(move || {
                for value in 0..25 {
                    record(
                        EventKind::Input,
                        &serde_json::json!({"producer":producer,"value":value}),
                    );
                }
            });
        }
    });
    session.finish().unwrap();
    assert!(!enabled());
    let text = std::fs::read_to_string(&path).unwrap();
    let events: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 100);
    let mut last_time = 0;
    let mut producer_order = [0; 4];
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], index + 1);
        let timestamp = event["elapsed_us"].as_u64().unwrap();
        assert!(timestamp >= last_time);
        last_time = timestamp;
        let producer = event["fields"]["producer"].as_u64().unwrap() as usize;
        assert_eq!(event["fields"]["value"], producer_order[producer]);
        producer_order[producer] += 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(matches!(
        start(&path, TraceOptions::default()),
        Err(TraceError::Open { .. })
    ));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        text,
        "existing data preserved"
    );
    let next = directory.path().join("next.jsonl");
    let session = start(
        &next,
        TraceOptions {
            content: ContentPolicy::Full,
        },
    )
    .unwrap();
    assert!(capture_content());
    record(
        EventKind::SessionEnd,
        &serde_json::json!({"reason":"finished"}),
    );
    drop(session);
    let next_text = std::fs::read_to_string(next).unwrap();
    let end: serde_json::Value = serde_json::from_str(next_text.trim()).unwrap();
    assert_eq!(end["seq"], 1);
    assert_eq!(end["event"], "session_end");
}
