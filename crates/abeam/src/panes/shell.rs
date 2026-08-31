//! The command view: a shell hosted in the right pane.
//!
//! What it is for is the round trip that otherwise costs a second window:
//! `git branch`, `uv run ruff format`, `cargo test` — run in the directory
//! abeam was pointed at, next to the agent session that is about to be told
//! what they printed. `F1, S` opens and focuses it; `F4` returns focus to the
//! agent.
//!
//! What it deliberately is not is a multiplexer. There is one child, started
//! when the pane is first drawn and never restarted behind your back; there are
//! no tabs and no splits. Nothing started here outlives abeam either, and that
//! one is not free — ending a shell does not end the `cargo build` the shell
//! started, so `abeam-pty` takes down the tree rather than the one process it
//! can see: a job object closed with the session on Windows, a kill aimed at
//! the child's own process group on Unix. It also keeps no buffer of its own:
//! the history it scrolls through is the one the `vt100` parser behind the pty
//! already writes into, and this pane only moves the window onto it.
//!
//! ## Why this pane is different from the other three
//!
//! Git, files and diagnostics are read-only, which is what lets an unbound
//! keystroke in them be harmless. This one hosts a live child and takes every
//! key it is given, `Esc` and `q` included: those cannot mean "back to the
//! agent" while something inside the pane is listening for them.
//!
//! Which is why nothing here is a fact about the *type*. The app decides where
//! `Esc` goes from what [`Pane::handle_key`] returned, so a live child claims
//! it by reporting `Yes` and a dead one lets it through to mean what it means
//! everywhere else; and it takes what the border promises from
//! [`Pane::exit_hint`], which is answered from the same live-or-not question
//! and is likewise about this instant rather than about the type. The single
//! frame on which that answer is stale is the first: the border is drawn
//! before `render`, and `render` is what spawns, so the frame that starts the
//! shell still advertises `esc→agent` — for the few milliseconds until the new
//! session's first output asks for another one.
//!
//! ## The contract
//!
//! - **Spawned on the first frame that draws it**, never at startup. Being
//!   drawn is the only signal a pane gets that it is the one on screen, and the
//!   viewer already uses it for exactly this. A session that never presses
//!   `F1, S` must never have paid for a shell process before it is chosen.
//! - **Restartable.** A child that exits leaves the pane saying so, with
//!   `Enter` to start another. While it is dead the pane must *not* claim every
//!   key — `Esc` and `q` fall through so the way back to the agent is the one
//!   the rest of the app taught.
//! - **Sized from the rect it was drawn into**, through
//!   [`Pane::on_resize`], which the app calls after every frame.
//! - **`tick` must not block.** `try_wait`, never `wait`
//!   (`docs/conpty-findings.md`, constraint 2).

use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::launch;
use crate::pane::{Handled, Pane};
use crate::panes::TerminalPane;
use crate::text::{self, dim, err};

/// Candidate shells on Windows, best first. Tried in order at spawn time; the
/// first one that starts wins, so a machine without PowerShell 7 falls back
/// rather than showing an error nobody can act on.
///
/// A constant here and a function on Unix, because there the best answer is not
/// a name anybody can write down in advance — see `shells` below.
#[cfg(windows)]
pub const SHELLS: &[&str] = &["pwsh.exe", "powershell.exe", "cmd.exe"];

/// The Unix candidate list, best first, built from the value of `$SHELL`.
///
/// The first answer on Unix is the shell the user already chose, and that is
/// something only the machine knows. `bash` and then `sh` stay behind it
/// because `$SHELL` is a *preference* and not a choice: nobody typed it at
/// abeam, it was set once and can name a shell that has been uninstalled since,
/// and falling through from a broken one is better than a pane with nothing in
/// it. `sh` is last because it is the one program name a Unix is not allowed to
/// be missing.
///
/// The value is handed in rather than read here, so that the tests which pin
/// this order can run beside two hundred others without one of them writing to
/// an environment the whole process shares.
#[cfg(unix)]
fn shells(login: Option<std::ffi::OsString>) -> Vec<String> {
    // Empty is unset with extra steps — `SHELL=` names no program, and passing
    // "" on would ask the resolver to walk `PATH` looking for nothing.
    //
    // `into_string` rather than `to_string_lossy`: a `$SHELL` that is not valid
    // UTF-8 cannot become the `String` the resolver takes, and a lossy
    // rendering of it names a *different* file. Dropping it lands on `bash`,
    // which is what having anything behind `$SHELL` at all is for.
    let chosen = login
        .and_then(|s| s.into_string().ok())
        .filter(|s| !s.is_empty());
    chosen
        .into_iter()
        .chain(["bash", "sh"].map(String::from))
        .collect()
}

/// The list to try when nothing was chosen, which is the one thing the two
/// platforms disagree about here: a constant on one and a question about the
/// environment on the other.
#[cfg(windows)]
fn preferred() -> Vec<String> {
    SHELLS.iter().copied().map(String::from).collect()
}

/// See the Windows twin above.
#[cfg(unix)]
fn preferred() -> Vec<String> {
    shells(std::env::var_os("SHELL"))
}

/// Rows one notch of the wheel moves. Three is the terminal convention, and the
/// same number `crate::scroll` uses; it is repeated rather than shared because
/// this pane holds no `Scroll` — what it moves is the parser's scrollback
/// offset, which is measured from the *bottom* and whose length nothing can ask
/// for.
const WHEEL: usize = 3;

/// A program and the arguments it wants, resolved once at construction so that
/// the shells to try are a fact about the pane rather than something `render`
/// works out again on every restart.
struct Candidate {
    program: String,
    args: Vec<String>,
}

impl Candidate {
    fn new(program: impl Into<String>) -> Self {
        let program = program.into();
        let args = args_for(&program).iter().copied().map(String::from).collect();
        Self { program, args }
    }
}

enum State {
    /// Nothing has drawn this pane, so nothing has been spawned. Where a
    /// session that never presses `Alt+S` stays forever.
    Cold,
    /// A child, alive or finished. `TerminalPane` knows which, and holds its
    /// last screen either way — a shell that has exited still has the output of
    /// the command that killed it on it.
    Hosted { name: String, term: TerminalPane },
    /// Every candidate refused to start. `render` cannot return an error and an
    /// empty box is indistinguishable from a shell that has not printed
    /// anything yet, so the failure has to be a state the pane holds and draws.
    Failed { tried: Vec<String>, why: String },
}

pub struct ShellPane {
    /// The child's working directory: the same root git and the viewer use, so
    /// a `git status` typed here answers about the repository on screen.
    root: PathBuf,
    candidates: Vec<Candidate>,
    state: State,
    /// The rect the last frame drew. The pty's size, the page a `PgDn` moves
    /// and the size a restart starts at all come from here, so there is one
    /// number rather than three that can drift apart.
    drawn: Rect,
}

impl ShellPane {
    pub fn new(root: PathBuf, program: Option<String>) -> Self {
        let candidates = match program {
            // An explicit program is a choice, not a first preference. Falling
            // back from it would hide a typo in `ABEAM_SHELL` behind a shell
            // nobody asked for, and the mistake would surface much later as
            // "why is my profile not loading".
            //
            // `$SHELL` sits on the other side of that line and is read by
            // `preferred` rather than reaching here, precisely because it is
            // the other kind of thing: a preference the user expressed to the
            // operating system, not a program they typed at abeam.
            Some(p) => vec![Candidate::new(p)],
            None => preferred().into_iter().map(Candidate::new).collect(),
        };
        Self {
            root,
            candidates,
            state: State::Cold,
            drawn: Rect::ZERO,
        }
    }

    /// Is a child still running in here?
    ///
    /// Two callers asking different questions of the same fact. The app asks
    /// before it lets abeam exit, because leaving kills whatever is in this pty
    /// and taking down someone's `cargo build` because the *other* pane
    /// finished is not a decision abeam gets to make on its own. `Pane` asks it
    /// as [`takes_input`](Pane::takes_input), because a live child is also the
    /// only thing here there is to type into.
    ///
    /// Answered from the last [`Pane::tick`], not from a fresh `try_wait`: the
    /// app polls once per loop and both callers run inside that loop, so a
    /// second poll here would cost a syscall to learn what is already known.
    pub fn is_live(&self) -> bool {
        matches!(&self.state, State::Hosted { term, .. } if !term.has_exited())
    }

    /// Whether no start has been attempted for this pane yet.
    ///
    /// This is state rather than a rendering flag: `Enter` can start a cold
    /// pane after a frame has supplied its dimensions even when the app chose
    /// not to render it in that frame (while confirming its close).
    pub fn is_cold(&self) -> bool {
        matches!(self.state, State::Cold)
    }

    /// Put `text` at the prompt **without submitting it**, and say whether it
    /// went.
    ///
    /// The only route into this child that is not a keystroke somebody made,
    /// and the missing newline is the whole of what it promises. `Enter` in the
    /// ask pane picks a single-line command out of an answer and hands it here;
    /// what arrives is a prompt with a command typed at it, which the reader
    /// then reads and submits, or edits, or backspaces away. Sending the
    /// newline as well would turn "abeam suggested this" into "abeam ran this",
    /// and the two are not the same decision — see `crate::panes::ask`, which
    /// refuses to join a multi-line block for the same reason.
    ///
    /// Three guards, and each is a different failure:
    ///
    /// - **[`live`](Self::live)**, because a cold pane has no child at all: it
    ///   spawns on the frame that draws it, so a hand-off arriving before that
    ///   frame has nowhere to go. `App` is what defers it a frame, the way it
    ///   already defers the queue's `Enter`.
    /// - **`set_scrollback(0)`**, for [`handle_key`](Pane::handle_key)'s
    ///   reason: text appearing at a prompt that is scrolled off the bottom of
    ///   the pane is text nobody can see arrive.
    /// - **[`bracketed_paste`](TerminalPane::bracketed_paste)**, which is the
    ///   one that costs something. `TerminalPane::send_text` degrades to raw
    ///   bytes for a child that never enabled the mode, and raw bytes carrying
    ///   a newline submit — so a caller sending text nobody typed has to check
    ///   first, which is the rule `App::pump_queue` already applies to the
    ///   agent's own pty. What it costs here is real and worth naming: PSReadLine
    ///   enables the mode and `cmd.exe` does not, so on the `cmd` fallback the
    ///   hand-off is refused rather than typed. Refusing is the safe direction —
    ///   nothing appears, rather than a command running unread — and the way
    ///   through is the shell the pane leads with.
    pub fn send_command(&mut self, text: &str) -> bool {
        let Some(term) = self.live() else {
            return false;
        };
        term.set_scrollback(0);
        if !term.bracketed_paste() {
            return false;
        }
        term.send_text(text).is_ok()
    }

