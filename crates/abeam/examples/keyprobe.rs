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
//! The twenty are `Alt`+`G E S Q Z J K`, `Alt+PageUp`/`Alt+PageDown`, bare
//! `F1`–`F9` and `F12`, and `Ctrl+\`. `docs/keymap.md` names this tool as the
//! second of the two steps that would upgrade the Copilot half of the audit:
//! run it here first to confirm all twenty arrive as abeam expects, then host
//! `copilot` inside abeam and send each one through with literal-next to see
//! whether the agent reacts.
//!
//! There is a twenty-first key here that is not a global at all: `Alt+T`, which
//! turns the scratch pad over and is read by the pad itself. It is here because
//! it is the key that sent somebody to this tool — it used to work from the
//! left `Alt` key and not the right one — and because a pane-local binding is
//! exactly the kind a probe over `keys::global` alone would report as
//! unclaimed.
//!
//! ## Read the modifiers, not just the code
//!
//! On Windows **AltGr is Ctrl+Alt**, so the right-hand `Alt` key on a UK, Irish
//! or continental layout arrives here as `CONTROL | ALT` and the left one as
//! `ALT`. Both are abeam's `Alt` — `keys::alt_chord` — so the verdict below
//! says which key you pressed rather than treating one of them as a stranger.
//! If a binding works from one `Alt` key and not the other, that is the
//! difference to look for, and it is a bug in abeam rather than in the console.
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
    (6, "the ad-hoc ask"),
    (7, "select rows of the right pane"),
    (8, "the queue"),
    (9, "the scratch pad"),
    (12, "literal-next, the Ctrl+\\ alias"),
];

/// The `Alt` letters abeam binds as globals, and what each one does.
///
/// The letters only — `Alt+PageUp` and `Alt+PageDown` are the two bindings in
/// this namespace that are not letters, and [`verdict`] matches them beside
/// this table. Copied out of `keys.rs` for the reason [`F_KEYS`] is, and
/// naming the action for the same reason: `mods=CONTROL | ALT` against a table
/// in another file is exactly the comparison nobody makes correctly by eye.
const ALT_KEYS: &[(char, &str)] = &[
    ('g', "the git view"),
    ('e', "the file view"),
    ('s', "the shell, focused"),
    ('q', "quit"),
    ('z', "zoom: hide / show the right pane"),
    ('j', "scroll the right pane down, without focusing it"),
    ('k', "scroll the right pane up, without focusing it"),
];

/// What abeam would do with this event, in the words of its binding table.
///
/// The three shapes `keys::global` recognises are a `Char` or a page key
/// carrying ALT, a function key with **no** modifier at all, and `Ctrl+\`. A
/// modified F-key is deliberately not abeam's — the audit cleared the bare
/// keys only — so it is called out rather than left blank, since a terminal
/// that adds a stray SHIFT to an F-key produces a binding that silently stops
/// working.
///
/// Every verdict names *which* `Alt` key arrived, because that is the
/// difference this tool exists to make visible: Windows spells AltGr as
/// Ctrl+Alt, abeam counts both as `Alt`, and a binding that answers to one of
/// them and not the other is a bug on abeam's side of the wire.
fn verdict(k: &KeyEvent) -> String {
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let bare = k.modifiers.is_empty();
    // CONTROL beside ALT is how the right-hand Alt key arrives on every layout
    // where it is AltGr. Naming it is the point: "works from one Alt key and
    // not the other" is a sentence somebody can act on, and `mods=CONTROL |
    // ALT` on its own is not.
    let which = if alt && ctrl {
        " (right Alt / AltGr)"
    } else {
        " (left Alt)"
    };

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
        KeyCode::Char(c) if alt => {
            let lower = c.to_ascii_lowercase();
            match ALT_KEYS.iter().find(|(l, _)| *l == lower) {
                Some((_, what)) => format!("  <-- abeam binds Alt+{c}{which}: {what}"),
                // The one pane-local key in this tool, and the reason it is here:
                // a probe that only knew `keys::global` would report abeam's
                // most-reported dead key as unbound.
                None if lower == 't' => format!(
                    "  <-- Alt+T{which} is the scratch pad's own key, not a global: \r
                     it turns the pad over, and only while the pad has focus (F9)"
                ),
                None => {
                    format!("  (Alt+{c}{which}: abeam does not bind it, so it goes to the agent)")
                }
            }
        }
        // The page keys are the two Alt bindings that are not letters, and Ink
        // reports Alt+PageUp as PageUp with `meta` set — so a console that
        // drops the ALT here hands the agent a page key instead of scrolling
        // the right pane.
        KeyCode::PageUp | KeyCode::PageDown if alt => {
            format!("  <-- abeam binds this{which}: page the right pane, without focusing it")
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
         To clear this terminal, walk all twenty abeam binds: Alt+G E S Q Z J K,\r\n\
         Alt+PgUp, Alt+PgDn, F1-F9, F12, and Ctrl+\\ - every one should print\r\n\
         a verdict, and the F-keys should name the right action.\r\n\
         Walk them with the LEFT Alt key and again with the RIGHT one: on a UK,\r\n\
         Irish or continental layout the right one is AltGr, which Windows\r\n\
         reports as Ctrl+Alt, and both must reach the same binding. Alt+T is\r\n\
         the scratch pad's own key rather than a global; try it here too.\r\n\
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
