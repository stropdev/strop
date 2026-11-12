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
