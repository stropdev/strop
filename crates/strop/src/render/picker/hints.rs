//! The picker's bottom-border hints: whole chords, by priority (0050 §8,
//! rule 10). The card primitive drops trailing groups that do not fit,
//! so this module only decides what the groups are and in which order.

use crate::editor::Editor;
use crate::render::card::Hint;

const ENTER_OPEN: Hint<'static> = Hint {
    key: "enter",
    desc: "open",
};
const ESC_CLOSE: Hint<'static> = Hint {
    key: "esc",
    desc: "close",
};
const COLLECT: Hint<'static> = Hint {
    key: "ctrl-o",
    desc: "collect",
};

pub(super) fn for_picker(
    editor: &Editor,
    glue: &strop_engine::editor::PickerGlue,
    card_width: u16,
) -> Vec<Hint<'static>> {
    use strop_picker::Kind;
    let kind = glue.picker.kind;
    if glue.suggestions.is_some() {
        return vec![
            Hint {
                key: "enter",
                desc: "accept suggestion",
            },
            Hint {
                key: "↑↓",
                desc: "choose",
            },
            Hint {
                key: "esc",
                desc: "dismiss",
            },
        ];
    }
    let narrow = card_width < 64;
    match kind {
        Kind::Search => {
            let read_only = editor
                .search_scope()
                .is_some_and(|scope| scope.root.local_path().is_none());
            let replacing = glue.picker.field == strop_picker::Field::Replace;
            let mut hints = vec![if replacing {
                Hint {
                    key: "enter",
                    desc: "review",
                }
            } else {
                ENTER_OPEN
            }];
            if read_only {
                hints.push(Hint {
                    key: "replace",
                    desc: "unavailable",
                });
            } else if replacing {
                hints.push(Hint {
                    key: "tab",
                    desc: "field",
                });
                hints.push(Hint {
                    key: "ctrl-t",
                    desc: "replace",
                });
            } else {
                hints.push(Hint {
                    key: "ctrl-t",
                    desc: "replace",
                });
            }
            if !narrow {
                hints.push(Hint {
                    key: "ctrl-x",
                    desc: "drop hit",
                });
                hints.push(Hint {
                    key: "ctrl-d",
                    desc: "drop file",
                });
                hints.push(COLLECT);
                if !replacing && !read_only {
                    hints.push(Hint {
                        key: "ctrl-space",
                        desc: "suggest",
                    });
                }
            }
            hints.push(ESC_CLOSE);
            hints
        }
        Kind::RemoteAddress => vec![
            Hint {
                key: "enter",
                desc: "connect",
            },
            ESC_CLOSE,
        ],
        Kind::RemoteHosts => vec![
            Hint {
                key: "enter",
                desc: "choose",
            },
            Hint {
                key: "↑↓",
                desc: "move",
            },
            ESC_CLOSE,
        ],
        Kind::Locations | Kind::Diagnostics => vec![ENTER_OPEN, COLLECT, ESC_CLOSE],
        Kind::Theme => vec![
            Hint {
                key: "enter",
                desc: "apply",
            },
            ESC_CLOSE,
        ],
        _ => vec![
            ENTER_OPEN,
            Hint {
                key: "↑↓",
                desc: "move",
            },
            // the advertised paste is vim's: the unnamed register
            // (0091 §1) — the terminal's own paste stays the clipboard
            Hint {
                key: "ctrl-r \"",
                desc: "paste yank",
            },
            ESC_CLOSE,
        ],
    }
}
