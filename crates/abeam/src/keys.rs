//! The global binding table, isolated so collisions are auditable in one file.
//!
//! # The invariant
//!
//! **Nothing abeam intercepts is a key any hosted agent can act on.** "Any" is
//! doing the work: a binding is safe only if it is a no-op in *every* agent
//! abeam can host, so gaining an agent can retire a key that was safe while
//! there was only one. It has already retired one, and this file is where that
//! shows.
//!
//! *Intercept* is doing work too, and it means exactly one thing: what
//! [`global`] claims before anybody else is offered it. A key a **focused
//! pane** handles is not interception — the agent is not listening, because the
//! keystroke was never going to reach it whatever this file said. That is why
//! the git view can have `w`, and the file reader `/`, `n` and `N`, on keys
//! this table would never dare take. The boundary is not a convention: it is
//! held by `global` returning `None` for every bare printable key, and pinned
//! by `plain_typing_is_never_a_global`. Stated here once, so a pane-local key
//! does not have to re-derive its own exemption in a comment beside itself.
//!
//! Every binding below was checked against Claude Code's own keymap, read
//! out of its binary, and against GitHub Copilot CLI's, read from GitHub's
//! published tables and from Ink's source. See `docs/keymap.md` for both
//! inventories, and for how much weaker the second one is than the first.
//!
//! Weaker in one way that belongs here rather than only there, because this is
//! the file that claims the keys: `Alt+G`, `Alt+S`, `Alt+J`, `Alt+K`,
//! `Alt+PageUp` and `Alt+PageDown` each shadow a key Copilot binds in its bare
//! form somewhere, and Ink hands a handler the bare form together with a `meta`
//! flag the handler is free to ignore — so for those six the invariant is
//! unrefuted rather than verified, and only a strings audit of the Copilot
//! binary can settle it.
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
//!
//! # Why `Alt` stopped being enough
//!
//! All of that still holds, and none of it is enough on its own any more. Alt
//! was a good namespace because Claude leaves most of it alone — which is a
//! fact about Claude, not a fact about terminals, and a second agent is under no
//! obligation to agree. Copilot CLI does not agree: GitHub's own command
//! reference declares **`Alt+←` / `Alt+→` as "move the cursor by a word"** on
//! Windows and Linux, and has done since 0.0.400. Those were abeam's focus keys.
//! Inside abeam a Copilot user would have had no word-motion at all —
//! `Ctrl+B`/`Ctrl+F` move one character, `Ctrl+W` deletes a word backwards, and
//! nothing else *moves* by a word — so abeam gave them up.
//!
//! It gave them up for every agent, not for Copilot. A per-agent table was
//! considered and rejected: a key that means one thing in front of one agent and
//! another in front of the next is a key nobody can learn, and the F1 overlay
//! would have to be right about which agent it is describing. The invariant is
//! about what abeam *intercepts*, and abeam should intercept the same set
//! whoever is listening.
//!
//! The replacement is `F4`/`F5`, and it is an F-key rather than two more Alt
//! letters because F-keys are the only namespace abeam has that is
//! *structurally* safe in both agents rather than merely unclaimed in both:
//!
//! - Claude binds no function key in any context, in the 2.1.220 binary.
//! - Copilot CLI is an Ink application, and `useInput` — the hook an Ink app
//!   reads keys through — hands its handler a `Key` record with no field for a
//!   function key at all, while `f1`–`f12` sit in Ink's `nonAlphanumericKeys` so
//!   the `input` string is blanked as well. Every bare F-key therefore arrives
//!   as `("", all-flags-false)`, indistinguishable from every other F-key and
//!   from nothing having happened. An Ink app cannot bind one even if it wants
//!   to.
//!
//! That is a stronger claim than "we could not find it documented", which is
//! precisely the evidence that would have cleared `Alt+F` in Claude.

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
    /// Show the queue: work lined up for the agent. A workspace view like git
    /// and the reader, and reached the same way — it does *not* take focus,
    /// because the common case is glancing at what is still to come while the
    /// agent works and you keep typing at it.
    ShowQueue,
    FocusLeft,
    FocusRight,
    /// Scroll the right pane *without focusing it* — glancing at git or at the
    /// markdown the agent just wrote is a read, and a read should not cost a focus
    /// round-trip. Carries the bare key the pane would have seen had it been
    /// focused, so there is one scroll vocabulary rather than two.
    ScrollRight(KeyCode),
    ToggleZoom,
    ToggleHelp,
    /// Show the pty instrument, or put back whatever it displaced.
    ToggleDiag,
    /// Flip the file reader between its light and dark palettes.
    ///
    /// Global rather than a key the viewer handles, so it works from the left
    /// pane — the reader is a thing you *glance* at, and reaching for it with
    /// `Alt+E` first to change how it looks would defeat the point. It affects
    /// no other view; see `panes::viewer::theme`.
    ToggleReaderTheme,
    /// Send the *next* keystroke to the agent verbatim, bypassing everything
    /// here.
    ///
    /// The pressure-release valve. If a future release of either agent binds
    /// `Alt+G`, this still reaches it, so abeam can never permanently shadow
    /// anything. It is what made `Alt+←` survivable for as long as it did, and
    /// what a third agent's collisions will be met with on the day they are
    /// found rather than on the day they are fixed.
    LiteralNext,
}

