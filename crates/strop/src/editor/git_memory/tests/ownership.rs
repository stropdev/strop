use super::*;
use crate::editor::git_memory::{
    BlameKey, CardKey, DiveKey, DiveTarget, GitMutation, HunkData, HunkKey, MutationKey,
    MutationKind, MutationOp,
};
use strop_core::worker::{Completion, Failure, FailureKind, Load, Outcome, Ticket};

/// An editor over a plain file in a plain tempdir (no repo), with
/// a hand-installed pure git context pointing at that tempdir: the
/// native halves of any real request fail and simply never drain.
fn editor_with_context() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.rs"), "one\ntwo\n").unwrap();
    let mut e = Editor::new(Buffer::open(dir.path().join("f.rs").to_str().unwrap()).unwrap());
    e.cwd = dir.path().to_path_buf();
    e.git = Some(strop_git::GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: dir.path().to_path_buf(),
        },
        head_sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
        head_branch: Some("main".into()),
        remotes: vec![],
    });
    (dir, e)
}

fn hunk_key(e: &Editor, view: strop_core::worker::WorkerId) -> HunkKey {
    HunkKey {
        document: e.current(),
        revision: e.buf().revision(),
        file: crate::files::FileTarget::Local(e.buf().path.clone().unwrap_or_default()),
        repo: strop_git::RepoTarget::Local {
            workdir: e.cwd.clone(),
        },
        git_view: view,
    }
}

fn install_running_hunk(e: &mut Editor, view: strop_core::worker::WorkerId) -> Ticket<HunkKey> {
    let key = hunk_key(e, view);
    let ticket = Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key,
    };
    e.hunk_load = Load::Running(ticket.clone());
    ticket
}

fn inject_hunks(e: &mut Editor, ticket: Ticket<HunkKey>, outcome: Outcome<HunkData>) {
    e.handle_git_job(GitJob::Hunks(Completion { ticket, outcome }));
}

/// Every outcome flavor for a request the editor no longer owns
/// (a newer owner took the slot) leaves the newer owner untouched.
#[test]
fn stale_hunk_results_never_clear_the_newer_owner() {
    let (_d, mut e) = editor_with_context();
    let view = e.git_view;
    let stale = install_running_hunk(&mut e, view);
    let owner = install_running_hunk(&mut e, view);
    let data = || HunkData {
        unstaged: vec![],
        staged: vec![],
        untracked: false,
    };
    inject_hunks(&mut e, stale.clone(), Outcome::Success(data()));
    inject_hunks(
        &mut e,
        stale.clone(),
        Outcome::Failed {
            failure: Failure::new(FailureKind::Panic, "worker panicked"),
            partial: None,
        },
    );
    inject_hunks(
        &mut e,
        stale,
        Outcome::Cancelled(strop_core::worker::CancelReason::Superseded),
    );
    assert!(e.hunks.is_empty() && e.staged_hunks.is_empty());
    inject_hunks(
        &mut e,
        owner,
        Outcome::Success(HunkData {
            unstaged: vec![all_add_hunk()],
            staged: vec![],
            untracked: false,
        }),
    );
    assert_eq!(
        e.hunks[0].new_count, 1,
        "the surviving request still publishes"
    );
}

