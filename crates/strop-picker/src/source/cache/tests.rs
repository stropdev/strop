use super::*;
use std::fs;
use std::sync::{mpsc, Barrier};
use strop_core::worker::CancelReason;

fn read_inventory(
    owner: &Mutex<Caches>,
    root: &Path,
    text: &str,
    token: &CancelToken,
    tick: &(impl Fn() + Sync),
) -> Delivery {
    let query = Arc::new(crate::query::parse_search(text));
    let plan = FileSelectionPlan::compile(&query).unwrap();
    inventory(
        owner,
        ScanRequest {
            root,
            query: &query,
            plan: &plan,
            policy: selection::DISCOVERY_POLICY,
            need_symbols: true,
        },
        1,
        token,
        tick,
    )
}

fn current(owner: &Mutex<Caches>, root: &Path) -> Inventory {
    let (token, _handle) = CancelToken::standalone();
    read_inventory(owner, root, "", &token, &|| {}).unwrap()
}

// Pause an actual worker after begin. The callback is an ordinary worker
// heartbeat, not a replacement walker, publisher or injected clock.
fn held(root: &Path, owner: &Mutex<Caches>, mutate: impl FnOnce()) -> Delivery {
    let entered = Barrier::new(2);
    let release = Barrier::new(2);
    let pause = AtomicBool::new(true);
    let (token, _handle) = CancelToken::standalone();
    std::thread::scope(|s| {
        let job = s.spawn(|| {
            read_inventory(owner, root, "", &token, &|| {
                if pause.swap(false, Ordering::AcqRel) {
                    entered.wait();
                    release.wait();
                }
            })
        });
        entered.wait();
        mutate();
        release.wait();
        job.join().unwrap()
    })
}

#[test]
fn late_rebuild_cannot_discharge_overflow_or_watch_transition() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("lib.rs"), "fn old() {}\n").unwrap();
    let owner = Mutex::new(Caches::default());
    owner.lock().set_watching(root, true);
    current(&owner, root);
    for watch_change in [false, true] {
        owner.lock().invalidate(root, &[], true);
        let result = held(root, &owner, || {
            if watch_change {
                owner.lock().set_watching(root, false);
                owner.lock().set_watching(root, true);
            } else {
                owner.lock().invalidate(root, &[], true);
            }
        });
        assert!(matches!(result, Err(InventoryError::Cancelled)));
        assert!(owner.lock().scopes.last().unwrap().rescan);
        current(&owner, root);
        assert!(owner.lock().stats(root).unwrap().full);
    }
}

#[test]
fn held_scan_cannot_restore_an_evicted_or_recreated_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("old.rs"), "fn old() {}\n").unwrap();
    let owner = Mutex::new(Caches::default());
    owner.lock().set_watching(&root, true);
    let old_load = owner.lock().root_ticket(&root).unwrap();
    let evicted = held(&root, &owner, || {
        for n in 0..SCOPE_LIMIT {
            let folder = temp.path().join(format!("other{n}"));
            fs::create_dir(&folder).unwrap();
            current(&owner, &folder);
        }
    });
    assert!(matches!(evicted, Err(InventoryError::Cancelled)));
    assert!(owner.lock().stats(&root).is_none());
    assert_eq!(owner.lock().scopes.len(), SCOPE_LIMIT);
    let replaced = held(&root, &owner, || {
        fs::remove_dir_all(&root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("new.rs"), "fn new() {}\n").unwrap();
        // No notification: the held native descriptor still rejects same-name replacement.
    });
    assert!(matches!(replaced, Err(InventoryError::Cancelled)));
    assert!(!owner.lock().set_declarations(&old_load, Arc::from([])));
    assert_eq!(
        current(&owner, &root).paths.as_slice(),
        &[PathBuf::from("new.rs")]
    );
    let delayed = held(&root, &owner, || {
        let next = temp.path().join("other0");
        let mut cache = owner.lock();
        cache.retire_root(&root);
        cache.set_watching(&next, true);
        cache.retire_root(&next);
        cache.set_watching(&root, true);
    });
    assert!(matches!(delayed, Err(InventoryError::Cancelled)));
    assert!(owner.lock().stats(&root).is_none());
}

