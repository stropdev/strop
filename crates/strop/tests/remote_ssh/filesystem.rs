use super::*;

#[test]
fn remote_file_operations_are_refused_and_change_nothing() {
    if !ssh_required("remote_file_operations_are_refused_and_change_nothing") {
        return;
    }
    let fixture = Fixture::new();
    let scope = fixture.root().join("workspace");
    std::fs::create_dir(&scope).unwrap();
    let original = scope.join("original.txt");
    std::fs::write(&original, "original\n").unwrap();
    let steps = fixture.script(
        "operations.steps",
        "settle\nkeys :fs create created.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nstate\nkeys :qa!<cr>\nsettle\n",
    );
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        uri("fixture", &scope).into(),
    ]);
    let message = states(&output)[0]["message"].as_str().unwrap().to_owned();
    assert!(
        message.contains("not supported") && message.contains(":terminal"),
        "{output}"
    );
    assert!(!scope.join("created.txt").exists());
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "original\n");
}
