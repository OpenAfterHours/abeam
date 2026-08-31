//! The global binding table, isolated so collisions are auditable in one file.
//!
//! ## Current command model
//!
//! Bare `F1` opens the command hub. Its bare or Shift mnemonic selects an
//! application command; `F4`, `F5`, `F7`, `F12`, and `Ctrl+\\` are the only
//! other globals. Alt and AltGr are deliberately unclaimed so Windows AltGr
//! text and agent shortcuts always reach the focused pane.
//!
//! ## Historical audit context
//!
//! The remaining audit narrative records the retired Alt-first design and why
//! it was replaced. It is not a description of the current bindings; consult
//! [`HUB`], [`HELP`], and [`global`] for those.
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
    /// Open the F1 command hub. This is deliberately a distinct action from
    /// the reference inside that hub: F1 is the reliable way *into* abeam's
    /// command namespace, while `F1, ?` is the exhaustive reference once the
    /// user has deliberately entered it.
    OpenHub,
    Quit,
    ShowGit,
    ShowViewer,
    /// Show the file browser and focus it. Unlike [`ShowViewer`](Self::ShowViewer),
    /// this is an explicit interactive view command rather than a repeated-key
    /// toggle on the reader command.
    ShowBrowser,
    /// Show the command view *and* focus it, because a command line you cannot
    /// type into is a picture of one. One of the two workspace views that move
    /// focus — [`Action::ShowPad`] is the other, and it is the other for this
    /// key's reason rather than for one of its own. ([`Action::ShowAsk`] and
    /// [`Action::ToggleSelect`] take focus too, and say so below; neither is a
    /// workspace view at all.) Repeating a hub command does not toggle focus;
    /// `F4` returns it to the agent.
    ShowShell,
    /// Create a fresh shell, select it, and focus the shell view.
    CreateShell,
    /// Select the previous shell in the current workspace.
    PreviousShell,
    /// Select the next shell in the current workspace.
    NextShell,
    /// Close the selected shell after application-level confirmation.
    CloseShell,
    /// Show the queue without moving focus. Selected by F1, W.
    ShowQueue,
    /// Show and focus the scratch pad. Selected by F1, P; repeat presses
    /// keep focus there, while F4 returns it to the agent.
    ShowPad,
    FocusLeft,
    FocusRight,
    /// Move to the next hosted agent and give it the keyboard. This is a hub
    /// command rather than F4's second meaning: focus and agent navigation are
    /// separate questions and no direct key answers both depending on state.
    NextAgent,
    /// Scroll the right pane *without focusing it* — glancing at git or at the
    /// markdown the agent just wrote is a read, and a read should not cost a focus
    /// round-trip. Carries the bare key the pane would have seen had it been
    /// focused, so there is one scroll vocabulary rather than two.
    ScrollRight(KeyCode),
    ToggleZoom,
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
    /// Select rows of the right pane, focusing it when selection begins.
    ToggleSelect,
    /// Flip the file reader between its light and dark palettes.
    ToggleReaderTheme,
    /// Send the next keystroke to the agent verbatim, bypassing every binding.
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

    // This return is intentionally the whole global key map. Application
    // commands now live behind F1; Alt and the retired F-key aliases must fall
    // through to the focused agent or pane.
    match key.code {
        KeyCode::Char('\\') if ctrl && !alt => Some(Action::LiteralNext),
        KeyCode::F(12) if bare => Some(Action::LiteralNext),
        KeyCode::F(1) if bare => Some(Action::OpenHub),
        KeyCode::F(4) if bare => Some(Action::FocusLeft),
        KeyCode::F(5) if bare => Some(Action::FocusRight),
        KeyCode::F(7) if bare => Some(Action::ToggleSelect),
        _ => None,
    }
}

/// An input accepted by the F1 command hub. The second key is deliberately
/// interpreted only after F1, which leaves normal typing and every Alt/AltGr
/// combination to the focused child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubCommand {
    /// Reopen the concise command hub from the exhaustive reference.
    Open,
    /// Show the exhaustive reference in the hub.
    Reference,
    /// Run the selected application action.
    Action(Action),
}

