//! Chat history.

use lattice_proto::Text;
use std::time::Instant;

pub struct ChatLine {
    pub text: Text,
    pub at: Instant,
    /// The signature it came with, by which a server can take it back.
    pub signature: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct ChatLog {
    pub lines: Vec<ChatLine>,
    /// Lines the player typed, newest last (Up/Down to recall).
    pub sent: Vec<String>,
}

const MAX_LINES: usize = 200;

impl ChatLog {
    pub fn push(&mut self, text: Text) {
        self.push_signed(text, None);
    }

    pub fn push_signed(&mut self, text: Text, signature: Option<Vec<u8>>) {
        self.lines.push(ChatLine { text, at: Instant::now(), signature });
        if self.lines.len() > MAX_LINES {
            self.lines.drain(..self.lines.len() - MAX_LINES);
        }
    }

    /// A server's taken a message back: it's replaced by a note saying so.
    pub fn delete(&mut self, signature: &[u8]) {
        for line in &mut self.lines {
            if line.signature.as_deref() == Some(signature) {
                line.text = Text::translate("chat.deleted_marker", Vec::new());
                line.signature = None;
            }
        }
    }

    pub fn remember_sent(&mut self, s: &str) {
        if self.sent.last().map(String::as_str) != Some(s) {
            self.sent.push(s.to_string());
        }
    }
}
