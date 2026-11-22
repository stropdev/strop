use super::ServerCaps;
use crate::protocol::{LocKind, RequestKind};
use async_lsp::lsp_types::{
    DefinitionOptions, HoverProviderCapability, InitializeResult, OneOf, ServerCapabilities,
};

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

/// Every provider the protocol lets a server answer with a boolean or an
/// options object, decoded from the initialize result as it arrives on the
/// wire: absent and `false` refuse the request, `true` and `{}` admit it.
#[test]
fn a_boolean_or_options_provider_is_enabled_by_its_value_not_its_presence() {
    let providers = [
        ("hoverProvider", RequestKind::Hover),
        ("definitionProvider", RequestKind::Goto),
        (
            "referencesProvider",
            RequestKind::Locations(LocKind::References),
        ),
        (
            "implementationProvider",
            RequestKind::Locations(LocKind::Implementation),
        ),
        (
            "typeDefinitionProvider",
            RequestKind::Locations(LocKind::TypeDefinition),
        ),
        (
            "declarationProvider",
            RequestKind::Locations(LocKind::Declaration),
        ),
        ("documentFormattingProvider", RequestKind::Format),
        ("renameProvider", RequestKind::Rename),
        ("codeActionProvider", RequestKind::CodeAction),
        ("documentSymbolProvider", RequestKind::DocumentSymbols),
        ("workspaceSymbolProvider", RequestKind::WorkspaceSymbols),
    ];
    let forms = [
        ("absent", None, false),
        ("false", Some(serde_json::json!(false)), false),
        ("true", Some(serde_json::json!(true)), true),
        ("{}", Some(serde_json::json!({})), true),
    ];
    for (key, kind) in providers {
        for (form, value, expected) in &forms {
            let mut capabilities = serde_json::Map::new();
            if let Some(value) = value {
                capabilities.insert(key.to_owned(), value.clone());
            }
            let wire = serde_json::json!({ "capabilities": capabilities });
            let result: InitializeResult = serde_json::from_value(wire).unwrap();
            let caps = ServerCaps::default();
            caps.set(result.capabilities);
            assert_eq!(caps.supports(kind), *expected, "{key}: {form}");
        }
    }
}
