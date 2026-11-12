//! The find budgets (0093 §7). Unlike the measurement scenarios beside
//! them, these two fail: an operation past its budget is an error, so the
//! release procedure that runs them stops.
//!
//! The per-key budget is for the list — the keystroke's ranking installed
//! and painted. The time until the new top row's preview has also been
//! read and highlighted is reported beside it, unbudgeted: it belongs to
//! the preview pipeline, and it only runs when the top row changes.
//!
//! Both generate their trees — the finder 100,000 empty files, the search
//! 8,000 source-shaped ones — as pure functions of the scenario. Budgets
//! are held against the 95th percentile, so one scheduler hiccup is not a
//! regression and a real one is.

use std::io;
use std::time::Duration;

use strop_core::frontend_input::Key;
use strop_core::Buffer;
use strop_picker::Kind;

use super::{picker_settled_ok, Drive, Fixture, Op, SETTLE};
use crate::editor::events::AppEvent;
use crate::editor::Editor;

const FIRST_PAINT: Duration = Duration::from_millis(20);
const KEY_SETTLE: Duration = Duration::from_millis(50);
const FIRST_HIT: Duration = Duration::from_millis(150);
const SEARCH_COMPLETE: Duration = Duration::from_millis(400);

pub(super) fn within(op: &Op, budget: Duration) -> io::Result<()> {
    let p95 = op.pct(95);
    let limit = budget.as_secs_f64() * 1000.0;
    if p95 > limit {
        return Err(io::Error::other(format!(
            "{} p95 {p95:.1} ms is over its {limit:.0} ms budget",
            op.name
        )));
    }
    Ok(())
}

/// 100 × 20 directories of 50 files: 100,000 paths with shared prefixes
/// and repeated names, the shape that makes ranking work.
fn finder_tree() -> io::Result<Fixture> {
    let fixture = Fixture::new("find-100k")?;
    for top in 0..100 {
        for mid in 0..20 {
            let dir = fixture.dir.join(format!("pkg{top:02}/unit{mid:02}"));
            std::fs::create_dir_all(&dir)?;
            for file in 0..50 {
                std::fs::write(dir.join(format!("part_{file:02}.rs")), "")?;
            }
        }
    }
    Ok(fixture)
}

pub(super) fn finder() -> io::Result<()> {
    let fixture = finder_tree()?;
    let mut first_paint = Op::new("first_paint");
    let mut catalog = Op::new("catalog");
    let mut key = Op::new("key_list");
    let mut with_preview = Op::new("key_preview");
    for _ in 0..5 {
        let mut drive = Drive::new(
            Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
            120,
            40,
        )?;
        first_paint.try_time(|| {
            drive.editor.open_picker(Kind::Files);
            drive.draw()
        })?;
        catalog.try_time(|| drive.settle(SETTLE))?;
        picker_settled_ok(&drive)?;
        // a name, then a second term that narrows by directory
        for typed in "part_42.rs pkg07".chars() {
            let started = std::time::Instant::now();
            drive.key(Key::Char(typed))?;
            // the list: this key's ranking installed and painted
            drive.wait_until(SETTLE, |editor| {
                editor
                    .picker()
                    .is_some_and(|glue| glue.rank_pending.is_none())
            })?;
            key.samples.push(started.elapsed());
            // everything: the new top row's preview read and highlighted
            drive.settle(SETTLE)?;
            with_preview.samples.push(started.elapsed());
        }
        let top = drive
            .editor
            .picker()
            .and_then(|glue| glue.picker.current())
            .map(|item| item.text.clone());
        if top.as_deref() != Some("pkg07/unit00/part_42.rs") {
            return Err(io::Error::other(format!(
                "finder scenario expected pkg07/unit00/part_42.rs on the top row, found {top:?}"
            )));
        }
        drive.editor.close_picker();
    }
    println!("find (100,000 paths)");
    for op in [&first_paint, &catalog, &key, &with_preview] {
        op.report();
    }
    within(&first_paint, FIRST_PAINT)?;
    within(&key, KEY_SETTLE)
}

/// 8,000 files of 100 source-shaped lines (~48 MB). One file two thirds
/// of the way through the tree holds the rare needle; every tenth file
/// holds the common one.
fn search_tree() -> io::Result<Fixture> {
    let fixture = Fixture::new("search-tree")?;
    for dir in 0..80 {
        let path = fixture.dir.join(format!("crate{dir:02}/src"));
        std::fs::create_dir_all(&path)?;
        for file in 0..100 {
            let index = dir * 100 + file;
            let mut text = String::with_capacity(100 * 64);
            for line in 0..100 {
                text.push_str(&format!(
                    "fn work_{index:05}_{line:03}(x: i32) -> i32 {{ x * {} + {} }} // pad\n",
                    line % 97,
                    index % 13
                ));
            }
            if index % 10 == 0 {
                text.push_str("let buffer = common_allocation(size);\n");
            }
            if index == 5_333 {
                text.push_str("let rare = schedule_once_only(deadline);\n");
            }
            std::fs::write(path.join(format!("unit{file:02}.rs")), text)?;
        }
    }
    Ok(fixture)
}

pub(super) fn search() -> io::Result<()> {
    let fixture = search_tree()?;
    let mut drive = Drive::new(
        Editor::new_in(Buffer::from_text(""), fixture.dir.clone()),
        120,
        40,
    )?;
    let mut first_hit = Op::new("first_hit");
    let mut complete = Op::new("complete");
    // one unmeasured pass reads the tree into the page cache: the budget
    // is for searching, not for a cold disk
    for (needle, hits, measured) in [
        ("common_allocation", 800, false),
        ("schedule_once_only", 1, true),
        ("common_allocation", 800, true),
    ] {
        for _ in 0..if measured { 5 } else { 1 } {
            drive.editor.open_search(false);
            drive.key(Key::CtrlU)?;
            let started = std::time::Instant::now();
            drive.input(AppEvent::Paste(needle.into()))?;
            drive.wait_until(SETTLE, |editor| {
                editor
                    .picker()
                    .is_some_and(|glue| !glue.picker.items.is_empty() || !glue.picker.streaming)
            })?;
            let first = started.elapsed();
            drive.settle(SETTLE)?;
            let total = started.elapsed();
            picker_settled_ok(&drive)?;
            let found = drive
                .editor
                .picker()
                .map_or(0, |glue| glue.picker.rows.len());
            if found != hits {
                return Err(io::Error::other(format!(
                    "search scenario expected {hits} hits for {needle}, found {found}"
                )));
            }
            if measured {
                first_hit.samples.push(first);
                complete.samples.push(total);
            }
            drive.editor.close_picker();
        }
    }
    println!("search (8,000 files, ~48 MB)");
    first_hit.report();
    complete.report();
    within(&first_hit, FIRST_HIT)?;
    within(&complete, SEARCH_COMPLETE)
}
