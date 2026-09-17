//! Checkpoint persistence (0056 AR04 §5): one framed record file per
//! workspace in the private state directory, staged with private
//! permissions and atomically published through the session persistence
//! boundary (0700 directories, 0600 staging, 16 MiB bound). A cohort is
//! all-or-nothing: a failed or refused publication leaves the last
//! complete checkpoint in place.

use super::record::{
    invalid, millis, CohortHeader, DraftOrigin, DraftRecord, Snapshot, SourceObservation,
    WorkspaceBinding, FORMAT_VERSION,
};
use crate::session::{persistence, SessionError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use strop_core::id::{BufferRevision, DocumentId};

const MAGIC: [u8; 8] = *b"STROPDR1";
const HEADER_LEN_BYTES: u64 = 8;
/// Header-size slack in the capture budget; the exact header length is
/// verified after serialization and records are demoted whole — never
/// truncated — if the slack was somehow insufficient.
const HEADER_SLACK: u64 = 64 * 1024;

/// One draft's immutable snapshot, moved whole to the persistence worker.
/// Text is a rope clone (structural, cheap) — byte materialization happens
/// off the input path.
pub struct PendingRecord {
    pub document: DocumentId,
    pub origin: DraftOrigin,
    pub revision: BufferRevision,
    pub source_path: Option<PathBuf>,
    pub text: ropey::Rope,
}

/// What a publication left durable, per record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedRecord {
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub captured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pub cohort: u64,
    pub captured_ms: u64,
    pub bytes: u64,
    pub records: Vec<PublishedRecord>,
    /// Records whose draft bytes did not fit the bound; their state is
    /// reported instead of silently truncating them.
    pub over_bound: Vec<(DocumentId, u64)>,
}

/// A checkpoint read back from the store: header plus the durable bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCohort {
    pub header: CohortHeader,
    /// Parallel to `header.records`; `Some` for captured records.
    pub texts: Vec<Option<Vec<u8>>>,
}

/// The workspace's checkpoint path — the same workspace key the session
/// store uses (`session_path`), a different owned directory and product.
pub fn checkpoint_path(base_dir: &Path, cwd: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    cwd.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    base_dir
        .join("strop")
        .join("recovery")
        .join(format!("{key}.recovery"))
}

/// Chunk-wise copy; runs on the persistence worker, never the input path.
fn rope_bytes(rope: &ropey::Rope) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rope.len_bytes());
    for chunk in rope.chunks() {
        bytes.extend_from_slice(chunk.as_bytes());
    }
    bytes
}

fn observe(path: &Path) -> Option<SourceObservation> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(SourceObservation {
        modified_ms: metadata.modified().map_or(0, millis),
        len: metadata.len(),
    })
}

/// Build and atomically publish one coherent cohort. Worker-side: every
/// blocking step (source observation, byte materialization, staging,
/// fsync) is off the input path.
pub fn publish_cohort(
    path: &Path,
    cohort: u64,
    captured_ms: u64,
    workspace: WorkspaceBinding,
    records: Vec<PendingRecord>,
) -> Result<Published, SessionError> {
    let mut drafted = Vec::with_capacity(records.len());
    for record in records {
        let source = record.source_path.as_deref().and_then(observe);
        drafted.push((record, source));
    }
    // Budget the cohort: every draft fits whole or is reported over-bound.
    let mut budget = MAGIC.len() as u64 + HEADER_LEN_BYTES + HEADER_SLACK;
    let mut snapshots: Vec<Snapshot> = Vec::with_capacity(drafted.len());
    for (record, _) in &drafted {
        let len = record.text.len_bytes() as u64;
        // The verified completeness decision (0057 VF18, strop_core::cohortguard):
        // whole or reported over-bound, never truncated. `budget <= MAX_BYTES`
        // holds: it starts below the limit and grows only by drafts that fit.
        if strop_core::cohortguard::fits(budget, len, persistence::MAX_BYTES) {
            budget += len;
            snapshots.push(Snapshot::Captured { text_len: len });
        } else {
            snapshots.push(Snapshot::OverBound { bytes: len });
        }
    }
    let build_header = |snapshots: &[Snapshot]| -> Result<Vec<u8>, SessionError> {
        let header = CohortHeader {
            version: FORMAT_VERSION,
            cohort,
            captured_ms,
            workspace: workspace.clone(),
            records: drafted
                .iter()
                .zip(snapshots)
                .map(|((record, source), snapshot)| DraftRecord {
                    origin: record.origin.clone(),
                    revision: record.revision,
                    document: record.document,
                    source: *source,
                    snapshot: snapshot.clone(),
                })
                .collect(),
        };
        Ok(serde_json::to_vec(&header)?)
    };
    let mut header = build_header(&snapshots)?;
    // The slack covered the header; if not, demote whole records until the
    // framed total honors the bound.
    let mut captured_bytes: u64 = snapshots
        .iter()
        .map(|snapshot| match snapshot {
            Snapshot::Captured { text_len } => *text_len,
            Snapshot::OverBound { .. } => 0,
        })
        .sum();
    while MAGIC.len() as u64 + HEADER_LEN_BYTES + header.len() as u64 + captured_bytes
        > persistence::MAX_BYTES
    {
        let Some(index) = snapshots
            .iter()
            .rposition(|snapshot| matches!(snapshot, Snapshot::Captured { .. }))
        else {
            return Err(invalid("header alone exceeds the capture limit".into()));
        };
        let Snapshot::Captured { text_len } = snapshots[index] else {
            continue;
        };
        snapshots[index] = Snapshot::OverBound { bytes: text_len };
        captured_bytes -= text_len;
        header = build_header(&snapshots)?;
    }
    let mut texts = Vec::new();
    for ((record, _), snapshot) in drafted.iter().zip(&snapshots) {
        if matches!(snapshot, Snapshot::Captured { .. }) {
            texts.push(rope_bytes(&record.text));
        }
    }
    let total = MAGIC.len() as u64
        + HEADER_LEN_BYTES
        + header.len() as u64
        + texts.iter().map(|text| text.len() as u64).sum::<u64>();
    persistence::publish(path, |file| {
        use std::io::Write;
        file.write_all(&MAGIC)?;
        file.write_all(&(header.len() as u64).to_le_bytes())?;
        file.write_all(&header)?;
        for text in &texts {
            file.write_all(text)?;
        }
        file.sync_all()
    })?;
    Ok(Published {
        cohort,
        captured_ms,
        bytes: total,
        records: drafted
            .iter()
            .zip(&snapshots)
            .map(|((record, _), snapshot)| PublishedRecord {
                document: record.document,
                revision: record.revision,
                captured: matches!(snapshot, Snapshot::Captured { .. }),
            })
            .collect(),
        over_bound: drafted
            .iter()
            .zip(&snapshots)
            .filter_map(|((record, _), snapshot)| match snapshot {
                Snapshot::OverBound { bytes } => Some((record.document, *bytes)),
                Snapshot::Captured { .. } => None,
            })
            .collect(),
    })
}

