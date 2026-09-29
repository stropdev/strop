#![cfg(windows)]

//! Real Win32 argv + WSL pipes, not a simulated transport. The native hardware
//! lane supplies an explicit distro/user and an already-verified Linux artifact.
//! All editor state and source files stay in the owned Linux temporary directory.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Duration;
use strop_core::frontend_input::Key;
use strop_ui_protocol::{AdmittedAction, Driver, ViewSnapshot};

struct Fixture {
    wsl: PathBuf,
    distro: String,
    user: String,
    backend: String,
    root: String,
}

impl Fixture {
    fn new() -> Self {
        let required = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"));
        let mut fixture = Self {
            wsl: PathBuf::from(required("STROP_WSL_EXE")),
            distro: required("STROP_WSL_DISTRO"),
            user: required("STROP_WSL_USER"),
            backend: required("STROP_WSL_BACKEND"),
            root: String::new(),
        };
        assert!(fixture.wsl.is_absolute());
        assert!(fixture.backend.starts_with('/'));
        let output = fixture.linux(&["/usr/bin/mktemp", "-d", "/tmp/strop-wsl-protocol.XXXXXXXX"]);
        let root = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        let suffix = root.strip_prefix("/tmp/strop-wsl-protocol.").unwrap();
        assert_eq!(suffix.len(), 8);
        assert!(suffix.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        // Arm cleanup only after the returned path proves our owned scope.
        fixture.root = root;
        fixture.linux(&[
            "/bin/sh",
            "-c",
            "mkdir -p -- \"$1/config\" \"$1/cache\" \"$1/state\" && printf 'before\\n' > \"$1/source 'quoted' λ.txt\"",
            "strop-owned-fixture",
            &fixture.root,
        ]);
        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.wsl);
        command.args(["--distribution", &self.distro, "--user", &self.user]);
        command
    }

    fn linux(&self, args: &[&str]) -> Output {
        let output = self.command().arg("--exec").args(args).output().unwrap();
        assert!(
            output.status.success(),
            "Linux fixture operation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn driver(&self) -> Driver {
        let home = format!("HOME={}", self.root);
        let config = format!("XDG_CONFIG_HOME={}/config", self.root);
        let cache = format!("XDG_CACHE_HOME={}/cache", self.root);
        let state = format!("XDG_STATE_HOME={}/state", self.root);
        let mut driver = Driver::spawn_args(
            &self.wsl,
            &[
                "--distribution",
                &self.distro,
                "--user",
                &self.user,
                "--cd",
                &self.root,
                "--exec",
                "/usr/bin/env",
                &home,
                &config,
                &cache,
                &state,
                &self.backend,
                "--ui-stdio",
            ],
            &std::env::temp_dir(),
            &[],
        )
        .unwrap();
        driver.set_budget(Duration::from_secs(30));
        driver
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.root.is_empty() {
            let output = self
                .command()
                .args(["--exec", "/bin/rm", "-rf", "--", &self.root])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "owned Linux fixture cleanup failed"
            );
        }
    }
}

fn first_line(view: &ViewSnapshot) -> Option<&str> {
    view.panes
        .get(view.active_pane)?
        .lines
        .first()
        .map(String::as_str)
}

#[test]
#[ignore = "requires explicit native Windows + WSL hardware-lane fixture"]
fn native_windows_wsl_unicode_edit_undo_save_and_shutdown() {
    let fixture = Fixture::new();
    let mut driver = fixture.driver();
    driver.act_keys(":e source 'quoted' λ.txt<cr>").unwrap();
    driver
        .wait_view("the Linux source open", |view| {
            first_line(view) == Some("before")
        })
        .unwrap();
    driver.act_keys("gg0i").unwrap();
    driver.act_text("λ🦀e\u{301} ").unwrap();
    driver
        .act(vec![AdmittedAction::EditorKey(Key::Esc)])
        .unwrap();
    driver
        .wait_view("the Unicode edit", |view| {
            first_line(view) == Some("λ🦀e\u{301} before")
        })
        .unwrap();
    driver.act_keys("u").unwrap();
    assert_eq!(first_line(driver.client().view().unwrap()), Some("before"));
    driver.act_keys("<C-r>:w<cr>").unwrap();
    driver
        .wait_view("the source save receipt", |view| {
            first_line(view) == Some("λ🦀e\u{301} before") && view.state["dirty"] == false
        })
        .unwrap();
    let saved = fixture.linux(&[
        "/bin/cat",
        "--",
        &format!("{}/source 'quoted' λ.txt", fixture.root),
    ]);
    assert_eq!(saved.stdout, "λ🦀e\u{301} before\n".as_bytes());
    assert!(driver.shutdown().unwrap().success());
}
