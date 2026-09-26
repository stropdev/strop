use super::*;
use std::time::Instant;

/// The real OpenSSH ProxyCommand fixture supplies a selected principal,
/// SFTP upload, verified object/receipt and direct worker exec. A cold
/// repetition removes only this fixture's unleased object and receipt
/// outside the timer; neither the product nor its GC has a benchmark path.
#[test]
#[ignore = "native OpenSSH worker cold/warm latency; run explicitly with --ignored --nocapture"]
fn native_ssh_cold_warm_deploy_and_launch_samples() {
    let Some(_serial) = serial() else {
        panic!("native SSH benchmark requires STROP_REQUIRE_SSH_TESTS=1");
    };
    let fixture = fixture();
    let endpoint = endpoint("worker-benchmark");
    let (cancel, _owner) = token();
    let facts = discover(&endpoint, &cancel);
    let target = facts.local_binary_target().unwrap();
    let bytes = std::fs::metadata(&fixture.artifact).unwrap().len();
    let sha = &fixture.artifact_sha256;
    let supply = || ArtifactSupply::LocalBinary {
        path: fixture.artifact.clone(),
    };
    let consent = || Consent::Granted {
        action: "native-worker-latency-fixture".into(),
    };
    let warmup = 8;
    let iterations = 64;
    let mut cold_ms = Vec::with_capacity(iterations);
    let mut warm_ms = Vec::with_capacity(iterations);
    let mut launch_ms = Vec::with_capacity(iterations);
    let mut cached: Option<strop_worker_deploy::deploy::VerifiedObject> = None;
    for index in 0..(warmup + iterations) {
        if let Some(previous) = cached.take() {
            let admitted = provider(&endpoint, &facts, &cancel);
            let layout = strop_worker_deploy::cache::resolve(&admitted).unwrap();
            assert!(
                admitted.list(&layout.leases_dir()).unwrap().is_empty(),
                "the stopped deployment probe cannot pin the cache object"
            );
            admitted.remove(&layout.receipt(&previous.sha256)).unwrap();
            admitted.remove(&previous.path).unwrap();
            assert!(admitted.lstat(&previous.path).unwrap().is_none());
        }
        let started = Instant::now();
        let current = discover(&endpoint, &cancel);
        assert_eq!(current.principal, facts.principal);
        let admitted = provider(&endpoint, &current, &cancel);
        let report = deploy(&admitted, &request(target, consent(), supply(), sha, bytes));
        let elapsed = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0;
        let ready = match report.outcome {
            DeployOutcome::Probed(ready) => ready,
            other => panic!(
                "cold worker activation refused: {other:?} ({:?})",
                report.trace
            ),
        };
        assert_eq!(ready.origin, DeployOrigin::Uploaded);
        assert_eq!((&ready.object.sha256, ready.object.bytes), (sha, bytes));
        if index >= warmup {
            cold_ms.push(elapsed);
        }
        cached = Some(ready.object);
    }
    let deployed = cached.unwrap();
    for index in 0..(warmup + iterations) {
        let started = Instant::now();
        let current = discover(&endpoint, &cancel);
        let admitted = provider(&endpoint, &current, &cancel);
        let report = deploy(
            &admitted,
            &request(target, Consent::Absent, supply(), sha, bytes),
        );
        let elapsed = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0;
        let ready = match report.outcome {
            DeployOutcome::Probed(ready) => ready,
            other => panic!("warm worker reuse refused: {other:?} ({:?})", report.trace),
        };
        assert_eq!(ready.origin, DeployOrigin::Reused);
        assert_eq!(ready.object, deployed);
        if index >= warmup {
            warm_ms.push(elapsed);
        }
    }
    let admitted = provider(&endpoint, &facts, &cancel);
    let layout = strop_worker_deploy::cache::resolve(&admitted).unwrap();
    let mut max_live_leases = 0;
    for index in 0..(warmup + iterations) {
        let worker = worker_transport::worker(&endpoint, &deployed.path, target);
        let started = Instant::now();
        worker.health(&cancel).unwrap();
        let elapsed = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0;
        let active = worker.session().expect("live SSH worker session");
        assert!(admitted
            .lstat(&layout.lease(active.lease.0))
            .unwrap()
            .is_some());
        max_live_leases = max_live_leases.max(admitted.list(&layout.leases_dir()).unwrap().len());
        worker.shutdown().unwrap();
        for _ in 0..256 {
            if admitted.list(&layout.leases_dir()).unwrap().is_empty() {
                break;
            }
        }
        assert!(admitted.list(&layout.leases_dir()).unwrap().is_empty());
        if index >= warmup {
            launch_ms.push(elapsed);
        }
    }
    let percentiles = |raw: &[f64]| {
        let mut ordered = raw.to_vec();
        ordered.sort_by(f64::total_cmp);
        let at = |n: usize| ordered[(n * ordered.len()).div_ceil(100) - 1];
        serde_json::json!({
            "p50": at(50), "p95": at(95), "p99": at(99), "max": ordered[ordered.len() - 1],
        })
    };
    let report = serde_json::json!({
        "artifact_sha256": sha,
        "artifact_bytes": bytes,
        "target": target,
        "fixture": "localhost OpenSSH -i through private ProxyCommand, distinct context, exact private-cache reset outside cold samples",
        "warmup_requests": warmup,
        "measured_requests": iterations,
        "cold_deploy_ms": percentiles(&cold_ms),
        "cold_raw_ms": cold_ms,
        "warm_reuse_ms": percentiles(&warm_ms),
        "warm_raw_ms": warm_ms,
        "warm_launch_ms": percentiles(&launch_ms),
        "launch_raw_ms": launch_ms,
        "max_observed_live_lease_records_after_welcome": max_live_leases,
        "fixture_resets_outside_measurement": warmup + iterations - 1,
    });
    println!("STROP_SSH_BENCH={report}");
}
