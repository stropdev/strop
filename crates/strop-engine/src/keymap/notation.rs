//! The key notation (0112 §4.2): what each word of a help cell is —
//! a key typed as written, a named key, a chord, a placeholder, a
//! separator, a command name. Three pure lexers, one per cell kind; no
//! allocation, lossless (the tokens' texts concatenated are the input).
//! Dispatch reads the same word classes (`lookup::per_key`), so what the
//! help inks as one key is one key to the trie.

/// What a token of a help cell is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A key typed as written: `h`, `gg`, `]d`, `"+y`.
    Key,
    /// A key with a name: `space`, `esc`, `enter`.
    Named,
    /// A modified key: `ctrl-w`, `alt-c`, `s-tab`.
    Chord,
    /// Stands for what you type: `<c>`, `{reg}`, `[file]`'s `file`.
    Placeholder,
    /// Not typed: ` / ` between alternatives, brackets in a usage.
    Separator,
    /// A command name (`:w`, `strop`) or a filter key (`language:`).
    Command,
    /// The optional rest of a command name: `elete` in `:bd[elete]`.
    Tail,
    /// The blanks between tokens.
    Space,
}

/// One token: its kind and its exact bytes of the cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind,
    pub text: &'a str,
}

/// The names of keys that are not written as themselves.
pub const NAMED: &[&str] = &[
    "space",
    "esc",
    "enter",
    "tab",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
];

/// A word the notation reads as one named key.
pub fn is_named(word: &str) -> bool {
    NAMED.contains(&word)
}

/// A modified key: `ctrl-`, `alt-` or `s-` with a key after it.
pub fn is_chord(word: &str) -> bool {
    ["ctrl-", "alt-", "s-"]
        .iter()
        .any(|prefix| word.len() > prefix.len() && word.starts_with(prefix))
}

/// A word dispatch reads as one key, never split into characters.
pub fn is_single_key(word: &str) -> bool {
    is_named(word) || is_chord(word)
}

fn token(kind: TokenKind, text: &str) -> Token<'_> {
    Token { kind, text }
}

/// The end of the run of spaces starting at `at`.
fn spaces_end(text: &str, at: usize) -> usize {
    text[at..]
        .find(|c: char| c != ' ')
        .map_or(text.len(), |offset| at + offset)
}

/// The end of the word starting at `at`.
fn word_end(text: &str, at: usize) -> usize {
    text[at..]
        .find(' ')
        .map_or(text.len(), |offset| at + offset)
}

/// A binding row's key cell (§4.2 `keys`).
pub fn keys(cell: &str) -> Keys<'_> {
    Keys {
        cell,
        at: 0,
        word: None,
        command: None,
        seen_word: false,
    }
}

/// The tokens of a key cell; see [`keys`].
pub struct Keys<'a> {
    cell: &'a str,
    at: usize,
    /// The end of the word being lexed piece by piece (rule 5).
    word: Option<usize>,
    /// A `:command` run handed to the command lexer (rule 4).
    command: Option<Command<'a>>,
    seen_word: bool,
}

/// `<letters>` or `{letters}` at the start of `text`: its length.
fn placeholder_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let close = match bytes.first() {
        Some(b'<') => b'>',
        Some(b'{') => b'}',
        _ => return None,
    };
    let letters = bytes[1..]
        .iter()
        .take_while(|byte| byte.is_ascii_alphabetic())
        .count();
    (letters > 0 && bytes.get(1 + letters) == Some(&close)).then_some(letters + 2)
}

impl<'a> Keys<'a> {
    /// One piece of a plain word: a placeholder, or the key run before
    /// the next one.
    fn piece(&mut self, end: usize) -> Token<'a> {
        let rest = &self.cell[self.at..end];
        let (kind, len) = match placeholder_len(rest) {
            Some(len) => (TokenKind::Placeholder, len),
            None => {
                let len = rest
                    .char_indices()
                    .skip(1)
                    .find(|&(at, _)| placeholder_len(&rest[at..]).is_some())
                    .map_or(rest.len(), |(at, _)| at);
                (TokenKind::Key, len)
            }
        };
        let text = &rest[..len];
        self.at += len;
        if self.at == end {
            self.word = None;
        }
        token(kind, text)
    }
}