/// Read and validate the durable checkpoint. Missing state is ordinary
/// absence, not an error.
pub fn load(path: &Path) -> Result<Option<StoredCohort>, SessionError> {
    let bytes = match persistence::read(path) {
        Ok(bytes) => bytes,
        Err(SessionError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    if bytes.len() < MAGIC.len() + HEADER_LEN_BYTES as usize || bytes[..8] != MAGIC {
        return Err(invalid("bad magic".into()));
    }
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&bytes[8..16]);
    let header_len = u64::from_le_bytes(len_bytes) as usize;
    let header_end = 16usize.saturating_add(header_len);
    if header_end > bytes.len() {
        return Err(invalid("truncated header".into()));
    }
    let header: CohortHeader = serde_json::from_slice(&bytes[16..header_end])?;
    header.validate()?;
    let mut texts = Vec::with_capacity(header.records.len());
    let mut offset = header_end;
    for record in &header.records {
        match record.snapshot {
            Snapshot::Captured { text_len } => {
                let end = offset.saturating_add(text_len as usize);
                if end > bytes.len() {
                    return Err(invalid("truncated draft text".into()));
                }
                texts.push(Some(bytes[offset..end].to_vec()));
                offset = end;
            }
            Snapshot::OverBound { .. } => texts.push(None),
        }
    }
    if offset != bytes.len() {
        return Err(invalid("trailing bytes after the cohort".into()));
    }
    Ok(Some(StoredCohort { header, texts }))
}

/// A deliberate discard: republish the cohort without one record. The
/// updated cohort comes back so the editor's surface stays truthful.
pub fn discard(
    path: &Path,
    stored: &StoredCohort,
    index: usize,
) -> Result<StoredCohort, SessionError> {
    if index >= stored.header.records.len() {
        return Err(invalid(format!("no recovery record #{}", index + 1)));
    }
    let kept: StoredCohort = StoredCohort {
        header: CohortHeader {
            records: stored
                .header
                .records
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, record)| record.clone())
                .collect(),
            ..stored.header.clone()
        },
        texts: stored
            .texts
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, text)| text.clone())
            .collect(),
    };
    let header = serde_json::to_vec(&kept.header)?;
    let captured: Vec<&[u8]> = kept.texts.iter().flatten().map(Vec::as_slice).collect();
    let total = MAGIC.len() as u64
        + HEADER_LEN_BYTES
        + header.len() as u64
        + captured.iter().map(|text| text.len() as u64).sum::<u64>();
    if total > persistence::MAX_BYTES {
        return Err(invalid("cohort exceeds the capture limit".into()));
    }
    persistence::publish(path, |file| {
        use std::io::Write;
        file.write_all(&MAGIC)?;
        file.write_all(&(header.len() as u64).to_le_bytes())?;
        file.write_all(&header)?;
        for text in &captured {
            file.write_all(text)?;
        }
        file.sync_all()
    })?;
    Ok(kept)
}

/// SystemTime now as unix milliseconds (checkpoint stamp).
pub fn now_ms() -> u64 {
    millis(SystemTime::now())
}
