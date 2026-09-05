//! Protocol surface (0009/0018): events, diagnostics, encodings,
//! location kinds — the types the editor and the client share.

use std::path::PathBuf;

/// One diagnostic, in UTF-8 buffer coordinates (the encoding boundary
/// lives here — 0009 §2.6).
#[derive(Debug, Clone)]
pub struct Diag {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub severity: u8, // 1 error, 2 warning, 3 info, 4 hint
    pub message: String,
}

impl Diag {
    pub fn severity_char(&self) -> char {
        match self.severity {
            1 => 'E',
            2 => 'W',
            3 => 'I',
            _ => 'H',
        }
    }
}

/// Events the editor drains from the channel.
/// Requests that can queue pre-init (0.6.1: gd on a freshly opened
/// project raced clangd's init and died silently).
#[derive(Debug, Clone, Copy)]
pub enum QueuedRequest {
    Goto,
    Hover,
    SwitchHeader,
    Locations(LocKind),
}

/// One pre-init request: target doc, position, kind.
pub struct PendingRequest {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub kind: QueuedRequest,
}

/// Position encoding negotiated with the server (LSP 3.17): strop is
/// byte-native and offers `utf-8` first; a server that only speaks
/// UTF-16 (the spec default) gets converted columns both ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// Byte column → server column for one line's text.
pub fn to_server_col(line: &str, byte_col: usize, enc: PositionEncoding) -> usize {
    match enc {
        PositionEncoding::Utf8 => byte_col,
        PositionEncoding::Utf16 => line
            .get(..byte_col)
            .unwrap_or(line)
            .chars()
            .map(|c| c.len_utf16())
            .sum(),
    }
}

/// Server column → byte column for one line's text.
pub fn to_byte_col(line: &str, server_col: usize, enc: PositionEncoding) -> usize {
    match enc {
        PositionEncoding::Utf8 => server_col,
        PositionEncoding::Utf16 => {
            let mut units = 0;
            for (i, c) in line.char_indices() {
                if units >= server_col {
                    return i;
                }
                units += c.len_utf16();
            }
            line.len()
        }
    }
}

/// Which location request a `Locations` event answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocKind {
    References,
    Implementation,
    TypeDefinition,
    Declaration,
}

impl LocKind {
    pub fn label(self) -> &'static str {
        match self {
            LocKind::References => "references",
            LocKind::Implementation => "implementation",
            LocKind::TypeDefinition => "type definition",
            LocKind::Declaration => "declaration",
        }
    }
}

pub enum LspEvent {
    Diagnostics {
        path: PathBuf,
        diags: Vec<Diag>,
        /// The server-side document version these were computed from
        /// (0018: converting old positions against new text misplaces
        /// them — the editor rejects older batches).
        version: Option<i32>,
    },
    Ready {
        server: &'static str,
    },
    Failed {
        server: &'static str,
        hint: &'static str,
    },
    HoverText {
        text: String,
    },
    GotoLocation {
        path: PathBuf,
        line: usize,
        col: usize,
    },
    /// references/implementation/typeDef/declaration results — many
    /// locations land in the picker, one jumps directly.
    Locations {
        kind: LocKind,
        items: Vec<(PathBuf, usize, usize)>,
    },
    /// A user-facing note that is neither ready nor failure (e.g.
    /// "no header counterpart").
    Note {
        text: String,
    },
}
