//! Editable filename fields use the lossless Directory spelling, not input URIs.
use std::path::PathBuf;
pub(super) fn decode(text: &str) -> Result<PathBuf, String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next().ok_or("unfinished filename escape")? {
                '\\' => bytes.push(b'\\'),
                'n' => bytes.push(b'\n'),
                'r' => bytes.push(b'\r'),
                't' => bytes.push(b'\t'),
                'x' => {
                    let high = characters
                        .next()
                        .and_then(|value| value.to_digit(16))
                        .ok_or("\\x needs two hexadecimal digits")?;
                    let low = characters
                        .next()
                        .and_then(|value| value.to_digit(16))
                        .ok_or("\\x needs two hexadecimal digits")?;
                    bytes.push(((high << 4) | low) as u8);
                }
                'u' => {
                    if characters.next() != Some('{') {
                        return Err("Unicode filename escapes use \\u{HEX}".into());
                    }
                    let mut value = 0_u32;
                    let mut count = 0;
                    loop {
                        let character = characters
                            .next()
                            .ok_or("unfinished Unicode filename escape")?;
                        if character == '}' {
                            break;
                        }
                        count += 1;
                        if count > 6 {
                            return Err("Unicode filename escape is too long".into());
                        }
                        value = value
                            .checked_mul(16)
                            .and_then(|value| {
                                character
                                    .to_digit(16)
                                    .and_then(|digit| value.checked_add(digit))
                            })
                            .ok_or("invalid Unicode filename escape")?;
                    }
                    if count == 0 {
                        return Err("empty Unicode filename escape".into());
                    }
                    let character =
                        char::from_u32(value).ok_or("invalid Unicode scalar in filename")?;
                    let mut encoded = [0; 4];
                    bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                }
                _ => return Err("unknown filename escape; use \\\\ for a literal backslash".into()),
            }
        } else {
            if character.is_control()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                return Err("control characters in filenames must use visible escapes".into());
            }
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
    }
    if bytes.contains(&0) {
        return Err("a filename cannot contain NUL".into());
    }
    let path =
        strop_workspace::addr::uri::bytes_to_path(bytes).map_err(|error| error.to_string())?;
    if path.is_absolute() || path.has_root() {
        return Err("draft destinations are relative to their Directory; use Move for an absolute destination".into());
    }
    if path.file_name().is_none() {
        return Err("a filename needs a nonempty final component".into());
    }
    Ok(path)
}
