//! Text commands choose typed remote intents; transport work stays on workers.
use super::{Editor, RemoteControl, RemoteView};
use crate::editor::io::OpenIntent;
use crate::files::FileTarget;
use strop_remote::{ReadLimit, ReadSelection, RemoteEndpoint, RemoteLocation};

impl Editor {
    pub(crate) fn run_remote_ex(&mut self, command: &str, argument: &str) -> bool {
        if !matches!(
            command,
            "tail" | "range" | "follow" | "unfollow" | "browse" | "filter" | "remote"
        ) {
            return false;
        }
        if let Err(message) = self.remote_ex(command, argument.trim()) {
            self.message = message;
        }
        true
    }
    fn remote_ex(&mut self, command: &str, argument: &str) -> Result<(), String> {
        let words: Vec<_> = argument.split_whitespace().collect();
        match command {
            "filter" => self.filter_remote_directory(argument.to_owned()),
            "unfollow" if words.is_empty() => {
                self.message = if self.stop_remote_follow(self.current()) {
                    "follow stopped"
                } else {
                    "not following"
                }
                .into();
                Ok(())
            }
            "remote" => {
                let operation = match words.as_slice() {
                    [] => {
                        self.open_remote_picker();
                        return Ok(());
                    }
                    ["root"] => return self.browse_remote_root(false),
                    ["home"] => return self.browse_remote_root(true),
                    ["edit"] => return self.enable_remote_edit(),
                    ["verify"] => return self.verify_remote_save(),
                    ["connect", endpoint] => RemoteControl::Connect(
                        RemoteEndpoint::parse(endpoint).map_err(|e| e.to_string())?,
                    ),
                    ["disconnect", endpoint] => RemoteControl::Disconnect(
                        RemoteEndpoint::parse(endpoint).map_err(|e| e.to_string())?,
                    ),
                    ["disconnect"] => RemoteControl::Disconnect(
                        self.remote_file()
                            .ok_or("no remote endpoint; use :remote disconnect ssh://HOST")?
                            .endpoint()
                            .clone(),
                    ),
                    ["clear"] => RemoteControl::DisconnectAll,
                    ["list"] => RemoteControl::Connections,
                    _ => return Err(
                        "usage: :remote [edit|verify|root|home|connect URI|disconnect [URI]|clear|list]"
                            .into(),
                    ),
                };
                self.request_remote_control(operation);
                Ok(())
            }
            "browse" if words.len() <= 1 => {
                let location = if let Some(uri) = words.first() {
                    RemoteLocation::parse(uri).map_err(|error| error.to_string())?
                } else if let Some(directory) = self.remote_directory() {
                    directory.directory.clone().into()
                } else {
                    let file = self
                        .remote_file()
                        .ok_or(":browse needs an ssh:// directory")?;
                    file.with_path(
                        file.path()
                            .parent()
                            .ok_or("remote path has no parent")?
                            .to_owned(),
                    )
                    .map_err(|error| error.to_string())?
                    .into()
                };
                self.request_target(FileTarget::Remote(location), OpenIntent::Browse);
                Ok(())
            }
            "follow" if words.len() <= 1 => {
                let limit = self
                    .cur()
                    .remote_metadata()
                    .and_then(|source| match source.selection {
                        ReadSelection::Tail(limit) => Some(limit),
                        _ => None,
                    })
                    .unwrap_or(ReadLimit::DEFAULT_TAIL);
                let location = self.remote_operand(words.first().copied())?;
                self.request_target(
                    FileTarget::Remote(location),
                    OpenIntent::RemoteView {
                        view: RemoteView::Follow(limit),
                        line: None,
                    },
                );
                Ok(())
            }
            "tail" if words.len() <= 2 => {
                let (limit, uri) = match words.as_slice() {
                    [] => (ReadLimit::DEFAULT_TAIL, None),
                    [uri] if uri.starts_with("ssh://") => (ReadLimit::DEFAULT_TAIL, Some(*uri)),
                    [bytes] => (super::view::limit(bytes).map_err(|e| e.to_string())?, None),
                    [bytes, uri] => (
                        super::view::limit(bytes).map_err(|e| e.to_string())?,
                        Some(*uri),
                    ),
                    _ => unreachable!("tail argument count checked"),
                };
                let location = self.remote_operand(uri)?;
                self.request_target(
                    FileTarget::Remote(location),
                    OpenIntent::RemoteView {
                        view: RemoteView::Snapshot(ReadSelection::Tail(limit)),
                        line: None,
                    },
                );
                Ok(())
            }
            "range" if words.len() == 3 => {
                let selection = ReadSelection::Range {
                    start: super::view::offset(words[0]).map_err(|e| e.to_string())?,
                    length: super::view::limit(words[1]).map_err(|e| e.to_string())?,
                };
                let location = self.remote_operand(Some(words[2]))?;
                self.request_target(
                    FileTarget::Remote(location),
                    OpenIntent::RemoteView {
                        view: RemoteView::Snapshot(selection),
                        line: None,
                    },
                );
                Ok(())
            }
            _ => Err(format!("usage: :{command} — see :help for remote commands")),
        }
    }
    fn remote_operand(&self, uri: Option<&str>) -> Result<RemoteLocation, String> {
        match uri {
            Some(uri) => RemoteLocation::parse(uri).map_err(|error| error.to_string()),
            None => self
                .remote_file()
                .cloned()
                .map(Into::into)
                .ok_or("command requires an ssh:// file".into()),
        }
    }
}
