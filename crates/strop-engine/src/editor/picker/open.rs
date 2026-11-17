//! Picker openers and the item builders behind them.

use super::{Editor, Item, Kind, Payload, Picker, PickerGlue};
use strop_picker::ItemFacts;

impl Editor {
    pub fn open_picker(&mut self, kind: Kind) {
        if kind == Kind::Actions {
            self.context_actions();
            return;
        }
        if kind == Kind::Search {
            self.open_search(false);
            return;
        }
        if kind == Kind::RemoteHosts {
            self.open_remote_picker();
            return;
        }
        if kind == Kind::Jumps {
            self.open_jumps_picker();
            return;
        }
        if kind == Kind::RemoteAddress {
            self.open_remote_address();
            return;
        }
        if kind == Kind::TabSize {
            self.open_tab_size_picker();
            return;
        }
        if kind == Kind::Branches {
            self.open_branches();
            return;
        }
        let items = match kind {
            Kind::Buffers => {
                let cwd = self.cwd.clone();
                self.mru
                    .iter()
                    .map(|&i| {
                        // Terminals are real switchable buffers: they list
                        // with vim's `!` job flag and their live phase, so
                        // "what is open?" has one truthful answer.
                        let (badge, name) = match self.terminal_document(i) {
                            Some(_) => {
                                let phase = self
                                    .terminal_phase(i)
                                    .map(terminal_phase_label)
                                    .unwrap_or_else(|| "unknown".into());
                                (Some("!"), format!("{} · {phase}", self.doc(i).label(&cwd)))
                            }
                            None => (None, self.doc(i).label(&cwd)),
                        };
                        let badge = badge.map(std::borrow::Cow::Borrowed);
                        Item {
                            badge,
                            text: name,
                            payload: Payload::Buffer(i),
                            facts: ItemFacts {
                                path: document_path(self.doc(i), &cwd),
                                ..ItemFacts::default()
                            },
                        }
                    })
                    .collect()
            }
            // Grep/Replace stream only once input registers a request;
            // Files launches its walk right after install.
            Kind::Files | Kind::Search | Kind::RemoteHosts | Kind::RemoteAddress => vec![],
            Kind::Jumps => unreachable!("the jumplist builds its own items"),
            Kind::TabSize => unreachable!("the tab-size selector builds its own items"),
            Kind::Theme => unreachable!("the theme selector builds its own items"),
            Kind::Branches => unreachable!("the branch list arrives from its job"),
            Kind::Actions => unreachable!("actions build their own captured selector"),
            Kind::Symbols | Kind::WorkspaceSymbols => vec![],
            Kind::Diagnostics | Kind::Locations => {
                unreachable!("location lists use PickerGlue::diagnostics")
            }
        };
        let mut picker = Picker::new(kind, items, false);
        match kind {
            // The current buffer leads the list; the bar rests on the one
            // before it, so `Space b` Enter is the alternate (0093 §3.6).
            Kind::Buffers => picker.resting_row = 1,
            Kind::Files => picker.priors = self.file_priors(),
            _ => {}
        }
        self.set_picker(PickerGlue::diagnostics(picker));
        if kind == Kind::Files {
            self.picker_input_changed();
        }
        if kind == Kind::WorkspaceSymbols {
            self.picker_input_changed();
        }
    }

    /// What the finder leads with (0093 §3.4): the other open files,
    /// most recently used first, then the current one, then this
    /// project's frecent files. The current file trails the open ones so
    /// `Space f` Enter is the file you were just in, never a no-op.
    fn file_priors(&self) -> strop_picker::rank::Priors {
        let root = self.current_workspace_root();
        if root.filesystem != strop_workspace::Filesystem::Local {
            return Default::default();
        }
        let current = self.current();
        let open = |id: &strop_core::id::DocumentId| {
            let path = self.docs.get(*id)?.buf.path.as_ref()?;
            let relative = path.strip_prefix(&root.path).ok()?;
            Some(strop_picker::display_path(relative).into_owned())
        };
        // Frecent paths are filed relative to the working directory; the
        // finder's rows are relative to its own root.
        let frecent = self.picker_ranking.recent.ranked().into_iter();
        let frecent = frecent.filter_map(|path| {
            let path = self.cwd.join(path);
            let relative = path.strip_prefix(&root.path).ok()?;
            Some(strop_picker::display_path(relative).into_owned())
        });
        let others = self.mru.iter().filter(|id| **id != current);
        strop_picker::rank::Priors::new(
            others.filter_map(open).chain(open(&current)).chain(frecent),
        )
    }

