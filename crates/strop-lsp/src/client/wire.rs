//! Request plumbing shared by the launcher: response shapes and the
//! locations family. Every path ends in a terminal event.
use async_lsp::lsp_types::{self as lt, TextDocumentPositionParams};
use std::path::PathBuf;
use strop_core::id::LineIndex;

use super::queue::WireEnv;
use crate::protocol::{
    LocKind, LspEvent, ReplyContext, ServerColumn, ServerEdit, ServerLocation, ServerPosition,
    WireVersion,
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

/// A wire text edit → a server-domain replacement span.
pub(super) fn server_edit(edit: lt::TextEdit) -> ServerEdit {
    let position = |p: lt::Position| ServerPosition {
        line: LineIndex::new(p.line as usize),
        column: ServerColumn::new(p.character as usize),
    };
    ServerEdit {
        start: position(edit.range.start),
        end: position(edit.range.end),
        new_text: edit.new_text,
    }
}

/// Why a server workspace edit can never be delivered as an applicable
/// edit set — rename refuses the whole reply, a code action degrades to
/// an inapplicable entry. Never a partial edit set.
pub(super) enum WorkspaceEditRefusal {
    /// The edit creates/renames/deletes files: resource operations are
    /// unsupported, and applying only the text edits would corrupt the
    /// change.
    ResourceOperations,
    /// A versioned text-document edit named a version this connection
    /// never sent (stale, or a document we do not hold).
    VersionMismatch(String),
    /// A target URI does not decode onto the server's workspace.
    Undecodable(String),
}

impl WorkspaceEditRefusal {
    /// The terminal-note text for a refused whole reply.
    pub(super) fn note_text(&self, kind_label: &str, workspace_label: &str) -> String {
        match self {
            Self::ResourceOperations => format!(
                "{kind_label} edit carries file operations (create/rename/delete) — refusing the whole edit"
            ),
            Self::VersionMismatch(uri) => format!(
                "{kind_label} targets a version of {uri} this connection never sent — refusing the whole edit"
            ),
            Self::Undecodable(uri) => {
                format!("{kind_label} target is not a file on {workspace_label}: {uri} — refusing the whole edit")
            }
        }
    }
}

/// Decode a server `WorkspaceEdit` — both the `changes` map and
/// versioned `documentChanges` — into per-resource server-domain edit
/// groups on the server's own filesystem. URI decoding is the same
/// workspace decode as locations; a `changes` map is sorted by path so
/// delivery order never depends on hash seed (R11).
pub(super) fn workspace_edits(
    env: &WireEnv,
    edit: lt::WorkspaceEdit,
) -> Result<Vec<(strop_workspace::ResourceLocation, Vec<ServerEdit>)>, WorkspaceEditRefusal> {
    let location = |uri: &lt::Url| {
        env.workspace
            .decode(uri)
            .map(|path| strop_workspace::ResourceLocation {
                filesystem: env.workspace.target(),
                path,
            })
    };
    match edit.document_changes {
        Some(lt::DocumentChanges::Operations(_)) => Err(WorkspaceEditRefusal::ResourceOperations),
        Some(lt::DocumentChanges::Edits(documents)) => {
            let mut out = Vec::with_capacity(documents.len());
            for document in documents {
                let uri = document.text_document.uri;
                let Some(doc) = location(&uri) else {
                    return Err(WorkspaceEditRefusal::Undecodable(uri.to_string()));
                };
                if let Some(version) = document.text_document.version {
                    if super::sync::sent_version(env, &doc.path) != Some(WireVersion::new(version))
                    {
                        return Err(WorkspaceEditRefusal::VersionMismatch(uri.to_string()));
                    }
                }
                let edits = document
                    .edits
                    .into_iter()
                    .map(|e| match e {
                        lt::OneOf::Left(edit) => server_edit(edit),
                        lt::OneOf::Right(annotated) => server_edit(annotated.text_edit),
                    })
                    .collect();
                out.push((doc, edits));
            }
            Ok(out)
        }
        None => {
            let Some(changes) = edit.changes else {
                return Ok(Vec::new());
            };
            let mut out = Vec::with_capacity(changes.len());
            for (uri, edits) in changes {
                let Some(doc) = location(&uri) else {
                    return Err(WorkspaceEditRefusal::Undecodable(uri.to_string()));
                };
                out.push((doc, edits.into_iter().map(server_edit).collect()));
            }
            out.sort_by(|a, b| a.0.path.cmp(&b.0.path));
            Ok(out)
        }
    }
}

/// A wire location → typed server location on the server's own
/// filesystem. `Err` keeps the URI for the terminal note when it does
/// not name a file there. Remote URIs decode onto the remote host's
/// paths — the analogous local path is never considered.
pub(super) fn to_server_location(
    l: lt::Location,
    workspace: &crate::target::Workspace,
) -> Result<ServerLocation, String> {
    match workspace.decode(&l.uri) {
        Some(path) => Ok(ServerLocation {
            doc: strop_workspace::ResourceLocation {
                filesystem: workspace.target(),
                path,
            },
            position: ServerPosition {
                line: LineIndex::new(l.range.start.line as usize),
                column: ServerColumn::new(l.range.start.character as usize),
            },
        }),
        None => Err(l.uri.to_string()),
    }
}

fn locations(
    items: Vec<lt::Location>,
    workspace: &crate::target::Workspace,
) -> (Vec<ServerLocation>, usize) {
    let mut dropped = 0;
    let items = items
        .into_iter()
        .filter_map(|l| match to_server_location(l, workspace) {
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
            let (items, dropped) = locations(locs, &env.workspace);
            if items.is_empty() && dropped > 0 {
                let _ = env.tx.send(LspEvent::Note {
                    context,
                    text: format!("no file on {} {}", env.workspace.label(), kind.label()),
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
