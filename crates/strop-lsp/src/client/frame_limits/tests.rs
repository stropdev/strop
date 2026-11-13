use super::*;
use std::task::Poll;
use tokio::io::AsyncReadExt;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    frame.extend_from_slice(body);
    frame
}

/// A body of `[0,0,…]` one token over the budget, wrapped as `head` + it +
/// `tail`.
fn flood(head: &str, tail: &str) -> Vec<u8> {
    let mut body = head.as_bytes().to_vec();
    body.push(b'[');
    body.extend(b"0,".repeat(MAX_DECODE_TOKENS));
    body.extend_from_slice(b"0]");
    body.extend_from_slice(tail.as_bytes());
    body
}

/// Split a reader's output back into frame bodies.
fn bodies(mut bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let end = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("a whole header")
            + 4;
        let length = content_length(&bytes[..end]).unwrap();
        out.push(bytes[end..end + length].to_vec());
        bytes = &bytes[end + length..];
    }
    out
}

/// Yields its bytes in the given chunks, then EOF.
struct Chunks(VecDeque<Vec<u8>>);
impl AsyncRead for Chunks {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(mut chunk) = this.0.pop_front() {
            let given = chunk.len().min(buffer.remaining());
            buffer.put_slice(&chunk[..given]);
            if given < chunk.len() {
                this.0.push_front(chunk.split_off(given));
            }
        }
        Poll::Ready(Ok(()))
    }
}

/// splitmix64: a seeded generator, so a failure names its case.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn oversized_declarations_fail_at_the_header_without_waiting_for_body_bytes() {
    block_on(async {
        for length in [MAX_FRAME_BYTES + 1, usize::MAX] {
            let header = format!("Content-Length: {length}\r\n\r\n");
            let mut reader = BoundedFrames::new(header.as_bytes());
            let mut received = [0; 128];
            let error = reader.read(&mut received).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
    });
}

