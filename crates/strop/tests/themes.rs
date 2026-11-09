//! Theme selection end to end: config, `--theme`, `:theme` and replay
//! through real child editors, asserting on the actual rendered cells.
//! The theme is session state on the editor (0088 §3), so these are
//! journeys, not isolation workarounds.
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
            } else if id == ThemeId::Vellum {
                ThemeId::Hone
            } else {
                ThemeId::Vellum
            };
            configure(dir.path(), configured.name());
            let (_, frames) = capture(dir.path(), SOURCE, flag);
            let frame = frames.last().expect("actual frame observation");
            assert_eq!(
                cell(frame, 40, 5)[3],
                color(id.palette().bg.base),
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
                color(id.palette().fg.text),
                "{} source text",
                id.name()
            );
        }
    }
}

#[test]
fn live_selector_accept_cancel_and_invalid_choice_preserve_session_ownership() {
    let dir = tempfile::tempdir().unwrap();
    configure(dir.path(), "slate");
    let script = format!(
        "{SOURCE}keys :theme<cr>\nsettle 5000\nkeys vellum\nsettle 5000\nkeys <esc><esc>\nframe\n\
         keys :theme<cr>\nsettle 5000\nkeys carbide\nsettle 5000\nkeys <cr>\nframe\n\
         keys :theme nonexistent-palette<cr>\nframe\nstate\n"
    );
    let (output, frames) = capture(dir.path(), &script, Some("paper-dark"));
    assert!(output.contains("nonexistent-palette"));
    let final_frame = frames.last().unwrap();
    assert_eq!(
        cell(final_frame, 40, 5)[3],
        color(ThemeId::Carbide.palette().bg.base)
    );
    // Opening/filtering/cancelling is not an uncommitted live preview.
    assert!(!frames
        .iter()
        .any(|frame| cell(frame, 40, 5)[3] == color(ThemeId::Vellum.palette().bg.base)));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config/strop/config.toml")).unwrap(),
        "theme = \"slate\"\n"
    );

    // Replay must restore the effective CLI theme, not re-read config or
    // default to hone; it also reproduces every subsequent theme switch.
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
fn paper_variants_switch_through_picker_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    configure(dir.path(), "slate");
    let script = format!(
        "{SOURCE}keys :theme<cr>\nsettle 5000\nkeys paper\nsettle 5000\nkeys <cr>\nframe\n\
         keys :theme<cr>\nsettle 5000\nkeys paper-dark\nsettle 5000\nkeys <cr>\nframe\n\
         keys :theme hone<cr>\nframe\n\
         keys :theme<cr>\nsettle 5000\nkeys paper-dark\nsettle 5000\nkeys <cr>\nframe\nstate\n"
    );
    let (_, frames) = capture(dir.path(), &script, Some("hone"));
    let expected = [
        ThemeId::Hone,
        ThemeId::Paper,
        ThemeId::PaperDark,
        ThemeId::Hone,
        ThemeId::PaperDark,
    ];
    let mut observed: Vec<_> = frames
        .iter()
        .filter_map(|frame| {
            expected
                .iter()
                .copied()
                .find(|id| cell(frame, 40, 5)[3] == color(id.palette().bg.base))
        })
        .collect();
    observed.dedup();
    assert_eq!(observed, expected);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config/strop/config.toml")).unwrap(),
        "theme = \"slate\"\n"
    );

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
    assert_eq!(state["message"], "theme: paper-dark palette");
}

#[test]
fn human_labels_and_canonical_names_choose_the_typed_registry_entry() {
    for (query, expected) in [
        ("Paper Dark", ThemeId::PaperDark),
        ("carbide", ThemeId::Carbide),
        ("vellum", ThemeId::Vellum),
        ("Slate", ThemeId::Slate),
    ] {
        let dir = tempfile::tempdir().unwrap();
        configure(dir.path(), "paper");
        let script =
            format!("{SOURCE}keys :theme<cr>\nsettle 5000\nkeys {query}<cr>\nsettle 5000\nframe\n");
        let (_, frames) = capture(dir.path(), &script, Some("hone"));
        assert_eq!(
            cell(frames.last().unwrap(), 40, 5)[3],
            color(expected.palette().bg.base),
            "query {query:?}"
        );
    }
}

/// A deleted or misspelt theme: `--theme` refuses before editing; a
/// config naming one falls back to the default with a warning that
/// lists the six choices (0088 §3) and the editor stays usable.
#[test]
fn invalid_launch_theme_refuses_before_editing_and_invalid_config_stays_usable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.txt"), "keep\n").unwrap();
    std::fs::write(dir.path().join("steps.keys"), "keys dd:w<cr>\n").unwrap();
    let output = command(dir.path())
        .args([
            "--theme",
            "catppuccin-mocha",
            "--headless",
            "steps.keys",
            "source.txt",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hone, slate, carbide, vellum, paper, paper-dark"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("source.txt")).unwrap(),
        "keep\n"
    );
    configure(dir.path(), "nord");
    let (output, frames) = capture(dir.path(), &format!("{SOURCE}state\n"), None);
    assert_eq!(
        cell(frames.last().unwrap(), 40, 5)[3],
        color(ThemeId::Hone.palette().bg.base)
    );
    assert!(
        output.contains("nord"),
        "the warning names the rejected theme: {output}"
    );
    assert!(
        output.contains("hone"),
        "the warning names the choices: {output}"
    );
}
