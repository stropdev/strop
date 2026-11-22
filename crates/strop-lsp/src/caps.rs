//! Server capability tracking (async_lsp hands them over in
//! initialize; consumers read the shared slot).

use std::sync::{Arc, Mutex};

use async_lsp::lsp_types::{
    CodeActionProviderCapability, DeclarationCapability, HoverProviderCapability,
    ImplementationProviderCapability, OneOf, ServerCapabilities, TypeDefinitionProviderCapability,
};

use crate::protocol::PositionEncoding;

#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
pub struct ServerCaps(Arc<Mutex<Option<ServerCapabilities>>>);

impl ServerCaps {
    pub(crate) fn set(&self, caps: ServerCapabilities) {
        let Ok(mut guard) = self.0.lock() else { return };
        *guard = Some(caps);
    }

    /// Capabilities not yet arrived (or a poisoned lock) count as no —
    /// requests never race server startup.
    fn flag(&self, f: impl Fn(&ServerCapabilities) -> bool) -> bool {
        let Ok(guard) = self.0.lock() else {
            return false;
        };
        guard.as_ref().is_some_and(f)
    }

    pub fn hover(&self) -> bool {
        self.flag(|c| enabled(&c.hover_provider))
    }
    pub fn goto_definition(&self) -> bool {
        self.flag(|c| enabled(&c.definition_provider))
    }
    pub fn references(&self) -> bool {
        self.flag(|c| enabled(&c.references_provider))
    }
    pub fn implementation(&self) -> bool {
        self.flag(|c| enabled(&c.implementation_provider))
    }
    pub fn type_definition(&self) -> bool {
        self.flag(|c| enabled(&c.type_definition_provider))
    }
    pub fn declaration(&self) -> bool {
        self.flag(|c| enabled(&c.declaration_provider))
    }
    pub fn workspace_symbols(&self) -> bool {
        self.flag(|c| enabled(&c.workspace_symbol_provider))
    }
    pub fn document_symbols(&self) -> bool {
        self.flag(|c| enabled(&c.document_symbol_provider))
    }
    pub fn formatting(&self) -> bool {
        self.flag(|c| enabled(&c.document_formatting_provider))
    }
    /// `workspace/willRenameFiles` advertised
    /// (`workspace.fileOperations.willRename`).
    pub fn will_rename_files(&self) -> bool {
        self.flag(|c| {
            c.workspace
                .as_ref()
                .and_then(|workspace| workspace.file_operations.as_ref())
                .is_some_and(|operations| operations.will_rename.is_some())
        })
    }
    pub fn rename(&self) -> bool {
        self.flag(|c| enabled(&c.rename_provider))
    }
    pub fn code_action(&self) -> bool {
        self.flag(|c| enabled(&c.code_action_provider))
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

    pub fn completion(&self) -> bool {
        self.flag(|caps| caps.completion_provider.is_some())
    }

    pub fn completion_resolve(&self) -> bool {
        self.flag(|caps| {
            caps.completion_provider
                .as_ref()
                .is_some_and(|provider| provider.resolve_provider == Some(true))
        })
    }

    pub fn completion_trigger(&self, character: char) -> bool {
        self.flag(|caps| {
            caps.completion_provider
                .as_ref()
                .and_then(|provider| provider.trigger_characters.as_ref())
                .is_some_and(|triggers| {
                    triggers
                        .iter()
                        .any(|trigger| trigger.chars().eq(std::iter::once(character)))
                })
        })
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
            RequestKind::Format => self.formatting(),
            RequestKind::Rename => self.rename(),
            RequestKind::CodeAction => self.code_action(),
            RequestKind::DocumentSymbols => self.document_symbols(),
            RequestKind::WillRenameFiles => self.will_rename_files(),
            RequestKind::WorkspaceSymbols => self.workspace_symbols(),
            RequestKind::Completion => self.completion(),
            RequestKind::CompletionResolve => self.completion_resolve(),
        }
    }
}

/// A provider the protocol lets a server answer with a boolean or an options
/// object: `true` or any options object enables it, `false` does not.
trait Provider {
    fn enabled(&self) -> bool;
}

/// Every boolean-or-options predicate goes through here, so a provider
/// present as `false` is never mistaken for one that is enabled.
fn enabled(provider: &Option<impl Provider>) -> bool {
    provider.as_ref().is_some_and(Provider::enabled)
}

impl<T> Provider for OneOf<bool, T> {
    fn enabled(&self) -> bool {
        matches!(self, OneOf::Left(true) | OneOf::Right(_))
    }
}

impl Provider for HoverProviderCapability {
    fn enabled(&self) -> bool {
        matches!(self, Self::Simple(true) | Self::Options(_))
    }
}

impl Provider for CodeActionProviderCapability {
    fn enabled(&self) -> bool {
        matches!(self, Self::Simple(true) | Self::Options(_))
    }
}

impl Provider for ImplementationProviderCapability {
    fn enabled(&self) -> bool {
        matches!(self, Self::Simple(true) | Self::Options(_))
    }
}

impl Provider for TypeDefinitionProviderCapability {
    fn enabled(&self) -> bool {
        matches!(self, Self::Simple(true) | Self::Options(_))
    }
}

impl Provider for DeclarationCapability {
    fn enabled(&self) -> bool {
        matches!(
            self,
            Self::Simple(true) | Self::RegistrationOptions(_) | Self::Options(_)
        )
    }
}
