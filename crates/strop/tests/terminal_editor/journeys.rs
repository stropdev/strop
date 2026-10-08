use super::*;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;
use std::time::Duration;

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
    tui.until(|screen| screen.contains("TERMINAL") && screen.contains("STROP-PTY>"));
    tui.send(b"printf x >> run-count; printf 'PTY-INPUT-OK\\n'\r");
    tui.until(|screen| line(screen, "PTY-INPUT-OK"));
    tui.send(b"printf '\\033[?2004lPASTE-READY\\n'; read first; read second; printf 'PASTE:%s|%s\\n' \"$first\" \"$second\"\r");
    tui.until(|screen| line(screen, "PASTE-READY"));
    tui.send(b"\x1b[200~alpha\nbeta\n\x1b[201~");
    let held = tui.until(|screen| screen.contains("paste held"));
    assert!(!line(&held, "alpha") && !line(&held, "beta"));
    tui.send(b"\x1c\x0e");
    tui.until(|screen| screen.contains("NORMAL") && screen.contains("snapshot"));
    tui.send(b":terminal-paste\r");
    tui.until(|screen| line(screen, "PASTE:alpha|beta") && screen.contains("TERMINAL"));
    tui.send(b"nvim --clean nested.txt\r");
    tui.until(|screen| {
        screen.contains("nested.txt") && screen.lines().any(|row| row.starts_with('~'))
    });
    tui.send(b"iNESTED-EDITOR-OK");
    tui.until(|screen| screen.contains("-- INSERT --"));
    tui.send(b"\x1b");
    tui.until(|screen| !screen.contains("-- INSERT --") && screen.contains("NESTED-EDITOR-OK"));
    tui.send(b":wq\r");
    tui.until(|screen| screen.contains("STROP-PTY>") && screen.contains("TERMINAL"));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("nested.txt")).unwrap(),
        "NESTED-EDITOR-OK\n"
    );
    tui.send(b"less pager.txt\r");
    tui.until(|screen| {
        screen.contains("PAGER-LINE-000")
            && screen.contains("pager.txt")
            && !screen.contains("STROP-PTY>")
    });
    tui.send(b"q");
    tui.until(|screen| screen.contains("STROP-PTY>") && screen.contains("TERMINAL"));
    tui.send(b"printf '\\033[2J\\033[HINTERRUPT-READY\\n'; cat\r");
    tui.until(|screen| line(screen, "INTERRUPT-READY"));
    tui.send(b"\x03");
    tui.until(|screen| screen.contains("STROP-PTY>") && screen.contains("TERMINAL"));
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
    // The prefix grammar's pass-through contract (0055 §12, literal
    // escape-prefix recovery): `Ctrl-W .` delivers the literal 0x17, and a
    // nonmatching `Ctrl-\` or `Ctrl-W` follow-up forwards the prefix byte
    // plus the key, in order — nothing the user typed may disappear.
    tui.send(b"stty raw -echo; printf 'PREFIX-READY\\n'; dd bs=1 count=5 2>/dev/null | od -An -tx1; stty sane; printf '\\r\\nPREFIX-DONE\\n'\r");
    tui.until(|screen| line(screen, "PREFIX-READY"));
    tui.send(b"\x17.\x1cz\x17z");
    tui.until(|screen| {
        screen.lines().any(|row| {
            strip_track(row)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                == "17 1c 7a 17 7a"
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
    // Sustained output (0055 §12): while a sixteen-KiB `yes` stream floods
    // the PTY, the editor still admits input and renders — the mode escape
    // lands mid-flood — and the bounded history drops the flood's head:
    // FLOOD-START cannot survive, FLOOD-DONE does, and the shell prompt
    // returns. Markers are line-anchored so the echoed command cannot
    // satisfy them. The session must ALSO stay fully capturable: frames
    // travel as cell runs, so the flood cannot exhaust the capture bound
    // and degrade the trace.
    tui.send(b"printf 'FLOOD-START\\n'; yes | head -c 16384; printf '\\r\\nFLOOD-DONE\\n'\r");
    tui.send(b"\x1c\x0e");
    // The capture's per-update frame records queue ahead of this escape on
    // slow runners; the escape still lands in order — the mid-flood input
    // contract. The pinned-view proof is the mode chip: the transient
    // "snapshot" message is cleared by the very next update while the
    // flood streams.
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("TERMINAL"));
    tui.send(b"i");
    let settled = tui.until(|screen| line(screen, "FLOOD-DONE") && screen.contains("STROP-PTY>"));
    assert!(
        !settled.contains("FLOOD-START"),
        "bounded history must drop the flood head"
    );
    tui.send(b"mkfifo pause; (exec 3<>pause; printf '\\r\\nINSPECTION-READY\\n'; read go <&3; printf '\\r\\nASYNC-INSPECTION-OUTPUT\\n') &\r");
    tui.until(|screen| line(screen, "INSPECTION-READY"));
    tui.send(b"\x1c\x0e");
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("TERMINAL"));
    File::options()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(directory.path().join("pause"))
        .unwrap()
        .write_all(b"go\n")
        .unwrap();
    let frozen = tui.until(|screen| screen.contains("new output"));
    assert!(!line(&frozen, "ASYNC-INSPECTION-OUTPUT"));
    // The pinned view never drags to new output (0065): refresh installs
    // the latest published frame, and a search inside the pinned buffer
    // is the honest probe that the deferred line landed — the caret
    // follows the match, revealing it. Frames may still be arriving at
    // the first refresh, so iterate the documented signal→refresh loop
    // like a user, bounded.
    let mut installed = false;
    for _ in 0..5 {
        tui.send(b":terminal-refresh\r");
        tui.until(|s| s.contains("refreshed") || s.contains("already shows"));
        tui.send(b"/ASYNC-INSPECTION-OUTPUT\r");
        if tui
            .until_soft(Duration::from_secs(5), |s| {
                numbered_line(s, "ASYNC-INSPECTION-OUTPUT")
            })
            .is_some()
        {
            installed = true;
            break;
        }
        // The refreshed frame predated the deferred line; let the
        // remaining frames settle, then refresh again.
        let _ = tui.until_soft(Duration::from_secs(2), |_| false);
    }
    assert!(
        installed,
        "terminal-refresh installs the deferred output into the pinned view"
    );
    tui.send(b"gg/^INSPECTION-READY\ryy:e yank-target.txt\r");
    tui.until(|screen| screen.contains("yank-target.txt") && !screen.contains("terminal #"));
    tui.send(b"p");
    tui.until(|screen| !screen.contains("terminal #") && numbered_line(screen, "INSPECTION-READY"));
    tui.send(b":q!\r");
    tui.until(|screen| screen.contains("NORMAL") && screen.contains("terminal #"));
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
    // From terminal input, the t_CTRL-W grammar moves panes without the
    // child seeing a byte: focus lands on the left editor pane (NORMAL),
    // then returns to the terminal pane (TERMINAL) still owning input.
    tui.send(b"\x17l");
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("TERMINAL"));
    tui.send(b"\x17h");
    tui.until(|screen| screen.contains("TERMINAL"));
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
        screen.contains("TERMINAL")
            && !screen
                .lines()
                .take(29)
                .any(|row| row.chars().take(119).any(|cell| cell == '\u{2502}'))
    });
    tui.send(b"\x1c\x0e:qa\r");
    tui.until(|screen| screen.contains("terminal sessions are running"));
    tui.send(b":terminal-stop\r");
    tui.until(|screen| {
        screen.contains("NORMAL")
            && (screen.contains("terminal ended") || screen.contains("terminal exited"))
    });
    tui.send(b":qa\r");
    assert!(tui.wait_exit().success());
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