/// Panic, thread-start failure, cancel and empty success are all
/// terminal: the slot frees, the failure is visible, and the next
/// explicit retry gets a different request at the same revision.
#[test]
fn hunk_failures_are_terminal_and_retry_gets_a_new_request() {
    let (_d, mut e) = editor_with_context();
    let view = e.git_view;
    for (kind, message) in [
        (FailureKind::Panic, "worker panicked"),
        (FailureKind::ThreadStart, "too many threads"),
    ] {
        let ticket = install_running_hunk(&mut e, view);
        inject_hunks(
            &mut e,
            ticket.clone(),
            Outcome::Failed {
                failure: Failure::new(kind, message),
                partial: None,
            },
        );
        assert!(
            matches!(&e.hunk_load, Load::Failed { key, .. } if key == &ticket.key),
            "{message:?} settles Failed"
        );
        assert!(e.message.contains("git diff failed"), "{}", e.message);
        // frames never retry: a refresh at the same key is covered
        e.refresh_hunks();
        assert!(
            matches!(&e.hunk_load, Load::Failed { .. }),
            "render does not retry a failed diff"
        );
        // the explicit command path retries and gets a NEW request
        e.hunk_load.retry_failed();
        e.refresh_hunks();
        let new = match &e.hunk_load {
            Load::Running(current) => current.clone(),
            other => panic!("retry re-registered: {other:?}"),
        };
        assert_ne!(new.request, ticket.request, "a different request id");
    }
    // empty success settles Ready — no hunks is a state, not a hole
    let ticket = install_running_hunk(&mut e, view);
    inject_hunks(
        &mut e,
        ticket.clone(),
        Outcome::Success(HunkData {
            unstaged: vec![],
            staged: vec![],
            untracked: false,
        }),
    );
    assert!(matches!(&e.hunk_load, Load::Ready(key) if key == &ticket.key));
}

/// A successful index mutation invalidates the git view: the
/// running hunk owner dies with it and its late result is refused.
#[test]
fn mutation_success_invalidates_view_and_rejects_preindex_results() {
    let (_d, mut e) = editor_with_context();
    let view = e.git_view;
    let hunk_ticket = install_running_hunk(&mut e, view);
    let key = MutationKey {
        document: e.current(),
        revision: e.buf().revision(),
        kind: MutationKind::Stage,
        rel: "f.rs".into(),
        repo: strop_git::RepoTarget::Local {
            workdir: e.cwd.clone(),
        },
        git_view: view,
    };
    let mutation_ticket = e.git_ticket(key).unwrap();
    e.git_mutation = Some(mutation_ticket.clone());
    e.handle_git_job(GitJob::Mutation(Completion {
        ticket: mutation_ticket,
        outcome: Outcome::Success(()),
    }));
    assert!(e.git_mutation.is_none(), "mutation settled");
    assert!(e.message.contains("staged"), "{}", e.message);
    assert_ne!(e.git_view, view, "a new git view was allocated");
    assert!(
        matches!(e.hunk_load, Load::Idle),
        "the pre-index hunk owner died with the view"
    );
    assert!(e.hunks.is_empty() && e.staged_hunks.is_empty());
    // the old view's result cannot repopulate anything
    inject_hunks(
        &mut e,
        hunk_ticket,
        Outcome::Success(HunkData {
            unstaged: vec![all_add_hunk()],
            staged: vec![],
            untracked: true,
        }),
    );
    assert!(e.hunks.is_empty(), "pre-index hunks refused");
}

