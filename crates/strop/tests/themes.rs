//! Theme selection runs in real child editors: process-global palette state
//! cannot race another test, and assertions inspect actual rendered cells.
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use strop_core::theme::{Rgb, ThemeId};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_strop"));
    command
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env_remove("STROP_LOG");
    command
}

fn successful(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn configure(root: &Path, name: &str) {
    std::fs::create_dir_all(root.join("config/strop")).unwrap();
    std::fs::write(
        root.join("config/strop/config.toml"),
        format!("theme = {name:?}\n"),
    )
    .unwrap();
}

fn capture(root: &Path, script: &str, flag: Option<&str>) -> (String, Vec<Value>) {
    std::fs::write(root.join("steps.keys"), script).unwrap();
    let mut command = command(root);
    command.args([
        "--headless",
        "steps.keys",
        "--log-file",
        "trace.jsonl",
        "--log-content",
    ]);
    if let Some(theme) = flag {
        command.args(["--theme", theme]);
    }
    let output = successful(command.output().unwrap());
    let frames = std::fs::read_to_string(root.join("trace.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["event"] == "replay" && record["fields"]["kind"] == "check")
        .filter_map(|record| {
            record["fields"]["value"]["cells"]
                .as_array()
                .map(|_| record["fields"]["value"].clone())
        })
        .collect::<Vec<_>>();
    (output, frames)
}

fn color(value: Rgb) -> u64 {
    (1 << 24) | (u64::from(value.r) << 16) | (u64::from(value.g) << 8) | u64::from(value.b)
}

fn cell(frame: &Value, column: usize, row: usize) -> &Value {
    let mut x = 0;
    frame["cells"][row]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| {
            let end = x + run[0].as_u64().unwrap() as usize;
            let contains = (x..end).contains(&column);
            x = end;
            contains
        })
        .unwrap()
}

const SOURCE: &str = "buffer \"theme sample\\n\"\nresize 90 24\nsettle 5000\nframe\n";

#[test]
fn every_registered_palette_renders_from_config_and_cli_override() {
    for id in ThemeId::ALL {
        for flag in [None, Some(id.name())] {
            let dir = tempfile::tempdir().unwrap();
            // With a flag, deliberately disagree with it in the config.
            let configured = if flag.is_none() {
                id
            } else if id == ThemeId::Light {
                ThemeId::Dark
            } else {
                ThemeId::Light
            };
            configure(dir.path(), configured.name());
            let (_, frames) = capture(dir.path(), SOURCE, flag);
            let frame = frames.last().expect("actual frame observation");
            assert_eq!(
                cell(frame, 40, 5)[3],
                color(id.palette().base),
                "{} background",
                id.name()
            );
            let source_row = frame["cells"][0].as_array().unwrap();
            let sample = source_row
                .iter()
                .find(|run| run[1] == "t")
                .expect("source text cell");
            assert_eq!(
                sample[2],
                color(id.palette().text),
                "{} source text",
                id.name()
            );
        }
    }
}

#[test]
fn live_selector_accept_cancel_and_invalid_choice_preserve_session_ownership() {
    let dir = tempfile::tempdir().unwrap();
    configure(dir.path(), "nord");
    let script = format!(
        "{SOURCE}keys :theme<cr>\nsettle 5000\nkeys light\nsettle 5000\nkeys <esc><esc>\nframe\n\
         keys :theme<cr>\nsettle 5000\nkeys strop-ink\nsettle 5000\nkeys <cr>\nframe\n\
         keys :theme nonexistent-palette<cr>\nframe\nstate\n"
    );
    let (output, frames) = capture(dir.path(), &script, Some("catppuccin-mocha"));
    assert!(output.contains("nonexistent-palette"));
    let final_frame = frames.last().unwrap();
    assert_eq!(
        cell(final_frame, 40, 5)[3],
        color(ThemeId::StropInk.palette().base)
    );
    // Opening/filtering/cancelling is not an uncommitted live preview.
    assert!(!frames
        .iter()
        .any(|frame| cell(frame, 40, 5)[3] == color(ThemeId::Light.palette().base)));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config/strop/config.toml")).unwrap(),
        "theme = \"nord\"\n"
    );

    // Replay must restore the effective CLI theme, not re-read config or
    // default to dark; it also reproduces every subsequent theme switch.
    let replay = successful(
        command(dir.path())
            .env("PATH", dir.path().join("no-native-executables"))
            .env("XDG_CONFIG_HOME", dir.path().join("no-config"))
            .args(["--replay", "trace.jsonl"])
            .output()
            .unwrap(),
    );
    let state: Value = serde_json::from_str(&replay).unwrap();
    assert_eq!(state["mode"], "NORMAL");
}

#[test]
fn invalid_launch_theme_refuses_before_editing_and_invalid_config_stays_usable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.txt"), "keep\n").unwrap();
    std::fs::write(dir.path().join("steps.keys"), "keys dd:w<cr>\n").unwrap();
    let output = command(dir.path())
        .args([
            "--theme",
            "nonexistent-palette",
            "--headless",
            "steps.keys",
            "source.txt",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("source.txt")).unwrap(),
        "keep\n"
    );
    configure(dir.path(), "nonexistent-palette");
    let (_, frames) = capture(dir.path(), SOURCE, None);
    assert_eq!(
        cell(frames.last().unwrap(), 40, 5)[3],
        color(ThemeId::Dark.palette().base)
    );
}