#[test]
fn compatible_demand_receives_whole_walk_when_warm_consumer_cancels() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/.ignore"), "ignored.rs\n").unwrap();
    fs::write(root.join("sub/ignored.rs"), "fn hidden() {}\n").unwrap();
    fs::write(root.join("sub/kept.rs"), "fn visible() {}\n").unwrap();
    let owner = Mutex::new(Caches::default());
    owner.lock().set_watching(root, true);
    let (leader, cancel_leader) = CancelToken::standalone();
    let (demand, _demand_handle) = CancelToken::standalone();
    let entered = Barrier::new(2);
    let release = Barrier::new(2);
    let once = AtomicBool::new(true);
    let (joined, receiver) = mpsc::sync_channel(1);
    std::thread::scope(|s| {
        let job = s.spawn(|| {
            read_inventory(&owner, root, "", &leader, &|| {
                if once.swap(false, Ordering::AcqRel) {
                    entered.wait();
                    release.wait();
                }
            })
        });
        entered.wait();
        let follower = s.spawn(|| {
            read_inventory(&owner, root, "kind:function needle", &demand, &|| {
                let _ = joined.try_send(());
            })
        });
        receiver.recv().unwrap();
        let live = owner.lock().scopes.last().unwrap().flights[0]
            .upgrade()
            .unwrap();
        assert!(
            live.follower.lock().is_some(),
            "matcher text did not split identical selection"
        );
        cancel_leader.cancel(CancelReason::Superseded);
        release.wait();
        assert!(matches!(
            job.join().unwrap(),
            Err(InventoryError::Cancelled)
        ));
        let answer = follower.join().unwrap().unwrap();
        assert!(answer.paths.contains(&PathBuf::from("sub/kept.rs")));
        assert!(!answer.paths.contains(&PathBuf::from("sub/ignored.rs")));
        assert_eq!(answer.symbols.unwrap().iter().count(), 1);
    });
    // No second press can use the prior traversal's negative membership.
    fs::remove_file(root.join("sub/.ignore")).unwrap();
    assert!(current(&owner, root)
        .paths
        .contains(&PathBuf::from("sub/ignored.rs")));
}

#[test]
fn narrower_and_include_ignored_requests_never_borrow_default_membership() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let folder = root.join("sub");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join(".ignore"), "skip*\n").unwrap();
    for name in [
        OsString::from_vec(b"keep-\xff.txt".to_vec()),
        OsString::from("skip.txt"),
    ] {
        fs::write(folder.join(name), "needle").unwrap();
    }
    fs::write(root.join("outside.txt"), "needle").unwrap();
    let owner = Mutex::new(Caches::default());
    owner.lock().set_watching(root, true);
    let default = held(root, &owner, || {
        for (cwd, text) in [
            (folder.as_path(), "ignored:include"),
            (root, "path:sub ignored:include"),
        ] {
            let (token, _handle) = CancelToken::standalone();
            let selected = read_inventory(&owner, cwd, text, &token, &|| {}).unwrap();
            let plan = FileSelectionPlan::compile(&crate::query::parse_search(text)).unwrap();
            let mut reference = Vec::new();
            selection::walk(cwd, &plan, selection::DISCOVERY_POLICY, &|| false, |p| {
                reference.push(p);
                true
            })
            .unwrap();
            let mut actual = Arc::unwrap_or_clone(selected.paths);
            actual.sort();
            reference.sort();
            assert_eq!(actual, reference);
            assert!(actual.iter().any(|p| p.ends_with("skip.txt")));
        }
    })
    .unwrap();
    assert!(!default.paths.contains(&PathBuf::from("sub/skip.txt")));
}

