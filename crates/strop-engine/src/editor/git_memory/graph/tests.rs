//! When a commit-graph is built (0101 §5.1, engine): after the event
//! that delivered a slow history read, never during a key; at most one;
//! never for a fast read, a remote repository or an editor without a
//! state directory; and nothing waits for one.

use super::*;
use crate::editor::events::AppEvent;
use crate::editor::git_memory::types::{LogDone, LogRequest};
use crate::editor::test_support::git::{fixture, pump, settle};
use crate::editor::trace::drive::Action;
use strop_core::frontend_input::Key;
use strop_core::worker::Ticket;

/// The fixture repository, with a state directory.
fn editor() -> (tempfile::TempDir, tempfile::TempDir, Editor) {
    let (repository, mut e) = fixture();
    let state = tempfile::tempdir().unwrap();
    e.set_state_dir(Some(state.path().to_path_buf()));
    (repository, state, e)
}

/// One turn of the event loop for `event`: the handler, then the
/// after-event work, as the terminal and the headless driver run it.
fn turn(e: &mut Editor, event: AppEvent) {
    e.apply_recorded(Action::Event(event)).unwrap();
}

/// Open the log and deliver its end as git having taken `took`: the
/// surface's own request, replaced by one whose completion is this.
fn finish_log(e: &mut Editor, took: Duration) -> AppEvent {
    e.open_log(false);
    let doc = e.current();
    let real = e.log_requests[&doc].ticket.clone();
    let owner = Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: real.key.clone(),
    };
    e.log_requests.insert(
        doc,
        LogRequest {
            ticket: owner.clone(),
            expected: real.key.revision,
            published: false,
        },
    );
    AppEvent::Git(GitJob::Log(Completion {
        ticket: owner,
        outcome: Outcome::Success(LogDone {
            rows: Vec::new(),
            took,
        }),
    }))
}

fn graph_file(state: &std::path::Path, e: &Editor) -> std::path::PathBuf {
    let workdir = e.git.as_ref().unwrap().repo.workdir().to_path_buf();
    let objects = side::common_objects(&workdir).unwrap();
    side::side_dir(&side::graphs_root(state), &objects).join("objects/info/commit-graph")
}

#[test]
fn a_slow_read_builds_once_after_its_event() {
    let (_repository, state, mut e) = editor();
    let done = finish_log(&mut e, Duration::from_millis(250));
    // the handler alone starts nothing: the key path never builds
    if let AppEvent::Git(job) = done {
        e.handle_git_job(job);
    }
    assert!(!e.git_work.graph.building());
    // the after-event work does
    turn(&mut e, AppEvent::EditorKey(Key::Char('j')));
    let first = e.git_work.graph.building.clone().expect("a build started");
    // a second slow read during the build starts no other
    let done = finish_log(&mut e, Duration::from_millis(250));
    turn(&mut e, done);
    assert_eq!(e.git_work.graph.building.as_ref(), Some(&first));
    settle(&mut e, |e| !e.git_work.graph.building());
    assert!(graph_file(state.path(), &e).is_file());
    // the same HEAD again: built already, nothing more
    let done = finish_log(&mut e, Duration::from_millis(250));
    turn(&mut e, done);
    assert!(!e.git_work.graph.building());
}

#[test]
fn a_fast_read_builds_nothing() {
    let (_repository, state, mut e) = editor();
    let done = finish_log(&mut e, Duration::from_millis(20));
    turn(&mut e, done);
    assert!(!e.git_work.graph.building());
    assert!(!graph_file(state.path(), &e).exists());
}

#[test]
fn no_state_directory_or_a_remote_repository_builds_nothing() {
    let (_repository, mut e) = fixture();
    let done = finish_log(&mut e, Duration::from_millis(250));
    turn(&mut e, done);
    assert!(!e.git_work.graph.building(), "no state directory");

    let (_repository, _state, mut e) = editor();
    let remote = RepoTarget::Remote {
        endpoint: strop_workspace::RemoteEndpoint::parse("ssh://host").unwrap(),
        workdir: "/repo".into(),
    };
    history_read(&mut e, &remote, Duration::from_secs(1));
    consider(&mut e);
    assert!(!e.git_work.graph.building(), "a remote repository");
}

#[test]
fn nothing_waits_for_a_graph_build() {
    let (repository, _state, mut e) = editor();
    let repo = e.git.as_ref().unwrap().repo.clone();
    // a build that never finishes
    e.git_work.graph.building = Some(Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: GraphKey { repo, head: None },
    });
    let origin = e.current();
    // keys move
    turn(&mut e, AppEvent::EditorKey(Key::Char('j')));
    assert_eq!(e.buf().line_of(e.head()), 1);
    // the log opens and fills
    e.open_log(false);
    pump(&mut e);
    assert!(e.buf().text().to_string().contains("add b"));
    e.feed_text("q");
    assert_eq!(e.current(), origin);
    // a save reaches the disk
    e.feed_text("Oheld<esc>:w<cr>");
    e.wait_io().unwrap();
    assert!(std::fs::read_to_string(repository.path().join("f.rs"))
        .unwrap()
        .starts_with("fn a() {}\nheld\n"));
    assert!(
        e.git_work.graph.building(),
        "all of that with the build held"
    );
}

#[test]
fn a_build_for_a_repository_nobody_shows_is_cancelled() {
    let (_repository, _state, mut e) = editor();
    let done = finish_log(&mut e, Duration::from_millis(250));
    turn(&mut e, done);
    assert!(e.git_work.graph.building());
    // a scratch buffer outside the repository, then every buffer of it closes
    let scratch = e
        .admit_document(crate::editor::Document::scratch(
            strop_core::Buffer::from_text("elsewhere\n"),
        ))
        .unwrap();
    let others: Vec<_> = e
        .docs
        .iter()
        .map(|(id, _)| id)
        .filter(|id| *id != scratch)
        .collect();
    for id in others {
        e.switch_to(id);
        e.close_buffer(true);
    }
    assert_eq!(e.current(), scratch);
    assert!(
        !e.git_work.graph.building(),
        "cancelled with its last buffer"
    );
}

#[test]
fn quitting_cancels_a_build() {
    let (_repository, _state, mut e) = editor();
    let done = finish_log(&mut e, Duration::from_millis(250));
    turn(&mut e, done);
    assert!(e.git_work.graph.building());
    e.finish_background_work();
    settle(&mut e, |e| !e.git_work.graph.building());
}
