//! Translation from crossterm input events into the byte sequences a terminal
//! application expects to read from its pty.
//!
//! This is the part that decides whether the hosted app *feels* right. Getting
//! it subtly wrong shows up as double-typed characters, dead arrow keys, or
//! escape sequences leaking into the prompt as literal text.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use tui_term::vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// The `xterm` modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
fn modifier_param(m: KeyModifiers) -> u8 {
    1 + u8::from(m.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(m.contains(KeyModifiers::ALT))
        + 4 * u8::from(m.contains(KeyModifiers::CONTROL))
}

fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c {
        ' ' | '@' => 0x00,
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' | '/' => 0x1f,
        '?' => 0x7f,
        _ => return None,
    })
}

/// Arrow/Home/End. These change shape when the app enables DECCKM
/// (application cursor mode) — `ESC O A` instead of `ESC [ A`.
fn cursor_key(final_byte: u8, app_cursor: bool, m: KeyModifiers) -> Vec<u8> {
    let p = modifier_param(m);
    if p != 1 {
        format!("\x1b[1;{p}{}", final_byte as char).into_bytes()
    } else if app_cursor {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde_key(n: u8, m: KeyModifiers) -> Vec<u8> {
    let p = modifier_param(m);
    if p != 1 {
        format!("\x1b[{n};{p}~").into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

fn function_key(n: u8, m: KeyModifiers) -> Option<Vec<u8>> {
    let p = modifier_param(m);
    let seq = match n {
        1..=4 => {
            let f = [b'P', b'Q', b'R', b'S'][(n - 1) as usize] as char;
            if p != 1 {
                format!("\x1b[1;{p}{f}")
            } else {
                format!("\x1bO{f}")
            }
        }
        5 => return Some(tilde_key(15, m)),
        6 => return Some(tilde_key(17, m)),
        7 => return Some(tilde_key(18, m)),
        8 => return Some(tilde_key(19, m)),
        9 => return Some(tilde_key(20, m)),
        10 => return Some(tilde_key(21, m)),
        11 => return Some(tilde_key(23, m)),
        12 => return Some(tilde_key(24, m)),
        _ => return None,
    };
    Some(seq.into_bytes())
}

/// Encode a key press for the pty. Returns `None` for keys with no meaningful
/// byte representation (modifier presses on their own, unsupported F-keys).
pub fn encode_key(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    // Windows delivers Press *and* Release for every key. Forwarding both
    // double-types everything. This single line is the most common reason a
    // hand-rolled Windows pty host feels broken.
    if key.kind == KeyEventKind::Release {
        return None;
    }

    let m = key.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);

    // `csi` marks sequences that already carry the modifier in their
    // parameters, so they must not also get an ESC prefix for Alt.
    let (mut bytes, csi) = match key.code {
        KeyCode::Char(c) => {
            let b = if ctrl {
                vec![ctrl_byte(c)?]
            } else {
                c.to_string().into_bytes()
            };
            (b, false)
        }
        KeyCode::Enter => (vec![b'\r'], false),
        KeyCode::Tab => (vec![b'\t'], false),
        KeyCode::BackTab => (b"\x1b[Z".to_vec(), true),
        KeyCode::Backspace => (vec![0x7f], false),
        KeyCode::Esc => (vec![0x1b], false),

        KeyCode::Up => (cursor_key(b'A', app_cursor, m), true),
        KeyCode::Down => (cursor_key(b'B', app_cursor, m), true),
        KeyCode::Right => (cursor_key(b'C', app_cursor, m), true),
        KeyCode::Left => (cursor_key(b'D', app_cursor, m), true),
        KeyCode::Home => (cursor_key(b'H', app_cursor, m), true),
        KeyCode::End => (cursor_key(b'F', app_cursor, m), true),

        KeyCode::Insert => (tilde_key(2, m), true),
        KeyCode::Delete => (tilde_key(3, m), true),
        KeyCode::PageUp => (tilde_key(5, m), true),
        KeyCode::PageDown => (tilde_key(6, m), true),

        KeyCode::F(n) => (function_key(n, m)?, true),
        _ => return None,
    };

    if alt && !csi {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// Wrap pasted text so the app can tell it apart from typing.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

/// ConPTY opens a session by asking the host terminal where the cursor is —
/// Device Status Report, `ESC [ 6 n` — and **blocks until it gets an answer**.
/// Miss this and the hosted process never even starts running its command; you
/// see four bytes of output and a hang that looks like a dead pty.
///
/// The query can land split across two reads, so carry the trailing bytes over.
#[derive(Default)]
pub struct DsrScanner {
    tail: Vec<u8>,
}

const DSR_QUERY: &[u8] = b"\x1b[6n";

impl DsrScanner {
    /// Number of DSR queries in this chunk, counting one that began in the
    /// previous chunk. Answer each with [`dsr_reply`].
    pub fn scan(&mut self, chunk: &[u8]) -> usize {
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(chunk);

        let count = buf.windows(DSR_QUERY.len()).filter(|w| *w == DSR_QUERY).count();

        // Keep one byte less than the query length: enough to catch a split
        // sequence, too little to re-match one we have already counted.
        let keep = buf.len().min(DSR_QUERY.len() - 1);
        self.tail = buf[buf.len() - keep..].to_vec();
        count
    }
}

/// The answer ConPTY is waiting for, from 0-based cursor coordinates.
pub fn dsr_reply(row: u16, col: u16) -> Vec<u8> {
    format!("\x1b[{};{}R", row + 1, col + 1).into_bytes()
}

fn button_num(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// Encode a mouse event, given coordinates already made pane-relative and
/// 0-based. Returns `None` when the hosted app has not asked for this class of
/// event — sending unrequested mouse reports dumps garbage into its input.
pub fn encode_mouse(
    ev: &MouseEvent,
    col: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }

    let (base, pressed) = match ev.kind {
        MouseEventKind::Down(b) => (button_num(b), true),
        MouseEventKind::Up(b) => (button_num(b), false),
        MouseEventKind::Drag(b) => (button_num(b) + 32, true),
        MouseEventKind::Moved => (35, true),
        MouseEventKind::ScrollUp => (64, true),
        MouseEventKind::ScrollDown => (65, true),
        MouseEventKind::ScrollLeft => (66, true),
        MouseEventKind::ScrollRight => (67, true),
    };

    // Honour what the app actually enabled.
    let motion = matches!(ev.kind, MouseEventKind::Drag(_) | MouseEventKind::Moved);
    match mode {
        MouseProtocolMode::None => return None,
        MouseProtocolMode::Press => {
            if !pressed || motion {
                return None;
            }
        }
        MouseProtocolMode::PressRelease => {
            if motion {
                return None;
            }
        }
        MouseProtocolMode::ButtonMotion => {
            if matches!(ev.kind, MouseEventKind::Moved) {
                return None;
            }
        }
        MouseProtocolMode::AnyMotion => {}
    }

    let m = ev.modifiers;
    let btn = base
        + 4 * u8::from(m.contains(KeyModifiers::SHIFT))
        + 8 * u8::from(m.contains(KeyModifiers::ALT))
        + 16 * u8::from(m.contains(KeyModifiers::CONTROL));

    let (col, row) = (col + 1, row + 1);
    let bytes = match encoding {
        MouseProtocolEncoding::Sgr => {
            format!("\x1b[<{btn};{col};{row}{}", if pressed { 'M' } else { 'm' }).into_bytes()
        }
        _ => {
            // X10: no release button identity, and coordinates cap at 223.
            if col > 223 || row > 223 {
                return None;
            }
            let b = if pressed { btn } else { 3 };
            vec![0x1b, b'[', b'M', 32 + b, 32 + col as u8, 32 + row as u8]
        }
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn enc(code: KeyCode, mods: KeyModifiers) -> Vec<u8> {
        encode_key(key(code, mods), false).expect("expected an encoding")
    }

    #[test]
    fn plain_characters_pass_through() {
        assert_eq!(enc(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        // Multi-byte UTF-8 must not be truncated to one byte.
        assert_eq!(enc(KeyCode::Char('é'), KeyModifiers::NONE), "é".as_bytes());
    }

    #[test]
    fn ctrl_maps_to_control_codes() {
        assert_eq!(enc(KeyCode::Char('c'), KeyModifiers::CONTROL), vec![3]);
        assert_eq!(enc(KeyCode::Char('a'), KeyModifiers::CONTROL), vec![1]);
        assert_eq!(enc(KeyCode::Char(' '), KeyModifiers::CONTROL), vec![0]);
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(enc(KeyCode::Char('b'), KeyModifiers::ALT), vec![0x1b, b'b']);
    }

    #[test]
    fn arrows_respect_application_cursor_mode() {
        let up = key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(encode_key(up, false).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(up, true).unwrap(), b"\x1bOA");
    }

    #[test]
    fn modified_arrows_use_csi_form_even_in_app_cursor_mode() {
        // With a modifier the sequence must carry the parameter, so the
        // ESC O form is not an option regardless of DECCKM.
        let shift_up = key(KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(encode_key(shift_up, true).unwrap(), b"\x1b[1;2A");

        let ctrl_right = key(KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(encode_key(ctrl_right, false).unwrap(), b"\x1b[1;5C");
    }

    #[test]
    fn alt_does_not_double_encode_csi_sequences() {
        // Alt+Up is ESC [ 1;3 A - not an extra ESC in front of it.
        let alt_up = key(KeyCode::Up, KeyModifiers::ALT);
        assert_eq!(encode_key(alt_up, false).unwrap(), b"\x1b[1;3A");
    }

    #[test]
    fn release_events_are_dropped() {
        // The Windows double-typing bug. If this regresses, every keystroke
        // reaches the hosted app twice.
        let ev = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(encode_key(ev, false), None);

        let ev = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        );
        assert_eq!(encode_key(ev, false), Some(b"a".to_vec()));
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(enc(KeyCode::Backspace, KeyModifiers::NONE), vec![0x7f]);
        assert_eq!(enc(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(enc(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(enc(KeyCode::F(1), KeyModifiers::NONE), b"\x1bOP");
        assert_eq!(enc(KeyCode::F(5), KeyModifiers::NONE), b"\x1b[15~");
        assert_eq!(enc(KeyCode::F(12), KeyModifiers::NONE), b"\x1b[24~");
    }

    #[test]
    fn paste_is_bracketed_only_when_requested() {
        assert_eq!(encode_paste("hi", true), b"\x1b[200~hi\x1b[201~");
        assert_eq!(encode_paste("hi", false), b"hi");
    }

    #[test]
    fn dsr_query_is_detected() {
        let mut s = DsrScanner::default();
        assert_eq!(s.scan(b"hello"), 0);
        assert_eq!(s.scan(b"\x1b[6n"), 1);
        assert_eq!(s.scan(b"pre\x1b[6npost"), 1);
        assert_eq!(s.scan(b"\x1b[6n\x1b[6n"), 2);
    }

    #[test]
    fn dsr_query_split_across_reads_is_still_caught() {
        // ConPTY emits this as the very first thing it writes, so a small read
        // buffer can easily cut it in half. Missing it hangs the session.
        let mut s = DsrScanner::default();
        assert_eq!(s.scan(b"\x1b["), 0);
        assert_eq!(s.scan(b"6n"), 1);
    }

    #[test]
    fn dsr_query_is_not_counted_twice() {
        let mut s = DsrScanner::default();
        assert_eq!(s.scan(b"\x1b[6n"), 1);
        assert_eq!(s.scan(b"more output"), 0);
    }

    #[test]
    fn dsr_reply_is_one_based() {
        assert_eq!(dsr_reply(0, 0), b"\x1b[1;1R");
        assert_eq!(dsr_reply(4, 9), b"\x1b[5;10R");
    }

    fn mouse(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn mouse_is_silent_unless_the_app_asks() {
        let ev = mouse(MouseEventKind::ScrollUp);
        assert_eq!(
            encode_mouse(
                &ev,
                3,
                4,
                MouseProtocolMode::None,
                MouseProtocolEncoding::Sgr
            ),
            None,
            "unrequested mouse reports leak into the hosted app's input"
        );
    }

    #[test]
    fn mouse_sgr_encoding_is_one_based() {
        let ev = mouse(MouseEventKind::ScrollUp);
        let out = encode_mouse(
            &ev,
            3,
            4,
            MouseProtocolMode::AnyMotion,
            MouseProtocolEncoding::Sgr,
        )
        .unwrap();
        assert_eq!(out, b"\x1b[<64;4;5M");
    }

    #[test]
    fn motion_suppressed_in_press_release_mode() {
        let drag = mouse(MouseEventKind::Drag(MouseButton::Left));
        assert_eq!(
            encode_mouse(
                &drag,
                1,
                1,
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Sgr
            ),
            None
        );
        assert!(encode_mouse(
            &drag,
            1,
            1,
            MouseProtocolMode::ButtonMotion,
            MouseProtocolEncoding::Sgr
        )
        .is_some());
    }
}