/// The mutation queue is strictly FIFO and one-at-a-time; a
/// failure passes the baton, a success invalidates the view and
/// the follow-up (computed against the old view) is refused.
#[test]
fn queued_mutations_are_serial_and_failure_keeps_the_queue_live() {
    let (_directory, mut e) = editor_with_context();
    let document = e.current();
    let revision = e.buf().revision();
    let workdir = e.cwd.clone();
    let view = e.git_view;
    let key = |kind| MutationKey {
        document,
        revision,
        kind,
        rel: "f.rs".into(),
        repo: strop_git::RepoTarget::Local {
            workdir: workdir.clone(),
        },
        git_view: view,
    };
    let op = MutationOp::Stage {
        hunk: all_add_hunk(),
    };
    e.git_mutations.push_back(GitMutation {
        key: key(MutationKind::Stage),
        op: op.clone(),
    });
    e.git_mutations.push_back(GitMutation {
        key: key(MutationKind::Unstage),
        op: MutationOp::Unstage {
            hunk: all_add_hunk(),
        },
    });
    e.pump_git_mutations();
    let first = e.git_mutation.clone().expect("exactly one running");
    assert_eq!(first.key.kind, MutationKind::Stage, "FIFO head first");
    // the first fails: the second (same view) may launch
    e.handle_git_job(GitJob::Mutation(Completion {
        ticket: first.clone(),
        outcome: Outcome::Failed {
            failure: Failure::new(FailureKind::Exit, "index locked"),
            partial: None,
        },
    }));
    assert!(
        e.message.contains("stage failed"),
        "the initiating view sees the failure: {}",
        e.message
    );
    let second = e.git_mutation.clone().expect("baton passed");
    assert_eq!(second.key.kind, MutationKind::Unstage);
    assert_ne!(second.request, first.request);
    // success settles and drains the queue
    e.handle_git_job(GitJob::Mutation(Completion {
        ticket: second,
        outcome: Outcome::Success(()),
    }));
    assert!(e.git_mutation.is_none() && e.git_mutations.is_empty());
    // now the invalidated-view variant: a queued mutation computed
    // against the pre-success view is refused at launch
    e.git_mutations.push_back(GitMutation {
        key: key(MutationKind::Stage),
        op,
    });
    e.pump_git_mutations();
    assert!(
        e.git_mutation.is_none(),
        "a stale-view mutation never launches"
    );
    assert!(e.git_mutations.is_empty(), "and is dropped from the queue");
}

/// Log results land only through the surface's registered request;
/// superseded or foreign tickets change nothing.
#[test]
fn log_results_need_the_registered_request() {
    let (_directory, mut e) = editor_with_context();
    e.open_log(false);
    let doc = e.current();
    let real = e.log_requests[&doc].clone();
    let owner = Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: real.key.clone(),
    };
    e.log_requests.insert(doc, owner.clone());
    // the superseded request's rows must not land
    e.handle_git_job(GitJob::Log(Completion {
        ticket: real,
        outcome: Outcome::Success(vec![LogRow {
            text: "POISON".into(),
            sha: None,
        }]),
    }));
    assert!(
        !e.doc(doc).buf.text().to_string().contains("POISON"),
        "superseded log result refused"
    );
    // the owner's rows land, focus positions the cursor
    if let Some(Surface::CommitLog { focus, .. }) = e.doc_mut(doc).surface_payload_mut() {
        *focus = Some("f1d0".into());
    } else {
        panic!("fixture must retain its log surface");
    }
    e.handle_git_job(GitJob::Log(Completion {
        ticket: owner,
        outcome: Outcome::Success(vec![
            LogRow {
                text: "row a".into(),
                sha: Some("0123".into()),
            },
            LogRow {
                text: "row b".into(),
                sha: Some("f1d0".into()),
            },
        ]),
    }));
    let text = e.doc(doc).buf.text().to_string();
    assert!(text.contains("row a") && text.contains("row b"), "{text}");
    assert!(!e.log_requests.contains_key(&doc), "request consumed");
    // a duplicate delivery cannot double-publish
}

