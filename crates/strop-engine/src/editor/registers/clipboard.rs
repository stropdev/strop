//! The system clipboard (`+`/`*`): writes stage an OSC52 payload,
//! reads run on a worker and land on the drain — never a subprocess on
//! the input path (0001 §3).

use strop_core::worker::{FailureKind, Outcome};

use super::{Register, RegisterShape};

/// Where register text lands — one delivery for every surface, so a
/// stored register and a late clipboard read paste identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasteDestination {
    /// Normal-mode `p`/`P` in the document.
    Put { before: bool, count: usize },
    /// Insert-mode `Ctrl-R {reg}`: literal text at every cursor.
    Insert,
    /// The open `: / ? |` line.
    Prompt,
    /// The open picker's focused field.
    Picker,
}

/// A register as one-line field text: a linewise register of a single
/// line pastes that line (its break is the register's shape, not
/// content). Text with inner breaks stays whole so the field refuses
/// it visibly instead of truncating.
pub(super) fn field_text(register: &Register) -> &str {
    if register.shape != RegisterShape::Linewise {
        return &register.text;
    }
    let line = register
        .text
        .strip_suffix('\n')
        .map_or(register.text.as_str(), |line| {
            line.strip_suffix('\r').unwrap_or(line)
        });
    if line.contains('\n') {
        &register.text
    } else {
        line
    }
}

/// Read the system clipboard via the first working provider (helix's
/// playbook: wl-paste, xclip, xsel). Runs on a worker; every
/// failure is terminal and typed — "not installed" tries the next
/// provider, anything else (spawn error, nonzero exit, non-UTF-8
/// data) reports itself instead of masquerading as an empty
/// clipboard.
pub(super) fn read_system_clipboard() -> Outcome<String> {
    let providers: [(&str, &[&str]); 3] = [
        ("wl-paste", &[]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
    ];
    let mut failed = Vec::new();
    for (cmd, args) in providers {
        let output = match std::process::Command::new(cmd)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        {
            Ok(output) => output,
            // not installed: the next provider gets its chance
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Outcome::failed(FailureKind::Spawn, format!("{cmd}: {error}"));
            }
        };
        if !output.status.success() {
            failed.push(format!("{cmd}: exited {}", output.status));
            continue;
        }
        return match String::from_utf8(output.stdout) {
            Ok(text) => Outcome::Success(text),
            Err(error) => Outcome::failed(
                FailureKind::Io,
                format!("{cmd}: clipboard is not UTF-8: {error}"),
            ),
        };
    }
    let mut message = String::from("no clipboard provider succeeded");
    if !failed.is_empty() {
        message.push_str(&format!(" ({})", failed.join("; ")));
    }
    Outcome::failed(FailureKind::Unavailable, message)
}
