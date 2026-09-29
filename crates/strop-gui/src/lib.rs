#![cfg(target_os = "windows")]

//! Native Windows frontend composition root. Editing state and admitted
//! actions remain the backend's; this package composes GPUI ownership with
//! the shared readonly protocol cache. No editor grammar, document state or
//! second protocol reducer lives here.

pub use gpui_platform::application;
pub use strop_ui_protocol::{AdmittedAction, Client, ClientEvent, ViewSnapshot};

pub mod bridge;
pub mod commands;
pub mod input;
pub mod wsl;
