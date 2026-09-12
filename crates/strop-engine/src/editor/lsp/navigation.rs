//! Namespace-preserving LSP jumps and source-context inheritance.
use super::{lsp_language, state};
use crate::editor::{trace, Editor};
use std::path::{Path, PathBuf};
use strop_workspace::{Filesystem, ResourceLocation};

impl Editor {
    pub(crate) fn jump_to_location(
        &mut self,
        location: strop_lsp::ServerLocation,
        context: strop_lsp::ReplyContext,
    ) {
        if !self.lsp_context_fresh(&context) {
            return;
        }
        let intent = crate::editor::io::OpenIntent::LspLocation {
            context,
            position: location.position,
        };
        match location.doc.filesystem {
            Filesystem::Local => self.request_open(location.doc.path, intent),
            Filesystem::Remote(endpoint) => {
                // The target is a file on the replying server's host:
                // resolve it through an open document's canonical seed
                // (`with_path` keeps endpoint + native bytes) — the
                // analogous local path is never opened or probed.
                match self.remote_file_for(&endpoint) {
                    Some(seed) => match seed.with_path(location.doc.path.clone()) {
                        Ok(file) => self
                            .request_target(crate::files::FileTarget::Remote(file.into()), intent),
                        Err(error) => {
                            trace::services::rejected("lsp", "remote navigation target invalid");
                            self.message = format!("lsp: remote target invalid: {error}");
                        }
                    },
                    None => {
                        trace::services::rejected("lsp", "remote navigation endpoint lost");
                        self.message =
                            "lsp: the remote workspace for this target was closed".into();
                    }
                }
            }
            Filesystem::Container(_) => {
                trace::services::rejected("lsp", "navigation into a container namespace (unwired)");
                self.message = "lsp: container locations are not navigable yet".into();
            }
        }
    }

    pub(crate) fn finish_lsp_jump(
        &mut self,
        target: strop_core::id::DocumentId,
        position: strop_lsp::ServerPosition,
        context: strop_lsp::ReplyContext,
    ) {
        if !self.lsp_context_fresh(&context) {
            trace::services::rejected("lsp", "navigation changed while target was loading");
            return;
        }
        let Some(target_doc) = self.docs.get(target) else {
            return;
        };
        let Some(binding) = self.lsp_state.bindings.get(&context.stamp.document) else {
            return;
        };
        let outside = match &target_doc.source {
            // A remote target is outside the workspace when its remote
            // path leaves the binding's remote root — never by
            // comparing against local paths.
            crate::editor::document::DocumentSource::Remote(file) => {
                !file.file.path().starts_with(&binding.root)
            }
            _ => target_doc
                .buf
                .path
                .as_ref()
                .is_some_and(|path| !self.cwd.join(path).starts_with(&binding.root)),
        };
        let line = position
            .line
            .get()
            .min(target_doc.buf.len_lines().saturating_sub(1));
        let text = target_doc
            .buf
            .text()
            .byte_slice(target_doc.buf.line_start(line)..target_doc.buf.line_end(line));
        let col = strop_lsp::to_byte_col_slice(text, position.column, context.encoding).get();
        let head = target_doc
            .buf
            .clamp_boundary(target_doc.buf.line_start(line).saturating_add(col));
        self.push_jump();
        self.lsp_state.navigation = None;
        self.switch_to(target);
        if outside && !self.buf().readonly {
            self.buf_mut().readonly = true;
            self.message = "readonly — outside workspace (:set noro to edit)".into();
        }
        self.set_head(head);
        self.clamp_cursor();
        // definition/reference landings use the 0051 §7 placement:
        // center the target unless it is already comfortably visible
        self.place_jump_target();
        // A server-originated jump carries its language-service context
        // (0049 §4.1): the replying server keeps answering inside the
        // target. A live binding another navigation established is never
        // switched (0049 §4.5), and namespaces never cross (0049 §4.7).
        let origin = self
            .lsp_state
            .bindings
            .get(&context.stamp.document)
            .map(|b| {
                (
                    b.server,
                    b.root.clone(),
                    b.target.clone(),
                    b.language.clone(),
                )
            });
        if let Some((server, root, origin_target, language)) = origin {
            let target_bound = self.lsp_state.bindings.contains_key(&target);
            let doc = self.lsp_doc_path(target);
            // C and C++ headers are interchangeable for the server that
            // serves both (0049 §4.4: an ambiguous `.h` inherits).
            let language_compatible = doc
                .as_ref()
                .and_then(|doc| lsp_language(&doc.path))
                .is_none_or(|known| {
                    known == language
                        || (matches!(known, "c" | "cpp")
                            && matches!(language.as_str(), "c" | "cpp"))
                });
            let context_free = !self.lsp_state.jump_contexts.contains_key(&target);
            if let (false, true, Some(doc), true) =
                (target_bound, context_free, doc, language_compatible)
            {
                if doc.filesystem == origin_target {
                    // A routing hint, not open state: didOpen follows in
                    // lsp_maybe_attach and becomes the real binding.
                    self.lsp_state.jump_contexts.insert(
                        target,
                        state::JumpContext {
                            server,
                            root,
                            language,
                            target: origin_target,
                        },
                    );
                }
            }
        }
        self.scroll_to_cursor(self.view_rows());
        self.lsp_maybe_attach();
    }

    /// A local picker hit with a live LSP request context: the server
    /// that produced the list owns the target's filesystem.
    pub(crate) fn lsp_jump_from_picker(
        &mut self,
        path: PathBuf,
        line: usize,
        col: usize,
        context: strop_lsp::ReplyContext,
    ) {
        self.jump_to_location(
            strop_lsp::ServerLocation {
                doc: ResourceLocation::local(path),
                position: strop_lsp::ServerPosition {
                    line: strop_core::id::LineIndex::new(line.saturating_sub(1)),
                    column: strop_lsp::ServerColumn::new(col.saturating_sub(1)),
                },
            },
            context,
        );
    }

    /// A remote picker hit (locations or diagnostics): re-parse the
    /// endpoint and route through the endpoint's file identity — with
    /// a live request context through the freshness-checked navigation
    /// path (server columns), without one as a direct remote open at a
    /// byte column. The analogous local path is never touched.
    pub(crate) fn lsp_open_remote_hit(
        &mut self,
        endpoint: &strop_workspace::RemoteEndpoint,
        path: &Path,
        line: usize,
        col: usize,
        context: Option<strop_lsp::ReplyContext>,
    ) {
        if let Some(context) = context {
            self.jump_to_location(
                strop_lsp::ServerLocation {
                    doc: ResourceLocation::remote(endpoint.clone(), path.to_owned()),
                    position: strop_lsp::ServerPosition {
                        line: strop_core::id::LineIndex::new(line.saturating_sub(1)),
                        column: strop_lsp::ServerColumn::new(col.saturating_sub(1)),
                    },
                },
                context,
            );
        } else if let Some(seed) = self.remote_file_for(endpoint) {
            // Context-free remote hits (symbol rows) record too —
            // ctrl-o after the jump returns (0047 §1).
            self.push_jump();
            match seed.with_path(path.to_owned()) {
                Ok(file) => self.request_target(
                    crate::files::FileTarget::Remote(file.into()),
                    crate::editor::io::OpenIntent::Grep {
                        line: strop_core::id::LineIndex::new(line.saturating_sub(1)),
                        column: strop_core::id::ByteColumn::new(col.saturating_sub(1)),
                    },
                ),
                Err(error) => self.message = format!("lsp remote location: {error}"),
            }
        } else {
            self.message = "lsp: the remote workspace for this hit was closed".into();
        }
    }
}
