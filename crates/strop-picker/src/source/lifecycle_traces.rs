//! 0063 §6.6 production correspondence: SearchLifecycle.tla's
//! EditBuffer → Accept trace replayed through the actual symbol-evidence
//! seam the spec names (`SymbolIndex::overlay`, `kind_decides`) — the
//! dirty-source revision bump and the acceptance re-check that keeps
//! StaleAcceptsNever (no known-stale symbol acceptance). The revision
//! decision itself is the verified kernel's
//! (`strop_core::searchguard::revision_is_current`).

#[cfg(test)]
mod tests {
    use super::super::symbols::SymbolIndex;
    use strop_syntax::symbols::SymbolKind;

    /// Trace: extract rows against the disk revision, bump the source
    /// revision (a dirty buffer overlays the path), then re-check
    /// acceptance. Invariant: StaleAcceptsNever — the answer extracted
    /// against the moved source is never consumed.
    #[test]
    fn edit_buffer_retires_disk_revision_evidence() {
        let dir = std::env::temp_dir().join("strop-picker-lifecycle-trace");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn disk_fn() {}\n").unwrap();
        // Extraction against revision 0 (the disk pass).
        let mut index = SymbolIndex::build(&dir, &["src/lib.rs".into()], &|| false);
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(1), &[SymbolKind::Function]),
            Some(true),
            "the disk revision's evidence answers"
        );
        assert!(
            strop_core::searchguard::revision_is_current(0, 0),
            "kernel: the observed revision accepts its own rows"
        );
        // EditBuffer: the dirty text is authoritative — the overlay
        // replaces whatever the disk pass extracted.
        index.overlay("src/lib.rs", "struct Dirty;\n", &|| false);
        // Accept re-check at the bumped revision: the stale disk answer
        // (a function at line 1) is gone; only current evidence decides.
        assert!(
            !strop_core::searchguard::revision_is_current(0, 1),
            "kernel: a moved source is never current"
        );
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(1), &[SymbolKind::Function]),
            Some(false),
            "StaleAcceptsNever: the pre-edit answer is never consumed"
        );
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(1), &[SymbolKind::Struct]),
            Some(true),
            "the overlaid revision's evidence answers"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Negative trace: a revision bump whose extraction has no evidence
    /// (cancellation) must refuse to decide — Unknown admits, never a
    /// stale or fabricated answer.
    #[test]
    fn incomplete_overlay_never_decides() {
        let dir = std::env::temp_dir().join("strop-picker-lifecycle-trace-cancel");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn disk_fn() {}\n").unwrap();
        let mut index = SymbolIndex::build(&dir, &["src/lib.rs".into()], &|| false);
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(1), &[SymbolKind::Function]),
            Some(true)
        );
        // EditBuffer with the extraction cancelled mid-flight: the
        // overlay records no evidence for the new revision.
        index.overlay("src/lib.rs", "fn dirty_fn() {}\n", &|| true);
        assert_eq!(
            index.kind_decides("src/lib.rs", Some(1), &[SymbolKind::Function]),
            None,
            "StaleAcceptsNever: incomplete evidence is Unknown — it admits, never claims"
        );
        assert!(
            index.iter().all(|(path, _)| path != "src/lib.rs"),
            "the incomplete entry never publishes as complete evidence"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
