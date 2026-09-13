use super::*;

#[test]
fn search_here_preserves_ssh_identity_through_preview_open_collection_and_replay() {
    if std::env::var_os("STROP_REQUIRE_SSH_TESTS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let fixture = Fixture::new();
    let scope = fixture.root().join("scope");
    std::fs::create_dir(&scope).unwrap();
    let source = scope.join(std::ffi::OsStr::from_bytes(b"source \xff '$;.txt"));
    std::fs::write(&source, "REMOTE needle 日本語\n").unwrap();
    let remote = uri("fixture", &scope);
    let steps = fixture.script("search.steps", &format!(
        "settle\nkeys ggccLOCAL DIRTY ONLY<esc>\nkeys :browse {remote}<cr>\nsettle\nkeys :fs search<cr>\npaste \"needle\"\nsettle\nframe\nkeys <c-r>\nstate\nkeys <c-o>\nsettle\nframe\nkeys :browse {remote}<cr>\nsettle\nkeys :fs search<cr>\nsettle\nkeys <cr>\nsettle\nkeys yy\nstate\nkeys :qa!<cr>\nsettle\n"
    ));
    let trace = fixture.root().join("search.jsonl");
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        source.clone().into(),
        "--log-file".into(),
        trace.clone().into(),
        "--log-content".into(),
    ]);
    let observed = states(&output);
    assert!(observed[0]["message"]
        .as_str()
        .unwrap()
        .contains("read-only"));
    assert_eq!(observed[1]["register"], "REMOTE needle 日本語\n");
    assert!(
        output.contains("ssh://fixture"),
        "SSH scope must be visible in the actual frame"
    );
    let collection = output.rsplit("─── frame ").next().unwrap();
    assert!(collection.contains("collection:"), "{collection}");
    assert!(collection.contains("REMOTE needle"), "{collection}");
    assert!(collection.contains("ssh://fixture"), "{collection}");
    assert!(!collection.contains("LOCAL DIRTY ONLY"), "{collection}");
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        "REMOTE needle 日本語\n"
    );
    std::fs::write(
        fixture.root().join("bin/ssh"),
        "#!/bin/sh\necho forbidden >&2\nexit 99\n",
    )
    .unwrap();
    let replay = fixture.run(&["--replay".into(), trace.into()]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&replay).unwrap()["should_quit"],
        true
    );
}
