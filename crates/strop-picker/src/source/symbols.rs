//! The syntax-fallback symbol index (0063 §2): declarations extracted
//! from the files the selection walk already admitted, keyed by
//! workspace-relative path. Built once per search on the source worker
//! — never the input/render path — and only when the query carries a
//! `kind:` atom, so ordinary searches pay nothing.
//!
//! Soundness over completeness (0063 §4): an absent entry (unsupported
//! language, bounds, unreadable file) or an incomplete one (parse
//! failure, cancellation) is Unknown and admits; only a complete
//! extraction may answer No. Unsaved sources overlay their disk entry
//! — dirty text is authoritative.
//!
//! A retained disk shard owns the same bytes its linked extractor saw.
//! Refresh compares bytes on the worker; watcher silence, matching
//! timestamps and display spellings are not reuse evidence.
use std::collections::HashMap;
use std::io::Read;
use std::mem::{size_of, size_of_val};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use strop_core::symbol_kind::SymbolKind;
use strop_syntax::languages;
use strop_syntax::symbols::{DeclExtractor, Declaration, DeclarationSchema};

#[derive(Clone, Copy)]
struct Limits {
    files: usize,
    file_bytes: usize,
    total_bytes: usize,
}

const LIMITS: Limits = Limits {
    files: 4096,
    file_bytes: 2 * 1024 * 1024,
    total_bytes: 64 * 1024 * 1024,
};

/// Immutable syntax evidence, not a document revision or semantic binding.
/// Compare the actual source and schema before adopting for another request.
pub struct DiskShard {
    source: Box<[u8]>,
    schema: DeclarationSchema,
    declarations: Box<[Declaration]>,
    allocation: usize,
}

impl DiskShard {
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    pub fn schema(&self) -> DeclarationSchema {
        self.schema
    }

    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }
}

#[derive(Clone)]
enum FileSymbols {
    Disk(Arc<DiskShard>),
    Buffer(Arc<[Declaration]>),
}

impl FileSymbols {
    fn declarations(&self) -> &[Declaration] {
        match self {
            Self::Disk(shard) => shard.declarations(),
            Self::Buffer(declarations) => declarations,
        }
    }

    fn allocation(&self) -> usize {
        // Two reference counts in each Arc allocation.
        2 * size_of::<usize>()
            + match self {
                Self::Disk(shard) => shard.allocation,
                Self::Buffer(rows) => declaration_bytes(rows),
            }
    }
}

fn declaration_bytes(rows: &[Declaration]) -> usize {
    size_of_val(rows) + rows.iter().map(|row| row.name.capacity()).sum::<usize>()
}

/// Per-worker pass, including dirty snapshots. Failed static queries are
/// cached too; learning eligibility never constructs a throwaway parser.
#[derive(Default)]
pub(super) struct Extractors(HashMap<languages::LanguageId, Option<DeclExtractor>>);

impl Extractors {
    fn get(&mut self, language: languages::LanguageId) -> Option<&mut DeclExtractor> {
        self.0
            .entry(language)
            .or_insert_with(|| DeclExtractor::for_language(language))
            .as_mut()
    }
}

/// Scan accounting for one build/refresh (0058 S7 evidence): how many
/// eligible files were re-parsed versus reused unchanged.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanStats {
    pub parsed: usize,
    pub reused: usize,
}

/// The per-search symbol evidence for `kind:` atoms.
#[derive(Clone)]
pub struct SymbolIndex {
    files: HashMap<Arc<Path>, FileSymbols>,
    eligible: usize,
    interrupted: bool,
    source_bytes: usize,
    allocation: usize,
}

impl SymbolIndex {
    /// Extract declarations from the selection's eligible files. Paths
    /// are workspace-relative, matching admission's path vocabulary.
    pub fn build(cwd: &Path, paths: &[PathBuf], cancelled: &dyn Fn() -> bool) -> Self {
        Self::refresh(None, cwd, paths, &[], cancelled).0
    }

    /// Refresh only the current selection. A hint retires a path even if
    /// recreated identically. Without a hint, reuse requires byte equality
    /// and the exact linked syntax format; a missing path is never copied.
    pub fn refresh(
        prior: Option<&SymbolIndex>,
        cwd: &Path,
        paths: &[std::path::PathBuf],
        invalidated: &[std::path::PathBuf],
        cancelled: &dyn Fn() -> bool,
    ) -> (Self, ScanStats) {
        Self::scan(cwd, paths, prior, invalidated, cancelled, LIMITS)
    }

