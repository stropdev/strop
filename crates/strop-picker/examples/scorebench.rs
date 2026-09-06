//! The picker regression bench (0023): measures the REAL hot path —
//! Picker::refilter over accumulated items, per keystroke — the way the
//! reviewer's audit did. Not a comparison: a regression floor. Run:
//! cargo run -p strop-picker --example scorebench --release

use std::time::Instant;

use strop_picker::{Item, Kind, Payload, Picker};

fn items(n: usize) -> Vec<Item> {
    (0..n)
        .map(|i| Item {
            text: format!("crates/package_{i:06}/src/main_{i}.rs"),
            payload: Payload::File(format!("{i}.rs").into()),
        })
        .collect()
}

fn main() {
    let median = |mut ns: Vec<u128>| {
        ns.sort_unstable();
        ns[ns.len() / 2] as f64 / 1e6
    };
    for n in [10_000usize, 50_000, 100_000] {
        let mut p = Picker::new(Kind::Files, items(n), false);
        p.input.text = "mainrs".into();
        let mut samples = Vec::new();
        for _ in 0..7 {
            let t = Instant::now();
            p.refilter();
            samples.push(t.elapsed().as_nanos());
        }
        println!(
            "PERF refilter items={n} rows={} median_ms={:.3}",
            p.rows.len(),
            median(samples)
        );
    }
}
