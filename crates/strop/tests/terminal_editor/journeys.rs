use super::*;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;
use std::time::Duration;

/// TERMINAL mode: its own chip with the one escape chord as the hint.
fn terminal_input(screen: &str) -> bool {
    screen.contains("TERMINAL") && screen.contains("ctrl-\\ ctrl-n  NORMAL")
}

/// The shell prompt on a grid row. The buffer name in the modeline quotes
/// the launch command (`term://…:env PS1='STROP-PTY> ' …`), so a bare
/// substring search would match the modeline, not the child.
fn at_prompt(screen: &str) -> bool {
    screen.lines().any(|row| row.starts_with("STROP-PTY>"))
}

/// NORMAL on a terminal buffer: a plain `NORMAL` chip over a `term://`
/// name — `i` re-enters TERMINAL mode.
fn terminal_inspection(screen: &str) -> bool {
    screen.contains(" NORMAL ") && screen.contains("term://") && !screen.contains("TERMINAL")
}

#[test]
fn kitty_shifted_punctuation_opens_ex_commands() {
    let directory = tempfile::tempdir().unwrap();
    let trace = directory.path().join("terminal.jsonl");
    let mut tui = Tui::start(directory.path(), &trace);
    tui.until(|screen| screen.contains("NORMAL"));
    assert!(
        tui.output
            .windows(b"\x1b[>15u".len())
            .any(|bytes| bytes == b"\x1b[>15u"),
        "terminal must request shifted alternate keys"
    );
    tui.send(b"\x1b[59:58;2u");
    tui.until(|screen| screen.contains("COMMAND"));
    tui.send(b"q\r");
    tui.expect_clean_exit("the editor did not exit cleanly");
}

/// The legacy C0 byte of `Ctrl-\` (0x1c) followed by `Ctrl-N` is the
/// one escape chord; F2 is just a key the child receives.
#[test]
fn real_ctrl_backslash_ctrl_n_enters_normal_and_f2_stays_with_the_child() {
    let directory = tempfile::tempdir().unwrap();
    let trace = directory.path().join("terminal.jsonl");
    let mut tui = Tui::start(directory.path(), &trace);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":terminal env PS1='STROP-PTY> ' /bin/sh -i\r");
    tui.until(|screen| terminal_input(screen) && at_prompt(screen));
    tui.send(b"stty -echo; printf 'F2-READY\\n'; cat -v\r");
    tui.until(|screen| line(screen, "F2-READY"));
    tui.send(b"\x1bOQ\r");
    tui.until(|screen| line(screen, "^[OQ") && terminal_input(screen));
    tui.send(b"\x1c\x0e");
    tui.until(terminal_inspection);
}

