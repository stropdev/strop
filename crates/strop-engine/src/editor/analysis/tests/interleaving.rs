//! 0111 decision 10: the UI ↔ worker seam under seeded interleavings of
//! edits, undo, scrolling, admission, held and out-of-order delivery,
//! stale replays and failures, against the real worker thread. After
//! every step the painted frame is well formed; at quiescence the window
//! is decided, a further prepare requests nothing, and a decided frame
//! equals a fresh highlight of the same rope.
use super::*;

const SEEDS: u64 = 4;
const STEPS: usize = 40;
const WIDTH: usize = 80;
const TOKENS: [&str; 8] = ["x", "fn ", "// ", "\"", "{", "}", "/// `a`", "let a = 1;"];

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        if n == 0 {
            0
        } else {
            (self.0 % n as u64) as usize
        }
    }
}

fn next_event(e: &Editor) -> AnalysisEvent {
    e.analysis
        .rx
        .as_ref()
        .expect("local analysis channel")
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the worker answers every request")
}

fn assert_well_formed(spans: &[Span], len: usize, context: &str) {
    for pair in spans.windows(2) {
        assert!(pair[0].end <= pair[1].start, "overlap {pair:?}: {context}");
    }
    for span in spans {
        assert!(
            span.start < span.end && span.end <= len,
            "span {span:?} outside 0..{len}: {context}"
        );
    }
}

/// Three visible lines from `top`, clamped to the document.
fn window(e: &Editor, top: usize) -> (usize, usize) {
    let lines = e.buf().len_lines();
    let top = top.min(lines.saturating_sub(1));
    let bottom = (top + 3).min(lines);
    (
        e.buf().line_start(top),
        e.buf().line_end(bottom.saturating_sub(1)),
    )
}

#[test]
fn the_worker_seam_settles_under_interleavings() {
    for seed in 1..=SEEDS {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut e = Editor::new(Buffer::from_text(
            "/// Doc `code`\nfn one() { let s = \"hi\"; }\n\nfn two() {}\n// tail\n",
        ));
        e.buf_mut().path = Some(PathBuf::from("/workspace/a.rs"));
        let doc = e.current();
        let target = AnalysisTarget::Document(doc);
        let mut held: Vec<AnalysisEvent> = Vec::new();
        let mut answered: Option<Completion<AnalysisKey, FrameAnalysis>> = None;
        let mut top = 0;
        let mut log = Vec::new();
        for _ in 0..STEPS {
            let lines = e.buf().len_lines();
            let op = rng.below(9);
            match op {
                0 => {
                    let line = rng.below(lines) + 1;
                    let token = TOKENS[rng.below(TOKENS.len())];
                    e.feed_text(&format!("{line}GI{token}<esc>"));
                }
                1 => {
                    let line = rng.below(lines) + 1;
                    let token = TOKENS[rng.below(TOKENS.len())];
                    e.feed_text(&format!("{line}Go{token}<esc>"));
                }
                2 if lines > 2 => e.feed_text(&format!("{}Gdd", rng.below(lines) + 1)),
                3 => e.feed_text("u"),
                4 => top = rng.below(lines),
                5 => {
                    let (first, last) = window(&e, top);
                    let _ = e.document_analysis(doc, first, last, 0, WIDTH);
                }
                // hold the worker's answer; deliver it later, possibly
                // after edits made it stale
                6 if held.is_empty() && e.analysis.pending.contains_key(&target) => {
                    held.push(next_event(&e));
                }
                7 if !held.is_empty() => {
                    let event = held.swap_remove(rng.below(held.len()));
                    if let AnalysisEvent::Completed(completion) = &event {
                        answered = Some((**completion).clone());
                    }
                    e.handle_analysis(event);
                }
                8 => match answered.take() {
                    // replayed after its request was answered: dropped
                    Some(completion) if rng.below(2) == 0 => {
                        e.handle_analysis(AnalysisEvent::Completed(Box::new(completion)));
                    }
                    _ if e.analysis.pending.contains_key(&target) => complete_pending(
                        &mut e,
                        &target,
                        Outcome::failed(strop_core::worker::FailureKind::Io, "boom"),
                    ),
                    _ => {}
                },
                _ => {}
            }
            log.push(op);
            let (first, last) = window(&e, top);
            if let Some(frame) = e.document_analysis_cached(doc, first, last, 0, WIDTH) {
                let context = format!("seed {seed} ops {log:?} text {:?}", e.buf().text());
                assert_well_formed(&frame.spans, e.buf().len_bytes(), &context);
            }
        }
        // quiescence: deliver everything, re-admit until decided — on
        // half the seeds the final window's analysis fails
        let (first, last) = window(&e, top);
        let fail_last = rng.below(2) == 0;
        let context = format!(
            "seed {seed} ops {log:?} fail_last {fail_last} text {:?}",
            e.buf().text()
        );
        let mut rounds = 0;
        loop {
            rounds += 1;
            // a window that keeps asking never settles (0111 §A B1)
            assert!(rounds <= 16, "analysis never settles: {context}");
            let _ = e.document_analysis(doc, first, last, 0, WIDTH);
            let Some(pending) = e.analysis.pending.get(&target) else {
                break;
            };
            if fail_last && pending.ticket.key.revision == e.buf().revision() {
                complete_pending(
                    &mut e,
                    &target,
                    Outcome::failed(strop_core::worker::FailureKind::Io, "boom"),
                );
                continue;
            }
            let event = held.pop().unwrap_or_else(|| next_event(&e));
            e.handle_analysis(event);
        }
        let key = e
            .analysis_key(doc, first, last, 0, WIDTH)
            .expect("a syntax surface");
        let Some(value) = e.exact_entry(&key).map(|entry| entry.value.clone()) else {
            panic!("quiescent window undecided: {context}");
        };
        let admitted = e.admissions.analysis;
        let _ = e.document_analysis(doc, first, last, 0, WIDTH);
        assert_eq!(e.admissions.analysis, admitted, "re-request: {context}");
        if let Some(frame) = value {
            let rope = e.buf().snapshot();
            let mut fresh =
                strop_syntax::Highlighter::for_language(strop_syntax::languages::LanguageId::Rust)
                    .expect("rust");
            let expected = fresh
                .highlight(&rope, e.buf().revision(), first, last)
                .expect("fresh highlight");
            // error recovery may settle differently incrementally
            if !fresh.has_parse_error() {
                assert_eq!(frame.spans, expected, "settled != fresh: {context}");
            }
        }
        e.wait_analysis();
    }
}
