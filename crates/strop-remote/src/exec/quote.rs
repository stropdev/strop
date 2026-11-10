//! The one remote command line: sshd hands the string after the host to
//! the user's login shell (`$SHELL -c`), so every argv word and the
//! working directory travel POSIX single-quoted. Inside single quotes
//! nothing is special except `'` itself, which closes, escapes and
//! reopens (`'\''`). Words are native bytes; nothing here is UTF-8.

/// Append `word` single-quoted.
fn push_quoted(line: &mut Vec<u8>, word: &[u8]) {
    line.push(b'\'');
    for &byte in word {
        if byte == b'\'' {
            line.extend_from_slice(b"'\\''");
        } else {
            line.push(byte);
        }
    }
    line.push(b'\'');
}

/// `cd '<cwd>' && exec '<argv0>' '<argv1>' …`: `exec` replaces the login
/// shell, so signals and the exit status belong to the program itself.
/// An empty `argv` is a caller bug.
pub(crate) fn remote_line(argv: &[&[u8]], cwd: Option<&[u8]>) -> Vec<u8> {
    debug_assert!(!argv.is_empty(), "a remote command needs a program");
    let mut line = Vec::new();
    if let Some(cwd) = cwd {
        line.extend_from_slice(b"cd ");
        push_quoted(&mut line, cwd);
        line.extend_from_slice(b" && ");
    }
    line.extend_from_slice(b"exec");
    for word in argv {
        line.push(b' ');
        push_quoted(&mut line, word);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::remote_line;
    use std::process::Command;

    /// The line a real POSIX shell parses back into exactly the argv
    /// that was quoted: `printf '%s\0'` echoes each word NUL-terminated.
    fn round_trip(words: &[&[u8]]) -> Vec<Vec<u8>> {
        let mut argv: Vec<&[u8]> = vec![b"printf", b"%s\\0"];
        argv.extend_from_slice(words);
        let line = remote_line(&argv, Some(b"/"));
        use std::os::unix::ffi::OsStrExt;
        let output = Command::new("sh")
            .arg("-c")
            .arg(std::ffi::OsStr::from_bytes(&line))
            .output()
            .expect("sh runs");
        assert!(output.status.success(), "{output:?}");
        let mut parsed: Vec<Vec<u8>> = output
            .stdout
            .split(|byte| *byte == 0)
            .map(<[u8]>::to_vec)
            .collect();
        assert_eq!(parsed.pop(), Some(Vec::new()), "NUL-terminated words");
        parsed
    }

    #[test]
    fn hostile_words_survive_the_remote_shell_exactly() {
        let words: [&[u8]; 9] = [
            b"plain",
            b"two words",
            b"it's",
            b"'",
            b"$HOME `id` $(id) \\n",
            b"semi; rm -rf / && echo |",
            b"line\nbreak",
            b"*?[glob]~",
            b"bytes-\xff\xfe",
        ];
        assert_eq!(round_trip(&words), words.map(<[u8]>::to_vec));
    }

    #[test]
    fn empty_words_stay_words() {
        assert_eq!(
            round_trip(&[b"", b"x", b""]),
            vec![vec![], b"x".to_vec(), vec![]]
        );
    }

    #[test]
    fn cwd_is_quoted_and_command_replaces_the_shell() {
        assert_eq!(
            remote_line(&[b"git", b"log"], Some(b"/srv/it's here")),
            b"cd '/srv/it'\\''s here' && exec 'git' 'log'".to_vec()
        );
        assert_eq!(remote_line(&[b"rg"], None), b"exec 'rg'".to_vec());
    }
}
