//! Request plumbing shared by the launcher: response shapes and the
//! locations family. Every path ends in a terminal event.
use async_lsp::lsp_types::{self as lt, TextDocumentPositionParams};
use std::path::PathBuf;
use strop_core::id::LineIndex;

use super::queue::WireEnv;
use crate::protocol::{
    LocKind, LspEvent, ReplyContext, ServerColumn, ServerLocation, ServerPosition,
};

pub(super) fn is_content_modified<T>(resp: &Result<T, async_lsp::Error>) -> bool {
    matches!(resp, Err(async_lsp::Error::Response(e))
        if e.code == async_lsp::ErrorCode::from(-32801))
}

/// First location of a goto-shaped response (scalar, array or link).
pub(super) fn first_location(response: lt::GotoDefinitionResponse) -> Option<lt::Location> {
    use lt::GotoDefinitionResponse as R;
    match response {
        R::Scalar(l) => Some(l),
        R::Array(v) => v.into_iter().next(),
        R::Link(v) => v.into_iter().next().map(|l| lt::Location {
            uri: l.target_uri,
            range: l.target_selection_range,
        }),
    }
}

/// A wire location → typed server location. `Err` keeps the URI for the
/// terminal note when it is not a local file.
pub(super) fn to_server_location(l: lt::Location) -> Result<ServerLocation, String> {
    match l.uri.to_file_path() {
        Ok(path) => Ok(ServerLocation {
            path,
            position: ServerPosition {
                line: LineIndex::new(l.range.start.line as usize),
                column: ServerColumn::new(l.range.start.character as usize),
            },
        }),
        Err(()) => Err(l.uri.to_string()),
    }
}

fn locations(items: Vec<lt::Location>) -> (Vec<ServerLocation>, usize) {
    let mut dropped = 0;
    let items = items
        .into_iter()
        .filter_map(|l| match to_server_location(l) {
            Ok(location) => Some(location),
            Err(_) => {
                dropped += 1;
                None
            }
        })
        .collect();
    (items, dropped)
}

/// The locations family: references/implementation/type definition/
/// declaration. Ok(None) and an all-dropped list are explicit empties;
/// an error is an explicit failure note.
pub(super) async fn request_locations(
    env: &WireEnv,
    kind: LocKind,
    tdp: TextDocumentPositionParams,
    context: ReplyContext,
    path: PathBuf,
) {
    use async_lsp::lsp_types::request as req;
    let socket = env.socket.clone();
    let _ = path;
    macro_rules! goto_shaped {
        ($request:ident, $params:expr) => {
            match socket.request::<req::$request>($params).await {
                Ok(Some(response)) => {
                    let locs = match response {
                        lt::GotoDefinitionResponse::Scalar(l) => vec![l],
                        lt::GotoDefinitionResponse::Array(v) => v,
                        lt::GotoDefinitionResponse::Link(v) => v
                            .into_iter()
                            .map(|l| lt::Location {
                                uri: l.target_uri,
                                range: l.target_selection_range,
                            })
                            .collect(),
                    };
                    Some(locs)
                }
                Ok(None) => Some(Vec::new()),
                Err(error) => {
                    let _ = env.tx.send(LspEvent::Note {
                        context,
                        text: format!("{} failed: {error}", kind.label()),
                    });
                    return;
                }
            }
        };
    }
    let items: Option<Vec<lt::Location>> = match kind {
        LocKind::References => {
            let params = lt::ReferenceParams {
                text_document_position: tdp,
                context: lt::ReferenceContext {
                    include_declaration: true,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            match socket.request::<req::References>(params).await {
                Ok(items) => items,
                Err(error) => {
                    let _ = env.tx.send(LspEvent::Note {
                        context,
                        text: format!("{} failed: {error}", kind.label()),
                    });
                    return;
                }
            }
        }
        LocKind::Implementation => {
            let params = lt::request::GotoImplementationParams {
                text_document_position_params: tdp,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            goto_shaped!(GotoImplementation, params)
        }
        LocKind::TypeDefinition => {
            let params = lt::request::GotoTypeDefinitionParams {
                text_document_position_params: tdp,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            goto_shaped!(GotoTypeDefinition, params)
        }
        LocKind::Declaration => {
            let params = lt::request::GotoDeclarationParams {
                text_document_position_params: tdp,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            goto_shaped!(GotoDeclaration, params)
        }
    };
    match items {
        Some(locs) => {
            let (items, dropped) = locations(locs);
            if items.is_empty() && dropped > 0 {
                let _ = env.tx.send(LspEvent::Note {
                    context,
                    text: format!("no local file {}", kind.label()),
                });
            } else {
                let _ = env.tx.send(LspEvent::Locations {
                    context,
                    kind,
                    items,
                });
            }
        }
        // Ok(None) for references: an explicit empty list.
        None => {
            let _ = env.tx.send(LspEvent::Locations {
                context,
                kind,
                items: Vec::new(),
            });
        }
    }
}
