//! Symbol replies: `textDocument/documentSymbol` and `workspace/symbol`,
//! both reply shapes, flattened into [`ProtoSymbol`] rows (0047 §1, 0063
//! §2). A row carries the lines its declaration spans (0107 §6.1).
use crate::protocol::*;
use strop_core::id::LineIndex;
use strop_workspace::ResourceLocation;

use super::api::note;
use super::queue::WireEnv;
use super::wire;
use async_lsp::lsp_types as lt;
use strop_core::symbol_kind::SymbolKind;

/// The lines a range covers. An end at column 0 of a later line does
/// not cover that line: the range stops at the line break before it.
pub(super) fn line_span(range: lt::Range) -> LineSpan {
    let first = range.start.line;
    let last = if range.end.character == 0 && range.end.line > first {
        range.end.line - 1
    } else {
        range.end.line.max(first)
    };
    LineSpan {
        first: LineIndex::new(first as usize),
        last: LineIndex::new(last as usize),
    }
}

/// A flat `SymbolInformation` row, decoded through the workspace.
fn information_row(info: lt::SymbolInformation, env: &WireEnv) -> Option<ProtoSymbol> {
    let span = line_span(info.location.range);
    let location = wire::to_server_location(info.location, &env.workspace).ok()?;
    Some(ProtoSymbol {
        name: info.name,
        container: info.container_name.unwrap_or_default(),
        kind: symbol_kind(info.kind),
        location,
        span,
    })
}

/// One workspace-symbol query (0063 §2): both reply shapes land in the
/// same row form as document symbols. A `WorkspaceSymbol` whose
/// location is uri-only (a resolve-support server contract we do not
/// advertise) has no position to jump to and is dropped.
pub(crate) async fn workspace_symbols(env: WireEnv, generation: u64, query: String) {
    let params = lt::WorkspaceSymbolParams {
        partial_result_params: Default::default(),
        work_done_progress_params: Default::default(),
        query,
    };
    let response = env
        .socket
        .request::<lt::request::WorkspaceSymbolRequest>(params)
        .await;
    let server = env.id;
    match response {
        Ok(Some(lt::WorkspaceSymbolResponse::Flat(informations))) => {
            let symbols = informations
                .into_iter()
                .filter_map(|info| information_row(info, &env))
                .collect::<Vec<_>>();
            let _ = env.tx.send(LspEvent::WorkspaceSymbols {
                server,
                generation,
                symbols,
            });
        }
        Ok(Some(lt::WorkspaceSymbolResponse::Nested(entries))) => {
            let symbols = entries
                .into_iter()
                .filter_map(|entry| {
                    let location = match entry.location {
                        lt::OneOf::Left(location) => location,
                        lt::OneOf::Right(_) => return None,
                    };
                    let span = line_span(location.range);
                    let location = wire::to_server_location(location, &env.workspace).ok()?;
                    Some(ProtoSymbol {
                        name: entry.name,
                        container: entry.container_name.unwrap_or_default(),
                        kind: symbol_kind(entry.kind),
                        location,
                        span,
                    })
                })
                .collect::<Vec<_>>();
            let _ = env.tx.send(LspEvent::WorkspaceSymbols {
                server,
                generation,
                symbols,
            });
        }
        Ok(None) => {
            let _ = env.tx.send(LspEvent::WorkspaceSymbols {
                server,
                generation,
                symbols: Vec::new(),
            });
        }
        Err(error) => {
            let _ = env.tx.send(LspEvent::WorkspaceSymbolsFailed {
                server,
                generation,
                reason: format!("workspace symbols failed: {error}"),
            });
        }
    }
}

