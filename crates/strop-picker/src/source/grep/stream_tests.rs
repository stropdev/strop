use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

use crate::source::{PickerMsg, SelectionPolicy, SourceSink, SourceSnapshot, SourceWorker};
use strop_core::worker::Outcome;
use strop_workspace::ResourceLocation;

const TIMEOUT: Duration = Duration::from_secs(20);
const POLICY: SelectionPolicy = SelectionPolicy {
    hidden: true,
    respect_ignore: true,
};
type Hit = (PathBuf, usize, usize, usize);

fn start(
    worker: &SourceWorker,
    root: &Path,
    query: &str,
    dirty: Vec<SourceSnapshot>,
    sink: impl Into<SourceSink>,
) -> strop_core::worker::CancelHandle {
    worker.search(
        Arc::new(crate::query::parse_search(query)),
        POLICY,
        ResourceLocation::local(root.to_owned()),
        dirty,
        None,
        sink,
    )
}

fn drain(rx: &mpsc::Receiver<PickerMsg>, root: &Path) -> Vec<Hit> {
    let mut hits = Vec::new();
    loop {
        match rx.recv_timeout(TIMEOUT).unwrap() {
            PickerMsg::Items(batch) => {
                for row in batch.into_items() {
                    let crate::Payload::Grep {
                        location,
                        line,
                        col,
                        match_len,
                        ..
                    } = row.payload
                    else {
                        panic!("expected a search hit")
                    };
                    hits.push((
                        location.path.strip_prefix(root).unwrap().to_owned(),
                        line,
                        col,
                        match_len,
                    ));
                }
            }
            PickerMsg::Finished(Outcome::Success(())) => {
                hits.sort();
                return hits;
            }
            other => panic!("expected a complete, untruncated search, got {other:?}"),
        }
    }
}

fn search(worker: &SourceWorker, root: &Path, query: &str, dirty: Vec<SourceSnapshot>) -> Vec<Hit> {
    let (tx, rx) = mpsc::channel();
    let _request = start(worker, root, query, dirty, tx);
    drain(&rx, root)
}

fn hit(path: &str, col: usize, len: usize) -> Hit {
    (path.into(), 1, col, len)
}

// Pause the real source thread inside the first published batch. There is no
// clock assumption about walker, matcher, UI delivery or the pending request.
fn held_first() -> (SourceSink, mpsc::Receiver<PickerMsg>, Arc<Barrier>) {
    let (tx, rx) = mpsc::channel();
    let barrier = Arc::new(Barrier::new(2));
    let release = barrier.clone();
    let once = std::sync::atomic::AtomicBool::new(true);
    let sink = SourceSink::new(move |event| {
        let hold = matches!(event, PickerMsg::Items(_))
            && once.swap(false, std::sync::atomic::Ordering::Relaxed);
        let sent = tx.send(event).is_ok();
        if hold && sent {
            release.wait();
        }
        sent
    });
    (sink, rx, barrier)
}

#[test]
fn warm_verified_hit_precedes_full_enumeration_and_cannot_hide_a_new_loose_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("z/child")).unwrap();
    fs::write(root.join("z/child/code.rs"), "needle needle\n").unwrap();
    let worker = SourceWorker::new().unwrap();
    let expected = [hit("z/child/code.rs", 1, 6), hit("z/child/code.rs", 8, 6)];
    assert_eq!(search(&worker, root, "needle", vec![]), expected);
    let (sink, rx, barrier) = held_first();
    let _request = start(&worker, root, "needle", vec![], sink);
    let first = rx.recv_timeout(TIMEOUT).unwrap();
    fs::write(root.join("loose.txt"), "é needle\n").unwrap();
    barrier.wait();
    assert!(
        matches!(&first, PickerMsg::Items(rows) if rows.len() == 2 && rows[0].text.starts_with("z/child/code.rs:"))
    );
    assert_eq!(drain(&rx, root), [hit("loose.txt", 4, 6)]);
    // A fresh process-equivalent source and the warmed source agree on native
    // identities, byte range and multiplicity after the insertion.
    assert_eq!(
        search(&worker, root, "needle", vec![]),
        search(&SourceWorker::new().unwrap(), root, "needle", vec![])
    );
}