    fn scan(
        cwd: &Path,
        paths: &[std::path::PathBuf],
        prior: Option<&SymbolIndex>,
        invalidated: &[std::path::PathBuf],
        cancelled: &dyn Fn() -> bool,
        limits: Limits,
    ) -> (Self, ScanStats) {
        let mut index = Self {
            files: HashMap::new(),
            eligible: 0,
            interrupted: false,
            source_bytes: 0,
            allocation: 0,
        };
        let mut stats = ScanStats::default();
        let mut extractors = Extractors::default();
        let mut scratch = Vec::new();
        for path in paths {
            if cancelled() {
                index.interrupted = true;
                break;
            }
            let Some(language) =
                languages::detect_id(path, None).filter(|id| DeclExtractor::supports(*id))
            else {
                continue;
            };
            index.eligible += 1;
            let remaining = limits.total_bytes.saturating_sub(index.source_bytes);
            if index.eligible > limits.files || remaining == 0 {
                continue;
            }
            let limit = remaining.min(limits.file_bytes);
            let Ok(file) = std::fs::File::open(cwd.join(path)) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > limit as u64 {
                continue;
            }
            // Size is only an allocation hint. A file growing during the
            // read must not create an oversized (or prefix-only) witness.
            scratch.clear();
            scratch.reserve(metadata.len() as usize);
            let read = file.take(limit as u64 + 1).read_to_end(&mut scratch);
            // The one overflow probe is never part of an extracted source.
            index.source_bytes += scratch.len().min(limit);
            if read.is_err() || scratch.len() > limit {
                continue;
            }
            if !invalidated.iter().any(|prefix| path.starts_with(prefix)) {
                if let Some((key, FileSymbols::Disk(shard))) =
                    prior.and_then(|prior| prior.files.get_key_value(path.as_path()))
                {
                    if shard.schema == DeclarationSchema::ByteRangesV2(language)
                        && shard.source() == scratch.as_slice()
                    {
                        index.insert(Arc::clone(key), FileSymbols::Disk(Arc::clone(shard)));
                        stats.reused += 1;
                        continue;
                    }
                }
            }
            let Some(extractor) = extractors.get(language) else {
                continue;
            };
            stats.parsed += 1;
            let Some(declarations) = extractor.extract(&scratch, cancelled) else {
                continue;
            };
            let allocation =
                size_of::<DiskShard>() + scratch.len() + declaration_bytes(&declarations);
            let shard = DiskShard {
                source: std::mem::take(&mut scratch).into_boxed_slice(),
                schema: extractor.schema(),
                declarations: declarations.into_boxed_slice(),
                allocation,
            };
            index.insert(
                Arc::from(path.as_path()),
                FileSymbols::Disk(Arc::new(shard)),
            );
        }
        (index, stats)
    }

    fn insert(&mut self, path: Arc<Path>, entry: FileSymbols) {
        self.allocation += path.as_os_str().len() + 2 * size_of::<usize>() + entry.allocation();
        self.files.insert(path, entry);
    }

