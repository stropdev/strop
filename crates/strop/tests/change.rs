//! Journeys on the real binary (plans/0094 §4, the 0091 acceptance tests):
//! every way of changing many places is one surface with one undo. Each
//! runs the shipped `strop --headless` in a tempdir and reads the disk.
use std::path::Path;
use std::process::Command;

/// Run `script` against the project at `root`, opening `file`. The script
/// and the editor's own state live beside the project, never inside the
/// tree it searches.
fn run(root: &Path, script: &str, file: &str) -> String {
    let outside = root.parent().unwrap();
    let steps = outside.join("steps");
    std::fs::write(&steps, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_strop"))
        .current_dir(root)
        .env("HOME", outside.join("home"))
        .env("XDG_CONFIG_HOME", outside.join("config"))
        .env("XDG_STATE_HOME", outside.join("state"))
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

/// Every `state` the script printed, in order.
fn states(output: &str) -> Vec<serde_json::Value> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("─── state "))
        .map(|state| serde_json::from_str(state).unwrap())
        .collect()
}

fn read(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join(name)).unwrap()
}

/// Search → collect → `cw` on three excerpts in two files → `:w` → both
/// files changed on disk and no buffer dirty; one `u` in the collection
/// reverts one edit in its source.
#[test]
fn excerpts_edit_their_sources_and_w_writes_the_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("origin.txt"), "start here\n").unwrap();
    let a = "fn first() {\n    needle_one();\n}\n\n\n\n\n\n\nfn second() {\n    needle_two();\n}\n";
    let b = "fn third() {\n    needle_three();\n}\n";
    std::fs::write(root.join("a.rs"), a).unwrap();
    std::fs::write(root.join("b.rs"), b).unwrap();
    let output = run(
        root,
        "keys <space>/\npaste \"needle_\"\nsettle\nkeys <c-o>\nsettle\nframe\n\
         keys /needle_one<cr>cwalpha<esc>\nsettle\n\
         keys /needle_two<cr>cwbeta<esc>\nsettle\n\
         keys /needle_three<cr>cwgamma<esc>\nsettle\n\
         frame\nkeys :w<cr>\nsettle\nstate\n\
         keys u\nsettle\nframe\nstate\n",
        "origin.txt",
    );
    let frames: Vec<&str> = output.split("─── frame ").collect();
    assert!(frames[1].contains("collection: search"), "{}", frames[1]);
    assert!(frames[1].contains("needle_one();"), "{}", frames[1]);
    assert!(frames[2].contains("alpha();"), "{}", frames[2]);
    assert!(frames[2].contains("gamma();"), "{}", frames[2]);

    // `:w` wrote both files and left nothing dirty
    let states = states(&output);
    assert_eq!(states[0]["message"], "collection: wrote 2 file(s)");
    assert_eq!(states[0]["dirty_buffers"], 0);
    // one `u` in the collection reverted one edit, in its source
    assert_eq!(states[1]["dirty_buffers"], 1);
    assert!(frames[3].contains("needle_three();"), "{}", frames[3]);
    assert!(frames[3].contains("beta();"), "{}", frames[3]);

    assert_eq!(
        read(root, "a.rs"),
        a.replace("needle_one", "alpha")
            .replace("needle_two", "beta")
    );
    assert_eq!(read(root, "b.rs"), b.replace("needle_three", "gamma"));
}

