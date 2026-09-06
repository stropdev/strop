//! client/wire.rs — request plumbing shared by the api surface.

use std::path::PathBuf;

use async_lsp::lsp_types::TextDocumentPositionParams;
use async_lsp::ServerSocket;

use crate::protocol::{LocKind, LspEvent};

pub(super) fn is_content_modified<T>(resp: &Result<T, async_lsp::Error>) -> bool {
    matches!(
        resp,
        Err(async_lsp::Error::Response(e))
            if e.code == async_lsp::ErrorCode::from(-32801)
    )
}

/// Hover content → plain text (markdown flattened).
/// The location-request body shared by `Client::locations` and the
/// pre-init flush.
pub(super) async fn request_locations(
    sock: ServerSocket,
    tx: std::sync::mpsc::Sender<LspEvent>,
    kind: LocKind,
    tdp: TextDocumentPositionParams,
    req_revision: u64,
) {
    use async_lsp::lsp_types as lt;
    let to_items = |locs: Vec<lt::Location>| -> Vec<(PathBuf, usize, usize)> {
        locs.into_iter()
            .filter_map(|l| {
                l.uri.to_file_path().ok().map(|p| {
                    (
                        p,
                        l.range.start.line as usize,
                        l.range.start.character as usize,
                    )
                })
            })
            .collect()
    };
    let from_goto =
        |r: Option<lt::GotoDefinitionResponse>| -> Option<Vec<(PathBuf, usize, usize)>> {
            r.map(|resp| {
                let locs = match resp {
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
                to_items(locs)
            })
        };
    let items = match kind {
        LocKind::References => {
            let params = lt::ReferenceParams {
                text_document_position: tdp,
                context: lt::ReferenceContext {
                    include_declaration: true,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            sock.request::<lt::request::References>(params)
                .await
                .ok()
                .flatten()
                .map(to_items)
        }
        LocKind::Implementation => {
            let params = lt::request::GotoImplementationParams {
                text_document_position_params: tdp,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            from_goto(
                sock.request::<lt::request::GotoImplementation>(params)
                    .await
                    .ok()
                    .flatten(),
            )
        }
        LocKind::TypeDefinition => {
            let params = lt::request::GotoTypeDefinitionParams {
                text_document_position_params: tdp,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            from_goto(
                sock.request::<lt::request::GotoTypeDefinition>(params)
                    .await
                    .ok()
                    .flatten(),
            )
        }
        LocKind::Declaration => {
            let params = lt::request::GotoDeclarationParams {
                text_document_position_params: tdp,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            from_goto(
                sock.request::<lt::request::GotoDeclaration>(params)
                    .await
                    .ok()
                    .flatten(),
            )
        }
    };
    if let Some(items) = items {
        let _ = tx.send(LspEvent::Locations {
            kind,
            items,
            req_revision,
        });
    }
}
