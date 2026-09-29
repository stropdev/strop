//! Native event routing into the ordered bridge. This is not a second input
//! model: GPUI events become protocol facts, and the backend's acknowledged
//! publication remains the only source state.

use gpui::{KeyDownEvent, KeyUpEvent};
use strop_ui_protocol::AdmittedAction;

use crate::bridge::{BridgeAction, BridgeError, WslBridge};
use crate::input::{key_input, key_release_input};

/// Queue one native key press/repeat. Unknown platform keys are typed
/// transport failures, never guessed editor bindings.
pub fn admit_key_down(bridge: &WslBridge, event: &KeyDownEvent) -> Result<(), BridgeError> {
    let Some(input) = key_input(event) else {
        return Err(BridgeError::Transport(format!(
            "unmapped native key: {}",
            event.keystroke.key
        )));
    };
    bridge.queue(BridgeAction::Act(vec![AdmittedAction::Input(input)]))
}

/// Queue one native key release as its own admitted input; terminal owners
/// decide whether releases matter. No coalescing or speculative replay.
pub fn admit_key_up(bridge: &WslBridge, event: &KeyUpEvent) -> Result<(), BridgeError> {
    let Some(input) = key_release_input(event) else {
        return Err(BridgeError::Transport(format!(
            "unmapped native key release: {}",
            event.keystroke.key
        )));
    };
    bridge.queue(BridgeAction::Act(vec![AdmittedAction::Input(input)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admitted_routes_are_typed_by_native_key_kind() {
        let down = KeyDownEvent {
            keystroke: gpui::Keystroke {
                key: "escape".into(),
                ..Default::default()
            },
            is_held: false,
            prefer_character_input: false,
        };
        assert!(key_input(&down).is_some());
        let up = KeyUpEvent {
            keystroke: gpui::Keystroke {
                key: "escape".into(),
                ..Default::default()
            },
        };
        assert!(key_release_input(&up).is_some());
    }
}
