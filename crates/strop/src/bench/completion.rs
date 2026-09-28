//! Stage-separated observations over the real bounded event channel, engine
//! reducer and TUI renderer. No external terminal/stdio transport is included.
use super::*;
use serde_json::{json, Value};

struct Sample {
    enqueue_to_consume_ms: f64,
    consume_to_frame_ms: f64,
    dispatch_ms: f64,
    frame_ms: f64,
}

fn key_sample(drive: &mut Drive, key: Key) -> io::Result<Sample> {
    let queued = Instant::now();
    drive
        .sender
        .send(AppEvent::EditorKey(key))
        .map_err(|_| io::Error::other("benchmark input queue refused its event"))?;
    loop {
        let event = drive.events.recv_timeout(SETTLE).map_err(|error| {
            io::Error::other(format!("benchmark input did not arrive: {error}"))
        })?;
        let input = matches!(event, AppEvent::EditorKey(_));
        let consumed = Instant::now();
        drive.editor.handle_app_event(event);
        let dispatched = Instant::now();
        if input {
            drive.drain();
            let frame_started = Instant::now();
            drive.draw()?;
            let painted = Instant::now();
            return Ok(Sample {
                enqueue_to_consume_ms: (consumed - queued).as_secs_f64() * 1000.0,
                consume_to_frame_ms: (painted - consumed).as_secs_f64() * 1000.0,
                dispatch_ms: (dispatched - consumed).as_secs_f64() * 1000.0,
                frame_ms: (painted - frame_started).as_secs_f64() * 1000.0,
            });
        }
    }
}

fn distribution(values: impl Iterator<Item = f64>) -> Value {
    let raw: Vec<_> = values.collect();
    let mut sorted = raw.clone();
    sorted.sort_by(f64::total_cmp);
    let at = |percent: usize| sorted[(percent * sorted.len()).div_ceil(100) - 1];
    json!({"raw_ms": raw, "p50": at(50), "p95": at(95), "p99": at(99), "max": at(100)})
}

fn words_ready(editor: &Editor) -> bool {
    editor.completion_menu().is_some_and(|menu| {
        menu.providers()
            .iter()
            .any(|provider| provider.source == "buf" && provider.state == "ready")
    })
}

fn scenario(
    label: &str,
    text: String,
    enabled: bool,
    automatic: bool,
    cold: bool,
) -> io::Result<Value> {
    let bytes = text.len();
    let mut editor = Editor::new_in(
        Buffer::from_text(&text),
        PathBuf::from("/completion-benchmark"),
    );
    drop(text);
    let mut config = editor.config().clone();
    config.completion.enabled = enabled;
    config.completion.auto_popup = automatic;
    editor.set_config(config);
    let mut drive = Drive::new(editor, 120, 40)?;
    drive.key(Key::Char('G'))?;
    drive.key(Key::Char('A'))?;
    let first = Instant::now();
    let _ = key_sample(&mut drive, Key::Char('e'))?;
    if enabled && !automatic {
        drive.key(Key::CtrlSpace)?;
    }
    let first_useful = if enabled && !cold {
        drive.wait_until(SETTLE, words_ready)?;
        Some(first.elapsed().as_secs_f64() * 1000.0)
    } else {
        None
    };
    let mut samples = Vec::with_capacity(128);
    for _ in 0..64 {
        samples.push(key_sample(&mut drive, Key::Char('s'))?);
        samples.push(key_sample(&mut drive, Key::Backspace)?);
    }
    let latest = Instant::now();
    if enabled {
        drive.wait_until(SETTLE, words_ready)?;
    }
    let latest_ready = latest.elapsed().as_secs_f64() * 1000.0;
    if enabled
        && !drive.editor.completion_menu().is_some_and(|menu| {
            (0..menu.len()).any(|index| menu.row(index).is_some_and(|row| row.label == "response"))
        })
    {
        return Err(io::Error::other(
            "latest completion did not match the restored prefix",
        ));
    }
    let observed: Value = serde_json::from_str(&strop_engine::editor::state_json(&drive.editor))
        .map_err(io::Error::other)?;
    let cancellation = key_sample(&mut drive, Key::CtrlE)?;
    if drive.editor.completion_menu().is_some() {
        return Err(io::Error::other("dismissal retained completion authority"));
    }
    let stopped = Instant::now();
    let tick = drive.editor.tape().sample_tick();
    drive
        .editor
        .recorded_action(crate::editor::trace::drive::Action::Finish, tick)?;
    drive.settle(SETTLE)?;
    let retired: Value = serde_json::from_str(&strop_engine::editor::state_json(&drive.editor))
        .map_err(io::Error::other)?;
    if retired["completion"]["worker"] != "idle"
        || retired["completion"]["publication"]["charged_bytes"] != 0
    {
        return Err(io::Error::other(
            "completion native retirement did not reach zero",
        ));
    }
    Ok(json!({
        "scenario": label, "source_bytes": bytes, "enabled": enabled, "automatic": automatic,
        "typing_during_initial_index": cold,
        "first_useful_ms": first_useful, "latest_query_after_typing_ms": latest_ready,
        "enqueue_to_consume_ms": distribution(samples.iter().map(|sample| sample.enqueue_to_consume_ms)),
        "consume_to_frame_ms": distribution(samples.iter().map(|sample| sample.consume_to_frame_ms)),
        "dispatch_ms": distribution(samples.iter().map(|sample| sample.dispatch_ms)),
        "frame_ms": distribution(samples.iter().map(|sample| sample.frame_ms)),
        "cancel_consume_to_frame_ms": cancellation.consume_to_frame_ms,
        "native_stop_ms": stopped.elapsed().as_secs_f64() * 1000.0,
        "completion_before_cancel": observed["completion"],
        "completion_after_retirement": retired["completion"],
    }))
}

pub(super) fn run() -> io::Result<()> {
    let small = "response result reusable return_value\nr\n";
    let cases = vec![
        scenario("disabled", small.into(), false, true, false)?,
        scenario("manual", small.into(), true, false, false)?,
        scenario("automatic", small.into(), true, true, false)?,
        scenario(
            "cold_16mib",
            format!("{}r\n", "response result padding\n".repeat(700_000)),
            true,
            true,
            true,
        )?,
        scenario(
            "line_1mib",
            format!("response result\n{}r\n", "padding ".repeat(131_072)),
            true,
            true,
            true,
        )?,
    ];
    println!(
        "STROP_COMPLETION_STAGES={}",
        json!({
            "method": "bounded in-process event enqueue to consume, engine dispatch and actual TUI frame; excludes external input/terminal transport",
            "version": env!("CARGO_PKG_VERSION"), "geometry": [120, 40], "samples_per_case": 128,
            "cases": cases,
        })
    );
    Ok(())
}
