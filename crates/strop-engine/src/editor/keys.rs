//! Streaming script-token decoding shared by Editor::feed_text and headless.
//! Unknown tokens stay literal, and a lone '<' never fabricates a closing '>'.
use super::Key;

pub fn parse(text: &str) -> Keys<'_> {
    Keys {
        remaining: text,
        literal_bytes: 0,
    }
}
pub struct Keys<'a> {
    remaining: &'a str,
    literal_bytes: usize,
}
impl Iterator for Keys<'_> {
    type Item = Key;
    fn next(&mut self) -> Option<Key> {
        let character = self.remaining.chars().next()?;
        if self.literal_bytes > 0 {
            self.remaining = &self.remaining[character.len_utf8()..];
            self.literal_bytes -= character.len_utf8();
            return Some(Key::Char(character));
        }
        if character == '<' {
            if let Some(end) = self.remaining.find('>') {
                let token = &self.remaining[1..end];
                if let Some((_, key)) = TOKENS
                    .iter()
                    .find(|(name, _)| token.eq_ignore_ascii_case(name))
                {
                    self.remaining = &self.remaining[end + 1..];
                    return Some(*key);
                }
                self.literal_bytes = end;
            }
        }
        self.remaining = &self.remaining[character.len_utf8()..];
        Some(match character {
            '\x1b' => Key::Esc,
            '\r' | '\n' => Key::Enter,
            '\x7f' => Key::Backspace,
            character => Key::Char(character),
        })
    }
}
const TOKENS: &[(&str, Key)] = &[
    ("esc", Key::Esc),
    ("cr", Key::Enter),
    ("enter", Key::Enter),
    ("bs", Key::Backspace),
    ("space", Key::Char(' ')),
    ("lt", Key::Char('<')),
    ("gt", Key::Char('>')),
    ("up", Key::Up),
    ("down", Key::Down),
    ("left", Key::Left),
    ("right", Key::Right),
    ("tab", Key::Tab),
    ("s-tab", Key::Backtab),
    ("c-r", Key::CtrlR),
    ("c-x", Key::CtrlX),
    ("c-d", Key::CtrlD),
    ("c-u", Key::CtrlU),
    ("c-f", Key::CtrlF),
    ("c-b", Key::CtrlB),
    ("c-^", Key::CtrlCaret),
    ("c-v", Key::CtrlV),
    ("c-w", Key::CtrlW),
    ("c-o", Key::CtrlO),
    ("c-l", Key::CtrlL),
    ("c-space", Key::CtrlSpace),
];