#[test]
fn hint_rechecks_ignore_path_policy_same_stamp_and_dirty_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let worker = SourceWorker::new().unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    let path = root.join("sub/hinted.txt");
    fs::write(&path, "needle\n").unwrap();
    let known = hit("sub/hinted.txt", 1, 6);
    assert_eq!(
        search(&worker, root, "needle", vec![]),
        std::slice::from_ref(&known)
    );
    fs::write(root.join("loose.txt"), "é needle\n").unwrap();
    let loose = hit("loose.txt", 4, 6);
    // The hinted file is eligible, but the captured empty rope owns it.
    let dirty = vec![SourceSnapshot {
        path: path.clone(),
        text: ropey::Rope::new(),
    }];
    assert_eq!(
        search(&worker, root, "needle", dirty),
        std::slice::from_ref(&loose)
    );
    assert_eq!(
        search(&worker, root, "path:sub/ needle", vec![]),
        std::slice::from_ref(&known)
    );
    assert_eq!(
        search(&worker, root, "path:loose.txt needle", vec![]),
        std::slice::from_ref(&loose)
    );
    assert_eq!(
        search(&worker, root, "path:sub/ needle", vec![]),
        std::slice::from_ref(&known)
    );
    fs::write(root.join("sub/.ignore"), "hinted.txt\n").unwrap();
    assert_eq!(
        search(&worker, root, "needle", vec![]),
        std::slice::from_ref(&loose)
    );
    assert_eq!(
        search(&worker, root, "ignored:include path:sub/ needle", vec![]),
        [known]
    );
    let stamp = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "absent\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    assert_eq!(
        search(&worker, root, "ignored:include needle", vec![]),
        [loose]
    );
    assert_eq!(
        search(
            &worker,
            root,
            "ignored:include path:hinted.txt regex:\"^\"",
            vec![]
        ),
        [hit("sub/hinted.txt", 1, 0)]
    );
    fs::remove_file(&path).unwrap();
    assert!(search(&worker, root, "ignored:include absent", vec![]).is_empty());
}

#[test]
fn overflow_during_hint_publication_retires_the_physical_scan() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::write(root.join("a"), "needle\n").unwrap();
    let worker = SourceWorker::new().unwrap();
    worker.set_watching(root, true);
    assert_eq!(search(&worker, root, "needle", vec![]), [hit("a", 1, 6)]);
    let (sink, rx, barrier) = held_first();
    let guard = sink.inventory_guard();
    let _request = start(&worker, root, "needle", vec![], sink);
    let row = rx.recv_timeout(TIMEOUT).unwrap();
    worker.invalidate(root, &[], true);
    fs::rename(root.join("a"), root.join("new")).unwrap();
    barrier.wait();
    assert!(matches!(row, PickerMsg::Items(_)));
    assert!(!guard.is_current());
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap(),
        PickerMsg::Finished(Outcome::Cancelled(_))
    ));
    assert_eq!(search(&worker, root, "needle", vec![]), [hit("new", 1, 6)]);
}

#[test]
fn gaining_watch_coverage_and_protocol_writes_do_not_cancel_an_authoritative_walk() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::write(root.join("note"), "needle\nneedle\n").unwrap();
    let worker = SourceWorker::new().unwrap();
    worker.set_watching(root, false);
    let (sink, rx, barrier) = held_first();
    let guard = sink.inventory_guard();
    let _request = start(&worker, root, "needle", vec![], sink);
    let first = rx.recv_timeout(TIMEOUT).unwrap();
    // Real startup ordering: an already pinned local root is now watched,
    // and this editor's lock file was created alongside the source.
    worker.set_watching(root, true);
    let lock = PathBuf::from(format!(".strop-lock-{}", "a".repeat(64)));
    fs::write(root.join(&lock), "needle\n").unwrap();
    worker.invalidate(root, std::slice::from_ref(&lock), false);
    barrier.wait();
    assert!(matches!(first, PickerMsg::Items(rows) if rows.len() == 1));
    assert!(guard.is_current());
    assert_eq!(drain(&rx, root), [(PathBuf::from("note"), 2, 1, 6)]);
    // A user-visible endpoint in the same notification batch still retires
    // the observation, including after the completion was already delivered.
    worker.invalidate(root, &[lock, PathBuf::from("note")], false);
    assert!(!guard.is_current());
}

#[test]
fn rapid_retype_while_a_warm_hit_is_in_flight_never_finishes_its_successor() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::write(root.join("a"), "oldneedle\n").unwrap();
    fs::write(root.join("b"), "newneedle\n").unwrap();
    let worker = SourceWorker::new().unwrap();
    assert_eq!(search(&worker, root, "oldneedle", vec![]), [hit("a", 1, 9)]);
    let (sink, old, barrier) = held_first();
    let _old_request = start(&worker, root, "oldneedle", vec![], sink);
    let first = old.recv_timeout(TIMEOUT).unwrap();
    let (tx, new) = mpsc::channel();
    let _new_request = start(&worker, root, "newneedle", vec![], tx);
    barrier.wait();
    assert!(matches!(first, PickerMsg::Items(_)));
    assert!(matches!(
        old.recv_timeout(TIMEOUT).unwrap(),
        PickerMsg::Finished(Outcome::Cancelled(_))
    ));
    assert_eq!(drain(&new, root), [hit("b", 1, 9)]);
    assert!(matches!(
        old.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}

#[cfg(unix)]
#[test]
fn priority_hint_and_dirty_mask_use_native_bytes_not_lossy_names() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let raw = Path::new(std::ffi::OsStr::from_bytes(b"bad\xff.txt"));
    let other = Path::new("bad�.txt");
    let worker = SourceWorker::new().unwrap();
    fs::write(root.join(raw), "needle\n").unwrap();
    assert_eq!(
        search(&worker, root, "needle", vec![]),
        [(raw.to_owned(), 1, 1, 6)]
    );
    fs::write(root.join(other), "needle\n").unwrap();
    let dirty = vec![SourceSnapshot {
        path: root.join(raw),
        text: ropey::Rope::new(),
    }];
    assert_eq!(
        search(&worker, root, "needle", dirty),
        [(other.to_owned(), 1, 1, 6)]
    );
}

