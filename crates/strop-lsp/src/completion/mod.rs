//! Completion-specific wire values, bounded decoding and resolve preservation.
//! No editor state, source mutation or rendering lives at this boundary.
mod decode;
mod filter;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use strop_core::id::ByteColumn;

pub use async_lsp::lsp_types::{
    CompletionItemKind, CompletionTextEdit, Documentation, InsertTextFormat, InsertTextMode,
    MarkupKind, Position, Range,
};
pub use decode::{decode_list, decode_resolved};
pub use filter::CompletionFilter;

pub const MAX_COMPLETION_ITEMS: usize = 128;
pub const MAX_COMPLETION_ITEM_BYTES: usize = 64 * 1024;
pub const MAX_COMPLETION_BYTES: usize = 512 * 1024;
pub const MAX_COMPLETION_ITEM_NODES: usize = 4096;
pub const MAX_COMPLETION_NODES: usize = 32 * 1024;
pub const MAX_FILTER_BYTES: usize = 1024;

/// Preserve unknown server extension fields as well as opaque data and tags.
/// Resolve sends this exact item, not a label-only reconstruction.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CompletionItem {
    #[serde(flatten)]
    protocol: async_lsp::lsp_types::CompletionItem,
    #[serde(flatten)]
    extensions: BTreeMap<String, serde_json::Value>,
}

/// Construction is bounded, and callers only receive readonly access. Resolve
/// admission can therefore charge the known maximum without serializing or
/// walking opaque server data on the interactive thread.
impl<'de> serde::Deserialize<'de> for CompletionItem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
        decode::deserialize_item(value).map_err(serde::de::Error::custom)
    }
}

#[derive(serde::Deserialize)]
struct WireItem {
    #[serde(flatten)]
    protocol: async_lsp::lsp_types::CompletionItem,
    #[serde(flatten)]
    extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CompletionTrigger {
    Invoked,
    Character(char),
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CompletionCall {
    Query {
        trigger: CompletionTrigger,
        prefix_start: ByteColumn,
    },
    Resolve {
        item: Arc<CompletionItem>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingCompletion {
    pub stamp: crate::RequestStamp,
    pub input: crate::RequestInput,
    pub call: CompletionCall,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompletionEntry {
    /// Original response ordinal, retained through filtering/sorting. Equal
    /// labels are never collapsed into the same candidate identity.
    pub ordinal: u32,
    pub item: Arc<CompletionItem>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompletionList {
    pub items: Vec<CompletionEntry>,
    pub server_incomplete: bool,
    pub client_truncated: bool,
    pub omitted_oversized: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CompletionResult {
    List(CompletionList),
    Resolved(Arc<CompletionItem>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionReply {
    pub context: crate::ReplyContext,
    pub outcome: strop_core::worker::Outcome<CompletionResult>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CompletionTransportEvent {
    Reply(CompletionReply),
    /// A physical request settled; a newest query refused as busy may retry.
    Capacity {
        server: crate::ServerId,
    },
    Closed {
        server: crate::ServerId,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompletionUsage {
    pub physical_requests: usize,
    pub captured_bytes: usize,
}

/// Invoked only by the LSP runtime/ordered wire, never the interactive thread.
/// The receiver owns newest-query admission and independent source freshness.
pub type CompletionSink = Arc<dyn Fn(CompletionTransportEvent) + Send + Sync>;

/// A metadata-only capacity wake. Clients keep only a Weak lease; an earlier
/// cancelled request cannot keep a disabled completion owner alive.
pub type CompletionWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CompletionDecodeError {
    #[error("language server returned a malformed completion list")]
    List,
    #[error("language server returned a malformed completion item")]
    Item,
    #[error("language server used unadvertised completion list defaults")]
    ListDefaults,
    #[error("completion item exceeds the retained data bound")]
    ItemLimit,
    #[error("completion filter position is invalid or exceeds its bounded prefix")]
    FilterPosition,
    #[error("completion resolve changed a non-resolvable edit or command")]
    ResolveChangedOperation,
    #[error("completion decoding was cancelled")]
    Cancelled,
}

impl CompletionItem {
    pub fn label(&self) -> &str {
        &self.protocol.label
    }
    pub fn filter_text(&self) -> &str {
        self.protocol
            .filter_text
            .as_deref()
            .filter(|text| !text.is_empty())
            .unwrap_or(&self.protocol.label)
    }
    pub fn sort_text(&self) -> &str {
        self.protocol
            .sort_text
            .as_deref()
            .filter(|text| !text.is_empty())
            .unwrap_or(&self.protocol.label)
    }
    /// The same matching policy serves initial decoding and safe complete-list
    /// reuse. Normalization affects matching, never the represented insertion.
    pub fn matches_prefix(&self, prefix: impl Iterator<Item = char>) -> bool {
        let mut wanted = prefix.flat_map(char::to_lowercase);
        let mut next = wanted.next();
        for character in self.filter_text().chars().flat_map(char::to_lowercase) {
            if Some(character) == next {
                next = wanted.next();
            }
            if next.is_none() {
                return true;
            }
        }
        next.is_none()
    }

    pub fn protocol(&self) -> &async_lsp::lsp_types::CompletionItem {
        &self.protocol
    }
}

/// Advertise only the supported operation/presentation contract. In particular,
/// preselection cannot silently turn Enter/Tab into acceptance, and snippets or
/// commit characters never masquerade as plain-text completion.
pub(crate) fn client_capabilities() -> async_lsp::lsp_types::CompletionClientCapabilities {
    use async_lsp::lsp_types as lt;
    use CompletionItemKind as Kind;
    lt::CompletionClientCapabilities {
        dynamic_registration: Some(false),
        context_support: Some(true),
        insert_text_mode: Some(InsertTextMode::AS_IS),
        completion_item: Some(lt::CompletionItemCapability {
            snippet_support: Some(false),
            commit_characters_support: Some(false),
            documentation_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
            deprecated_support: Some(false),
            preselect_support: Some(false),
            insert_replace_support: Some(true),
            resolve_support: Some(lt::CompletionItemCapabilityResolveSupport {
                properties: vec![
                    "detail".into(),
                    "documentation".into(),
                    "additionalTextEdits".into(),
                ],
            }),
            insert_text_mode_support: Some(lt::InsertTextModeSupport {
                value_set: vec![InsertTextMode::AS_IS],
            }),
            label_details_support: Some(false),
            ..Default::default()
        }),
        completion_item_kind: Some(lt::CompletionItemKindCapability {
            value_set: Some(vec![
                Kind::TEXT,
                Kind::METHOD,
                Kind::FUNCTION,
                Kind::CONSTRUCTOR,
                Kind::FIELD,
                Kind::VARIABLE,
                Kind::CLASS,
                Kind::INTERFACE,
                Kind::MODULE,
                Kind::PROPERTY,
                Kind::UNIT,
                Kind::VALUE,
                Kind::ENUM,
                Kind::KEYWORD,
                Kind::SNIPPET,
                Kind::COLOR,
                Kind::FILE,
                Kind::REFERENCE,
                Kind::FOLDER,
                Kind::ENUM_MEMBER,
                Kind::CONSTANT,
                Kind::STRUCT,
                Kind::EVENT,
                Kind::OPERATOR,
                Kind::TYPE_PARAMETER,
            ]),
        }),
        completion_list: Some(lt::CompletionListCapability {
            item_defaults: Some(Vec::new()),
        }),
    }
}
