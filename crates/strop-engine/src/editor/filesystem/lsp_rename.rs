//! Language-server file-rename choreography around the reviewed
//! filesystem apply (0054 §12): `workspace/willRenameFiles` fires after
//! the user confirms the review and before the mutation, the server's
//! preparation edits apply straight to open buffers through the
//! revision-checked plan gateway, the mutation then runs, and
//! `workspace/didRenameFiles` reports the receipts that committed.
use super::Proposal;
use crate::editor::changes::ChangeProducer;
use crate::editor::Editor;
use std::path::PathBuf;
use strop_core::id::{ByteColumn, DocumentId, LineIndex};
use strop_lsp::protocol::ServerEdit;
use strop_lsp::{ReplyContext, RequestInput, RequestKind, RequestStamp, ServerId};
use strop_workspace::operation::OperationKind;
use strop_workspace::ResourceLocation;

/// Per-server rename pairs staged for the post-commit notification.
pub(crate) type RenameServers = Vec<(ServerId, Vec<(PathBuf, PathBuf)>)>;

impl Editor {
    /// Fire `willRenameFiles` on every ready server that advertises it.
    /// `Some((stamps, servers))` means at least one server admitted the
    /// request and the caller stages the proposal; `None` (no rename
    /// steps, no interested server, every admission refused) leaves the
    /// ordinary apply path untouched. Borrows the proposal so the
    /// common no-server case never moves it.
    pub(crate) fn admit_will_rename(
        &self,
        proposal: &Proposal,
    ) -> Option<(Vec<RequestStamp>, RenameServers)> {
        let pairs: Vec<(strop_workspace::Filesystem, PathBuf, PathBuf)> = proposal
            .batch
            .steps
            .iter()
            .filter(|step| step.intent.kind == OperationKind::Rename)
            .filter_map(
                |step| match (&step.intent.source, &step.intent.destination) {
                    (Some(source), Some(destination)) => Some((
                        source.filesystem.clone(),
                        source.path.clone(),
                        destination.path.clone(),
                    )),
                    _ => None,
                },
            )
            .collect();
        if pairs.is_empty() {
            return None;
        }
        let mut stamps = Vec::new();
        let mut servers: RenameServers = Vec::new();
        for server in &self.lsp_servers {
            if !server.ready {
                continue;
            }
            let Some(client) = server.client.clone() else {
                continue;
            };
            if !client.caps().will_rename_files() {
                continue;
            }
            let workspace = client.workspace();
            let server_pairs: Vec<(PathBuf, PathBuf)> = pairs
                .iter()
                .filter(|(filesystem, old, _)| {
                    *filesystem == workspace.target() && old.starts_with(workspace.root())
                })
                .map(|(_, old, new)| (old.clone(), new.clone()))
                .collect();
            if server_pairs.is_empty() {
                continue;
            }
            // The request rides an anchor document's admission record;
            // the position itself is protocol-dead for this kind.
            let anchor = self
                .lsp_state
                .bindings
                .iter()
                .find(|(_, binding)| binding.server == server.id)
                .map(|(&document, _)| document);
            let Some(input) = anchor.and_then(|document| self.will_rename_input(document)) else {
                continue;
            };
            if let Ok(stamp) = client.will_rename_files(input, server_pairs.clone()) {
                stamps.push(stamp);
                servers.push((server.id, server_pairs));
            }
        }
        if stamps.is_empty() {
            return None;
        }
        Some((stamps, servers))
    }

    /// A non-position request still needs a valid admission anchor: any
    /// open document bound to the server supplies ownership identity.
    fn will_rename_input(&self, document: DocumentId) -> Option<RequestInput> {
        let location = self.lsp_document_location(document)?;
        let buf = &self.docs.get(document)?.buf;
        Some(RequestInput {
            document,
            revision: buf.revision(),
            path: location.path.clone(),
            line: LineIndex::new(0),
            byte_col: ByteColumn::new(0),
            line_text: strop_lsp::FrozenLine::from_slice(
                buf.text().byte_slice(buf.line_start(0)..buf.line_end(0)),
            ),
            kind: RequestKind::WillRenameFiles,
            rename_to: None,
            rename_files: None,
        })
    }

    /// One server's will-rename reply: apply its preparation edits
    /// through the revision-checked gateway (a stale anchor means this
    /// server contributes nothing), then resume the staged filesystem
    /// apply once every admitted server has answered.
    pub(crate) fn will_rename_edits(
        &mut self,
        context: ReplyContext,
        edits: Vec<(ResourceLocation, Vec<ServerEdit>)>,
    ) {
        let admitted = self
            .filesystem
            .will_rename
            .as_ref()
            .is_some_and(|stage| stage.stamps.contains(&context.stamp));
        if !admitted {
            return; // cancelled or superseded stage: the reply is dead weight
        }
        // The stage's stamp set proves admission; freshness is the
        // anchor binding's server/revision still holding. Focus and the
        // navigation slot do not apply — focus sits on the review
        // buffer and several servers can be in flight at once. Edits
        // targeting other documents are revision-checked individually
        // by the plan gateway.
        let anchor_fresh = !self.docs.is_empty()
            && self
                .lsp_state
                .bindings
                .get(&context.stamp.document)
                .is_some_and(|binding| {
                    binding.server == context.stamp.server
                        && binding.revision == context.stamp.revision
                });
        if !edits.is_empty() && anchor_fresh {
            let plan = self.build_change_plan(ChangeProducer::FileRename, edits, context.encoding);
            self.apply_change_plan(plan);
        }
        let complete = {
            let Some(stage) = self.filesystem.will_rename.as_mut() else {
                return;
            };
            stage.stamps.retain(|stamp| *stamp != context.stamp);
            stage.stamps.is_empty()
        };
        if complete {
            if let Some(stage) = self.filesystem.will_rename.take() {
                self.dispatch_filesystem_apply(stage.proposal, stage.servers);
            }
        }
    }

    /// `workspace/didRenameFiles` for the rename receipts that committed,
    /// limited to the pairs each server was asked to prepare.
    pub(crate) fn notify_did_rename(&mut self, index: usize) {
        let attempt = &self.filesystem.history[index];
        if attempt.lsp_renames.is_empty() {
            return;
        }
        let committed: Vec<(PathBuf, PathBuf)> = attempt
            .receipts
            .iter()
            .filter(|receipt| {
                receipt.outcome.is_committed()
                    && receipt.operation.intent.kind == OperationKind::Rename
            })
            .filter_map(|receipt| {
                match (
                    &receipt.operation.intent.source,
                    &receipt.operation.intent.destination,
                ) {
                    (Some(source), Some(destination)) => {
                        Some((source.path.clone(), destination.path.clone()))
                    }
                    _ => None,
                }
            })
            .collect();
        if committed.is_empty() {
            return;
        }
        for (server, staged) in attempt.lsp_renames.clone() {
            let done: Vec<(PathBuf, PathBuf)> = committed
                .iter()
                .filter(|(old, _)| staged.iter().any(|(staged_old, _)| staged_old == old))
                .cloned()
                .collect();
            if done.is_empty() {
                continue;
            }
            if let Some(client) = self.lsp_live_client(server) {
                client.did_rename_files(done);
            }
        }
    }
}
