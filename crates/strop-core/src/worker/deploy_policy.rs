//! Deployment admission decisions (0058 WK18/WK19). The digest
//! gate, lease-aware retirement and staged-activation admission executed
//! by `strop-worker-deploy` call these exact decisions; remote/provider
//! observations are caller premises the kernels never re-derive.

/// Executable twin of [`hex_digit`]: one lowercase hex digit.
pub fn hex_digit_exec(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

/// Executable: the shipped `is_content_address` decision, lifted so the
/// cache, GC and receipts prove against one source.
pub fn is_content_address(bytes: &[u8]) -> bool {
    if bytes.len() != 64 {
        return false;
    }
    let mut all_hex = true;
    let mut i = 0;
    while i < bytes.len() {
        all_hex = all_hex && hex_digit_exec(bytes[i]);
        i += 1;
    }
    all_hex
}

/// The GC keep decision: retire only unpinned objects.
pub fn keep_object(is_current: bool, live_leases: u64) -> bool {
    is_current || live_leases > 0
}

pub fn activation_admitted(published: bool, digest_matches: bool, receipted: bool) -> bool {
    published && digest_matches && receipted
}

/// Catalog admission order: a compatible artifact requires matching
/// wire protocol, matching release and an artifact for the exact target.
/// First mismatch wins, mirroring `ReleaseCatalog::compatibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogVerdict {
    Compatible,
    ProtocolMismatch,
    VersionMismatch,
    NoArtifactForTarget,
}

pub fn catalog_verdict(
    protocol_matches: bool,
    version_matches: bool,
    target_available: bool,
) -> CatalogVerdict {
    if !protocol_matches {
        CatalogVerdict::ProtocolMismatch
    } else if !version_matches {
        CatalogVerdict::VersionMismatch
    } else if target_available {
        CatalogVerdict::Compatible
    } else {
        CatalogVerdict::NoArtifactForTarget
    }
}