    /// Try each candidate in order and keep the first that starts.
    ///
    /// Takes `&self` and returns the state rather than assigning it, which is
    /// what lets the caller be `render` — a spawn in the middle of a frame
    /// cannot also be holding a mutable borrow of the thing it is drawing.
    fn start(&self, at: Rect) -> State {
        let mut why = String::new();
        for candidate in &self.candidates {
            // Resolved to an absolute path first, and the pty is never given
            // anything else. A bare name reaching the spawn itself is a
            // vulnerability rather than a convenience — on Windows because it
            // is looked for in the current directory before `PATH` is consulted
            // at all, and on Unix because a `PATH` with a relative or empty
            // entry on it is a `PATH` the repository on screen can be part of.
            // See [`crate::launch`], which refuses both.
            //
            // The same resolver the left pane uses, script routing included,
            // rather than a stricter one for this pane. `ABEAM_SHELL` pointing
            // at a wrapper — a login script, a `nu.cmd` — is the same wish as
            // `abeam +claude` on an npm install, and two implementations of that
            // wish would be two places for it to be got wrong. On Unix there is
            // no routing to share, because a `#!` line is the kernel's business
            // and the wrapper simply is the program. What is special about this
            // pane is the *list* of shells and where they live, which is
            // `preferred` and `known_home` and stays here.
            let launch = match launch::resolve_preferring(
                &candidate.program,
                &candidate.args,
                known_home(&candidate.program),
            ) {
                Ok(launch) => launch,
                Err(reason) => {
                    why = reason;
                    continue;
                }
            };
            let cfg = launch
                .config()
                .cwd(&self.root)
                .size(at.height.max(1), at.width.max(1));
            match TerminalPane::spawn_with(cfg) {
                // Named after what started, not after what was asked for. With
                // a resolution step in front those can differ, and the border
                // has one job: to say which program is taking the typing. The
                // *target* rather than the program, so a routed `nu.cmd` is
                // still "nu" and not "cmd".
                Ok(term) => {
                    return State::Hosted {
                        name: name_of(&launch.target),
                        term,
                    };
                }
                // Only the last reason is kept. On a machine without PowerShell
                // 7 the first candidate fails every single time, and leading
                // with that is leading with the least informative half of it.
                Err(e) => why = format!("{e:#}"),
            }
        }
        State::Failed {
            tried: self.candidates.iter().map(|c| c.program.clone()).collect(),
            why,
        }
    }

    // --- history ---------------------------------------------------------

    // Both of these match on `Hosted` without asking whether the child is still
    // alive, and that is the point: the history of a shell that has exited is
    // the most interesting history there is, and `G` still has to reach the
    // bottom of it.

    /// Where the view is, in rows above the live screen.
    fn at(&self) -> usize {
        match &self.state {
            State::Hosted { term, .. } => term.scrollback(),
            _ => 0,
        }
    }

    fn to(&mut self, rows: usize) -> Handled {
        match &self.state {
            State::Hosted { term, .. } => term.set_scrollback(rows).into(),
            _ => Handled::No,
        }
    }

    /// Positive is backwards, into the history; negative is towards the live
    /// screen. The offset counts up as you go back, which is the opposite of
    /// every other pane's, so the sign is fixed here once and the callers below
    /// read as the directions a user would name.
    ///
    /// Relative all the way down to the parser, rather than a read here and a
    /// write after it: the reader thread moves the same offset as rows arrive,
    /// so a `PgUp` pressed while output is flowing would otherwise compute from
    /// a base that had already shifted under it.
    fn scroll(&mut self, rows: isize) -> Handled {
        match &self.state {
            State::Hosted { term, .. } => term.scroll_by(rows).into(),
            _ => Handled::No,
        }
    }

    /// A page keeps one row of overlap, so the eye has an anchor across the
    /// jump. The same arithmetic as `crate::scroll::Scroll::page`.
    fn page(&self) -> isize {
        self.drawn.height.saturating_sub(1).max(1) as isize
    }

    /// The scroll vocabulary the F1 overlay promises, moving the parser's
    /// scrollback rather than any offset of this pane's own.
    ///
    /// `g`/`Home` reach the oldest row kept and `G`/`End` the live screen,
    /// which is the same top and bottom the other panes mean even though the
    /// number behind them counts the other way. Ctrl is excluded outright:
    /// every Ctrl+letter belongs to whatever is hosted here, and the two the
    /// shared vocabulary claims are not in the overlay to be honoured anyway.
    fn scroll_binding(&mut self, key: KeyEvent) -> Handled {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Handled::No;
        }
        let page = self.page();
        match key.code {
            KeyCode::Char('k') | KeyCode::Up => self.scroll(1),
            KeyCode::Char('j') | KeyCode::Down => self.scroll(-1),
            KeyCode::Char('b') | KeyCode::PageUp => self.scroll(page),
            KeyCode::Char(' ') | KeyCode::PageDown => self.scroll(-page),
            KeyCode::Char('g') | KeyCode::Home => self.to(usize::MAX),
            KeyCode::Char('G') | KeyCode::End => self.to(0),
            _ => Handled::No,
        }
    }

    /// [`is_live`](Self::is_live), in the form that hands over the child.
    fn live(&mut self) -> Option<&mut TerminalPane> {
        match &mut self.state {
            State::Hosted { term, .. } if !term.has_exited() => Some(term),
            _ => None,
        }
    }
}

impl Pane for ShellPane {
    /// Clipped from the right in a 46-column pane, so each state leads with
    /// the thing that would be worth the last few columns.
    fn title(&self) -> String {
        match &self.state {
            State::Cold => "shell".to_string(),
            State::Hosted { name, term } => match term.exit_status() {
                // Two things matter about a dead shell and neither of them is
                // which shell it was: that it is dead, and that Enter is the
                // way back.
                Some(status) => {
                    format!("exited ({}) · enter restarts · {name}", status.exit_code())
                }
                None => format!("shell · {name}"),
            },
            State::Failed { .. } => "no shell · enter retries".to_string(),
        }
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        self.drawn = inner;

        // Being drawn *is* the signal that this pane is the one on screen, and
        // it is the only such signal a pane gets — the viewer takes up a
        // pending file here for exactly the same reason. Starting the child
        // anywhere earlier would charge every session for a shell it may never
        // ask for.
        if matches!(self.state, State::Cold) {
            self.state = self.start(inner);
        }

        match &mut self.state {
            // Including after it has exited: the last screen is the output of
            // whatever killed it, and throwing that away to draw a tombstone
            // would lose the only thing worth reading. The border says so.
            State::Hosted { term, .. } => term.render(f, inner),
            State::Failed { tried, why } => {
                f.render_widget(
                    Paragraph::new(failure(tried, why, inner.width as usize)),
                    inner,
                );
            }
            // Unreachable: `start` leaves one of the other two, always.
            State::Cold => {}
        }
    }

    fn tick(&mut self) -> bool {
        let State::Hosted { term, .. } = &mut self.state else {
            return false;
        };
        let was_live = !term.has_exited();
        // Unconditionally and first: it clears the flag as it reads it.
        let dirty = term.tick();
        // `try_wait`, never `wait` — under ConPTY the latter never returns
        // (`docs/conpty-findings.md`, constraint 2). A dropped error is the
        // right outcome: `tick` has nowhere to put one, and a child we cannot
        // ask about is a child the pane goes on showing the last screen of.
        let _ = term.poll_exit();
        dirty || (was_live && term.has_exited())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        if let Some(term) = self.live() {
            // Typing is a request to be at the prompt. Without this a key
            // pressed while the view is scrolled back would land somewhere off
            // the bottom of the pane, which no terminal anyone has used does.
            term.set_scrollback(0);
            // A pty whose child died between the last `tick` and this key can
            // refuse the write. Losing the keystroke is the right outcome;
            // taking the whole session down with it is not, and the frame this
            // reports is the one in which `tick` notices the exit.
            let _ = term.handle_key(key);
            return Ok(Handled::Yes);
        }

        // Nothing to type into. `Esc` and `q` must fall through from here: the
        // shell reads an unhandled one as "back to the agent", and that is the
        // way out the other three views taught.
        if key.code == KeyCode::Enter {
            // ...but not before a frame has said how big this pane is. `App`
            // drains every pending event before drawing, so `Alt+S` and `Enter`
            // pressed together both land while `drawn` is still empty, and a
            // shell started at the 1x1 that gets clamped to has already
            // reflowed its banner into a single column by the time the next
            // frame corrects it. Declining costs nothing: that frame spawns it.
            if self.drawn.width == 0 || self.drawn.height == 0 {
                return Ok(Handled::No);
            }
            self.state = self.start(self.drawn);
            return Ok(Handled::Yes);
        }
        Ok(self.scroll_binding(key))
    }

    /// `Alt+J`/`Alt+K` arrive here as a bare `Down`/`Up`. Forwarded to a live
    /// shell those are its *history* keys, so glancing at this pane would
    /// quietly load an earlier command into its prompt. They scroll the pane
    /// instead, which is what the person who pressed them asked for.
    fn scroll_key(&mut self, key: KeyEvent) -> Result<Handled> {
        Ok(self.scroll_binding(key))
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        if let Some(term) = self.live()
            // Gated on the mode the child enabled: an unrequested report dumps
            // escape sequences into its prompt (conpty-findings, constraint 4).
            // The same dead-pty race as `handle_key`, and the same answer.
            && term.handle_mouse(ev).unwrap_or(Handled::No).is_yes()
        {
            return Ok(Handled::Yes);
        }

        // The child did not want it, so a wheel notch here means what `Alt+J`
        // means: move the window onto the history.
        Ok(match ev.kind {
            MouseEventKind::ScrollUp => self.scroll(WHEEL as isize),
            MouseEventKind::ScrollDown => self.scroll(-(WHEEL as isize)),
            _ => Handled::No,
        })
    }

    fn takes_input(&self) -> bool {
        self.is_live()
    }

    /// The one pane that answers this, for [`Pane::selected_text`]'s reason:
    /// there is a terminal grid behind these rows and it knows which of them
    /// are continuations of the row above.
    ///
    /// Answered whatever the child is doing, exited included — the last screen
    /// of a shell that died is the output of whatever killed it, which is the
    /// single most likely thing anybody wants to hand the agent.
    fn selected_text(&self, first: u16, last: u16) -> Option<String> {
        let State::Hosted { term, .. } = &self.state else {
            return None;
        };
        Some(term.rows_text(first, last))
    }

