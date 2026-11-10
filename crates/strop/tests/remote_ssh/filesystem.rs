use super::*;

#[test]
fn remote_files_are_created_copied_renamed_and_removed_through_review() {
    if !ssh_required("remote_files_are_created_copied_renamed_and_removed_through_review") {
        return;
    }
    let fixture = Fixture::new();
    let scope = fixture.root().join("workspace");
    std::fs::create_dir(&scope).unwrap();
    let original = scope.join("original.txt");
    std::fs::write(&original, "original\n").unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o640)).unwrap();
    let doomed = scope.join("doomed.txt");
    std::fs::write(&doomed, "doomed\n").unwrap();
    std::fs::create_dir(scope.join("empty-dir")).unwrap();
    let steps = fixture.script(
        "operations.steps",
        &format!(
            "settle\n\
             keys :fs create nested/new.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\n\
             keys :e {original}<cr>\nsettle\n\
             keys :fs copy stored copy.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\n\
             keys :e {original}<cr>\nsettle\n\
             keys :fs rename renamed.txt<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nkeys <c-^>\nsettle\nframe\n\
             keys :e {doomed}<cr>\nsettle\nkeys :fs remove<cr>\nsettle\nframe\nkeys :apply-change<cr>\nsettle\n\
             keys :e {scope}/<cr>\nsettle\nkeys /empty-dir<cr>:fs remove<cr>\nsettle\nstate\nkeys :apply-change<cr>\nsettle\nstate\n\
             keys :qa!<cr>\nsettle\n",
            original = uri("fixture", &original),
            doomed = uri("fixture", &doomed),
            scope = uri("fixture", &scope),
        ),
    );
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        uri("fixture", &scope).into(),
    ]);
    // Creation made the missing parent as its own step.
    assert_eq!(
        std::fs::read(scope.join("nested/new.txt")).unwrap(),
        b"",
        "{output}"
    );
    // Copy kept the bytes and the permission bits.
    let copy = scope.join("copy.txt");
    assert_eq!(std::fs::read(&copy).unwrap(), b"original\n", "{output}");
    assert_eq!(
        std::fs::metadata(&copy).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    // Rename moved the file and the open buffer followed it.
    assert!(!original.exists(), "{output}");
    assert_eq!(
        std::fs::read(scope.join("renamed.txt")).unwrap(),
        b"original\n"
    );
    let followed = output.split("─── frame ").nth(1).unwrap();
    let modeline = followed
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap();
    assert!(modeline.contains("renamed.txt"), "{followed}");
    // Removal is permanent and says so in the review.
    let review = output.rsplit("─── frame ").next().unwrap();
    assert!(review.contains("PERMANENT DELETION"), "{review}");
    assert!(review.contains("no Trash"), "{review}");
    assert!(!doomed.exists(), "{output}");
    assert!(!scope.join("empty-dir").exists(), "{output}");
}

#[test]
fn remote_operations_never_replace_an_existing_name() {
    if !ssh_required("remote_operations_never_replace_an_existing_name") {
        return;
    }
    let fixture = Fixture::new();
    let scope = fixture.root().join("workspace");
    std::fs::create_dir(&scope).unwrap();
    let original = scope.join("original.txt");
    std::fs::write(&original, "original\n").unwrap();
    std::fs::write(scope.join("taken.txt"), "taken\n").unwrap();
    std::fs::write(scope.join("full.txt"), "full\n").unwrap();
    std::fs::create_dir(scope.join("full-dir")).unwrap();
    std::fs::write(scope.join("full-dir/child"), "child\n").unwrap();
    // Each occupied destination is refused in the review (cancelled, not
    // applied); a non-empty directory is refused by the host at apply.
    let steps = fixture.script(
        "occupied.steps",
        &format!(
            "settle\n\
             keys :fs create taken.txt<cr>\nsettle\nframe\nkeys :cancel-change<cr>\nsettle\n\
             keys :e {original}<cr>\nsettle\n\
             keys :fs rename full.txt<cr>\nsettle\nframe\nkeys :cancel-change<cr>\nsettle\n\
             keys :e {original}<cr>\nsettle\n\
             keys :fs copy stored taken.txt<cr>\nsettle\nframe\nkeys :cancel-change<cr>\nsettle\n\
             keys :e {scope}/<cr>\nsettle\nkeys /full-dir<cr>:fs remove<cr>\nsettle\nkeys :apply-change<cr>\nsettle\nframe\n\
             keys :e {scope}/<cr>\nsettle\nkeys /taken<cr>:fs trash<cr>\nsettle\nframe\nkeys /REFUSED Trash<cr>yy\nstate\nkeys :cancel-change<cr>\nsettle\n\
             keys :qa!<cr>\nsettle\n",
            original = uri("fixture", &original),
            scope = uri("fixture", &scope),
        ),
    );
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        uri("fixture", &scope).into(),
    ]);
    let frames: Vec<&str> = output.split("─── frame ").skip(1).collect();
    assert_eq!(frames.len(), 5, "{output}");
    // Paths are long here, so the reason is clipped at 100 columns;
    // the refusal and the untouched files are the evidence.
    for refused in &frames[..3] {
        assert!(
            refused.contains("0 exact steps; 1 refused") && refused.contains("REFUSED"),
            "{refused}"
        );
    }
    assert_eq!(std::fs::read(scope.join("taken.txt")).unwrap(), b"taken\n");
    assert_eq!(std::fs::read(scope.join("full.txt")).unwrap(), b"full\n");
    assert_eq!(std::fs::read(&original).unwrap(), b"original\n");
    // The host refused the non-empty directory; it is untouched.
    assert!(
        frames[3].contains("REFUSED") && frames[3].contains("must be empty"),
        "{}",
        frames[3]
    );
    assert!(scope.join("full-dir/child").exists());
    // An SSH host has no Trash: the review refuses it and says why.
    assert!(frames[4].contains("REFUSED Trash"), "{}", frames[4]);
    let refusal = states(&output)[0]["register"].as_str().unwrap().to_owned();
    assert!(refusal.contains("no Trash"), "{refusal}");
}
