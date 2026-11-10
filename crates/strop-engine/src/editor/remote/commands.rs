//! `:remote` chooses a typed remote intent; transport work stays on jobs.
use super::{Editor, RemoteControl};
use strop_workspace::RemoteEndpoint;

impl Editor {
    pub(crate) fn run_remote_ex(&mut self, command: &str, argument: &str) -> bool {
        if command != "remote" {
            return false;
        }
        if let Err(message) = self.remote_ex(argument.trim()) {
            self.message = message;
        }
        true
    }
    fn remote_ex(&mut self, argument: &str) -> Result<(), String> {
        let words: Vec<_> = argument.split_whitespace().collect();
        let operation = match words.as_slice() {
            [] => {
                self.open_remote_picker();
                return Ok(());
            }
            ["root"] => return self.browse_remote_root(false),
            ["home"] => return self.browse_remote_root(true),
            ["connect", endpoint] => {
                RemoteControl::Connect(RemoteEndpoint::parse(endpoint).map_err(|e| e.to_string())?)
            }
            ["disconnect", endpoint] => RemoteControl::Disconnect(
                RemoteEndpoint::parse(endpoint).map_err(|e| e.to_string())?,
            ),
            ["disconnect"] => RemoteControl::Disconnect(
                self.remote_endpoint()
                    .ok_or("no remote endpoint; use :remote disconnect ssh://HOST")?
                    .clone(),
            ),
            ["clear"] => RemoteControl::DisconnectAll,
            ["list"] => RemoteControl::Connections,
            _ => {
                return Err(
                    "usage: :remote [root|home|connect URI|disconnect [URI]|clear|list]".into(),
                )
            }
        };
        self.request_remote_control(operation);
        Ok(())
    }
}
