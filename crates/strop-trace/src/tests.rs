use std::io;

use super::*;

/// `start` is process-global; tests that open a session serialize on this.
static SESSION: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
#[test]
fn trace_lifecycle_is_exclusive_ordered_and_durable() {
    let _session = SESSION.lock();
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
    // 100 produced records plus the mandatory terminal marker.
    assert_eq!(events.len(), 101);
    let mut last_time = 0;
    let mut producer_order = [0; 4];
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], index + 1);
        let timestamp = event["elapsed_us"].as_u64().unwrap();
        assert!(timestamp >= last_time);
        last_time = timestamp;
        if event["event"] == "trace_end" {
            assert_eq!(index, 100, "terminal marker is the last record");
            assert_eq!(event["fields"]["complete"], true);
            assert_eq!(event["fields"]["reason"], "complete");
            continue;
        }
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
            ..TraceOptions::default()
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
    let mut lines = next_text.lines();
    let end: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(end["seq"], 1);
    assert_eq!(end["event"], "session_end");
    let terminal: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(terminal["event"], "trace_end");
    assert_eq!(terminal["fields"]["complete"], true);
    assert!(
        lines.next().is_none(),
        "nothing follows the terminal marker"
    );
}

#[test]
fn invalid_limits_are_refused_before_any_file_exists() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("never.jsonl");
    for limits in [
        Limits {
            bytes: 1024,
            ..Limits::default()
        }, // below writer+terminal floor
        Limits {
            bytes: MAX_CAPTURE_BYTES * 2,
            ..Limits::default()
        },
        Limits {
            events: 0,
            ..Limits::default()
        },
        Limits {
            events: MAX_CAPTURE_EVENTS + 1,
            ..Limits::default()
        },
        Limits {
            record_bytes: 0,
            ..Limits::default()
        },
        Limits {
            record_bytes: MAX_RECORD_BYTES + 1,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            start(
                &path,
                TraceOptions {
                    limits,
                    ..TraceOptions::default()
                }
            ),
            Err(TraceError::Incomplete(_))
        ));
    }
    assert!(!path.exists());
}

#[test]
fn event_cap_ends_with_explicit_incomplete_terminal_marker() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capped.jsonl");
    let session = start(
        &path,
        TraceOptions {
            limits: Limits {
                events: 5,
                ..Limits::default()
            },
            ..TraceOptions::default()
        },
    )
    .unwrap();
    for value in 0..50 {
        record(EventKind::Input, &serde_json::json!({"value":value}));
    }
    let failure = session.finish();
    assert!(
        matches!(&failure, Err(TraceError::Incomplete(message)) if message.contains("capture limit")),
        "finish reports the cap: {failure:?}"
    );
    let events: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 6, "5 admitted events + terminal marker");
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], index + 1, "sequence stays contiguous");
    }
    let terminal = events.last().unwrap();
    assert_eq!(terminal["event"], "trace_end");
    assert_eq!(terminal["fields"]["complete"], false);
    assert_eq!(terminal["fields"]["reason"], "capture_limit");
    // The capped trace is honest about itself and is refused for replay.
    assert!(export::replay_nodes(io::BufReader::new(std::fs::File::open(&path).unwrap())).is_err());
}

#[test]
fn oversize_record_marks_capture_incomplete() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("oversize.jsonl");
    let session = start(&path, TraceOptions::default()).unwrap();
    let huge = "x".repeat(MAX_RECORD_BYTES + 10);
    record(EventKind::Input, &serde_json::json!({"secret":huge}));
    record(
        EventKind::Input,
        &serde_json::json!({"after":"the failure"}),
    );
    assert!(session.finish().is_err());
    let events: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let terminal = events.last().unwrap();
    assert_eq!(terminal["event"], "trace_end");
    assert_eq!(terminal["fields"]["complete"], false);
    assert_eq!(terminal["fields"]["reason"], "capture_failure");
}

