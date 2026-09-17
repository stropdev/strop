//! `strop --ui-stdio` (0056 AR09): the non-graphical UI backend. One
//! editor, one framed protocol client on stdin/stdout (stdout carries
//! frames only; bounded diagnostics stay on stderr), served through the
//! SAME admitted engine paths the TUI and headless driver use.
//!
//! Editor setup mirrors the headless composition root: services start
//! through the admitted `Start` action, sessions stay disabled (the
//! client owns persistence decisions), and the shared bounded event
//! channel carries every async completion.

mod serve;
mod snapshot;

use std::error::Error;
use std::io;

use strop_core::Buffer;
use strop_engine::{config, editor, session};

pub fn run() -> Result<(), Box<dyn Error>> {
    let mut editor = editor::Editor::new(Buffer::from_text(""));
    editor.set_terminal_keyboard(strop_terminal::model::SUPPORTED_KEYBOARD_FLAGS)?;
    let (configuration, error) = config::Config::load();
    editor.set_config(configuration);
    editor.reresolve_indents();
    editor.set_state_dir(session::state_root());
    editor.set_session_policy(session::SessionPolicy::Disabled);
    if let Some(error) = error {
        editor.set_message(error);
    }
    let (tx, events) = editor::events::channel();
    editor.connect_events(tx);
    let tick = editor.tape().sample_tick();
    editor.recorded_action(editor::trace::drive::Action::Start { open: None }, tick)?;
    serve::Session::new().serve(&mut editor, &events, io::stdin(), io::stdout().lock())?;
    Ok(())
}
