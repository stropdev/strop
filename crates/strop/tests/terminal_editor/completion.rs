use super::*;
use std::io::{BufRead, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

impl Tui {
    fn resize_completion(&mut self, columns: u16, rows: u16) {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        self.screen.set_size(rows, columns);
        // SAFETY: the owned PTY master is live and size is initialized. The
        // kernel's TIOCSWINSZ delivers the real terminal resize notification.
        assert_eq!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) },
            0
        );
        self.send(b"\x0c");
        self.until(|screen| {
            screen.lines().count() == usize::from(rows)
                && screen
                    .lines()
                    .last()
                    .is_some_and(|line| line.contains("INSERT") || line.contains("NORMAL"))
        });
    }
}

#[test]
fn completion_physical_word_keys_resize_accept_escape_and_undo() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let original = "result response reusable return_value\n\n";
    std::fs::write(root.join("words.txt"), original).unwrap();
    let (binary, digest, bytes) = benchmark_binary();
    eprintln!("0059 PTY binary={digest} bytes={bytes}");
    let mut tui = Tui::spawn(root, &root.join("completion.jsonl"), &binary, true);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":e words.txt\r");
    tui.until(|screen| numbered_line(screen, "result response reusable return_value"));
    tui.send(b"Gire\x0e");
    let wide = tui.until(|screen| {
        screen.contains("completions")
            && screen.contains("response")
            && screen.contains("\u{258c}word")
            && screen.contains("INSERT")
    });
    assert!(numbered_line(&wide, "re"));
    eprintln!("0059 WORD PTY 120x30\n{wide}");
    tui.resize_completion(26, 12);
    let narrow = tui.until(|screen| {
        screen.contains("completions")
            && screen
                .lines()
                .any(|line| line.contains("response") && line.contains("buf"))
    });
    assert!(numbered_line(&narrow, "re"));
    eprintln!("0059 WORD PTY 26x12\n{narrow}");
    tui.send(b"\x19");
    tui.until(|screen| numbered_line(screen, "response") && !screen.contains("completions"));
    tui.send(b"\x1b");
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b"u");
    tui.until(|screen| !numbered_line(screen, "response") && !numbered_line(screen, "re"));
    tui.send(b"\x12");
    tui.until(|screen| numbered_line(screen, "response"));
    tui.send(b":w\r");
    tui.until(|_| {
        std::fs::read_to_string(root.join("words.txt")).unwrap()
            == "result response reusable return_value\nresponse\n"
    });
    tui.send(b":q!\r");
    assert!(tui.wait_exit().success());
    assert_eq!(
        std::fs::read_to_string(root.join("words.txt")).unwrap(),
        "result response reusable return_value\nresponse\n"
    );
}

#[test]
fn completion_tab_preview_cycle_revert_and_commit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let original = "result response reusable return_value\n\n";
    std::fs::write(root.join("words.txt"), original).unwrap();
    let (binary, digest, bytes) = benchmark_binary();
    eprintln!("0059 PTY binary={digest} bytes={bytes}");
    let mut tui = Tui::spawn(root, &root.join("tab-preview.jsonl"), &binary, true);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":e words.txt\r");
    tui.until(|screen| numbered_line(screen, "result response reusable return_value"));
    let words = ["result", "response", "reusable", "return_value"];
    // Tab focuses the popup and previews the first candidate (0059 §6.1).
    tui.send(b"Gire");
    tui.until(|screen| screen.contains("completions"));
    tui.send(b"\t");
    let previewed = tui.until(|screen| {
        screen.contains("completions") && words.iter().any(|word| numbered_line(screen, word))
    });
    let first = words
        .iter()
        .find(|word| numbered_line(&previewed, word))
        .expect("first previewed candidate")
        .to_string();
    eprintln!("0059 TAB PREVIEW PTY 120x30\n{previewed}");
    // Shift-Tab wraps to the last candidate, Tab cycles back to the first.
    tui.send(b"\x1b[Z");
    let wrapped = tui.until(|screen| {
        screen.contains("completions")
            && words
                .iter()
                .any(|word| *word != first && numbered_line(screen, word))
    });
    eprintln!("0059 SHIFT-TAB WRAP PTY 120x30\n{wrapped}");
    tui.send(b"\t");
    tui.until(|screen| numbered_line(screen, &first));
    // Escape reverts the preview exactly and leaves Insert in one event.
    tui.send(b"\x1b");
    tui.until(|screen| {
        screen.contains("NORMAL")
            && numbered_line(screen, "re")
            && words.iter().all(|word| !numbered_line(screen, word))
    });
    // Retype, cycle once, and Ctrl-Y commits the previewed candidate.
    tui.send(b"u");
    tui.until(|screen| !numbered_line(screen, "re"));
    tui.send(b"ire");
    tui.until(|screen| screen.contains("completions"));
    tui.send(b"\t");
    let cycled = tui.until(|screen| {
        screen.contains("completions") && words.iter().any(|word| numbered_line(screen, word))
    });
    let chosen = words
        .iter()
        .find(|word| numbered_line(&cycled, word))
        .expect("cycled candidate")
        .to_string();
    tui.send(b"\x19");
    tui.until(|screen| !screen.contains("completions") && numbered_line(screen, &chosen));
    tui.send(b"\x1b");
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":w\r");
    tui.until(|_| {
        std::fs::read_to_string(root.join("words.txt")).unwrap()
            == format!("result response reusable return_value\n{chosen}\n")
    });
    tui.send(b":q!\r");
    assert!(tui.wait_exit().success());
    assert_eq!(
        std::fs::read_to_string(root.join("words.txt")).unwrap(),
        format!("result response reusable return_value\n{chosen}\n")
    );
}

