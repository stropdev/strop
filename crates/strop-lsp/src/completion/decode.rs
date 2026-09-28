use super::{
    CompletionDecodeError as Error, CompletionEntry, CompletionFilter, CompletionItem,
    CompletionList, MAX_COMPLETION_BYTES, MAX_COMPLETION_ITEMS, MAX_COMPLETION_ITEM_BYTES,
    MAX_COMPLETION_ITEM_NODES, MAX_COMPLETION_NODES,
};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

#[derive(Default, Clone, Copy)]
struct Footprint {
    bytes: usize,
    nodes: usize,
}

/// Bound each item's data before typed decoding. The enclosing framed reader
/// separately bounds allocation of the initial JSON Value tree.
fn footprint(value: &Value, depth: usize, size: &mut Footprint) -> Result<(), Error> {
    size.nodes += 1;
    if depth > 64 || size.nodes > MAX_COMPLETION_ITEM_NODES {
        return Err(Error::ItemLimit);
    }
    match value {
        Value::String(text) => size.bytes += text.len(),
        Value::Array(items) => {
            for item in items {
                footprint(item, depth + 1, size)?;
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                size.bytes += key.len();
                if size.bytes > MAX_COMPLETION_ITEM_BYTES {
                    return Err(Error::ItemLimit);
                }
                footprint(item, depth + 1, size)?;
            }
        }
        _ => {}
    }
    if size.bytes > MAX_COMPLETION_ITEM_BYTES {
        Err(Error::ItemLimit)
    } else {
        Ok(())
    }
}

fn item(value: Value) -> Result<(CompletionItem, Footprint), Error> {
    let mut size = Footprint::default();
    footprint(&value, 0, &mut size)?;
    let decoded: super::WireItem = serde_json::from_value(value).map_err(|_| Error::Item)?;
    Ok((
        CompletionItem {
            protocol: decoded.protocol,
            extensions: decoded.extensions,
        },
        size,
    ))
}

pub(super) fn deserialize_item(value: Value) -> Result<CompletionItem, Error> {
    item(value).map(|(item, _)| item)
}

/// Called on the language-service runtime. The retained window is ranked after
/// filtering, so a useful item late in a large server list is not lost just
/// because unrelated items arrived first. Server completeness stays independent
/// from client candidate/count/byte limits.
pub fn decode_list(
    value: Value,
    filter: &CompletionFilter<'_>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<CompletionList, Error> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let (server_incomplete, values) = match value {
        Value::Null => (false, Vec::new()),
        Value::Array(values) => (false, values),
        Value::Object(mut fields) => {
            if fields.remove("itemDefaults").is_some_and(|defaults| {
                !defaults.is_null()
                    && !matches!(&defaults, Value::Object(fields) if fields.is_empty())
            }) {
                return Err(Error::ListDefaults);
            }
            let incomplete = fields
                .remove("isIncomplete")
                .and_then(|value| value.as_bool())
                .ok_or(Error::List)?;
            let Some(Value::Array(values)) = fields.remove("items") else {
                return Err(Error::List);
            };
            (incomplete, values)
        }
        _ => return Err(Error::List),
    };
    let mut best = BinaryHeap::with_capacity(MAX_COMPLETION_ITEMS);
    let mut matches = 0usize;
    let mut omitted_oversized = 0usize;
    for (ordinal, value) in values.into_iter().enumerate() {
        if ordinal % 32 == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        let (item, footprint) = match item(value) {
            Ok(item) => item,
            Err(Error::ItemLimit) => {
                omitted_oversized += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        if !filter.matches(&item)? {
            continue;
        }
        matches += 1;
        let candidate = RankedItem {
            ordinal,
            item,
            footprint,
        };
        if best.len() < MAX_COMPLETION_ITEMS {
            best.push(candidate);
        } else if best.peek().is_some_and(|worst| candidate < *worst) {
            best.pop();
            best.push(candidate);
        }
    }
    let mut items = Vec::with_capacity(best.len());
    let mut retained = Footprint::default();
    let mut client_truncated = matches > MAX_COMPLETION_ITEMS || omitted_oversized > 0;
    for candidate in best.into_sorted_vec() {
        if retained.bytes + candidate.footprint.bytes > MAX_COMPLETION_BYTES
            || retained.nodes + candidate.footprint.nodes > MAX_COMPLETION_NODES
        {
            client_truncated = true;
            continue;
        }
        retained.bytes += candidate.footprint.bytes;
        retained.nodes += candidate.footprint.nodes;
        items.push(CompletionEntry {
            ordinal: u32::try_from(candidate.ordinal).map_err(|_| Error::List)?,
            item: Arc::new(candidate.item),
        });
    }
    Ok(CompletionList {
        items,
        server_incomplete,
        client_truncated,
        omitted_oversized,
    })
}

/// Missing immutable fields in a resolve reply retain their original meaning;
/// the engine overlays only the advertised detail/documentation/additional-edit
/// properties. A newly introduced or changed primary edit/command never becomes
/// an unreviewed operation hidden behind documentation resolution.
pub fn decode_resolved(
    value: Value,
    original: &CompletionItem,
) -> Result<Arc<CompletionItem>, Error> {
    let (resolved, _) = item(value)?;
    let before = &original.protocol;
    let after = &resolved.protocol;
    if after.label != before.label
        || after
            .text_edit
            .as_ref()
            .is_some_and(|edit| Some(edit) != before.text_edit.as_ref())
        || after.insert_text.as_deref().is_some_and(|text| {
            let text = if text.is_empty() {
                after.label.as_str()
            } else {
                text
            };
            text != before
                .insert_text
                .as_deref()
                .filter(|text| !text.is_empty())
                .unwrap_or(&before.label)
        })
        || after.insert_text_format.is_some_and(|format| {
            format
                != before
                    .insert_text_format
                    .unwrap_or(super::InsertTextFormat::PLAIN_TEXT)
        })
        || after.insert_text_mode.is_some_and(|mode| {
            mode != before
                .insert_text_mode
                .unwrap_or(super::InsertTextMode::AS_IS)
        })
        || after
            .command
            .as_ref()
            .is_some_and(|command| Some(command) != before.command.as_ref())
    {
        return Err(Error::ResolveChangedOperation);
    }
    Ok(Arc::new(resolved))
}

struct RankedItem {
    ordinal: usize,
    item: CompletionItem,
    footprint: Footprint,
}
impl Ord for RankedItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.item
            .sort_text()
            .cmp(other.item.sort_text())
            .then_with(|| self.ordinal.cmp(&other.ordinal))
    }
}
impl PartialOrd for RankedItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for RankedItem {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for RankedItem {}
