//! Process-isolated regressions for capture privacy, shutdown and script extraction.
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

fn run(directory: &Path, args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(directory)
        .env("HOME", directory.join("home"))
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("XDG_STATE_HOME", directory.join("state"))
        .env_remove("STROP_LOG")
        .args(args)
        .output()
        .unwrap()
}
fn successful(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn events(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn full_trace_roundtrips_literal_keys_and_closes_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let file = root.join("input.txt");
    let script = root.join("input.keys");
    let trace = root.join("session.jsonl");
    std::fs::write(&file, "needle\n").unwrap();
    std::fs::write(&script, "keys A<space>é<lt>x<gt><esc>\nframe\nstate\n").unwrap();
    let output = successful(run(
        root,
        &[
            "--headless".as_ref(),
            script.as_os_str(),
            file.as_os_str(),
            "--log-file".as_ref(),
            trace.as_os_str(),
            "--log-content".as_ref(),
        ],
    ));
    let captured = events(&trace);
    assert_eq!(captured.first().unwrap()["event"], "session_start");
    assert_eq!(captured.last().unwrap()["event"], "session_end");
    assert_eq!(captured.last().unwrap()["fields"]["success"], true);
    let first_edit = captured
        .iter()
        .position(|event| event["event"] == "mutation")
        .unwrap();
    let commit = captured
        .iter()
        .position(|event| event["event"] == "history")
        .unwrap();
    assert!(
        first_edit < commit,
        "uncommitted typing is visible before Esc"
    );
    let text = successful(run(root, &["--replay-script".as_ref(), trace.as_os_str()]));
    let replay = root.join("replay.keys");
    std::fs::write(&replay, text).unwrap();
    let replayed = successful(run(root, &["--headless".as_ref(), replay.as_os_str()]));
    let final_state = |text: &str| -> Value {
        serde_json::from_str(
            text.lines()
                .find_map(|line| line.strip_prefix("─── state "))
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(final_state(&replayed), final_state(&output));
    assert!(output.contains("needle é<x>"));

    let quit_script = root.join("quit.keys");
    let quit_trace = root.join("quit.jsonl");
    std::fs::write(&quit_script, "keys :q!<cr>\n").unwrap();
    successful(run(
        root,
        &[
            "--headless".as_ref(),
            quit_script.as_os_str(),
            file.as_os_str(),
            "--log-file".as_ref(),
            quit_trace.as_os_str(),
        ],
    ));
    let quit = events(&quit_trace);
    assert!(quit
        .iter()
        .any(|event| event["event"] == "state" && event["fields"]["should_quit"] == true));
    assert_eq!(quit.last().unwrap()["event"], "session_end");
}

#[test]
fn metadata_does_not_disclose_paste_and_existing_log_is_not_truncated() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let script = root.join("paste.keys");
    let trace = root.join("private.jsonl");
    let secret = "unique clipboard credential 7c3a80";
    std::fs::write(
        &script,
        format!("paste {}\nstate\n", serde_json::to_string(secret).unwrap()),
    )
    .unwrap();
    successful(run(
        root,
        &[
            "--script".as_ref(),
            script.as_os_str(),
            "--log-file".as_ref(),
            trace.as_os_str(),
        ],
    ));
    let saved = std::fs::read_to_string(&trace).unwrap();
    assert!(
        !saved.contains(secret),
        "metadata capture must not leak clipboard text through state or frames"
    );
    let paste = events(&trace)
        .into_iter()
        .find(|event| event["event"] == "paste")
        .unwrap();
    assert_eq!(paste["fields"]["bytes"], secret.len());
    assert!(paste["fields"]["text"].is_null());
    let failed = run(
        root,
        &[
            "--script".as_ref(),
            script.as_os_str(),
            "--log-file".as_ref(),
            trace.as_os_str(),
        ],
    );
    assert!(!failed.status.success());
    assert_eq!(std::fs::read_to_string(&trace).unwrap(), saved);
}
