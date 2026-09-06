//! Turn a full-content trace into an inspectable script, never execute a log.
use crate::editor::Key;
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::path::Path;

pub fn write_script(path: &Path, out: &mut dyn Write) -> io::Result<()> {
    let source = io::BufReader::new(std::fs::File::open(path)?);
    let mut initial = false;
    let mut complete = false;
    writeln!(
        out,
        "# Extracted input only: inspect before executing; commands may write files or run shells."
    )?;
    writeln!(out, "# External services and filesystem state are not replayed. Initial content is a scratch buffer.")?;
    for (expected_sequence, line) in (1..).zip(source.lines()) {
        let line = line?;
        let event: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
        if event["schema_version"] != strop_trace::SCHEMA_VERSION
            || event["seq"] != expected_sequence
        {
            return Err(io::Error::other(
                "unsupported schema or incomplete trace sequence",
            ));
        }
        let fields = &event["fields"];
        match event["event"].as_str() {
            Some("document") if !initial => {
                let text = fields["text"].as_str().ok_or_else(|| {
                    io::Error::other("reproduction needs a trace recorded with --log-content")
                })?;
                writeln!(out, "buffer {}", serde_json::to_string(text)?)?;
                initial = true;
            }
            Some("input") if fields["source"] == "external" => {
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
            Some("error") if fields["incomplete"] == true => {
                return Err(io::Error::other("trace reports lost events"))
            }
            _ => {}
        }
    }
    if !initial {
        return Err(io::Error::other("trace has no initial document snapshot"));
    }
    if !complete {
        writeln!(
            out,
            "# Trace has no successful session end (crash/error or truncated capture)."
        )?;
    }
    writeln!(out, "frame\nstate")
}
