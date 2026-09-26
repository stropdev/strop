//! Integration tests on real tmpdirs: real inotify instances, real kernel
//! queues, real churn. inotify events are queued by the kernel before the
//! causing syscall returns, so `drain` immediately after an operation is
//! deterministic — no wall-clock sleeps anywhere.

use std::fs;
use std::path::Path;

use strop_worker_protocol::{Event, NotifyKind, Subscription};
use strop_workspace::ResourceLocation;

use super::{NotifyConfig, NotifyError, NotifyManager};

/// Small caps so overflow/exclusion paths trigger deterministically.
fn test_config() -> NotifyConfig {
    NotifyConfig {
        max_subscriptions: 8,
        max_watches_per_subscription: 256,
        max_pending_hints: 64,
        max_hints_per_event: 8,
        max_read_cycles: 64,
    }
}

fn subscribe(mgr: &mut NotifyManager, root: &Path, recursive: bool) -> Subscription {
    mgr.subscribe(&ResourceLocation::local(root.to_path_buf()), recursive)
        .map(|s| s.subscription)
        .unwrap_or_else(|e| panic!("subscribe {} failed: {e}", root.display()))
}

fn hints(events: &[Event], sub: Subscription) -> Vec<(Vec<u8>, NotifyKind)> {
    let mut out = Vec::new();
    for event in events {
        if let Event::Notify {
            subscription,
            hints,
            ..
        } = event
        {
            if *subscription == sub {
                out.extend(hints.iter().map(|h| (h.path.clone(), h.kind)));
            }
        }
    }
    out
}

fn overflowed(events: &[Event], sub: Subscription) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::NotifyOverflow { subscription, .. } if *subscription == sub))
}

fn boundary(events: &[Event], sub: Subscription) -> Option<u64> {
    events.iter().find_map(|e| match e {
        Event::ReconcileBoundary {
            subscription,
            sequence,
        } if *subscription == sub => Some(*sequence),
        _ => None,
    })
}

/// Drain until the kernel queue is empty (a drain that publishes nothing
/// and leaves nothing). Bounded: a quiet tree drains in one cycle.
fn drain_idle(mgr: &mut NotifyManager) {
    for _ in 0..128 {
        if mgr.drain().expect("drain").is_empty() {
            return;
        }
    }
    panic!("kernel queue never went quiet");
}

#[test]
fn create_modify_rename_replace_delete_classify() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, false);
    drain_idle(&mut mgr);

    fs::write(root.join("a"), b"one").expect("write a");
    fs::rename(root.join("a"), root.join("b")).expect("rename a b");
    fs::write(root.join("stage"), b"two").expect("write stage");
    fs::rename(root.join("stage"), root.join("b")).expect("atomic replace b");
    fs::remove_file(root.join("b")).expect("delete b");

    let events = mgr.drain().expect("drain");
    let got = hints(&events, sub);
    for expected in [
        (b"a".to_vec(), NotifyKind::Created),
        (b"a".to_vec(), NotifyKind::Modified),
        (b"a".to_vec(), NotifyKind::Renamed),
        (b"b".to_vec(), NotifyKind::Renamed),
        (b"stage".to_vec(), NotifyKind::Renamed),
        (b"b".to_vec(), NotifyKind::Removed),
    ] {
        assert!(got.contains(&expected), "missing {expected:?} in {got:?}");
    }
    // Atomic replacement is a kernel-paired rename onto the victim's name:
    // the victim path is invalidated, never silently relocated.
    assert!(got
        .iter()
        .any(|(p, k)| p == b"b" && *k == NotifyKind::Renamed));
    assert!(
        !overflowed(&events, sub),
        "no overflow in a quiet run: {events:?}"
    );
}

#[test]
fn rename_out_of_scope_is_ambiguous_never_guessed() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let outside = tempfile::tempdir().expect("outside");
    let root = tmp.path().to_path_buf();
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, false);
    fs::write(root.join("x"), b"x").expect("write");
    drain_idle(&mut mgr);

    fs::rename(root.join("x"), outside.path().join("x")).expect("rename out");
    let got = hints(&mgr.drain().expect("drain"), sub);
    assert!(
        got.contains(&(b"x".to_vec(), NotifyKind::Ambiguous)),
        "move-out must be Ambiguous, got {got:?}"
    );
    assert!(!got
        .iter()
        .any(|(p, k)| p == b"x" && *k == NotifyKind::Renamed));
}

