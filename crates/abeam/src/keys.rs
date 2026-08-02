//! The global binding table, isolated so collisions are auditable in one file.
//!
//! # The invariant
//!
//! **Nothing abeam intercepts is a key Claude can act on.** Every binding below
//! was checked against Claude Code's own keymap and is a verified no-op there.
//! See `docs/keymap.md` for the inventory that check was made against.
//!
//! The short version of why the namespace is `Alt`:
//!
//! - Every `Ctrl`+letter is already bound in Claude, so no Ctrl binding — and
//!   no Ctrl *prefix* — is safe. `Ctrl+X` is itself a prefix inside Claude.
//! - `Ctrl+]` was the spike's detach key. It is `app:openArtifact` in Claude.
//!   It is retired here: quitting when the user meant to open an artifact is
//!   the worst possible failure for a binding nobody chose.
//! - Alt is only partly claimed: `v m p o t w b f d y`, Up, Down, Backspace.
//!   Everything else under Alt is discarded by Claude's prompt editor today.
//! - No F-key is bound by Claude in any context.
//!
//! `Alt+F` is *not* free — it is `nextWord`, hardcoded in the prompt editor and
//! absent from Claude's declared keybinding table. That is why the file view is
//! `Alt+E` for "explorer". An audit that reads only the documented keymap would
//! have shipped that collision.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    ShowGit,
    ShowViewer,
    /// Show the command view *and* focus it — the one binding that moves focus,
    /// because a command line you cannot type into is a picture of one. Pressed
    /// again while it already has focus, it hands focus back, so the round trip
    /// to run `git branch` is one key out and the same key home.
    ShowShell,
    FocusLeft,
    FocusRight,
    /// Scroll the right pane *without focusing it* — glancing at git or at the
    /// markdown Claude just wrote is a read, and a read should not cost a focus
    /// round-trip. Carries the bare key the pane would have seen had it been
    /// focused, so there is one scroll vocabulary rather than two.
    ScrollRight(KeyCode),
    ToggleZoom,
    ToggleHelp,
    /// Show the pty instrument, or put back whatever it displaced.
    ToggleDiag,
    /// Send the *next* keystroke to Claude verbatim, bypassing everything here.
    ///
    /// The pressure-release valve. If a future Claude release binds `Alt+G`,
    /// this still reaches it, so abeam can never permanently shadow anything.
    LiteralNext,
}

/// Resolve a global binding, or `None` to let the focused pane have the key.
///
/// Release events must already have been filtered out by the caller.
pub fn global(key: &KeyEvent) -> Option<Action> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // The audit that cleared the F-keys cleared the *bare* F-keys. A modified
    // one is a key abeam knows nothing about, so it belongs to Claude — and
    // swallowing Ctrl+F12 would arm literal-next invisibly, which then eats the
    // following keystroke as well. One press, two keys misrouted.
    let bare = key.modifiers.is_empty();

    match key.code {
        // Ctrl+\ is awkward on layouts that put backslash behind AltGr, so F12
        // is an alias. No F-key is bound by Claude, which is also why F1 works.
        KeyCode::Char('\\') if ctrl => Some(Action::LiteralNext),
        KeyCode::F(12) if bare => Some(Action::LiteralNext),
        KeyCode::F(1) if bare => Some(Action::ToggleHelp),
        // An F-key rather than another Alt letter, and not because Alt is
        // short of room. The audit that cleared `Alt+E` also found `Alt+F`
        // bound by a readline binding Claude does not declare anywhere, and
        // the classic readline set includes `Alt+U`, `Alt+L` and `Alt+C` —
        // exactly the letters a diagnostics key would want. F-keys are bound
        // by Claude in no context at all, which is already why F1 is safe.
        KeyCode::F(2) if bare => Some(Action::ToggleDiag),

        _ if !alt => None,

        // Two direct keys rather than one cycle key: a cycle needs you to know
        // the current state before you press it, which fails exactly when you
        // are glancing rather than looking.
        KeyCode::Char('g') | KeyCode::Char('G') => Some(Action::ShowGit),
        KeyCode::Char('e') | KeyCode::Char('E') => Some(Action::ShowViewer),
        // `s` for shell. Not in Claude's declared table, and not one of the
        // four letters its undeclared readline switch handles (`b f d y`) —
        // held to the same standard as `g` and `e`, and re-checked against the
        // installed binary when the command view landed. See docs/keymap.md.
        KeyCode::Char('s') | KeyCode::Char('S') => Some(Action::ShowShell),

        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        KeyCode::Char('z') | KeyCode::Char('Z') => Some(Action::ToggleZoom),

        KeyCode::Left => Some(Action::FocusLeft),
        KeyCode::Right => Some(Action::FocusRight),

        KeyCode::Char('k') | KeyCode::Char('K') => Some(Action::ScrollRight(KeyCode::Up)),
        KeyCode::Char('j') | KeyCode::Char('J') => Some(Action::ScrollRight(KeyCode::Down)),
        KeyCode::PageUp => Some(Action::ScrollRight(KeyCode::PageUp)),
        KeyCode::PageDown => Some(Action::ScrollRight(KeyCode::PageDown)),

        _ => None,
    }
}