#[test]
fn declaration_and_language_mix_do_not_narrow_text_or_expand_a_folder() {
    for (a, b) in [("main.cpp", "main.py"), ("main.rs", "main.go")] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("work");
        fs::create_dir_all(root.join("a/service")).unwrap();
        fs::create_dir_all(root.join("b/service")).unwrap();
        let a = format!("a/service/{a}");
        let b = format!("b/service/{b}");
        for path in [&a[..], &b, "notes.txt"] {
            fs::write(root.join(path), "é needle\n").unwrap();
        }
        fs::write(tmp.path().join("outside"), "needle\n").unwrap();
        let worker = SourceWorker::new().unwrap();
        worker.set_watching(&root, true);
        let expected = [hit(&a, 4, 6), hit(&b, 4, 6), hit("notes.txt", 4, 6)];
        assert_eq!(search(&worker, &root, "needle", vec![]), expected);
        let ticket = worker.inventory_root(&root).unwrap();
        let record =
            crate::DeclaredBoundary::new("a/service".into(), Some(vec!["unknown".into()])).unwrap();
        assert!(worker.set_declarations(&ticket, Arc::from([record])));
        assert_eq!(search(&worker, &root, "needle", vec![]), expected);
        let folder = root.join("a/service");
        assert_eq!(
            search(&worker, &folder, "needle", vec![]),
            [hit(a.rsplit('/').next().unwrap(), 4, 6)]
        );
    }
}

/// Differential reference with a different traversal: finish the authoritative
/// serial file-finder walk, mask dirty paths, then run the full disk pass.
#[test]
fn priority_stream_matches_gathered_exhaustive_native_spans() {
    use crate::source::{flow::StreamSender, selection, snapshots};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("pkg")).unwrap();
    for (name, text) in [
        ("pkg/a.rs", "é Needle needle\nnext\n"),
        ("pkg/b.go", "needle needle\n"),
        ("pkg/dirty", "needle\n"),
        ("pkg/.ignore", "ignored\n"),
        ("pkg/ignored", "Needle\n"),
        ("notes.txt", "needle\n"),
    ] {
        fs::write(root.join(name), text).unwrap();
    }
    let dirty = || {
        vec![SourceSnapshot {
            path: root.join("pkg/dirty"),
            text: ropey::Rope::from_str("edited Needle\n"),
        }]
    };
    let worker = SourceWorker::new().unwrap();
    for query in [
        "needle",
        "Needle",
        "path:pkg/ needle",
        "ignored:include needle",
        "regex:\"^ne+\"",
        "path:a.rs regex:\"^\"",
        "absent",
        "text:needle NOT text:next",
    ] {
        let parsed = crate::query::parse_search(query);
        let plan = crate::query::FileSelectionPlan::compile(&parsed).unwrap();
        let content = crate::query::ContentPlan::compile(&parsed)
            .unwrap()
            .unwrap();
        let mut paths = Vec::new();
        selection::walk(root, &plan, POLICY, &|| false, |path| {
            paths.push(path);
            true
        })
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let stream = StreamSender::new(tx.into(), Arc::default(), Arc::default());
        let (cancel, _handle) = strop_core::worker::CancelToken::standalone();
        snapshots::emit_snapshots(
            &ResourceLocation::local(root.to_owned()),
            &content,
            dirty(),
            &mut paths,
            &stream,
            &cancel,
            snapshots::Sources {
                catalog: None,
                symbols: None,
            },
        )
        .unwrap_or_else(|error| panic!("{}", String::from(error)));
        let terminal = super::disk::search(
            &super::disk::Scope {
                cwd: root,
                content: &content,
                catalog: None,
                symbols: None,
                tx: &stream,
                cancel: &cancel,
            },
            &paths,
            2,
        );
        stream.control(PickerMsg::Finished(terminal));
        let expected = drain(&rx, root);
        assert_eq!(
            search(&worker, root, query, dirty()),
            expected,
            "first: {query}"
        );
        assert_eq!(
            search(&worker, root, query, dirty()),
            expected,
            "repeat: {query}"
        );
    }
}