/// `textDocument/documentSymbol`: both reply shapes flatten into
/// [`ProtoSymbol`] rows (0047 §1). Hierarchical trees join ancestors
/// with ` :: ` as the container path; the jump position is the
/// selection range's start (the identifier, not the block), the span is
/// the whole range's lines.
pub(super) async fn document_symbols(
    env: WireEnv,
    text_document: lt::TextDocumentIdentifier,
    context: ReplyContext,
    path: std::path::PathBuf,
) {
    let params = lt::DocumentSymbolParams {
        text_document,
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let response = env
        .socket
        .request::<lt::request::DocumentSymbolRequest>(params)
        .await;
    match response {
        Ok(Some(lt::DocumentSymbolResponse::Flat(informations))) => {
            let symbols = informations
                .into_iter()
                .filter_map(|info| information_row(info, &env))
                .collect::<Vec<_>>();
            let _ = env.tx.send(LspEvent::Symbols { context, symbols });
        }
        Ok(Some(lt::DocumentSymbolResponse::Nested(tree))) => {
            let doc = ResourceLocation {
                filesystem: env.workspace.target(),
                path,
            };
            let mut symbols = Vec::new();
            flatten_symbols(&tree, "", &doc, &mut symbols);
            let _ = env.tx.send(LspEvent::Symbols { context, symbols });
        }
        Ok(None) => note(&env, context, "no symbols in this document".into()),
        Err(error) => note(&env, context, format!("document symbols failed: {error}")),
    }
}

fn flatten_symbols(
    symbols: &[lt::DocumentSymbol],
    ancestors: &str,
    doc: &ResourceLocation,
    out: &mut Vec<ProtoSymbol>,
) {
    for symbol in symbols {
        let start = symbol.selection_range.start;
        out.push(ProtoSymbol {
            name: symbol.name.clone(),
            container: ancestors.to_string(),
            kind: symbol_kind(symbol.kind),
            location: ServerLocation {
                doc: doc.clone(),
                position: ServerPosition {
                    line: LineIndex::new(start.line as usize),
                    column: ServerColumn::new(start.character as usize),
                },
            },
            span: line_span(symbol.range),
        });
        if let Some(children) = symbol.children.as_deref() {
            let path = if ancestors.is_empty() {
                symbol.name.clone()
            } else {
                format!("{ancestors} :: {}", symbol.name)
            };
            flatten_symbols(children, &path, doc, out);
        }
    }
}

/// The protocol's kind as strop's one kind table names it (0109): all
/// 26 protocol kinds have a row; a kind a later protocol adds is Other.
fn symbol_kind(kind: lt::SymbolKind) -> SymbolKind {
    match kind {
        lt::SymbolKind::FILE => SymbolKind::File,
        lt::SymbolKind::MODULE | lt::SymbolKind::NAMESPACE | lt::SymbolKind::PACKAGE => {
            SymbolKind::Module
        }
        lt::SymbolKind::CLASS => SymbolKind::Class,
        lt::SymbolKind::METHOD => SymbolKind::Method,
        lt::SymbolKind::PROPERTY => SymbolKind::Property,
        lt::SymbolKind::FIELD => SymbolKind::Field,
        lt::SymbolKind::CONSTRUCTOR => SymbolKind::Constructor,
        lt::SymbolKind::ENUM => SymbolKind::Enum,
        lt::SymbolKind::INTERFACE => SymbolKind::Interface,
        lt::SymbolKind::FUNCTION => SymbolKind::Function,
        lt::SymbolKind::VARIABLE => SymbolKind::Variable,
        lt::SymbolKind::CONSTANT => SymbolKind::Constant,
        lt::SymbolKind::STRING
        | lt::SymbolKind::NUMBER
        | lt::SymbolKind::BOOLEAN
        | lt::SymbolKind::ARRAY
        | lt::SymbolKind::NULL => SymbolKind::Value,
        // rust-analyzer reports an impl block as an Object
        lt::SymbolKind::OBJECT => SymbolKind::Impl,
        lt::SymbolKind::KEY => SymbolKind::Key,
        lt::SymbolKind::ENUM_MEMBER => SymbolKind::Variant,
        lt::SymbolKind::STRUCT => SymbolKind::Struct,
        lt::SymbolKind::EVENT => SymbolKind::Event,
        lt::SymbolKind::OPERATOR => SymbolKind::Operator,
        lt::SymbolKind::TYPE_PARAMETER => SymbolKind::Type,
        _ => SymbolKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every protocol kind lands on the kind whose row lists its name,
    /// so `kind:` judges every server row (0109 cause B).
    #[test]
    fn every_protocol_kind_has_its_own_row() {
        let names = [
            (lt::SymbolKind::FILE, "File"),
            (lt::SymbolKind::MODULE, "Module"),
            (lt::SymbolKind::NAMESPACE, "Namespace"),
            (lt::SymbolKind::PACKAGE, "Package"),
            (lt::SymbolKind::CLASS, "Class"),
            (lt::SymbolKind::METHOD, "Method"),
            (lt::SymbolKind::PROPERTY, "Property"),
            (lt::SymbolKind::FIELD, "Field"),
            (lt::SymbolKind::CONSTRUCTOR, "Constructor"),
            (lt::SymbolKind::ENUM, "Enum"),
            (lt::SymbolKind::INTERFACE, "Interface"),
            (lt::SymbolKind::FUNCTION, "Function"),
            (lt::SymbolKind::VARIABLE, "Variable"),
            (lt::SymbolKind::CONSTANT, "Constant"),
            (lt::SymbolKind::STRING, "String"),
            (lt::SymbolKind::NUMBER, "Number"),
            (lt::SymbolKind::BOOLEAN, "Boolean"),
            (lt::SymbolKind::ARRAY, "Array"),
            (lt::SymbolKind::OBJECT, "Object"),
            (lt::SymbolKind::KEY, "Key"),
            (lt::SymbolKind::NULL, "Null"),
            (lt::SymbolKind::ENUM_MEMBER, "EnumMember"),
            (lt::SymbolKind::STRUCT, "Struct"),
            (lt::SymbolKind::EVENT, "Event"),
            (lt::SymbolKind::OPERATOR, "Operator"),
            (lt::SymbolKind::TYPE_PARAMETER, "TypeParameter"),
        ];
        for (kind, name) in names {
            assert!(symbol_kind(kind).spec().lsp.contains(&name), "{name}");
        }
    }

    fn range(start: (u32, u32), end: (u32, u32)) -> lt::Range {
        lt::Range {
            start: lt::Position::new(start.0, start.1),
            end: lt::Position::new(end.0, end.1),
        }
    }

    fn lines(span: LineSpan) -> (usize, usize) {
        (span.first.get(), span.last.get())
    }

    #[test]
    fn an_end_at_column_zero_stops_one_line_earlier() {
        assert_eq!(lines(line_span(range((2, 0), (5, 1)))), (2, 5));
        assert_eq!(lines(line_span(range((2, 0), (6, 0)))), (2, 5));
        // a one-line range ending at column 0 is still its own line
        assert_eq!(lines(line_span(range((4, 0), (4, 0)))), (4, 4));
        // an end before its start (a sloppy server) is one line
        assert_eq!(lines(line_span(range((4, 3), (1, 0)))), (4, 4));
    }
}