/// Rendered by the F1 overlay. Kept next to the table so the two cannot drift.
pub const HELP: &[(&str, &str)] = &[
    ("Alt+G", "right pane: git"),
    ("Alt+E", "right pane: files (again for the file list)"),
    ("Alt+S", "right pane: a shell, focused (again to leave)"),
    ("Alt+Left / Alt+Right", "move focus"),
    ("Alt+J / Alt+K", "scroll right pane, without focusing it"),
    ("Alt+PgDn / Alt+PgUp", "page right pane, without focusing it"),
    ("Alt+Z", "zoom: hide / show the right pane"),
    ("Alt+Q", "quit (press twice while Claude is running)"),
    ("F1", "this help"),
    ("F2", "pty diagnostics, and back"),
    ("Ctrl+\\ or F12", "send the next key to Claude verbatim"),
    ("", ""),
    ("j / k, arrows", "right pane, when focused: scroll a line"),
    ("space / b, PgDn / PgUp", "scroll a page"),
    // Claimed by `crate::scroll` in every pane, and missing from this table
    // until the command view's scrollback made the omission load-bearing: a
    // pane reasoning about "the keys the overlay promises" was reading a
    // shorter list than the one the code implements.
    ("Ctrl+D / Ctrl+U", "scroll a half page"),
    ("g / G, Home / End", "jump to top / bottom"),
    ("Tab / Shift+Tab", "next / previous item"),
    ("Enter", "git: open the file · list: open · doc: reload"),
    ("t", "files: rendered markdown / its source"),
    ("/", "file list: find a file anywhere under the root"),
    ("Backspace or -", "file list: up a directory"),
    ("r", "refresh"),
    ("Esc or q", "back to Claude (the shell keeps them)"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_typing_is_never_a_global() {
        // Anything abeam claims here is a keystroke Claude never sees.
        for c in "abcdefghijklmnopqrstuvwxyz0123456789 /?.".chars() {
            assert_eq!(global(&k(KeyCode::Char(c), KeyModifiers::NONE)), None);
            assert_eq!(global(&k(KeyCode::Char(c), KeyModifiers::SHIFT)), None);
        }
        for code in [
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Esc,
            KeyCode::Backspace,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            assert_eq!(global(&k(code, KeyModifiers::NONE)), None);
        }
    }

    #[test]
    fn no_ctrl_letter_is_claimed() {
        // Every Ctrl+letter is bound inside Claude Code. Claiming one shadows
        // a real feature, which is exactly what Ctrl+] did in the spike.
        for c in 'a'..='z' {
            assert_eq!(
                global(&k(KeyCode::Char(c), KeyModifiers::CONTROL)),
                None,
                "Ctrl+{c} is bound in Claude Code and must stay unclaimed"
            );
        }
        assert_eq!(global(&k(KeyCode::Char(']'), KeyModifiers::CONTROL)), None);
    }

    #[test]
    fn claudes_alt_bindings_are_left_alone() {
        // `b f d y v m p o t w` and Alt+arrows-up/down are Claude's, several of
        // them undeclared readline bindings in its prompt editor.
        for c in "bfdyvmpotw".chars() {
            assert_eq!(
                global(&k(KeyCode::Char(c), KeyModifiers::ALT)),
                None,
                "Alt+{c} is Claude's"
            );
        }
        assert_eq!(global(&k(KeyCode::Up, KeyModifiers::ALT)), None);
        assert_eq!(global(&k(KeyCode::Down, KeyModifiers::ALT)), None);
        assert_eq!(global(&k(KeyCode::Backspace, KeyModifiers::ALT)), None);
    }

    #[test]
    fn the_abeam_namespace_resolves() {
        assert_eq!(
            global(&k(KeyCode::Char('g'), KeyModifiers::ALT)),
            Some(Action::ShowGit)
        );
        assert_eq!(
            global(&k(KeyCode::Char('e'), KeyModifiers::ALT)),
            Some(Action::ShowViewer)
        );
        assert_eq!(
            global(&k(KeyCode::Char('s'), KeyModifiers::ALT)),
            Some(Action::ShowShell)
        );
        assert_eq!(
            global(&k(KeyCode::Char('q'), KeyModifiers::ALT)),
            Some(Action::Quit)
        );
        assert_eq!(
            global(&k(KeyCode::Right, KeyModifiers::ALT)),
            Some(Action::FocusRight)
        );
        assert_eq!(
            global(&k(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
            Some(Action::LiteralNext)
        );
        assert_eq!(
            global(&k(KeyCode::F(1), KeyModifiers::NONE)),
            Some(Action::ToggleHelp)
        );
        assert_eq!(
            global(&k(KeyCode::F(2), KeyModifiers::NONE)),
            Some(Action::ToggleDiag)
        );
    }

    #[test]
    fn a_modified_f_key_belongs_to_claude() {
        // The keymap audit cleared the bare F-keys. Ctrl+F12 is not one of
        // them, and swallowing it would arm literal-next with nothing on screen
        // to say so — the *next* key would then be forwarded raw as well.
        for mods in [
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            for n in [1u8, 2, 12] {
                assert_eq!(
                    global(&k(KeyCode::F(n), mods)),
                    None,
                    "F{n} with {mods:?} is not abeam's"
                );
            }
        }
    }

    #[test]
    fn every_help_entry_names_a_binding_that_exists() {
        // The help table is a separate list from the match above, so the two
        // can drift. This catches a renamed key; nothing can catch a key that
        // was added and never documented.
        for (k, what) in HELP {
            assert_eq!(k.is_empty(), what.is_empty(), "half-empty help row: {k:?}");
        }
        let listed: Vec<&str> = HELP.iter().map(|(k, _)| *k).collect();
        for expected in ["Alt+G", "Alt+E", "Alt+S", "Alt+Q", "Alt+Z", "F1", "F2"] {
            assert!(
                listed.iter().any(|k| k.contains(expected)),
                "{expected} is bound but not in the F1 overlay"
            );
        }
    }
}
