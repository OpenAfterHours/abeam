//! What does this terminal actually send?
//!
//! ## Current binding model
//!
//! Abeam has five bare function-key globals: `F1`, `F4`, `F5`, `F7`, and
//! `F12`, plus `Ctrl+\\`. `F1` opens the command hub; its bare or Shift
//! mnemonic chooses the command. Alt and AltGr are not global bindings.
//!
//! abeam's bindings are unit-tested against synthetic `KeyEvent`s, which
//! proves routing but not what crossterm reports in a particular terminal.
//! This tool prints each reported event and the global binding it resolves to,
//! if any. It is especially useful for checking that F1 reaches the terminal
//! as a bare function key, and that AltGr text is not treated as a hub command.
//!
//! On Windows **AltGr is Ctrl+Alt**, so the right-hand Alt key on many layouts
//! arrives as `CONTROL | ALT`. abeam deliberately leaves Alt and AltGr
//! unclaimed for focused panes and hosted agents.
//!
//! Run this, press the key that is misbehaving, and read what comes out.
//!
//!   cargo run -p abeam --example keyprobe
//!
//! Press Esc three times in a row to exit.

use std::io::Write;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

/// The *bare* function keys abeam binds, and what each one does.
///
/// Copied from `crates/abeam/src/keys.rs` rather than imported, because this is
/// an example against the library's public surface and the table is not part of
/// it. Naming the action is the whole point of the arm that reads this: a
/// terminal that reports `F(4)` for the key labelled F5 is a fault the raw
/// dump shows and nobody notices.
const F_KEYS: &[(u8, &str)] = &[
    (1, "the command hub (press a mnemonic next)"),
    (4, "focus left"),
    (5, "focus right"),
    (7, "select rows of the right pane"),
    (12, "literal-next, the Ctrl+\\ alias"),
];


/// What abeam would do with this event, in the words of its binding table.
///
/// `keys::global` recognises bare function keys and `Ctrl+\\`; modified
/// function keys, Alt, and AltGr are deliberately unclaimed.
fn verdict(k: &KeyEvent) -> String {
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let bare = k.modifiers.is_empty();
    match k.code {
        // `!alt` and not just `ctrl`, because `\` lives behind AltGr on several
        // layouts and typing one must stay a backslash rather than arming
        // literal-next. `keys::global` says the same thing the same way.
        KeyCode::Char('\\') if ctrl && !alt => {
            "  <-- literal-next: abeam sends the NEXT key to the agent raw".to_string()
        }
        KeyCode::Char('\\') if ctrl && alt => {
            "  (AltGr+backslash: a backslash, and deliberately not literal-next - F12 is \r
              the alias to use on this layout)"
                .to_string()
        }
        KeyCode::Char(_) | KeyCode::PageUp | KeyCode::PageDown if alt => {
            "  (Alt / AltGr is not an abeam global; it goes to the focused pane or agent)"
                .to_string()
        }
        KeyCode::F(n) if bare => match F_KEYS.iter().find(|(f, _)| *f == n) {
            Some((_, what)) => format!("  <-- abeam binds bare F{n}: {what}"),
            None => format!("  (bare F{n}, which abeam does not bind: it goes to the agent)"),
        },
        KeyCode::F(n) => {
            format!(
                "  (a *modified* F{n}: abeam claims only the bare F-keys, so this goes to the agent)"
            )
        }
        KeyCode::Esc => "  (a lone Esc - if Alt+key produces this plus a letter, \r
                          this terminal sends Alt as an Esc prefix)"
            .to_string(),
        _ => String::new(),
    }
}

fn main() -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut out = std::io::stdout();

    // Raw mode means no implicit carriage returns, so every line needs \r\n.
    write!(
        out,
        "keyprobe - press keys to see how this terminal reports them.\r\n\
         Try the key that is not working, then compare with a plain letter.\r\n\
         Abeam binds bare F1, F4, F5, F7, F12 and Ctrl+\\.\r\n\
         F1 opens the command hub; then press G/E/B/S/W/P/A/D/T/Z/J/K/N/Q,\r\n\
         PgUp/PgDn, or ? in abeam itself. Alt and AltGr are left to the pane.\r\n\
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
                write!(
                    out,
                    "key   code={:<18} mods={:<28} kind={:?}{}\r\n",
                    format!("{:?}", k.code),
                    mods,
                    k.kind,
                    verdict(&k)
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
