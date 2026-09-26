//! Same-source deployment admission kernels (0058 WK18/WK19). The digest
//! gate, lease-aware retirement and staged-activation admission executed
//! by `strop-worker-deploy` call these exact decisions; remote/provider
//! observations are caller premises the kernels never re-derive.

use vstd::prelude::*;

verus! {

/// One lowercase hex digit: `0`-`9` or `a`-`f`.
pub open spec fn hex_digit(byte: u8) -> bool {
    (0x30 <= byte <= 0x39) || (0x61 <= byte <= 0x66)
}
/// Executable twin of [`hex_digit`]: one lowercase hex digit.
pub fn hex_digit_exec(byte: u8) -> (hex: bool)
    ensures hex == hex_digit(byte),
{
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

/// The 64-character lowercase hexadecimal content address that the
/// verified cache, GC and receipts all share.
pub open spec fn is_hex_address(bytes: &[u8]) -> bool {
    bytes.len() == 64
        && forall|i: int| 0 <= i < bytes.len() ==> #[trigger] hex_digit(bytes[i])
}

/// Executable: the shipped `is_content_address` decision, lifted so the
/// cache, GC and receipts prove against one source.
pub fn is_content_address(bytes: &[u8]) -> (address: bool)
    ensures address == is_hex_address(bytes),
{
    if bytes.len() != 64 {
        return false;
    }
    let mut all_hex = true;
    let mut i = 0;
    while i < bytes.len()
        invariant
            0 <= i <= bytes.len(),
            all_hex == (forall|j: int| 0 <= j < i ==> #[trigger] hex_digit(bytes[j])),
        decreases bytes.len() - i,
    {
        all_hex = all_hex && hex_digit_exec(bytes[i]);
        i += 1;
    }
    all_hex
}

/// An object is pinned when it is the client's current executable or any
/// live lease references it (a live worker holds its lease for its whole
/// lifetime).
pub open spec fn object_pinned(is_current: bool, live_leases: u64) -> bool {
    is_current || live_leases > 0
}

/// The GC keep decision: retire only unpinned objects.
pub fn keep_object(is_current: bool, live_leases: u64) -> (keep: bool)
    ensures keep == object_pinned(is_current, live_leases),
{
    is_current || live_leases > 0
}

/// The load-bearing retirement theorem: a live lease's artifact is never
/// retired, no matter what the current-object pointer says.
proof fn live_lease_object_survives(is_current: bool, live_leases: u64)
    requires live_leases > 0,
    ensures object_pinned(is_current, live_leases),
{
}

/// Staged-activation admission: an object activates only when it is
/// published at its final path, the observed bytes still match the
/// expected digest, and a receipt exists. The provider's lstat/fetch
/// observations are premises; this kernel is the decision the state
/// machine consults before launching.
pub open spec fn activation_permits(
    published: bool,
    digest_matches: bool,
    receipted: bool,
) -> bool {
    published && digest_matches && receipted
}

pub fn activation_admitted(
    published: bool,
    digest_matches: bool,
    receipted: bool,
) -> (admitted: bool)
    ensures admitted == activation_permits(published, digest_matches, receipted),
{
    published && digest_matches && receipted
}

/// An unpublish or digest drift observed before activation is a truthful
/// refusal, never a launch of unverified bytes.
proof fn unverified_object_never_activates(
    published: bool,
    digest_matches: bool,
    receipted: bool,
)
    requires !published || !digest_matches,
    ensures !activation_permits(published, digest_matches, receipted),
{
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
) -> (verdict: CatalogVerdict)
    ensures
        (verdict == CatalogVerdict::Compatible)
            == (protocol_matches && version_matches && target_available),
        (verdict == CatalogVerdict::ProtocolMismatch) == !protocol_matches,
        (verdict == CatalogVerdict::VersionMismatch)
            == (protocol_matches && !version_matches),
        (verdict == CatalogVerdict::NoArtifactForTarget)
            == (protocol_matches && version_matches && !target_available),
{
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

} // verus!
