//! Calibration of the decode budget (0101 §4.2, §5.2): what decoding one
//! frame really costs on the heap, measured with a counting allocator on
//! the decode path async-lsp runs — the body buffer, then
//! `serde_json::from_slice` into an untagged message whose payload is a
//! `Value` — for the honest replies strop keeps and for adversarial shapes.
//! Every one must stay under `tokens × DECODE_BYTES_PER_TOKEN` plus the
//! body. A `serde_json` or `async-lsp` upgrade that makes decoding dearer
//! fails here, and the constant moves with it.
//!
//! Its own test binary, with exactly one test: the allocator is global,
//! and a second test running beside it would count into its peaks.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use async_lsp::lsp_types::{DocumentSymbolResponse, NumberOrString};
use async_lsp::{AnyNotification, AnyRequest, ResponseError};
use strop_lsp::{structure_tokens, DECODE_BYTES_PER_TOKEN, MAX_DECODE_TOKENS};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to `System` with the caller's own pointer
// and layout; the counters are plain atomics and never touch the memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(layout.size(), Relaxed) + layout.size();
        PEAK.fetch_max(live, Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        System.dealloc(pointer, layout)
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if size > layout.size() {
            let grow = size - layout.size();
            let live = LIVE.fetch_add(grow, Relaxed) + grow;
            PEAK.fetch_max(live, Relaxed);
        } else {
            LIVE.fetch_sub(layout.size() - size, Relaxed);
        }
        System.realloc(pointer, layout, size)
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// async-lsp 0.2.4's private `RawMessage<Message>`, field for field.
#[derive(serde::Deserialize)]
struct RawMessage {
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(flatten)]
    inner: Message,
}
#[derive(serde::Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
enum Message {
    Request(AnyRequest),
    Response(AnyResponse),
    Notification(AnyNotification),
}
#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct AnyResponse {
    id: NumberOrString,
    result: Option<serde_json::Value>,
    error: Option<ResponseError>,
}

/// The peak heap of reading and decoding one reply whose `result` is
/// `result`, and, for symbols, of turning it into the typed reply.
fn peak(result: &str, typed: bool) -> (usize, usize, usize) {
    let body = format!(r#"{{"jsonrpc":"2.0","id":7,"result":{result}}}"#).into_bytes();
    let tokens = structure_tokens(&body);
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    // async-lsp's read: a buffer of Content-Length bytes, then the decode.
    let mut read = vec![0u8; body.len()];
    read.copy_from_slice(&body);
    let message = serde_json::from_slice::<RawMessage>(&read).expect("decodes");
    if typed {
        if let Message::Response(AnyResponse {
            result: Some(value),
            ..
        }) = message.inner
        {
            let reply: DocumentSymbolResponse = serde_json::from_value(value).expect("typed");
            std::hint::black_box(&reply);
        }
    } else {
        std::hint::black_box(&message);
    }
    drop(read);
    (tokens, body.len(), PEAK.load(Relaxed) - base)
}

fn symbols(count: usize) -> String {
    let mut out = String::from("[");
    for i in 0..count {
        if i > 0 {
            out.push(',');
        }
        let line = i * 3;
        out.push_str(&format!(
            r#"{{"name":"function_number_{i}","detail":"fn(&self, value: usize) -> usize","kind":12,"tags":[],"deprecated":false,"range":{{"start":{{"line":{line},"character":0}},"end":{{"line":{},"character":1}}}},"selectionRange":{{"start":{{"line":{line},"character":7}},"end":{{"line":{line},"character":24}}}},"children":[]}}"#,
            line + 2
        ));
    }
    out.push(']');
    out
}

fn repeated(open: &str, item: &str, count: usize, last: &str) -> String {
    format!("{open}{}{last}", item.repeat(count))
}

#[test]
fn no_shape_decodes_above_the_budgeted_cost_per_token() {
    let mut shapes: Vec<(String, String, bool)> = Vec::new();
    for count in [1_000, 6_500, 20_000] {
        shapes.push((format!("documentSymbol x{count}"), symbols(count), true));
    }
    shapes.push((
        "semantic tokens, 1M integers".into(),
        format!("{{\"data\":[{}0]}}", "12,".repeat(1_000_000)),
        false,
    ));
    // the adversarial shapes, on both sides of a capacity doubling
    for count in [131_072, 131_073, 262_144, 262_145] {
        let flat = [
            ("[[0],…]", repeated("[", "[0],", count, "[]]")),
            (
                "[{\"a\":[0]},…]",
                repeated("[", "{\"a\":[0]},", count, "[]]"),
            ),
            ("[{\"a\":0},…]", repeated("[", "{\"a\":0},", count, "{}]")),
            ("[0,…]", repeated("[", "0,", count, "0]")),
            ("[[[]],…]", repeated("[", "[[]],", count, "[]]")),
        ];
        for (name, result) in flat {
            shapes.push((format!("{name} x{count}"), result, false));
        }
        let keys: String = (0..count).map(|i| format!("\"{i}\":0,")).collect();
        shapes.push((
            format!("{{\"k\":0,…}} x{count}"),
            format!("{{{keys}\"z\":0}}"),
            false,
        ));
    }
    let mut worst = 0.0f64;
    for (name, result, typed) in &shapes {
        let (tokens, body, peak) = peak(result, *typed);
        let per_token = peak.saturating_sub(body) as f64 / tokens as f64;
        worst = worst.max(per_token);
        println!(
            "{name:32} tokens {tokens:>9} body {body:>9} peak {peak:>11} = {per_token:6.1} B/token"
        );
        assert!(
            peak <= tokens * DECODE_BYTES_PER_TOKEN + body,
            "{name}: {peak} bytes for {tokens} tokens and a {body}-byte body is over \
             {DECODE_BYTES_PER_TOKEN} B per token"
        );
    }
    println!(
        "worst {worst:.1} B/token; budget {DECODE_BYTES_PER_TOKEN}, {MAX_DECODE_TOKENS} tokens"
    );
}
