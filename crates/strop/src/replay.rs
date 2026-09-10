//! Trace consumers. `write_script` is the honest INPUT extractor: it
//! turns a full-content trace into an inspectable headless script and
//! never executes anything. `run_full` is the R11 forensic replay: a
//! complete full-content capture is reconstructed deterministically from
//! its seed and injected service results.
use crate::editor::Key;
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Schemas the input extractor understands. Schema 2 adds the forensic
/// `replay` substream and the terminal `trace_end` marker; schema 3 adds
/// chunked oversize carriers, which input extraction skips — a chunked
/// seed still refuses loudly as "no initial document snapshot".
const EXTRACT_SCHEMAS: [u32; 3] = [1, 2, 3];

pub fn write_script(path: &Path, out: &mut dyn Write) -> io::Result<()> {
    let source = io::BufReader::new(std::fs::File::open(path)?);
    let mut initial = false;
    let mut complete = false;
    let mut ended = false;
    writeln!(
        out,
        "# Extracted input only: inspect before executing; commands may write files or run shells."
    )?;
    writeln!(out, "# External services and filesystem state are not replayed. Initial content is a scratch buffer.")?;
    for (expected_sequence, line) in (1..).zip(source.lines()) {
        let line = line?;
        let mut event: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
        if !event["schema_version"]
            .as_u64()
            .and_then(|version| u32::try_from(version).ok())
            .is_some_and(|version| EXTRACT_SCHEMAS.contains(&version))
            || event["seq"] != expected_sequence
        {
            return Err(io::Error::other(
                "unsupported schema or incomplete trace sequence",
            ));
        }
        if ended {
            return Err(io::Error::other("records after trace terminal"));
        }
        let fields = &event["fields"];
        match event["event"].as_str() {
            Some("replay") if !initial && fields["kind"] == "seed" => {
                let seed: crate::editor::trace::seed::Seed =
                    serde_json::from_value(event["fields"]["value"].take())
                        .map_err(io::Error::other)?;
                writeln!(out, "buffer {}", serde_json::to_string(seed.input_text()?)?)?;
                initial = true;
            }
            Some("document") if !initial => {
                let text = fields["text"].as_str().ok_or_else(|| {
                    io::Error::other("reproduction needs a trace recorded with --log-content")
                })?;
                writeln!(out, "buffer {}", serde_json::to_string(text)?)?;
                initial = true;
            }
            Some("input") if fields["source"] == "external" => {
                if !initial {
                    return Err(io::Error::other(
                        "input precedes the initial document snapshot",
                    ));
                }
                if fields["action"] == "quit_intent" {
                    writeln!(out, "quit-intent")?;
                } else {
                    let key: Key =
                        serde_json::from_value(fields["key"].clone()).map_err(io::Error::other)?;
                    writeln!(out, "key {}", serde_json::to_string(&key)?)?;
                }
            }
            Some("paste") => {
                let text = fields["text"]
                    .as_str()
                    .ok_or_else(|| io::Error::other("paste contents were not captured"))?;
                writeln!(out, "paste {}", serde_json::to_string(text)?)?;
            }
            Some("resize") => {
                let columns = fields["columns"]
                    .as_u64()
                    .ok_or_else(|| io::Error::other("missing resize columns"))?;
                let rows = fields["rows"]
                    .as_u64()
                    .ok_or_else(|| io::Error::other("missing resize rows"))?;
                writeln!(out, "resize {columns} {rows}")?;
            }
            Some("session_end") => complete = fields["success"] == true,
            Some("trace_end") => {
                ended = true;
                if fields["complete"] != true {
                    return Err(io::Error::other("trace reports a capped or failed capture"));
                }
            }
            Some("error") if fields["incomplete"] == true => {
                return Err(io::Error::other("trace reports lost events"))
            }
            _ => {}
        }
    }
    if !initial {
        return Err(io::Error::other("trace has no initial document snapshot"));
    }
    if !ended {
        // Schema-1 traces ended with session_end only.
        writeln!(
            out,
            "# Trace has no terminal marker (schema 1) or was truncated."
        )?;
    }
    if !complete {
        writeln!(
            out,
            "# Trace has no successful session end (crash/error or truncated capture)."
        )?;
    }
    writeln!(out, "frame\nstate")
}

/// Full forensic replay (`--replay TRACE`): reconstruct the seeded editor
/// and re-run every recorded action with native launches suppressed. The
/// final logical state is printed; divergence is an error, not a diff.
pub fn run_full(path: &Path, out: &mut dyn Write) -> io::Result<()> {
    let source = io::BufReader::new(std::fs::File::open(path)?);
    let nodes = strop_trace::export::replay_nodes(source)?;
    let editor = crate::editor::trace::drive::replay(nodes)?;
    writeln!(out, "{}", crate::headless::state_json(&editor))
}
