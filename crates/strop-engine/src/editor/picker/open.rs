//! Picker openers and the item builders behind them.

use super::{Editor, Item, Kind, Payload, Picker, PickerGlue};

impl Editor {
    pub fn open_picker(&mut self, kind: Kind) {
        if kind == Kind::FilesystemActions {
            self.open_filesystem_actions();
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
                            Some(terminal) => {
                                let phase = self
                                    .terminal_phase(i)
                                    .map(terminal_phase_label)
                                    .unwrap_or_else(|| "unknown".into());
                                let directory = self
                                    .terminal_launch_directory(i)
                                    .and_then(|path| {
                                        path.file_name()
                                            .map(|name| name.to_string_lossy().into_owned())
                                    })
                                    .unwrap_or_default();
                                (
                                    Some("!"),
                                    format!(
                                        "terminal #{} · {directory} · {phase}",
                                        terminal.session.get()
                                    ),
                                )
                            }
                            None => (None, self.doc(i).label(&cwd)),
                        };
                        let badge = badge.map(str::to_owned);
                        Item {
                            badge,
                            text: name,
                            payload: Payload::Buffer(i),
                        }
                    })
                    .collect()
            }
            // Grep/Replace stream only once input registers a request;
            // Files launches its walk right after install.
            Kind::Files
            | Kind::Search
            | Kind::RemoteHosts
            | Kind::RemoteAddress
            | Kind::CodeActions
            | Kind::Containers => vec![],
            Kind::Jumps => unreachable!("the jumplist builds its own items"),
            Kind::SearchOptions => unreachable!("search options build their own items"),
            Kind::TabSize => unreachable!("the tab-size selector builds its own items"),
            Kind::FilesystemActions => {
                unreachable!("filesystem actions build their own captured selector")
            }
            Kind::Symbols | Kind::WorkspaceSymbols => vec![],
            Kind::Diagnostics | Kind::Locations => {
                unreachable!("location lists use PickerGlue::diagnostics")
            }
        };
        self.set_picker(PickerGlue::diagnostics(Picker::new(kind, items, false)));
        if kind == Kind::Files {
            self.picker_input_changed();
        }
        if kind == Kind::WorkspaceSymbols {
            self.picker_input_changed();
        }
    }

    /// `space S` (0063 §2): every declaration in the opened scope —
    /// syntax-fallback tier now, language servers merge in as they
    /// warm up. The canonical query AST drives admission; ranking is
    /// local per keystroke.
    pub(crate) fn open_workspace_symbols(&mut self) {
        self.open_picker(Kind::WorkspaceSymbols);
    }

    /// `:search-options` (0051 R03): the hidden/ignore controls with
    /// their live values; Enter toggles and the row updates in place.
    pub(crate) fn open_search_options(&mut self) {
        let item = |setting: strop_picker::SearchSetting, on: bool| strop_picker::Item {
            badge: None,
            text: format!(
                "{}: {}",
                match setting {
                    strop_picker::SearchSetting::Hidden => "hidden (dotfiles)",
                    strop_picker::SearchSetting::RespectIgnore => "ignored entries",
                },
                match (setting, on) {
                    (strop_picker::SearchSetting::Hidden, true)
                    | (strop_picker::SearchSetting::RespectIgnore, false) => "include",
                    _ => "exclude",
                },
            ),
            payload: strop_picker::Payload::SearchOption(setting),
        };
        let items = vec![
            item(
                strop_picker::SearchSetting::Hidden,
                self.config.search_show_hidden,
            ),
            item(
                strop_picker::SearchSetting::RespectIgnore,
                self.config.search_respect_ignore,
            ),
        ];
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::SearchOptions,
            items,
            false,
        )));
    }

    /// The jumplist as a menu (0047 §2): past newest-first, the current
    /// position marked, then the future; dead documents are filtered.
    pub(crate) fn open_jumps_picker(&mut self) {
        let mut items = Vec::new();
        for entry in self.jumplist_past.iter().rev() {
            items.extend(jump_row(self, entry, "  "));
        }
        items.extend(jump_row(self, &self.jump_record(), "> "));
        for entry in self.jumplist_future.iter().rev() {
            items.extend(jump_row(self, entry, "  "));
        }
        self.set_picker(PickerGlue::diagnostics(Picker::new(
            Kind::Jumps,
            items,
            false,
        )));
    }
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
    let line = doc.buf.line_of(record.offset.min(doc.buf.len_bytes()));
    let text: String = doc.buf.line_text(line).trim().chars().take(48).collect();
    Some(Item {
        badge: None,
        text: format!("{marker}{name}:{}  {text}", line + 1),
        payload: Payload::Jump {
            document: record.document,
            offset: record.offset,
        },
    })
}
