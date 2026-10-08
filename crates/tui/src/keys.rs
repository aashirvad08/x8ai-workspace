//! Keys and pastes as the bytes a terminal sends to the program in it.
//!
//! The terminal `x8ai` runs in reports keys as events (through crossterm); a
//! shell or editor in a pane expects the bytes xterm would send for them. This
//! is the usual xterm encoding, without the kitty keyboard protocol: the panes'
//! emulator does not offer it, so no program asks for it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const ESC: u8 = 0x1b;

/// The bytes for `key`, or `None` for a key that sends nothing (a release, a
/// media key). `app_cursor`: the program asked for application cursor keys
/// (DECCKM), as shells' line editors and full-screen programs do.
pub fn encode(key: &KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // xterm's modifier parameter: 1 plus Shift 1, Alt 2, Ctrl 4.
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let bytes = match key.code {
        KeyCode::Char(c) => {
            let mut out = Vec::with_capacity(5);
            if alt {
                out.push(ESC);
            }
            match control_byte(c).filter(|_| ctrl) {
                Some(byte) => out.push(byte),
                None => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            }
            out
        }
        KeyCode::Enter => with_alt(alt, b"\r"),
        KeyCode::Tab if shift => b"\x1b[Z".to_vec(),
        KeyCode::Tab => with_alt(alt, b"\t"),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => with_alt(alt, if ctrl { b"\x08" } else { b"\x7f" }),
        KeyCode::Esc => with_alt(alt, b"\x1b"),
        KeyCode::Up => cursor(b'A', modifier, app_cursor),
        KeyCode::Down => cursor(b'B', modifier, app_cursor),
        KeyCode::Right => cursor(b'C', modifier, app_cursor),
        KeyCode::Left => cursor(b'D', modifier, app_cursor),
        KeyCode::Home => cursor(b'H', modifier, app_cursor),
        KeyCode::End => cursor(b'F', modifier, app_cursor),
        KeyCode::Insert => tilde(2, modifier),
        KeyCode::Delete => tilde(3, modifier),
        KeyCode::PageUp => tilde(5, modifier),
        KeyCode::PageDown => tilde(6, modifier),
        KeyCode::F(n) => function(n, modifier)?,
        KeyCode::Null => vec![0],
        _ => return None,
    };
    Some(bytes)
}

/// Pasted text as the program should receive it. In bracketed paste mode it is
/// marked as a paste, so a shell does not run it line by line; an ESC in it is
/// dropped, since it could end the paste early and send what follows as typed
/// keys. Otherwise line breaks become Enter (`\r`), as typing them would.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend(
            text.chars()
                .filter(|&c| c != '\x1b')
                .collect::<String>()
                .bytes(),
        );
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// Ctrl with `c`: the control character a terminal sends, if there is one.
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        ' ' | '@' | '2' => Some(0),
        '[' | '3' => Some(ESC),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '/' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

fn with_alt(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if alt {
        out.push(ESC);
    }
    out.extend_from_slice(bytes);
    out
}

/// Arrows, Home and End: `ESC [ A`, or `ESC O A` in application mode, or
/// `ESC [ 1 ; m A` with modifiers.
fn cursor(last: u8, modifier: u8, app_cursor: bool) -> Vec<u8> {
    match (modifier, app_cursor) {
        (1, true) => vec![ESC, b'O', last],
        (1, false) => vec![ESC, b'[', last],
        _ => format!("\x1b[1;{modifier}{}", last as char).into_bytes(),
    }
}

/// `ESC [ n ~`, or `ESC [ n ; m ~` with modifiers.
fn tilde(n: u8, modifier: u8) -> Vec<u8> {
    if modifier == 1 {
        format!("\x1b[{n}~").into_bytes()
    } else {
        format!("\x1b[{n};{modifier}~").into_bytes()
    }
}

