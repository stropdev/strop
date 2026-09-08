//! Shared request ownership, diagnostics and negotiated coordinate domains.
use crate::target::DocPath;
use std::path::PathBuf;
use strop_core::id::{BufferRevision, ByteColumn, DocumentId, LineIndex};

/// Diagnostic severity (R13): a named domain, never a raw u8. Variant
/// order matches the LSP rank, so `min_by_key` keeps the worst entry.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    /// The LSP wire code (1=error … 4=hint) — display/trace boundary only.
    pub const fn code(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warning => 2,
            Self::Information => 3,
            Self::Hint => 4,
        }
    }

    /// Gutter/picker letter.
    pub const fn char(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warning => 'W',
            Self::Information => 'I',
            Self::Hint => 'H',
        }
    }
}

/// A text-document wire version on one connection. Monotonic across
/// reopens so a stale versioned diagnostic can never relabel itself as
/// belonging to a new document incarnation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct WireVersion(i32);

impl WireVersion {
    pub const fn new(value: i32) -> Self {
        Self(value)
    }
    pub const fn get(self) -> i32 {
        self.0
    }
    pub(crate) fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// A diagnostic as the server sent it: server-domain columns until the
/// editor resolves them against its rope with the negotiated encoding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diag {
    pub line: LineIndex,
    pub col: ServerColumn,
    pub end_line: LineIndex,
    pub end_col: ServerColumn,
    pub severity: Severity,
    pub message: String,
}

impl Diag {
    /// Convert to the editor's byte domain. Lines beyond the current
    /// document (the server computed on older content) clamp instead of
    /// panicking; empty documents resolve to line 0.
    pub fn resolve(self, encoding: PositionEncoding, buffer: &strop_core::Buffer) -> ResolvedDiag {
        let last = buffer.len_lines().saturating_sub(1);
        let line = LineIndex::new(self.line.get().min(last));
        let end_line = LineIndex::new(self.end_line.get().min(last));
        let start_text = buffer.line_text(line);
        let end_text = if end_line == line {
            start_text.clone()
        } else {
            buffer.line_text(end_line)
        };
        ResolvedDiag {
            line,
            col: to_byte_col(&start_text, self.col, encoding),
            end_line,
            end_col: to_byte_col(&end_text, self.end_col, encoding),
            severity: self.severity,
            message: self.message,
        }
    }
}

/// A diagnostic in the editor's byte domain: columns are UTF-8 byte
/// offsets into the line, ready for gutter/underline math.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedDiag {
    pub line: LineIndex,
    pub col: ByteColumn,
    pub end_line: LineIndex,
    pub end_col: ByteColumn,
    pub severity: Severity,
    pub message: String,
}

impl ResolvedDiag {
    pub fn severity_char(&self) -> char {
        self.severity.char()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ServerId(u64);
impl ServerId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub(crate) fn allocate() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        match NEXT.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |n| n.checked_add(1),
        ) {
            Ok(value) => Self(value),
            Err(_) => panic!("LSP server identity exhausted"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct RequestId(u64);
impl RequestId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RequestStamp {
    pub request: RequestId,
    pub server: ServerId,
    pub document: DocumentId,
    pub revision: BufferRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// Byte column → server column for one line's text.
pub fn to_server_col(line: &str, byte_col: ByteColumn, enc: PositionEncoding) -> ServerColumn {
    ServerColumn::new(match enc {
        PositionEncoding::Utf8 => byte_col.get(),
        PositionEncoding::Utf16 => {
            let prefix = match line.get(..byte_col.get()) {
                Some(prefix) => prefix,
                None => line,
            };
            prefix.chars().map(char::len_utf16).sum()
        }
    })
}

/// Server column → byte column for one line's text.
pub fn to_byte_col(line: &str, server_col: ServerColumn, enc: PositionEncoding) -> ByteColumn {
    ByteColumn::new(match enc {
        PositionEncoding::Utf8 => server_col.get(),
        PositionEncoding::Utf16 => {
            let mut units = 0;
            for (i, c) in line.char_indices() {
                if units >= server_col.get() {
                    return ByteColumn::new(i);
                }
                units += c.len_utf16();
            }
            line.len()
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LocKind {
    References,
    Implementation,
    TypeDefinition,
    Declaration,
}
impl LocKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::References => "references",
            Self::Implementation => "implementation",
            Self::TypeDefinition => "type definition",
            Self::Declaration => "declaration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RequestKind {
    Goto,
    Hover,
    SwitchHeader,
    Locations(LocKind),
}

impl RequestKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Goto => "goto definition",
            Self::Hover => "hover",
            Self::SwitchHeader => "switch source/header",
            Self::Locations(kind) => kind.label(),
        }
    }
}

/// Why a request was never admitted (R9: no silent `None`). Refused
/// requests get no stamp and no wire traffic; the caller reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RequestRefusal {
    /// The document is not open on this connection.
    NotOpen,
    /// The buffer moved past the captured revision — re-request.
    StaleRevision,
    /// The server advertised no provider for this request kind.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ServerColumn(usize);
impl ServerColumn {
    pub const fn new(value: usize) -> Self {
        Self(value)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServerPosition {
    pub line: LineIndex,
    pub column: ServerColumn,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServerLocation {
    /// Endpoint-scoped identity of the target: a remote location can
    /// never alias the analogous local path.
    pub doc: DocPath,
    pub position: ServerPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplyContext {
    pub stamp: RequestStamp,
    pub encoding: PositionEncoding,
    pub kind: RequestKind,
}

/// Source position remains byte-native until initialize negotiates
/// encoding. Serializable: the replay tape records admissions and
/// relaunches against the identical payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RequestInput {
    pub document: DocumentId,
    pub revision: BufferRevision,
    #[serde(with = "strop_core::path_serde")]
    pub path: PathBuf,
    pub line: LineIndex,
    pub byte_col: ByteColumn,
    pub line_text: String,
    pub kind: RequestKind,
}

/// An admitted request: its owning stamp plus the captured input.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingRequest {
    pub stamp: RequestStamp,
    pub input: RequestInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiagnosticContext {
    pub server: ServerId,
    pub document: DocumentId,
    /// Current sent revision at receipt, not proof of computation freshness
    /// for versionless diagnostics.
    pub revision: BufferRevision,
    pub encoding: PositionEncoding,
    pub version: Option<WireVersion>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum LspEvent {
    Diagnostics {
        context: DiagnosticContext,
        /// The diagnosed document: endpoint-scoped, so remote
        /// diagnostics never collide with same-bytes local paths.
        doc: DocPath,
        diags: Vec<Diag>,
    },
    Ready {
        server: ServerId,
        name: String,
    },
    Failed {
        server: ServerId,
        name: String,
        hint: String,
    },
    HoverText {
        context: ReplyContext,
        text: String,
    },
    GotoLocation {
        context: ReplyContext,
        location: ServerLocation,
    },
    Locations {
        context: ReplyContext,
        kind: LocKind,
        items: Vec<ServerLocation>,
    },
    /// Terminal note: empty result, typed failure or cancellation. The
    /// context is the ORIGINAL request's — never re-derived.
    Note {
        context: ReplyContext,
        text: String,
    },
}
