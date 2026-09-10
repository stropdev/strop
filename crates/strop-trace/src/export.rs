//! Readers over a finished trace file: the privacy-preserving metadata
//! export and the forensic node extractor. Both enforce the same physical
//! contract the writer guarantees — schema, contiguous sequence, a single
//! terminal marker — and never trust a truncated file.
use std::io::{self, BufRead, Read, Write};

use serde::Deserialize;
use serde_json::Value;

use crate::{EventKind, MAX_CAPTURE_BYTES, MAX_RECORD_BYTES, SCHEMA_VERSION};

/// One physical trace line as written by the writer.
#[derive(Deserialize)]
pub struct Row {
    pub schema_version: u32,
    pub seq: u64,
    pub event: EventKind,
    pub fields: Value,
}

/// Bounded line size for the reader: a physical line may never exceed the
/// writer's per-record cap plus the envelope the writer adds around it.
const LINE_LIMIT: usize = MAX_RECORD_BYTES + 1024;
/// Reader total: the writer's hard capture bound. A bigger "trace file"
/// is not a trace this crate ever produced.
const TOTAL_LIMIT: usize = MAX_CAPTURE_BYTES;

/// Stream the rows of a trace. Verifies schema, sequence continuity, a
/// single terminal `TraceEnd` and bounded size. Returns whether the file
/// ended complete (`TraceEnd` present with `complete: true`).
///
/// Schema 2 and 3 are both accepted (homogeneously per file). Schema-3
/// chunk carriers are reassembled before delivery: visitors see logical
/// records only, an assembled record keeps its first chunk's sequence, and
/// every corruption of a chunk run is an explicit error. A run abandoned
/// by the stream makes the trace incomplete — an error when the terminal
/// marker claims otherwise — and its partial bytes are never delivered.
pub fn scan(
    mut input: impl BufRead,
    mut visit: impl FnMut(Row) -> io::Result<()>,
) -> io::Result<bool> {
    let (mut total, mut seq, mut ended, mut complete) = (0usize, 1u64, false, false);
    let mut version: Option<u32> = None;
    let mut chunks = crate::chunk::Assembler::new();
    loop {
        let mut bytes = Vec::new();
        let read = Read::by_ref(&mut input)
            .take(LINE_LIMIT as u64 + 1)
            .read_until(b'\n', &mut bytes)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read)
            .ok_or_else(|| io::Error::other("input limit"))?;
        if read > LINE_LIMIT || total > TOTAL_LIMIT || bytes.last() != Some(&b'\n') {
            return Err(io::Error::other("trace truncated or exceeds input limit"));
        }
        if ended {
            return Err(io::Error::other("records after trace terminal"));
        }
        let row: Row =
            serde_json::from_slice(&bytes).map_err(|_| io::Error::other("invalid trace record"))?;
        // Schemas 2 and 3 decode identically apart from chunk records, and
        // a file never mixes versions.
        if version.is_none() {
            version = Some(row.schema_version);
        }
        let supported = row.schema_version == SCHEMA_VERSION || row.schema_version == 2;
        if !supported || version != Some(row.schema_version) || row.seq != seq {
            return Err(io::Error::other("unsupported schema or missing sequence"));
        }
        seq = seq
            .checked_add(1)
            .ok_or_else(|| io::Error::other("sequence overflow"))?;
        if row.event == EventKind::ReplayChunk {
            if row.schema_version < 3 {
                return Err(io::Error::other("chunk record in pre-chunk schema"));
            }
            if let Some((first, event, fields)) = chunks.accept(row.seq, row.fields)? {
                visit(Row {
                    schema_version: row.schema_version,
                    seq: first,
                    event,
                    fields,
                })?;
            }
            continue;
        }
        if row.event == EventKind::TraceEnd {
            ended = true;
            complete = row.fields["complete"] == true;
            if complete && chunks.is_open() {
                return Err(io::Error::other(
                    "complete trace cannot abandon a chunk run",
                ));
            }
        }
        visit(row)?;
    }
    Ok(ended && complete)
}

/// Metadata export: a POSITIVE projection of the trace. Only the sequence
/// number and the closed `EventKind` category survive — no keys, paste or
/// file text, paths or native byte arrays, argv, command strings, messages,
/// errors, backtraces, protocol packets, identities, content hashes or
/// arbitrary nested fields. It records that categories occurred, nothing
/// else, and is explicitly not replayable.
pub fn metadata(input: impl BufRead, mut out: impl Write) -> io::Result<()> {
    writeln!(
        out,
        "{{\"schema\":\"strop-metadata-export-v1\",\"replayable\":false}}"
    )?;
    let complete = scan(input, |row| {
        serde_json::to_writer(
            &mut out,
            &serde_json::json!({"seq": row.seq, "category": row.event}),
        )?;
        out.write_all(b"\n")
    })?;
    serde_json::to_writer(
        &mut out,
        &serde_json::json!({"export_end":true,"source_complete":complete,"replayable":false}),
    )?;
    out.write_all(b"\n")
}

/// Extract the forensic node stream. Requires a complete full-content
/// capture — a capped, failed or metadata trace is never replayed as a
/// valid prefix — and the nodes must run `Seed`…`End`.
pub fn replay_nodes(input: impl BufRead) -> io::Result<Vec<crate::replay::Node>> {
    let mut nodes = Vec::new();
    let mut full = false;
    let complete = scan(input, |row| {
        if row.seq == 1 {
            full = row.event == EventKind::SessionStart && row.fields["full_content"] == true;
        }
        if row.event == EventKind::Replay {
            nodes.push(
                serde_json::from_value(row.fields)
                    .map_err(|_| io::Error::other("invalid forensic record"))?,
            );
        }
        Ok(())
    })?;
    if !full || !complete {
        return Err(io::Error::other(
            "full replay requires complete full-content capture",
        ));
    }
    if !matches!(nodes.first(), Some(crate::replay::Node::Seed { .. }))
        || !matches!(nodes.last(), Some(crate::replay::Node::End))
    {
        return Err(io::Error::other("missing forensic seed or end"));
    }
    Ok(nodes)
}
