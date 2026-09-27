use super::*;

/// Count the actual worker executable, not shell-supervisor command
/// strings which legitimately contain `--worker-stdio` as an argument.
fn native_worker_count(processes: &str, objects_dir: &str) -> usize {
    processes
        .lines()
        .filter(|line| {
            let mut columns = line.split_whitespace().skip(3);
            columns.next().is_some_and(|program| {
                program
                    .strip_prefix(objects_dir)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                    && columns.any(|argument| argument == "--worker-stdio")
            })
        })
        .count()
}

/// Opt-in WK20 product-path observation: actual scoped Docker upload,
/// verified activation/probe, cache reuse and one admitted live worker.
/// Every cold repetition starts after this private fixture deletes only
/// its own *unleased* object and receipt, outside the timed operation.
/// The reset is a measurement fixture, not production garbage collection.
#[test]
#[ignore = "native Docker worker deployment latency; run explicitly with --ignored --nocapture"]
fn native_container_cold_warm_deploy_and_launch_samples() {
    if !gate("native_container_cold_warm_deploy_and_launch_samples") {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let binary = worker_binary(&directory);
    let artifact_bytes = std::fs::metadata(&binary).unwrap().len();
    let fixture = Fixture::busybox_raw_for(&tag(), &[], "1800");
    let (cancel, _owner) = token();
    let engine_ref = engine(&cancel).expect("engine probes");
    let identity = fixture.identity(&engine_ref, &cancel);
    let provider =
        ContainerProvider::capture(&engine_ref, &identity, ShellPolicy::Required, &cancel)
            .expect("the fixture endpoint is admitted");
    let supply = || ArtifactSupply::LocalBinary {
        path: binary.clone(),
    };
    let warmups = 8;
    let samples = 64;
    let mut cold_ms = Vec::with_capacity(samples);
    let mut warm_ms = Vec::with_capacity(samples);
    let mut launch_ms = Vec::with_capacity(samples);
    let mut cached: Option<strop_worker_deploy::deploy::VerifiedObject> = None;
    for index in 0..(warmups + samples) {
        if let Some(previous) = cached.take() {
            fixture.assert_workers_reaped(Duration::from_secs(10));
            let layout = strop_worker_deploy::cache::resolve(&provider).unwrap();
            provider.remove(&layout.receipt(&previous.sha256)).unwrap();
            provider.remove(&previous.path).unwrap();
            assert!(provider.lstat(&previous.path).unwrap().is_none());
        }
        let started = Instant::now();
        let ready = probed(deploy(&provider, &request(supply())).outcome);
        let elapsed = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0;
        assert_eq!(ready.origin, DeployOrigin::Uploaded);
        assert_eq!(ready.object.bytes, artifact_bytes);
        if index >= warmups {
            cold_ms.push(elapsed);
        }
        cached = Some(ready.object);
    }
    let deployed = cached.unwrap();
    let objects_dir = strop_worker_deploy::cache::resolve(&provider)
        .unwrap()
        .objects_dir();
    let mut observed_probe_peak = 0;
    for index in 0..(warmups + samples) {
        let started = Instant::now();
        let ready = probed(deploy(&provider, &request(supply())).outcome);
        let elapsed = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0;
        assert_eq!(ready.origin, DeployOrigin::Reused);
        assert_eq!(ready.object, deployed);
        observed_probe_peak =
            observed_probe_peak.max(native_worker_count(&fixture.processes(), &objects_dir));
        if index >= warmups {
            warm_ms.push(elapsed);
        }
    }
    fixture.assert_workers_reaped(Duration::from_secs(10));
    let observed_orphans = fixture
        .processes()
        .lines()
        .filter(|line| line.ends_with(" [cat]") || line.ends_with(" [sh]"))
        .count();
    for index in 0..(warmups + samples) {
        let worker = provider.worker(&deployed.path);
        let started = Instant::now();
        worker
            .health(&cancel)
            .expect("live in-namespace worker handshake");
        let elapsed = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0;
        let processes = fixture.processes();
        assert_eq!(
            native_worker_count(&processes, &objects_dir),
            1,
            "one authorized live editor lease, not an accumulation:\n{processes}"
        );
        worker.shutdown().unwrap();
        fixture.assert_workers_reaped(Duration::from_secs(10));
        if index >= warmups {
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
        "artifact_sha256": deployed.sha256,
        "artifact_bytes": artifact_bytes,
        "target": provider.endpoint().target,
        "fixture": "one writable BusyBox container, selected principal, exact unleased-object reset before cold samples",
        "warmup_requests": warmups,
        "measured_requests": samples,
        "cold_deploy_ms": percentiles(&cold_ms),
        "cold_raw_ms": cold_ms,
        "warm_reuse_ms": percentiles(&warm_ms),
        "warm_raw_ms": warm_ms,
        "warm_launch_ms": percentiles(&launch_ms),
        "launch_raw_ms": launch_ms,
        "worker_count_after_each_live_welcome": 1,
        "observed_probe_worker_peak_after_reuse": observed_probe_peak,
        "observed_unreaped_fixture_children_after_reuse": observed_orphans,
        "fixture_resets_outside_measurement": warmups + samples - 1,
    });
    println!("STROP_CONTAINER_BENCH={report}");
}
