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

/// `cmd | strop -`: stdin becomes the buffer (headless reads it whole).
#[test]
fn a_dash_operand_edits_stdin() {
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let script = root.join("steps.keys");
    std::fs::write(&script, "keys jx\nframe\nstate\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env_remove("STROP_LOG")
        .args(["headless".as_ref(), script.as_os_str(), OsStr::new("-")])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"piped one\npiped two\n")
        .unwrap();
    let output = text(child.wait_with_output().unwrap());
    assert!(output.contains("piped one"), "{output}");
    assert!(
        output.contains("iped two"),
        "x edited the stdin text: {output}"
    );
    assert!(
        output.contains(" stdin "),
        "the buffer is named stdin: {output}"
    );
    assert_eq!(first_line(&output), 2);
}

fn message(output: &str) -> String {
    let state = output
        .lines()
        .find_map(|line| line.strip_prefix("─── state "))
        .unwrap();
    serde_json::from_str::<serde_json::Value>(state).unwrap()["message"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// 0108 H6: the file on the command line goes through `:e`'s guarded
/// read. A FIFO, a device and a file that is not UTF-8 reach the editor
/// with `:e`'s refusal on an empty buffer — never a hang, an unbounded
/// read or an exit.
#[test]
fn startup_file_is_refused_exactly_like_e() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    std::fs::write(root.join("steps"), "state\nkeys ihello<esc>\nframe\n").unwrap();
    let fifo = std::ffi::CString::new(root.join("pipe").as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: `fifo` is a NUL-terminated path that outlives the call.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::fs::write(root.join("latin.txt"), b"caf\xe9\n").unwrap();
    for (operand, refusal) in [
        ("pipe", "only regular files and directories can be opened"),
        (
            "/dev/zero",
            "only regular files and directories can be opened",
        ),
        ("latin.txt", "stream contained invalid UTF-8"),
    ] {
        // a FIFO read blocks forever and /dev/zero never ends: the run
        // owns a thread and the test a deadline, so a regression fails
        let (sender, receiver) = std::sync::mpsc::channel();
        let workdir = root.clone();
        std::thread::spawn(move || {
            let args = ["--headless".into(), "steps".into(), operand.into()];
            let _ = sender.send(run(&workdir, &args));
        });
        let output = receiver
            .recv_timeout(std::time::Duration::from_secs(20))
            .unwrap_or_else(|_| panic!("{operand}: strop did not reach the editor"));
        let output = text(output);
        let shown = message(&output);
        assert!(
            shown.starts_with("open ") && shown.ends_with(refusal),
            "{operand}: {shown}"
        );
        assert!(output.contains("hello"), "{operand}: the editor runs");
    }
}
