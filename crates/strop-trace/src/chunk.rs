//! Bounded chunking for forensic values that exceed the per-record cap.
//!
//! A value whose serialized fields do not fit one record travels as an
//! ordered run of `replay_chunk` records sharing a capture id. Every chunk
//! declares the assembled byte total, the chunk count and the SHA-256 digest
//! of the assembled bytes, so the reader reassembles strictly: bounded by
//! the declared total, refusing missing, duplicated, reordered or foreign
//! chunks, a wrong total, a bad digest and an abandoned run. A partial run
//! is never presented as a complete value.
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::bounded::Bytes;
use crate::{EventKind, MAX_CAPTURE_BYTES};

/// An assembled value may never exceed the capture that holds it: anything
/// larger could never have been captured completely.
pub(crate) const MAX_VALUE_BYTES: usize = MAX_CAPTURE_BYTES;

/// A chunk slice rides inside the record envelope as a JSON string, whose
/// escaping at most doubles the source bytes; the reserve covers the chunk
/// metadata itself. Record caps below the floor make chunking absurd, so
/// the capture ends honestly instead of shredding into metadata.
const ENVELOPE_RESERVE: usize = 1024;
const MIN_CHUNK_RECORD: usize = 4096;

/// Process-unique chunk run id. The reader checks uniqueness within one
/// trace; a global counter makes collisions impossible by construction
/// rather than by convention.
static CAPTURE: AtomicU64 = AtomicU64::new(1);

#[derive(Serialize)]
struct Chunk<'a> {
    capture: u64,
    event: EventKind,
    total: usize,
    digest: [u8; 32],
    chunks: u32,
    index: u32,
    data: &'a str,
}

#[derive(Deserialize)]
struct ChunkBody {
    capture: u64,
    event: EventKind,
    total: usize,
    digest: [u8; 32],
    chunks: u32,
    index: u32,
    data: String,
}

/// Serialize `fields` as an ordered run of chunk record bodies, each within
/// `max_record`. Returns `None` when the value exceeds the assembled bound
/// or cannot serialize — the caller then ends the capture as it does today.
pub(crate) fn serialize<T: Serialize>(
    kind: EventKind,
    fields: &T,
    max_record: usize,
) -> Option<Vec<Vec<u8>>> {
    if max_record < MIN_CHUNK_RECORD {
        return None;
    }
    let mut bytes = Bytes::new(MAX_VALUE_BYTES);
    serde_json::to_writer(&mut bytes, fields).ok()?;
    let text = String::from_utf8(bytes.into_vec()).ok()?;
    let total = text.len();
    let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    let capture = CAPTURE.fetch_add(1, Ordering::Relaxed);
    let budget = (max_record - ENVELOPE_RESERVE) / 2;
    let mut slices = Vec::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let mut end = budget.min(rest.len());
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        debug_assert!(end > 0, "the budget dwarfs the widest UTF-8 char");
        slices.push(&rest[..end]);
        rest = &rest[end..];
    }
    let chunks = u32::try_from(slices.len()).ok()?;
    let mut records = Vec::with_capacity(slices.len());
    for (index, data) in slices.into_iter().enumerate() {
        let chunk = Chunk {
            capture,
            event: kind,
            total,
            digest,
            chunks,
            index: u32::try_from(index).ok()?,
            data,
        };
        // The geometry above guarantees the fit; the bounded encoder stays
        // the authority so a mistake here can never become a sliced line.
        let mut encoded = Bytes::new(max_record);
        serde_json::to_writer(&mut encoded, &chunk).ok()?;
        records.push(encoded.into_vec());
    }
    Some(records)
}

/// Reader-side reassembly over one record stream. At most one run is open:
/// the forensic tape lives on a single thread, so interleaved runs only
/// exist in a corrupt file.
pub(crate) struct Assembler {
    open: Option<Open>,
    seen: HashSet<u64>,
}

struct Open {
    capture: u64,
    seq: u64,
    event: EventKind,
    total: usize,
    digest: [u8; 32],
    chunks: u32,
    next: u32,
    data: String,
}