/// Gutter markers are incarnation-safe: a toggle-off/on cycle at
/// the same path cannot be populated by the previous incarnation's
/// result; failures remove the marker so the next toggle retries.
#[test]
fn gutter_incarnations_are_ticket_owned() {
    let (_d, mut e) = editor_with_context();
    e.toggle_blame_gutter();
    let path = e.current();
    let first = e
        .blame_gutters
        .get(&path)
        .and_then(|g| g.request.clone())
        .expect("marker owns a request");
    // toggle off/on: same path, same revision — a NEW incarnation
    e.toggle_blame_gutter();
    e.toggle_blame_gutter();
    let second = e
        .blame_gutters
        .get(&path)
        .and_then(|g| g.request.clone())
        .expect("new marker owns a new request");
    assert_ne!(first.request, second.request);
    // the old incarnation's result cannot populate the new marker
    e.handle_git_job(GitJob::Gutter(Completion {
        ticket: first,
        outcome: Outcome::Success(vec![blame_line("oldsha")]),
    }));
    assert!(
        e.blame_gutters
            .get(&path)
            .is_some_and(|g| g.lines.is_empty()),
        "the new marker stays a loading marker"
    );
    // the new incarnation's failure removes ITS marker and reports
    e.handle_git_job(GitJob::Gutter(Completion {
        ticket: second.clone(),
        outcome: Outcome::Failed {
            failure: Failure::new(FailureKind::Exit, "no blame for file"),
            partial: None,
        },
    }));
    assert!(!e.blame_gutters.contains_key(&path), "failed load removed");
    assert!(e.message.contains("blame failed"), "{}", e.message);
    // …so the next toggle starts yet another request
    e.toggle_blame_gutter();
    let third = e
        .blame_gutters
        .get(&path)
        .and_then(|g| g.request.clone())
        .expect("third incarnation registered");
    assert_ne!(third.request, second.request);
    // a successful load supersedes the interim card: its late
    // success cannot reappear
    let card = e.card_request.clone().expect("interim card pending");
    e.handle_git_job(GitJob::Gutter(Completion {
        ticket: third,
        outcome: Outcome::Success(vec![blame_line("aa"), blame_line("bb")]),
    }));
    assert!(e.card_request.is_none(), "interim card cancelled");
    assert!(e.blame_card.is_none());
    e.handle_git_job(GitJob::Card(Completion {
        ticket: card,
        outcome: Outcome::Success(Box::new(strop_git::memory::BlameCard {
            sha: "aa".into(),
            short_sha: "aa".into(),
            author: "t".into(),
            age: "1m".into(),
            summary: "late".into(),
            line: 1,
        })),
    }));
    assert!(e.blame_card.is_none(), "the late card stays dismissed");
}

/// Card replies belong to the exact line they were asked about;
/// dismissal revokes a pending request.
#[test]
fn card_results_follow_their_line_and_dismissal() {
    let (_d, mut e) = editor_with_context();
    e.blame_line(); // cursor on line 1
    let foreign = Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: CardKey {
            origin: BlameKey {
                document: e.current(),
                revision: e.buf().revision(),
                file: e.cur().file_target(&e.cwd).unwrap(),
                repo: strop_git::RepoTarget::Local {
                    workdir: e.cwd.clone(),
                },
            },
            line: 2, // not the cursor's line
        },
    };
    let owner = e.card_request.clone().expect("card registered");
    e.card_request = Some(foreign.clone());
    e.handle_git_job(GitJob::Card(Completion {
        ticket: foreign,
        outcome: Outcome::Success(Box::new(strop_git::memory::BlameCard {
            sha: "ff".into(),
            short_sha: "ff".into(),
            author: "x".into(),
            age: "1m".into(),
            summary: "wrong line".into(),
            line: 2,
        })),
    }));
    assert!(e.blame_card.is_none(), "a wrong-line card is refused");
    // restore the real owner: its result lands on the right line
    e.card_request = Some(owner.clone());
    e.handle_git_job(GitJob::Card(Completion {
        ticket: owner.clone(),
        outcome: Outcome::Success(Box::new(strop_git::memory::BlameCard {
            sha: "ab".into(),
            short_sha: "ab".into(),
            author: "t".into(),
            age: "1m".into(),
            summary: "right".into(),
            line: 1,
        })),
    }));
    assert!(e.blame_card.as_ref().is_some_and(|c| c.line == 1));
    // a visible card + pending authority: dismissal takes both
    e.card_request = Some(owner.clone());
    let taken = e.dismiss_card_authority();
    assert!(taken.is_some(), "the visible card was taken");
    assert!(e.card_request.is_none() && e.blame_card.is_none());
    e.handle_git_job(GitJob::Card(Completion {
        ticket: owner,
        outcome: Outcome::Success(Box::new(strop_git::memory::BlameCard {
            sha: "cd".into(),
            short_sha: "cd".into(),
            author: "t".into(),
            age: "1m".into(),
            summary: "late".into(),
            line: 1,
        })),
    }));
    assert!(e.blame_card.is_none(), "a dismissed card cannot return");
}

