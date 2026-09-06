//! Editor-specific producers. Storage and ordering belong to strop-trace.
pub mod frame;
pub mod services;
mod snapshot;

use super::{Editor, Key};
use serde::Serialize;
use std::cell::Cell;
use strop_trace::{enabled, record, EventKind};

thread_local! { static INPUT_DEPTH: Cell<usize> = const { Cell::new(0) }; }

pub struct InputScope;
impl InputScope {
    pub fn enter(editor: &Editor, key: Key) -> Option<Self> {
        if !enabled() {
            return None;
        }
        let depth = INPUT_DEPTH.with(|value| {
            let depth = value.get();
            value.set(depth + 1);
            depth
        });
        #[derive(Serialize)]
        struct Input {
            action: &'static str,
            source: InputSource,
            key: Key,
            replay: String,
        }
        let source = if editor.macro_depth > 0 {
            InputSource::Macro
        } else if depth > 0 {
            InputSource::Synthetic
        } else {
            InputSource::External
        };
        record(
            EventKind::Input,
            &Input {
                action: "key",
                source,
                key,
                replay: replay_token(key),
            },
        );
        Some(Self)
    }
}
impl Drop for InputScope {
    fn drop(&mut self) {
        INPUT_DEPTH.with(|value| value.set(value.get().saturating_sub(1)));
    }
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum InputSource {
    External,
    Synthetic,
    Macro,
}

pub fn replay_token(key: Key) -> String {
    match key {
        Key::Char('<') => "<lt>".into(),
        Key::Char('>') => "<gt>".into(),
        Key::Char(c) => c.to_string(),
        Key::Esc => "<esc>".into(),
        Key::Enter => "<cr>".into(),
        Key::Backspace => "<bs>".into(),
        Key::Up => "<up>".into(),
        Key::Down => "<down>".into(),
        Key::Left => "<left>".into(),
        Key::Right => "<right>".into(),
        Key::Tab => "<tab>".into(),
        Key::Backtab => "<s-tab>".into(),
        Key::CtrlD => "<c-d>".into(),
        Key::CtrlR => "<c-r>".into(),
        Key::CtrlO => "<c-o>".into(),
        Key::CtrlW => "<c-w>".into(),
        Key::CtrlX => "<c-x>".into(),
        Key::CtrlU => "<c-u>".into(),
        Key::CtrlF => "<c-f>".into(),
        Key::CtrlB => "<c-b>".into(),
        Key::CtrlCaret => "<c-^>".into(),
        Key::CtrlV => "<c-v>".into(),
        Key::CtrlL => "<c-l>".into(),
    }
}