#[test]
fn real_terminal_input_consent_quit_and_execution_free_replay() {
    let directory = tempfile::tempdir().unwrap();
    let trace = directory.path().join("terminal.jsonl");
    std::fs::write(
        directory.path().join("pager.txt"),
        (0..100)
            .map(|index| format!("PAGER-LINE-{index:03}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let mut tui = Tui::start(directory.path(), &trace);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":terminal env PS1='STROP-PTY> ' /bin/sh -i\r");
    tui.until(|screen| terminal_input(screen) && at_prompt(screen));
    tui.send(b"printf x >> run-count; printf 'PTY-INPUT-OK\\n'\r");
    tui.until(|screen| line(screen, "PTY-INPUT-OK"));
    // A multiline paste into a child without bracketed paste goes through
    // raw, as every terminal sends it (0090 §4): no held paste, no review.
    tui.send(b"printf '\\033[?2004lPASTE-READY\\n'; read first; read second; printf 'PASTE:%s|%s\\n' \"$first\" \"$second\"\r");
    tui.until(|screen| line(screen, "PASTE-READY"));
    tui.send(b"\x1b[200~alpha\nbeta\n\x1b[201~");
    tui.until(|screen| line(screen, "PASTE:alpha|beta") && terminal_input(screen));
    tui.send(b"nvim --clean nested.txt\r");
    tui.until(|screen| {
        screen.contains("nested.txt") && screen.lines().any(|row| row.starts_with('~'))
    });
    tui.send(b"iNESTED-EDITOR-OK");
    tui.until(|screen| screen.contains("-- INSERT --"));
    tui.send(b"\x1b");
    tui.until(|screen| !screen.contains("-- INSERT --") && screen.contains("NESTED-EDITOR-OK"));
    tui.send(b":wq\r");
    tui.until(|screen| at_prompt(screen) && terminal_input(screen));
    let nested = std::fs::read_to_string(directory.path().join("nested.txt"))
        .unwrap_or_else(|error| panic!("{error}:\n{}", tui.screen.screen().contents()));
    assert_eq!(nested, "NESTED-EDITOR-OK\n");
    tui.send(b"less pager.txt\r");
    tui.until(|screen| {
        screen.contains("PAGER-LINE-000") && screen.contains("pager.txt") && !at_prompt(screen)
    });
    tui.send(b"q");
    tui.until(|screen| at_prompt(screen) && terminal_input(screen));
    tui.send(b"printf '\\033[2J\\033[HINTERRUPT-READY\\n'; cat\r");
    tui.until(|screen| line(screen, "INTERRUPT-READY"));
    tui.send(b"\x03");
    tui.until(|screen| at_prompt(screen) && terminal_input(screen));
    // (raw Esc/Alt byte fidelity is asserted exactly by the od step below)
    tui.send(b"printf 'ESC-REMAINED\\n'\r");
    tui.send(b"stty raw -echo; printf 'BYTE-READY\\n'; dd bs=1 count=8 2>/dev/null | od -An -tx1; stty sane; printf '\\r\\nBYTE-DONE\\n'\r");
    tui.until(|screen| line(screen, "BYTE-READY"));
    tui.send(b"\x12\x1bx\x1b[15~");
    let expected_bytes = |screen: &str| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "12 1b 78 1b 5b 31 35 7e"
        })
    };
    if tui
        .until_soft(Duration::from_secs(30), expected_bytes)
        .is_none()
    {
        // Complete a short read for diagnosis without accepting substituted
        // bytes: the expected exact sequence above still fails.
        tui.send(b"????????");
        let observed =
            tui.until_within(Duration::from_secs(15), |screen| line(screen, "BYTE-DONE"));
        panic!(
            "terminal did not pass the exact raw input bytes:\n{observed}\nconsented key actions:\n{}",
            tui.recent_input_trace()
        );
    }
    tui.until(|screen| line(screen, "BYTE-DONE"));
    // Ctrl-W is ordinary child input. A nonmatching Ctrl-\ follow-up
    // forwards the escape-prefix byte plus the key, in order.
    tui.send(b"stty raw -echo; printf 'PREFIX-READY\\n'; dd bs=1 count=3 2>/dev/null | od -An -tx1; stty sane; printf '\\r\\nPREFIX-DONE\\n'\r");
    tui.until(|screen| line(screen, "PREFIX-READY"));
    tui.send(b"\x17\x1cz");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "17 1c 7a"
        })
    });
    tui.until(|screen| line(screen, "PREFIX-DONE"));
    // Application versus normal cursor-key mode (0055 §12): the child's
    // DECCKM toggle re-aims the same Up key through the mode-aware
    // encoder — SS3 (ESC O A) in application mode, CSI (ESC [ A) after
    // the reset. Byte-asserted, not assumed from the engine choice.
    tui.send(b"stty raw -echo; printf '\\033[?1hMODE-APP\\n'; dd bs=1 count=3 2>/dev/null | od -An -tx1; printf '\\033[?1lMODE-NORM\\n'; dd bs=1 count=3 2>/dev/null | od -An -tx1; stty sane; printf '\\r\\nMODE-DONE\\n'\r");
    tui.until(|screen| line(screen, "MODE-APP"));
    tui.send(b"\x1b[A");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "1b 4f 41"
        })
    });
    tui.until(|screen| line(screen, "MODE-NORM"));
    tui.send(b"\x1b[A");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "1b 5b 41"
        })
    });
    tui.until(|screen| line(screen, "MODE-DONE"));
    // The chord enters NORMAL at a calm prompt; `i` returns.
    tui.send(b"\x1c\x0e");
    tui.until(|screen| terminal_inspection(screen) && !terminal_input(screen));
    tui.send(b"i");
    tui.until(terminal_input);
    // Sustained output (0055 §12): while a sixteen-KiB `yes` stream floods
    // the PTY, the editor still admits input and renders — the mode escape
    // lands mid-flood — and the bounded history drops the flood's head:
    // FLOOD-START cannot survive, FLOOD-DONE does, and the shell prompt
    // returns. Markers are line-anchored so the echoed command cannot
    // satisfy them. The session must ALSO stay fully capturable: frames
    // travel as cell runs, so the flood cannot exhaust the capture bound
    // and degrade the trace.
    tui.send(b"printf 'FLOOD-START\\n'; yes | head -c 16384; printf '\\r\\nFLOOD-DONE\\n'\r");
    tui.send(b"\x1c");
    tui.send(b"\x0e");
    // The capture's per-update frame records queue ahead of this escape on
    // slow runners; the escape still lands in order — the mid-flood input
    // contract. The mode chip is the proof.
    tui.until(|screen| terminal_inspection(screen) && !terminal_input(screen));
    tui.send(b"i");
    let settled = tui.until(|screen| line(screen, "FLOOD-DONE") && at_prompt(screen));
    assert!(
        !settled.contains("FLOOD-START"),
        "bounded history must drop the flood head"
    );
    tui.send(b"mkfifo pause; (exec 3<>pause; printf '\\r\\nINSPECTION-READY\\n'; read go <&3; printf '\\r\\nASYNC-INSPECTION-OUTPUT\\n') &\r");
    tui.until(|screen| line(screen, "INSPECTION-READY"));
    tui.send(b"\x1c");
    tui.send(b"\x0e");
    tui.until(|screen| terminal_inspection(screen) && !terminal_input(screen));
    File::options()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(directory.path().join("pause"))
        .unwrap()
        .write_all(b"go\n")
        .unwrap();
    // NORMAL is a live buffer (0090 §4): the deferred line lands in the
    // numbered view by itself, and the cursor on the last line follows
    // it — no refresh, no attention badge.
    let landed = tui.until(|screen| numbered_line(screen, "ASYNC-INSPECTION-OUTPUT"));
    assert!(terminal_inspection(&landed), "{landed}");
    assert!(!landed.contains("out+"), "{landed}");
    tui.send(b"gg/^INSPECTION-READY\ryy:e yank-target.txt\r");
    tui.until(|screen| screen.contains("yank-target.txt") && !screen.contains("term://"));
    tui.send(b"p");
    tui.until(|screen| !screen.contains("term://") && numbered_line(screen, "INSPECTION-READY"));
    tui.send(b":q!\r");
    tui.until(|screen| terminal_inspection(screen) && screen.contains("term://"));
    tui.send(b":vs\ri");
    // 0064 §1 geometry: left pane cols 0..58, its track 58, divider 59,
    // right pane content 60..119, right track 119 — compare by range,
    // never by the divider glyph (the track shares it).
    let split = tui.until(|screen| {
        screen
            .lines()
            .any(|row| cells(row, 60, 119).trim() == "ASYNC-INSPECTION-OUTPUT")
    });
    assert!(!split
        .lines()
        .any(|row| cells(row, 0, 58).trim() == "ASYNC-INSPECTION-OUTPUT"));
    // The chord then Space w moves panes without the child seeing a byte:
    // focus lands on the left pane (NORMAL), then returns to the terminal
    // pane and resumes live input with i.
    tui.send(b"\x1c\x0e w l");
    tui.until(|screen| terminal_inspection(screen) && !terminal_input(screen));
    tui.send(b" w hi");
    tui.until(terminal_input);
    tui.send(b"printf 'SIZE:'; stty size\r");
    tui.until(|screen| {
        // The 60-column right pane reserves one track column (0064 §1),
        // so the child's grid is 28x59.
        screen
            .lines()
            .any(|row| cells(row, 60, 119).trim() == "SIZE:28 59")
    });
    tui.send(b"\x1c\x0e:q\ri");
    tui.until(|screen| {
        // split closed: no divider remains; the only reserved cell is
        // the single pane's own track at the last column (0064 §1).
        terminal_input(screen)
            && !screen
                .lines()
                .take(29)
                .any(|row| row.chars().take(119).any(|cell| cell == '\u{2502}'))
    });
    // :qa with a running session is one notice, no picker (0089 B7);
    // :qa! stops the shell and quits.
    tui.send(b"\x1c\x0e:qa\r");
    tui.until(|screen| screen.contains("1 terminal running") && screen.contains(":qa! stops them"));
    tui.send(b":qa!\r");
    tui.expect_clean_exit("the editor did not exit cleanly");
    // The flood never degraded the capture: the file says so itself.
    let terminal = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .last()
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .unwrap_or_default();
    assert_eq!(
        terminal["event"], "trace_end",
        "trace must end with its marker"
    );
    assert_eq!(
        terminal["fields"]["complete"], true,
        "flooded capture stayed complete: {terminal}"
    );
    let before = std::fs::read(directory.path().join("run-count")).unwrap();
    assert_eq!(before, b"x");
    let replay = Command::new(env!("CARGO_BIN_EXE_strop"))
        .arg("--replay")
        .arg(&trace)
        .env("HOME", directory.path())
        .env("SHELL", "/unavailable-during-replay")
        .env_remove("STROP_LOG")
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("run-count")).unwrap(),
        before
    );
}

#[test]
fn two_escapes_in_one_write_close_a_picker() {
    // B4: a fast double Escape arrives as `\x1b\x1b` in one read; vim reads
    // two keys. The first leaves the query field, the second closes.
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("main.rs"), "fn main() {}\n").unwrap();
    let trace = directory.path().join("terminal.jsonl");
    let mut tui = Tui::start(directory.path(), &trace);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b" f");
    tui.until(|screen| screen.contains("FILES") && screen.contains("esc  close"));
    tui.send(b"\x1b\x1b");
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("esc  close"));
    tui.send(b":q\r");
    tui.expect_clean_exit("the editor did not exit cleanly");
}