#[test]
fn moved_child_and_declaration_reload_cannot_republish_old_project() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("a/service/pkg")).unwrap();
    fs::create_dir_all(root.join("b/service")).unwrap();
    fs::write(root.join("a/service/pkg/loose.txt"), "needle").unwrap();
    let owner = Mutex::new(Caches::default());
    owner.lock().set_watching(root, true);
    let install = |path: &str| {
        let ticket = owner.lock().root_ticket(root).unwrap();
        let record =
            DeclaredBoundary::new(path.into(), Some(vec!["rust".into(), "unknown".into()]))
                .unwrap();
        assert!(owner.lock().set_declarations(&ticket, Arc::from([record])));
    };
    install("a/service/pkg");
    assert!(current(&owner, root)
        .catalog
        .projects()
        .any(|p| p.root == Path::new("a/service/pkg")));
    let old = held(root, &owner, || {
        fs::rename(root.join("a/service/pkg"), root.join("b/service/pkg")).unwrap();
        owner.lock().invalidate(
            root,
            &["a/service/pkg".into(), "b/service/pkg".into()],
            false,
        );
        install("b/service/pkg");
    });
    assert!(matches!(old, Err(InventoryError::Cancelled)));
    let answer = current(&owner, root);
    assert_eq!(
        &answer.paths[..],
        &[PathBuf::from("b/service/pkg/loose.txt")]
    );
    assert!(answer
        .catalog
        .projects()
        .any(|p| p.root == Path::new("b/service/pkg")));
    assert!(!answer
        .catalog
        .projects()
        .any(|p| p.root == Path::new("a/service/pkg")));
}

#[test]
fn cancelled_delivery_cannot_clear_a_newer_physical_scan_slot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("sample.rs"), "fn sample() {}\n").unwrap();
    let mut cache = Caches::default();
    cache.set_watching(root, true);
    let pin = cache.root_ticket(root).unwrap();
    let dir = Directory::open(root).unwrap();
    let observed = Directories {
        pinned: Some((pin.clone(), dir.clone())),
        folder: dir,
    };
    let query = Arc::new(SearchQuery::default());
    let begin = |cache: &mut Caches, token: &CancelToken| {
        let (work, _) = cache.begin(
            root,
            &observed,
            &query,
            selection::DISCOVERY_POLICY,
            true,
            token,
        );
        let Work::Build(build) = work.unwrap() else {
            panic!("expected separate physical scan")
        };
        build
    };
    let (token, cancel) = CancelToken::standalone();
    let first = begin(&mut cache, &token);
    // An actual, successful payload already exists when the scan is cancelled.
    let result = Ok(current(&Mutex::new(Caches::default()), root));
    cancel.cancel(CancelReason::Superseded);
    let (new_token, _handle) = CancelToken::standalone();
    let second = begin(&mut cache, &new_token);
    assert_eq!(first.slot, second.slot);
    assert_ne!(first.started.scan, second.started.scan);
    assert!(!cache.settle(&first, &result, BaselineStats::default(), 1).0);
    assert!(cache.disk_shard(&pin, &root.join("sample.rs")).is_none());
    assert!(
        cache
            .settle(&second, &result, BaselineStats::default(), 1)
            .0
    );
    assert!(cache.disk_shard(&pin, &root.join("sample.rs")).is_some());
    // A redundant completion cannot charge or overwrite the published value.
    assert!(
        !cache
            .settle(&second, &result, BaselineStats::default(), usize::MAX)
            .0
    );

    let big = begin(&mut cache, &new_token);
    assert!(
        cache
            .settle(
                &big,
                &result,
                BaselineStats::default(),
                SHARD_BYTES_PER_SCOPE + 1
            )
            .0
    );
    assert!(cache.disk_shard(&pin, &root.join("sample.rs")).is_none());
}

#[test]
fn prefix_storm_coalesces_without_restoring_old_hints_on_abandon() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("loose.rs"), "fn loose() {}\n").unwrap();
    let owner = Mutex::new(Caches::default());
    owner.lock().set_watching(root, true);
    current(&owner, root);
    owner.lock().invalidate(
        root,
        &["src/lib.rs".into(), "src".into(), "src/sub/out.rs".into()],
        false,
    );
    assert_eq!(
        owner.lock().scopes[0].invalidated.as_slice(),
        &[PathBuf::from("src")]
    );
    let result = held(root, &owner, || {
        let storm: Vec<PathBuf> = (0..=INVALIDATION_LIMIT)
            .map(|n| format!("dir{n}").into())
            .collect();
        owner.lock().invalidate(root, &storm, false);
    });
    assert!(matches!(result, Err(InventoryError::Cancelled)));
    assert!(owner.lock().scopes[0].rescan);
    assert!(owner.lock().scopes[0].invalidated.is_empty());
}
