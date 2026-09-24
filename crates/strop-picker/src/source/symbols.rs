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
//! Incremental reuse (0063 residual, 0058 §2): [`SymbolIndex::refresh`]
//! reuses an entry only when the file's fresh stat matches the
//! observation recorded at parse time AND no notification hint
//! invalidated the path — observation self-heals missed hints, so
//! watching improves freshness but is never the only cache-validity
//! mechanism. Entries are `Arc`-shared: a per-search working copy is a
//! shallow clone, and the dirty-text overlay never mutates the retained
//! disk-pure baseline.
use std::collections::HashMap;
use std::path::Path;
use strop_syntax::languages;
use strop_syntax::symbols::{DeclExtractor, Declaration, SymbolKind};

/// Files whose extraction is attempted per search. Files beyond the
/// bound stay absent — Unknown, sound overfetch.
const FILE_LIMIT: usize = 4096;
/// Per-file source bound; larger sources stay absent.
const FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Total source bytes extracted per search.
const TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// One file's declarations. `complete` is false when extraction had no
/// evidence (parse failure or cancellation) — never a claim of
/// emptiness.
pub struct FileSymbols {
    declarations: Vec<Declaration>,
    complete: bool,
    /// The stat evidence (length, mtime) captured when the source was
    /// read for this extraction; refresh reuses the entry only while a
    /// fresh stat still matches it.
    observed: FileObservation,
}

/// The on-disk state an extraction was taken against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileObservation {
    len: u64,
    modified: Option<std::time::SystemTime>,
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
    files: HashMap<Box<str>, std::sync::Arc<FileSymbols>>,
    /// Files whose extension maps to an extractor language.
    eligible: usize,
    /// Eligible files with complete evidence (parse succeeded).
    covered: usize,
}

impl SymbolIndex {
    /// Extract declarations from the selection's eligible files. Paths
    /// are workspace-relative, matching admission's path vocabulary.
    pub fn build(cwd: &Path, paths: &[std::path::PathBuf], cancelled: &dyn Fn() -> bool) -> Self {
        Self::scan(cwd, paths, None, &[], cancelled).0
    }

    /// Refresh against the current selection: with `prior`, reuse an
    /// entry only while its recorded observation still matches a fresh
    /// stat and no hint invalidated the path (a relative path or a
    /// directory prefix of it); re-parse everything else. The
    /// self-healing stat check means a missed or coalesced hint costs
    /// one re-parse, never a stale answer.
    pub fn refresh(
        prior: Option<&SymbolIndex>,
        cwd: &Path,
        paths: &[std::path::PathBuf],
        invalidated: &[std::path::PathBuf],
        cancelled: &dyn Fn() -> bool,
    ) -> (Self, ScanStats) {
        Self::scan(cwd, paths, prior, invalidated, cancelled)
    }

