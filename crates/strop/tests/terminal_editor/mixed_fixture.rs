use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

pub(crate) struct Fixture {
    tmp: tempfile::TempDir,
    pub work: PathBuf,
    pub large: &'static str,
    pub rare: &'static str,
    pub common: &'static str,
}

impl Fixture {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("workarea");
        let large = "platform/agent/src/lib.rs";
        let rare = "uniqueneedle076";
        let common = "sharedneedle076";
        for dir in [
            "team-a/service/src",
            "team-b/service/src",
            "team-a/service/nested",
            "platform/agent/src",
            "tools/reporter",
            "tools/second",
            "docs",
        ] {
            fs::create_dir_all(work.join(dir)).unwrap();
        }
        for (name, contents) in [
            ("team-a/service/CMakeLists.txt", "project(agent)\n"),
            (
                "team-a/service/src/main.cpp",
                "void render() {} // sharedneedle076\n",
            ),
            (
                "team-b/service/pyproject.toml",
                "[project]\nname = 'service'\n",
            ),
            (
                "team-b/service/src/main.py",
                "def render(): pass # sharedneedle076\n",
            ),
            (
                "team-a/service/nested/go.mod",
                "module example.invalid/nested\n",
            ),
            ("team-a/service/nested/.ignore", "ignored.txt\n"),
            ("team-a/service/nested/ignored.txt", "sharedneedle076\n"),
            (
                "platform/agent/main.go",
                "package main\nfunc AgentMain() {}\n",
            ),
            ("tools/reporter/lib.rs", "pub fn reporter_symbol() {}\n"),
            (
                "tools/second/main.go",
                "package second\nfunc SecondSymbol() {}\n",
            ),
            ("notes.txt", "sharedneedle076\n"),
            ("docs/plain.unknown", "text without a semantic provider\n"),
        ] {
            fs::write(work.join(name), contents).unwrap();
        }
        fs::write(tmp.path().join("outside.txt"), format!("{rare} {common}\n")).unwrap();
        let mut source = String::from("pub fn measured_symbol() {}\n");
        for i in 0..2800 {
            writeln!(source, "pub fn generated_{i:04}() -> usize {{ {i} }}").unwrap();
        }
        fs::write(work.join(large), source).unwrap();
        let mut ordinary = "ordinary project fixture line\n".repeat(63);
        ordinary.push_str("ordinary project fixture line widespread076\n");
        for dir in 0..64 {
            let base = work.join(format!("bulk/d{dir:02}"));
            fs::create_dir_all(&base).unwrap();
            for file in 0..64 {
                let path = base.join(format!("p{file:02}.txt"));
                if (dir, file) == (37, 19) {
                    fs::write(path, format!("{ordinary}{rare}\n")).unwrap();
                } else {
                    fs::write(path, &ordinary).unwrap();
                }
            }
        }
        Self {
            tmp,
            work,
            large,
            rare,
            common,
        }
    }
    pub fn spawn(&self, binary: &std::path::Path) -> super::Tui {
        let home = self.tmp.path();
        super::Tui::spawn_with_env(
            &self.work,
            &home.join("unused.trace"),
            binary,
            false,
            home,
            home.join("no-programs").as_os_str(),
        )
    }

    pub fn config(&self) -> PathBuf {
        let config = self.tmp.path().join("config/strop");
        fs::create_dir_all(&config).unwrap();
        config
    }
}

pub(crate) struct Usage {
    rchar: u64,
    read_bytes: u64,
    peak_kib: u64,
}

impl Usage {
    pub fn read(tui: &super::Tui) -> Self {
        let proc = PathBuf::from("/proc").join(tui.child.id().to_string());
        let io = fs::read_to_string(proc.join("io")).unwrap();
        let status = fs::read_to_string(proc.join("status")).unwrap();
        fn field(text: &str, name: &str) -> u64 {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap()
        }
        Self {
            rchar: field(&io, "rchar:"),
            read_bytes: field(&io, "read_bytes:"),
            peak_kib: field(&status, "VmHWM:"),
        }
    }

    pub fn report_since(&self, before: &Self) -> serde_json::Value {
        serde_json::json!({"process_read_chars": self.rchar - before.rchar, "storage_read_bytes": self.read_bytes - before.read_bytes, "process_peak_rss_kib": self.peak_kib})
    }
}