/// A dive surface changed underneath its request: the data lands
/// nowhere. A matching surface gets it.
#[test]
fn dive_results_need_their_surface() {
    let (_d, mut e) = editor_with_context();
    // hand-build a CommitLog surface with one sha row
    e.push_surface(
        Some("git log"),
        "loading log…\n",
        Surface::CommitLog {
            rows: vec![LogRow {
                text: "c0".into(),
                sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
            }],
            focus: None,
            return_to: None,
        },
    );
    let doc = e.current();
    let foreign = Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: DiveKey {
            document: doc,
            repo: strop_git::RepoTarget::Local {
                workdir: e.cwd.clone(),
            },
            target: DiveTarget::CommitFiles {
                sha: "0123456789abcdef0123456789abcdef01234567".into(),
            },
        },
    };
    let owner = e
        .git_ticket(DiveKey {
            document: doc,
            repo: strop_git::RepoTarget::Local {
                workdir: e.cwd.clone(),
            },
            target: DiveTarget::CommitFiles {
                sha: "0123456789abcdef0123456789abcdef01234567".into(),
            },
        })
        .unwrap();
    e.dive_requests.insert(doc, owner.clone());
    // a foreign ticket lands nothing
    e.handle_git_job(GitJob::Dive(Completion {
        ticket: foreign,
        outcome: Outcome::Success(crate::editor::git_memory::DiveData::Files(vec![])),
    }));
    assert_eq!(e.current(), doc, "no surface was pushed");
    // the owner's files land as a ChangedFiles surface
    let file = strop_git::memory::ChangedFile {
        path: "f.rs".into(),
        added: 1,
        deleted: 0,
    };
    e.handle_git_job(GitJob::Dive(Completion {
        ticket: owner,
        outcome: Outcome::Success(crate::editor::git_memory::DiveData::Files(vec![file])),
    }));
    assert!(
        matches!(e.surface(), Some(Surface::ChangedFiles { .. })),
        "the dive landed"
    );
    assert!(e.buf().text().to_string().contains("f.rs"));
}

/// A failed log leaves a terminal status in the surface — never a
/// forever-loading buffer — and does not touch an unrelated
/// document's state.
#[test]
fn failed_log_settles_the_surface() {
    let (_d, mut e) = editor_with_context();
    e.open_log(false);
    let doc = e.current();
    let owner = e.log_requests.get(&doc).cloned().expect("registered");
    // an unrelated document with a message must keep it
    let other = e
        .docs
        .insert(crate::editor::Document::scratch(Buffer::from_text(
            "other\n",
        )));
    e.message = "unrelated".into();
    e.handle_git_job(GitJob::Log(Completion {
        ticket: owner,
        outcome: Outcome::Failed {
            failure: Failure::new(FailureKind::Exit, "bad revision"),
            partial: None,
        },
    }));
    let text = e.doc(doc).buf.text().to_string();
    assert!(text.contains("git log failed"), "{text}");
    assert!(
        !text.contains("loading"),
        "the surface never loads forever: {text}"
    );
    // the log surface is current → its failure is reported; the
    // unrelated document's text is untouched
    assert_eq!(e.doc(other).buf.text().to_string(), "other\n");
}

// helpers ---------------------------------------------------------------

fn blame_line(sha: &str) -> strop_git::memory::BlameLine {
    strop_git::memory::BlameLine {
        sha: sha.into(),
        author: "t".into(),
        age: "1m".into(),
        ts: 0,
    }
}

fn all_add_hunk() -> strop_git::Hunk {
    strop_git::Hunk::build(
        0,
        0,
        1,
        1,
        vec![strop_git::DiffLine {
            origin: strop_git::LineOrigin::Addition,
            old_lineno: None,
            new_lineno: Some(1),
            text: b"one".to_vec(),
            has_newline: true,
        }],
    )
}
