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
//! the git view can have `w`, and the file reader `t`, `r`, `/`, `f`, `n` and
//! `N`, on keys this table would never dare take. The list is kept complete
//! rather than illustrative, because the comments beside those keys point back
//! at this sentence by name instead of re-deriving the exemption. The boundary
//! is not a convention: it is
//! held by `global` returning `None` for every bare printable key, and pinned
//! by `plain_typing_is_never_a_global`. Stated here once, so a pane-local key
//! does not have to re-derive its own exemption in a comment beside itself.
//!
//! Every binding below was checked against Claude Code's own keymap, read
//! out of its binary, against GitHub Copilot CLI's, read from GitHub's
//! published tables and from Ink's source, and against Codex CLI's defaults.
//! See `docs/keymap.md` for all three inventories, and for the different
//! confidence each one earns.
//!
//! Weaker in one way that belongs here rather than only there, because this is
//! the file that claims the keys: `Alt+G`, `Alt+S`, `Alt+J`, `Alt+K`,
//! `Alt+PageUp` and `Alt+PageDown` each shadow a key Copilot binds in its bare
//! form somewhere, and Ink hands a handler the bare form together with a `meta`
//! flag the handler is free to ignore — so for those six the invariant is
//! unrefuted rather than verified, and only a strings audit of the Copilot
//! binary can settle it.
//!
//! Codex adds a different limit: its `/keymap` command can remap TUI actions,
//! including to function keys. This table is clear of Codex's **defaults**, not
//! of every user configuration. `Alt+A` is not clear even by that standard:
//! Codex uses it to open its agent-session overview, so abeam yielded it and
//! moved the queue to `F8`.
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
//! letters because F-keys were the only namespace abeam had that was
//! *structurally* safe in the two agents supported at the time rather than
//! merely unclaimed in both:
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
//!
//! Codex can see and remap F-keys, so that structural claim no longer extends
//! to every hosted agent. The shipped Codex binary exposes its default
//! `Alt+A` action and no `F8` action in its strings; that absence is weaker
//! evidence than the collision itself. `F8` is the least disruptive
//! replacement for the queue, but a user who remaps Codex to `F8` must use
//! literal-next or choose a non-colliding Codex binding.
//!
//! # What counts as `Alt`
//!
//! One question, one answer, in [`alt_chord`] — and it is a function rather
//! than a line inside [`global`] because the pad reads `Alt+T` for itself and
//! got a different answer. On Windows **AltGr is Ctrl+Alt**: the OS sets
//! `LEFT_CTRL_PRESSED` alongside `RIGHT_ALT_PRESSED`, and crossterm reports
//! the pair as `ALT | CONTROL`. So on every layout whose right-hand Alt key is
//! AltGr — every UK, Irish and continental European one — a test of the shape
//! `alt && !ctrl` is a test for *which Alt key you pressed*, and half a
//! keyboard fails it. `global` had always ignored CONTROL and the pad had
//! always excluded it, which is why `Alt+S` reached the shell from either key
//! and `Alt+T` turned the pad over from only one. `altgr_is_alt` and the pad's
//! own `the_chord_turns_the_pad_over_from_either_alt_key` pin both halves.
//!
//! Nothing is lost by ignoring CONTROL, and the reason is crossterm's rather
//! than this file's: when an AltGr combination *produces a character* the
//! reported `KeyCode` is that character — `€`, `@` — and not the letter under
//! the key, because `u_char` is non-zero and the layout fallback never runs.
//! A binding letter therefore only ever arrives here from a combination that
//! typed nothing. The mirror of that claim is what lets the panes take AltGr
//! text; see `crate::panes::pad`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// True for `Alt`+key however the terminal reports it, AltGr included.
///
/// The single definition of abeam's namespace, used by [`global`] and by every
/// pane that reads an `Alt` chord of its own. The module doc has the argument;
/// the short of it is that `CONTROL` alongside `ALT` is how Windows spells
/// AltGr, so excluding it excludes a keyboard rather than a key.
pub fn alt_chord(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
}

