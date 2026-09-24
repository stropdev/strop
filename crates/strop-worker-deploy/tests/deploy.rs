//! Deploy state machine: consent gating, cache reuse, preinstalled
//! validation, offline/truthful refusal and fallback decisions (hermetic,
//! fake provider).

mod common;

use common::{catalog, local_binary, sha256_hex, Entry, FakeProvider};
use strop_worker_deploy::cache::{CacheLayout, RECEIPT_SCHEMA};
use strop_worker_deploy::deploy::{
    deploy, ArtifactSupply, Consent, DeployOrigin, DeployOutcome, DeployRefusal, DeployRequest,
};
use strop_worker_deploy::manifest::Fallback;
use strop_worker_deploy::provider::DeployProvider;
use strop_worker_deploy::MAX_WORKER_BYTES;

const VERSION: &str = "0.35.0";

fn request(supply: ArtifactSupply, consent: Consent) -> DeployRequest {
    DeployRequest {
        catalog: catalog(VERSION),
        editor_version: VERSION.to_string(),
        consent,
        supply,
        online: true,
    }
}

fn granted() -> Consent {
    Consent::Granted {
        action: "open-remote-workspace".to_string(),
    }
}

fn layout(provider: &FakeProvider) -> CacheLayout {
    strop_worker_deploy::cache::resolve(provider).expect("cache resolves")
}

fn ready(outcome: DeployOutcome) -> (DeployOrigin, String) {
    match outcome {
        DeployOutcome::Ready(ready) => (ready.origin, ready.object.sha256),
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[test]
fn consent_gated_first_deploy_publishes_and_activates() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let sha = sha256_hex(&std::fs::read(&local).unwrap());

    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: local }, granted()),
    );
    let (origin, object_sha) = ready(report.outcome);
    assert_eq!(origin, DeployOrigin::Uploaded);
    assert_eq!(object_sha, sha);

    // The published object is content-addressed, private and executable;
    // the receipt binds provenance; the lease is registered; staging is
    // empty.
    let layout = layout(&provider);
    let object = provider.entry(&layout.object(&sha)).expect("object");
    assert_eq!(object.mode, 0o500);
    assert_eq!(object.owner, "alice");
    let receipt = provider.entry(&layout.receipt(&sha)).expect("receipt");
    let receipt: serde_json::Value = serde_json::from_slice(&receipt.bytes).unwrap();
    assert_eq!(receipt["schema"], RECEIPT_SCHEMA);
    assert_eq!(receipt["context"], "dev-box");
    assert_eq!(receipt["principal"], "alice");
    assert_eq!(receipt["version"], VERSION);
    assert_eq!(receipt["tarball_sha256"], "cccc");
    assert_eq!(provider.names(&layout.staging_dir()), Vec::<String>::new());
    assert_eq!(
        provider.names(&layout.leases_dir()),
        vec!["7.json".to_string()]
    );
}

#[test]
fn first_deploy_without_consent_refuses_with_precise_reason() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);

    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: local }, Consent::Absent),
    );
    match report.outcome {
        DeployOutcome::Refused(DeployRefusal::ConsentRequired {
            context,
            principal,
            version,
            target,
            destination,
        }) => {
            assert_eq!((context, principal), ("dev-box".into(), "alice".into()));
            assert_eq!(version, VERSION);
            assert_eq!(target, "x86_64-unknown-linux-musl");
            assert!(destination.contains("/objects/"));
        }
        other => panic!("expected ConsentRequired, got {other:?}"),
    }
    // Nothing uploaded, nothing published, nothing staged.
    assert_eq!(provider.counts.borrow().upload, 0);
    let layout = layout(&provider);
    assert_eq!(provider.names(&layout.objects_dir()), Vec::<String>::new());
    assert_eq!(provider.names(&layout.staging_dir()), Vec::<String>::new());
}

#[test]
fn previously_authorized_endpoint_reuses_authorization_quietly() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);

    // A prior authorized deployment left a receipt for this endpoint.
    let first = deploy(
        &provider,
        &request(
            ArtifactSupply::LocalBinary {
                path: local.clone(),
            },
            granted(),
        ),
    );
    assert!(matches!(first.outcome, DeployOutcome::Ready(_)));

    // A new version deploys without fresh consent: the receipt authorizes.
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    provider
        .write(
            &layout.receipt(&"f".repeat(64)),
            br#"{"schema":1,"context":"dev-box","principal":"alice","version":"0.35.0","target":"x86_64-unknown-linux-musl","object_sha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","object_bytes":1,"tarball_sha256":"aa"}"#,
        )
        .unwrap();
    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: local }, Consent::Absent),
    );
    assert_eq!(ready(report.outcome).0, DeployOrigin::Uploaded);
}

#[test]
fn exact_verified_cache_hit_is_reused_without_upload() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let bytes = std::fs::read(&local).unwrap();
    let sha = sha256_hex(&bytes);

    let layout = layout(&provider);
    provider.write(&layout.object(&sha), &bytes).unwrap();
    provider.set_mode(&layout.object(&sha), 0o500).unwrap();

    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: local }, Consent::Absent),
    );
    let (origin, object_sha) = ready(report.outcome);
    assert_eq!(origin, DeployOrigin::Reused);
    assert_eq!(object_sha, sha);
    assert_eq!(provider.counts.borrow().upload, 0);
}