impl Assembler {
    pub(crate) fn new() -> Self {
        Self {
            open: None,
            seen: HashSet::new(),
        }
    }

    /// A run the terminal marker (or end of stream) would abandon.
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Accept one chunk record (`seq` is its physical sequence number) and
    /// return the assembled record — first chunk's sequence, original event
    /// and decoded fields — when a run completes.
    pub(crate) fn accept(
        &mut self,
        seq: u64,
        fields: Value,
    ) -> io::Result<Option<(u64, EventKind, Value)>> {
        let chunk: ChunkBody =
            serde_json::from_value(fields).map_err(|_| io::Error::other("invalid chunk record"))?;
        if chunk.chunks == 0 || chunk.total > MAX_VALUE_BYTES {
            return Err(io::Error::other("chunk declares impossible bounds"));
        }
        let open = match &mut self.open {
            Some(open) => open,
            None => {
                if chunk.index != 0 {
                    return Err(io::Error::other("orphan chunk record"));
                }
                if self.seen.contains(&chunk.capture) {
                    return Err(io::Error::other("chunk capture id reused"));
                }
                self.open.insert(Open {
                    capture: chunk.capture,
                    seq,
                    event: chunk.event,
                    total: chunk.total,
                    digest: chunk.digest,
                    chunks: chunk.chunks,
                    next: 0,
                    data: String::new(),
                })
            }
        };
        if open.capture != chunk.capture {
            return Err(io::Error::other("foreign chunk interrupts a chunk run"));
        }
        if chunk.event != open.event
            || chunk.total != open.total
            || chunk.digest != open.digest
            || chunk.chunks != open.chunks
        {
            return Err(io::Error::other("inconsistent chunk declaration"));
        }
        if chunk.index != open.next {
            return Err(io::Error::other("chunk missing, duplicated or reordered"));
        }
        if chunk.data.len() > open.total.saturating_sub(open.data.len()) {
            return Err(io::Error::other("chunk bytes exceed declared total"));
        }
        open.data.push_str(&chunk.data);
        open.next += 1;
        if open.next < open.chunks {
            return Ok(None);
        }
        if open.data.len() != open.total {
            return Err(io::Error::other(
                "assembled size differs from declared total",
            ));
        }
        let digest: [u8; 32] = Sha256::digest(open.data.as_bytes()).into();
        if digest != open.digest {
            return Err(io::Error::other("chunk digest mismatch"));
        }
        let value = serde_json::from_str(&open.data)
            .map_err(|_| io::Error::other("assembled value does not decode"))?;
        let assembled = (open.seq, open.event, value);
        self.seen.insert(chunk.capture);
        self.open = None;
        Ok(Some(assembled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export;

    fn chunk_lines(capture: u64, pieces: &[&str], total: usize, digest: [u8; 32]) -> Vec<String> {
        let chunks = pieces.len() as u32;
        pieces
            .iter()
            .enumerate()
            .map(|(index, data)| {
                serde_json::json!({
                    "schema_version": 3, "seq": 0, "elapsed_us": 0, "event": "replay_chunk",
                    "fields": {"capture": capture, "event": "replay", "total": total,
                               "digest": digest, "chunks": chunks, "index": index, "data": data}
                })
                .to_string()
            })
            .collect()
    }

    /// Physical sequence numbers must stay contiguous after mutations, so a
    /// corruption trips the chunk checks, never the sequence check.
    fn trace(mut lines: Vec<String>, complete: bool) -> String {
        lines.push(
            serde_json::json!({"schema_version": 3, "seq": 0, "elapsed_us": 0,
                "event": "trace_end", "fields": {"complete": complete, "reason": "complete"}})
            .to_string(),
        );
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let mut row: Value = serde_json::from_str(line).unwrap();
                row["seq"] = (index + 1).into();
                row.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    fn pieces(text: &str, size: usize) -> Vec<&str> {
        text.as_bytes()
            .chunks(size)
            .map(|slice| std::str::from_utf8(slice).unwrap())
            .collect()
    }

    fn visited(trace: &str) -> (io::Result<bool>, Vec<(u64, EventKind, Value)>) {
        let mut rows = Vec::new();
        let result = export::scan(trace.as_bytes(), |row| {
            rows.push((row.seq, row.event, row.fields));
            Ok(())
        });
        (result, rows)
    }

    const PAYLOAD: &str = r#"{"kind":"check","value":{"completion":"aaaaaaaaaaaaaaaaaaaa"}}"#;

    fn valid(complete: bool) -> String {
        let parts = pieces(PAYLOAD, 5);
        trace(
            chunk_lines(7, &parts, PAYLOAD.len(), Sha256::digest(PAYLOAD).into()),
            complete,
        )
    }

    #[test]
    fn chunk_run_reassembles_into_one_logical_record() {
        let (result, rows) = visited(&valid(true));
        assert!(result.unwrap(), "complete chunked trace scans complete");
        assert_eq!(rows.len(), 2, "one assembled record plus the terminal");
        assert_eq!(rows[0].0, 1, "the assembled record keeps the first seq");
        assert_eq!(rows[0].1, EventKind::Replay);
        let node: crate::replay::Node = serde_json::from_value(rows[0].2.clone()).unwrap();
        assert_eq!(
            node,
            crate::replay::Node::Check {
                value: serde_json::json!({"completion": "aaaaaaaaaaaaaaaaaaaa"})
            }
        );
        assert_eq!(rows[1].1, EventKind::TraceEnd);
    }

    #[test]
    fn ordinary_records_may_interleave_a_chunk_run() {
        let parts = pieces(PAYLOAD, 5);
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), Sha256::digest(PAYLOAD).into());
        lines.insert(
            2,
            serde_json::json!({"schema_version": 3, "seq": 0, "elapsed_us": 0,
                "event": "input", "fields": {"key": "x"}})
            .to_string(),
        );
        let (result, rows) = visited(&trace(lines, true));
        assert!(result.unwrap());
        assert_eq!(
            rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            vec![EventKind::Input, EventKind::Replay, EventKind::TraceEnd]
        );
    }

    #[test]
    fn missing_duplicate_and_reordered_chunks_are_refused() {
        let parts = pieces(PAYLOAD, 5);
        let digest = Sha256::digest(PAYLOAD).into();

        // Missing: drop the second chunk.
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), digest);
        lines.remove(1);
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error
            .to_string()
            .contains("missing, duplicated or reordered"));