#[test]
fn move_in_from_outside_is_created() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let outside = tempfile::tempdir().expect("outside");
    let root = tmp.path().to_path_buf();
    fs::write(outside.path().join("y"), b"y").expect("write outside");
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, false);
    drain_idle(&mut mgr);

    fs::rename(outside.path().join("y"), root.join("y")).expect("rename in");
    let got = hints(&mgr.drain().expect("drain"), sub);
    assert!(
        got.contains(&(b"y".to_vec(), NotifyKind::Created)),
        "got {got:?}"
    );
}

#[test]
fn recursive_covers_new_directory_trees() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, true);
    drain_idle(&mut mgr);

    // The mkdir itself reports; the drain registers the new subtree's
    // watch (inotify recursion is dynamic: a watch can only cover changes
    // after its registration — the Created hint on "d" is what triggers
    // the client's reobservation of anything earlier).
    fs::create_dir(root.join("d")).expect("mkdir");
    let got = hints(&mgr.drain().expect("drain"), sub);
    assert!(
        got.contains(&(b"d".to_vec(), NotifyKind::Created)),
        "got {got:?}"
    );

    fs::create_dir(root.join("d").join("deep")).expect("mkdir deep");
    let got = hints(&mgr.drain().expect("drain"), sub);
    assert!(
        got.contains(&(b"d/deep".to_vec(), NotifyKind::Created)),
        "got {got:?}"
    );

    fs::write(root.join("d").join("deep").join("g"), b"g").expect("write deep");
    let got = hints(&mgr.drain().expect("drain"), sub);
    assert!(
        got.contains(&(b"d/deep/g".to_vec(), NotifyKind::Created)),
        "got {got:?}"
    );
}

#[test]
fn in_scope_directory_rename_rekeys_subtree() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    fs::create_dir(root.join("old")).expect("mkdir");
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, true);
    drain_idle(&mut mgr);

    fs::rename(root.join("old"), root.join("new")).expect("rename dir");
    fs::write(root.join("new").join("f"), b"f").expect("write in renamed dir");
    let got = hints(&mgr.drain().expect("drain"), sub);
    assert!(
        got.contains(&(b"old".to_vec(), NotifyKind::Renamed)),
        "got {got:?}"
    );
    assert!(
        got.contains(&(b"new".to_vec(), NotifyKind::Renamed)),
        "got {got:?}"
    );
    // The subtree watch followed the rename: writes under the new path report.
    assert!(
        got.contains(&(b"new/f".to_vec(), NotifyKind::Created)),
        "got {got:?}"
    );
    assert!(
        !got.iter().any(|(p, _)| p.starts_with(b"old/")),
        "stale paths in {got:?}"
    );
}

#[test]
fn excluded_subtree_invalidates_baseline_at_subscribe() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    fs::create_dir(root.join("sub")).expect("mkdir sub");
    // Two watches total: the root tree and the parent guard. The recursive
    // walk cannot cover `sub` inside that bound — an explicit coverage gap.
    let config = NotifyConfig {
        max_watches_per_subscription: 2,
        ..test_config()
    };
    let mut mgr = NotifyManager::new(config);
    let sub = subscribe(&mut mgr, &root, true);

    let events = mgr.drain().expect("drain");
    assert_eq!(
        boundary(&events, sub),
        Some(0),
        "boundary first: {events:?}"
    );
    assert!(
        overflowed(&events, sub),
        "excluded subtree demands rescan: {events:?}"
    );
}

#[test]
fn full_pending_queue_latches_overflow_never_silent_loss() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let config = NotifyConfig {
        max_pending_hints: 4,
        ..test_config()
    };
    let mut mgr = NotifyManager::new(config);
    let sub = subscribe(&mut mgr, &root, false);
    drain_idle(&mut mgr);

    for i in 0..16u32 {
        fs::write(root.join(format!("f{i}")), b"x").expect("write");
    }
    let events = mgr.drain().expect("drain");
    assert!(
        overflowed(&events, sub),
        "queue overrun must publish the rescan obligation: {events:?}"
    );
}