    /// `Esc` belongs to the child, so the border names the key that does not.
    /// Once the child has exited it belongs to nobody, and the way out is the
    /// one every other view taught — which is the default, and saying it here
    /// would be one more place for the two to disagree.
    fn exit_hint(&self) -> &'static str {
        if self.is_live() {
            "f4→agent"
        } else {
            "esc→agent"
        }
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        let State::Hosted { term, .. } = &self.state else {
            return None;
        };
        // A dead child has no prompt to point at, and the pane no longer takes
        // typing — a cursor left blinking there would be the strongest possible
        // lie about where the keys are going.
        if term.has_exited() {
            return None;
        }
        let (col, row) = term.cursor()?;
        // Scrolled back, the live screen has moved down the pane by exactly
        // that many rows, and can be off the bottom of it entirely.
        let row = row.checked_add(u16::try_from(self.at()).ok()?)?;
        (row < self.drawn.height).then_some((col, row))
    }

    /// Remembered whatever the state, so that `Enter` starts the next child at
    /// the size that is on screen rather than at whatever the last one had.
    fn on_resize(&mut self, inner: Rect) -> Result<()> {
        self.drawn = inner;
        if let Some(term) = self.live() {
            // Swallowed, like every other call into this child — and this is
            // the one that used not to be. `App::draw` propagates what comes
            // back here, so a `ResizePseudoConsole` refusing in *this* pane
            // ended the agent session in the other one and skipped the
            // transcript abeam prints on the way out. The left pane
            // propagating is right, because if the agent's pty cannot be
            // resized abeam is over; this pane is exactly where that stops
            // being true.
            let _ = term.on_resize(inner);
        }
        // A child that has exited is deliberately not resized. Its last screen
        // is the output of whatever killed it, and `vt100`'s `set_size` rewrites
        // a screen destructively — dragging the window narrower would quietly
        // eat the thing the pane is still being kept open to show.
        Ok(())
    }

    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        let Some(term) = self.live() else {
            return Ok(Handled::No);
        };
        term.set_scrollback(0);
        let _ = term.handle_paste(text);
        Ok(Handled::Yes)
    }
}

/// Both PowerShells open with a banner and a blank line, which in a short pane
/// is most of the first screen spent on nothing. `cmd` has no equivalent flag
/// and is given none.
///
/// Matched on the file stem so an explicit `ABEAM_SHELL=C:\…\pwsh.exe` is
/// recognised too, and so anything else — `bash`, `nu` — is spawned bare rather
/// than handed a flag it will refuse to start without understanding.
fn args_for(program: &str) -> &'static [&'static str] {
    match stem_of(program).as_str() {
        "pwsh" | "powershell" => &["-NoLogo"],
        _ => &[],
    }
}

// --- finding a shell ------------------------------------------------------
//
// The search itself is `crate::launch`, shared with the program `main` hosts.
// What is left here is the part that is knowledge about *shells* rather than
// about launching: which ones to try, which is `preferred` above, and where the
// operating system keeps them, which is below.

/// Where Windows keeps the shells it ships, consulted before `PATH`.
///
/// `cmd.exe` and `powershell.exe` are operating-system components with exactly
/// one right answer, and taking that answer from `PATH` means taking it from
/// something a user, an installer, or a directory added to the front of the
/// list can reorder. PowerShell 7 is not part of Windows and its installer is
/// only *usually* here, so its entry is a first guess with the `PATH` walk
/// still behind it.
#[cfg(windows)]
fn known_home(program: &str) -> Option<PathBuf> {
    let windows = || std::env::var_os("SystemRoot").map(PathBuf::from);
    match stem_of(program).as_str() {
        "cmd" => Some(windows()?.join("System32").join("cmd.exe")),
        "powershell" => Some(
            windows()?
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
        ),
        "pwsh" => Some(
            PathBuf::from(std::env::var_os("ProgramFiles")?)
                .join("PowerShell")
                .join("7")
                .join("pwsh.exe"),
        ),
        _ => None,
    }
}

/// Nothing, on Unix, and the emptiness is the decision rather than a gap left
/// to fill in later.
///
/// The table above exists because a Windows component has exactly one right
/// answer that `PATH` is not to be trusted for. Unix has no such answer to
/// write down: the shells people run live in `/bin` on one distribution and
/// `/usr/bin` on the next, with the two the same directory on some and not on
/// others, and the candidate this list is most confident of — `$SHELL` —
/// already arrives as an absolute path and never reaches a search at all.
/// Hardcoding `/bin/sh` would buy nothing the `PATH` walk does not already
/// give, because that walk takes absolute entries only and so cannot be
/// steered by a relative one somebody put at the front; it would only be a
/// second place to be wrong about a machine neither of us has seen.
#[cfg(unix)]
fn known_home(_program: &str) -> Option<PathBuf> {
    None
}

/// `C:\…\pwsh.exe` is what gets started and `pwsh` is what the border says;
/// `/usr/bin/fish` and `fish` are the same sentence on the other platform.
fn name_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The same name folded to one case, for the tables that match on it:
/// [`args_for`] on both platforms and `known_home` on Windows.
///
/// The fold is a Windows fact applied everywhere, and on Unix it is wrong in a
/// way worth writing down rather than repairing. `/usr/bin/Pwsh` is a different
/// file from `/usr/bin/pwsh` there, and this hands `-NoLogo` to the first on
/// the strength of a name only the second has. Nothing follows from it — the
/// flag is PowerShell's under either spelling, and a capitalised `pwsh` is not
/// a file anybody ships — so the one rule stays rather than growing a `cfg` for
/// a machine that has never existed. It is here because a silent assumption
/// left behind by a port that audited this exact class of them reads as an
/// oversight.
fn stem_of(program: &str) -> String {
    name_of(Path::new(program)).to_ascii_lowercase()
}

/// What the pane says when nothing would start.
///
/// It names every program that was tried, because the useful next move depends
/// entirely on which list this was: a single name means `ABEAM_SHELL` is wrong,
/// and the whole candidate list means this is not the machine anyone expected —
/// a Windows without `cmd.exe` on it, a Unix without `sh`.
///
/// The reason from the operating system comes *last*, under the advice rather
/// than above it. It is the only part of this screen nobody can act on, and it
/// arrives wrapped over four rows of a 46-column pane — put first, it pushes
/// the one sentence that fixes the problem off the bottom.
fn failure(tried: &[String], why: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = text::block("No shell would start.", width, err());
    let mut say = |body: &str, style| {
        lines.push(Line::default());
        lines.extend(text::block(body, width, style));
    };
    say(&format!("Tried: {}", tried.join(", ")), dim());
    say(
        "Set ABEAM_SHELL to the program you want, and press Enter to try again.",
        dim(),
    );
    if !why.is_empty() {
        say(why, dim());
    }
    lines
}

/// The one table in this module whose answers are the same on both platforms,
/// which is why it is not behind a `cfg` and not written twice: `pwsh` is a
/// program on Linux as much as on Windows, and the flag it is given is the
/// right flag in both places.
#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn the_powershells_are_asked_not_to_print_a_banner_and_cmd_is_not() {
        // Delete this and nothing notices until a shell pane opens on a banner
        // and a blank line, which in a short pane is most of the first screen
        // spent on nothing.
        assert_eq!(args_for("pwsh.exe"), ["-NoLogo"]);
        assert_eq!(args_for("powershell.exe"), ["-NoLogo"]);
        // An explicit ABEAM_SHELL is often a full path, and is still PowerShell.
        // The separator is the only thing on this table that is not the same in
        // both places, so it is the one line that has to be asked twice.
        #[cfg(windows)]
        assert_eq!(
            args_for(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            ["-NoLogo"]
        );
        #[cfg(unix)]
        assert_eq!(args_for("/usr/bin/pwsh"), ["-NoLogo"]);
        // Anything else is spawned bare: a flag a shell does not understand is
        // a shell that refuses to start.
        assert!(args_for("cmd.exe").is_empty());
        assert!(args_for("bash").is_empty());
        assert!(args_for("nu.exe").is_empty());
    }
}

