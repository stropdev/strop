//! Real command-line locations, including ambiguous and native filenames.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Output};

fn run(root: &Path, args: &[OsString]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env_remove("STROP_LOG")
        .args(args)
        .output()
        .unwrap()
}

fn text(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn first_line(output: &str) -> u64 {
    let state = output
        .lines()
        .find_map(|line| line.strip_prefix("─── state "))
        .unwrap();
    serde_json::from_str::<serde_json::Value>(state).unwrap()["line"]
        .as_u64()
        .unwrap()
}

#[test]
fn both_location_forms_position_before_the_first_edit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let file = root.join("file with spaces.txt");
    let script = root.join("steps");
    std::fs::write(&file, "alpha\nbeta\ngamma\ndelta\n").unwrap();
    std::fs::write(&script, "state\nkeys dd\nframe\n").unwrap();
    let mut suffix = file.as_os_str().to_owned();
    suffix.push(":3");
    for location in [
        vec![OsString::from("+3"), file.as_os_str().to_owned()],
        vec![suffix],
    ] {
        let mut args = vec![OsString::from("--headless"), script.as_os_str().to_owned()];
        args.extend(location);
        let output = text(run(root, &args));
        assert_eq!(first_line(&output), 3);
        assert!(output.contains("beta") && output.contains("delta"));
        assert!(
            !output.contains("gamma"),
            "the first edit targets the requested line"
        );
    }
}

#[test]
fn line_beyond_end_clamps_to_the_last_content_line() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("file.txt"), "one\ntwo\n").unwrap();
    std::fs::write(root.join("steps"), "state\n").unwrap();
    let output = text(run(
        root,
        &[
            "--headless".into(),
            "steps".into(),
            "+99".into(),
            "file.txt".into(),
        ],
    ));
    assert_eq!(first_line(&output), 2);
}

#[test]
fn literal_separator_protects_a_real_colon_filename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("literal:3"), "literal filename body\n").unwrap();
    std::fs::write(root.join("literal"), "wrong file\n").unwrap();
    std::fs::write(root.join("steps"), "frame\nstate\n").unwrap();
    let output = text(run(
        root,
        &[
            "--headless".into(),
            "steps".into(),
            "--".into(),
            "literal:3".into(),
        ],
    ));
    assert_eq!(first_line(&output), 1);
    assert!(output.contains("literal filename body"));
    assert!(!output.contains("wrong file"));
}

#[test]
fn invalid_explicit_location_is_a_launch_error() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("file.txt"), "keep\n").unwrap();
    std::fs::write(root.join("steps"), "keys dd\n").unwrap();
    let output = run(
        root,
        &[
            "--headless".into(),
            "steps".into(),
            "+0".into(),
            "file.txt".into(),
        ],
    );
    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
    assert_eq!(std::fs::read(root.join("file.txt")).unwrap(), b"keep\n");
}

#[cfg(unix)]
#[test]
fn native_filename_bytes_survive_the_real_cli() {
    use std::os::unix::ffi::OsStrExt;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join(OsStr::from_bytes(b"\xff.txt")),
        "one\ntwo\nthree\n",
    )
    .unwrap();
    std::fs::write(root.join("steps"), "state\nframe\n").unwrap();
    let output = text(run(
        root,
        &[
            "--headless".into(),
            "steps".into(),
            OsStr::from_bytes(b"\xff.txt:3").to_owned(),
        ],
    ));
    assert_eq!(first_line(&output), 3);
    assert!(output.contains("three"));
}