impl<'a> Iterator for Keys<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        if let Some(command) = &mut self.command {
            match command.next() {
                Some(token) => return Some(token),
                None => self.command = None,
            }
        }
        if self.at >= self.cell.len() {
            return None;
        }
        if let Some(end) = self.word {
            return Some(self.piece(end));
        }
        let start = self.at;
        if self.cell.as_bytes()[start] == b' ' {
            self.at = spaces_end(self.cell, start);
            return Some(token(TokenKind::Space, &self.cell[start..self.at]));
        }
        let end = word_end(self.cell, start);
        let word = &self.cell[start..end];
        let first = !self.seen_word;
        self.seen_word = true;
        let last = self.cell[end..].trim_start().is_empty();
        // A `/` between two words is an alternative; a leading one is the
        // search key and a trailing one a key of the sequence (`space /`),
        // as `lookup::expand` reads them.
        let kind = if word == "/" && !first && !last {
            Some(TokenKind::Separator)
        } else if is_named(word) {
            Some(TokenKind::Named)
        } else if is_chord(word) {
            Some(TokenKind::Chord)
        } else {
            None
        };
        if let Some(kind) = kind {
            self.at = end;
            return Some(token(kind, word));
        }
        if word.len() > 1 && word.starts_with(':') {
            // the command runs to the next word that starts with `:`
            let mut stop = end;
            while stop < self.cell.len() {
                let next = spaces_end(self.cell, stop);
                if next >= self.cell.len() || self.cell.as_bytes()[next] == b':' {
                    break;
                }
                stop = word_end(self.cell, next);
            }
            self.at = stop;
            let mut command = command(&self.cell[start..stop]);
            let first = command.next();
            self.command = Some(command);
            return first;
        }
        self.word = Some(end);
        Some(self.piece(end))
    }
}

/// A command's usage (§4.2 `command`): `:bd[elete][!] [number]`.
pub fn command(cell: &str) -> Command<'_> {
    Command {
        text: cell,
        at: 0,
        depth: 0,
        name_next: true,
        tail_ready: false,
        in_tail: false,
    }
}

/// The tokens of a usage; see [`command`].
pub struct Command<'a> {
    text: &'a str,
    at: usize,
    depth: usize,
    /// The next word is a command name.
    name_next: bool,
    /// A `[` here opens the name's tail (it follows a letter of the
    /// name, or a tail group's `]`).
    tail_ready: bool,
    in_tail: bool,
}

impl<'a> Iterator for Command<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        let start = self.at;
        let rest = &self.text[start..];
        let first = *rest.as_bytes().first()?;
        if self.name_next {
            self.name_next = false;
            let len = rest.find([' ', '[', '{', '/']).unwrap_or(rest.len());
            if len > 0 {
                let name = &rest[..len];
                self.at += len;
                self.tail_ready = name.as_bytes()[len - 1].is_ascii_alphabetic();
                return Some(token(TokenKind::Command, name));
            }
        }
        let tail_ready = std::mem::take(&mut self.tail_ready);
        let (kind, len) = match first {
            b' ' => {
                let end = spaces_end(self.text, start);
                self.name_next = self.text.as_bytes().get(end) == Some(&b':');
                (TokenKind::Space, end - start)
            }
            b'[' | b'{' => {
                if first == b'[' && tail_ready && self.depth == 0 {
                    self.in_tail = true;
                }
                self.depth += 1;
                (TokenKind::Separator, 1)
            }
            b']' | b'}' => {
                self.depth = self.depth.saturating_sub(1);
                if first == b']' && std::mem::take(&mut self.in_tail) {
                    self.tail_ready = true;
                }
                (TokenKind::Separator, 1)
            }
            b'|' => (TokenKind::Separator, 1),
            b'/' if self.depth == 0 => (TokenKind::Separator, 1),
            _ => {
                let depth = self.depth;
                let len = rest
                    .find(|c: char| {
                        matches!(c, ' ' | '[' | ']' | '{' | '}' | '|') || (c == '/' && depth == 0)
                    })
                    .unwrap_or(rest.len());
                let kind = if self.in_tail {
                    TokenKind::Tail
                } else {
                    TokenKind::Placeholder
                };
                (kind, len)
            }
        };
        self.at += len;
        Some(token(kind, &rest[..len]))
    }
}

/// A filter cell (§4.2 `filter`): `language:rust`, `-kind:`.
pub fn filter(cell: &str) -> Filter<'_> {
    Filter {
        cell,
        at: 0,
        value: None,
    }
}

/// The tokens of a filter cell; see [`filter`].
pub struct Filter<'a> {
    cell: &'a str,
    at: usize,
    /// The value after a word's key, still to come.
    value: Option<&'a str>,
}

impl<'a> Iterator for Filter<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        if let Some(value) = self.value.take() {
            return Some(token(TokenKind::Placeholder, value));
        }
        let start = self.at;
        let first = *self.cell.as_bytes().get(start)?;
        if first == b' ' {
            self.at = spaces_end(self.cell, start);
            return Some(token(TokenKind::Space, &self.cell[start..self.at]));
        }
        let end = word_end(self.cell, start);
        self.at = end;
        let word = &self.cell[start..end];
        Some(match word.find(':') {
            Some(colon) => {
                let (key, value) = word.split_at(colon + 1);
                self.value = (!value.is_empty()).then_some(value);
                token(TokenKind::Command, key)
            }
            None => token(TokenKind::Key, word),
        })
    }
}

#[cfg(test)]
mod tests;