/// Everything here starts a real child in a real pty, so it is Windows-only
/// like the rest of the pty-backed suite — including the two tests that start
/// nothing, because what they read is a list of Windows program names and a
/// directory only Windows has.
///
/// The children are `cmd.exe`: bare when the test needs one that stays, `/c`
/// when it needs one that is already gone. What is under test is what this pane
/// does with a child, never what the child prints — with the one exception of
/// the working directory, which is only observable by asking it.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::{Duration, Instant};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn wheel(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// A pane that will spawn exactly this, bypassing the [`SHELLS`] search.
    /// Tests need a child that exits the moment it starts, and `ABEAM_SHELL`
    /// names a program rather than a command line.
    fn pane(dir: &TempDir, program: &str, args: &[&str]) -> ShellPane {
        ShellPane {
            root: dir.path().to_path_buf(),
            candidates: vec![Candidate {
                program: program.to_string(),
                args: args.iter().copied().map(String::from).collect(),
            }],
            state: State::Cold,
            drawn: Rect::ZERO,
        }
    }

    /// The same with a whole search list, for the two tests that are about the
    /// list rather than about what is on it.
    fn panes(dir: &TempDir, programs: &[&str]) -> ShellPane {
        ShellPane {
            root: dir.path().to_path_buf(),
            candidates: programs.iter().copied().map(Candidate::new).collect(),
            state: State::Cold,
            drawn: Rect::ZERO,
        }
    }

    /// Draw one frame at this size, which is also what starts the child.
    fn draw(pane: &mut ShellPane, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| pane.render(f, f.area())).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn hosted(pane: &ShellPane) -> &TerminalPane {
        match &pane.state {
            State::Hosted { term, .. } => term,
            _ => panic!("no child: {}", pane.title()),
        }
    }

    /// Poll until the child has been reaped. There is no `wait` to block on
    /// (`docs/conpty-findings.md`, constraint 2), which is why this is a loop.
    fn wait_for_exit(pane: &mut ShellPane) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            pane.tick();
            if hosted(pane).has_exited() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the child never exited");
    }

    /// A live child with plenty behind the screen, and finished producing it.
    ///
    /// Both halves are load-bearing. The tests below count rows, so waiting for
    /// merely *some* history lets a `PgUp` clamp against a scrollback that is
    /// still filling. And `vt100` advances the offset itself as rows arrive —
    /// on purpose, so that a reader who has scrolled back stays where they are
    /// looking — so a test that began while output was still coming would watch
    /// its own offsets move underneath it. Waiting for quiet buys both.
    ///
    /// Reading a file rather than looping in `cmd` keeps the whole command out
    /// of the argument quoting rules, which are their own subject.
    fn with_history(dir: &TempDir) -> ShellPane {
        /// Comfortably more than the deepest scroll any test here performs.
        const ENOUGH: usize = 20;

        let body: String = (1..=60).map(|i| format!("line-{i}\r\n")).collect();
        dir.write("many.txt", body.as_bytes());
        // `/k`, so the child is still there afterwards: the wheel fallback is
        // only interesting while something is alive to decline the event.
        let mut pane = pane(dir, "cmd.exe", &["/k", "type", "many.txt"]);
        draw(&mut pane, 30, 6);

        let deadline = Instant::now() + Duration::from_secs(15);
        let (mut last, mut still) = (0, 0);
        while Instant::now() < deadline {
            pane.tick();
            let read = hosted(&pane).diagnostics().bytes_read;
            // How much history there is, asked by going as far back as the
            // parser will allow and reading where that landed.
            pane.to(usize::MAX);
            let depth = pane.at();
            pane.to(0);

            still = if depth >= ENOUGH && read == last { still + 1 } else { 0 };
            if still == 3 {
                return pane;
            }
            last = read;
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("a six-row pane fed sixty lines never settled with history behind it");
    }

    #[test]
    fn nothing_is_spawned_until_the_pane_is_drawn() {
        // A session that never presses Alt+S must never pay for a shell
        // process, and being drawn is the only signal a pane gets that it is
        // the one on screen.
        let dir = TempDir::new("shell-cold");
        let mut pane = pane(&dir, "cmd.exe", &[]);
        assert!(matches!(pane.state, State::Cold));
        assert_eq!(pane.title(), "shell");
        assert!(!pane.takes_input());
        // ...and nothing here that quitting abeam would kill.
        assert!(!pane.is_live());
        assert!(!pane.tick());

        // A rect with nothing in it is not being on screen either.
        let mut term = Terminal::new(TestBackend::new(10, 4)).unwrap();
        term.draw(|f| pane.render(f, Rect::ZERO)).unwrap();
        assert!(matches!(pane.state, State::Cold));

        draw(&mut pane, 40, 8);
        assert!(matches!(pane.state, State::Hosted { .. }));
        assert!(pane.takes_input());
        assert_eq!(pane.title(), "shell · cmd");
    }

    #[test]
    fn a_live_child_takes_esc_and_q_and_a_dead_one_leaves_them_to_the_shell() {
        let dir = TempDir::new("shell-esc");

        let mut live = pane(&dir, "cmd.exe", &[]);
        draw(&mut live, 40, 8);
        assert!(live.takes_input(), "the border promises F4 as the way out");
        // The same fact the app reads before it lets abeam exit: quitting would
        // kill this child, so quitting has to ask first.
        assert!(live.is_live());
        let mut sent = hosted(&live).diagnostics().keys_sent;
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            assert_eq!(live.handle_key(key(code)).unwrap(), Handled::Yes);
            // `Yes` alone would still be reported by a `handle_key` that had
            // stopped writing to the child altogether — claiming the key is
            // only half of taking it.
            let now = hosted(&live).diagnostics().keys_sent;
            assert!(now > sent, "{code:?} was claimed but never sent");
            sent = now;
        }

        let mut dead = pane(&dir, "cmd.exe", &["/c", "exit"]);
        draw(&mut dead, 40, 8);
        wait_for_exit(&mut dead);
        assert!(!dead.takes_input());
        assert!(!dead.is_live(), "nothing left for abeam to wait for");
        // ...so the app can read them as "give focus back to the agent", which
        // is the only route out of a pane that is no longer hosting anything.
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            assert_eq!(dead.handle_key(key(code)).unwrap(), Handled::No);
        }
    }

    #[test]
    fn enter_restarts_a_dead_child_and_the_title_says_so_first() {
        let dir = TempDir::new("shell-restart");
        let mut pane = pane(&dir, "cmd.exe", &["/c", "exit"]);
        draw(&mut pane, 40, 8);
        wait_for_exit(&mut pane);

        let title = pane.title();
        assert!(title.starts_with("exited (0)"), "got: {title}");
        // A 46-column pane clips from the right, so the way back has to survive
        // the clip that loses the shell's name.
        assert!(title.contains("enter restarts"), "got: {title}");

        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert!(pane.takes_input(), "a fresh child takes typing again");
        assert_eq!(pane.title(), "shell · cmd");
    }

    #[test]
    fn the_rect_the_pane_was_drawn_into_is_the_size_the_pty_is_told() {
        // One number, not two. A hosted shell wrapping in a different place
        // from the pane it is drawn in is the failure this rules out, and it is
        // ruled out by there being no second calculation to disagree.
        //
        // Both numbers are asserted, and `parser_size` is the one that matters:
        // portable-pty answers `pty_size` from a field it wrote itself during
        // the last resize, so on its own it only proves the call was made,
        // whereas the parser's size is what the widget renders from.
        let dir = TempDir::new("shell-size");
        let mut pane = pane(&dir, "cmd.exe", &[]);
        draw(&mut pane, 40, 8);
        assert_eq!(hosted(&pane).diagnostics().parser_size, (8, 40));
        assert_eq!(hosted(&pane).diagnostics().pty_size, Some((8, 40)));

        pane.on_resize(Rect::new(0, 0, 33, 11)).unwrap();
        assert_eq!(hosted(&pane).diagnostics().parser_size, (11, 33));
        assert_eq!(hosted(&pane).diagnostics().pty_size, Some((11, 33)));
    }

    #[test]
    fn a_child_that_has_gone_takes_nothing_down_with_it() {
        // Every call into the child is swallowed, and the reason is this exact
        // window: `try_wait` is polled once a loop, so a key can arrive while
        // the pane still believes a dead child is live. The other tests all go
        // through the `live() == None` branch, which is not the branch the
        // swallowing is in.
        let dir = TempDir::new("shell-raced");
        let mut pane = pane(&dir, "cmd.exe", &["/c", "exit"]);
        draw(&mut pane, 40, 8);

        // Gone, but deliberately never polled — so the pane is in the state it
        // would be in mid-loop, believing itself live.
        std::thread::sleep(Duration::from_millis(600));
        assert!(pane.is_live(), "nothing has told the pane yet, which is the point");

        assert_eq!(pane.handle_key(key(KeyCode::Char('x'))).unwrap(), Handled::Yes);
        assert_eq!(pane.handle_paste("git status\r").unwrap(), Handled::Yes);
        pane.handle_mouse(&wheel(MouseEventKind::ScrollUp)).unwrap();

        // And the one that used to be able to end the agent session in the
        // *other* pane: `App::draw` propagates what this returns.
        pane.on_resize(Rect::new(0, 0, 20, 5)).unwrap();
        wait_for_exit(&mut pane);
        pane.on_resize(Rect::new(0, 0, 25, 7)).unwrap();
    }

    #[test]
    fn the_exit_is_news_exactly_once() {
        // The frame this asks for is the only thing that puts "exited · enter
        // restarts" in the border. Reported forever, it would redraw the
        // agent's whole screen on every idle loop; reported never, the title
        // would still say `shell · cmd` over a dead pane until something else
        // happened to want a frame.
        let dir = TempDir::new("shell-news");
        let mut pane = pane(&dir, "cmd.exe", &["/c", "exit"]);
        draw(&mut pane, 40, 8);

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && !hosted(&pane).has_exited() {
            pane.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(hosted(&pane).has_exited(), "the child never exited");
        assert!(pane.title().starts_with("exited"));

        // Whatever output was still in flight has landed by now, so a further
        // `true` could only be the transition being reported twice.
        std::thread::sleep(Duration::from_millis(300));
        pane.tick();
        assert!(!pane.tick(), "a settled dead child is not news every loop");
    }

    #[test]
    fn enter_before_the_first_frame_waits_for_it_rather_than_spawning_at_one_column() {
        // `App` drains every pending event before drawing, so Alt+S and Enter
        // pressed together both arrive while this pane has never been sized. A
        // shell started at the 1x1 that gets clamped to has already reflowed
        // its banner into one column by the time a frame corrects it.
        let dir = TempDir::new("shell-early-enter");
        let mut pane = pane(&dir, "cmd.exe", &[]);
        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert!(matches!(pane.state, State::Cold), "nothing was started");

        // The frame that was always going to spawn it does so, at its size.
        draw(&mut pane, 40, 8);
        assert_eq!(hosted(&pane).diagnostics().parser_size, (8, 40));
    }

    #[test]
    fn alt_j_scrolls_the_pane_where_a_typed_down_is_still_the_shells() {
        // Alt+J arrives here as a bare `Down`. Forwarded to a live shell that
        // is its history key, so glancing at this pane would load an earlier
        // command into the prompt — silently, and while focus is elsewhere.
        let dir = TempDir::new("shell-glance");
        let mut pane = pane(&dir, "cmd.exe", &[]);
        draw(&mut pane, 40, 8);
        let sent = hosted(&pane).diagnostics().keys_sent;

        for code in [KeyCode::Down, KeyCode::Up, KeyCode::PageDown, KeyCode::PageUp] {
            pane.scroll_key(key(code)).unwrap();
        }
        assert_eq!(
            hosted(&pane).diagnostics().keys_sent,
            sent,
            "not one of them reached the child"
        );
        // Nothing has scrolled off yet, so none of them moved anything either,
        // and a key that moved nothing must not cost a frame.
        assert_eq!(pane.scroll_key(key(KeyCode::Down)).unwrap(), Handled::No);

        // ...whereas the same key typed into the focused pane is the child's.
        pane.handle_key(key(KeyCode::Down)).unwrap();
        assert!(hosted(&pane).diagnostics().keys_sent > sent);
    }

    #[test]
    fn output_that_scrolled_off_is_reachable_by_key_and_by_wheel() {
        let dir = TempDir::new("shell-history");
        let mut pane = with_history(&dir);
        assert_eq!(pane.at(), 0, "the view starts on the live screen");

        assert_eq!(pane.scroll_key(key(KeyCode::Up)).unwrap(), Handled::Yes);
        assert_eq!(pane.at(), 1);
        assert_eq!(pane.scroll_key(key(KeyCode::PageUp)).unwrap(), Handled::Yes);
        assert_eq!(pane.at(), 1 + 5, "a page keeps one row of overlap");
        pane.scroll_key(key(KeyCode::PageDown)).unwrap();
        assert_eq!(pane.at(), 1);

        // A plain shell asks for no mouse reports, so the notch is ours.
        assert_eq!(
            pane.handle_mouse(&wheel(MouseEventKind::ScrollUp)).unwrap(),
            Handled::Yes
        );
        assert_eq!(pane.at(), 1 + WHEEL);
        pane.handle_mouse(&wheel(MouseEventKind::ScrollDown)).unwrap();
        assert_eq!(pane.at(), 1);

        // Typing is a request to be at the prompt, and the prompt is at the
        // bottom. Anything else types into a screen you cannot see.
        pane.handle_key(key(KeyCode::Char('x'))).unwrap();
        assert_eq!(pane.at(), 0);
        assert_eq!(
            pane.scroll_key(key(KeyCode::Down)).unwrap(),
            Handled::No,
            "already live, so nothing moved"
        );
    }

    #[test]
    fn a_cursor_is_drawn_in_a_live_prompt_and_nowhere_else() {
        // The strongest focus signal there is: if it is not blinking in the
        // shell's prompt, your keys are not going to the shell.
        let dir = TempDir::new("shell-cursor");
        let mut pane = with_history(&dir);
        assert!(pane.cursor().is_some());

        // Scrolled far enough back, the live screen — and the prompt on it —
        // is off the bottom of the pane, so there is nothing to point at.
        pane.to(usize::MAX);
        assert_eq!(pane.cursor(), None);
        pane.to(0);
        assert!(pane.cursor().is_some());

        let mut dead = pane_that_exited(&dir);
        assert_eq!(dead.cursor(), None, "a dead child has no prompt");
        assert_eq!(dead.handle_paste("git status\r").unwrap(), Handled::No);
    }

    fn pane_that_exited(dir: &TempDir) -> ShellPane {
        let mut pane = pane(dir, "cmd.exe", &["/c", "exit"]);
        draw(&mut pane, 40, 8);
        wait_for_exit(&mut pane);
        pane
    }

    #[test]
    fn a_spawn_that_fails_is_a_message_naming_what_was_tried() {
        // `render` cannot return an error, so this has to be a state the pane
        // holds. The alternative is an empty box, which is exactly what a shell
        // that has not printed anything yet also looks like.
        let dir = TempDir::new("shell-missing");
        let mut pane = pane(&dir, "abeam-no-such-shell.exe", &[]);
        let screen = draw(&mut pane, 46, 12);

        assert!(matches!(pane.state, State::Failed { .. }));
        assert!(!pane.takes_input(), "there is nothing to type into");
        assert_eq!(pane.title(), "no shell · enter retries");
        // On screen, not merely in the struct — and naming the program, which
        // is the only thing that tells a reader whether ABEAM_SHELL is at fault.
        assert!(screen.contains("abeam-no-such-shell.exe"), "got: {screen}");
        assert!(screen.contains("ABEAM_SHELL"), "got: {screen}");

        // Enter retries, for the case where the fix was made outside abeam.
        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert!(matches!(pane.state, State::Failed { .. }));
    }

    #[test]
    fn the_child_starts_in_the_directory_abeam_was_pointed_at() {
        // The whole point of this pane over a second window: a `git status`
        // typed here answers about the repository on screen, not about wherever
        // abeam happened to be launched from.
        let dir = TempDir::new("shell-cwd");
        let mut pane = pane(&dir, "cmd.exe", &["/c", "cd"]);
        draw(&mut pane, 100, 8);
        wait_for_exit(&mut pane);
        // try_wait can answer while the last of the output is still in flight.
        std::thread::sleep(Duration::from_millis(200));

        let printed = hosted(&pane).last_screen().join("\n").to_lowercase();
        let want = dir.path().to_string_lossy().to_lowercase();
        assert!(printed.contains(&want), "expected {want:?} in {printed:?}");
    }

    #[test]
    fn an_explicit_program_is_a_choice_rather_than_a_first_preference() {
        // Falling back from ABEAM_SHELL would hide a typo in it behind a shell
        // nobody asked for, and the mistake would surface much later.
        let named = ShellPane::new(PathBuf::from("."), Some("nu.exe".to_string()));
        let only: Vec<&str> = named.candidates.iter().map(|c| c.program.as_str()).collect();
        assert_eq!(only, ["nu.exe"]);

        let searched = ShellPane::new(PathBuf::from("."), None);
        let order: Vec<&str> = searched
            .candidates
            .iter()
            .map(|c| c.program.as_str())
            .collect();
        assert_eq!(order, SHELLS, "best first, and the last one always exists");
    }

    // --- finding a shell, safely ------------------------------------------

    #[test]
    fn a_shell_the_first_candidate_cannot_supply_falls_through_to_the_next() {
        // The reason [`SHELLS`] is a list: PowerShell 7 is not part of Windows,
        // so on most machines the first name on it resolves to nothing. Every
        // other test here injects a single candidate, which is the one shape
        // that cannot show this.
        let dir = TempDir::new("shell-fallback");
        let mut pane = panes(&dir, &["abeam-no-such-shell.exe", "cmd.exe"]);
        draw(&mut pane, 40, 8);

        assert!(pane.is_live());
        assert_eq!(pane.title(), "shell · cmd", "the second candidate won");
    }

    #[test]
    fn a_search_that_finds_nothing_names_every_shell_it_looked_for() {
        // Which list this was is the whole diagnosis: one name means
        // ABEAM_SHELL is wrong, three means this is not the Windows anyone
        // expected — and the reader can only tell them apart by reading them.
        let dir = TempDir::new("shell-none");
        let missing = ["abeam-no-such-a.exe", "abeam-no-such-b.exe", "abeam-no-such-c.exe"];
        let mut pane = panes(&dir, &missing);
        let screen = draw(&mut pane, 46, 14);

        for name in missing {
            assert!(screen.contains(name), "{name} is missing from: {screen}");
        }
        assert!(screen.contains("not found on PATH"), "got: {screen}");
    }

    // The search that answers these lives in `crate::launch` and is tested
    // there, including the hole a bare name opens. What is asserted here is the
    // half this pane still owns: the table of where Windows keeps its shells,
    // and what the border says once one has started.

    #[test]
    fn the_shells_windows_ships_resolve_to_the_ones_windows_shipped() {
        // Not "resolve to something": to the copy under %SystemRoot%. `cmd.exe`
        // taken from PATH is `cmd.exe` taken from whatever put itself at the
        // front of PATH. `known_home` is the one thing the shared resolver is
        // told rather than works out, and this is why it exists.
        let system32 =
            PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32");
        let found = |name: &str| {
            launch::resolve_preferring(name, &[], known_home(name))
                .expect(name)
                .program
        };
        assert_eq!(found("cmd.exe"), system32.join("cmd.exe"));
        assert_eq!(
            found("powershell.exe"),
            system32
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        );
        // ...and whatever comes back is absolute, always, which is the property
        // the whole search is for.
        assert!(found("cmd.exe").is_absolute());
    }

    /// A child that has enabled bracketed paste, which is the mode
    /// [`ShellPane::send_command`] refuses to write without.
    ///
    /// Typed out of a *file* rather than asked for by the child, exactly as
    /// `crate::app`'s own fixture does it: `cmd.exe` has no line editor that
    /// would ever ask, so a test that waited for one would wait for ever. The
    /// pty forwards the bytes and the parser behind the pane picks the mode up,
    /// which is the same route a real PSReadLine's takes.
    fn asks_for_paste(dir: &TempDir) -> ShellPane {
        dir.write("bracketed.txt", b"\x1b[?2004h");
        pane(dir, "cmd.exe", &["/k", "type", "bracketed.txt"])
    }

    /// Poll until the child has enabled bracketed paste, or give up loudly.
    ///
    /// Its own wait rather than a predicate handed to [`settled`], because what
    /// it asks about is the parser and not the screen — and a closure reading
    /// the pane cannot be handed to a function already holding it mutably.
    fn wait_for_paste(pane: &mut ShellPane) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            pane.tick();
            if hosted(pane).bracketed_paste() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the child never asked for bracketed paste, so nothing would ever be sent");
    }

    /// Draw until what is on screen satisfies `enough`, or give up loudly.
    ///
    /// A bounded wait rather than a sleep, because what is being waited for is
    /// a real child echoing at a real pty and how long that takes is the
    /// machine's business. The panic carries the screen: a timeout with no
    /// account of what was actually drawn is a test somebody deletes.
    fn settled(pane: &mut ShellPane, what: &str, enough: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            pane.tick();
            let screen = draw(pane, 120, 12);
            if enough(&screen) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "waited twenty seconds for {what}. The screen says:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_handed_over_command_is_typed_at_the_prompt_and_nothing_submits_it() {
        // The whole promise of the ask pane's hand-off, asked of a real pty:
        // what ends up at the prompt is what was on the screen, and it is the
        // reader who runs it. Delete the `bracketed_paste` check and this still
        // passes; delete the missing newline and it fails on the count below,
        // which is the assertion this test exists for.
        let dir = TempDir::new("shell-handoff");

        // Cold: nothing to type into, and nothing started by trying. `App`
        // defers the hand-off a frame for exactly this — a pane spawns its
        // child on the frame that draws it, so the keystroke that chose the
        // command arrives before there is a child to give it to.
        let mut cold = pane(&dir, "cmd.exe", &[]);
        assert!(!cold.send_command("echo ZQXJ"));
        assert!(
            matches!(cold.state, State::Cold),
            "a refused hand-off started a shell"
        );

        // Live, and provably never going to ask for bracketed paste: refused,
        // because without that mode `send_text` writes raw bytes and a newline
        // among them would submit. `cmd.exe` is that child, which is why this
        // branch is a cost somebody pays rather than a hypothetical.
        let mut plain = pane(&dir, "cmd.exe", &[]);
        draw(&mut plain, 120, 12);
        settled(&mut plain, "the plain shell to print anything", |screen| {
            !screen.trim().is_empty()
        });
        assert!(!hosted(&plain).bracketed_paste());
        assert!(!plain.send_command("echo ZQXJ"));

        // And a child that did ask takes it.
        let mut pane = asks_for_paste(&dir);
        draw(&mut pane, 120, 12);
        wait_for_paste(&mut pane);
        assert!(pane.send_command("echo ZQXJ"));

        // At the prompt, once, and unrun. Twice would be the command *and* its
        // output, which is exactly what a newline sent along with it would have
        // produced — so the count is the assertion and `contains` is only what
        // makes the wait terminate.
        let typed = settled(&mut pane, "the command to appear", |screen| {
            screen.contains("echo ZQXJ")
        });
        assert_eq!(
            typed.matches("ZQXJ").count(),
            1,
            "the command was submitted rather than typed:\n{typed}"
        );

        // The reader's own `Enter` is what runs it, and this half is what
        // proves the text is really in the child's input rather than painted on
        // the screen by an echo abeam could have faked.
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        settled(&mut pane, "the command to run", |screen| {
            screen.matches("ZQXJ").count() >= 2
        });
    }

    #[test]
    fn a_shell_that_is_a_cmd_wrapper_starts_and_the_border_names_the_wrapper() {
        // `ABEAM_SHELL=…\nu.cmd` is the same wish as `abeam +claude` on an npm
        // install, so this pane routes scripts too rather than holding a
        // stricter contract of its own. What is specific to the pane is the
        // border: it has to name what the person chose, because the
        // interpreter is a detail of starting it and naming *that* would make
        // every wrapper on the machine look like the same shell.
        let dir = TempDir::new("shell-wrapper");
        let script = dir.write("abeam-wrapper.cmd", b"@echo off\r\ncmd.exe\r\n");
        let named = script.to_string_lossy().into_owned();
        let mut pane = pane(&dir, &named, &[]);
        draw(&mut pane, 40, 8);

        assert!(pane.is_live(), "the wrapper did not start: {}", pane.title());
        assert_eq!(pane.title(), "shell · abeam-wrapper");
    }

    // --- what a selection over this pane copies ----------------------------

    const WRAP_COLS: u16 = 30;
    const WRAP_ROWS: u16 = 14;

    /// A child that has printed a line too long for the pane, and stopped.
    ///
    /// The pane is deliberately narrow and the line deliberately longer than
    /// it: what is under test is a row and its continuation, which cannot be
    /// arranged without a wrap actually happening. Tall enough that a `cmd`
    /// prompt — an absolute path, over three of these columns — cannot push the
    /// output off the top before the test looks at it.
    fn with_a_long_line(dir: &TempDir, line: &str) -> ShellPane {
        dir.write("wide.txt", format!("head\r\n{line}\r\ntail\r\n").as_bytes());
        let mut pane = pane(dir, "cmd.exe", &["/k", "type", "wide.txt"]);
        draw(&mut pane, WRAP_COLS, WRAP_ROWS);

        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            pane.tick();
            let screen = draw(&mut pane, WRAP_COLS, WRAP_ROWS);
            if screen.contains("tail") {
                return pane;
            }
            assert!(
                Instant::now() < deadline,
                "the long line never arrived. The screen says:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_selection_rejoins_a_line_the_pane_was_too_narrow_for() {
        // The whole reason this pane answers `selected_text` at all rather than
        // letting the app read the rows back off the frame. A frame cannot tell
        // a wrapped row from a row that happens to be full, so a path or a URL
        // copied out of one would arrive at the agent with a newline through
        // the middle of it — which is worse than not copying it.
        let dir = TempDir::new("shell-wrap");
        let line = "the-quick-brown-fox/jumps/over/the/lazy/dog.rs";
        assert!(line.len() > WRAP_COLS as usize, "the fixture does not wrap");
        let pane = with_a_long_line(&dir, line);

        let text = pane
            .selected_text(0, WRAP_ROWS - 1)
            .expect("a hosted child says what is on its screen");
        assert!(
            text.lines().any(|row| row == line),
            "the wrapped line came back in pieces:\n{text}"
        );
        // And the rows either side are still rows: rejoining must not swallow a
        // boundary it was not asked about.
        assert!(text.lines().any(|row| row == "head"), "{text}");
        assert!(text.lines().any(|row| row == "tail"), "{text}");
    }

    #[test]
    fn a_selection_reads_the_rows_under_the_view_and_not_the_live_screen() {
        // The property that makes selecting worth anything in a shell: what you
        // scrolled back to is what you copy. Both `rows` and `contents_between`
        // walk `visible_rows`, which is what the scrollback offset moves — so
        // this asks whether the pane consults the parser rather than
        // remembering a screen.
        let dir = TempDir::new("shell-select-history");
        let mut pane = with_history(&dir);

        let live = pane.selected_text(0, 5).expect("a hosted child");
        pane.to(20);
        let back = pane.selected_text(0, 5).expect("a hosted child");
        assert_ne!(live, back, "scrolling back copied the live screen anyway");
        assert!(
            back.lines().any(|row| row.starts_with("line-")),
            "the history under the view is not what came back:\n{back}"
        );

        // A row past the bottom of the pty is clamped rather than refused: a
        // pane one row taller than its child, for the frame between a resize
        // and the pty being told, is an ordinary state.
        assert!(!pane.selected_text(0, u16::MAX).unwrap().is_empty());
        // And a span that begins past the end is empty rather than a panic.
        assert!(pane.selected_text(u16::MAX, u16::MAX).unwrap().is_empty());
    }
}