#[test]
#[ignore = "0059 real-server qualification; requires installed clangd"]
fn completion_real_clangd_docs_narrow_split_and_acceptance() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("config/strop")).unwrap();
    std::fs::write(
        root.join("config/strop/config.toml"),
        "auto_format = false\n",
    )
    .unwrap();
    let source = "struct Request {\n    /// Budget used by the retry loop.\n    int retry_budget;\n    /// Number of retries already attempted.\n    int retry_count;\n};\nint main(void) {\n    struct Request request = {0};\n    request.\n}\n";
    std::fs::write(root.join("complete.c"), source).unwrap();
    let (binary, digest, bytes) = benchmark_binary();
    eprintln!("0059 PTY binary={digest} bytes={bytes}");
    let mut tui = Tui::spawn(root, &root.join("clangd.jsonl"), &binary, true);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":e complete.c\r");
    tui.until(|screen| screen.contains("struct Request request"));
    // Initialize/Ready precedes clangd's first AST. Its diagnostic for the
    // deliberately unfinished member expression witnesses that source parse.
    tui.until(|screen| {
        screen.contains("clangd ready") && screen.lines().any(|line| line.starts_with('●'))
    });
    tui.send(b"9G$a\x18\x0f");
    tui.until(|screen| {
        screen.contains("completions")
            && screen.contains("retry_budget")
            && screen.contains("retry_count")
            && screen.contains("lsp")
    });
    tui.send(b"\x0e");
    let wide = tui.until(|screen| {
        screen.contains(" documentation ") && screen.contains("Budget used by the retry loop.")
    });
    assert!(wide
        .lines()
        .any(|line| line.contains("request.") && !line.contains("request.retry")));
    eprintln!("0059 CLANGD PTY 120x30\n{wide}");
    tui.resize_completion(60, 18);
    let narrow = tui.until(|screen| {
        screen.contains("completions")
            && screen.contains("retry_budget")
            && screen.contains("lsp")
            && !screen.contains("documentation")
    });
    assert!(narrow
        .lines()
        .any(|line| line.contains("request.") && !line.contains("request.retry")));
    eprintln!("0059 CLANGD PTY 60x18\n{narrow}");
    tui.send(b"\x1b");
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("completions"));
    tui.resize_completion(120, 30);
    tui.send(b":vs\r9G$a\x18\x0f");
    let split = tui.until(|screen| {
        screen.contains("completions") && screen.contains("retry_budget") && screen.contains("lsp")
    });
    eprintln!("0059 CLANGD PTY SPLIT 120x30\n{split}");
    tui.send(b"\x0e");
    tui.until(|screen| {
        screen
            .lines()
            .any(|line| line.contains("\u{258c}field") && line.contains("retry_budget"))
    });
    tui.send(b"\x19");
    tui.until(|screen| screen.contains("request.retry_budget") && !screen.contains("completions"));
    tui.send(b"\x1b");
    tui.until(|screen| screen.contains("NORMAL"));
    let expected = source.replace("request.\n", "request.retry_budget\n");
    tui.send(b":w\r");
    tui.until(|_| std::fs::read_to_string(root.join("complete.c")).unwrap() == expected);
    tui.send(b":q!\r:q!\r");
    assert!(tui.wait_exit().success());
    assert_eq!(
        std::fs::read_to_string(root.join("complete.c")).unwrap(),
        expected
    );
}

