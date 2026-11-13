//! 0102: the folder view end to end on the real binary — browse in and
//! out, edit the names, `:w`, `y`, and the disk is what the screen said.
use super::*;

/// The row whose name is `name`, with the cursor mark when `cursor`.
fn row(screen: &str, name: &str, cursor: bool) -> bool {
    screen.lines().any(|row| {
        row.starts_with('▌') == cursor
            && row
                .split_whitespace()
                .nth(usize::from(cursor) + 1)
                .is_some_and(|text| text == name)
    })
}

#[test]
fn browse_edit_names_write_and_apply() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(project.join("doomed.txt"), "doomed\n").unwrap();
    std::fs::write(project.join("old.txt"), "kept\n").unwrap();
    let trace = directory.path().join("folder.jsonl");
    let mut tui = Tui::start(directory.path(), &trace);
    tui.until(|screen| screen.contains("NORMAL"));

    tui.send(b":e project\r");
    tui.until(|screen| {
        row(screen, "src/", true) && screen.contains("project/") && screen.contains("3 items")
    });
    tui.send(b"\r");
    tui.until(|screen| row(screen, "main.rs", true) && screen.contains("project/src/"));
    tui.send(b"-");
    tui.until(|screen| row(screen, "src/", true) && row(screen, "old.txt", false));

    // Each Esc lands (NORMAL again) before the next key, so the two never
    // read as one Alt chord.
    tui.send(b"Gccrenamed.txt\x1b");
    tui.until(|screen| {
        screen.contains(" NORMAL ")
            && screen
                .lines()
                .any(|row| row.contains("renamed.txt") && row.contains("was old.txt"))
    });
    tui.send(b"onew.txt\x1b");
    tui.until(|screen| {
        screen.contains(" NORMAL ")
            && screen
                .lines()
                .any(|row| row.contains("new.txt") && row.trim_end().ends_with(" new"))
    });
    tui.send(b"gg/^doomed\rdd");
    tui.until(|screen| !screen.contains("doomed.txt"));
    tui.send(b":w\r");
    let confirm = tui.until(|screen| screen.contains("y apply   q cancel"));
    assert!(confirm.contains("project/ — 3 changes"), "{confirm}");
    assert!(
        confirm.contains("rename  old.txt → renamed.txt"),
        "{confirm}"
    );
    assert!(confirm.contains("create  new.txt"), "{confirm}");
    assert!(confirm.contains("trash   doomed.txt"), "{confirm}");
    tui.send(b"y");
    tui.until(|screen| screen.contains("3 changes applied") && row(screen, "renamed.txt", true));

    assert_eq!(
        std::fs::read_to_string(project.join("renamed.txt")).unwrap(),
        "kept\n"
    );
    assert_eq!(std::fs::read(project.join("new.txt")).unwrap(), b"");
    assert!(!project.join("old.txt").exists());
    assert!(!project.join("doomed.txt").exists());
    let mut names: Vec<_> = std::fs::read_dir(&project)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["new.txt", "renamed.txt", "src"],
        "no lock files left"
    );

    // Enter at once: the renamed row opens its new name even while the
    // folder is still being listed again. Space e comes back to it.
    tui.send(b"\r");
    tui.until(|screen| numbered_line(screen, "kept"));
    tui.send(b" e");
    tui.until(|screen| row(screen, "renamed.txt", true));
    tui.send(b":qa!\r");
    tui.expect_clean_exit("the editor did not exit cleanly");
}
