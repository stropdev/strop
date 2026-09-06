//! The nucleo evaluation (0022 §2): the picker's scorer vs
//! nucleo-matcher on realistic refilter workloads. Decision with
//! numbers, not principle. Run: cargo run -p strop-picker --example scorebench --release

use std::time::Instant;

use strop_picker::fuzzy_score;

/// A realistic file-list item corpus: repo-shaped paths with varied
/// depth, camelCase, and separators.
fn corpus(n: usize) -> Vec<String> {
    const PARTS: &[&str] = &[
        "src", "crates", "editor", "render", "core", "git", "lsp", "picker", "syntax", "grammar",
        "mod", "lib", "main", "normal", "visual", "insert", "document", "panes", "buffer", "diff",
    ];
    const NAMES: &[&str] = &[
        "mod.rs",
        "lib.rs",
        "main.rs",
        "normal.rs",
        "buffer.rs",
        "keymap.rs",
        "undo.rs",
        "cursor.rs",
        "picker_card.rs",
        "hover_card.rs",
        "diff.rs",
        "memory.rs",
        "resolve.rs",
        "parse.rs",
        "selection.rs",
        "history.rs",
    ];
    (0..n)
        .map(|i| {
            let a = PARTS[i % PARTS.len()];
            let b = PARTS[(i / PARTS.len()) % PARTS.len()];
            let name = NAMES[i % NAMES.len()];
            format!("{a}/{b}/{name}")
        })
        .collect()
}

fn main() {
    let queries = ["nm", "modrs", "ednor", "pk", "buffer", "xyzqqq"];
    for n in [10_000usize, 50_000, 100_000] {
        let items = corpus(n);

        // custom scorer: full refilter (the per-keystroke hot path)
        let t0 = Instant::now();
        let mut hits = 0usize;
        for q in &queries {
            for it in &items {
                if fuzzy_score(q, it).is_some() {
                    hits += 1;
                }
            }
        }
        let custom = t0.elapsed();

        // nucleo-matcher
        let mut matcher = nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT);
        let t1 = Instant::now();
        let mut nhits = 0usize;
        for q in &queries {
            let pat = nucleo_matcher::pattern::Pattern::parse(
                q,
                nucleo_matcher::pattern::CaseMatching::Smart,
                nucleo_matcher::pattern::Normalization::Smart,
            );
            for it in &items {
                if pat
                    .score(nucleo_matcher::Utf32Str::Ascii(it.as_bytes()), &mut matcher)
                    .is_some()
                {
                    nhits += 1;
                }
            }
        }
        let nucleo = t1.elapsed();

        println!(
            "{n:>7} items × {} queries — custom: {:>7.2?} ({hits} hits) · nucleo: {:>7.2?} ({nhits} hits)",
            queries.len(),
            custom,
            nucleo
        );
    }
}
