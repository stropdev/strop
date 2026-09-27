//! Interrupted-deploy matrix (WK06 acceptance): a provider failure at
//! every state cleans up only positively owned staging, retains old
//! entries, and never partially activates.

mod common;

use common::{catalog, local_binary, sha256_hex, FakeProvider};
use strop_worker_deploy::cache::CacheLayout;
use strop_worker_deploy::deploy::{
    deploy, ArtifactSupply, Consent, DeployOrigin, DeployOutcome, DeployRefusal, DeployRequest,
};
use strop_worker_deploy::provider::{DeployProvider, ProviderError};

const VERSION: &str = "0.35.0";

fn request(local: std::path::PathBuf) -> DeployRequest {
    DeployRequest {
        catalog: catalog(VERSION),
        editor_version: VERSION.to_string(),
        consent: Consent::Granted {
            action: "open-remote-workspace".to_string(),
        },
        supply: ArtifactSupply::LocalBinary { path: local },
        online: true,
    }
}

fn layout(provider: &FakeProvider) -> CacheLayout {
    strop_worker_deploy::cache::resolve(provider).expect("cache resolves")
}

/// Seed an old worker from a previous version, as a concurrent/previous
/// install would leave it.
struct OldEntry {
    object_path: String,
    receipt_path: String,
}

fn seed_old_entry(provider: &FakeProvider, layout: &CacheLayout) -> OldEntry {
    let old_sha = sha256_hex(b"old worker 0.34.0");
    provider
        .write(&layout.object(&old_sha), b"old worker 0.34.0")
        .unwrap();
    provider.set_mode(&layout.object(&old_sha), 0o500).unwrap();
    provider
        .write(
            &layout.receipt(&old_sha),
            br#"{"schema":1,"context":"dev-box","principal":"alice","version":"0.34.0","target":"x86_64-unknown-linux-musl","object_sha256":""}"#,
        )
        .unwrap();
    OldEntry {
        object_path: layout.object(&old_sha),
        receipt_path: layout.receipt(&old_sha),
    }
}

fn assert_old_intact(provider: &FakeProvider, old: &OldEntry) {
    assert!(
        provider.entry(&old.object_path).is_some(),
        "old object retired by an unrelated failure"
    );
    assert!(
        provider.entry(&old.receipt_path).is_some(),
        "old receipt retired by an unrelated failure"
    );
}

fn assert_no_partial_activation(provider: &FakeProvider, layout: &CacheLayout) {
    assert_eq!(
        provider.names(&layout.staging_dir()),
        Vec::<String>::new(),
        "staging left behind"
    );
    assert_eq!(
        provider.names(&layout.leases_dir()),
        Vec::<String>::new(),
        "lease registered without readiness"
    );
}

#[test]
fn upload_failure_leaves_no_trace() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    let old = seed_old_entry(&provider, &layout);
    provider.failures.borrow_mut().upload = Some(ProviderError::DiskFull("quota".into()));

    let dir = tempfile::tempdir().unwrap();
    let report = deploy(&provider, &request(local_binary(&dir, VERSION)));
    match report.outcome {
        DeployOutcome::Refused(DeployRefusal::Transfer { state, error }) => {
            assert_eq!(state, strop_worker_deploy::deploy::State::Upload);
            assert!(matches!(error, ProviderError::DiskFull(_)));
        }
        other => panic!("expected Transfer refusal, got {other:?}"),
    }
    assert_old_intact(&provider, &old);
    assert_no_partial_activation(&provider, &layout);
    assert_eq!(
        provider.names(&layout.objects_dir()),
        vec![sha256_hex(b"old worker 0.34.0")]
    );
}

#[test]
fn corrupt_transfer_never_publishes() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    provider.failures.borrow_mut().corrupt_upload = true;

    let dir = tempfile::tempdir().unwrap();
    let report = deploy(&provider, &request(local_binary(&dir, VERSION)));
    assert!(matches!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::CorruptTransfer { .. })
    ));
    assert_no_partial_activation(&provider, &layout);
    assert_eq!(provider.names(&layout.objects_dir()), Vec::<String>::new());
}

#[test]
fn rename_failure_retains_old_entry_and_drops_staging() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    let old = seed_old_entry(&provider, &layout);
    provider.failures.borrow_mut().rename = Some(ProviderError::ReadOnly("ro mount".into()));

    let dir = tempfile::tempdir().unwrap();
    let report = deploy(&provider, &request(local_binary(&dir, VERSION)));
    match report.outcome {
        DeployOutcome::Refused(DeployRefusal::Transfer { state, error }) => {
            assert_eq!(state, strop_worker_deploy::deploy::State::Publish);
            assert!(matches!(error, ProviderError::ReadOnly(_)));
        }
        other => panic!("expected Transfer refusal, got {other:?}"),
    }
    assert_old_intact(&provider, &old);
    assert_no_partial_activation(&provider, &layout);
}

