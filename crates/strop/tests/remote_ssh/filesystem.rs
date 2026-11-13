use super::*;

#[test]
fn a_remote_folder_creates_copies_renames_and_deletes_in_one_write() {
    if !ssh_required("a_remote_folder_creates_copies_renames_and_deletes_in_one_write") {
        return;
    }
    let fixture = Fixture::new();
    let scope = fixture.root().join("workspace");
    std::fs::create_dir(&scope).unwrap();
    let original = scope.join("original.txt");
    std::fs::write(&original, "original\n").unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o640)).unwrap();
    let moved = scope.join("moved.txt");
    std::fs::write(&moved, "moved\n").unwrap();
    let doomed = scope.join("doomed.txt");
    std::fs::write(&doomed, "doomed\n").unwrap();
    std::fs::create_dir(scope.join("empty-dir")).unwrap();
    // The folder lists empty-dir/, doomed.txt, moved.txt, original.txt.
    // One write creates (with its missing parent), copies, renames and
    // deletes. (One file is never both copied and renamed in one write.)
    let steps = fixture.script(
        "operations.steps",
        &format!(
            "settle\n\
             keys :e {moved}<cr>\nsettle\n\
             keys :e {scope}/<cr>\nsettle\n\
             keys Gonested/new.txt<esc>\n\
             keys /^original<cr>yypcccopy.txt<esc>\n\
             keys gg/^moved<cr>ccrenamed.txt<esc>\n\
             keys gg/^doomed<cr>dd\n\
             keys gg/^empty-dir<cr>dd\n\
             keys :w<cr>\nsettle\nframe\nkeys y\nsettle\n\
             keys :b renamed<cr>\nsettle\nframe\n\
             keys :qa!<cr>\nsettle\n",
            moved = uri("fixture", &moved),
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
        std::fs::read(scope.join("nested/new.txt"))
            .unwrap_or_else(|error| panic!("{error}\n{output}")),
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
    assert!(!moved.exists(), "{output}");
    assert_eq!(
        std::fs::read(scope.join("renamed.txt")).unwrap(),
        b"moved\n"
    );
    assert_eq!(std::fs::read(&original).unwrap(), b"original\n");
    let frames: Vec<&str> = output.split("─── frame ").skip(1).collect();
    assert_eq!(frames.len(), 2, "{output}");
    // An SSH host has no Trash: the confirm says delete, not trash.
    assert!(frames[0].contains("delete  doomed.txt"), "{}", frames[0]);
    assert!(frames[0].contains("delete  empty-dir/"), "{}", frames[0]);
    assert!(!frames[0].contains("trash"), "{}", frames[0]);
    let modeline = frames[1]
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap();
    assert!(modeline.contains("renamed.txt"), "{}", frames[1]);
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
    // The folder lists full-dir/, full.txt, original.txt, taken.txt.
    // A name already in the folder is refused before anything is
    // prepared; a non-empty folder is refused by the host when applied.
    let steps = fixture.script(
        "occupied.steps",
        "settle\n\
             keys /^original<cr>ccfull.txt<esc>:w<cr>\nsettle\nstate\nkeys :e!<cr>\nsettle\n\
             keys /^original<cr>yypcctaken.txt<esc>:w<cr>\nsettle\nstate\nkeys :e!<cr>\nsettle\n\
             keys gg/^full-dir<cr>dd:w<cr>\nsettle\nkeys y\nsettle\nframe\n\
             keys :qa!<cr>\nsettle\n",
    );
    let output = fixture.run(&[
        "--headless".into(),
        steps.into(),
        uri("fixture", &scope).into(),
    ]);
    for state in &states(&output)[..2] {
        let message = state["message"].as_str().unwrap();
        assert!(message.contains("name one destination"), "{message}");
    }
    let frames: Vec<&str> = output.split("─── frame ").skip(1).collect();
    assert_eq!(frames.len(), 1, "{output}");
    assert_eq!(std::fs::read(scope.join("taken.txt")).unwrap(), b"taken\n");
    assert_eq!(std::fs::read(scope.join("full.txt")).unwrap(), b"full\n");
    assert_eq!(std::fs::read(&original).unwrap(), b"original\n");
    // The host refused the non-empty folder; it is untouched and the
    // result says so.
    assert!(
        frames[0].contains("0 of 1 change applied") && frames[0].contains("failed"),
        "{}",
        frames[0]
    );
    assert!(scope.join("full-dir/child").exists());
}