/// Replace across 34 files → Enter → Enter: 34 files changed on disk and
/// no buffer dirty. `u`: 34 files byte-identical to before, no buffer dirty.
#[test]
fn replace_is_two_steps_and_u_restores_every_file() {
    const FILES: usize = 34;
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("origin.txt"), "start here\n").unwrap();
    let original = |index: usize| {
        format!("// module {index}\nfn open_picker_{index}() {{}}\nlet next = open_picker;\n\ttab\tindented open_picker\n")
    };
    for index in 0..FILES {
        std::fs::write(root.join(format!("mod_{index:02}.rs")), original(index)).unwrap();
    }
    let output = run(
        root,
        "keys <space>R\npaste \"open_picker\"\nsettle\nkeys <tab>\npaste \"open_the_picker\"\nsettle\n\
         keys <cr>\nsettle\nframe\nstate\n\
         keys <cr>\nsettle\nframe\nstate\n",
        "origin.txt",
    );
    let frames: Vec<&str> = output.split("─── frame ").collect();
    assert!(
        frames[1].contains("replace: 34 file(s) to change"),
        "{}",
        frames[1]
    );
    assert!(
        frames[1].contains("Enter applies and writes"),
        "{}",
        frames[1]
    );
    assert!(
        frames[2].contains("replace: changed and wrote 34 file(s)"),
        "{}",
        frames[2]
    );
    assert!(frames[2].contains("u reverts"), "{}", frames[2]);
    let states = states(&output);
    assert_eq!(states[0]["dirty_buffers"], 0, "a review changes nothing");
    assert_eq!(states[1]["dirty_buffers"], 0, "apply wrote what it changed");
    assert_eq!(states[1]["message"], "replace: wrote 34 file(s)");
    for index in 0..FILES {
        assert_eq!(
            read(root, &format!("mod_{index:02}.rs")),
            original(index).replace("open_picker", "open_the_picker")
        );
    }

    // A headless run is read from disk when it ends, so the revert is a
    // second run from the same starting tree: replace, apply, then `u`.
    for index in 0..FILES {
        std::fs::write(root.join(format!("mod_{index:02}.rs")), original(index)).unwrap();
    }
    let output = run(
        root,
        "keys <space>R\npaste \"open_picker\"\nsettle\nkeys <tab>\npaste \"open_the_picker\"\nsettle\n\
         keys <cr>\nsettle\nkeys <cr>\nsettle\n\
         keys u\nsettle\nframe\nstate\n",
        "origin.txt",
    );
    let frame = output.rsplit("─── frame ").next().unwrap();
    assert!(
        frame.contains("replace: reverted and wrote 34 file(s)"),
        "{frame}"
    );
    assert_eq!(states_last(&output)["dirty_buffers"], 0);
    for index in 0..FILES {
        assert_eq!(
            std::fs::read(root.join(format!("mod_{index:02}.rs"))).unwrap(),
            original(index).into_bytes(),
            "mod_{index:02}.rs is byte-identical"
        );
    }
}

fn states_last(output: &str) -> serde_json::Value {
    states(output).pop().unwrap()
}

/// The quickfix names over the same list: `:cdo s/…/…/` at every hit,
/// `:cfdo w` to write, and the files on disk agree.
#[test]
fn cdo_changes_every_hit_and_cfdo_writes() {
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("origin.txt"), "start here\n").unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(root.join(name), "keep\nold value\nkeep\n").unwrap();
    }
    let output = run(
        root,
        "keys <space>/\npaste \"old value\"\nsettle\nkeys <c-o>\nsettle\n\
         keys :cdo s/old/new/<cr>\nsettle\nstate\nkeys :cfdo w<cr>\nsettle\nstate\n\
         keys ]q\nsettle\nstate\n",
        "origin.txt",
    );
    let states = states(&output);
    assert_eq!(
        states[0]["message"],
        ":cdo s/old/new/ — ran at 3 hit(s) in 3 file(s)"
    );
    assert_eq!(states[1]["dirty_buffers"], 0);
    assert!(
        states[2]["message"]
            .as_str()
            .unwrap()
            .starts_with("quickfix 1 of 3: a.txt:2"),
        "{}",
        states[2]["message"]
    );
    for name in ["a.txt", "b.txt", "c.txt"] {
        assert_eq!(read(root, name), "keep\nnew value\nkeep\n");
    }
}

/// 0091 B2/B6 on the real binary: `\C` in a `:s` pattern makes case count
/// (an all-lowercase pattern alone is smart case, 0093 §3.7), and an 8-row
/// terminal draws the command line instead of panicking.
#[test]
fn substitute_matches_case_when_asked_and_survives_a_tiny_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("graph.rs"), "struct Edge;\nlet edge = Edge;\n").unwrap();
    let output = run(
        root,
        "resize 90 8\nkeys :%s/edge\\C/rim/g\nframe\nkeys <cr>\nsettle\nkeys :w<cr>\nsettle\nstate\n",
        "graph.rs",
    );
    let frame = output.split("─── frame ").nth(1).unwrap();
    assert!(frame.contains("%s/edge\\C/rim/g"), "{frame}");
    assert_eq!(read(root, "graph.rs"), "struct Edge;\nlet rim = Edge;\n");
}