fn function(n: u8, modifier: u8) -> Option<Vec<u8>> {
    match n {
        1..=4 => {
            let last = b"PQRS"[usize::from(n - 1)];
            Some(if modifier == 1 {
                vec![ESC, b'O', last]
            } else {
                format!("\x1b[1;{modifier}{}", last as char).into_bytes()
            })
        }
        5..=12 => Some(tilde(
            [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)],
            modifier,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn bytes(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        encode(&key(code, modifiers), false).unwrap()
    }

    #[test]
    fn characters_are_sent_as_utf8() {
        assert_eq!(bytes(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(bytes(KeyCode::Char('A'), KeyModifiers::SHIFT), b"A");
        assert_eq!(
            bytes(KeyCode::Char('é'), KeyModifiers::NONE),
            "é".as_bytes()
        );
    }

    #[test]
    fn control_keys_are_control_characters() {
        assert_eq!(bytes(KeyCode::Char('c'), KeyModifiers::CONTROL), [0x03]);
        assert_eq!(bytes(KeyCode::Char('d'), KeyModifiers::CONTROL), [0x04]);
        assert_eq!(bytes(KeyCode::Char('z'), KeyModifiers::CONTROL), [0x1a]);
        assert_eq!(bytes(KeyCode::Char(' '), KeyModifiers::CONTROL), [0x00]);
        assert_eq!(bytes(KeyCode::Char('['), KeyModifiers::CONTROL), [0x1b]);
        assert_eq!(bytes(KeyCode::Char('_'), KeyModifiers::CONTROL), [0x1f]);
        // Ctrl with a key that has no control character sends the key.
        assert_eq!(bytes(KeyCode::Char('.'), KeyModifiers::CONTROL), b".");
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(bytes(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
        assert_eq!(
            bytes(
                KeyCode::Char('x'),
                KeyModifiers::ALT | KeyModifiers::CONTROL
            ),
            [0x1b, 0x18]
        );
        assert_eq!(bytes(KeyCode::Backspace, KeyModifiers::ALT), b"\x1b\x7f");
    }

    #[test]
    fn editing_keys() {
        assert_eq!(bytes(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(bytes(KeyCode::Tab, KeyModifiers::NONE), b"\t");
        assert_eq!(bytes(KeyCode::BackTab, KeyModifiers::SHIFT), b"\x1b[Z");
        assert_eq!(bytes(KeyCode::Backspace, KeyModifiers::NONE), b"\x7f");
        assert_eq!(bytes(KeyCode::Esc, KeyModifiers::NONE), b"\x1b");
        assert_eq!(bytes(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(bytes(KeyCode::PageUp, KeyModifiers::NONE), b"\x1b[5~");
        assert_eq!(bytes(KeyCode::PageDown, KeyModifiers::SHIFT), b"\x1b[6;2~");
    }

    #[test]
    fn arrows_follow_the_cursor_key_mode() {
        let up = key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(encode(&up, false).unwrap(), b"\x1b[A");
        assert_eq!(encode(&up, true).unwrap(), b"\x1bOA");
        // With a modifier, the mode does not matter.
        let ctrl_left = key(KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(encode(&ctrl_left, true).unwrap(), b"\x1b[1;5D");
        assert_eq!(
            bytes(KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT),
            b"\x1b[1;4C"
        );
        assert_eq!(bytes(KeyCode::Home, KeyModifiers::NONE), b"\x1b[H");
        assert_eq!(
            encode(&key(KeyCode::End, KeyModifiers::NONE), true).unwrap(),
            b"\x1bOF"
        );
    }

    #[test]
    fn function_keys() {
        assert_eq!(bytes(KeyCode::F(1), KeyModifiers::NONE), b"\x1bOP");
        assert_eq!(bytes(KeyCode::F(4), KeyModifiers::SHIFT), b"\x1b[1;2S");
        assert_eq!(bytes(KeyCode::F(5), KeyModifiers::NONE), b"\x1b[15~");
        assert_eq!(bytes(KeyCode::F(12), KeyModifiers::CONTROL), b"\x1b[24;5~");
        assert_eq!(
            encode(&key(KeyCode::F(13), KeyModifiers::NONE), false),
            None
        );
    }

    #[test]
    fn releases_send_nothing() {
        let mut release = key(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(encode(&release, false), None);
    }

    #[test]
    fn a_bracketed_paste_is_marked_and_cannot_end_itself() {
        assert_eq!(
            paste("ls\necho \x1b[201~rm -rf ~\n", true),
            b"\x1b[200~ls\necho [201~rm -rf ~\n\x1b[201~"
        );
    }

    #[test]
    fn a_plain_paste_turns_line_breaks_into_enter() {
        assert_eq!(paste("a\r\nb\nc", false), b"a\rb\rc");
    }
}
