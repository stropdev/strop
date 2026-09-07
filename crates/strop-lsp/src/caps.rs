//! Server capability tracking (async_lsp hands them over in
//! initialize; consumers read the shared slot).

use std::sync::{Arc, Mutex};

use crate::protocol::PositionEncoding;

#[derive(Clone, Default)]
pub struct ServerCaps(Arc<Mutex<Option<async_lsp::lsp_types::ServerCapabilities>>>);

impl ServerCaps {
    pub(crate) fn set(&self, caps: async_lsp::lsp_types::ServerCapabilities) {
        let Ok(mut guard) = self.0.lock() else { return };
        *guard = Some(caps);
    }

    /// Hover supported? Capabilities not yet arrived (or a poisoned
    /// lock) count as no — requests never race server startup.
    pub fn hover(&self) -> bool {
        use async_lsp::lsp_types::HoverProviderCapability;
        let Ok(guard) = self.0.lock() else {
            return false;
        };
        matches!(
            guard.as_ref().and_then(|c| c.hover_provider.as_ref()),
            Some(HoverProviderCapability::Simple(true)) | Some(HoverProviderCapability::Options(_))
        )
    }

    /// Goto-definition supported? (same unknown-is-no rule as hover)
    pub fn goto_definition(&self) -> bool {
        use async_lsp::lsp_types::OneOf;
        let Ok(guard) = self.0.lock() else {
            return false;
        };
        matches!(
            guard.as_ref().and_then(|c| c.definition_provider.as_ref()),
            Some(OneOf::Left(true)) | Some(OneOf::Right(_))
        )
    }

    fn flag(&self, f: impl Fn(&async_lsp::lsp_types::ServerCapabilities) -> bool) -> bool {
        let Ok(guard) = self.0.lock() else {
            return false;
        };
        guard.as_ref().is_some_and(f)
    }

    pub fn references(&self) -> bool {
        self.flag(|c| {
            matches!(
                c.references_provider,
                Some(async_lsp::lsp_types::OneOf::Left(true))
                    | Some(async_lsp::lsp_types::OneOf::Right(_))
            )
        })
    }
    pub fn implementation(&self) -> bool {
        self.flag(|c| c.implementation_provider.is_some())
    }
    pub fn type_definition(&self) -> bool {
        self.flag(|c| c.type_definition_provider.is_some())
    }
    pub fn declaration(&self) -> bool {
        self.flag(|c| c.declaration_provider.is_some())
    }

    /// The negotiated column encoding (spec default UTF-16 until the
    /// initialize result says otherwise).
    pub fn encoding(&self) -> PositionEncoding {
        let Ok(guard) = self.0.lock() else {
            return PositionEncoding::Utf16;
        };
        match guard.as_ref().and_then(|c| c.position_encoding.as_ref()) {
            Some(k) if *k == async_lsp::lsp_types::PositionEncodingKind::UTF8 => {
                PositionEncoding::Utf8
            }
            _ => PositionEncoding::Utf16,
        }
    }

    /// Does the negotiated capability set admit this request kind?
    /// Unknown capabilities (pre-initialize) count as no — requests
    /// never race server startup.
    pub(crate) fn supports(&self, kind: crate::protocol::RequestKind) -> bool {
        use crate::protocol::{LocKind, RequestKind};
        match kind {
            RequestKind::Hover => self.hover(),
            RequestKind::Goto => self.goto_definition(),
            RequestKind::SwitchHeader => true,
            RequestKind::Locations(LocKind::References) => self.references(),
            RequestKind::Locations(LocKind::Implementation) => self.implementation(),
            RequestKind::Locations(LocKind::TypeDefinition) => self.type_definition(),
            RequestKind::Locations(LocKind::Declaration) => self.declaration(),
        }
    }
}