#[test]
fn reader_rejects_sequence_gaps_duplicates_and_trailing_records() {
    let rows = [
        r#"{"schema_version":2,"seq":1,"elapsed_us":0,"event":"input","fields":{}}"#,
        r#"{"schema_version":2,"seq":3,"elapsed_us":1,"event":"input","fields":{}}"#,
    ]
    .join("\n");
    let rows = rows + "\n";
    let error = export::scan(rows.as_bytes(), |_| Ok(())).unwrap_err();
    assert!(error.to_string().contains("sequence"));

    let rows = [
        r#"{"schema_version":2,"seq":1,"elapsed_us":0,"event":"input","fields":{}}"#,
        r#"{"schema_version":2,"seq":1,"elapsed_us":1,"event":"input","fields":{}}"#,
    ]
    .join("\n");
    let rows = rows + "\n";
    assert!(export::scan(rows.as_bytes(), |_| Ok(())).is_err());

    let rows = [
        r#"{"schema_version":2,"seq":1,"elapsed_us":0,"event":"input","fields":{}}"#,
        r#"{"schema_version":2,"seq":2,"elapsed_us":1,"event":"trace_end","fields":{"complete":true,"reason":"complete"}}"#,
        r#"{"schema_version":2,"seq":3,"elapsed_us":2,"event":"input","fields":{}}"#,
    ]
    .join("\n");
    let rows = rows + "\n";
    assert!(export::scan(rows.as_bytes(), |_| Ok(())).is_err());

    // A file with no terminal marker is not a finished trace.
    let rows =
        [r#"{"schema_version":2,"seq":1,"elapsed_us":0,"event":"input","fields":{}}"#].join("\n");
    let rows = rows + "\n";
    assert!(!export::scan(rows.as_bytes(), |_| Ok(())).unwrap());

    // A physically truncated last line (no newline) is refused.
    let rows = concat!(
        "{\"schema_version\":2,\"seq\":1,\"elapsed_us\":0,\"event\":\"input\",\"fields\":{}}\n",
        "{\"schema_version\":2,\"seq\":2,\"elapsed_us\":1,\"event\":\"inp"
    );
    assert!(export::scan(rows.as_bytes(), |_| Ok(())).is_err());
}

#[test]
fn metadata_export_removes_every_content_bearing_field() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("privacy.jsonl");
    let secret = "hunter2-hunter2-hunter2";
    let paste = "top secret paste payload";
    let session = start(&path, TraceOptions::default()).unwrap();
    record(
        EventKind::SessionStart,
        &serde_json::json!({"argv":["strop","/home/user/secret-project/main.rs"],"cwd":"/home/user/secret-project"}),
    );
    record(
        EventKind::Input,
        &serde_json::json!({"key":{"Char":"h"},"replay":"h","source":"external"}),
    );
    record(
        EventKind::Paste,
        &serde_json::json!({"bytes":paste.len(),"text":null}),
    );
    record(
        EventKind::Document,
        &serde_json::json!({"path":"/home/user/secret-project/main.rs","path_bytes":[47,104,111,109,101],"name":"main.rs","bytes":42}),
    );
    record(
        EventKind::LspMessage,
        &serde_json::json!({"direction":"tx","method":"textDocument/hover","payload_bytes":128}),
    );
    record(
        EventKind::Error,
        &serde_json::json!({"message":&format!("failed on {secret}")}),
    );
    record(
        EventKind::Render,
        &serde_json::json!({"cell_hash":"0123abcd0123abcd","columns":80,"rows":24}),
    );
    session.finish().unwrap();

    let mut exported = Vec::new();
    export::metadata(
        io::BufReader::new(std::fs::File::open(&path).unwrap()),
        &mut exported,
    )
    .unwrap();
    let text = String::from_utf8(exported).unwrap();
    // Positive projection only: seq + closed category, nothing else.
    for line in text.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        if value.get("schema").is_some() || value.get("export_end").is_some() {
            continue;
        }
        assert!(value.get("seq").is_some(), "line carries seq: {line}");
        assert!(
            value.get("category").is_some(),
            "line carries category: {line}"
        );
        assert_eq!(
            value.as_object().unwrap().len(),
            2,
            "no other key survives: {line}"
        );
    }
    for banned in [
        "hunter2",
        "secret-project",
        "main.rs",
        "path_bytes",
        "argv",
        "cell_hash",
        "hover",
        "payload_bytes",
        "key",
        "failed on",
    ] {
        assert!(!text.contains(banned), "export leaks {banned}:\n{text}");
    }
    assert!(text.contains("\"replayable\":false"));
    assert!(text.contains("\"source_complete\":true"));
    // Occurrence counting survives; content does not.
    assert_eq!(text.matches("\"category\":\"input\"").count(), 1);
    assert_eq!(text.matches("\"category\":\"render\"").count(), 1);
}