#[test]
fn generation_staleness_is_refused() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, false);

    let stale = Subscription {
        generation: sub.generation + 9,
        ..sub
    };
    let err = mgr
        .unsubscribe(stale)
        .expect_err("stale generation must refuse");
    assert!(
        matches!(err, NotifyError::StaleGeneration { current, .. } if current == sub.generation),
        "got {err:?}"
    );

    // Reinstall the same scope: same id, bumped generation.
    let reinstalled = subscribe(&mut mgr, &root, false);
    assert_eq!(reinstalled.id, sub.id);
    assert_eq!(reinstalled.generation, sub.generation + 1);

    // The old identity is dead: unsubscribing with it refuses, and no
    // event can ever carry it again.
    let err = mgr
        .unsubscribe(sub)
        .expect_err("dead generation must refuse");
    assert!(
        matches!(err, NotifyError::StaleGeneration { current, .. } if current == reinstalled.generation),
        "got {err:?}"
    );
    mgr.unsubscribe(reinstalled)
        .expect("current generation retires");
    let err = mgr
        .unsubscribe(reinstalled)
        .expect_err("retired id must refuse");
    assert!(
        matches!(err, NotifyError::UnknownSubscription { .. }),
        "got {err:?}"
    );
}

#[test]
fn unsubscribe_stops_events_promptly() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, false);
    fs::write(root.join("before"), b"1").expect("write before");
    drain_idle(&mut mgr);

    mgr.unsubscribe(sub).expect("unsubscribe");
    fs::write(root.join("after"), b"2").expect("write after");
    fs::write(root.join("after2"), b"3").expect("write after2");
    let events = mgr.drain().expect("drain");
    assert!(
        !events.iter().any(|e| matches!(e,
            Event::Notify { subscription, .. }
            | Event::NotifyOverflow { subscription, .. }
            | Event::ReconcileBoundary { subscription, .. } if *subscription == sub)),
        "dead subscription published: {events:?}"
    );
    assert!(mgr.is_empty());
}

#[test]
fn root_removal_and_rename_are_explicit() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let watched = tmp.path().join("watched");
    fs::create_dir(&watched).expect("mkdir watched");

    // Deleting the scope root: empty-path Removed plus a rescan obligation.
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &watched, true);
    drain_idle(&mut mgr);
    fs::remove_dir(&watched).expect("rmdir");
    let events = mgr.drain().expect("drain");
    let got = hints(&events, sub);
    assert!(
        got.contains(&(Vec::new(), NotifyKind::Removed)),
        "root removal must name the empty path: {got:?}"
    );
    assert!(
        overflowed(&events, sub),
        "root removal invalidates: {events:?}"
    );

    // Renaming the scope root away: Ambiguous plus rescan, never tracked.
    let watched2 = tmp.path().join("watched2");
    fs::create_dir(&watched2).expect("mkdir watched2");
    let sub2 = subscribe(&mut mgr, &watched2, true);
    drain_idle(&mut mgr);
    fs::rename(&watched2, tmp.path().join("elsewhere")).expect("rename root");
    let events = mgr.drain().expect("drain");
    let got = hints(&events, sub2);
    assert!(
        got.contains(&(Vec::new(), NotifyKind::Ambiguous)),
        "root rename must be Ambiguous on the empty path: {got:?}"
    );
    assert!(
        overflowed(&events, sub2),
        "root rename invalidates: {events:?}"
    );
}

