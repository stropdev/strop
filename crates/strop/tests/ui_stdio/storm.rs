//! 0057 VF19 storm campaign, the UiSessionModel finding: a 16-way
//! parallel storm of streaming-search-then-shutdown backends once wedged
//! one backend's `finish()` past an 8s jobs budget (exited without bye)
//! — a storm-full semantic lane refused the forwarded picker-ranking
//! `Stopped`, the forwarder died and `picker_ranking.retiring` never
//! emptied. Serial repros settled in <0.2s. The mechanism is pinned by
//! `forwarded_semantic_event_survives_a_full_lane` (strop-engine) and
//! the `forward-drop-refused` mutant (verification/mutants.json); this
//! campaign is the end-to-end bound: every storm backend must settle —
//! `STROP_JOBS_BUDGET_MS` is a hard hang canary, never a sleep.

use std::ffi::OsString;
use std::path::Path;

use crate::harness::{fixture, strop};
use strop_ui_protocol::Driver;

/// Parallel backends: the finding's storm was 16-way on a loaded host;
/// 8 keeps the campaign hermetic on small CI runners while still
/// forcing the streaming/teardown overlap on every backend.
const BACKENDS: usize = 8;
/// Files per shared workspace: enough rows that the walk is still
/// streaming when the shutdown lands (the finding's exact overlap).
const FILES: usize = 4000;
/// Hard settle canary per backend (the finding's wedge was unbounded:
/// any finite budget fails it). Sized like the test lane's own
/// `STROP_JOBS_BUDGET_MS` doctrine — it must never budget legitimate
/// work on a storm-loaded host running the full suite in parallel.
const JOBS_BUDGET_MS: &str = "60000";

fn backend_env(root: &Path, backend: usize) -> Vec<(&'static str, OsString)> {
    let home = root.join(format!("home{backend}"));
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::create_dir_all(home.join("state")).unwrap();
    vec![
        ("HOME", home.clone().into_os_string()),
        ("XDG_CONFIG_HOME", home.join("config").into_os_string()),
        ("XDG_STATE_HOME", home.join("state").into_os_string()),
        ("STROP_LOG", OsString::new()),
        ("STROP_JOBS_BUDGET_MS", JOBS_BUDGET_MS.into()),
    ]
}

#[test]
fn streaming_search_shutdown_storm_settles() {
    let fixture = fixture();
    let root = fixture.dir.path();
    for file in 0..FILES {
        std::fs::write(
            root.join(format!("storm{file:04}.txt")),
            "needle one\nneedle two\nplain line\n",
        )
        .unwrap();
    }
    let mut backends = Vec::new();
    for backend in 0..BACKENDS {
        let root = root.to_path_buf();
        backends.push(std::thread::spawn(move || {
            let env = backend_env(&root, backend);
            let mut driver = Driver::spawn(Path::new(strop()), &root, &env)
                .unwrap_or_else(|error| panic!("backend {backend} handshake: {error}"));
            // Open the workspace search and type a query that matches
            // every fixture file; the stream is still landing when the
            // shutdown is authorized (the finding's exact shape).
            driver
                .act_keys("<space>/")
                .unwrap_or_else(|error| panic!("backend {backend} open search: {error}"));
            driver
                .act_keys("needle")
                .unwrap_or_else(|error| panic!("backend {backend} query: {error}"));
            driver
                .wait_view("the search stream", |view| {
                    view.state["picker_streaming"] == true
                })
                .unwrap_or_else(|error| panic!("backend {backend} streaming: {error}"));
            let status = driver
                .shutdown()
                .unwrap_or_else(|error| panic!("backend {backend} shutdown: {error}"));
            assert!(
                status.success(),
                "backend {backend} must settle inside its jobs budget: {status}"
            );
        }));
    }
    for backend in backends {
        backend.join().unwrap();
    }
}
