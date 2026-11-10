use super::*;

#[test]
fn remote_git_log_and_signs_run_git_on_the_host() {
    if !ssh_required("remote_git_log_and_signs_run_git_on_the_host") {
        return;
    }
    let fixture = Fixture::new();
    let repo = fixture.root().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let source = repo.join("source.txt");
    std::fs::write(&source, "one\ntwo\n").unwrap();
    let git = |args: &[&str]| {
        successful(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@invalid",
                ])
                .args(args)
                .current_dir(&repo),
        )
    };
    git(&["init", "-q"]);
    git(&["add", "source.txt"]);
    git(&["commit", "-qm", "the remote first commit"]);
    let steps = fixture.script(
        "git.steps",
        "settle\nkeys ggciwONE<esc>\nsettle\nstate\nkeys <space>gl\nsettle\nframe\nkeys :qa!<cr>\nsettle\n",
    );
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        uri("fixture", &source).into(),
    ]);
    let observed = states(&output);
    assert!(
        observed[0]["git"].to_string().contains("ssh://fixture")
            || output.contains("the remote first commit"),
        "{output}"
    );
    let frame = output.rsplit("─── frame ").next().unwrap();
    assert!(frame.contains("the remote first commit"), "{frame}");
}