    /// Bounded ownership charge, computed on the worker, O(1) to inspect
    /// under the owner's lock. Charge shared shards in full, plus map slack.
    pub(crate) fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.allocation
            + (self.files.capacity() * 2 + 1) * (size_of::<(Arc<Path>, FileSymbols)>() + 1)
            + 16
    }

    /// Look up a candidate by native path in this inventory's root. Callers
    /// may hold a single immutable shard without pinning the whole inventory.
    pub fn disk_shard(&self, relative: &Path) -> Option<&Arc<DiskShard>> {
        match self.files.get(relative)? {
            FileSymbols::Disk(shard) => Some(shard),
            FileSymbols::Buffer(_) => None,
        }
    }

    /// An eligible dirty rope replaces its disk evidence even when its
    /// extraction fails or exceeds the budget. Never copy an oversize rope.
    pub(super) fn overlay(
        &mut self,
        relative: &Path,
        text: &ropey::Rope,
        extractors: &mut Extractors,
        cancelled: &dyn Fn() -> bool,
    ) {
        let prior = self.files.remove_entry(relative).map(|(key, entry)| {
            self.allocation -= key.as_os_str().len() + 2 * size_of::<usize>() + entry.allocation();
            key
        });
        if cancelled()
            || text.len_bytes() > LIMITS.file_bytes
            || text.len_bytes() > LIMITS.total_bytes.saturating_sub(self.source_bytes)
            || self.files.len() >= LIMITS.files
        {
            return;
        }
        let Some(extractor) =
            languages::detect_id(relative, None).and_then(|id| extractors.get(id))
        else {
            return;
        };
        self.source_bytes += text.len_bytes();
        let source: std::borrow::Cow<'_, str> = text.slice(..).into();
        if let Some(declarations) = extractor.extract(source.as_bytes(), cancelled) {
            self.insert(
                prior.unwrap_or_else(|| Arc::from(relative)),
                FileSymbols::Buffer(declarations.into()),
            );
        }
    }

    /// Decide whether `line` (1-based) lies inside a declaration of one
    /// of `kinds`. `None` is Unknown: no entry, incomplete extraction,
    /// or a surface without line numbers.
    pub fn kind_decides(
        &self,
        relative: impl AsRef<Path>,
        line: Option<usize>,
        kinds: &[SymbolKind],
    ) -> Option<bool> {
        let entry = self.files.get(relative.as_ref())?;
        let line = line?;
        Some(entry.declarations().iter().any(|declaration| {
            let (first, last) = declaration.lines();
            kinds.contains(&declaration.kind) && first <= line && line <= last
        }))
    }

    /// Honest coverage (0063 §2): some eligible file contributed no
    /// evidence — bounds, unreadable, or a failed parse. True means
    /// the list is knowingly incomplete, never silently empty.
    pub fn coverage_gap(&self) -> bool {
        self.interrupted || self.files.len() < self.eligible
    }

    /// Iterate every complete entry, path first: `(relative path,
    /// declarations)`. Incomplete entries are absent — no evidence.
    pub fn iter(&self) -> impl Iterator<Item = (&Path, &[Declaration])> {
        self.files
            .iter()
            .map(|(path, entry)| (path.as_ref(), entry.declarations()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn decides_from_complete_evidence() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/lib.rs",
            "fn hello() {\n    let x = 1;\n}\nstruct St;\n",
        );
        write(dir.path(), "src/plain.txt", "fn not rust\n");
        let paths = vec![
            std::path::PathBuf::from("src/lib.rs"),
            std::path::PathBuf::from("src/plain.txt"),
        ];
        let index = SymbolIndex::build(dir.path(), &paths, &|| false);
        let function = &[SymbolKind::Function, SymbolKind::Method];
        // Inside the fn body: Yes; struct line: No (complete evidence).
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(2), function),
            Some(true)
        );
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(4), function),
            Some(false)
        );
        // Struct kind on the struct line: Yes.
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(4), &[SymbolKind::Struct]),
            Some(true)
        );
        // Unsupported language: absent → Unknown.
        assert_eq!(index.kind_decides("src/plain.txt", Some(1), function), None);
        // Never indexed: Unknown.
        assert_eq!(index.kind_decides("src/other.rs", Some(1), function), None);
        // No line number (line-less surface): Unknown.
        assert_eq!(index.kind_decides("src/lib.rs", None, function), None);
    }

    #[test]
    fn kind_impl_decides_lines_inside_an_impl_block() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/lib.rs",
            "struct St;\nimpl St {\n    fn f(&self) {}\n}\nfn g() {}\n",
        );
        let paths = vec![std::path::PathBuf::from("src/lib.rs")];
        let index = SymbolIndex::build(dir.path(), &paths, &|| false);
        let kinds = &[SymbolKind::Impl];
        assert_eq!(index.kind_decides("src/lib.rs", Some(3), kinds), Some(true));
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(5), kinds),
            Some(false)
        );
    }

    #[test]
    fn overlay_replaces_disk_evidence() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "fn disk_only() {\n}\n");
        let paths = vec![std::path::PathBuf::from("src/lib.rs")];
        let mut index = SymbolIndex::build(dir.path(), &paths, &|| false);
        let baseline = index.clone();
        index.overlay(
            Path::new("src/lib.rs"),
            &ropey::Rope::from_str("fn dirty() {\n    let y = 2;\n}\n"),
            &mut Extractors::default(),
            &|| false,
        );
        assert!(index.disk_shard(&paths[0]).is_none());
        assert_eq!(
            baseline.disk_shard(&paths[0]).unwrap().declarations()[0].name,
            "disk_only"
        );
        let function = &[SymbolKind::Function, SymbolKind::Method];
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(2), function),
            Some(true)
        );
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(4), function),
            Some(false),
            "the dirty text is authoritative: past its end is complete evidence of no span"
        );
    }

    #[test]
    fn file_bound_leaves_rest_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let big = "fn f() {}\n".repeat(300 * 1024);
        write(dir.path(), "big.rs", &big);
        let paths = vec![std::path::PathBuf::from("big.rs")];
        let index = SymbolIndex::build(dir.path(), &paths, &|| false);
        assert!(index.coverage_gap());
        assert_eq!(
            index.kind_decides("big.rs", Some(1), &[SymbolKind::Function]),
            None,
            "a source past the per-file bound stays Unknown"
        );
    }

    #[test]
    fn cancelled_build_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "fn hello() {}\n");
        let paths = vec![std::path::PathBuf::from("src/lib.rs")];
        let index = SymbolIndex::build(dir.path(), &paths, &|| true);
        assert!(index.coverage_gap());
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(1), &[SymbolKind::Function]),
            None
        );
    }

    #[test]
    fn same_length_restored_mtime_is_not_a_byte_witness() {
        let dir = tempfile::tempdir().unwrap();
        let path = PathBuf::from("lib.rs");
        let paths = std::slice::from_ref(&path);
        let absolute = dir.path().join(&path);
        let before = b"fn first() {}\n";
        let after = b"fn other() {}\n";
        std::fs::write(&absolute, before).unwrap();
        let mtime = std::fs::metadata(&absolute).unwrap().modified().unwrap();
        let initial = SymbolIndex::build(dir.path(), paths, &|| false);
        let (unchanged, stats) =
            SymbolIndex::refresh(Some(&initial), dir.path(), paths, &[], &|| false);
        let shard = initial.disk_shard(&path).unwrap();
        assert!(Arc::ptr_eq(shard, unchanged.disk_shard(&path).unwrap()));
        assert_eq!(
            stats,
            ScanStats {
                parsed: 0,
                reused: 1
            }
        );
        std::fs::write(&absolute, after).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&absolute)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        assert_eq!(
            std::fs::metadata(&absolute).unwrap().modified().unwrap(),
            mtime
        );
        let (changed, stats) =
            SymbolIndex::refresh(Some(&unchanged), dir.path(), paths, &[], &|| false);
        assert_eq!(
            stats,
            ScanStats {
                parsed: 1,
                reused: 0
            }
        );
        let new = changed.disk_shard(&path).unwrap();
        assert_eq!(shard.source(), before);
        assert_eq!(new.source(), after);
        assert_eq!(
            new.schema(),
            DeclarationSchema::ByteRangesV2(languages::LanguageId::Rust)
        );
        assert_eq!(
            new.declarations(),
            SymbolIndex::build(dir.path(), paths, &|| false)
                .disk_shard(&path)
                .unwrap()
                .declarations()
        );
        assert_eq!(new.declarations()[0].name, "other");
    }

    #[test]
    fn rename_and_recreate_retire_old_shards() {
        let dir = tempfile::tempdir().unwrap();
        let old = PathBuf::from("a/lib.rs");
        let new = PathBuf::from("b/lib.rs");
        write(dir.path(), "a/lib.rs", "fn old() {}\n");
        let prior = SymbolIndex::build(dir.path(), std::slice::from_ref(&old), &|| false);
        std::fs::rename(dir.path().join("a"), dir.path().join("b")).unwrap();
        let (moved, stats) = SymbolIndex::refresh(
            Some(&prior),
            dir.path(),
            std::slice::from_ref(&new),
            &[],
            &|| false,
        );
        assert_eq!(stats.parsed, 1);
        assert!(moved.disk_shard(&old).is_none());
        assert_eq!(
            moved.disk_shard(&new).unwrap().declarations()[0].name,
            "old"
        );
        write(dir.path(), "a/lib.rs", "fn old() {}\n");
        let (recreated, stats) = SymbolIndex::refresh(
            Some(&prior),
            dir.path(),
            std::slice::from_ref(&old),
            &[PathBuf::from("a")],
            &|| false,
        );
        assert_eq!(stats.reused, 0);
        assert!(!Arc::ptr_eq(
            prior.disk_shard(&old).unwrap(),
            recreated.disk_shard(&old).unwrap()
        ));
    }

    #[test]
    fn missing_files_and_exhausted_refresh_budgets_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let paths = [PathBuf::from("a.rs"), PathBuf::from("b.rs")];
        let text = "fn name() {}\n";
        for path in &paths {
            std::fs::write(dir.path().join(path), text).unwrap();
        }
        let prior = SymbolIndex::build(dir.path(), &paths, &|| false);
        assert!(!prior.coverage_gap());
        assert!(prior.retained_bytes() >= 2 * (text.len() + size_of::<Declaration>()));
        for limits in [
            Limits { files: 1, ..LIMITS },
            Limits {
                total_bytes: text.len(),
                ..LIMITS
            },
            Limits {
                total_bytes: 2 * text.len() - 1,
                ..LIMITS
            },
        ] {
            let (capped, stats) =
                SymbolIndex::scan(dir.path(), &paths, Some(&prior), &[], &|| false, limits);
            assert!(capped.coverage_gap());
            assert_eq!(
                stats,
                ScanStats {
                    parsed: 0,
                    reused: 1
                }
            );
            assert!(capped.disk_shard(&paths[1]).is_none());
            assert!(capped.source_bytes <= limits.total_bytes);
        }
        std::fs::remove_file(dir.path().join(&paths[1])).unwrap();
        let (missing, _) = SymbolIndex::refresh(Some(&prior), dir.path(), &paths, &[], &|| false);
        assert!(missing.coverage_gap());
        assert_eq!(
            missing.kind_decides(&paths[1], Some(1), &[SymbolKind::Function]),
            None
        );
    }
}