        // Duplicate: the second chunk twice.
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), digest);
        lines.insert(1, lines[1].clone());
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error
            .to_string()
            .contains("missing, duplicated or reordered"));

        // Reordered: swap the second and third chunks.
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), digest);
        lines.swap(1, 2);
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error
            .to_string()
            .contains("missing, duplicated or reordered"));
    }

    #[test]
    fn foreign_orphan_and_reused_chunks_are_refused() {
        let parts = pieces(PAYLOAD, 5);
        let digest = Sha256::digest(PAYLOAD).into();

        // Foreign: another capture's chunk interrupts the open run.
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), digest);
        lines.splice(
            1..1,
            chunk_lines(99, &["{}"], 2, Sha256::digest("{}").into()),
        );
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("foreign"));

        // Orphan: the first chunk claims index 1.
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), digest);
        let mut row: Value = serde_json::from_str(&lines[0]).unwrap();
        row["fields"]["index"] = 1.into();
        lines[0] = row.to_string();
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("orphan"));

        // Reused: a second run repeats a completed capture id.
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), digest);
        lines.extend(chunk_lines(7, &parts, PAYLOAD.len(), digest));
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("reused"));
    }

    #[test]
    fn wrong_totals_and_bad_digests_are_refused() {
        let parts = pieces(PAYLOAD, 5);
        let digest: [u8; 32] = Sha256::digest(PAYLOAD).into();

        // One chunk disagrees about the declaration.
        let mut lines = chunk_lines(7, &parts, PAYLOAD.len(), digest);
        let mut row: Value = serde_json::from_str(&lines[2]).unwrap();
        row["fields"]["total"] = (PAYLOAD.len() + 1).into();
        lines[2] = row.to_string();
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("inconsistent"));

        // A consistent declaration the bytes do not honour.
        let lines = chunk_lines(7, &parts, PAYLOAD.len() + 1, digest);
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("declared total"));

        // Consistent declaration, matching size, wrong digest.
        let wrong: [u8; 32] = Sha256::digest("something else").into();
        let lines = chunk_lines(7, &parts, PAYLOAD.len(), wrong);
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("digest"));

        // A declared total beyond the assembled-value bound.
        let lines = chunk_lines(7, &parts, MAX_VALUE_BYTES + 1, digest);
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("impossible bounds"));
    }

    #[test]
    fn an_interrupted_run_is_detectable_and_never_delivered() {
        let parts = pieces(PAYLOAD, 5);
        // The stream dies two chunks short of completing the declared run.
        let mut lines = chunk_lines(7, &parts[..parts.len() - 2], PAYLOAD.len(), [0; 32]);
        for line in &mut lines {
            let mut row: Value = serde_json::from_str(line).unwrap();
            let digest: [u8; 32] = Sha256::digest(PAYLOAD).into();
            row["fields"]["digest"] = serde_json::to_value(digest).unwrap();
            row["fields"]["chunks"] = parts.len().into();
            *line = row.to_string();
        }
        // The stream simply ends mid-run: not complete, nothing delivered.
        let text = lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let mut row: Value = serde_json::from_str(line).unwrap();
                row["seq"] = (index + 1).into();
                row.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let (result, rows) = visited(&text);
        assert!(
            !result.unwrap(),
            "a stream without a terminal is incomplete"
        );
        assert!(rows.is_empty(), "a partial run is never presented");

        // A capped capture abandoned the run: still incomplete, still quiet.
        let (result, rows) = visited(&trace(lines.clone(), false));
        assert!(!result.unwrap());
        assert_eq!(rows.len(), 1, "only the terminal marker survives");
        assert_eq!(rows[0].1, EventKind::TraceEnd);

        // Claiming complete over an open run is corruption, not a cap.
        let error = visited(&trace(lines, true)).0.unwrap_err();
        assert!(error.to_string().contains("abandon"));
    }

    #[test]
    fn chunk_records_require_schema_three() {
        // A homogeneous schema-2 file containing a chunk record: schema 2
        // predates chunking, so this is corruption, not compatibility.
        let text = valid(true).replace("\"schema_version\":3", "\"schema_version\":2");
        let error = visited(&text).0.unwrap_err();
        assert!(error.to_string().contains("schema"));
    }

    #[test]
    fn worst_case_escaping_stays_within_the_record_cap() {
        // Every source char escapable: serialized JSON nearly doubles again
        // inside each chunk record's string.
        let fields = serde_json::json!({"data": "\"".repeat(200_000)});
        let records = serialize(EventKind::Replay, &fields, crate::MAX_RECORD_BYTES).unwrap();
        assert!(records.len() > 1, "the value had to be chunked");
        let mut assembler = Assembler::new();
        let mut delivered = None;
        for (index, body) in records.iter().enumerate() {
            assert!(body.len() <= crate::MAX_RECORD_BYTES);
            let fields: Value = serde_json::from_slice(body).unwrap();
            if let Some(assembled) = assembler.accept(index as u64 + 1, fields).unwrap() {
                delivered = Some(assembled);
            }
        }
        let (seq, event, value) = delivered.unwrap();
        assert_eq!((seq, event), (1, EventKind::Replay));
        assert_eq!(value, fields);
    }

    #[test]
    fn a_single_chunk_run_round_trips() {
        let fields = serde_json::json!({"data": "small but handed to the chunker"});
        let records = serialize(EventKind::Replay, &fields, crate::MAX_RECORD_BYTES).unwrap();
        assert_eq!(records.len(), 1);
        let mut assembler = Assembler::new();
        let body: Value = serde_json::from_slice(&records[0]).unwrap();
        let (_, event, value) = assembler.accept(9, body).unwrap().unwrap();
        assert_eq!(event, EventKind::Replay);
        assert_eq!(value, fields);
    }
}
