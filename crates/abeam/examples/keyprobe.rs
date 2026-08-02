//! What does this terminal actually send?
//!
//! abeam's bindings are unit-tested against synthetic `KeyEvent`s, which proves
//! the routing and says nothing at all about what crossterm reports on a given
//! terminal. That is the other half of the keymap question, and no audit of an
//! agent's binary can answer it: `docs/keymap.md` can establish that Copilot
//! does nothing with `F4`, and still not establish that `F4` leaves this
//! console as `F4`. Windows consoles vary — some deliver `Alt+Q` as one event
//! with the ALT modifier, some send `Esc` then `q` as two events, some swallow
//! a combination entirely before the process ever sees it, and a window
//! manager or a terminal's own shortcut table can take an F-key off the wire.
//!
//! So this prints, for every event crossterm reports, the `KeyCode`, the
//! modifier set and the event kind — and then a verdict naming which of
//! abeam's sixteen bindings the event would resolve to, if any. The verdict is
//! the part to read. Comparing `code=F(4)` against `keys.rs` by eye is exactly
//! the step where a mis-wiring survives being looked at.
//!
//! The sixteen are `Alt`+`G E S Q Z J K`, `Alt+PageUp`/`Alt+PageDown`, bare
//! `F1`–`F5` and `F12`, and `Ctrl+\`. `docs/keymap.md` names this tool as the
//! second of the two steps that would upgrade the Copilot half of the audit:
//! run it here first to confirm all sixteen arrive as abeam expects, then host
//! `copilot` inside abeam and send each one through with literal-next to see
//! whether the agent reacts.
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
    (1, "the help overlay"),
    (2, "pty diagnostics"),
    (3, "the file reader's light / dark page"),
    (4, "focus left"),
    (5, "focus right"),
    (12, "literal-next, the Ctrl+\\ alias"),
];

/// What abeam would do with this event, in the words of its binding table.
///
/// The three shapes `keys::global` recognises are a `Char` or a page key
/// carrying ALT, a function key with **no** modifier at all, and `Ctrl+\`. A
/// modified F-key is deliberately not abeam's — the audit cleared the bare
/// keys only — so it is called out rather than left blank, since a terminal
/// that adds a stray SHIFT to an F-key produces a binding that silently stops
/// working.
fn verdict(k: &KeyEvent) -> String {
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let bare = k.modifiers.is_empty();

    match k.code {
        KeyCode::Char('\\') if ctrl => {
            "  <-- literal-next: abeam sends the NEXT key to the agent raw".to_string()
        }
        // The page keys are the two Alt bindings that are not letters, and Ink
        // reports Alt+PageUp as PageUp with `meta` set — so a console that
        // drops the ALT here hands the agent a page key instead of scrolling
        // the right pane.
        KeyCode::Char(_) | KeyCode::PageUp | KeyCode::PageDown if alt => {
            "  <-- abeam would see this as an Alt binding".to_string()
        }
        KeyCode::F(n) if bare => match F_KEYS.iter().find(|(f, _)| *f == n) {
            Some((_, what)) => format!("  <-- abeam binds bare F{n}: {what}"),
            None => format!("  (bare F{n}, which abeam does not bind: it goes to the agent)"),
        },
        KeyCode::F(n) => {
            format!("  (a *modified* F{n}: abeam claims only the bare F-keys, so this goes to the agent)")
        }
        KeyCode::Esc => "  (a lone Esc - if Alt+key produces this plus a letter, \
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
         To clear this terminal, walk all sixteen abeam binds: Alt+G E S Q Z J K,\r\n\
         Alt+PgUp, Alt+PgDn, F1 F2 F3 F4 F5 F12, and Ctrl+\\ - every one should\r\n\
         print a verdict, and the F-keys should name the right action.\r\n\
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