/// Resolve a global binding, or `None` to let the focused pane have the key.
///
/// Release events must already have been filtered out by the caller.
pub fn global(key: &KeyEvent) -> Option<Action> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // The audit that cleared the F-keys cleared the *bare* F-keys. A modified
    // one is a key abeam knows nothing about, so it belongs to the agent — and
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
        // F3 for the same reason as F2: the letters a theme key would want
        // under Alt — `t` for theme, `l` for light, `d` for dark — are all
        // spoken for. `Alt+T` is Claude's outright, and `Alt+L`/`Alt+D` are
        // readline bindings its prompt editor handles without declaring them.
        // No F-key is bound by Claude in any context.
        KeyCode::F(3) if bare => Some(Action::ToggleReaderTheme),
        // Two direct keys rather than one toggle, for the reason the view keys
        // are two direct keys: a toggle needs you to know the current state
        // before you press it, which fails exactly when you are glancing rather
        // than looking. Focus is glanced at the same way a view is.
        //
        // These were `Alt+←`/`Alt+→` until abeam gained a second agent. GitHub
        // declares that pair as word-motion in Copilot CLI's command reference,
        // so it is the agent's key and not abeam's; the module doc has the
        // argument, and `the_agents_alt_bindings_are_left_alone` pins it.
        KeyCode::F(4) if bare => Some(Action::FocusLeft),
        KeyCode::F(5) if bare => Some(Action::FocusRight),

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
        // `a` for the agenda of work still to come. Held to the same standard
        // as `g`, `e` and `s`, and cleared the same way: `meta+a` and `alt+a`
        // are both absent from the 2.1.220 binary — zero matches, where the
        // undeclared readline bindings that caught `Alt+F` do appear as text —
        // and `a` is not in the classic readline meta set (`b f d l u c t r y
        // n p`) that Claude's prompt editor handles without declaring. It is a
        // letter rather than an F-key because it joins a set: `Alt+G`, `Alt+E`,
        // `Alt+S`, `Alt+A` are the four workspace views, and a fourth spelled
        // `F6` would be a key nobody groups with the other three.
        KeyCode::Char('a') | KeyCode::Char('A') => Some(Action::ShowQueue),

        KeyCode::Char('q') | KeyCode::Char('Q') => Some(Action::Quit),
        KeyCode::Char('z') | KeyCode::Char('Z') => Some(Action::ToggleZoom),
        // There is no arm here for `Alt+←` or `Alt+→`, and the gap is the
        // point: they moved focus until Copilot CLI turned out to declare them
        // as word-motion. They fall through to the agent now, like every other
        // key abeam does not claim, and focus is `F4`/`F5` above.
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
    ("Alt+A", "right pane: the queue of work for the agent"),
    ("F4 / F5", "move focus left / right"),
    ("Alt+J / Alt+K", "scroll right pane, without focusing it"),
    (
        "Alt+PgDn / Alt+PgUp",
        "page right pane, without focusing it",
    ),
    ("Alt+Z", "zoom: hide / show the right pane"),
    // "while a child is live", not "while the agent is running": `app::act`
    // quits outright only when the agent has exited *and* no shell is live, so
    // a dead agent with a shell still in the right pane asks twice as well.
    ("Alt+Q", "quit (press twice while a child is live)"),
    ("F1", "this help"),
    ("F2", "pty diagnostics, and back"),
    ("F3", "file reader: light / dark page"),
    ("Ctrl+\\ or F12", "send the next key to the agent verbatim"),
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
    (
        "Enter",
        "git: open the file · list: open · queue: do it now",
    ),
    // The right pane only. The agent's pty cannot be moved to another
    // directory — a live child's cwd is the child's — so this is the one place
    // the two halves of the window deliberately disagree about where they are.
    ("Enter (worktrees)", "point the right pane at that worktree"),
    ("t", "files: rendered markdown / its source"),
    (
        "/",
        "list: find a file under the root · document: search the page",
    ),
    ("n / N", "document: next / previous match, outside the box"),
    ("Backspace or -", "file list: up a directory"),
    ("r", "refresh · queue: clear what has finished"),
    // Not a fifth global view key: `Alt+W` is Claude's, and a fifth view
    // spelled `F6` would be a key nobody groups with the other three. Why a
    // bare letter is allowed at all is the *intercept* paragraph at the top of
    // this file, stated there once rather than re-argued beside every key that
    // relies on it.
    ("w", "git: the worktrees of this repository"),
    // The queue's own four. `space` is conspicuously not among them: it pages,
    // here as in every other pane, and arming moved to `a` rather than take a
    // key out of the shared vocabulary this table promises three rows above.
    // A key that pages in three panes and toggles a mode in the fourth is a
    // key nobody can learn.
    ("i", "queue: write a new item"),
    ("a", "queue: arm / disarm sending to the agent"),
    ("d", "queue: delete the selected item"),
    ("m", "queue: switch an item between send and dispatch"),
    // One row rather than a caveat on the three scroll rows above and the two
    // letter rows below, all five of which stop being true one keystroke after
    // `/`. Stating the rule once is this file's idiom and it is also the only
    // version a reader can hold: "in a box, letters are letters" covers the
    // keys that exist today and the ones added next.
    (
        "(in a / box)",
        "every letter is typed; arrows and Tab move, Esc leaves",
    ),
    (
        "Esc or q",
        "back to the agent (a shell and a / box keep both; worktrees keep Esc)",
    ),
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
    fn the_agents_alt_bindings_are_left_alone() {
        // Plural, since abeam gained a second agent: Alt is claimed by both of
        // them, in different places, and abeam has to clear both.

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

        // And the left and right arrows are Copilot's, which is the one place a
        // second agent cost abeam a key it already had. GitHub's command
        // reference declares them as "move the cursor by a word" on Windows and
        // Linux and has since 0.0.400, and inside abeam they are a Copilot
        // user's only way to move by a word at all. Reclaiming them for focus
        // would leave that user with none, so this is the assertion a future
        // edit has to argue with rather than a line it can quietly delete.
        for (code, arrow) in [(KeyCode::Left, '←'), (KeyCode::Right, '→')] {
            assert_eq!(
                global(&k(code, KeyModifiers::ALT)),
                None,
                "Alt+{arrow} is Copilot's word-motion; abeam's focus keys are F4 and F5"
            );
        }
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
        assert_eq!(
            global(&k(KeyCode::F(3), KeyModifiers::NONE)),
            Some(Action::ToggleReaderTheme)
        );
        assert_eq!(
            global(&k(KeyCode::F(4), KeyModifiers::NONE)),
            Some(Action::FocusLeft)
        );
        assert_eq!(
            global(&k(KeyCode::F(5), KeyModifiers::NONE)),
            Some(Action::FocusRight)
        );
    }

    #[test]
    fn a_modified_f_key_belongs_to_the_agent() {
        // Not "to Claude" any more: the bare F-keys are cleared in both agents,
        // and by different arguments — absent from Claude's binary, and beyond
        // what Ink's `useInput` can even describe to a Copilot handler. Neither
        // argument was ever made about a *modified* F-key, so those stay the
        // agent's, whichever agent it is.
        //
        // Ctrl+F12 is the one that shows what the cost of guessing would be:
        // swallowing it would arm literal-next with nothing on screen to say so,
        // and the *next* key would then be forwarded raw as well.
        for mods in [
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            for n in [1u8, 2, 3, 4, 5, 12] {
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
        for expected in [
            "Alt+G", "Alt+E", "Alt+S", "Alt+A", "Alt+Q", "Alt+Z", "F1", "F2", "F3", "F4",
            "F5",
        ] {
            assert!(
                listed.iter().any(|k| k.contains(expected)),
                "{expected} is bound but not in the F1 overlay"
            );
        }
    }
}