fn fixture_command(
    control: &mut std::io::BufReader<UnixStream>,
    command: serde_json::Value,
) -> serde_json::Value {
    writeln!(control.get_mut(), "{command}").unwrap();
    let mut response = String::new();
    control.read_line(&mut response).unwrap();
    serde_json::from_str(&response).unwrap()
}

fn selected_source_word(screen: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains('\u{258c}') && line.contains("response") && line.contains("buf"))
}

#[test]
#[ignore = "0059 controlled slow-server qualification; requires Python and the repository fixture"]
fn completion_held_language_keeps_physical_typing_resize_and_split_live() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("config/strop")).unwrap();
    std::fs::write(
        root.join("config/strop/config.toml"),
        "auto_format = false\n",
    )
    .unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../verification/completion_server.py")
        .canonicalize()
        .unwrap();
    let socket = root.join("control.sock");
    let args = serde_json::to_string(&[
        fixture.to_str().unwrap(),
        "--mode",
        "hold",
        "--control",
        socket.to_str().unwrap(),
    ])
    .unwrap();
    std::fs::write(
        root.join("config/strop/languages.toml"),
        format!("[language-server.clangd]\ncommand = \"python3\"\nargs = {args}\n"),
    )
    .unwrap();
    std::fs::write(root.join("held.c"), "// fixture\nresponse result\nre\n").unwrap();
    let (binary, digest, bytes) = benchmark_binary();
    eprintln!("0059 SLOW PTY binary={digest} bytes={bytes}");
    let mut tui = Tui::spawn(root, &root.join("held.jsonl"), &binary, true);
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b":e held.c\r");
    tui.until(|screen| screen.contains("clangd ready"));
    let socket = UnixStream::connect(socket).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut control = std::io::BufReader::new(socket);
    tui.send(b"G$as");
    tui.until(|screen| screen.contains("completions") && screen.contains("response"));
    fixture_command(
        &mut control,
        serde_json::json!({"action": "await", "counter": "queries", "at_least": 1}),
    );
    tui.send(b"\x0e");
    let wide = tui.until(selected_source_word);
    eprintln!("0059 SLOW PTY 120x30\n{wide}");
    tui.send(b"p");
    fixture_command(
        &mut control,
        serde_json::json!({"action": "await", "counter": "queries", "at_least": 2}),
    );
    tui.send(b"\x7f");
    for _ in 0..8 {
        tui.send(b"p\x7f");
    }
    tui.until(|screen| numbered_line(screen, "res") && selected_source_word(screen));
    let held = fixture_command(
        &mut control,
        serde_json::json!({"action": "await", "counter": "cancels", "at_least": 2}),
    );
    assert_eq!(held["counts"]["queries"], 2);
    assert_eq!(held["counts"]["physical_high_water"], 2);
    assert_eq!(held["held"].as_array().unwrap().len(), 2);
    tui.resize_completion(26, 12);
    let narrow = tui.until(selected_source_word);
    assert!(numbered_line(&narrow, "res"));
    eprintln!("0059 SLOW PTY 26x12\n{narrow}");
    tui.send(b"\x1b");
    tui.until(|screen| screen.contains("NORMAL") && !screen.contains("completions"));
    tui.resize_completion(120, 30);
    tui.send(b":vs\rG$ap\x7f");
    tui.until(|screen| screen.contains("completions") && screen.contains("response"));
    tui.send(b"\x0e");
    tui.until(selected_source_word);
    fixture_command(&mut control, serde_json::json!({"action": "release"}));
    fixture_command(
        &mut control,
        serde_json::json!({"action": "await", "counter": "queries", "at_least": 3}),
    );
    fixture_command(&mut control, serde_json::json!({"action": "release"}));
    let split = tui.until(|screen| {
        selected_source_word(screen)
            && screen
                .lines()
                .any(|line| line.contains("response") && line.contains("lsp"))
    });
    eprintln!("0059 LATE LANGUAGE PTY SPLIT 120x30\n{split}");
    tui.send(b"\x19");
    tui.until(|screen| {
        !screen.contains("completions")
            && screen
                .lines()
                .any(|row| row.split('│').any(|pane| numbered_line(pane, "response")))
    });
    tui.send(b"\x1b");
    tui.until(|screen| screen.contains("NORMAL"));
    tui.send(b"u");
    tui.until(|screen| {
        screen.lines().any(|row| {
            row.split('│').any(|pane| numbered_line(pane, "res"))
                && !row.split('│').any(|pane| numbered_line(pane, "response"))
        })
    });
    tui.send(b":q!\r:q!\r");
    assert!(tui.wait_exit().success());
}