#[test]
fn corrupt_cache_object_is_retired_and_redeployed() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let dir = tempfile::tempdir().unwrap();
    let local = local_binary(&dir, VERSION);
    let bytes = std::fs::read(&local).unwrap();
    let sha = sha256_hex(&bytes);

    let layout = layout(&provider);
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 0xff;
    provider.write(&layout.object(&sha), &corrupt).unwrap();
    provider.set_mode(&layout.object(&sha), 0o500).unwrap();

    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: local }, granted()),
    );
    let (origin, object_sha) = ready(report.outcome);
    assert_eq!(origin, DeployOrigin::Uploaded);
    assert_eq!(object_sha, sha);
    let object = provider.entry(&layout.object(&sha)).expect("object");
    assert_eq!(object.bytes, bytes);
}

#[test]
fn offline_without_artifact_is_a_truthful_refusal() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let mut offline = request(ArtifactSupply::None, granted());
    offline.online = false;
    let report = deploy(&provider, &offline);
    assert_eq!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::OfflineNoArtifact {
            version: VERSION.to_string(),
            target: "x86_64-unknown-linux-musl".to_string(),
        })
    );
    // No install attempt of any kind touched the endpoint.
    assert_eq!(provider.counts.borrow().upload, 0);
    assert_eq!(
        report.trace,
        vec![strop_worker_deploy::deploy::State::Decide]
    );
}

#[test]
fn online_without_local_artifact_refuses_and_points_at_the_pipeline() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let report = deploy(&provider, &request(ArtifactSupply::None, granted()));
    assert!(matches!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::ArtifactNotStaged { .. })
    ));
    assert_eq!(provider.counts.borrow().upload, 0);
}

#[test]
fn endpoint_without_worker_artifact_falls_back_honestly() {
    let mut provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    provider.endpoint.target = "riscv64gc-unknown-none-elf".to_string();
    let report = deploy(
        &provider,
        &request(
            ArtifactSupply::LocalBinary {
                path: std::path::PathBuf::from("/nonexistent"),
            },
            granted(),
        ),
    );
    assert_eq!(
        report.outcome,
        DeployOutcome::Fallback(Fallback::NoArtifactForTarget {
            target: "riscv64gc-unknown-none-elf".to_string()
        })
    );
}

#[test]
fn preinstalled_validates_and_activates_in_place() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    provider
        .write("/opt/strop/worker", b"admin provisioned worker")
        .unwrap();
    provider.set_mode("/opt/strop/worker", 0o755).unwrap();

    let report = deploy(
        &provider,
        &request(
            ArtifactSupply::Preinstalled {
                path: "/opt/strop/worker".to_string(),
            },
            Consent::Absent,
        ),
    );
    assert_eq!(ready(report.outcome).0, DeployOrigin::Preinstalled);
    // No cache writes at all: the override never manufactures a cache.
    assert!(provider.entry("/home/alice/.cache/strop-worker").is_none());
}

#[test]
fn invalid_preinstalled_override_never_chooses_another_file() {
    // Symlink: a replaceable path.
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    provider
        .entries
        .borrow_mut()
        .insert("/opt/strop/worker".to_string(), Entry::symlink("root"));
    let report = deploy(
        &provider,
        &request(
            ArtifactSupply::Preinstalled {
                path: "/opt/strop/worker".to_string(),
            },
            granted(),
        ),
    );
    assert!(matches!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::PreinstalledInvalid { .. })
    ));

    // Group/world-writable.
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    provider.write("/opt/strop/worker", b"bytes").unwrap();
    provider.set_mode("/opt/strop/worker", 0o777).unwrap();
    let report = deploy(
        &provider,
        &request(
            ArtifactSupply::Preinstalled {
                path: "/opt/strop/worker".to_string(),
            },
            granted(),
        ),
    );
    assert!(matches!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::PreinstalledInvalid { .. })
    ));
    assert_eq!(provider.counts.borrow().upload, 0);
}

#[test]
fn handshake_identity_mismatch_is_refusal_not_downgrade() {
    let mut handshake = FakeProvider::handshake_ok(VERSION);
    handshake.worker.version = "0.34.0".to_string();
    let provider = FakeProvider::new(handshake);
    provider.write("/opt/strop/worker", b"bytes").unwrap();
    provider.set_mode("/opt/strop/worker", 0o700).unwrap();
    let report = deploy(
        &provider,
        &request(
            ArtifactSupply::Preinstalled {
                path: "/opt/strop/worker".to_string(),
            },
            granted(),
        ),
    );
    assert_eq!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::IdentityMismatch {
            field: "version",
            expected: VERSION.to_string(),
            reported: "0.34.0".to_string(),
        })
    );
}

#[test]
fn oversized_local_artifact_is_refused_before_any_upload() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("strop");
    // Sparse file over the bound; no allocation needed.
    let file = std::fs::File::create(&local).unwrap();
    file.set_len(MAX_WORKER_BYTES + 1).unwrap();
    drop(file);
    let report = deploy(
        &provider,
        &request(ArtifactSupply::LocalBinary { path: local }, granted()),
    );
    assert!(matches!(
        report.outcome,
        DeployOutcome::Refused(DeployRefusal::Oversized { .. })
    ));
    assert_eq!(provider.counts.borrow().upload, 0);
}