/// Resolve an F1-hub key. Only bare and Shift variants are accepted: modified
/// events remain in the hub and do not execute, leak to a child, or turn an
/// AltGr character into an application command.
pub fn hub(key: &KeyEvent) -> Option<HubCommand> {
    if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
        return None;
    }

    Some(match key.code {
        KeyCode::F(1) => HubCommand::Open,
        KeyCode::Char('?') => HubCommand::Reference,
        KeyCode::Char('g') | KeyCode::Char('G') => HubCommand::Action(Action::ShowGit),
        KeyCode::Char('e') | KeyCode::Char('E') => HubCommand::Action(Action::ShowViewer),
        KeyCode::Char('b') | KeyCode::Char('B') => HubCommand::Action(Action::ShowBrowser),
        KeyCode::Char('s') | KeyCode::Char('S') => HubCommand::Action(Action::ShowShell),
        KeyCode::Char('c') | KeyCode::Char('C') => HubCommand::Action(Action::CreateShell),
        KeyCode::Left => HubCommand::Action(Action::PreviousShell),
        KeyCode::Right => HubCommand::Action(Action::NextShell),
        KeyCode::Char('x') | KeyCode::Char('X') => HubCommand::Action(Action::CloseShell),
        KeyCode::Char('w') | KeyCode::Char('W') => HubCommand::Action(Action::ShowQueue),
        KeyCode::Char('p') | KeyCode::Char('P') => HubCommand::Action(Action::ShowPad),
        KeyCode::Char('a') | KeyCode::Char('A') => HubCommand::Action(Action::ShowAsk),
        KeyCode::Char('d') | KeyCode::Char('D') => HubCommand::Action(Action::ToggleDiag),
        KeyCode::Char('t') | KeyCode::Char('T') => HubCommand::Action(Action::ToggleReaderTheme),
        KeyCode::Char('z') | KeyCode::Char('Z') => HubCommand::Action(Action::ToggleZoom),
        KeyCode::Char('j') | KeyCode::Char('J') => {
            HubCommand::Action(Action::ScrollRight(KeyCode::Down))
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            HubCommand::Action(Action::ScrollRight(KeyCode::Up))
        }
        KeyCode::PageUp => HubCommand::Action(Action::ScrollRight(KeyCode::PageUp)),
        KeyCode::PageDown => HubCommand::Action(Action::ScrollRight(KeyCode::PageDown)),
        KeyCode::Char('n') | KeyCode::Char('N') => HubCommand::Action(Action::NextAgent),
        KeyCode::Char('q') | KeyCode::Char('Q') => HubCommand::Action(Action::Quit),
        _ => return None,
    })
}

/// The concise command hub, rendered as soon as F1 is pressed.
pub const HUB: &[(&str, &str)] = &[
    ("G", "git (keeps current focus)"),
    ("E", "files / reader (keeps current focus)"),
    ("B", "file browser (opens and focuses)"),
    ("S", "active shell (starts one if needed; focuses)"),
    ("C", "new shell (starts fresh and focuses)"),
    ("← / →", "previous / next shell (wraps)"),
    ("X", "close active shell (repeat F1, X to confirm)"),
    ("W", "work queue (keeps current focus)"),
    ("P", "scratch pad (opens and focuses)"),
    ("A", "ask (opens and focuses)"),
    ("D", "diagnostics (keeps current focus)"),
    ("T", "reader theme (keeps current focus)"),
    ("Z", "hide / show right pane"),
    ("J / K", "scroll right pane down / up"),
    ("PgDn / PgUp", "page right pane down / up"),
    ("N", "next agent (focuses it)"),
    ("Q", "quit (confirm with F1, Q if a child is live)"),
    ("?", "full key reference"),
    ("Esc", "dismiss commands"),
];

