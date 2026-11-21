//! #268: declarations change legitimate attachment roots, never the universe of files.
//! Drive the shipped editor; only the LSP peer is controlled by the test.
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = ".strop/workspace.toml";
const CPP: &str = "team-a/service/src/main.cpp";
const PY: &str = "team-b/service/src/main.py";
const NOTE: &str = "café.txt";
const PLAIN: &str = "manual/read.opaque";
const NEEDLE: &str = "shared268";
const WARM: &str = "keys <space>S\nsettle\nframe\n";
const CLOSE: &str = "keys <esc><esc>\n";

struct Tree {
    tmp: tempfile::TempDir,
    work: PathBuf,
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

impl Tree {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("workarea");
        write(&work.join(NOTE), "🦀 café shared268\n");
        Self { tmp, work }
    }

    fn file(&self, path: &str, text: &str) {
        write(&self.work.join(path), text);
    }

    fn manifest(&self, projects: &[(&str, Option<&[&str]>)]) {
        let mut text = String::from("version = 1\n");
        for (root, languages) in projects {
            text.push_str(&format!("[[project]]\npath = {root:?}\n"));
            if let Some(ids) = languages {
                text.push_str(&format!("languages = {ids:?}\n"));
            }
        }
        // Atomic replacement, rather than hoping the watcher observes the last
        // write to a half-written file. The staging file isn't in the scope.
        let path = self.work.join(MANIFEST);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let stage = self.tmp.path().join("replacement");
        fs::write(&stage, text).unwrap();
        fs::rename(stage, path).unwrap();
    }

