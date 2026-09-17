//! Unit tests for the supervised exec boundary: spec validation,
//! argv shape, session records and bounded retention.
use super::*;

use crate::engine::test_support;
use crate::identity::ContainerIdentity;

fn reference() -> ContainerRef {
    let identity = ContainerIdentity {
        id: "a".repeat(64),
        name: "fixture".into(),
        image: "busybox".into(),
        started_at: "2026-09-10T08:00:00Z".into(),
        user: String::new(),
        workdir: String::new(),
    };
    ContainerRef::of(&identity).unwrap()
}

fn spec() -> ExecSpec {
    ExecSpec::new(
        &test_support::engine_fixture(),
        &reference(),
        "pyright-langserver",
        &["--stdio".into()],
        Path::new("/src"),
    )
    .unwrap()
}

fn args_of(command: &Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn spec_validation_refuses_unsafe_shape_before_any_engine() {
    let engine = test_support::engine_fixture();
    let reference = reference();
    let cwd = Path::new("/src");
    assert!(ExecSpec::new(&engine, &reference, "", &[], cwd).is_err());
    assert!(ExecSpec::new(&engine, &reference, "bad\0prog", &[], cwd).is_err());
    assert!(
        ExecSpec::new(&engine, &reference, "git", &["x\0y".into()], cwd).is_err(),
        "NUL in argv"
    );
    let valid = ExecSpec::new(&engine, &reference, "git", &[], cwd).unwrap();
    assert!(valid.clone().with_user("-evil").is_err());
    assert!(valid.clone().with_user("").is_err());
    assert!(valid
        .clone()
        .with_env(&[("BAD=NAME".into(), "v".into())])
        .is_err());
    assert!(valid
        .with_env(&[("GOOD".into(), "value with spaces".into())])
        .is_ok());
}

#[test]
fn command_is_pinned_argv_only_supervised_stdio() {
    let admitted = AdmittedExec {
        spec: spec()
            .with_user("1000")
            .unwrap()
            .with_env(&[("RUST_LOG".into(), "debug".into())])
            .unwrap(),
        key: SessionKey::fixed([7u8; 16]),
    };
    let command = admitted.command();
    assert_eq!(command.get_program(), "docker");
    let args = args_of(&command);
    // The probe-pinned connection leads.
    assert_eq!(args[..3], ["--context", "test-context", "exec"]);
    assert!(args.contains(&"-i".to_string()));
    // Workdir/principal/env are options before the container id.
    let id_at = args.iter().position(|arg| arg == &"a".repeat(64)).unwrap();
    assert_eq!(
        args[id_at - 1],
        "RUST_LOG=debug",
        "env value immediately precedes the id"
    );
    let window = &args[..id_at];
    assert!(window.windows(2).any(|pair| pair == ["--workdir", "/src"]));
    assert!(window.windows(2).any(|pair| pair == ["--user", "1000"]));
    assert!(window
        .windows(2)
        .any(|pair| pair == ["--env", "RUST_LOG=debug"]));
    // After the id: the fixed supervisor, then inert positional data.
    let tail = &args[id_at + 1..];
    assert_eq!(tail[0], "sh");
    assert_eq!(tail[1], "-c");
    assert!(tail[2].contains("STROP-EXEC-v1"), "fixed supervisor source");
    assert_eq!(tail[3], "strop-supervisor");
    assert_eq!(tail[4], SessionKey::fixed([7u8; 16]).nonce_hex());
    assert_eq!(tail[6], "relay", "command() is the interactive channel");
    assert_eq!(tail[7], "pyright-langserver");
    assert_eq!(tail[8], "--stdio");
    // The program's argv is never shell-interpolated: exactly one -c
    // payload exists and it is the fixed source.
    assert_eq!(tail.iter().filter(|arg| *arg == "-c").count(), 1);
    assert!(
        !tail[2].contains("pyright"),
        "program never enters the source"
    );
}

#[test]
fn capture_mode_selects_the_null_lease() {
    let admitted = AdmittedExec {
        spec: spec(),
        key: SessionKey::fixed([9u8; 16]),
    };
    let command = admitted.build(Mode::Null);
    let args = args_of(&command);
    let id_at = args.iter().position(|arg| arg == &"a".repeat(64)).unwrap();
    assert_eq!(
        args[id_at + 7],
        "null",
        "capture() is the finite null-lease mode"
    );
}

#[test]
fn nonces_are_unique_per_session() {
    assert_ne!(SessionKey::new(), SessionKey::new());
}

#[test]
fn records_parse_only_this_sessions_marked_lines() {
    let key = SessionKey::fixed([0xab; 16]);
    let hex = key.nonce_hex();
    let foreign = SessionKey::fixed([0xcd; 16]).nonce_hex();
    let stderr = format!(
        "random program output\n\
         STROP-EXEC-v1 {hex} launched 42\n\
         STROP-EXEC-v1 {foreign} launched 999\n\
         STROP-EXEC-v1 {hex} lease-closed\n\
         garbage STROP-EXEC-v1 {hex} terminated\n\
         STROP-EXEC-v1 {hex} exec-error not-found git\n\
         STROP-EXEC-v1 {hex} exec-error not-executable /opt/tool shim\n\
         STROP-EXEC-v1 {hex} exit 127\n"
    );
    let records = key.records(stderr.as_bytes());
    assert_eq!(
        records,
        vec![
            ExecRecord::Launched { pid: 42 },
            ExecRecord::LeaseClosed,
            // The interleaved line is still this session's record.
            ExecRecord::Terminated,
            ExecRecord::LaunchFailed {
                program: "git".into(),
                cause: LaunchCause::NotFound,
            },
            ExecRecord::LaunchFailed {
                program: "/opt/tool shim".into(),
                cause: LaunchCause::NotExecutable,
            },
            ExecRecord::Exit { code: 127 },
        ]
    );
    assert!(foreign_key_records_empty(&stderr));
}

fn foreign_key_records_empty(stderr: &str) -> bool {
    SessionKey::new().records(stderr.as_bytes()).is_empty()
}

#[test]
fn retain_keeps_head_plus_tail_and_counts_the_middle() {
    let data: Vec<u8> = (0..1000u32).flat_map(|n| n.to_le_bytes()).collect();
    let kept = retain(&data[..], 100, 50).unwrap();
    assert_eq!(kept.bytes.len(), 150);
    assert_eq!(&kept.bytes[..100], &data[..100]);
    assert_eq!(&kept.bytes[100..], &data[data.len() - 50..]);
    assert_eq!(kept.dropped, (data.len() - 150) as u64);
    let small = retain(&data[..200], 100, 0).unwrap();
    assert_eq!(small.bytes.len(), 100);
    assert_eq!(small.dropped, 100);
}
