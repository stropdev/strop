use super::*;

#[test]
fn admitted_ssh_terminal_executes_remotely_and_retains_launch_identity() {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let fixture = Fixture::new();
    let scope = fixture.root().join("terminal-workspace");
    std::fs::create_dir(&scope).unwrap();
    let source = scope.join("source.txt");
    std::fs::write(&source, "remote source\n").unwrap();
    let target = uri("fixture", &source);
    let steps = fixture.script(
        "terminal.steps",
        "settle\nkeys :remote worker<cr>\nsettle\nkeys :terminal /bin/sh<cr>\nsettle\nkeys echo REMOTE_TERMINAL<cr>\nwait 1000\nsettle\nkeys <f2>\nstate\nframe\nkeys :qa!<cr>\nsettle\n",
    );
    let output = fixture.run(&["--headless".into(), steps.into(), target.into()]);
    let observed = states(&output);
    let terminal = observed.last().unwrap()["terminal"].clone();
    assert!(
        terminal["namespace"]
            .as_str()
            .unwrap_or_default()
            .starts_with("ssh://fixture"),
        "{output}"
    );
    assert!(
        terminal["launch_location"]
            .as_str()
            .unwrap_or_default()
            .contains("terminal-workspace"),
        "{output}"
    );
    assert!(output.contains("REMOTE_TERMINAL"), "{output}");
    let frame = output.rsplit("─── frame ").next().unwrap();
    assert!(frame.contains("ssh://fixture"), "{frame}");
    assert!(!frame.contains("launch local"), "{frame}");
}
