//! Lease-aware GC: a live lease's object and the caller's current object
//! survive; unreferenced content-addressed objects retire with their
//! receipts; stale lease records are swept; foreign entries are never
//! touched.

mod common;

use common::FakeProvider;
use strop_worker_deploy::cache::CacheLayout;
use strop_worker_deploy::gc::{collect, GcPolicy};
use strop_worker_deploy::provider::DeployProvider;

const VERSION: &str = "0.35.0";

fn layout(provider: &FakeProvider) -> CacheLayout {
    strop_worker_deploy::cache::resolve(provider).expect("cache resolves")
}

fn seed_object(provider: &FakeProvider, layout: &CacheLayout, sha: &str, bytes: &[u8]) {
    provider.write(&layout.object(sha), bytes).unwrap();
    provider.set_mode(&layout.object(sha), 0o500).unwrap();
    provider
        .write(
            &layout.receipt(sha),
            format!(
                r#"{{"schema":1,"context":"dev-box","principal":"alice","version":"{VERSION}","target":"x86_64-unknown-linux-musl","object_sha256":"{sha}","object_bytes":1,"tarball_sha256":"aa"}}"#
            )
            .as_bytes(),
        )
        .unwrap();
}

#[test]
fn gc_retires_only_unreferenced_objects() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    let current = "a".repeat(64);
    let leased = "b".repeat(64);
    let stale = "c".repeat(64);
    seed_object(&provider, &layout, &current, b"current worker");
    seed_object(&provider, &layout, &leased, b"leased worker");
    seed_object(&provider, &layout, &stale, b"stale worker");
    // Lease 7 is live and pins `leased`; lease 9 is stale.
    provider
        .write(
            &layout.lease(7),
            format!(r#"{{"lease":7,"object_sha256":"{leased}"}}"#).as_bytes(),
        )
        .unwrap();
    provider
        .write(
            &layout.lease(9),
            format!(r#"{{"lease":9,"object_sha256":"{stale}"}}"#).as_bytes(),
        )
        .unwrap();
    // A foreign entry that merely shares the directory.
    provider
        .write(&format!("{}/README", layout.objects_dir()), b"not ours")
        .unwrap();

    let report = collect(
        &provider,
        &layout,
        &GcPolicy {
            current_object: Some(current.clone()),
            live_leases: &[7],
        },
    )
    .expect("gc");

    assert_eq!(report.removed_objects, vec![stale.clone()]);
    assert_eq!(report.removed_receipts, 1);
    assert_eq!(report.removed_stale_leases, 1);
    assert_eq!(report.kept_objects, 2);
    assert!(provider.entry(&layout.object(&current)).is_some());
    assert!(provider.entry(&layout.object(&leased)).is_some());
    assert!(provider.entry(&layout.receipt(&leased)).is_some());
    assert!(provider.entry(&layout.object(&stale)).is_none());
    assert!(provider.entry(&layout.receipt(&stale)).is_none());
    // The live lease record stays; the foreign file is never touched.
    assert!(provider.entry(&layout.lease(7)).is_some());
    assert!(provider
        .entry(&format!("{}/README", layout.objects_dir()))
        .is_some());
}

#[test]
fn live_lease_artifact_is_never_retired_even_when_not_current() {
    let provider = FakeProvider::new(FakeProvider::handshake_ok(VERSION));
    let layout = layout(&provider);
    let leased = "d".repeat(64);
    seed_object(
        &provider,
        &layout,
        &leased,
        b"another editor version's worker",
    );
    provider
        .write(
            &layout.lease(42),
            format!(r#"{{"lease":42,"object_sha256":"{leased}"}}"#).as_bytes(),
        )
        .unwrap();

    // This client runs a different version; the concurrent client's live
    // lease still pins its object.
    let report = collect(
        &provider,
        &layout,
        &GcPolicy {
            current_object: None,
            live_leases: &[42],
        },
    )
    .expect("gc");
    assert!(report.removed_objects.is_empty());
    assert_eq!(report.kept_objects, 1);
    assert!(provider.entry(&layout.object(&leased)).is_some());
}