#[test]
fn replay_nodes_require_complete_full_content_capture() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    // Metadata capture: forensic nodes were never recorded.
    let meta = directory.path().join("meta.jsonl");
    let session = start(&meta, TraceOptions::default()).unwrap();
    replay::Tape::live().finish().unwrap();
    session.finish().unwrap();
    let error =
        export::replay_nodes(io::BufReader::new(std::fs::File::open(&meta).unwrap())).unwrap_err();
    assert!(error.to_string().contains("full-content"));

    // Full capture with a proper seed..end forensic stream round-trips.
    let full = directory.path().join("full.jsonl");
    let session = start(
        &full,
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
    let mut tick = replay::Tick::default();
    tick.monotonic_ms += 1;
    tape.action(tick, &serde_json::json!({"kind":"event"}))
        .unwrap();
    let observed: u32 = tape.call("git.discover", &"/recorded", || 7).unwrap();
    assert_eq!(observed, 7);
    tape.request("shell.display", &serde_json::json!({"ticket":1}))
        .unwrap();
    tape.check(&serde_json::json!({"mode":"NORMAL"})).unwrap();
    tape.finish().unwrap();
    session.finish().unwrap();

    let nodes =
        export::replay_nodes(io::BufReader::new(std::fs::File::open(&full).unwrap())).unwrap();
    assert!(matches!(nodes.first(), Some(replay::Node::Seed { .. })));
    assert!(matches!(nodes.last(), Some(replay::Node::End)));
    assert_eq!(nodes.len(), 6);

    // The recorded tape replays without any native access.
    let replayed = replay::Tape::replay(nodes);
    let seed: serde_json::Value = replayed.take_seed().unwrap();
    assert_eq!(seed["documents"], 1);
    let action: serde_json::Value = replayed.next().unwrap().unwrap();
    assert_eq!(action["kind"], "event");
    let observed: u32 = replayed
        .call("git.discover", &"/recorded", || panic!("native ran"))
        .unwrap();
    assert_eq!(observed, 7);
    assert!(!replayed
        .request("shell.display", &serde_json::json!({"ticket":1}))
        .unwrap());
    replayed
        .check(&serde_json::json!({"mode":"NORMAL"}))
        .unwrap();
    assert!(replayed.next::<serde_json::Value>().unwrap().is_none());
    replayed.healthy().unwrap();
}

#[test]
fn oversize_forensic_value_chunks_and_replays_completely() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("chunked.jsonl");
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
    let huge = "xyzzy".repeat(100_000); // 500_000 bytes, well over the record cap
    tape.check(&serde_json::json!({"completion": huge}))
        .unwrap();
    tape.finish().unwrap();
    // The capture completes: an oversize forensic value no longer refuses it.
    session.finish().unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"replay_chunk\""));
    let nodes =
        export::replay_nodes(io::BufReader::new(std::fs::File::open(&path).unwrap())).unwrap();
    assert_eq!(nodes.len(), 3, "seed, oversized check, end");
    let replayed = replay::Tape::replay(nodes);
    let seed: serde_json::Value = replayed.take_seed().unwrap();
    assert_eq!(seed["documents"], 1);
    replayed
        .check(&serde_json::json!({"completion": huge}))
        .unwrap();
    assert!(replayed.next::<serde_json::Value>().unwrap().is_none());
    replayed.healthy().unwrap();

    // The metadata export of the same file projects categories only: the
    // chunk carriers and the payload bytes never survive the projection.
    let mut out = Vec::new();
    export::metadata(
        io::BufReader::new(std::fs::File::open(&path).unwrap()),
        &mut out,
    )
    .unwrap();
    let exported = String::from_utf8(out).unwrap();
    assert!(exported.contains("\"category\":\"replay\""));
    assert!(!exported.contains("replay_chunk"));
    assert!(!exported.contains("xyzzy"));
}

