//! strop-lsp: the LSP client. async-lsp transport (0009 §2.1), tokio on
//! a worker thread, the editor sees a channel of typed events — never an
//! async type, never an await in the input path (0001 §5.6).
pub mod languages;

mod caps;
mod client;
mod convert;
pub mod protocol;
pub mod registry;

pub use caps::ServerCaps;
pub use client::{log_line, Client};
pub use protocol::*;

#[cfg(test)]
mod tests {
    use super::{to_byte_col, to_server_col, PositionEncoding, ServerCaps};
    use async_lsp::lsp_types::{
        DefinitionOptions, HoverProviderCapability, OneOf, ServerCapabilities,
    };

    #[test]
    fn capabilities_gate_hover_and_goto() {
        let caps = ServerCaps::default();
        // pre-initialize: capabilities unknown → requests must not race
        // server startup
        assert!(!caps.hover());
        assert!(!caps.goto_definition());

        // initialized, nothing advertised → both gated
        caps.set(ServerCapabilities::default());
        assert!(!caps.hover());
        assert!(!caps.goto_definition());

        // definition only → hover stays dropped
        caps.set(ServerCapabilities {
            definition_provider: Some(OneOf::Left(true)),
            ..Default::default()
        });
        assert!(!caps.hover());
        assert!(caps.goto_definition());

        // hover on, definition explicitly off
        caps.set(ServerCapabilities {
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(false)),
            ..Default::default()
        });
        assert!(caps.hover());
        assert!(!caps.goto_definition());
    }

    #[test]
    fn option_shaped_providers_count_as_enabled() {
        let caps = ServerCaps::default();
        caps.set(ServerCapabilities {
            hover_provider: Some(HoverProviderCapability::Options(Default::default())),
            definition_provider: Some(OneOf::Right(DefinitionOptions {
                work_done_progress_options: Default::default(),
            })),
            ..Default::default()
        });
        assert!(caps.hover());
        assert!(caps.goto_definition());
    }

    #[test]
    fn column_encoding_roundtrips_unicode() {
        // the LSP wire is UTF-16 unless negotiated; strop is byte-native
        let line = "aé🦀b"; // bytes: 1+2+4+1, utf16: 1+1+2+1
                            // byte col of 'b' = 7; utf-16 col = 4
        assert_eq!(to_server_col(line, 7, PositionEncoding::Utf16), 4);
        assert_eq!(to_byte_col(line, 4, PositionEncoding::Utf16), 7);
        assert_eq!(to_server_col(line, 7, PositionEncoding::Utf8), 7);
        assert_eq!(to_byte_col(line, 7, PositionEncoding::Utf8), 7);
        // inside the emoji (byte 3..7): utf16 col 2..4
        assert_eq!(to_server_col(line, 3, PositionEncoding::Utf16), 2);
        assert_eq!(to_server_col(line, 7, PositionEncoding::Utf16), 4);
        assert_eq!(to_byte_col(line, 2, PositionEncoding::Utf16), 3);
        // past-the-end clamps
        assert_eq!(to_byte_col(line, 99, PositionEncoding::Utf16), line.len());
        // combining marks: e + U+0301 is 3 bytes, 2 utf-16 units
        let comb = "e\u{0301}x";
        assert_eq!(to_server_col(comb, 3, PositionEncoding::Utf16), 2);
        assert_eq!(to_byte_col(comb, 2, PositionEncoding::Utf16), 3);
    }
}