/// The same suite on Unix, question for question. A second module rather than a
/// `cfg` inside the first, because what differs is not the question but the
/// child: every test here needs a real one, and `cmd.exe /c exit` and `/bin/sh
/// -c exit` have no line in common. The three questions with no twin are the
/// three whose answers are Windows program names or a Windows directory.
///
/// The children are `/bin/sh`, by absolute path: bare when the test needs one
/// that stays, `-c` when it needs one that is already gone. Absolute so that a
/// failure here is a fact about this pane and not about the runner's `PATH`.
/// What is under test is what this pane does with a child, never what the child
/// prints — with the one exception of the working directory, which is only
/// observable by asking it.
#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use crate::testutil::TempDir;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::time::{Duration, Instant};

    /// The shell that is always there, and the one this module spawns.
    const SH: &str = "/bin/sh";

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn wheel(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// A pane that will spawn exactly this, bypassing the candidate search.
    /// Tests need a child that exits the moment it starts, and `ABEAM_SHELL`
    /// names a program rather than a command line.
    fn pane(dir: &TempDir, program: &str, args: &[&str]) -> ShellPane {
        ShellPane {
            root: dir.path().to_path_buf(),
            candidates: vec![Candidate {
                program: program.to_string(),
                args: args.iter().copied().map(String::from).collect(),
            }],
            state: State::Cold,
            drawn: Rect::ZERO,
        }
    }

    /// The same with a whole search list, for the two tests that are about the
    /// list rather than about what is on it.
    fn panes(dir: &TempDir, programs: &[&str]) -> ShellPane {
        ShellPane {
            root: dir.path().to_path_buf(),
            candidates: programs.iter().copied().map(Candidate::new).collect(),
            state: State::Cold,
            drawn: Rect::ZERO,
        }
    }

    /// Draw one frame at this size, which is also what starts the child.
    fn draw(pane: &mut ShellPane, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| pane.render(f, f.area())).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn hosted(pane: &ShellPane) -> &TerminalPane {
        match &pane.state {
            State::Hosted { term, .. } => term,
            _ => panic!("no child: {}", pane.title()),
        }
    }

    /// Poll until the child has been reaped. The pane never blocks on a child
    /// (`docs/conpty-findings.md`, constraint 2), so nor can the tests: there is
    /// no `wait` to call, which is why this is a loop.
    fn wait_for_exit(pane: &mut ShellPane) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            pane.tick();
            if hosted(pane).has_exited() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the child never exited");
    }

    /// A live child with plenty behind the screen, and finished producing it.
    ///
    /// Both halves are load-bearing. The tests below count rows, so waiting for
    /// merely *some* history lets a `PgUp` clamp against a scrollback that is
    /// still filling. And `vt100` advances the offset itself as rows arrive —
    /// on purpose, so that a reader who has scrolled back stays where they are
    /// looking — so a test that began while output was still coming would watch
    /// its own offsets move underneath it. Waiting for quiet buys both.
    ///
    /// `cat` of a file rather than a loop in the shell keeps the whole command
    /// out of the quoting rules, which are their own subject; `exec` at the end
    /// is what `cmd /k` is on the other side, leaving something alive to decline
    /// a wheel event.
    fn with_history(dir: &TempDir) -> ShellPane {
        /// Comfortably more than the deepest scroll any test here performs.
        const ENOUGH: usize = 20;

        // `\r\n` rather than `\n`, so that this does not depend on the line
        // discipline having `ONLCR` on: the parser is fed exactly what it needs
        // either way, and a doubled carriage return costs a column nobody reads.
        let body: String = (1..=60).map(|i| format!("line-{i}\r\n")).collect();
        dir.write("many.txt", body.as_bytes());
        let mut pane = pane(dir, SH, &["-c", "cat many.txt; exec /bin/sh"]);
        draw(&mut pane, 30, 6);

        let deadline = Instant::now() + Duration::from_secs(15);
        let (mut last, mut still) = (0, 0);
        while Instant::now() < deadline {
            pane.tick();
            let read = hosted(&pane).diagnostics().bytes_read;
            // How much history there is, asked by going as far back as the
            // parser will allow and reading where that landed.
            pane.to(usize::MAX);
            let depth = pane.at();
            pane.to(0);

            still = if depth >= ENOUGH && read == last { still + 1 } else { 0 };
            if still == 3 {
                return pane;
            }
            last = read;
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("a six-row pane fed sixty lines never settled with history behind it");
    }

    #[test]
    fn nothing_is_spawned_until_the_pane_is_drawn() {
        // A session that never presses Alt+S must never pay for a shell
        // process, and being drawn is the only signal a pane gets that it is
        // the one on screen.
        let dir = TempDir::new("shell-cold");
        let mut pane = pane(&dir, SH, &[]);
        assert!(matches!(pane.state, State::Cold));
        assert_eq!(pane.title(), "shell");
        assert!(!pane.takes_input());
        // ...and nothing here that quitting abeam would kill.
        assert!(!pane.is_live());
        assert!(!pane.tick());

        // A rect with nothing in it is not being on screen either.
        let mut term = Terminal::new(TestBackend::new(10, 4)).unwrap();
        term.draw(|f| pane.render(f, Rect::ZERO)).unwrap();
        assert!(matches!(pane.state, State::Cold));

        draw(&mut pane, 40, 8);
        assert!(matches!(pane.state, State::Hosted { .. }));
        assert!(pane.takes_input());
        assert_eq!(pane.title(), "shell · sh");
    }

    #[test]
    fn a_live_child_takes_esc_and_q_and_a_dead_one_leaves_them_to_the_shell() {
        let dir = TempDir::new("shell-esc");

        let mut live = pane(&dir, SH, &[]);
        draw(&mut live, 40, 8);
        assert!(live.takes_input(), "the border promises F4 as the way out");
        // The same fact the app reads before it lets abeam exit: quitting would
        // kill this child, so quitting has to ask first.
        assert!(live.is_live());
        let mut sent = hosted(&live).diagnostics().keys_sent;
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            assert_eq!(live.handle_key(key(code)).unwrap(), Handled::Yes);
            // `Yes` alone would still be reported by a `handle_key` that had
            // stopped writing to the child altogether — claiming the key is
            // only half of taking it.
            let now = hosted(&live).diagnostics().keys_sent;
            assert!(now > sent, "{code:?} was claimed but never sent");
            sent = now;
        }

        let mut dead = pane(&dir, SH, &["-c", "exit"]);
        draw(&mut dead, 40, 8);
        wait_for_exit(&mut dead);
        assert!(!dead.takes_input());
        assert!(!dead.is_live(), "nothing left for abeam to wait for");
        // ...so the app can read them as "give focus back to the agent", which
        // is the only route out of a pane that is no longer hosting anything.
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            assert_eq!(dead.handle_key(key(code)).unwrap(), Handled::No);
        }
    }

    #[test]
    fn enter_restarts_a_dead_child_and_the_title_says_so_first() {
        let dir = TempDir::new("shell-restart");
        let mut pane = pane(&dir, SH, &["-c", "exit"]);
        draw(&mut pane, 40, 8);
        wait_for_exit(&mut pane);

        let title = pane.title();
        assert!(title.starts_with("exited (0)"), "got: {title}");
        // A 46-column pane clips from the right, so the way back has to survive
        // the clip that loses the shell's name.
        assert!(title.contains("enter restarts"), "got: {title}");

        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert!(pane.takes_input(), "a fresh child takes typing again");
        assert_eq!(pane.title(), "shell · sh");
    }

    #[test]
    fn the_rect_the_pane_was_drawn_into_is_the_size_the_pty_is_told() {
        // One number, not two. A hosted shell wrapping in a different place
        // from the pane it is drawn in is the failure this rules out, and it is
        // ruled out by there being no second calculation to disagree.
        //
        // Both numbers are asserted, and `parser_size` is the one that matters:
        // portable-pty answers `pty_size` from a field it wrote itself during
        // the last resize, so on its own it only proves the call was made,
        // whereas the parser's size is what the widget renders from.
        let dir = TempDir::new("shell-size");
        let mut pane = pane(&dir, SH, &[]);
        draw(&mut pane, 40, 8);
        assert_eq!(hosted(&pane).diagnostics().parser_size, (8, 40));
        assert_eq!(hosted(&pane).diagnostics().pty_size, Some((8, 40)));

        pane.on_resize(Rect::new(0, 0, 33, 11)).unwrap();
        assert_eq!(hosted(&pane).diagnostics().parser_size, (11, 33));
        assert_eq!(hosted(&pane).diagnostics().pty_size, Some((11, 33)));
    }

    #[test]
    fn a_child_that_has_gone_takes_nothing_down_with_it() {
        // Every call into the child is swallowed, and the reason is this exact
        // window: `try_wait` is polled once a loop, so a key can arrive while
        // the pane still believes a dead child is live. The other tests all go
        // through the `live() == None` branch, which is not the branch the
        // swallowing is in.
        let dir = TempDir::new("shell-raced");
        let mut pane = pane(&dir, SH, &["-c", "exit"]);
        draw(&mut pane, 40, 8);

        // Gone, but deliberately never polled — so the pane is in the state it
        // would be in mid-loop, believing itself live.
        std::thread::sleep(Duration::from_millis(600));
        assert!(pane.is_live(), "nothing has told the pane yet, which is the point");

        assert_eq!(pane.handle_key(key(KeyCode::Char('x'))).unwrap(), Handled::Yes);
        assert_eq!(pane.handle_paste("git status\r").unwrap(), Handled::Yes);
        pane.handle_mouse(&wheel(MouseEventKind::ScrollUp)).unwrap();

        // And the one that used to be able to end the agent session in the
        // *other* pane: `App::draw` propagates what this returns.
        pane.on_resize(Rect::new(0, 0, 20, 5)).unwrap();
        wait_for_exit(&mut pane);
        pane.on_resize(Rect::new(0, 0, 25, 7)).unwrap();
    }

    #[test]
    fn the_exit_is_news_exactly_once() {
        // The frame this asks for is the only thing that puts "exited · enter
        // restarts" in the border. Reported forever, it would redraw the
        // agent's whole screen on every idle loop; reported never, the title
        // would still say `shell · sh` over a dead pane until something else
        // happened to want a frame.
        let dir = TempDir::new("shell-news");
        let mut pane = pane(&dir, SH, &["-c", "exit"]);
        draw(&mut pane, 40, 8);

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && !hosted(&pane).has_exited() {
            pane.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(hosted(&pane).has_exited(), "the child never exited");
        assert!(pane.title().starts_with("exited"));

        // Whatever output was still in flight has landed by now, so a further
        // `true` could only be the transition being reported twice.
        std::thread::sleep(Duration::from_millis(300));
        pane.tick();
        assert!(!pane.tick(), "a settled dead child is not news every loop");
    }

    #[test]
    fn enter_before_the_first_frame_waits_for_it_rather_than_spawning_at_one_column() {
        // `App` drains every pending event before drawing, so Alt+S and Enter
        // pressed together both arrive while this pane has never been sized. A
        // shell started at the 1x1 that gets clamped to has already reflowed
        // its banner into one column by the time a frame corrects it.
        let dir = TempDir::new("shell-early-enter");
        let mut pane = pane(&dir, SH, &[]);
        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert!(matches!(pane.state, State::Cold), "nothing was started");

        // The frame that was always going to spawn it does so, at its size.
        draw(&mut pane, 40, 8);
        assert_eq!(hosted(&pane).diagnostics().parser_size, (8, 40));
    }

    #[test]
    fn alt_j_scrolls_the_pane_where_a_typed_down_is_still_the_shells() {
        // Alt+J arrives here as a bare `Down`. Forwarded to a live shell that
        // is its history key, so glancing at this pane would load an earlier
        // command into the prompt — silently, and while focus is elsewhere.
        let dir = TempDir::new("shell-glance");
        let mut pane = pane(&dir, SH, &[]);
        draw(&mut pane, 40, 8);
        let sent = hosted(&pane).diagnostics().keys_sent;

        for code in [KeyCode::Down, KeyCode::Up, KeyCode::PageDown, KeyCode::PageUp] {
            pane.scroll_key(key(code)).unwrap();
        }
        assert_eq!(
            hosted(&pane).diagnostics().keys_sent,
            sent,
            "not one of them reached the child"
        );
        // Nothing has scrolled off yet, so none of them moved anything either,
        // and a key that moved nothing must not cost a frame.
        assert_eq!(pane.scroll_key(key(KeyCode::Down)).unwrap(), Handled::No);

        // ...whereas the same key typed into the focused pane is the child's.
        pane.handle_key(key(KeyCode::Down)).unwrap();
        assert!(hosted(&pane).diagnostics().keys_sent > sent);
    }

    #[test]
    fn output_that_scrolled_off_is_reachable_by_key_and_by_wheel() {
        let dir = TempDir::new("shell-history");
        let mut pane = with_history(&dir);
        assert_eq!(pane.at(), 0, "the view starts on the live screen");

        assert_eq!(pane.scroll_key(key(KeyCode::Up)).unwrap(), Handled::Yes);
        assert_eq!(pane.at(), 1);
        assert_eq!(pane.scroll_key(key(KeyCode::PageUp)).unwrap(), Handled::Yes);
        assert_eq!(pane.at(), 1 + 5, "a page keeps one row of overlap");
        pane.scroll_key(key(KeyCode::PageDown)).unwrap();
        assert_eq!(pane.at(), 1);

        // A plain shell asks for no mouse reports, so the notch is ours.
        assert_eq!(
            pane.handle_mouse(&wheel(MouseEventKind::ScrollUp)).unwrap(),
            Handled::Yes
        );
        assert_eq!(pane.at(), 1 + WHEEL);
        pane.handle_mouse(&wheel(MouseEventKind::ScrollDown)).unwrap();
        assert_eq!(pane.at(), 1);

        // Typing is a request to be at the prompt, and the prompt is at the
        // bottom. Anything else types into a screen you cannot see.
        pane.handle_key(key(KeyCode::Char('x'))).unwrap();
        assert_eq!(pane.at(), 0);
        assert_eq!(
            pane.scroll_key(key(KeyCode::Down)).unwrap(),
            Handled::No,
            "already live, so nothing moved"
        );
    }

    #[test]
    fn a_cursor_is_drawn_in_a_live_prompt_and_nowhere_else() {
        // The strongest focus signal there is: if it is not blinking in the
        // shell's prompt, your keys are not going to the shell.
        let dir = TempDir::new("shell-cursor");
        let mut pane = with_history(&dir);
        assert!(pane.cursor().is_some());

        // Scrolled far enough back, the live screen — and the prompt on it —
        // is off the bottom of the pane, so there is nothing to point at.
        pane.to(usize::MAX);
        assert_eq!(pane.cursor(), None);
        pane.to(0);
        assert!(pane.cursor().is_some());

        let mut dead = pane_that_exited(&dir);
        assert_eq!(dead.cursor(), None, "a dead child has no prompt");
        assert_eq!(dead.handle_paste("git status\r").unwrap(), Handled::No);
    }

    fn pane_that_exited(dir: &TempDir) -> ShellPane {
        let mut pane = pane(dir, SH, &["-c", "exit"]);
        draw(&mut pane, 40, 8);
        wait_for_exit(&mut pane);
        pane
    }

    #[test]
    fn a_spawn_that_fails_is_a_message_naming_what_was_tried() {
        // `render` cannot return an error, so this has to be a state the pane
        // holds. The alternative is an empty box, which is exactly what a shell
        // that has not printed anything yet also looks like.
        let dir = TempDir::new("shell-missing");
        let mut pane = pane(&dir, "abeam-no-such-shell", &[]);
        let screen = draw(&mut pane, 46, 12);

        assert!(matches!(pane.state, State::Failed { .. }));
        assert!(!pane.takes_input(), "there is nothing to type into");
        assert_eq!(pane.title(), "no shell · enter retries");
        // On screen, not merely in the struct — and naming the program, which
        // is the only thing that tells a reader whether ABEAM_SHELL is at fault.
        assert!(screen.contains("abeam-no-such-shell"), "got: {screen}");
        assert!(screen.contains("ABEAM_SHELL"), "got: {screen}");

        // Enter retries, for the case where the fix was made outside abeam.
        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert!(matches!(pane.state, State::Failed { .. }));
    }

    #[test]
    fn the_child_starts_in_the_directory_abeam_was_pointed_at() {
        // The whole point of this pane over a second window: a `git status`
        // typed here answers about the repository on screen, not about wherever
        // abeam happened to be launched from.
        let dir = TempDir::new("shell-cwd");
        let mut pane = pane(&dir, SH, &["-c", "pwd"]);
        draw(&mut pane, 100, 8);
        wait_for_exit(&mut pane);
        // try_wait can answer while the last of the output is still in flight.
        std::thread::sleep(Duration::from_millis(200));

        // Canonicalised, and not folded to one case the way the Windows twin
        // is: `pwd` in a shell that has just been given a directory answers
        // with the physical path, and a temporary directory can be reached
        // through a symlinked `/tmp`. Case is not the difference here — these
        // paths are compared exactly, because on this platform they are.
        let printed = hosted(&pane).last_screen().join("\n");
        let want = dir.path().canonicalize().expect("the temporary directory");
        let want = want.to_string_lossy();
        assert!(printed.contains(&*want), "expected {want:?} in {printed:?}");
    }

    #[test]
    fn an_explicit_program_is_a_choice_and_a_login_shell_is_only_a_preference() {
        // The distinction the whole module is built on, and the one thing that
        // would be easy to lose in a port: `ABEAM_SHELL` gets no fallback,
        // because falling back from it would hide a typo behind a shell nobody
        // asked for. `$SHELL` gets two, because nobody typed it at abeam — it
        // was set once, years ago, and a login shell uninstalled since must not
        // leave this pane with nothing in it.
        let named = ShellPane::new(PathBuf::from("."), Some("nu".to_string()));
        let only: Vec<&str> = named.candidates.iter().map(|c| c.program.as_str()).collect();
        assert_eq!(only, ["nu"]);

        // Handed in rather than exported. Two hundred tests share this
        // process's environment and run beside this one, so `$SHELL` is not a
        // variable any of them gets to write — which is the whole reason the
        // list is built by a function taking a value.
        assert_eq!(
            shells(Some(OsString::from("/usr/bin/fish"))),
            ["/usr/bin/fish", "bash", "sh"],
            "what the user already chose leads, and the name a Unix cannot be missing is last"
        );
        // Unset and set-to-empty are the same thing: neither names a program,
        // and passing "" on would ask the resolver to search for nothing.
        assert_eq!(shells(None), ["bash", "sh"]);
        assert_eq!(shells(Some(OsString::new())), ["bash", "sh"]);
        // And a `$SHELL` that is not text falls through to the same two. A Unix
        // path is bytes, so this is a value the environment really can hold;
        // the resolver takes a `String`, and a lossy rendering of these bytes
        // would name a *different* file. Dropping it is what the `into_string`
        // above is for, and without this line nothing would notice it becoming
        // a `to_string_lossy`.
        assert_eq!(shells(Some(OsString::from_vec(vec![0xff]))), ["bash", "sh"]);

        // ...and the constructor's `None` arm really does ask `preferred` for
        // that list, which is the coupling the Windows twin asserts outright
        // and this one cannot: `$SHELL` belongs to whoever is running the
        // suite, so the head of the list is not a thing to assert. The tail is.
        // Replace the arm with a list of abeam's own and this fails here rather
        // than in front of somebody whose login shell was ignored.
        let searched = ShellPane::new(PathBuf::from("."), None);
        let order: Vec<&str> = searched
            .candidates
            .iter()
            .map(|c| c.program.as_str())
            .collect();
        assert_eq!(
            order[order.len() - 2..],
            ["bash", "sh"],
            "the two names behind `$SHELL` are what `preferred` ends with"
        );
    }

    // --- finding a shell, safely ------------------------------------------

    #[test]
    fn a_shell_the_first_candidate_cannot_supply_falls_through_to_the_next() {
        // The reason the candidate list is a list: `$SHELL` can name a shell
        // that was uninstalled after it was set, so the first name on it
        // resolves to nothing and the pane must still open. Every other test
        // here injects a single candidate, which is the one shape that cannot
        // show this.
        let dir = TempDir::new("shell-fallback");
        let mut pane = panes(&dir, &["abeam-no-such-shell", SH]);
        draw(&mut pane, 40, 8);

        assert!(pane.is_live());
        assert_eq!(pane.title(), "shell · sh", "the second candidate won");
    }

    #[test]
    fn a_search_that_finds_nothing_names_every_shell_it_looked_for() {
        // Which list this was is the whole diagnosis: one name means
        // ABEAM_SHELL is wrong, the whole list means this is not the machine
        // anyone expected — and the reader can only tell them apart by reading
        // them.
        let dir = TempDir::new("shell-none");
        let missing = ["abeam-no-such-a", "abeam-no-such-b", "abeam-no-such-c"];
        let mut pane = panes(&dir, &missing);
        let screen = draw(&mut pane, 46, 14);

        for name in missing {
            assert!(screen.contains(name), "{name} is missing from: {screen}");
        }
        // That the reason from the resolver reached the screen at all, which is
        // the last of the three things this pane promises to say. The sentence
        // is `crate::launch`'s and is byte-identical on both platforms, so this
        // is asserted as tightly as its Windows twin asserts it: at 46 columns
        // it does not wrap, and a looser match would go green against wording
        // that no longer says a search happened.
        assert!(screen.contains("not found on PATH"), "got: {screen}");
    }

    /// A child that has enabled bracketed paste, and one that provably never
    /// will — the same pair `crate::app`'s fixture keeps and for the same
    /// reason.
    ///
    /// `cat` rather than a shell, and that is a decision rather than a shortage
    /// of ideas: whether a child asks for the mode is the whole subject of two
    /// assertions below, and `/bin/sh` is bash on some distributions and dash
    /// on others — bash's readline enables bracketed paste and dash has no line
    /// editor at all, so the answer would depend on which image CI pulled.
    /// `cat` has no prompt and no opinion about terminals: it copies bytes. So
    /// the mode is handed over in a file when it is wanted and simply absent
    /// when it is not.
    fn asks_for_paste(dir: &TempDir) -> ShellPane {
        dir.write("bracketed.txt", b"\x1b[?2004h");
        pane(dir, "/bin/cat", &["bracketed.txt", "-"])
    }

    fn never_asks(dir: &TempDir) -> ShellPane {
        dir.write("plain.txt", b"nothing here is an escape sequence\n");
        pane(dir, "/bin/cat", &["plain.txt", "-"])
    }

    /// Poll until the child has enabled bracketed paste, or give up loudly.
    fn wait_for_paste(pane: &mut ShellPane) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            pane.tick();
            if hosted(pane).bracketed_paste() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the child never asked for bracketed paste, so nothing would ever be sent");
    }

    /// Draw until what is on screen satisfies `enough`, or give up loudly.
    fn settled(pane: &mut ShellPane, what: &str, enough: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            pane.tick();
            let screen = draw(pane, 120, 12);
            if enough(&screen) {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "waited twenty seconds for {what}. The screen says:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_handed_over_command_is_typed_at_the_prompt_and_nothing_submits_it() {
        // The whole promise of the ask pane's hand-off, asked of a real pty:
        // what ends up at the prompt is what was on the screen, and it is the
        // reader who runs it. Delete the `bracketed_paste` check and this still
        // passes; delete the missing newline and it fails on the count below,
        // which is the assertion this test exists for.
        //
        // The echo is the tty's, not the child's: a pty is opened with `ECHO`
        // and `ICANON` on, so what is written appears immediately and `cat`
        // does not see it until a newline completes the line. Which is why the
        // second half below is the strong one — it is `cat` repeating the line,
        // and it cannot happen until somebody presses `Enter`.
        let dir = TempDir::new("shell-handoff");

        // Cold: nothing to type into, and nothing started by trying. `App`
        // defers the hand-off a frame for exactly this — a pane spawns its
        // child on the frame that draws it, so the keystroke that chose the
        // command arrives before there is a child to give it to.
        let mut cold = pane(&dir, SH, &[]);
        assert!(!cold.send_command("echo ZQXJ"));
        assert!(
            matches!(cold.state, State::Cold),
            "a refused hand-off started a shell"
        );

        // Live, and provably never going to ask for bracketed paste: refused,
        // because without that mode `send_text` writes raw bytes and a newline
        // among them would submit.
        let mut plain = never_asks(&dir);
        draw(&mut plain, 120, 12);
        settled(&mut plain, "the plain child to print anything", |screen| {
            screen.contains("nothing here is an escape sequence")
        });
        assert!(!hosted(&plain).bracketed_paste());
        assert!(!plain.send_command("echo ZQXJ"));

        // And a child that did ask takes it.
        let mut pane = asks_for_paste(&dir);
        draw(&mut pane, 120, 12);
        wait_for_paste(&mut pane);
        assert!(pane.send_command("echo ZQXJ"));

        // Once, and unrun. Twice would be the tty's echo *and* `cat` repeating
        // the completed line, which is exactly what a newline sent along with
        // it would have produced — so the count is the assertion and
        // `contains` is only what makes the wait terminate.
        let typed = settled(&mut pane, "the command to appear", |screen| {
            screen.contains("echo ZQXJ")
        });
        assert_eq!(
            typed.matches("ZQXJ").count(),
            1,
            "the command was submitted rather than typed:\n{typed}"
        );

        // The reader's own `Enter` is what completes the line, and this half is
        // what proves the text is really in the child's input rather than
        // painted on the screen by an echo abeam could have faked.
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        settled(&mut pane, "the line to come back", |screen| {
            screen.matches("ZQXJ").count() >= 2
        });
    }

    #[test]
    fn a_shell_that_is_a_script_wrapper_starts_and_the_border_names_the_wrapper() {
        // `ABEAM_SHELL=~/bin/nu-wrapper` is the same wish as `abeam +claude` on
        // an npm install, and here the kernel grants it: a `#!` line is its
        // business, so `launch::unix::into_launch` hands back a `Launch` whose
        // `program` and `target` are the same file and nothing is routed.
        //
        // What this pins is therefore the two things that are left — that a
        // script carrying the execute bit resolves and starts at all, and that
        // the border says its stem rather than the full path it was named by.
        // Not the regression its Windows twin exists for: replacing
        // `name_of(&launch.target)` with `launch.program` leaves this test and
        // the whole Unix suite green, because those two are never different
        // here. Naming the interpreter is only catchable where something routes
        // an interpreter, which is Windows.
        let dir = TempDir::new("shell-wrapper");
        // With the execute bit, because a script without one is not a program
        // and the resolver is right to say so.
        let script = dir.write_exec("abeam-wrapper", b"#!/bin/sh\nexec /bin/sh\n");
        let named = script.to_string_lossy().into_owned();
        let mut pane = pane(&dir, &named, &[]);
        draw(&mut pane, 40, 8);

        assert!(pane.is_live(), "the wrapper did not start: {}", pane.title());
        assert_eq!(pane.title(), "shell · abeam-wrapper");
    }

    // --- what a selection over this pane copies ----------------------------
    //
    // The Windows twins of these two carry the argument; what differs here is
    // the child and the fact that a `sh` prompt is one character rather than an
    // absolute path, so nothing is at risk of being pushed off the top.

    const WRAP_COLS: u16 = 30;
    const WRAP_ROWS: u16 = 14;

    /// A child that has printed a line too long for the pane, and stopped.
    fn with_a_long_line(dir: &TempDir, line: &str) -> ShellPane {
        // `\r\n` for `with_history`'s reason: the parser is fed what it needs
        // whether or not the line discipline has `ONLCR` on.
        dir.write("wide.txt", format!("head\r\n{line}\r\ntail\r\n").as_bytes());
        let mut pane = pane(dir, SH, &["-c", "cat wide.txt; exec /bin/sh"]);
        draw(&mut pane, WRAP_COLS, WRAP_ROWS);

        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            pane.tick();
            let screen = draw(&mut pane, WRAP_COLS, WRAP_ROWS);
            if screen.contains("tail") {
                return pane;
            }
            assert!(
                Instant::now() < deadline,
                "the long line never arrived. The screen says:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_selection_rejoins_a_line_the_pane_was_too_narrow_for() {
        // The whole reason this pane answers `selected_text` at all rather than
        // letting the app read the rows back off the frame: a frame cannot tell
        // a wrapped row from a row that happens to be full.
        let dir = TempDir::new("shell-wrap");
        let line = "the-quick-brown-fox/jumps/over/the/lazy/dog.rs";
        assert!(line.len() > WRAP_COLS as usize, "the fixture does not wrap");
        let pane = with_a_long_line(&dir, line);

        let text = pane
            .selected_text(0, WRAP_ROWS - 1)
            .expect("a hosted child says what is on its screen");
        assert!(
            text.lines().any(|row| row == line),
            "the wrapped line came back in pieces:\n{text}"
        );
        assert!(text.lines().any(|row| row == "head"), "{text}");
        assert!(text.lines().any(|row| row == "tail"), "{text}");
    }

    #[test]
    fn a_selection_reads_the_rows_under_the_view_and_not_the_live_screen() {
        // What you scrolled back to is what you copy, which is the property
        // that makes selecting worth anything in a shell.
        let dir = TempDir::new("shell-select-history");
        let mut pane = with_history(&dir);

        let live = pane.selected_text(0, 5).expect("a hosted child");
        pane.to(20);
        let back = pane.selected_text(0, 5).expect("a hosted child");
        assert_ne!(live, back, "scrolling back copied the live screen anyway");
        assert!(
            back.lines().any(|row| row.starts_with("line-")),
            "the history under the view is not what came back:\n{back}"
        );

        // Clamped rather than refused, and empty rather than a panic.
        assert!(!pane.selected_text(0, u16::MAX).unwrap().is_empty());
        assert!(pane.selected_text(u16::MAX, u16::MAX).unwrap().is_empty());
    }
}