#[test]
fn value_at_record_cap_boundary_chunks_exactly_when_over() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    // A Check node serializes as {"kind":"check","value":{"d":"…"}}.
    let envelope = r#"{"kind":"check","value":{"d":""}}"#.len();
    for (path, extra, chunked) in [("exact.jsonl", 0, false), ("over.jsonl", 1, true)] {
        let path = directory.path().join(path);
        let session = start(
            &path,
            TraceOptions {
                content: ContentPolicy::Full,
                ..TraceOptions::default()
            },
        )
        .unwrap();
        let node = replay::Node::Check {
            value: serde_json::json!({"d": "x".repeat(MAX_RECORD_BYTES - envelope + extra)}),
        };
        assert_eq!(
            serde_json::to_vec(&node).unwrap().len(),
            MAX_RECORD_BYTES + extra
        );
        record(EventKind::Replay, &node);
        session.finish().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.contains("\"replay_chunk\""), chunked);
        let mut rows = Vec::new();
        let complete = export::scan(
            io::BufReader::new(std::fs::File::open(&path).unwrap()),
            |row| {
                rows.push(row);
                Ok(())
            },
        )
        .unwrap();
        assert!(complete);
        assert_eq!(rows.len(), 2, "one logical record plus the terminal");
        assert_eq!(rows[0].event, EventKind::Replay);
        let decoded: replay::Node = serde_json::from_value(rows.remove(0).fields).unwrap();
        assert_eq!(decoded, node);
    }
}

#[test]
fn metadata_mode_never_writes_chunked_payloads() {
    let _session = SESSION.lock();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("meta.jsonl");
    let session = start(&path, TraceOptions::default()).unwrap();
    let needle = "confidential-".repeat(30_000);
    record(EventKind::Replay, &serde_json::json!({"data": needle}));
    // Same honest refusal as today: metadata capture never carries payloads,
    // chunked or otherwise.
    assert!(session.finish().is_err());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("replay_chunk"));
    assert!(!text.contains("confidential"));
}

#[test]
fn schema_two_traces_read_unchanged() {
    let rows = [
        r#"{"schema_version":2,"seq":1,"elapsed_us":0,"event":"session_start","fields":{"full_content":true}}"#,
        r#"{"schema_version":2,"seq":2,"elapsed_us":1,"event":"replay","fields":{"kind":"seed","value":{"documents":0}}}"#,
        r#"{"schema_version":2,"seq":3,"elapsed_us":2,"event":"replay","fields":{"kind":"end"}}"#,
        r#"{"schema_version":2,"seq":4,"elapsed_us":3,"event":"trace_end","fields":{"complete":true,"reason":"complete"}}"#,
    ]
    .join("\n")
        + "\n";
    assert!(export::scan(rows.as_bytes(), |_| Ok(())).unwrap());
    let nodes = export::replay_nodes(rows.as_bytes()).unwrap();
    assert!(matches!(nodes.first(), Some(replay::Node::Seed { .. })));
    assert!(matches!(nodes.last(), Some(replay::Node::End)));
    // A file mixing schema versions is refused, not guessed at.
    let mixed = rows.replace(
        r#""schema_version":2,"seq":4"#,
        r#""schema_version":3,"seq":4"#,
    );
    assert!(export::scan(mixed.as_bytes(), |_| Ok(())).is_err());
}

