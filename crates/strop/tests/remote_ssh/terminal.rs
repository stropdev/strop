use super::*;

#[test]
fn ssh_terminal_runs_on_the_host_takes_ctrl_c_and_retains_launch_identity() {
    if !ssh_required("ssh_terminal_runs_on_the_host_takes_ctrl_c_and_retains_launch_identity") {
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
        "settle\nkeys :terminal /bin/sh<cr>\nkeys echo TYPED_AHEAD<cr>\nwait 1000\nsettle\nkeys sleep 30<cr>\nwait 300\nkeys <c-c>\nkeys echo REMOTE_$((40+2))<cr>\nwait 1000\nsettle\nkeys <c-\\><c-n>\nstate\nframe\nkeys :qa!<cr>\nsettle\n",
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
    // Keys typed before the remote shell was ready were not lost, and
    // Ctrl-C reached the remote `sleep` (the next command ran at once).
    assert!(output.contains("TYPED_AHEAD"), "{output}");
    assert!(output.contains("REMOTE_42"), "{output}");
    let frame = output.rsplit("─── frame ").next().unwrap();
    assert!(frame.contains("ssh://fixture"), "{frame}");
    assert!(!frame.contains("launch local"), "{frame}");
}
