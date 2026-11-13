//! One pass over a frame body: how many JSON structure tokens decoding it
//! will allocate for, and enough of its top level to answer for it when it
//! is over the budget — whether it names a `method`, and its `id`.
use serde_json::Value;

/// The longest top-level `id` kept, in raw bytes. A longer one cannot be
/// answered and is treated as a request (0101 §4.2).
const MAX_ID_BYTES: usize = 128;
/// Top-level keys are compared raw; only `id` and `method` matter.
const MAX_KEY_BYTES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    Outside,
    Quoted,
    Escape,
}

/// Where the scan is in the top-level object.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Not inside a top-level object, or inside a nested value.
    None,
    /// Before a key (after `{` or `,`) or inside one.
    Key,
    /// The value of the top-level `id`.
    Id,
    /// The value of any other top-level key.
    Value,
}

/// What an over-budget frame is, as far as the bytes say.
#[derive(Debug, PartialEq)]
pub(super) enum Shape {
    /// A top-level `id` and no `method`: a reply, answered as an error.
    Response(Value),
    /// A `method` and no `id`: dropped.
    Notification,
    /// A server request, or a frame whose id cannot be read: nothing
    /// honest can answer it.
    Request,
}

pub(super) struct Scan {
    tokens: usize,
    depth: usize,
    quote: Quote,
    slot: Slot,
    object: bool,
    in_key: bool,
    key: Vec<u8>,
    id: Vec<u8>,
    has_id: bool,
    has_method: bool,
}

impl Default for Scan {
    fn default() -> Self {
        Self {
            tokens: 0,
            depth: 0,
            quote: Quote::Outside,
            slot: Slot::None,
            object: false,
            in_key: false,
            key: Vec::new(),
            id: Vec::new(),
            has_id: false,
            has_method: false,
        }
    }
}

impl Scan {
    /// `{ [ , :` outside strings: each one is a node or a separator the
    /// decoder allocates for.
    pub fn tokens(&self) -> usize {
        self.tokens
    }

    pub fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.byte(byte);
        }
    }

    fn byte(&mut self, byte: u8) {
        match self.quote {
            Quote::Escape => {
                self.quote = Quote::Quoted;
                self.keep(byte);
            }
            Quote::Quoted => {
                match byte {
                    b'\\' => self.quote = Quote::Escape,
                    b'"' => self.quote = Quote::Outside,
                    _ => {}
                }
                if self.in_key && self.quote == Quote::Outside {
                    self.in_key = false;
                } else {
                    self.keep(byte);
                }
            }
            Quote::Outside => self.structure(byte),
        }
    }

    fn structure(&mut self, byte: u8) {
        match byte {
            b'"' => {
                self.quote = Quote::Quoted;
                if self.depth == 1 && self.slot == Slot::Key {
                    self.in_key = true;
                    self.key.clear();
                } else {
                    self.keep(byte);
                }
            }
            b'{' | b'[' => {
                self.tokens += 1;
                if self.depth == 0 {
                    self.object = byte == b'{';
                    self.slot = if self.object { Slot::Key } else { Slot::None };
                } else {
                    self.keep(byte);
                }
                self.depth += 1;
            }
            b'}' | b']' => {
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 {
                    self.slot = Slot::None;
                } else {
                    self.keep(byte);
                }
            }
            b',' => {
                self.tokens += 1;
                if self.depth == 1 && self.slot != Slot::None {
                    self.slot = Slot::Key;
                } else {
                    self.keep(byte);
                }
            }
            b':' => {
                self.tokens += 1;
                if self.depth == 1 && self.slot == Slot::Key {
                    self.slot = match self.key.as_slice() {
                        b"id" => {
                            self.has_id = true;
                            self.id.clear();
                            Slot::Id
                        }
                        b"method" => {
                            self.has_method = true;
                            Slot::Value
                        }
                        _ => Slot::Value,
                    };
                } else {
                    self.keep(byte);
                }
            }
            _ => self.keep(byte),
        }
    }

    /// One byte of a key or of the id's value. The id keeps one byte past
    /// its bound, so a long id is known to be long.
    fn keep(&mut self, byte: u8) {
        if self.in_key {
            if self.key.len() <= MAX_KEY_BYTES {
                self.key.push(byte);
            }
        } else if self.slot == Slot::Id && self.id.len() <= MAX_ID_BYTES {
            self.id.push(byte);
        }
    }

    /// The frame's shape, once its last byte has been scanned.
    pub fn shape(&self) -> Shape {
        match (self.object, self.has_id, self.has_method) {
            (true, false, true) => Shape::Notification,
            (true, true, false) if self.id.len() <= MAX_ID_BYTES => {
                match serde_json::from_slice::<Value>(&self.id) {
                    Ok(id @ (Value::Number(_) | Value::String(_))) => Shape::Response(id),
                    _ => Shape::Request,
                }
            }
            _ => Shape::Request,
        }
    }
}

/// The structure tokens of one whole body, as the guard counts them.
pub fn structure_tokens(body: &[u8]) -> usize {
    let mut scan = Scan::default();
    scan.push(body);
    scan.tokens()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shape(body: &str) -> Shape {
        let mut scan = Scan::default();
        scan.push(body.as_bytes());
        scan.shape()
    }

    #[test]
    fn the_top_level_names_the_shape_wherever_the_id_sits() {
        assert_eq!(
            shape(r#"{"jsonrpc":"2.0","id":7,"result":[{"id":9,"method":"x"}]}"#),
            Shape::Response(json!(7))
        );
        assert_eq!(
            shape(r#"{"result":{"a":[1,{"b":"}"}]}, "id" : "r-\"1\"" }"#),
            Shape::Response(json!("r-\"1\""))
        );
        assert_eq!(
            shape(r#"{"method":"window/logMessage","params":{"id":3}}"#),
            Shape::Notification
        );
        assert_eq!(
            shape(r#"{"id":4,"method":"workspace/configuration","params":{}}"#),
            Shape::Request
        );
        assert_eq!(shape(r#"[{"id":1}]"#), Shape::Request, "a batch");
        assert_eq!(shape(r#"{"id":null,"result":1}"#), Shape::Request);
        assert_eq!(shape(r#"{"id":{"x":1},"result":1}"#), Shape::Request);
        let long = format!(r#"{{"id":"{}","result":1}}"#, "x".repeat(MAX_ID_BYTES));
        assert_eq!(shape(&long), Shape::Request, "an id past its bound");
    }

    #[test]
    fn quoted_structure_is_not_counted() {
        assert_eq!(structure_tokens(br#"{"a":"[,{}:\"","b":[1,2]}"#), 6);
    }
}