#[test]
fn reconcile_boundary_precedes_hints_and_sequences_are_monotone() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let mut mgr = NotifyManager::new(test_config());
    let sub = subscribe(&mut mgr, &root, false);

    let events = mgr.drain().expect("drain");
    assert_eq!(
        events.len(),
        1,
        "only the boundary before any change: {events:?}"
    );
    assert_eq!(boundary(&events, sub), Some(0));

    let mut last = 0u64;
    for i in 0..3u32 {
        fs::write(root.join(format!("f{i}")), b"x").expect("write");
        for event in mgr.drain().expect("drain") {
            let sequence = match event {
                Event::Notify {
                    subscription,
                    sequence,
                    ..
                }
                | Event::NotifyOverflow {
                    subscription,
                    sequence,
                }
                | Event::ReconcileBoundary {
                    subscription,
                    sequence,
                } if subscription == sub => sequence,
                _ => continue,
            };
            assert!(
                sequence > last,
                "sequence must climb: {sequence} after {last}"
            );
            last = sequence;
        }
    }
}

#[test]
fn churn_storm_bounds_hold() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let config = test_config();
    let mut mgr = NotifyManager::new(config);
    let sub = subscribe(&mut mgr, &root, false);
    fs::write(root.join("base"), b"x").expect("base");
    drain_idle(&mut mgr);

    // 100k entries of churn via hardlinks: two syscalls per entry, no data
    // writes. This overruns the kernel queue (default 16384) and our tiny
    // pending cap many times over.
    let victim = root.join("base");
    let scratch = root.join("scratch");
    for _ in 0..100_000u32 {
        fs::hard_link(&victim, &scratch).expect("link");
        fs::remove_file(&scratch).expect("unlink");
    }

    let mut saw_overflow = false;
    let mut rounds = 0usize;
    loop {
        let events = mgr.drain().expect("drain");
        rounds += 1;
        for event in &events {
            if let Event::Notify { hints, .. } = event {
                assert!(
                    hints.len() <= config.max_hints_per_event,
                    "batch over the wire bound: {}",
                    hints.len()
                );
            }
            if matches!(event, Event::NotifyOverflow { subscription, .. } if *subscription == sub) {
                saw_overflow = true;
            }
        }
        if events.is_empty() {
            break;
        }
        assert!(rounds < 4096, "drain never caught up with a quiet queue");
    }
    assert!(
        saw_overflow,
        "a 100k-entry storm must invalidate the baseline"
    );

    // The subscription survives its own overflow: fresh changes report.
    fs::write(root.join("after"), b"y").expect("write after storm");
    let got = hints(&mgr.drain().expect("drain"), sub);
    assert!(
        got.contains(&(b"after".to_vec(), NotifyKind::Created))
            || overflowed(&mgr.drain().expect("drain2"), sub),
        "post-storm change must surface: {got:?}"
    );
}

#[test]
fn remote_and_container_scopes_are_typed_refusals() {
    let mut mgr = NotifyManager::new(test_config());
    let remote = ResourceLocation::parse_uri("ssh://example.com/data").expect("remote uri");
    let err = mgr
        .subscribe(&remote, true)
        .expect_err("remote must refuse");
    assert!(
        matches!(err, NotifyError::UnsupportedNamespace),
        "got {err:?}"
    );
    assert!(mgr.is_empty());

    // A relative local path never resolves here either.
    let relative = ResourceLocation {
        filesystem: Default::default(),
        path: Path::new("rel").to_path_buf(),
    };
    let err = mgr
        .subscribe(&relative, false)
        .expect_err("relative must refuse");
    assert!(matches!(err, NotifyError::RelativeScope), "got {err:?}");
}

#[test]
fn subscription_limit_is_typed() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let config = NotifyConfig {
        max_subscriptions: 1,
        ..test_config()
    };
    let mut mgr = NotifyManager::new(config);
    subscribe(&mut mgr, tmp.path(), false);
    let other = tempfile::tempdir().expect("other");
    let err = mgr
        .subscribe(&ResourceLocation::local(other.path().to_path_buf()), false)
        .expect_err("second subscription over the bound must refuse");
    assert!(
        matches!(err, NotifyError::SubscriptionLimit(1)),
        "got {err:?}"
    );
}

#[test]
fn poll_reports_readiness() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    let mut mgr = NotifyManager::new(test_config());
    subscribe(&mut mgr, &root, false);
    drain_idle(&mut mgr);

    fs::write(root.join("p"), b"p").expect("write");
    assert!(mgr
        .poll(Some(std::time::Duration::from_secs(5)))
        .expect("poll"));
}