#[test]
fn receipt_failure_publishes_but_never_claims_ready() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    provider.failures.borrow_mut().write = Some(ProviderError::Transport("dropped".into()));

    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let sha = sha256_hex(&std::fs::read(&local).unwrap());
    let report = deploy(&provider, &request(local));
    assert!(matches!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::Transfer { .. })
    ));
    // The verified object may stay cached (immutable, harmless); no
    // lease, no readiness, staging clean.
    assert_no_partial_activation(&provider, &layout);
    assert!(provider.entry(&layout.object(&sha)).is_some());
}

#[test]
fn lost_receipt_after_successful_write_never_activates() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    provider.failures.borrow_mut().lose_receipt_after_write = true;
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let sha = sha256_hex(&std::fs::read(&local).unwrap());

    let report = deploy(&provider, &request(local));
    assert!(matches!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::Transfer {
            state: strop_worker_deploy::deploy::State::WriteReceipt,
            ..
        })
    ));
    assert_eq!(
        report.trace.last(),
        Some(&strop_worker_deploy::deploy::State::WriteReceipt)
    );
    assert!(provider.entry(&layout.object(&sha)).is_some());
    assert!(provider.entry(&layout.receipt(&sha)).is_none());
    assert_no_partial_activation(&provider, &layout);
}

#[test]
fn reused_object_reissues_receipt_for_exact_version_and_target() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let binary = std::fs::read(&local).unwrap();
    let sha = sha256_hex(&binary);
    let object_bytes = binary.len() as u64;
    let stale = strop_core::worker::cache_record::CacheReceipt {
        schema: strop_core::worker::cache_record::RECEIPT_SCHEMA,
        context: provider.endpoint.context.clone(),
        principal: provider.endpoint.principal.clone(),
        version: "old-release".into(),
        target: provider.endpoint.target.clone(),
        object_sha256: sha.clone(),
        object_bytes: 0,
        tarball_sha256: "wrong-provenance".into(),
    };
    provider
        .write(&layout.receipt(&sha), &serde_json::to_vec(&stale).unwrap())
        .unwrap();

    let request = request(local);
    let report = deploy(&provider, &request);
    assert!(
        matches!(report.outcome, DeployOutcome::Probed(_)),
        "{report:?}"
    );
    let actual = strop_worker_deploy::cache::read_receipt(&provider, &layout, &sha)
        .unwrap()
        .unwrap();
    assert_eq!(actual.version, VERSION);
    assert_eq!(actual.context, provider.endpoint.context);
    assert_eq!(actual.principal, provider.endpoint.principal);
    assert_eq!(actual.target, provider.endpoint.target);
    assert_eq!(actual.object_sha256, sha);
    assert_eq!(actual.object_bytes, object_bytes);
    assert_eq!(actual.tarball_sha256, request.catalog.artifacts[0].sha256);
}

#[test]
fn handshake_failure_is_published_not_ready() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    provider.failures.borrow_mut().handshake = Some(ProviderError::NoExec("noexec mount".into()));

    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let sha = sha256_hex(&std::fs::read(&local).unwrap());
    let report = deploy(&provider, &request(local));
    match report.outcome {
        DeployOutcome::PublishedNotReady {
            object,
            origin,
            reason,
        } => {
            assert_eq!(object.sha256, sha);
            assert_eq!(origin, DeployOrigin::Uploaded);
            assert!(reason.contains("launch refused"));
        }
        other => panic!("expected PublishedNotReady, got {other:?}"),
    }
    // The verified object and its receipt stay for a later attempt;
    // no lease, staging clean.
    assert!(provider.entry(&layout.object(&sha)).is_some());
    assert!(provider.entry(&layout.receipt(&sha)).is_some());
    assert_no_partial_activation(&provider, &layout);
}

#[test]
fn probe_lease_cleanup_failure_never_claims_a_live_worker() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    provider.failures.borrow_mut().remove =
        Some(ProviderError::Transport("cannot retire the probe".into()));

    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let sha = sha256_hex(&std::fs::read(&local).unwrap());
    let report = deploy(&provider, &request(local));
    assert!(matches!(
        report.outcome,
        DeployOutcome::PublishedNotReady {
            origin: DeployOrigin::Uploaded,
            ..
        }
    ));
    assert!(provider.entry(&layout.object(&sha)).is_some());
    assert!(provider.entry(&layout.receipt(&sha)).is_some());
    assert!(provider.entry(&layout.lease(7)).is_some());
    assert!(provider.names(&layout.staging_dir()).is_empty());
}
