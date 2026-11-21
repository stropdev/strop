//! #243: observe the spawned server, not the path Strop says it selected.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const CPP: &str = "team-a/service";
const PY: &str = "team-b/service";
const WARM: &str = "keys <space>S\nsettle 20000\nframe\n";
const COLD: &str = "keys <space>s\nsettle 20000\nframe\n";
const TRUST: &str = "keys :trust<cr>\nsettle 20000\n";

struct Tree {
    tmp: tempfile::TempDir,
    work: PathBuf,
    python: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("workarea");
        for (project, marker) in [(CPP, "CMakeLists.txt"), (PY, "pyproject.toml")] {
            fs::create_dir_all(work.join(project)).unwrap();
            fs::write(work.join(project).join(marker), "").unwrap();
        }
        fs::write(work.join("notes.txt"), "notes\n").unwrap();
        fs::write(work.join(PY).join("main.py"), "answer = 42\n").unwrap();
        let python = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join("python3"))
            .find(|path| path.is_file())
            .expect("python3 for the fixture server");
        let tree = Self { tmp, work, python };
        let config = tree.tmp.path().join("config/strop");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("languages.toml"),
            tree.server("clangd", "cpp") + &tree.server("pyright", "python"),
        )
        .unwrap();
        tree
    }

    fn server(&self, name: &str, label: &str) -> String {
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/warm_root_server.py"
        );
        let log = self.tmp.path().join("initialize.log");
        format!(
            "[language-server.{name}]\ncommand = {:?}\nargs = [{script:?}, {log:?}, {label:?}]\n",
            self.python
        )
    }

    fn git_init(&self, relative: &str) {
        let result = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(self.work.join(relative))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    /// Each request starts in a fresh editor. An empty expected list also checks
    /// a refusal did not silently launch another server. No system servers on PATH.
    fn run(&self, file: &str, steps: &str, expected: &[(&str, &str)]) {
        let base = self.tmp.path();
        let log = base.join("initialize.log");
        fs::write(&log, "").unwrap();
        let script = base.join("steps");
        fs::write(&script, format!("settle 20000\n{steps}")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_strop"))
            .current_dir(&self.work)
            .env("HOME", base.join("home"))
            .env("XDG_CONFIG_HOME", base.join("config"))
            .env("XDG_STATE_HOME", base.join("state"))
            .env("PATH", base.join("empty-bin"))
            .env_remove("STROP_LOG")
            .arg("headless")
            .arg(script)
            .arg(file)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let frame = String::from_utf8(output.stdout).unwrap();
        let mut seen = Vec::new();
        for line in fs::read_to_string(log).unwrap().lines() {
            let [label, cwd, cwd_uri, initialize_uri]: [String; 4] =
                serde_json::from_str(line).unwrap();
            assert_eq!(
                initialize_uri, cwd_uri,
                "{label}: initialize must name its process cwd"
            );
            seen.push((label, PathBuf::from(cwd)));
        }
        seen.sort();
        let mut expected: Vec<_> = expected
            .iter()
            .map(|&(label, relative)| (label.to_owned(), self.work.join(relative)))
            .collect();
        expected.sort();
        assert_eq!(seen, expected, "{frame}");
    }
}

#[test]
fn warm_and_cold_discovery_agree_inside_and_outside_git() {
    let tree = Tree::new();
    let check = |root: &str| {
        tree.run("notes.txt", WARM, &[("cpp", CPP), ("python", root)]);
        tree.run("team-b/service/main.py", COLD, &[("python", root)]);
    };
    check(PY);
    tree.git_init("team-b");
    check("team-b");
}

#[test]
fn warm_project_layer_precedes_git_and_never_borrows_sibling_trust() {
    let tree = Tree::new();
    tree.git_init("");
    for (project, name, label) in [(CPP, "clangd", "cpp-local"), (PY, "pyright", "py-local")] {
        let config = tree.work.join(project).join(".strop");
        fs::create_dir(&config).unwrap();
        fs::write(config.join("languages.toml"), tree.server(name, label)).unwrap();
    }
    tree.run("team-a/service/CMakeLists.txt", TRUST, &[]);
    // Only the trusted C++ child may initialize. The server log names its
    // real cwd; the picker's abbreviated trust message is not that witness.
    tree.run("notes.txt", WARM, &[("cpp-local", CPP)]);
    tree.run("team-b/service/main.py", TRUST, &[("py-local", PY)]);
    tree.run("notes.txt", WARM, &[("cpp-local", CPP), ("py-local", PY)]);
}
