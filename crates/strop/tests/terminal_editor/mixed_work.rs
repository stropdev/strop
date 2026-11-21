//! Count observed work separately: tracing changes latency, so none of these
//! samples are fed into the terminal timing distribution.
use super::{benchmark_binary, mixed_fixture::Fixture};
use std::fs;
use std::process::Command;

#[test]
#[ignore = "opt-in external profiler; STROP_STRACE points to strace"]
fn mixed_work_counts() {
    let (binary, sha, _) = benchmark_binary();
    let fixture = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("services.jsonl");
    let syscalls = dir.path().join("syscalls");
    let script = dir.path().join("actions");
    let query = serde_json::to_string(fixture.rare).unwrap();
    fs::write(&script, format!("settle 20000\nkeys <space>s\nsettle 20000\nframe\nkeys <esc><esc>\nkeys <space>s\nsettle 20000\nframe\nkeys <esc><esc>\nkeys O// changed<esc>\nkeys <space>s\nsettle 20000\nframe\nkeys <esc><esc>\nkeys <space>/\npaste {query}\nsettle 20000\nframe\n")).unwrap();
    let observed = Command::new(std::env::var_os("STROP_STRACE").expect("set STROP_STRACE"))
        .args(["-f", "-qq", "-yy", "-s", "0", "-e", "getdents64", "-o"])
        .arg(&syscalls)
        .arg(binary)
        .arg("--log-file")
        .arg(&trace)
        .arg("headless")
        .arg(script)
        .arg(fixture.large)
        .current_dir(&fixture.work)
        .env_clear()
        .env("HOME", dir.path())
        .env("XDG_STATE_HOME", dir.path().join("state"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("PATH", dir.path().join("empty"))
        .output()
        .unwrap();
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    let frame = String::from_utf8(observed.stdout).unwrap();
    assert!(
        frame.contains("measured_symbol") && frame.contains("p19.txt"),
        "{frame}"
    );
    let mut outlines = 0;
    let mut kept = 0;
    let mut grep_jobs = 0;
    for line in fs::read_to_string(trace).unwrap().lines() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        let fields = &event["fields"];
        if event["event"] == "job_started" {
            outlines +=
                usize::from(fields["service"] == "symbols" && fields["request"] == "outline");
            grep_jobs += usize::from(fields["service"] == "picker" && fields["source"] == "grep");
        } else if event["event"] == "job_finished" {
            kept += usize::from(fields["service"] == "symbols" && fields["result"] == "kept");
        }
    }
    let calls = fs::read_to_string(syscalls).unwrap();
    let root = format!("<{}>", fixture.work.display());
    let below = format!("<{}/", fixture.work.display());
    let visited_batches = calls
        .lines()
        .filter(|s| s.contains("getdents64(") && (s.contains(&root) || s.contains(&below)))
        .count();
    assert!(outlines > 0 && grep_jobs > 0 && visited_batches > 0);
    println!(
        "STROP_MIXED_WORK={}",
        serde_json::json!({"binary_sha256":sha,"outline_jobs_started":outlines,"kept_outline_opens":kept,"grep_source_jobs_started":grep_jobs,"workspace_getdents64_calls":visited_batches})
    );
}
