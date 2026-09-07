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
pub use client::Client;
pub use protocol::*;

#[cfg(test)]
mod tests {
    use super::{to_byte_col, to_server_col, PositionEncoding, ServerCaps, ServerColumn};
    use async_lsp::lsp_types::{
        DefinitionOptions, HoverProviderCapability, OneOf, ServerCapabilities,
    };
    use strop_core::id::ByteColumn;

    #[test]
    fn capabilities_gate_hover_and_goto() {
        let caps = ServerCaps::default();
        // Pre-initialize: capabilities unknown, requests must not race startup.
        assert!(!caps.hover());
        assert!(!caps.goto_definition());
        caps.set(ServerCapabilities::default());
        assert!(!caps.hover());
        assert!(!caps.goto_definition());
        caps.set(ServerCapabilities {
            definition_provider: Some(OneOf::Left(true)),
            ..Default::default()
        });
        assert!(!caps.hover());
        assert!(caps.goto_definition());
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
        let line = "aé🦀b"; // bytes: 1+2+4+1, utf16: 1+1+2+1
        assert_eq!(
            to_server_col(line, ByteColumn::new(7), PositionEncoding::Utf16).get(),
            4
        );
        assert_eq!(
            to_byte_col(line, ServerColumn::new(4), PositionEncoding::Utf16).get(),
            7
        );
        assert_eq!(
            to_server_col(line, ByteColumn::new(7), PositionEncoding::Utf8).get(),
            7
        );
        assert_eq!(
            to_byte_col(line, ServerColumn::new(7), PositionEncoding::Utf8).get(),
            7
        );
        assert_eq!(
            to_server_col(line, ByteColumn::new(3), PositionEncoding::Utf16).get(),
            2
        );
        assert_eq!(
            to_byte_col(line, ServerColumn::new(2), PositionEncoding::Utf16).get(),
            3
        );
        assert_eq!(
            to_byte_col(line, ServerColumn::new(99), PositionEncoding::Utf16).get(),
            line.len()
        );
        let comb = "e\u{0301}x";
        assert_eq!(
            to_server_col(comb, ByteColumn::new(3), PositionEncoding::Utf16).get(),
            2
        );
        assert_eq!(
            to_byte_col(comb, ServerColumn::new(2), PositionEncoding::Utf16).get(),
            3
        );
    }

    #[cfg(unix)]
    #[test]
    fn server_location_trace_round_trips_native_path_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let location = super::ServerLocation {
            path: std::path::PathBuf::from(std::ffi::OsString::from_vec(
                b"/workspace/a\xff.rs".to_vec(),
            )),
            position: super::ServerPosition {
                line: strop_core::id::LineIndex::new(2),
                column: super::ServerColumn::new(5),
            },
        };
        let encoded = serde_json::to_vec(&location).unwrap();
        let decoded: super::ServerLocation = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, location);
    }
}