#[test]
fn tape_divergences_are_sticky_and_never_reach_native() {
    let nodes = vec![
        replay::Node::Seed {
            value: serde_json::json!({"n":1}),
        },
        replay::Node::Request {
            operation: "shell.display".into(),
            arguments: serde_json::json!({"ticket":1,"command":"ls"}),
        },
        replay::Node::Check {
            value: serde_json::json!({"mode":"NORMAL"}),
        },
    ];
    let tape = replay::Tape::replay(nodes);
    let seed: serde_json::Value = tape.take_seed().unwrap();
    assert_eq!(seed["n"], 1);
    // Wrong request arguments: refused, native never consulted, fault sticky.
    assert!(tape
        .request(
            "shell.display",
            &serde_json::json!({"ticket":2,"command":"ls"})
        )
        .is_err());
    assert!(tape.healthy().is_err());
    assert!(tape
        .request(
            "shell.display",
            &serde_json::json!({"ticket":1,"command":"ls"})
        )
        .is_err());
    assert!(tape.check(&serde_json::json!({"mode":"NORMAL"})).is_err());
    assert!(tape.next::<serde_json::Value>().is_err());

    // State divergence is detected by value, not shape.
    let tape = replay::Tape::replay(vec![replay::Node::Check {
        value: serde_json::json!({"mode":"NORMAL"}),
    }]);
    assert!(tape.check(&serde_json::json!({"mode":"INSERT"})).is_err());

    // A stale unconsumed observation before the next action fails.
    let tape = replay::Tape::replay(vec![replay::Node::Call {
        operation: "read".into(),
        arguments: serde_json::json!(null),
        result: serde_json::json!(null),
    }]);
    assert!(tape.next::<serde_json::Value>().is_err());

    // Records after End are refused rather than ignored.
    let tape = replay::Tape::replay(vec![replay::Node::End, replay::Node::End]);
    assert!(tape.next::<serde_json::Value>().is_err());
}

#[test]
fn live_tape_without_capture_runs_native_and_records_nothing() {
    assert!(!capture_content());
    let tape = replay::Tape::live();
    tape.seed(&serde_json::json!({"n":1})).unwrap();
    assert!(tape
        .request("shell.display", &serde_json::json!({"ticket":1}))
        .unwrap());
    let value: u32 = tape.call("probe", &0, || 41).unwrap();
    assert_eq!(value, 41);
    tape.check(&serde_json::json!({"ok":true})).unwrap();
    tape.finish().unwrap();
    tape.healthy().unwrap();
}

#[cfg(feature = "test-support")]
mod fixture_tests {
    use super::*;
    use std::io;

    #[test]
    fn fixture_records_without_native_and_replays_identically() {
        let tape = replay::Tape::fixture(|operation, _| match operation {
            "git.discover" => Ok(serde_json::json!({"head":null})),
            _ => Err(io::Error::other("unexpected native observation")),
        });
        tape.seed(&serde_json::json!({"cwd":"/recorded"})).unwrap();
        let mut tick = replay::Tick::default();
        tick.monotonic_ms += 1;
        tape.action(tick, &serde_json::json!({"kind":"event"}))
            .unwrap();
        let context: serde_json::Value = tape
            .call("git.discover", &"/recorded", || panic!("native ran"))
            .unwrap();
        assert_eq!(context["head"], serde_json::Value::Null);
        assert!(!tape
            .request("picker.files", &serde_json::json!({"ticket":3}))
            .unwrap());
        tape.check(&serde_json::json!({"documents":1})).unwrap();
        tape.finish().unwrap();
        let nodes = tape.fixture_nodes();
        assert!(matches!(nodes.first(), Some(replay::Node::Seed { .. })));
        assert!(matches!(nodes.last(), Some(replay::Node::End)));

        // The recorded fixture replays with no responder at all.
        let replayed = replay::Tape::replay(nodes);
        let seed: serde_json::Value = replayed.take_seed().unwrap();
        assert_eq!(seed["cwd"], "/recorded");
        let action: serde_json::Value = replayed.next().unwrap().unwrap();
        assert_eq!(action["kind"], "event");
        let context: serde_json::Value = replayed
            .call("git.discover", &"/recorded", || panic!("native ran"))
            .unwrap();
        assert_eq!(context["head"], serde_json::Value::Null);
        assert!(!replayed
            .request("picker.files", &serde_json::json!({"ticket":3}))
            .unwrap());
        replayed.check(&serde_json::json!({"documents":1})).unwrap();
        assert!(replayed.next::<serde_json::Value>().unwrap().is_none());
    }

    #[test]
    fn fixture_missing_observation_fails_the_test_not_native() {
        let tape = replay::Tape::fixture(|_, _| Err(io::Error::other("no recorded value")));
        assert!(tape
            .call::<_, u32>("never.recorded", &0, || panic!("native ran"))
            .is_err());
        assert!(tape.healthy().is_err());
    }
}
