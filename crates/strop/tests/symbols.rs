//! `Space s` on the real binary (plans/0099 §6): a file no language server
//! serves lists its declarations, and the list is never one from before an
//! edit. Runs the shipped `strop --headless` in a tempdir with an empty
//! `PATH`, so no server can be found whatever the machine has installed.
use std::path::Path;
use std::process::Command;

fn run(root: &Path, script: &str, file: &str) -> String {
    let outside = root.parent().unwrap();
    let steps = outside.join("steps");
    std::fs::write(&steps, script).unwrap();
    let no_programs = outside.join("bin");
    std::fs::create_dir_all(&no_programs).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(root)
        .env("HOME", outside.join("home"))
        .env("XDG_CONFIG_HOME", outside.join("config"))
        .env("XDG_STATE_HOME", outside.join("state"))
        .env("PATH", &no_programs)
        .env_remove("STROP_LOG")
        .arg("--headless")
        .arg(&steps)
        .arg(file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Each printed frame, in order.
fn frames(output: &str) -> Vec<&str> {
    output.split("─── frame ").skip(1).collect()
}

/// The picker row naming `symbol` in one frame.
fn row<'a>(frame: &'a str, symbol: &str) -> &'a str {
    frame
        .lines()
        .find(|line| line.contains(&format!(" {symbol} ")) && line.contains(" fn "))
        .unwrap_or_else(|| panic!("no row for {symbol} in:\n{frame}"))
}

#[test]
fn symbols_without_a_server_follow_every_edit() {
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("lib.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
    let output = run(
        root,
        "settle 20000\n\
         keys <space>s\n\
         settle 20000\n\
         frame\n\
         keys <esc><esc>\n\
         keys O// moved<esc>\n\
         keys <space>s\n\
         settle 20000\n\
         frame\n",
        "lib.rs",
    );
    let frames = frames(&output);
    assert_eq!(frames.len(), 2, "{output}");
    assert!(frames[0].contains("symbols"), "{}", frames[0]);
    assert!(row(frames[0], "alpha").contains(":1"), "{}", frames[0]);
    assert!(row(frames[0], "beta").contains(":2"), "{}", frames[0]);
    // one line was opened above both: the rows moved with the text
    assert!(row(frames[1], "alpha").contains(":2"), "{}", frames[1]);
    assert!(row(frames[1], "beta").contains(":3"), "{}", frames[1]);
}

/// `python3` from the test's own `PATH`: the editor itself runs with none.
fn python() -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("python3"))
        .find(|path| path.is_file())
        .expect("python3 on PATH")
}

#[test]
fn a_symbol_reply_too_large_to_decode_fails_one_request_and_keeps_the_server() {
    // plans/0101 §4.2: the server is asked ahead of the key about a
    // 6,000-line file, answers with 40,000 symbols — over the decode budget
    // — and the connection lives: the press asks again and gets its list.
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::write(root.join("lib.rs"), "fn f() {}\n".repeat(6_000)).unwrap();
    let log = directory.path().join("server.log");
    let config = directory.path().join("config/strop");
    std::fs::create_dir_all(&config).unwrap();
    let server = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/symbols_server.py"
    );
    std::fs::write(
        config.join("languages.toml"),
        format!(
            "[language-server.rust-analyzer]\ncommand = {:?}\nargs = [{server:?}, {:?}]\n",
            python().display().to_string(),
            log.display().to_string()
        ),
    )
    .unwrap();
    let output = run(
        root,
        "settle 20000\n\
         keys <space>s\n\
         settle 20000\n\
         frame\n",
        "lib.rs",
    );
    let frames = frames(&output);
    assert_eq!(frames.len(), 1, "{output}");
    assert!(frames[0].contains("only_symbol"), "{}", frames[0]);
    assert!(!frames[0].contains("failed"), "{}", frames[0]);
    let asked = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        asked.lines().collect::<Vec<_>>(),
        ["initialize", "documentSymbol", "documentSymbol"],
        "one server, asked ahead and then by the press"
    );
}
