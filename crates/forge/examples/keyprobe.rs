//! What does this terminal actually send?
//!
//! forge's bindings are unit-tested against synthetic `KeyEvent`s, which proves
//! the routing but says nothing about what crossterm reports on a given
//! terminal. Windows consoles vary: some deliver `Alt+Q` as one event with the
//! ALT modifier, some send `Esc` then `q` as two events, some swallow it
//! entirely before it ever reaches the process.
//!
//! Run this, press the key that is misbehaving, and read what comes out.
//!
//!   cargo run -p forge --example keyprobe
//!
//! Press Esc three times in a row to exit.

use std::io::Write;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

fn main() -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut out = std::io::stdout();

    // Raw mode means no implicit carriage returns, so every line needs \r\n.
    write!(
        out,
        "keyprobe - press keys to see how this terminal reports them.\r\n\
         Try the key that is not working, then compare with a plain letter.\r\n\
         Esc three times to exit.\r\n\r\n"
    )?;
    out.flush()?;

    let mut escapes = 0;

    loop {
        let ev = event::read()?;

        if let Event::Key(KeyEvent { code, kind, .. }) = ev {
            // Only count deliberate presses, or auto-repeat would exit for you.
            if code == KeyCode::Esc && kind != KeyEventKind::Release {
                escapes += 1;
                if escapes >= 3 {
                    break;
                }
            } else if kind != KeyEventKind::Release {
                escapes = 0;
            }
        }

        match ev {
            Event::Key(k) => {
                let mods = if k.modifiers.is_empty() {
                    "none".to_string()
                } else {
                    format!("{:?}", k.modifiers)
                };
                // The verdict line is what matters: forge's global bindings all
                // test `modifiers.contains(ALT)` against a `Char`.
                let verdict = match (k.code, k.modifiers.contains(KeyModifiers::ALT)) {
                    (KeyCode::Char(_), true) => "  <-- forge would see this as an Alt binding",
                    (KeyCode::Esc, _) => "  (a lone Esc - if Alt+key produces this plus a letter, \
                                          this terminal sends Alt as an Esc prefix)",
                    _ => "",
                };
                write!(
                    out,
                    "key   code={:<18} mods={:<28} kind={:?}{}\r\n",
                    format!("{:?}", k.code),
                    mods,
                    k.kind,
                    verdict
                )?;
            }
            Event::Paste(t) => write!(out, "paste {} bytes\r\n", t.len())?,
            Event::Resize(w, h) => write!(out, "resize {w}x{h}\r\n")?,
            other => write!(out, "other {other:?}\r\n")?,
        }
        out.flush()?;
    }

    disable_raw_mode()?;
    println!("\ndone");
    Ok(())
}