    /// `space S` (0063 §2): every declaration in the opened scope —
    /// syntax-fallback tier now, language servers merge in as they
    /// warm up. The canonical query AST drives admission; ranking is
    /// local per keystroke.
    pub(crate) fn open_workspace_symbols(&mut self) {
        self.open_picker(Kind::WorkspaceSymbols);
    }

    /// The jumplist as a menu: older records newest-first, the current
    /// position marked, then newer records; dead documents are filtered.
    pub(crate) fn open_jumps_picker(&mut self) {
        let items = jump_items(self);
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::Jumps,
            items,
            false,
        )));
    }
}

/// Older entries appear newest-first, followed by the selected stored entry,
/// the synthetic live row and newer entries nearest-first.
fn jump_items(editor: &Editor) -> Vec<Item> {
    let jumps = &editor.view().jumps;
    let selected = jumps.position.unwrap_or(jumps.entries.len());
    let mut items = Vec::new();
    for entry in jumps.entries[..selected].iter().rev() {
        items.extend(jump_row(editor, entry, "  "));
    }
    if let Some(entry) = jumps.entries.get(selected) {
        items.extend(jump_row(editor, entry, "  "));
    }
    items.extend(jump_row(editor, &editor.jump_record(), "> "));
    for entry in &jumps.entries[selected.saturating_add(1).min(jumps.entries.len())..] {
        items.extend(jump_row(editor, entry, "  "));
    }
    items
}

/// The workspace-relative (or native remote) path a buffer or jump row
/// is filtered by. Nameless scratch and output buffers have none, so
/// path filters keep them (0063 §4).
fn document_path(
    document: &super::super::Document,
    cwd: &std::path::Path,
) -> Option<std::sync::Arc<str>> {
    if let Some(remote) = document.remote_metadata() {
        return Some(remote.file.to_string().into());
    }
    document.buf.path.as_ref().map(|path| {
        path.strip_prefix(cwd)
            .unwrap_or(path)
            .display()
            .to_string()
            .into()
    })
}

/// One jumplist row; dead documents drop out (0047 §2). The payload
/// stays a plain destination — accepting a menu entry is a NEW jump
/// landing (0051 §7), not a ctrl-o view restore.
fn terminal_phase_label(phase: &strop_terminal::model::Phase) -> String {
    use strop_terminal::model::Phase;
    match phase {
        Phase::Starting => "starting".into(),
        Phase::Running => "running".into(),
        Phase::Closing => "closing".into(),
        Phase::Exited {
            code: Some(code),
            signal: None,
        } => format!("exited {code}"),
        Phase::Exited {
            code: None,
            signal: Some(signal),
        } => format!("killed {signal}"),
        Phase::Exited { .. } => "exited".into(),
        Phase::Failed(_) => "failed".into(),
    }
}
fn jump_row(
    editor: &Editor,
    record: &super::super::jumps::JumpRecord,
    marker: &str,
) -> Option<Item> {
    let doc = editor.docs.get(record.document)?;
    let name = doc.label(&editor.cwd);
    let offset = record.offset.min(doc.buf.len_bytes());
    let line = doc.buf.line_of(offset);
    let line_text: String = doc.buf.line_text(line);
    let shown: String = line_text.trim().chars().take(48).collect();
    let text = format!("{marker}{name}:{}  {shown}", line + 1);
    // Content filters match the line, not the file name before it.
    let subject = (text.len() - shown.len()) as u32..text.len() as u32;
    Some(Item {
        badge: None,
        text,
        payload: Payload::Jump {
            document: record.document,
            offset: record.offset,
        },
        facts: ItemFacts {
            path: document_path(doc, &editor.cwd),
            subject: Some(subject),
            ..ItemFacts::default()
        },
    })
}
