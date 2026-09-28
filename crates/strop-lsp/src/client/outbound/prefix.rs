//! The local async-lsp encoder writes id/method before params. Read only those
//! bounded header fields, never copy or parse a didChange document body twice.
use super::{invalid, Method, RpcId};
use serde::Deserialize;
use std::io;

pub(super) const MAX_PREFIX_BYTES: usize = 1024;
pub(super) struct Head {
    pub id: Option<RpcId>,
    pub method: Method,
}

pub(super) fn decode(bytes: &[u8]) -> io::Result<Option<Head>> {
    let bytes = bytes.trim_ascii_start();
    if bytes.is_empty() {
        return Ok(None);
    }
    let Some(mut rest) = bytes.strip_prefix(b"{") else {
        return Err(invalid("outgoing JSON-RPC frame is not an object"));
    };
    let mut id = None;
    let mut method = None;
    loop {
        rest = rest.trim_ascii_start();
        let Some((key, used)) = atom::<&str>(rest)? else {
            return Ok(None);
        };
        rest = rest[used..].trim_ascii_start();
        if rest.is_empty() {
            return Ok(None);
        }
        let Some(after_colon) = rest.strip_prefix(b":") else {
            return Err(invalid("outgoing JSON-RPC header has no field separator"));
        };
        rest = after_colon.trim_ascii_start();
        match key {
            "params" => {
                if method.is_none() {
                    return Err(invalid("outgoing JSON-RPC method is missing before params"));
                }
                return finish(id, method).map(Some);
            }
            "result" | "error" => return finish(id, method).map(Some),
            "jsonrpc" => {
                let Some((version, used)) = atom::<&str>(rest)? else {
                    return Ok(None);
                };
                if version != "2.0" {
                    return Err(invalid("unexpected outgoing JSON-RPC version"));
                }
                rest = &rest[used..];
            }
            "id" => {
                let Some((value, used)) = atom::<RpcId>(rest)? else {
                    return Ok(None);
                };
                id = Some(value);
                rest = &rest[used..];
            }
            "method" => {
                let Some((name, used)) = atom::<&str>(rest)? else {
                    return Ok(None);
                };
                method = Some(Method::parse(name));
                rest = &rest[used..];
            }
            _ => {
                return Err(invalid(
                    "unexpected field before outgoing JSON-RPC identity",
                ))
            }
        }
        rest = rest.trim_ascii_start();
        match rest.first() {
            Some(b',') => rest = &rest[1..],
            Some(b'}') => return finish(id, method).map(Some),
            None => return Ok(None),
            _ => return Err(invalid("invalid outgoing JSON-RPC field boundary")),
        }
    }
}

fn finish(id: Option<RpcId>, method: Option<Method>) -> io::Result<Head> {
    if let (Some(RpcId::Number(id)), Some(_)) = (&id, method) {
        // async-lsp 0.2 uses a signed request counter. Close before its next
        // increment could wrap and alias an unanswered request on this peer.
        if *id >= i32::MAX - 1 || *id < 0 {
            return Err(invalid(
                "language-service JSON-RPC request identity space exhausted",
            ));
        }
    }
    Ok(Head {
        id,
        method: method.unwrap_or(Method::Other),
    })
}

fn atom<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> io::Result<Option<(T, usize)>> {
    let mut values = serde_json::Deserializer::from_slice(bytes).into_iter::<T>();
    match values.next() {
        Some(Ok(value)) => Ok(Some((value, values.byte_offset()))),
        None => Ok(None),
        Some(Err(error)) if error.is_eof() => Ok(None),
        Some(Err(_)) => Err(invalid("invalid outgoing JSON-RPC identity prefix")),
    }
}