/// Rendered by `F1, ?`. Kept next to the table so the two cannot drift.
pub const HELP: &[(&str, &str)] = &[
    // First, because it is a fact about every row under it rather than a
    // binding of its own, and because somebody whose `Alt` key "does not work"
    // opens this overlay before they open an issue. Every `Alt` row below is
    // reachable from either `Alt` key: Windows spells AltGr as Ctrl+Alt, and
    // `alt_chord` counts both.
    (
        "F1",
        "open the command hub; command keys below are sequences, not chords",
    ),
    ("F1, G / E", "show git / reader, keeping current focus"),
    ("F1, B", "open the file browser and focus it"),
    (
        "F1, S",
        "show and focus the active shell; create one if none exists",
    ),
    ("F1, C", "create and focus a fresh shell"),
    (
        "F1, ← / →",
        "select the previous / next shell, wrapping around",
    ),
    ("F1, X", "close the active shell (repeat F1, X to confirm)"),
    ("F1, W", "show the work queue, keeping current focus"),
    (
        "F1, P / A",
        "open the scratch pad / ask composer and focus it",
    ),
    (
        "F1, D / T",
        "toggle diagnostics / reader theme, keeping current focus",
    ),
    ("F1, Z", "hide / show the right pane"),
    // The parenthetical is the whole of what a second agent costs this table.
    // `F4` has always meant "give the keys to the left" and a second press did
    // nothing at all, so "again" is a meaning added to a dead press rather than
    // a key taken from anybody — which is why there is no new row here and no
    // new audit under docs/keymap.md. One direction, because a modified F-key
    // is deliberately not abeam's; see `global` above on `Ctrl+F12`.
    ("F4 / F5", "focus agent / right pane"),
    ("F1, J / K", "scroll right pane, without focusing it"),
    ("F1, PgDn / PgUp", "page right pane, without focusing it"),
    ("F1, N", "next agent and focus it"),
    // "while a child is live", not "while the agent is running": `app::act`
    // quits outright only when the agent has exited *and* no shell is live, so
    // a dead agent with a shell still in the right pane asks twice as well.
    ("F1, Q", "quit (repeat F1, Q while a child is live)"),
    ("F1, ?", "this full reference"),
    // Next to F2 because it behaves like F2 — a view that displaces one and puts
    // it back — and phrased against `?` two dozen rows below, which is the same
    // pane reached the other way. "about nothing in particular" is the whole
    // difference between them, and it is the half a reader is looking for when
    // they are typing at the agent and have a question about the repository
    // rather than about a file.
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
    // The right pane only, and nothing abeam does moves an agent pane: a pty is
    // opened with a working directory and there is no call that re-roots it.
    // That is what lets the two halves of the window disagree about where they
    // are, which is what the border's workspace label exists to say.
    //
    // **Not that the agent stays put — the session inside the pty can move,
    // and the row two below is about the key that follows it there.** Claude
    // Code makes worktrees and moves into them; what cannot be moved is the
    // pane, not the conversation in it.
    ("Enter (worktrees)", "point the right pane at that worktree"),
    // Beside `Enter` because they are the two keys on one row and the pair is
    // the whole of what a reader has to hold: one moves the right pane, the
    // other starts a child in the left column and moves neither. "another" is
    // load-bearing — the session already has one — and "there" is where the
    // pane is *opened*, which is the half the row can promise: a pty is spawned
    // in a directory and cannot be moved, though the session inside it can go
    // on to make a worktree and move into that. The pane's border follows it
    // when it does.
    (
        "a (worktrees)",
        "start another agent there (F1, N reaches it)",
    ),
    // The same request, reached the short way, and it is in this table because
    // it is the one most sessions want. Claude Code makes its own worktrees, so
    // the ordinary second agent is one opened *here* and told to branch off —
    // and routing that through a list of the checkouts you are not in is the
    // long way round to the row you are already standing on. "here" against the
    // row above's "there" is the whole difference between them.
    (
        "a (git)",
        "start another agent here, in the checkout on screen",
    ),
    // The other half of the pair the two `a` rows above make — they start a
    // pane and this closes one — and the only row in this block that is about
    // the *left* column, which the key column says as it does for every other
    // conditional row here. ("That pair" used to mean `Enter (worktrees)` and
    // the `a` beside it, and the second `a` row is what made the phrase
    // ambiguous enough to be worth spelling out.)
    //
    // It is in the table for the reason `a` is: a key
    // whose condition is a state you have to arrive at is one nobody discovers
    // by pressing things, and the state this one needs is a pane you are
    // already looking at wondering how to get rid of.
    //
    // "that has exited" is load-bearing twice over. It is the whole of what
    // makes a bare letter legal here — the child that would have received it
    // has gone, so no hosted agent can ever be shadowed — and it is the answer
    // to the question the row otherwise invites, which is whether this kills
    // anything. It does not, and the pane it will not close is named because
    // pressing twice at the wrong one is how a reader finds out.
    //
    // `x` rather than `q` deliberately: `q` is documented three rows down as
    // the way *out* of the right pane, and a table that taught one letter for
    // "leave this" and the same letter for "destroy this" would be teaching a
    // mistake. `docs/keymap.md` has the argument.
    (
        "x (exited agent)",
        "close that pane, twice over — never the session's own agent",
    ),
    // The third of the trio, and the row that has to say where it is pressed
    // *and* what it costs. It is in this table for `a`'s reason — a key whose
    // condition is a state you have to arrive at is one nobody discovers by
    // pressing things — and it is worded harder than the row above because
    // what it destroys is different: not a frozen screen but a turn somebody
    // is paying for.
    //
    // "worktrees" in the key column and not "agent", which is the whole of why
    // this is not the row above with a wider condition. `x` at a live agent is
    // that child's letter and abeam never takes it; the only place the key can
    // be claimed is a list that is up while the right pane has focus. The
    // detour is also the guard, and `docs/keymap.md` carries the argument.
    (
        "x (worktrees)",
        "kill the agent standing there, twice over — the pane's border says which",
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
            global(&k(KeyCode::Char('\\'), KeyModifiers::CONTROL)),
            Some(Action::LiteralNext)
        );
        assert_eq!(
            global(&k(KeyCode::F(1), KeyModifiers::NONE)),
            Some(Action::OpenHub)
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
            global(&k(KeyCode::F(7), KeyModifiers::NONE)),
            Some(Action::ToggleSelect)
        );
    }

    #[test]
    fn retired_globals_fall_through_and_the_hub_owns_their_actions() {
        for mods in [KeyModifiers::ALT, KeyModifiers::ALT | KeyModifiers::CONTROL] {
            for c in "gesqwpadztjk".chars() {
                assert_eq!(global(&k(KeyCode::Char(c), mods)), None, "{mods:?}+{c}");
            }
            for code in [KeyCode::PageUp, KeyCode::PageDown] {
                assert_eq!(global(&k(code, mods)), None, "{mods:?}+{code:?}");
            }
        }
        for n in [2, 3, 6, 8, 9] {
            assert_eq!(global(&k(KeyCode::F(n), KeyModifiers::NONE)), None, "F{n}");
        }

        let cases = [
            (KeyCode::Char('g'), Action::ShowGit),
            (KeyCode::Char('e'), Action::ShowViewer),
            (KeyCode::Char('b'), Action::ShowBrowser),
            (KeyCode::Char('s'), Action::ShowShell),
            (KeyCode::Char('c'), Action::CreateShell),
            (KeyCode::Char('x'), Action::CloseShell),
            (KeyCode::Char('w'), Action::ShowQueue),
            (KeyCode::Char('p'), Action::ShowPad),
            (KeyCode::Char('a'), Action::ShowAsk),
            (KeyCode::Char('d'), Action::ToggleDiag),
            (KeyCode::Char('t'), Action::ToggleReaderTheme),
            (KeyCode::Char('z'), Action::ToggleZoom),
            (KeyCode::Char('n'), Action::NextAgent),
            (KeyCode::Char('q'), Action::Quit),
        ];
        for (code, action) in cases {
            assert_eq!(
                hub(&k(code, KeyModifiers::NONE)),
                Some(HubCommand::Action(action))
            );
            assert_eq!(
                hub(&k(code, KeyModifiers::SHIFT)),
                Some(HubCommand::Action(action))
            );
            if let KeyCode::Char(c) = code {
                assert_eq!(
                    hub(&k(
                        KeyCode::Char(c.to_ascii_uppercase()),
                        KeyModifiers::SHIFT
                    )),
                    Some(HubCommand::Action(action)),
                    "the shifted character spelling must resolve too"
                );
            }
            assert_eq!(hub(&k(code, KeyModifiers::ALT)), None);
            assert_eq!(hub(&k(code, KeyModifiers::CONTROL)), None);
            assert_eq!(
                hub(&k(code, KeyModifiers::ALT | KeyModifiers::CONTROL)),
                None,
                "AltGr must not trigger F1 hub commands"
            );
        }
        assert_eq!(
            hub(&k(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Some(HubCommand::Reference)
        );
        assert_eq!(
            hub(&k(KeyCode::F(1), KeyModifiers::NONE)),
            Some(HubCommand::Open)
        );
    }

    #[test]
    fn shell_navigation_is_owned_by_the_hub_only() {
        for (code, action) in [
            (KeyCode::Left, Action::PreviousShell),
            (KeyCode::Right, Action::NextShell),
        ] {
            assert_eq!(
                hub(&k(code, KeyModifiers::NONE)),
                Some(HubCommand::Action(action))
            );
            assert_eq!(
                hub(&k(code, KeyModifiers::SHIFT)),
                Some(HubCommand::Action(action))
            );
            assert_eq!(hub(&k(code, KeyModifiers::ALT)), None);
            assert_eq!(hub(&k(code, KeyModifiers::CONTROL)), None);
            assert_eq!(
                hub(&k(code, KeyModifiers::ALT | KeyModifiers::CONTROL)),
                None,
                "AltGr must not trigger shell navigation"
            );
            assert_eq!(global(&k(code, KeyModifiers::NONE)), None);
        }
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
        // The F1 hub releases the Alt namespace outright, including Windows'
        // Ctrl+Alt representation of AltGr. This count proves the loop did not
        // accidentally retain a classic binding.
        assert_eq!(resolved, 0, "no Alt binding remains global");

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
    fn the_ask_is_a_hub_command_and_question_marks_remain_local() {
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
            .find(|(key, _)| *key == "F1, P / A")
            .expect("the key the overlay promises");
        assert_eq!(*key, "F1, P / A");
        assert!(said.contains("ask"), "got: {said}");
        assert!(said.contains("focus"), "the ask focus rule: {said}");
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
            "F1",
            "F1, G / E",
            "F1, B",
            "F1, S",
            "F1, C",
            "F1, ← / →",
            "F1, X",
            "F1, W",
            "F1, P / A",
            "F1, D / T",
            "F1, Z",
            "F1, N",
            "F1, Q",
            "F4 / F5",
            "F7",
            "Ctrl+\\ or F12",
        ] {
            assert!(
                listed.iter().any(|k| k.contains(expected)),
                "{expected} is bound but not in the F1 overlay"
            );
        }
    }
}