    fn server(&self, name: &str, label: &str, filename: &str) -> String {
        let python = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join("python3"))
            .find(|path| path.is_file())
            .expect("python3 for fixture server");
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/manifest_server.py"
        );
        let log = self.tmp.path().join("server.log");
        format!("[language-server.{name}]\ncommand = {python:?}\nargs = [{script:?}, {log:?}, {label:?}, {filename:?}]\n")
    }

    fn configure(&self, servers: &str) {
        write(
            &self.tmp.path().join("config/strop/languages.toml"),
            servers,
        );
    }

    fn run(&self, start: &str, actions: &str) -> String {
        let base = self.tmp.path();
        fs::write(base.join("server.log"), "").unwrap();
        let script = base.join("steps");
        fs::write(&script, format!("resize 180 46\nsettle\n{actions}")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_strop"))
            .current_dir(&self.work)
            .env("HOME", base)
            .env("XDG_CONFIG_HOME", base.join("config"))
            .env("XDG_STATE_HOME", base.join("state"))
            .env("PATH", base.join("no-programs"))
            .env_remove("STROP_LOG")
            .arg("headless")
            .arg(script)
            .arg(start)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{actions}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn events(&self) -> Vec<Value> {
        fs::read_to_string(self.tmp.path().join("server.log"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn roots(&self, expected: &[(&str, &str)]) {
        let mut seen = Vec::new();
        for e in self.events().iter().filter(|e| e["method"] == "initialize") {
            assert_eq!(e["uri"], e["cwd_uri"], "{e}");
            seen.push((
                e["label"].as_str().unwrap().to_owned(),
                PathBuf::from(e["cwd"].as_str().unwrap()),
            ));
        }
        let mut wanted: Vec<_> = expected
            .iter()
            .map(|&(s, p)| (s.to_owned(), self.work.join(p)))
            .collect();
        seen.sort();
        wanted.sort();
        assert_eq!(seen, wanted, "actual server process roots");
    }

    fn tui(&self) -> harness::Tui {
        let base = self.tmp.path();
        let mut tui = harness::Tui::spawn_with_env(
            &self.work,
            &base.join("unused.trace"),
            Path::new(env!("CARGO_BIN_EXE_strop")),
            false,
            base,
            base.join("no-programs").as_os_str(),
        );
        tui.until(|s| s.contains("NORMAL"));
        tui.send(format!(":e {NOTE}\r").as_bytes());
        tui.until(|s| s.contains(NEEDLE) && s.contains("NORMAL"));
        tui
    }
}

fn frames(output: &str) -> Vec<&str> {
    output
        .split("─── frame ")
        .skip(1)
        .map(|s| s.split("─── state ").next().unwrap())
        .collect()
}

fn states(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter_map(|s| s.strip_prefix("─── state "))
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

// Only picker result cells count: not a prompt, source preview or backdrop.
fn rows(frame: &str) -> impl Iterator<Item = &str> {
    frame.lines().filter_map(|line| {
        let mut cells = line.split('│');
        cells.next()?;
        let list = cells.next()?;
        cells.next()?;
        Some(list.trim().trim_start_matches('▌').trim())
    })
}

fn has_symbol(frame: &str, name: &str) -> bool {
    rows(frame).any(|r| r.split_whitespace().nth(1) == Some(name))
}

fn assert_opened(state: &Value, frame: &str, path: &str) {
    assert_eq!(state["picker"], false, "{frame}");
    assert!(
        frame
            .lines()
            .last()
            .unwrap()
            .split_whitespace()
            .any(|field| field == path),
        "wrong target:\n{frame}"
    );
}

fn assert_jump(state: &Value, frame: &str, path: &str, text: &str, token: &str) {
    let byte = text.find(token).unwrap();
    assert_opened(state, frame, path);
    assert_eq!(state["cursor"], byte, "{frame}");
    assert_eq!(
        state["col"],
        byte - text[..byte].rfind('\n').map_or(0, |i| i + 1) + 1,
        "{frame}"
    );
}

#[test]
fn declarations_keep_finder_text_and_byte_jumps_in_the_pinned_workspace() {
    let tree = Tree::new();
    for (path, text) in [
        ("team-a/service/CMakeLists.txt", "project(service)\n"),
        (CPP, "void render() {} // café 🦀 shared268\n"),
        ("team-b/service/pyproject.toml", "[project]\n"),
        (PY, "def render(): pass # café 🦀 shared268\n"),
        (PLAIN, "☃ shared268\n"),
        ("team-a/service/nested/go.mod", "module nested\n"),
        ("team-a/service/nested/.ignore", "ignored.txt\n"),
        ("team-a/service/nested/ignored.txt", NEEDLE),
    ] {
        tree.file(path, text);
    }
    fs::create_dir(tree.work.join("team-a/service/.git")).unwrap();
    write(&tree.tmp.path().join("outside.txt"), NEEDLE);
    let expected = [CPP, PY, NOTE, PLAIN];
    let visible_files = [
        CPP,
        PY,
        NOTE,
        PLAIN,
        "team-a/service/CMakeLists.txt",
        "team-b/service/pyproject.toml",
        "team-a/service/nested/go.mod",
    ];
    for declared in [false, true] {
        if declared {
            // Python and the loose note are deliberately unlisted; nested go.mod
            // remains discovery evidence. Text-only content is still searchable.
            tree.manifest(&[("team-a/service", Some(&["cpp"])), ("manual", Some(&[]))]);
        }
        let start = if declared { PY } else { CPP };
        // File rows may shorten or omit their directory. Count the full
        // eligible set, then actually open every path under the same policy.
        let mut finder = format!("keys <space>f\npaste \"hidden:exclude\"\nsettle\nstate\n{CLOSE}");
        for path in visible_files {
            finder.push_str(&format!(
                "keys <space>f\nkeys <c-u>\npaste \"hidden:exclude path:{path}\"\n\
                 settle\nkeys <cr>\nsettle\nstate\nframe\n"
            ));
        }
        let output = tree.run(start, &finder);
        let screens = frames(&output);
        let results = states(&output);
        assert_eq!(results[0]["picker_items"], visible_files.len(), "{output}");
        assert_eq!(screens.len(), visible_files.len(), "{output}");
        for (i, path) in visible_files.iter().enumerate() {
            assert_opened(&results[i + 1], screens[i], path);
        }

        let mut steps = format!("keys <space>/\npaste {NEEDLE:?}\nsettle\nframe\n");
        for path in expected {
            steps.push_str(&format!("keys <c-u>\npaste \"{NEEDLE} path:{path}\"\nsettle\nkeys <cr>\nsettle\nstate\nframe\nkeys <space>/\n"));
        }
        steps.push_str(&format!("{CLOSE}keys :e {CPP}<cr>\nsettle\nkeys ggccno match<esc>\nkeys <space>/\nkeys <c-u>\npaste {NEEDLE:?}\nsettle\nframe\n"));
        let output = tree.run(start, &steps);
        let screens = frames(&output);
        let jumps = states(&output);
        assert_eq!(screens.len(), expected.len() + 2, "{output}");
        assert_eq!(jumps.len(), expected.len(), "{output}");
        for (i, path) in expected.iter().enumerate() {
            let source = fs::read_to_string(tree.work.join(path)).unwrap();
            assert_jump(&jumps[i], screens[1 + i], path, &source, NEEDLE);
        }
        assert!(screens[0].contains("4 hits · 4 files"), "{}", screens[0]);
        let dirty = screens.last().unwrap();
        assert!(
            dirty.contains("3 hits · 3 files") && !rows(dirty).any(|r| r.contains("main.cpp")),
            "{dirty}"
        );
        for basename in ["main.py", NOTE, "read.opaque"] {
            assert!(rows(dirty).any(|r| r.contains(basename)), "{dirty}");
        }

        let output = tree.run(
            "team-a/service/src",
            &format!("keys <space>a\nsettle\nkeys <cr>\npaste {NEEDLE:?}\nsettle\nframe\n"),
        );
        let folder = frames(&output)[0];
        assert!(
            folder.contains("1 hit · 1 file") && rows(folder).any(|r| r.contains("main.cpp")),
            "{folder}"
        );
    }
}

#[test]
fn same_named_cxx_and_python_syntax_declarations_jump_to_the_selected_document() {
    let tree = Tree::new();
    tree.file(CPP, "/* café */ void render() {}\n");
    tree.file(PY, "# café\ndef render(): pass\n");
    tree.manifest(&[("team-a/service", Some(&[])), ("team-b/service", Some(&[]))]);
    for path in [CPP, PY] {
        let output = tree.run(NOTE, &format!("{WARM}paste \"path:{path} render\"\nsettle\nkeys <cr>\nsettle\nstate\nframe\nkeys <space>s\nsettle\nframe\n"));
        let screens = frames(&output);
        assert_eq!(
            rows(screens[0])
                .filter(|r| r.split_whitespace().nth(1) == Some("render"))
                .count(),
            2,
            "{}",
            screens[0]
        );
        assert_jump(
            &states(&output)[0],
            screens[1],
            path,
            &fs::read_to_string(tree.work.join(path)).unwrap(),
            "render",
        );
        assert!(has_symbol(screens[2], "render"), "{}", screens[2]);
        assert_eq!(
            rows(screens[2])
                .filter(|r| r.split_whitespace().nth(1) == Some("render"))
                .count(),
            1,
            "{}",
            screens[2]
        );
    }
}

#[test]
fn hints_choose_warm_roots_not_file_languages_trust_or_capabilities() {
    let tree = Tree::new();
    for (path, text) in [
        ("platform/agent/src/api.rs", "// café 🦀 AgentRust\n"),
        ("platform/agent/src/api.go", "// café 🦀 AgentGo\n"),
        ("tools/agent/src/api.rs", "// café 🦀 ToolsRust\n"),
        ("refused/src/api.rs", "// café 🦀 NeverRun\n"),
        ("silent/pyproject.toml", "[project]\n"),
        ("silent/src/api.py", "# café 🦀 NoCapability\n"),
        (
            "platform/agent/nested/go.mod",
            "module example.test/nested\n",
        ),
        ("platform/agent/nested/src/api.go", "// café 🦀 NestedGo\n"),
        ("loose.lua", "function syntax_only() end\n"),
        (PLAIN, "def invented(): pass\n"),
    ] {
        tree.file(path, text);
    }
    let rust = tree.server("rust-analyzer", "rust", "api.rs");
    tree.configure(
        &(rust
            + &tree.server("gopls", "go", "api.go")
            + &tree.server("pyright", "no-capability", "api.py")),
    );
    tree.file(
        "refused/.strop/languages.toml",
        &tree.server("rust-analyzer", "untrusted", "api.rs"),
    );
    let discovery = tree.run(NOTE, WARM);
    tree.roots(&[("go", "platform/agent/nested"), ("no-capability", "silent")]);
    let screen = frames(&discovery)[0];
    assert!(has_symbol(screen, "NestedGo"), "{screen}");
    assert!(
        !has_symbol(screen, "NoCapability") && !has_symbol(screen, "AgentRust"),
        "{screen}"
    );
    assert!(
        screen.contains("python server does not provide workspace symbols"),
        "{screen}"
    );
    // Fresh process: an already-warm server (or a restored buffer) cannot
    // mask a cold attach that mistakes this document's parent for the project.
    let cold = tree.run(
        "platform/agent/nested/src/api.go",
        "keys <space>s\nsettle\nframe\n",
    );
    tree.roots(&[("go", "platform/agent/nested")]);
    assert!(has_symbol(frames(&cold)[0], "NestedGo"), "{cold}");

    tree.manifest(&[
        ("platform/agent", Some(&["go", "rust", "lua", "unknown268"])),
        ("tools/agent", Some(&["rust"])),
        ("refused", Some(&["rust"])),
        ("silent", Some(&[])),
        ("manual", None),
    ]);
    // Grant one of two project-local Rust commands. Same language and even
    // the same binary do not authorize the command in the other child.
    tree.file(
        "tools/agent/.strop/languages.toml",
        &tree.server("rust-analyzer", "granted", "api.rs"),
    );
    tree.file("tools/agent/grant.txt", "");
    tree.run("tools/agent/grant.txt", "keys :trust<cr>\nsettle\n");
    let output = tree.run(NOTE, WARM);
    tree.roots(&[
        ("go", "platform/agent/nested"),
        ("go", "platform/agent"),
        ("rust", "platform/agent"),
        ("granted", "tools/agent"),
    ]);
    let screen = frames(&output)[0];
    for name in [
        "AgentRust",
        "AgentGo",
        "ToolsRust",
        "NestedGo",
        "syntax_only",
    ] {
        assert!(has_symbol(screen, name), "missing {name}:\n{screen}");
    }
    for name in ["NeverRun", "NoCapability", "invented"] {
        assert!(!has_symbol(screen, name), "invented {name}:\n{screen}");
    }
    for badge in ["no srv", "no exec", "trust"] {
        assert!(
            rows(screen).any(|r| r.starts_with(badge)),
            "missing {badge} status:\n{screen}"
        );
    }
    assert!(rows(screen).any(|r| r.contains("unknown268")), "{screen}");
    // Cold opens detect the actual document regardless of the warm list ([]
    // included). The server receives the actual languageId, not the extension.
    for (root, file, label, language, name) in [
        ("platform/agent", "api.rs", "rust", "rust", "AgentRust"),
        ("platform/agent", "api.go", "go", "go", "AgentGo"),
        ("platform/agent/nested", "api.go", "go", "go", "NestedGo"),
        ("tools/agent", "api.rs", "granted", "rust", "ToolsRust"),
        (
            "silent",
            "api.py",
            "no-capability",
            "python",
            "NoCapability",
        ),
    ] {
        let path = format!("{root}/src/{file}");
        let output = tree.run(
            &path,
            "keys <space>s\nsettle\nframe\nkeys <cr>\nsettle\nstate\nframe\n",
        );
        tree.roots(&[(label, root)]);
        let events = tree.events();
        assert!(
            events.iter().any(|e| e["method"] == "textDocument/didOpen"
                && e["language"] == language
                && e["uri"].as_str().is_some_and(|uri| uri.ends_with(&path))),
            "{events:?}"
        );
        let screens = frames(&output);
        assert!(has_symbol(screens[0], name), "{}", screens[0]);
        assert_jump(
            &states(&output)[0],
            screens[1],
            &path,
            &fs::read_to_string(tree.work.join(&path)).unwrap(),
            name,
        );
    }
}

#[test]
fn invalid_or_unbounded_manifests_apply_no_valid_prefix() {
    let tree = Tree::new();
    tree.configure(
        &(tree.server("gopls", "declared", "api.go")
            + &tree.server("pyright", "discovered", "api.py")),
    );
    tree.file("member/src/api.go", "// DeclaredOnly\n");
    tree.file("ordinary/pyproject.toml", "");
    tree.file("ordinary/src/api.py", "# OrdinaryOnly\n");
    let prefix = "version=1\n[[project]]\npath='member'\nlanguages=['go']\n";
    for input in [
        format!("{prefix}[[project]]\npath='member'\n"),
        format!("{prefix}[[project]]\npath='../outside'\n"),
        format!("{prefix}command='not a manifest option'\n"),
        format!("{prefix}#{}", "x".repeat(64 * 1024)),
        (0..256).fold(prefix.to_string(), |mut text, i| {
            text.push_str(&format!("[[project]]\npath='extra-{i}'\n"));
            text
        }),
    ] {
        tree.file(MANIFEST, &input);
        let output = tree.run(NOTE, "state\n");
        assert!(
            states(&output)[0]["message"]
                .as_str()
                .unwrap()
                .contains("workspace.toml"),
            "missing diagnostic: {output}"
        );
        // Ordinary discovery is asynchronous. Observe the rendered fallback
        // result and accept it, not a transient frame between reply and ranking.
        let mut tui = tree.tui();
        tui.send(b" S");
        let screen = tui.until(|s| {
            s.lines()
                .next()
                .is_some_and(|header| header.trim_end().ends_with(" 1 ╮"))
                && has_symbol(s, "OrdinaryOnly")
        });
        assert!(!has_symbol(&screen, "DeclaredOnly"), "{screen}");
        tui.send(b"\r");
        tui.until(|s| {
            s.lines()
                .last()
                .is_some_and(|m| m.contains("NORMAL") && m.contains("ordinary/src/api.py"))
        });
        tui.send(b":qa!\r");
        tui.expect_clean_exit("ordinary discovery after invalid manifest");
        tree.roots(&[("discovered", "ordinary")]);
    }
}

#[test]
fn only_a_regular_manifest_at_the_pinned_root_can_add_authority() {
    let tree = Tree::new();
    tree.configure(&tree.server("gopls", "go", "api.go"));
    tree.file("nested/member/src/api.go", "// OutsideConfigOnly\n");
    let config =
        |member: &str| format!("version=1\n[[project]]\npath={member:?}\nlanguages=['go']\n");
    tree.file("nested/.strop/workspace.toml", &config("member"));
    write(
        &tree.tmp.path().join(MANIFEST),
        &config("workarea/nested/member"),
    );
    tree.run(NOTE, WARM);
    tree.roots(&[]);

    let external = tree.tmp.path().join("external.toml");
    write(&external, &config("nested/member"));
    fs::create_dir(tree.work.join(".strop")).unwrap();
    std::os::unix::fs::symlink(external, tree.work.join(MANIFEST)).unwrap();
    let output = tree.run(NOTE, &format!("state\n{WARM}"));
    tree.roots(&[]);
    assert!(
        states(&output)[0]["message"]
            .as_str()
            .unwrap()
            .contains("workspace.toml"),
        "{output}"
    );
    assert!(
        !has_symbol(frames(&output)[0], "OutsideConfigOnly"),
        "{output}"
    );
}

#[path = "terminal_editor/harness.rs"]
#[allow(dead_code)]
mod harness;

#[cfg(target_os = "linux")]
mod reload {
    use super::*;
    use harness::Tui;
    use std::ffi::CString;
    use std::io::{ErrorKind, Read, Write};
    use std::os::unix::{
        fs::symlink,
        net::{UnixListener, UnixStream},
    };
    use std::time::{Duration, Instant};

    #[derive(Copy, Clone, Debug, PartialEq)]
    enum Change {
        Replace,
        Malformed,
        Remove,
        Retarget,
    }

    // The server connects only after computing this request's immutable reply.
    // The PTY must keep draining while waiting for a process on another fd.
    fn take(tui: &mut Tui, listener: &UnixListener, name: &str, change: Change) -> UnixStream {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match listener.accept() {
                Ok((gate, _)) => {
                    gate.set_read_timeout(Some(Duration::from_secs(30)))
                        .unwrap();
                    return gate;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => (),
                Err(e) => panic!("server rendezvous: {e}"),
            }
            assert!(
                Instant::now() < deadline,
                "{change:?}: no {name} request:\n{}",
                tui.screen.screen().contents()
            );
            tui.until_soft(Duration::from_millis(10), |_| false);
        }
    }

    // Writing the actual LSP response can instead find a transport already
    // closed by retirement. An elapsed timeout is not evidence of revocation.
    fn release(mut peer: UnixStream) -> bool {
        let mut exchange = || -> std::io::Result<()> {
            peer.write_all(b"!")?;
            let mut ack = [0];
            peer.read_exact(&mut ack)?;
            assert_eq!(&ack, b"!");
            Ok(())
        };
        match exchange() {
            Ok(()) => true,
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
                ) =>
            {
                false
            }
            Err(e) => panic!("server gate: {e}"),
        }
    }

    fn in_flight(change: Change) {
        let tree = Tree::new();
        let base = tree.tmp.path();
        tree.configure(
            &(tree.server("gopls", "old-go", "api.go")
                + &tree.server("pyright", "live-py", "api.py")),
        );
        tree.file("old/src/api.go", "// café 🦀 EpochOld\n");
        let fresh = "# café 🦀 EpochLive";
        let outside = base.join("outside");
        write(&outside.join("src/api.go"), "// café 🦀 EpochEscape\n");
        write(&outside.join("outside.txt"), "OnlyOutside268\n");
        match change {
            Change::Retarget => {
                tree.file("old/src/api.py", fresh);
                // This symlink cannot authorize an attach yet. Exchange it for
                // the real child in one operation below; the manifest is untouched.
                symlink(&outside, tree.work.join("live")).unwrap();
                tree.manifest(&[("old", Some(&["go"])), ("live", Some(&["python"]))]);
            }
            _ => {
                tree.file("live/src/api.py", fresh);
                if change != Change::Replace {
                    tree.file("live/pyproject.toml", "");
                }
                tree.manifest(&[("old", Some(&["go"])), ("live", Some(&[]))]);
            }
        }
        let old_requests = UnixListener::bind(base.join("EpochOld")).unwrap();
        let live_requests = UnixListener::bind(base.join("EpochLive")).unwrap();
        let mut tui = tree.tui();
        tui.send(b" S");
        tui.until(|s| s.contains(" workspace symbols "));
        tui.send(b"\x1b[200~Epoch\x1b[201~");
        let old = take(&mut tui, &old_requests, "old declared", change);
        // Leave exactly the same picker and query open for the whole edit.
        match change {
            Change::Replace => tree.manifest(&[("live", Some(&["python"]))]),
            Change::Malformed => {
                let stage = base.join("malformed");
                fs::write(&stage, "version=1\n[[project\n").unwrap();
                fs::rename(stage, tree.work.join(MANIFEST)).unwrap();
            }
            Change::Remove => fs::remove_file(tree.work.join(MANIFEST)).unwrap(),
            Change::Retarget => {
                let old =
                    CString::new(tree.work.join("old").as_os_str().as_encoded_bytes()).unwrap();
                let live =
                    CString::new(tree.work.join("live").as_os_str().as_encoded_bytes()).unwrap();
                // SAFETY: two live, NUL-terminated absolute names. Exchange is
                // atomic: every watcher observation sees one tree or the other.
                let result = unsafe {
                    libc::renameat2(
                        libc::AT_FDCWD,
                        old.as_ptr(),
                        libc::AT_FDCWD,
                        live.as_ptr(),
                        libc::RENAME_EXCHANGE,
                    )
                };
                assert_eq!(
                    result,
                    0,
                    "atomic retarget: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
        // This producer wasn't warmable under the old manifest. Its request
        // is the editor's acknowledgement of replacement/fallback, not merely
        // a filesystem observation or a request from before the change.
        let live = take(&mut tui, &live_requests, "rebound", change);
        let _delivered_before_transport_closed = release(old);
        assert!(release(live), "{change:?}: successor transport retired");
        let screen = tui.until(|s| has_symbol(s, "EpochLive"));
        for stale in ["EpochOld", "EpochEscape"] {
            assert!(!has_symbol(&screen, stale), "{change:?}: {screen}");
        }
        tui.send(b"\r");
        let position = format!("1:{}", fresh.find("EpochLive").unwrap() + 1);
        tui.until(|s| {
            s.contains(fresh)
                && s.lines().last().is_some_and(|m| {
                    m.contains("live/src/api.py") && m.split_whitespace().any(|f| f == position)
                })
        });
        if change == Change::Retarget {
            tui.send(b" /\x1b[200~OnlyOutside268\x1b[201~");
            tui.until(|s| {
                s.lines()
                    .next()
                    .is_some_and(|header| header.trim_end().ends_with("0 hits · 0 files ╮"))
                    && s.contains("Find OnlyOutside268▏")
                    && rows(s).any(|r| r.starts_with("No matches"))
            });
            tui.send(b"\x1b\x1b");
            tui.until(|s| s.contains("NORMAL") && !s.contains(" SEARCH "));
        }
        tui.send(b":qa!\r");
        tui.expect_clean_exit("manifest replacement editor");
        // An identically named target or real process started outside the
        // workspace can't hide behind the picker's shortened display label.
        for e in tree.events().iter().filter(|e| e["method"] == "initialize") {
            assert_eq!(e["uri"], e["cwd_uri"], "{e}");
            let root = Path::new(e["cwd"].as_str().unwrap());
            assert!(
                (root == tree.work.join("old") && e["label"] == "old-go")
                    || (root == tree.work.join("live") && e["label"] == "live-py"),
                "{change:?}: unexpected process {e}"
            );
        }
    }

    #[test]
    fn replacement_retires_in_flight_association() {
        in_flight(Change::Replace);
    }

    #[test]
    fn malformed_replacement_retires_in_flight_association() {
        in_flight(Change::Malformed);
    }

    #[test]
    fn removal_retires_in_flight_association() {
        in_flight(Change::Remove);
    }

    #[test]
    fn retargeted_member_cannot_publish_or_jump_outside() {
        in_flight(Change::Retarget);
    }
}
