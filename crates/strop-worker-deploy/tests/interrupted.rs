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
fn lease_registration_failure_is_published_not_ready() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    // Only lease/receipt writes fail after the handshake succeeds... the
    // receipt write happens first, so fail selectively by path is more
    // than the fake supports; instead fail the write op after priming a
    // valid receipt via a completed first deploy.
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let first = deploy(&provider, &request(local.clone()));
    assert!(matches!(first.outcome, DeployOutcome::Ready(_)));
    // Remove the registered lease, fail writes, redeploy: the cache hit
    // path finds the receipt, reuses the object, then the lease write
    // fails.
    let leases = provider.names(&layout.leases_dir());
    for lease in leases {
        provider
            .remove(&format!("{}/{lease}", layout.leases_dir()))
            .unwrap();
    }
    provider.failures.borrow_mut().write = Some(ProviderError::Transport("dropped".into()));
    let report = deploy(&provider, &request(local));
    assert!(matches!(
        report.outcome,
        DeployOutcome::PublishedNotReady {
            origin: DeployOrigin::Reused,
            ..
        }
    ));
    assert_no_partial_activation(&provider, &layout);
}
