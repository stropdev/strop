//! `:messages` (vim's message history): the last [`MESSAGE_LOG_BOUND`]
//! things the echo row said — every ephemeral `Editor.message` (refusals,
//! ex errors, confirmations) and every feedback record, with its severity
//! where the record knows it. The echo row shows one message at a time
//! and clips at three rows; this is where the whole text stays readable.

use std::collections::VecDeque;
use std::fmt::Write as _;

use super::FeedbackSeverity;

pub(crate) const MESSAGE_LOG_BOUND: usize = 50;

pub(crate) struct LoggedMessage {
    /// `None` for the untyped ephemeral channel.
    severity: Option<FeedbackSeverity>,
    text: String,
}

#[derive(Default)]
pub(crate) struct MessageLog {
    entries: VecDeque<LoggedMessage>,
    /// The ephemeral message last logged, so a message that stays on
    /// screen across events is logged once. Cleared with the message on
    /// every key, so the same refusal on a later key is logged again.
    echoed: String,
}

impl MessageLog {
    pub(crate) fn push(&mut self, severity: Option<FeedbackSeverity>, text: &str) {
        if self.entries.len() == MESSAGE_LOG_BOUND {
            self.entries.pop_front();
        }
        self.entries.push_back(LoggedMessage {
            severity,
            text: text.to_owned(),
        });
    }

    /// Log the ephemeral message if it is new since the last observation.
    /// Allocates only when a new message appeared.
    pub(crate) fn observe(&mut self, message: &str) {
        if message.is_empty() || message == self.echoed {
            return;
        }
        self.echoed.clear();
        self.echoed.push_str(message);
        self.push(None, message);
    }

    /// The key that clears the echo row forgets it here too.
    pub(crate) fn forget_echo(&mut self) {
        self.echoed.clear();
    }

    pub(super) fn render(&self) -> String {
        let mut text = String::from("messages — oldest first   (q: close)\n");
        if self.entries.is_empty() {
            text.push_str("  no messages yet\n");
        }
        for entry in &self.entries {
            let glyph = match entry.severity {
                Some(FeedbackSeverity::Failure) => "✗",
                Some(FeedbackSeverity::Warning) => "▲",
                Some(FeedbackSeverity::Info) => "●",
                None => "›",
            };
            for (index, line) in entry.text.lines().enumerate() {
                let lead = if index == 0 { glyph } else { " " };
                let _ = writeln!(text, "{lead} {line}");
            }
        }
        text
    }
}
