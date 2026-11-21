//! Worker-side traversal and delivery for a captured inventory scan.
use super::{
    selection, Arc, BaselineStats, Caches, CancelToken, Delivery, Directories, Inventory,
    InventoryError, Mutex, ProjectCatalog, ScanRequest, SymbolIndex, Work,
};

/// Worker-only. Joining receives the entire physical walk, including its
/// beginning; a later press cannot subscribe to a completed path list.
pub(in crate::source) fn inventory(
    cache: &Mutex<Caches>,
    request: ScanRequest<'_>,
    threads: usize,
    token: &CancelToken,
    heartbeat: &(impl Fn() + Sync),
) -> Delivery {
    let ScanRequest {
        root,
        query,
        plan,
        policy,
        need_symbols,
    } = request;
    if token.is_cancelled() {
        return Err(InventoryError::Cancelled);
    }
    let directories = Directories::open(cache, root)?;
    let (work, evicted) = cache.lock().begin(
        root,
        &directories,
        query,
        policy.effective(plan),
        need_symbols,
        token,
    );
    drop(evicted);
    let mut build = match work? {
        Work::Joined(flight) => return flight.wait(token, heartbeat),
        Work::Build(build) => build,
    };
    let stopped = || {
        heartbeat();
        build.flight.stopped()
    };
    let mut stats = BaselineStats {
        full: build.prior.is_none(),
        ..Default::default()
    };
    let mut delivery = (|| {
        let declarations = if build.flight.folder.as_path() == build.flight.root.as_path() {
            std::borrow::Cow::Borrowed(build.declarations.as_ref())
        } else if !build.declarations.is_empty()
            && selection::observes_folder(&build.flight.root, root, &stopped)
        {
            let relative = root
                .strip_prefix(build.flight.root.as_path())
                .unwrap_or(root);
            std::borrow::Cow::Owned(
                build
                    .declarations
                    .iter()
                    .filter_map(|d| d.relative_to(relative))
                    .collect(),
            )
        } else {
            std::borrow::Cow::Borrowed(&[][..])
        };
        let (paths, catalog, catalog_stats) = if build.flight.policy == selection::DISCOVERY_POLICY
        {
            let mut observer = ProjectCatalog::observer(root, &declarations);
            let mut paths = Vec::new();
            let mut bytes = 0;
            let mut exceeded = false;
            let exhausted = selection::walk_observing(
                root,
                plan,
                policy,
                &stopped,
                |dir| observer.visit(dir),
                |path| {
                    bytes += path.as_os_str().as_encoded_bytes().len();
                    if paths.len() >= selection::PATH_LIMIT || bytes > selection::PATH_BYTES {
                        exceeded = true;
                        return false;
                    }
                    paths.push(path);
                    true
                },
            )
            .map_err(InventoryError::Walk)?;
            if exceeded {
                return Err(InventoryError::Limit);
            }
            let (catalog, scanned) = observer.finish(exhausted);
            (paths, catalog, scanned)
        } else {
            // Different directory universes: declarations must not acquire
            // authority in an ignored directory just because a query shows it.
            let paths = selection::collect(root, plan, policy, &stopped, threads)?;
            let (catalog, scanned) = ProjectCatalog::scan(root, &stopped, &declarations);
            (paths, catalog, scanned)
        };
        if stopped() {
            return Err(InventoryError::Cancelled);
        }
        stats.catalog = catalog_stats;
        let symbols = need_symbols.then(|| {
            let (index, scanned) = SymbolIndex::refresh(
                build.prior.as_deref(),
                root,
                &paths,
                &build.invalidated,
                &stopped,
            );
            stats.symbols = scanned;
            Arc::new(index)
        });
        Ok(Inventory {
            paths: Arc::new(paths),
            catalog: Arc::new(catalog),
            symbols,
            epoch: build.flight.current.clone(),
        })
    })();
    if !directories.current(cache, root) {
        delivery = Err(InventoryError::Cancelled);
    }
    let cost = delivery
        .as_ref()
        .ok()
        .and_then(|inventory| inventory.symbols.as_ref())
        .map_or(0, |index| index.retained_bytes());
    // Keep even rejected payload destruction on the worker outside the lock.
    let (admitted, retired) = cache.lock().settle(&build, &delivery, stats, cost);
    drop(retired);
    if !admitted && delivery.is_ok() {
        delivery = Err(InventoryError::Cancelled);
    }
    if build.flight.follower.lock().is_some() {
        *build.flight.result.lock() = Some(delivery.clone());
        build.flight.ready.notify_all();
    }
    build.delivered = true;
    if token.is_cancelled() {
        Err(InventoryError::Cancelled)
    } else {
        delivery
    }
}