#[test]
fn duplicate_lengths_cannot_bypass_the_allocation_bound() {
    for header in [
        "Content-Length: 1\r\nContent-Length: 33554433\r\n\r\n",
        "Content-Length: 33554433\r\nContent-Length: 1\r\n\r\n",
    ] {
        let mut guard = FrameGuard::default();
        let error = guard.push(header.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn structural_floods_are_answered_but_quoted_protocol_text_is_not_a_tree() {
    let quoted = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "result": "[,{}:\"".repeat(MAX_DECODE_TOKENS)
    }))
    .unwrap();
    let reply = flood(r#"{"jsonrpc":"2.0","id":2,"result":"#, "}");
    let mut input = framed(&quoted);
    input.extend(framed(&reply));
    let output = block_on(async {
        let mut out = Vec::new();
        BoundedFrames::new(input.as_slice())
            .read_to_end(&mut out)
            .await
            .unwrap();
        out
    });
    let bodies = bodies(&output);
    assert_eq!(bodies[0], quoted, "a string is not a tree: released whole");
    let answer: serde_json::Value = serde_json::from_slice(&bodies[1]).unwrap();
    assert_eq!(answer["id"], 2);
    assert_eq!(answer["error"]["code"], -32603);
    assert_eq!(
        answer["error"]["message"],
        format!(
            "reply too large to decode: {} tokens, limit 1315860 (512 MiB)",
            structure_tokens(&reply)
        )
    );
}

#[test]
fn an_over_budget_notification_is_dropped_once_reported_and_the_next_frame_read() {
    let notification = flood(r#"{"jsonrpc":"2.0","method":"$/flood","params":"#, "}");
    let next = br#"{"jsonrpc":"2.0","id":3,"result":null}"#;
    let mut input = framed(&notification);
    input.extend(framed(&notification));
    input.extend(framed(next));
    let reports = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let output = block_on(async {
        let mut out = Vec::new();
        let seen = reports.clone();
        BoundedFrames::new(input.as_slice())
            .reporting(move |tokens| seen.lock().push(tokens))
            .read_to_end(&mut out)
            .await
            .unwrap();
        out
    });
    assert_eq!(bodies(&output), vec![next.to_vec()]);
    assert_eq!(
        *reports.lock(),
        vec![structure_tokens(&notification)],
        "said once"
    );
}

#[test]
fn an_over_budget_server_request_ends_the_connection() {
    let request = flood(
        r#"{"jsonrpc":"2.0","id":9,"method":"workspace/configuration","params":"#,
        "}",
    );
    let input = framed(&request);
    let error = block_on(async {
        let mut out = Vec::new();
        BoundedFrames::new(input.as_slice())
            .read_to_end(&mut out)
            .await
            .unwrap_err()
    });
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn output_ending_inside_a_frame_is_an_error_not_a_clean_end() {
    let mut input = framed(br#"{"jsonrpc":"2.0","id":1,"result":null}"#);
    input.truncate(input.len() - 3);
    let error = block_on(async {
        let mut out = Vec::new();
        BoundedFrames::new(input.as_slice())
            .read_to_end(&mut out)
            .await
            .unwrap_err()
    });
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

/// A small JSON value with structure inside and outside strings, escapes
/// and multi-byte text.
fn value(rng: &mut Rng, depth: usize) -> String {
    match rng.below(if depth > 2 { 3 } else { 6 }) {
        0 => rng.below(1000).to_string(),
        1 => ["\"\"", "\"[,{}:\\\"\"", "\"\\\\\"", "\"é😀\""][rng.below(4)].into(),
        2 => "null".into(),
        3 | 4 => {
            let items: Vec<String> = (0..rng.below(4)).map(|_| value(rng, depth + 1)).collect();
            format!("[{}]", items.join(","))
        }
        _ => {
            let items: Vec<String> = (0..rng.below(4))
                .map(|i| format!("\"k{i}\":{}", value(rng, depth + 1)))
                .collect();
            format!("{{{}}}", items.join(","))
        }
    }
}

#[test]
fn any_split_of_any_frames_comes_out_admitted_in_order_or_answered() {
    for seed in 0..24u64 {
        let mut rng = Rng(seed);
        let mut input = Vec::new();
        let mut expected = Vec::new();
        let mut flooded = false;
        for frame in 0..1 + rng.below(8) {
            let id = if rng.below(2) == 0 {
                frame.to_string()
            } else {
                format!("\"r{frame}\"")
            };
            let payload = value(&mut rng, 0);
            let body = match rng.below(if flooded { 3 } else { 5 }) {
                0 => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{payload}}}"#),
                1 => format!(r#"{{"result":{payload},"jsonrpc":"2.0","id":{id}}}"#),
                2 => format!(r#"{{"jsonrpc":"2.0","method":"m","params":{payload}}}"#),
                3 => {
                    flooded = true;
                    let body = flood(r#"{"jsonrpc":"2.0","result":"#, &format!(r#","id":{id}}}"#));
                    let tokens = structure_tokens(&body);
                    expected.extend(too_large(&serde_json::from_str(&id).unwrap(), tokens));
                    input.extend(framed(&body));
                    continue;
                }
                _ => {
                    flooded = true;
                    input.extend(framed(&flood(
                        r#"{"jsonrpc":"2.0","method":"m","params":"#,
                        "}",
                    )));
                    continue;
                }
            };
            expected.extend(framed(body.as_bytes()));
            input.extend(framed(body.as_bytes()));
        }
        let mut chunks = VecDeque::new();
        let mut rest = input.as_slice();
        while !rest.is_empty() {
            let largest = if rng.below(4) == 0 { 8 } else { 70_000 };
            let size = (1 + rng.below(largest)).min(rest.len());
            chunks.push_back(rest[..size].to_vec());
            rest = &rest[size..];
        }
        let output = block_on(async {
            let mut reader = BoundedFrames::new(Chunks(chunks));
            let mut out = Vec::new();
            loop {
                let mut room = vec![0; 1 + rng.below(4096)];
                let read = reader.read(&mut room).await.unwrap();
                if read == 0 {
                    break out;
                }
                out.extend_from_slice(&room[..read]);
            }
        });
        assert!(output == expected, "seed {seed}: frames differ");
        for body in bodies(&output) {
            serde_json::from_slice::<serde_json::Value>(&body)
                .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
        }
    }
}
