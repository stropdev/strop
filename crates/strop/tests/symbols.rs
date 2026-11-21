//! `Space s` on the real binary (plans/0099 §6): a file no language server
//! serves lists its declarations, and the list is never one from before an
//! edit. Runs the shipped `strop headless` in a tempdir with an empty
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
        .arg("headless")
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

/// The owner's report (plan 0109): `kind:` in `Space s` filters, the
/// words beside it stay fuzzy, and an unknown kind says so under the
/// word instead of keeping everything.
#[test]
fn symbols_kind_filters_and_keeps_the_words_fuzzy() {
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(
        root.join("lib.rs"),
        "struct Config {}\nfn parse_config() {}\nfn wrap() {}\n",
    )
    .unwrap();
    let output = run(
        root,
        "settle 20000\n\
         keys <space>s\n\
         settle 20000\n\
         keys kind:function pcfg\n\
         settle 20000\n\
         frame\n\
         keys <c-u>kind:fnc\n\
         settle 20000\n\
         frame\n",
        "lib.rs",
    );
    let frames = frames(&output);
    assert_eq!(frames.len(), 2, "{output}");
    assert!(
        row(frames[0], "parse_config").contains(":2"),
        "{}",
        frames[0]
    );
    assert!(!frames[0].contains(" wrap "), "{}", frames[0]);
    assert!(!frames[0].contains("struct  Config"), "{}", frames[0]);
    assert!(
        frames[1].contains("^^^^^^^^ unknown kind: fnc"),
        "the error sits under the word: {}",
        frames[1]
    );
}

/// The frame's last row: the modeline.
fn modeline(frame: &str) -> &str {
    frame
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or_default()
}

#[test]
fn the_modeline_names_the_method_the_cursor_is_in() {
    // plans/0107 §10.6: no server, so the outline names the impl block
    // and its method; at top level the breadcrumb is blank.
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(
        root.join("lib.rs"),
        "struct Modeline;\n\nimpl Modeline {\n    fn fit(&self) {\n        let cells = 1;\n    }\n}\n",
    )
    .unwrap();
    let output = run(
        root,
        "settle 20000\n\
         keys 5G\n\
         settle 20000\n\
         frame\n\
         keys 2G\n\
         frame\n",
        "lib.rs",
    );
    let frames = frames(&output);
    assert_eq!(frames.len(), 2, "{output}");
    let inside = modeline(frames[0]);
    assert!(inside.contains("impl Modeline › fit"), "{inside}");
    let top = modeline(frames[1]);
    assert!(!top.contains("› ") && !top.contains("fit"), "{top}");
}

/// `python3` from the test's own `PATH`: the editor itself runs with none.
fn python() -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("python3"))
        .find(|path| path.is_file())
        .expect("python3 on PATH")
}

#[test]
fn a_symbol_reply_too_large_to_decode_fails_one_request_and_keeps_the_server() {
    // plans/0101 §4.2: the server is asked ahead of the key about a
    // 6,000-line file, answers with 40,000 symbols — over the decode budget
    // — and the connection lives: the press asks again and gets its list.
    let directory = tempfile::tempdir().unwrap();
    let root = &directory.path().join("project");
    std::fs::create_dir(root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::write(root.join("lib.rs"), "fn f() {}\n".repeat(6_000)).unwrap();
    let log = directory.path().join("server.log");
    let config = directory.path().join("config/strop");
    std::fs::create_dir_all(&config).unwrap();
    let server = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/symbols_server.py"
    );
    std::fs::write(
        config.join("languages.toml"),
        format!(
            "[language-server.rust-analyzer]\ncommand = {:?}\nargs = [{server:?}, {:?}]\n",
            python().display().to_string(),
            log.display().to_string()
        ),
    )
    .unwrap();
    let output = run(
        root,
        "settle 20000\n\
         keys <space>s\n\
         settle 20000\n\
         frame\n",
        "lib.rs",
    );
    let frames = frames(&output);
    assert_eq!(frames.len(), 1, "{output}");
    assert!(frames[0].contains("only_symbol"), "{}", frames[0]);
    assert!(!frames[0].contains("failed"), "{}", frames[0]);
    let asked = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        asked.lines().collect::<Vec<_>>(),
        ["initialize", "documentSymbol", "documentSymbol"],
        "one server, asked ahead and then by the press"
    );
}

fn check_symbol_jump(root: &Path, start: &str, keys: &str, name: &str, mixed: bool) {
    let output = run(
        root,
        &format!(
            "settle 20000\nkeys {keys}\nsettle 20000\nframe\n\
             keys {name}\nsettle 20000\nkeys <enter>\nsettle 20000\nstate\nframe\n"
        ),
        start,
    );
    let screens = frames(&output);
    if mixed {
        for row in ["SECOND_CONST", "THIRD_CONST"] {
            assert!(
                screens[0].contains(row),
                "missing {row} in the mixed picker:\n{output}"
            );
        }
    }
    let state = output
        .lines()
        .find_map(|line| line.strip_prefix("─── state "))
        .unwrap();
    let state: serde_json::Value = serde_json::from_str(state).unwrap();
    assert_eq!(state["picker"], false, "{output}");
    assert_eq!(state["line"], 1, "{output}");
    assert_eq!(state["cursor"], 28, "{output}"); // zero-based byte offset
    assert_eq!(state["col"], 29, "{output}"); // one-based byte column
    assert!(
        screens[1].contains(name),
        "opened another target:\n{output}"
    );
}

#[test]
fn symbol_jumps_use_each_producing_servers_encoding() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("notes.txt"), "no server here\n").unwrap();
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/symbol_jumps_server.py"
    );
    let mut config = String::new();
    let projects = [
        (
            "python",
            "pyproject.toml",
            "main.py",
            "pyright",
            "utf-16",
            "SECOND_CONST",
            "# éé",
        ),
        (
            "rust",
            "Cargo.toml",
            "lib.rs",
            "rust-analyzer",
            "utf-8",
            "THIRD_CONST",
            "//éé",
        ),
    ];
    for (project, marker, file, server, encoding, name, prefix) in projects {
        let project = root.join(project);
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join(marker), "").unwrap();
        let path = project.join(file);
        // Only the semantic server has this row; the grammar sees a comment.
        let padding = " ".repeat(28 - prefix.len());
        std::fs::write(&path, format!("{prefix}{padding}{name}\n")).unwrap();
        config.push_str(&format!(
            "[language-server.{server}]\ncommand = {:?}\nargs = [{fixture:?}, {encoding:?}, {path:?}, {name:?}]\n",
            python(),
        ));
    }
    let dir = directory.path().join("config/strop");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("languages.toml"), config).unwrap();
    for (project, _, file, _, _, name, _) in projects {
        // Cold document: the picker opens from a retained documentSymbol
        // reply. Warm workspace: both servers contribute to one picker,
        // and Enter has to open the previously unopened target file.
        check_symbol_jump(&root, &format!("{project}/{file}"), "<space>s", name, false);
        check_symbol_jump(&root, "notes.txt", "<space>S", name, true);
    }
}

#[test]
fn symbol_jumps_keep_syntax_outline_byte_columns() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let text = format!("/*éé*/{}const SECOND_CONST: i32 = 1;\n", " ".repeat(14));
    assert_eq!(text.find("SECOND_CONST"), Some(28));
    std::fs::write(root.join("lib.rs"), text).unwrap();
    std::fs::write(root.join("notes.txt"), "").unwrap();
    check_symbol_jump(&root, "lib.rs", "<space>s", "SECOND_CONST", false);
    check_symbol_jump(&root, "notes.txt", "<space>S", "SECOND_CONST", false);
}
