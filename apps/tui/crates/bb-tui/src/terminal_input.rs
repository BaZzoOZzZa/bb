use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub const DETACH_BYTE: u8 = 0x1c;
pub const DETACH_PREFIX: u8 = 0x02;
pub const DETACH_SUFFIX: u8 = b'd';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModes {
    pub application_cursor: bool,
}

pub fn is_detach_key(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('\\') | KeyCode::Char('4'))
}

fn modifier_param(modifiers: KeyModifiers) -> u8 {
    1 + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL))
}

fn ctrl_byte(ch: char) -> Option<u8> {
    Some(match ch {
        'a'..='z' => ch as u8 - b'a' + 1,
        'A'..='Z' => ch as u8 - b'A' + 1,
        '@' | ' ' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

fn cursor_key(letter: char, modifiers: KeyModifiers, modes: KeyModes) -> Vec<u8> {
    let param = modifier_param(modifiers);
    if param > 1 {
        format!("\x1b[1;{param}{letter}").into_bytes()
    } else if modes.application_cursor {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn tilde_key(code: u8, modifiers: KeyModifiers) -> Vec<u8> {
    let param = modifier_param(modifiers);
    if param > 1 {
        format!("\x1b[{code};{param}~").into_bytes()
    } else {
        format!("\x1b[{code}~").into_bytes()
    }
}

fn ss3_key(letter: char, modifiers: KeyModifiers) -> Vec<u8> {
    let param = modifier_param(modifiers);
    if param > 1 {
        format!("\x1b[1;{param}{letter}").into_bytes()
    } else {
        format!("\x1bO{letter}").into_bytes()
    }
}

fn with_alt(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
    if alt && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }
    bytes
}

pub fn encode_key(key: &KeyEvent, modes: KeyModes) -> Vec<u8> {
    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char(ch) if ctrl => match ctrl_byte(ch) {
            Some(byte) => with_alt(alt, vec![byte]),
            None => with_alt(alt, ch.to_string().into_bytes()),
        },
        KeyCode::Char(ch) => with_alt(alt, ch.to_string().into_bytes()),
        KeyCode::Enter => with_alt(alt, vec![b'\r']),
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => b"\x1b[Z".to_vec(),
        KeyCode::Tab => with_alt(alt, vec![b'\t']),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if ctrl => with_alt(alt, vec![0x08]),
        KeyCode::Backspace => with_alt(alt, vec![0x7f]),
        KeyCode::Esc => with_alt(alt, vec![0x1b]),
        KeyCode::Up => cursor_key('A', modifiers, modes),
        KeyCode::Down => cursor_key('B', modifiers, modes),
        KeyCode::Right => cursor_key('C', modifiers, modes),
        KeyCode::Left => cursor_key('D', modifiers, modes),
        KeyCode::Home => cursor_key('H', modifiers, modes),
        KeyCode::End => cursor_key('F', modifiers, modes),
        KeyCode::Insert => tilde_key(2, modifiers),
        KeyCode::Delete => tilde_key(3, modifiers),
        KeyCode::PageUp => tilde_key(5, modifiers),
        KeyCode::PageDown => tilde_key(6, modifiers),
        KeyCode::F(n @ 1..=4) => ss3_key(char::from(b'P' + n - 1), modifiers),
        KeyCode::F(n) => match n {
            5 => tilde_key(15, modifiers),
            6 => tilde_key(17, modifiers),
            7 => tilde_key(18, modifiers),
            8 => tilde_key(19, modifiers),
            9 => tilde_key(20, modifiers),
            10 => tilde_key(21, modifiers),
            11 => tilde_key(23, modifiers),
            12 => tilde_key(24, modifiers),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend(normalized.replace("\x1b[201~", "").into_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        normalized.into_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DetachState {
    prefix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawSplit {
    pub forward: Vec<u8>,
    pub detach: bool,
}

impl DetachState {
    pub fn feed(&mut self, bytes: &[u8]) -> RawSplit {
        let mut forward = Vec::with_capacity(bytes.len() + 1);
        if self.prefix {
            self.prefix = false;
            if bytes == [DETACH_SUFFIX] {
                return RawSplit {
                    forward,
                    detach: true,
                };
            }
            forward.push(DETACH_PREFIX);
        }
        if bytes == [DETACH_PREFIX] {
            self.prefix = true;
            return RawSplit {
                forward,
                detach: false,
            };
        }
        match bytes.iter().position(|byte| *byte == DETACH_BYTE) {
            Some(index) => {
                forward.extend_from_slice(&bytes[..index]);
                RawSplit {
                    forward,
                    detach: true,
                }
            }
            None => {
                forward.extend_from_slice(bytes);
                RawSplit {
                    forward,
                    detach: false,
                }
            }
        }
    }
}

pub const RESET_MODES: &[u8] = b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1l\x1b>\x1b[?2004l\x1b[0m\x1b[?25h\x1b[?1049l\r\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn plain(code: KeyCode) -> Vec<u8> {
        encode_key(&key(code, KeyModifiers::NONE), KeyModes::default())
    }

    #[test]
    fn printable_and_control_keys() {
        assert_eq!(plain(KeyCode::Char('é')), "é".as_bytes());
        assert_eq!(plain(KeyCode::Enter), b"\r");
        assert_eq!(plain(KeyCode::Backspace), [0x7f]);
        assert_eq!(plain(KeyCode::Esc), [0x1b]);
        assert_eq!(plain(KeyCode::BackTab), b"\x1b[Z");
        let ctrl = |ch| {
            encode_key(
                &key(KeyCode::Char(ch), KeyModifiers::CONTROL),
                KeyModes::default(),
            )
        };
        assert_eq!(ctrl('c'), [0x03]);
        assert_eq!(ctrl('a'), [0x01]);
        assert_eq!(ctrl(' '), [0x00]);
        assert_eq!(ctrl('['), [0x1b]);
        assert_eq!(ctrl(']'), [0x1d]);
        assert_eq!(
            encode_key(
                &key(KeyCode::Char('b'), KeyModifiers::ALT),
                KeyModes::default()
            ),
            b"\x1bb"
        );
        assert_eq!(
            encode_key(
                &key(
                    KeyCode::Char('x'),
                    KeyModifiers::ALT | KeyModifiers::CONTROL
                ),
                KeyModes::default()
            ),
            [0x1b, 0x18]
        );
    }

    #[test]
    fn cursor_keys_follow_application_mode_and_modifiers() {
        assert_eq!(plain(KeyCode::Up), b"\x1b[A");
        let app = KeyModes {
            application_cursor: true,
        };
        assert_eq!(
            encode_key(&key(KeyCode::Up, KeyModifiers::NONE), app),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Home, KeyModifiers::NONE), app),
            b"\x1bOH"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Right, KeyModifiers::CONTROL), app),
            b"\x1b[1;5C"
        );
        assert_eq!(
            encode_key(
                &key(KeyCode::Left, KeyModifiers::SHIFT | KeyModifiers::ALT),
                app
            ),
            b"\x1b[1;4D"
        );
        assert_eq!(plain(KeyCode::Delete), b"\x1b[3~");
        assert_eq!(
            encode_key(
                &key(KeyCode::PageUp, KeyModifiers::SHIFT),
                KeyModes::default()
            ),
            b"\x1b[5;2~"
        );
        assert_eq!(plain(KeyCode::F(1)), b"\x1bOP");
        assert_eq!(plain(KeyCode::F(5)), b"\x1b[15~");
        assert_eq!(plain(KeyCode::F(12)), b"\x1b[24~");
        assert_eq!(
            encode_key(
                &key(KeyCode::F(2), KeyModifiers::CONTROL),
                KeyModes::default()
            ),
            b"\x1b[1;5Q"
        );
    }

    #[test]
    fn paste_normalizes_newlines_and_brackets_when_requested() {
        assert_eq!(encode_paste("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(
            encode_paste("x\x1b[201~y", true),
            b"\x1b[200~xy\x1b[201~".to_vec()
        );
    }

    #[test]
    fn detach_chords_split_raw_input() {
        let mut state = DetachState::default();
        assert_eq!(
            state.feed(b"ls\r"),
            RawSplit {
                forward: b"ls\r".to_vec(),
                detach: false
            }
        );
        assert_eq!(
            state.feed(&[b'a', DETACH_BYTE, b'b']),
            RawSplit {
                forward: b"a".to_vec(),
                detach: true
            }
        );
        let mut state = DetachState::default();
        assert!(!state.feed(&[DETACH_PREFIX]).detach);
        assert!(state.feed(b"d").detach);
        let mut state = DetachState::default();
        assert_eq!(state.feed(&[DETACH_PREFIX]).forward, Vec::<u8>::new());
        assert_eq!(state.feed(b"x").forward, vec![DETACH_PREFIX, b'x']);
        assert!(is_detach_key(&key(
            KeyCode::Char('\\'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_detach_key(&key(
            KeyCode::Char('\\'),
            KeyModifiers::NONE
        )));
    }
}