/// True when a key event is somebody **typing** rather than reaching for a
/// chord — the question every pane with a box in it has to answer.
///
/// The companion to [`alt_chord`], and it exists for the other half of the same
/// Windows fact. If AltGr is Ctrl+Alt, then a guard of the shape
/// `!ctrl && !alt` does not only reject chords: it rejects every character that
/// lives behind AltGr. `€` on a UK layout, `@` and `€` on a German one, `#`,
/// `~`, `|`, `\` and `}` on several — the characters somebody writing a note
/// about code reaches for most. All three of abeam's boxes — the pad, the ask
/// and the queue — dropped them silently.
///
/// `Ctrl` and `Alt` **together** are therefore text, and `Ctrl` or `Alt` alone
/// is not. What that gives up is `Ctrl+Alt`+letter as a chord, which nothing in
/// abeam binds and nothing hosted can hear: the three panes that ask this
/// question are abeam's own composers, with no child in them for a chord to be
/// aimed at. `Shift` is not part of the question at all — it is what made the
/// letter a capital.
///
/// A pane that also reads an `Alt` binding of its own must match it *before*
/// this, which is what `crate::panes::pad`'s `Alt+T` arm does. That ordering is
/// safe rather than lucky: crossterm reports an AltGr combination that produces
/// a character *as that character*, so `Alt+T` and a layout's AltGr text can
/// never be the same event.
pub fn is_text(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match (ctrl, alt) {
        // Bare, or with the Shift that made the letter a capital.
        (false, false) => true,
        // AltGr, as Windows spells it.
        (true, true) => true,
        // A chord: Ctrl+letter belongs to whatever is hosted, and Alt+letter is
        // either abeam's or the agent's, but neither is a character.
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    ShowGit,
    ShowViewer,
    /// Show the command view *and* focus it, because a command line you cannot
    /// type into is a picture of one. One of the two workspace views that move
    /// focus — [`Action::ShowPad`] is the other, and it is the other for this
    /// key's reason rather than for one of its own. ([`Action::ShowAsk`] and
    /// [`Action::ToggleSelect`] take focus too, and say so below; neither is a
    /// workspace view at all.) Pressed again while it already has focus, it
    /// hands focus back, so the round trip to run `git branch` is one key out
    /// and the same key home.
    ShowShell,
    /// Show the queue: work lined up for the agent. A workspace view like git
    /// and the reader, reached with `F8` — it does *not* take focus,
    /// because the common case is glancing at what is still to come while the
    /// agent works and you keep typing at it.
    ShowQueue,
    /// Show the scratch pad *and* focus it, on `F9`.
    ///
    /// It takes focus for [`Action::ShowShell`]'s reason rather than by
    /// analogy with it: the pad exists to be typed into, and a pad that needed
    /// a second key before it would accept a word is one nobody reaches for in
    /// the ten seconds the thought lasts. Pressed again from inside it hands
    /// focus back, so the round trip is `F9`, type, `F9` — the same shape as
    /// the shell's, because it is the same promise.
    ///
    /// A workspace view, unlike [`Action::ShowAsk`] and [`Action::ToggleDiag`]:
    /// it displaces nothing and puts nothing back, so `F2` and `Esc` return to
    /// it. See `crate::panes::RightView::Pad`.
    ShowPad,
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
    /// Show the ask with **nothing attached**, or put back whatever it
    /// displaced.
    ///
    /// The global half of `?`, and the two are deliberately not the same key.
    /// `?` is pane-local and means "about the file I am reading"; there is no
    /// file here and no pane to have pressed it from — the common case is
    /// mid-sentence at the agent, wondering something about the repository — so
    /// this one is reachable from everywhere and carries no context at all.
    ///
    /// **Showing it also detaches**, which is worth stating because it is the
    /// only way a file comes back *off* the composer. `?` attaches and the
    /// attachment survives until the question goes, so without this a reader who
    /// pressed `?` on a file and then thought better of it had no way to ask
    /// about anything else. Detaching is disclosed the moment it happens: the
    /// row above the composer is what goes away.
    ///
    /// It takes focus, unlike [`Action::ToggleDiag`] and like
    /// [`Action::ShowShell`], for that key's reason — a box you have to press a
    /// second key to type into is not a box you can ask a question in.
    ShowAsk,
    /// Select rows of the right pane, to copy them or to hand them to the
    /// agent. Pressed again, it puts the selection away.
    ///
    /// An F-key for a reason the three above it only half have. `F2`, `F3` and
    /// `F6` are F-keys because the `Alt` letters they wanted were taken; this
    /// one could not have been a letter under *any* namespace, because the pane
    /// it acts on is the one pane that takes every key it is given. `Alt+S`,
    /// type, `Alt+S` is the shell's whole round trip, and a selection key that
    /// only worked in the four read-only views would be missing from the view
    /// the feature exists for — you select what a command printed.
    ///
    /// It takes focus, like [`Action::ShowShell`] and [`Action::ShowAsk`] and
    /// for their reason: a caret you have to press a second key to move is not
    /// a caret. What it does *not* do is switch views. The rows it selects are
    /// the rows already on screen, so dragging another view in front of them
    /// would be selecting from somewhere nobody was looking.
    ToggleSelect,
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
    let alt = alt_chord(key);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // The audit that cleared the F-keys cleared the *bare* F-keys. A modified
    // one is a key abeam knows nothing about, so it belongs to the agent — and
    // swallowing Ctrl+F12 would arm literal-next invisibly, which then eats the
    // following keystroke as well. One press, two keys misrouted.
    let bare = key.modifiers.is_empty();

    match key.code {
        // Ctrl+\ is awkward on layouts that put backslash behind AltGr, so F12
        // is an alias. No F-key is bound by Claude, which is also why F1 works.
        //
        // `!alt` is the whole of what makes that sentence true rather than
        // merely sympathetic. On a German, Spanish or Italian layout `\` *is*
        // AltGr+key, and AltGr is Ctrl+Alt — so without this, typing a
        // backslash anywhere in abeam armed literal-next and the next keystroke
        // went to the agent raw. The key an F-key was aliased for was not
        // awkward on those layouts, it was unusable, and it took the character
        // with it. With `!alt` the combination falls through to the pane, where
        // `is_text` reads it as the backslash it is.
        KeyCode::Char('\\') if ctrl && !alt => Some(Action::LiteralNext),
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
        //
        // **`F4` has since acquired a second meaning, and it is the exception
        // the paragraph above has to admit rather than a counter-example
        // somebody will find later.** Pressed while the keys are already on the
        // left it moves along the hosted agents — see `crate::app`'s
        // `Action::FocusLeft` arm — which is a *cycle*, on a key whose whole
        // defence was that it is not one.
        //
        // What keeps the defence standing is that the two meanings are not the
        // same question. The first press answers "give me the keys", is a
        // direct key still, and is the only meaning in a session with one
        // agent — which is most of them. The second answers "the next one", and
        // by then the reader is not glancing: they have the keyboard, they are
        // looking at the pane it belongs to, and its border says `2/3`.
        //
        // The state to know before pressing is therefore on screen, which is
        // the condition the paragraph above actually imposes — and it is on
        // screen *because* of this key rather than as a coincidence, which is
        // why `crate::app::App::agent_tag` is not optional chrome. The case
        // that pays for it is zoom: with the right pane hidden `App::ui` holds
        // focus on the left, so every press cycles, including the one somebody
        // pressed meaning "back to the agent". Without the position in the
        // border that press would silently hand their next sentence to another
        // session.
        KeyCode::F(4) if bare => Some(Action::FocusLeft),
        KeyCode::F(5) if bare => Some(Action::FocusRight),
        // F6 for the ask, and an F-key rather than an `Alt` letter for a
        // stronger version of F2's and F3's reason. The letters this key would
        // want are gone twice over: `?` is not reachable under `Alt` on every
        // layout, `Alt+A` is Codex's, and the classic readline meta set that
        // caught `Alt+F` covers most of what is left. Copilot CLI is an Ink
        // application whose `useInput` cannot describe a function key to a
        // handler at all; Claude binds none, and Codex's defaults leave F6
        // alone. Codex can remap it, which is the custom-keymap limitation the
        // module documentation and docs/keymap.md disclose.
        //
        // It joins F2 rather than the view keys, and the grouping is real
        // rather than a leftover: `Diag` and `Ask` are the two views that
        // *displace* something and put it back, and neither is remembered as a
        // workspace view. The row further down this file arguing that a view
        // spelled `F6` would be a key nobody groups with the workspace ones
        // still stands — this is not one of them.
        KeyCode::F(6) if bare => Some(Action::ShowAsk),
        // F7 for the selection, and the argument is not "one more F-key was
        // free". It is the only namespace that *can* carry this: the key has to
        // work while the shell view has focus, and a shell with a live child in
        // it takes every key including `Esc` and `q`. A letter — bare, or under
        // the `Alt` the agents use — would either be swallowed by that child or
        // shadow a binding of the agent on the left. The default-keymap audits
        // in this file's header are what make this one safe, and they are about
        // the *agent*; what makes it safe in the shell is that abeam claims it
        // before any pane is offered it, which is what `global` means.
        KeyCode::F(7) if bare => Some(Action::ToggleSelect),
        // Codex owns the queue's former `Alt+A`: its default global keymap uses
        // that key to open the agent-session overview. No Codex `F8` action
        // surfaced in the shipped binary's strings. Unlike Claude and Copilot,
        // Codex can distinguish and remap function keys, so this is absence
        // evidence rather than a structural guarantee; docs/keymap.md records
        // the audited build and the custom-keymap limitation.
        KeyCode::F(8) if bare => Some(Action::ShowQueue),
        // Whether `F9` is clear of the three hosted agents is the audit
        // docs/keymap.md carries, recorded there beside the builds it was run
        // against rather than restated here.
        //
        // The other half of the argument is not about agents at all, and it is
        // why the pad is `F9` rather than the next number along: `F11` is
        // fullscreen in Windows Terminal and in most other emulators, and `F10`
        // activates the menu bar in several, so neither reliably reaches an
        // application at all. A key the terminal eats is worse than a key an
        // agent binds — literal-next can hand a key to a child, and nothing
        // abeam can do reaches past the emulator. That leaves `F9` as the only
        // clean slot, and it is a fact about terminals rather than one about
        // whatever is running in them.
        KeyCode::F(9) if bare => Some(Action::ShowPad),

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
    // First, because it is a fact about every row under it rather than a
    // binding of its own, and because somebody whose `Alt` key "does not work"
    // opens this overlay before they open an issue. Every `Alt` row below is
    // reachable from either `Alt` key: Windows spells AltGr as Ctrl+Alt, and
    // `alt_chord` counts both.
    ("Alt or AltGr", "either key works for every Alt row below"),
    ("Alt+G", "right pane: git"),
    ("Alt+E", "right pane: files (again for the file list)"),
    ("Alt+S", "right pane: a shell, focused (again to leave)"),
    ("F8", "right pane: the queue of work for the agent"),
    (
        "F9",
        "right pane: the scratch pad, focused (again to leave)",
    ),
    // The parenthetical is the whole of what a second agent costs this table.
    // `F4` has always meant "give the keys to the left" and a second press did
    // nothing at all, so "again" is a meaning added to a dead press rather than
    // a key taken from anybody — which is why there is no new row here and no
    // new audit under docs/keymap.md. One direction, because a modified F-key
    // is deliberately not abeam's; see `global` above on `Ctrl+F12`.
    ("F4 / F5", "move focus left / right (F4 again: next agent)"),
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
    // Next to F2 because it behaves like F2 — a view that displaces one and puts
    // it back — and phrased against `?` two dozen rows below, which is the same
    // pane reached the other way. "about nothing in particular" is the whole
    // difference between them, and it is the half a reader is looking for when
    // they are typing at the agent and have a question about the repository
    // rather than about a file.
    ("F6", "ask a second agent, about nothing in particular"),
    // Next to F6 rather than beside the view keys, because it is not a view: it
    // acts on whatever is already on screen. "rows" and not "text" is the
    // honest word — the selection is whole rows of the pane, which is the one
    // thing about it a reader has to know before they press it.
    //
    // The parenthetical is not a footnote about a second way in: dragging is
    // the *first* way, and this key is what a keyboard has instead. Saying so
    // here is what stops the overlay reading as though a mode had to be entered
    // before anything could be copied.
    (
        "F7",
        "select rows of the right pane, and focus it — or just drag, which copies on its own",
    ),
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
    // Beside `Enter` because they are the two keys on one row and the pair is
    // the whole of what a reader has to hold: one moves the right pane, the
    // other starts a child in the left column and moves neither. "another" is
    // load-bearing — the session already has one — and "there" is the half that
    // cannot be undone later, since a live child's working directory belongs to
    // the child.
    (
        "a (worktrees)",
        "start another agent there (F4 again reaches it)",
    ),
    // Not "rendered markdown", which is what this said and what it stopped
    // meaning: a `.py` or a `.rs` with documentation in it is rendered too, and
    // a reader looking for the key that put their `"""` back was reading a row
    // about a file type they did not have open. The two words are the two forms
    // the pane holds, in the order the key toggles them, and neither names a
    // language — which is what keeps this row true the next time the list of
    // languages grows.
    ("t", "files: the rendering / what was typed"),
    // The pad's own version of the row above, and it is in this table because
    // otherwise it is written down in exactly one place: the pad's opening
    // screen, which is drawn only while the pad is empty. Somebody who types a
    // single character into a fresh pad has just lost the only pointer there
    // was to the rendering, which is the shape of bug this overlay exists to
    // prevent.
    //
    // A chord where the reader's is a bare letter, because in the pad's edit
    // form `t` is a letter somebody is typing — the box rule further down,
    // arriving in a pane that is a box the whole way through. Both keys are on
    // one row because they are one question asked in the two forms, which is
    // also why the words are the reader's words: the two forms of a document
    // are the same two forms wherever they are met.
    //
    // **"when focused" is the only condition in this table that is about where
    // the key goes rather than what it does**, and it is here because the
    // failure is silent. `F9`, type, `F9` leaves the pad on screen with the
    // keys back at the agent, which is the state somebody is most likely to be
    // in when they think of turning it over — and `Alt+T` from there is not a
    // key that does nothing. It is Claude's `chat:thinkingToggle`, so the pad
    // sits there unchanged while a setting moves in the pane next door. The
    // reader's bare `t` cannot do this: `global` claims no bare letter, so an
    // unfocused `t` was always the agent's and never looked like the reader's.
    (
        "Alt+T",
        "pad, when focused: the rendering / what was typed (t, in the rendering)",
    ),
    // The three searches are three questions, and the rows name them as
    // questions: two keys that both ended "under the root" differed by five
    // characters read at a glance, which is not a difference anyone reads.
    (
        "/",
        "which file is called this · where is it on this page · results: retype",
    ),
    // The one net row this feature adds. It is a bare letter for the reason `t`
    // and `w` are: the *intercept* paragraph at the top of this file, which is
    // about what `global` claims before a focused pane is offered anything.
    (
        "f",
        "files: which files say this — reads every file under the root",
    ),
    ("n / N", "document: next / previous match, outside the box"),
    // A bare letter for the reason `f` and `w` are, which is the *intercept*
    // paragraph at the top of this file: `global` claims only `Ctrl+\`, the
    // F-keys and the Alt combinations, so a key a focused pane handles was
    // never going to reach the agent whatever this table said. Cited rather
    // than re-argued, the way those two rows do.
    //
    // "document" and not "files", for the reason the `?` row gives lower down:
    // this is a row about the reader's *modes*. The file list and the `f`
    // results each own every key while they are up, so `o` reaches nothing
    // there.
    //
    // "if it has any" is doing real work rather than hedging. The key declines
    // on a `.txt` and on a `.json` — see `panes::viewer::outline` — and this
    // table is the only place that can say so, because a title already carrying
    // the name, the form, the query and the position has no room to advertise a
    // key per document.
    //
    // Markdown used to be in that list of refusals, in its `t` form, and is
    // not any more: the source of a document is parsed by the same parser that
    // renders it, so `o` answers in both forms and the reader does not pay a
    // table of contents for pressing `t`. Which is why this row says "document"
    // and not "rendered document" — the word here has to be about the reader's
    // mode, and there is no form of a markdown file where the key is dead.
    (
        "o",
        "document: jump to a heading or a definition, if it has any",
    ),
    ("Backspace or -", "file list: up a directory"),
    ("r", "refresh · queue: clear what has finished (twice)"),
    // Not another global view key: `Alt+W` is Claude's, and one spelled `F6`
    // would be a key nobody groups with the workspace views. Why a bare letter
    // is allowed at all is the *intercept* paragraph at the top of this file,
    // stated there once rather than re-argued beside every key that relies on
    // it.
    ("w", "git: the worktrees of this repository"),
    // The second key that opens a view without being in the `Alt` table, and
    // it is pane-local for `w`'s reason rather than for a reason of its own:
    // the *intercept* paragraph at the top of this file. A question about the
    // file you are reading is asked from where you are reading it, so the key
    // is only ever delivered to a focused pane and no agent is listening for
    // it. `Esc` puts back whatever view it displaced, the way `F2` does.
    //
    // "document" and not "files", which is a row about the reader's *modes*
    // rather than about the reader. The file list and the `f` results each own
    // every key while they are up — a pane cannot hand the same key to two
    // vocabularies and hope — so `?` reaches nothing there, and this table must
    // not advertise a key two rows below `Alt+E` that `Alt+E` `Alt+E` turns
    // off. The document view and the git view are the whole of where it works.
    // "a second agent" and not "a second Claude": the pane can drive Copilot's
    // print mode too now, and a row promising Claude in a session hosting
    // Copilot would be advertising the wrong program. Which one it is is on the
    // pane's own opening screen, where there is room to say what each of them
    // can and cannot promise.
    (
        "?",
        "document, git: ask a second agent about the file on screen (F6: about nothing)",
    ),
    // The queue's own four. `space` is conspicuously not among them: it pages,
    // here as in every other pane, and arming moved to `a` rather than take a
    // key out of the shared vocabulary this table promises three rows above.
    // A key that pages in four panes and toggles a mode in the fifth is a
    // key nobody can learn.
    ("i", "queue: write a new item"),
    ("a", "queue: arm / disarm sending to the agent"),
    ("d", "queue: delete the selected item (twice)"),
    ("m", "queue: switch an item between send and dispatch"),
    // One row rather than a caveat on the three scroll rows above and the two
    // letter rows below, all five of which stop being true one keystroke after
    // `/`. Stating the rule once is this file's idiom and it is also the only
    // version a reader can hold: "in a box, letters are letters" covers the
    // keys that exist today and the ones added next.
    // The box rule, stated once — and now carrying the one key in the feature
    // that behaves unlike anything else in the program. The two filter boxes
    // narrow as you type and `Enter` opens what is chosen; `f`'s box does
    // nothing at all until `Enter`, because it reads every file under the root.
    // A box that appears not to work is exactly what needs saying here.
    (
        "(in a find box)",
        "every letter is typed; arrows and Tab move, Esc leaves; f's box runs on Enter",
    ),
    // The same rule, and the ask pane needs its own row rather than being
    // folded into the one above because the box there is never *shut*. A find
    // box is a state you leave; the ask's composer is the pane, so `j`, `k`,
    // `g`, `G`, `space`, `b`, `r` and `q` are letters for the whole of the time
    // you are in it and the three scroll rows near the top of this table are
    // simply untrue there. Naming exactly what does scroll is the honest
    // version: this table must not promise a key that types a letter.
    //
    // `Esc` is on this row rather than left to the last row of the table
    // because it is the one key here that does something *before* it does what
    // the last row says: it clears a draft, and only an already-empty composer
    // falls through to the way out. Stated here, beside the rule it belongs to,
    // rather than as a fourth clause on a row that is already a list.
    (
        "(in the ask)",
        "every letter is typed; arrows, PgUp/PgDn, Home/End, Ctrl+D/U scroll; Esc clears the draft",
    ),
    // The fourth statement of the box rule, and it is the one this table owed
    // the longest: the pad is a document somebody is *writing*, so the three
    // scroll rows near the top — `j`/`k`, `space`/`b`, `g`/`G` — are not merely
    // untrue here, they are the letters being typed. The `(in the ask)` row
    // above states the standard this row is held to: this table must not
    // promise a key that types a letter.
    //
    // It names `Ctrl+D`/`Ctrl+U` because they are the one place the pad is
    // *worse* than the ask rather than merely different. There they still
    // scroll, and a reader who learned that pair as the half page they are in
    // every other pane would find them doing nothing at all here — a dead key
    // is indistinguishable from a pane that has stopped listening, which is
    // the whole reason this row exists.
    //
    // "editing" and not "the pad", because the rendering takes every one of
    // them back: it is a read-only view like the others and the rows near the
    // top of this table are simply true there. `Alt+T` is the key between the
    // two, and it has its own row further up.
    (
        "(in the pad, editing)",
        "every letter is typed; arrows, Home/End move the caret; PgUp/PgDn page; Ctrl+D/U do nothing",
    ),
    // The third statement of the box rule, and the one that has to be loudest:
    // this mode swallows *every* key, over a pane that may have a live shell in
    // it. A reader who does not know that is a reader typing at a child that is
    // not listening. The scroll rows near the top of this table are true here —
    // the same keys, moving a caret rather than a view — which is why only what
    // is new to this mode is named.
    (
        "(selecting)",
        "the scroll keys move the caret; v anchors, y or Ctrl+C copies, Esc leaves",
    ),
    // Its own row because it is the one place in the program where a
    // `Ctrl`+letter is not the child's, and somebody who does not know that is
    // somebody whose `Ctrl+C` did not interrupt what they thought it would.
    // `global` still claims nothing — see the module doc — but the overlay has
    // to say what the key does where it does it.
    (
        "Ctrl+C (selecting)",
        "copies · to interrupt something instead, leave the selection first",
    ),
    (
        "Enter (selecting)",
        "put the selected rows in the agent's composer, unsent, and go back to it",
    ),
    // `Enter` has a row above for the three views where it opens something.
    // Here it does two things and neither is running a command: it sends the
    // question, or — with nothing typed — types the selected command at the
    // shell **without submitting it**, which is the whole promise of the
    // hand-off. `Tab` walks the offered commands, which is the same "next /
    // previous item" the row above already means.
    (
        "Enter (ask)",
        "send · with nothing typed, type the chosen command at the shell",
    ),
    // Its own row rather than a clause on the one above, because what it does
    // is not what a reader would guess from "clear": it ends the *session*, not
    // the rows showing it. That is the whole point — a conversation left open
    // is re-sent as context with every question, so the file you have finished
    // with goes on being paid for until the child holding it is gone.
    (
        "Ctrl+L (ask)",
        "end the conversation and start fresh — the child goes too",
    ),
    // The parenthetical is an exhaustive list of the panes that deviate, which
    // is the only thing that makes it worth having, so a view that deviates has
    // to be added to it on the day it lands. The ask is the fourth: `q` is a
    // letter there for the whole of the time the pane is up — it is never the
    // way out — and this row sits *after* the ask's own two rows, so leaving it
    // out left the last thing a reader saw contradicting the two above it.
    // `Esc` is the ask's only while there is a draft to clear, which the
    // `(in the ask)` row says rather than this one: this row would have had to
    // carry the condition as well as the key, and it is already a list.
    //
    // The pad is the fifth, added on the day it landed as this comment asks.
    // It keeps `q` for the ask's reason exactly — a letter is a letter in a
    // pane being typed into — and only in the form that is being typed into:
    // the rendering hands both keys back like any other read-only view. That
    // condition lives on the `(in the pad, editing)` row rather than here, for
    // the reason the ask's does. `Esc` is not in the pad's half of this list
    // because the pad declines it in both forms, which is this row working
    // rather than an exception to it.
    (
        "Esc or q",
        "back to the agent (a shell and a find box keep both; ask and pad keep q; worktrees keep Esc)",
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
        // Plural because Alt is claimed by all three agents in different
        // places, and abeam has to clear every one.

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

        // Codex's default global keymap uses Alt+A to open the agent-session
        // overview. This was abeam's queue key before Codex became a built-in;
        // the queue moved to F8 rather than shadowing a hosted agent action.
        for c in ['a', 'A'] {
            assert_eq!(
                global(&k(KeyCode::Char(c), KeyModifiers::ALT)),
                None,
                "Alt+{c} is Codex's open-agents binding; abeam's queue key is F8"
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
        assert_eq!(
            global(&k(KeyCode::F(6), KeyModifiers::NONE)),
            Some(Action::ShowAsk)
        );
        assert_eq!(
            global(&k(KeyCode::F(7), KeyModifiers::NONE)),
            Some(Action::ToggleSelect)
        );
        assert_eq!(
            global(&k(KeyCode::F(8), KeyModifiers::NONE)),
            Some(Action::ShowQueue)
        );
        assert_eq!(
            global(&k(KeyCode::F(9), KeyModifiers::NONE)),
            Some(Action::ShowPad)
        );
    }

    #[test]
    fn altgr_is_alt() {
        // On Windows AltGr *is* Ctrl+Alt — the OS sets `LEFT_CTRL_PRESSED`
        // beside `RIGHT_ALT_PRESSED` and crossterm reports the pair — so on
        // every layout whose right-hand Alt key is AltGr, an `Alt` binding that
        // looked at CONTROL would work from one half of the keyboard and not
        // the other.
        //
        // Written as "the same answer" rather than as a list of the bindings,
        // because a list is a thing a new binding can be left out of. Whatever
        // this table does with `Alt`+key it must do with `Ctrl+Alt`+key, and
        // that includes declining: the keys the agents own are still theirs
        // from the right-hand Alt key too.
        let mut resolved = 0;
        let alt = KeyModifiers::ALT;
        let altgr = KeyModifiers::ALT | KeyModifiers::CONTROL;
        for c in 'a'..='z' {
            let want = global(&k(KeyCode::Char(c), alt));
            resolved += usize::from(want.is_some());
            assert_eq!(
                global(&k(KeyCode::Char(c), altgr)),
                want,
                "AltGr+{c} must mean what Alt+{c} means"
            );
        }
        for code in [
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
        ] {
            let want = global(&k(code, alt));
            resolved += usize::from(want.is_some());
            assert_eq!(global(&k(code, altgr)), want, "AltGr+{code:?}");
        }
        // A loop that compared nothing to nothing would pass just as happily,
        // so the count is the part that keeps this test honest: seven letters
        // (`g e s q z j k`) and the two page keys.
        assert_eq!(resolved, 9, "the Alt bindings this test actually walked");

        // And the one key where AltGr must *not* mean Alt, which is the same
        // fact read the other way: on the layouts that put `\` behind AltGr the
        // character arrives as Ctrl+Alt+backslash, and literal-next has to
        // decline it or nobody on those layouts can type a backslash at all.
        // `F12` is the alias that is reachable everywhere.
        assert_eq!(
            global(&k(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
            Some(Action::LiteralNext)
        );
        assert_eq!(
            global(&k(KeyCode::Char('\\'), altgr)),
            None,
            "AltGr+backslash is a backslash, not literal-next"
        );
    }

    #[test]
    fn typing_and_chords_are_told_apart_the_same_way_everywhere() {
        // `is_text` is the pad's, the ask's and the queue's shared answer, and
        // the AltGr row is the one it exists for: all three used to spell it
        // `!ctrl && !alt`, which drops `€`, `@` and every other character that
        // lives behind that key.
        for mods in [
            KeyModifiers::NONE,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT | KeyModifiers::CONTROL,
            KeyModifiers::ALT | KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            assert!(is_text(&k(KeyCode::Char('a'), mods)), "{mods:?}");
        }
        for mods in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        ] {
            assert!(!is_text(&k(KeyCode::Char('a'), mods)), "{mods:?}");
        }
        // And the two questions are deliberately both true for AltGr, which is
        // what makes the order of a pane's arms load-bearing: `crate::panes::pad`
        // matches its `Alt+T` before its text arm, so `AltGr+T` turns the page
        // over rather than typing a `t`. Safe because crossterm reports an
        // AltGr combination that *produces* a character as that character —
        // `€`, not the `e` under the key — so the two can never collide.
        let altgr_t = k(
            KeyCode::Char('t'),
            KeyModifiers::ALT | KeyModifiers::CONTROL,
        );
        assert!(alt_chord(&altgr_t) && is_text(&altgr_t));
    }

    #[test]
    fn selecting_is_a_global_because_the_shell_takes_every_letter() {
        // The assertion the F7 arm rests on, written as the thing a future edit
        // has to argue with. A live shell claims `Esc`, `q` and every letter,
        // so a selection key that was pane-local would be missing from the one
        // view the feature exists for — and no `Alt` letter is available
        // either, because that namespace belongs to the agents.
        assert_eq!(global(&k(KeyCode::Char('v'), KeyModifiers::NONE)), None);
        assert_eq!(global(&k(KeyCode::Char('y'), KeyModifiers::NONE)), None);
        assert_eq!(global(&k(KeyCode::Char('v'), KeyModifiers::ALT)), None);
        assert_eq!(global(&k(KeyCode::Char('y'), KeyModifiers::ALT)), None);
    }

    #[test]
    fn the_ad_hoc_ask_is_an_f_key_because_alt_belongs_to_the_agents() {
        // The argument for F6 rather than a letter, written as the assertion it
        // rests on: `?` is the pane-local half and it cannot be the global one,
        // because a bare `?` typed at the agent is a `?`.
        assert_eq!(global(&k(KeyCode::Char('?'), KeyModifiers::NONE)), None);
        assert_eq!(global(&k(KeyCode::Char('?'), KeyModifiers::SHIFT)), None);
        // And `Alt+?` is not it either. It is a shifted key under `Alt`, which
        // is a shape neither agent's keymap has been audited against — and `Alt`
        // is the namespace all of them actually use. The default keymaps leave
        // F6 alone; Codex's remapping support is documented separately.
        assert_eq!(global(&k(KeyCode::Char('?'), KeyModifiers::ALT)), None);
        assert_eq!(
            global(&k(
                KeyCode::Char('?'),
                KeyModifiers::ALT | KeyModifiers::SHIFT
            )),
            None
        );
        // A modified F6 stays the agent's, exactly as every other F-key does:
        // the audits that cleared these cleared the *bare* ones.
        for mods in [
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
        ] {
            assert_eq!(global(&k(KeyCode::F(6), mods)), None);
        }
        // And the overlay says so, in a row that names the difference from `?`
        // rather than repeating it: the whole point of a second way in is that
        // it carries no file.
        let (key, said) = HELP
            .iter()
            .find(|(key, _)| *key == "F6")
            .expect("the key the overlay promises");
        assert_eq!(*key, "F6");
        assert!(said.contains("ask"), "got: {said}");
        assert!(said.contains("nothing"), "what it does not attach: {said}");
    }

    #[test]
    fn a_modified_f_key_belongs_to_the_agent() {
        // Not "to Claude" any more: the bare F-keys are cleared against every
        // agent's default keymap, by different arguments — absent from Claude's
        // binary, beyond what Ink's `useInput` can describe to a Copilot
        // handler, and unbound by default in Codex. None of those audits cleared
        // a *modified* F-key, so those stay the agent's, whichever agent it is.
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
            for n in [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 12] {
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
            "Alt+G", "Alt+E", "Alt+S", "Alt+Q", "Alt+Z", "F1", "F2", "F3", "F4", "F5", "F6", "F7",
            "F8", "F9",
        ] {
            assert!(
                listed.iter().any(|k| k.contains(expected)),
                "{expected} is bound but not in the F1 overlay"
            );
        }
    }
}