    fn scan(
        cwd: &Path,
        paths: &[std::path::PathBuf],
        prior: Option<&SymbolIndex>,
        invalidated: &[std::path::PathBuf],
        cancelled: &dyn Fn() -> bool,
    ) -> (Self, ScanStats) {
        let mut index = Self {
            files: HashMap::new(),
            eligible: 0,
            covered: 0,
        };
        let mut stats = ScanStats::default();
        let mut extractors: HashMap<languages::LanguageId, DeclExtractor> = HashMap::new();
        let mut total = 0u64;
        for path in paths {
            if cancelled() || index.files.len() >= FILE_LIMIT || total >= TOTAL_BYTES {
                break;
            }
            let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let Some(spec) = languages::for_extension(extension) else {
                continue;
            };
            let Some(language) = DeclExtractor::for_language(spec.id).map(|_| spec.id) else {
                // Not extractor-eligible: out of the tier's declared
                // coverage, not a coverage gap.
                continue;
            };
            index.eligible += 1;
            let absolute = cwd.join(path);
            let Ok(metadata) = std::fs::metadata(&absolute) else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > FILE_BYTES {
                continue;
            }
            let observed = FileObservation {
                len: metadata.len(),
                modified: metadata.modified().ok(),
            };
            let key = path.to_string_lossy().into_owned().into_boxed_str();
            let hinted = invalidated.iter().any(|prefix| path.starts_with(prefix));
            if !hinted && !cancelled() {
                if let Some(entry) = prior.and_then(|prior| prior.files.get(&key)) {
                    if entry.observed == observed {
                        // Same bytes as the retained extraction —
                        // complete or not, the parse outcome is
                        // identical, so reuse keeps soundness.
                        index.covered += usize::from(entry.complete);
                        stats.reused += 1;
                        index.files.insert(key, std::sync::Arc::clone(entry));
                        continue;
                    }
                }
            }
            let extractor = match extractors.entry(language) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    match DeclExtractor::for_language(language) {
                        Some(extractor) => entry.insert(extractor),
                        None => continue,
                    }
                }
            };
            let Ok(source) = std::fs::read(&absolute) else {
                continue;
            };
            total = total.saturating_add(source.len() as u64);
            let declarations = extractor.extract(&source, cancelled);
            let complete = declarations.is_some();
            index.covered += usize::from(complete);
            stats.parsed += 1;
            index.files.insert(
                key,
                std::sync::Arc::new(FileSymbols {
                    declarations: declarations.unwrap_or_default(),
                    complete,
                    observed,
                }),
            );
        }
        (index, stats)
    }

    /// Overlay one unsaved source: its text is authoritative for that
    /// path, replacing whatever the disk pass extracted.
    pub fn overlay(&mut self, relative: &str, text: &str, cancelled: &dyn Fn() -> bool) {
        let Some(language) = languages::detect(Path::new(relative), None).map(|spec| spec.id)
        else {
            return;
        };
        let Some(mut extractor) = DeclExtractor::for_language(language) else {
            return;
        };
        let declarations = extractor.extract(text.as_bytes(), cancelled);
        let complete = declarations.is_some();
        self.files.insert(
            relative.to_owned().into_boxed_str(),
            std::sync::Arc::new(FileSymbols {
                declarations: declarations.unwrap_or_default(),
                complete,
                // Overlays live in per-search working copies only; the
                // sentinel observation is never a disk baseline.
                observed: FileObservation {
                    len: text.len() as u64,
                    modified: None,
                },
            }),
        );
    }

    /// Decide whether `line` (1-based) lies inside a declaration of one
    /// of `kinds`. `None` is Unknown: no entry, incomplete extraction,
    /// or a surface without line numbers.
    pub fn kind_decides(
        &self,
        relative: &str,
        line: Option<usize>,
        kinds: &[SymbolKind],
    ) -> Option<bool> {
        let entry = self.files.get(relative)?;
        if !entry.complete {
            return None;
        }
        let line = line?;
        Some(entry.declarations.iter().any(|declaration| {
            kinds.contains(&declaration.kind)
                && declaration.line <= line
                && line <= declaration.end_line
        }))
    }

    /// Honest coverage (0063 §2): some eligible file contributed no
    /// evidence — bounds, unreadable, or a failed parse. True means
    /// the list is knowingly incomplete, never silently empty.
    pub fn coverage_gap(&self) -> bool {
        self.covered < self.eligible
    }

    /// Iterate every complete entry, path first: `(relative path,
    /// declarations)`. Incomplete entries are absent — no evidence.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[Declaration])> {
        self.files
            .iter()
            .filter(|(_, entry)| entry.complete)
            .map(|(path, entry)| (path.as_ref(), entry.declarations.as_slice()))
    }

    /// `kind:` values the query language accepts (0063 §2/§3).
    /// `function` subsumes methods.
    pub fn kinds_for_value(value: &str) -> Option<&'static [SymbolKind]> {
        use SymbolKind::*;
        match value {
            "function" => Some(&[Function, Method]),
            "method" => Some(&[Method]),
            "class" => Some(&[Class]),
            "struct" => Some(&[Struct]),
            "enum" => Some(&[Enum]),
            "interface" => Some(&[Interface]),
            "module" => Some(&[Module]),
            "constant" => Some(&[Constant]),
            _ => None,
        }
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
    fn overlay_replaces_disk_evidence() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "fn disk_only() {\n}\n");
        let paths = vec![std::path::PathBuf::from("src/lib.rs")];
        let mut index = SymbolIndex::build(dir.path(), &paths, &|| false);
        index.overlay("src/lib.rs", "fn dirty() {\n    let y = 2;\n}\n", &|| false);
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
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(1), &[SymbolKind::Function]),
            None
        );
    }

    #[test]
    fn kinds_cover_methods_under_function() {
        assert!(SymbolIndex::kinds_for_value("function")
            .unwrap()
            .contains(&SymbolKind::Method));
        assert_eq!(
            SymbolIndex::kinds_for_value("method").unwrap(),
            &[SymbolKind::Method]
        );
        assert!(SymbolIndex::kinds_for_value("banana").is_none());
    }
}
