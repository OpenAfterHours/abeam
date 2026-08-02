//! The command view: a shell hosted in the right pane.
//!
//! What it is for is the round trip that otherwise costs a second window:
//! `git branch`, `uv run ruff format`, `cargo test` — run in the directory
//! abeam was pointed at, next to the Claude session that is about to be told
//! what they printed. `Alt+S` out, type, `Alt+S` home.
//!
//! What it deliberately is not is a multiplexer. There is one child, started
//! when the pane is first drawn and never restarted behind your back; there are
//! no tabs and no splits. Nothing started here outlives abeam either, and that
//! one is not free — `TerminateProcess` reaches a shell and not the `cargo
//! build` the shell started, so `abeam_pty::job` puts the whole tree in a job
//! object and closes it with the session. It also keeps no buffer of its own:
//! the history it scrolls through is the one the `vt100` parser behind the pty
//! already writes into, and this pane only moves the window onto it.
//!
//! ## Why this pane is different from the other three
//!
//! Git, files and diagnostics are read-only, which is what lets an unbound
//! keystroke in them be harmless. This one hosts a live child and takes every
//! key it is given, `Esc` and `q` included: those cannot mean "back to Claude"
//! while something inside the pane is listening for them.
//!
//! Which is why nothing here is a fact about the *type*. The app decides where
//! `Esc` goes from what [`Pane::handle_key`] returned, so a live child claims
//! it by reporting `Yes` and a dead one lets it through to mean what it means
//! everywhere else; and it takes what the border promises from
//! [`Pane::takes_input`], which is likewise a question about this instant. The
//! single frame on which that answer is stale is the first: the border is drawn
//! before `render`, and `render` is what spawns, so the frame that starts the
//! shell still advertises `esc→claude` — for the few milliseconds until the new
//! session's first output asks for another one.
//!
//! ## The contract
//!
//! - **Spawned on the first frame that draws it**, never at startup. Being
//!   drawn is the only signal a pane gets that it is the one on screen, and the
//!   viewer already uses it for exactly this. A session that never presses
//!   `Alt+S` must never have paid for a shell process.
//! - **Restartable.** A child that exits leaves the pane saying so, with
//!   `Enter` to start another. While it is dead the pane must *not* claim every
//!   key — `Esc` and `q` fall through so the way back to Claude is the one the
//!   rest of the app taught.
//! - **Sized from the rect it was drawn into**, through
//!   [`Pane::on_resize`], which the app calls after every frame.
//! - **`tick` must not block.** `try_wait`, never `wait`
//!   (`docs/conpty-findings.md`, constraint 2).

use std::path::{Path, PathBuf};

use abeam_pty::PtyConfig;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::pane::{Handled, Pane};
use crate::panes::TerminalPane;
use crate::text::{self, dim, err};

/// Candidate shells, best first. Tried in order at spawn time; the first one
/// that starts wins, so a machine without PowerShell 7 falls back rather than
/// showing an error nobody can act on.
pub const SHELLS: &[&str] = &["pwsh.exe", "powershell.exe", "cmd.exe"];

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
            Some(p) => vec![Candidate::new(p)],
            None => SHELLS.iter().copied().map(Candidate::new).collect(),
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

    /// Try each candidate in order and keep the first that starts.
    ///
    /// Takes `&self` and returns the state rather than assigning it, which is
    /// what lets the caller be `render` — a spawn in the middle of a frame
    /// cannot also be holding a mutable borrow of the thing it is drawing.
    fn start(&self, at: Rect) -> State {
        let mut why = String::new();
        for candidate in &self.candidates {
            // Resolved to an absolute path first, and the pty is never given
            // anything else. A bare name reaching `CreateProcessW` is a
            // vulnerability rather than a convenience — see [`resolve`].
            let exe = match resolve(&candidate.program) {
                Ok(exe) => exe,
                Err(reason) => {
                    why = reason;
                    continue;
                }
            };
            let cfg = PtyConfig::new(exe.to_string_lossy())
                .args(candidate.args.iter().cloned())
                .cwd(&self.root)
                .size(at.height.max(1), at.width.max(1));
            match TerminalPane::spawn_with(cfg) {
                // Named after what started, not after what was asked for. With
                // a resolution step in front those can differ, and the border
                // has one job: to say which program is taking the typing.
                Ok(term) => {
                    return State::Hosted {
                        name: name_of(&exe),
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
        // shell reads an unhandled one as "back to Claude", and that is the way
        // out the other three views taught.
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

    /// `Esc` belongs to the child, so the border names the key that does not.
    /// Once the child has exited it belongs to nobody, and the way out is the
    /// one every other view taught — which is the default, and saying it here
    /// would be one more place for the two to disagree.
    fn exit_hint(&self) -> &'static str {
        if self.is_live() {
            " · alt+s→claude"
        } else {
            " · esc→claude"
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
            // ended the Claude session in the other one and skipped the
            // transcript abeam prints on the way out. The left pane
            // propagating is right, because if Claude's pty cannot be resized
            // abeam is over; this pane is exactly where that stops being true.
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

/// The extensions `CreateProcessW` will start on its own.
///
/// Everything else `PATHEXT` lists — `.cmd`, `.bat`, `.ps1`, `.js` — needs an
/// interpreter named in front of it, and there is no flag that makes the API
/// supply one. A shell named as one of those is a spawn that fails with "%1 is
/// not a valid Win32 application" and no hint as to why, so it is turned away
/// here with a sentence instead.
const IMAGES: &[&str] = &["exe", "com"];

/// `PATHEXT`'s default, for the rare environment that does not set it. The
/// whole list, not an opinion about it: what abeam can *start* is [`IMAGES`],
/// and the two questions are answered separately on purpose.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Turn the name of a shell into an absolute path to something Windows can
/// start, or into the sentence explaining why there is not one.
///
/// This exists because handing a **bare name** to `CreateProcessW` is a
/// vulnerability rather than a convenience. portable-pty searches `PATH` for
/// one and, when nothing matches, passes the name straight through as
/// `lpApplicationName` — at which point Windows resolves it against the
/// *calling process's* current directory before it looks anywhere else.
/// `PtyConfig::cwd` has no bearing on that; the directory in question is the
/// one abeam itself is standing in, which is the repository. So a repository
/// containing a file called `pwsh.exe` would have run it, with the user's full
/// token, on the first `Alt+S` — on any machine without PowerShell 7, which is
/// precisely the machine the [`SHELLS`] fallback exists for. `main` now stands
/// in `%SystemRoot%` as well; this is the half of the fix that does not depend
/// on it, and either alone is sufficient.
///
/// Two narrower holes close with it. `std::env::split_paths` yields an *empty*
/// path for the `;;` or the trailing `;` that a `PATH` accumulates, and joining
/// a name onto that produces a relative path whose existence check tests the
/// current directory again — reachable even on a machine that has all three
/// shells. And an extensionless file is only taken once every `PATHEXT` match
/// has been tried, because an npm install puts `claude`, `claude.cmd` and
/// `claude.ps1` in one directory and the first of those is a POSIX shell
/// script.
fn resolve(program: &str) -> Result<PathBuf, String> {
    let named = Path::new(program);

    let found = if named.is_absolute() {
        // Probed rather than trusted: an `ABEAM_SHELL` pointing at something
        // that has since been uninstalled should arrive here as "not found"
        // rather than as whatever `CreateProcessW` makes of it.
        probe(named.parent().unwrap_or(named), &file_name_of(named))
    } else if named.parent().is_some_and(|p| !p.as_os_str().is_empty()) {
        // A relative path names a place, and the place it would name is
        // relative to the repository on screen — the one directory in this
        // whole question that somebody else gets to write to.
        return Err(format!(
            "`{program}` is a relative path, and abeam will not resolve one \
             here: it would be resolved against the repository on screen. \
             Give an absolute path, or a bare name to look up on PATH."
        ));
    } else {
        known_home(program)
            .filter(|home| home.is_file())
            .or_else(|| walk_path(program))
    };

    let Some(found) = found else {
        return Err(format!("`{program}` was not found on PATH."));
    };
    if !found
        .extension()
        .is_some_and(|e| IMAGES.iter().any(|image| e.eq_ignore_ascii_case(image)))
    {
        return Err(format!(
            "`{}` is a script rather than a program. Windows starts only .exe \
             and .com directly; a .cmd or a .ps1 needs a shell named in front \
             of it, and abeam has no way to know which one you meant.",
            found.display()
        ));
    }
    Ok(found)
}

/// Where Windows keeps the shells in [`SHELLS`], consulted before `PATH`.
///
/// `cmd.exe` and `powershell.exe` are operating-system components with exactly
/// one right answer, and taking that answer from `PATH` means taking it from
/// something a user, an installer, or a directory added to the front of the
/// list can reorder. PowerShell 7 is not part of Windows and its installer is
/// only *usually* here, so its entry is a first guess with the `PATH` walk
/// still behind it.
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

/// Look `name` up on `PATH`, and answer only with absolute paths.
fn walk_path(name: &str) -> Option<PathBuf> {
    walk(&std::env::var_os("PATH")?, name)
}

/// The search itself, over a `PATH` handed in rather than read.
///
/// Split out so that the entries this refuses to look in can be pinned by a
/// test without a test reaching for the process's environment or its current
/// directory — both of which are shared by every other test running beside it.
fn walk(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        // Drops both the empty entry a `;;` leaves behind and any genuinely
        // relative one. Joining a name onto either gives a *relative* path, and
        // checking whether that exists asks about the current directory — which
        // is the repository, and the whole thing this module exists to stop.
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| probe(&dir, name))
}

/// `name` inside `dir`, with whatever extension makes it startable.
fn probe(dir: &Path, name: &str) -> Option<PathBuf> {
    // `PATHEXT` matches before the bare file, deliberately, and this is the one
    // place the order matters: an npm install of Claude Code leaves `claude`,
    // `claude.cmd` and `claude.ps1` side by side, and the extensionless one is
    // a POSIX shell script. Taking it first — which is what portable-pty's own
    // search does — is a program that cannot start.
    pathext()
        .iter()
        .map(|ext| dir.join(format!("{name}{ext}")))
        .chain(std::iter::once(dir.join(name)))
        .find(|file| file.is_file())
}

/// `PATHEXT` spelled in lower case.
///
/// Windows sets it in capitals and matches file names without regard to case,
/// so the only thing the choice affects is the path abeam then *shows* — in the
/// message about a script it will not start, and in the pty diagnostics.
/// `claude.cmd` is what a reader has on disk; `claude.CMD` is a second thing to
/// wonder about.
fn pathext() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| DEFAULT_PATHEXT.to_string())
        .split(';')
        .filter(|ext| ext.starts_with('.'))
        .map(str::to_ascii_lowercase)
        .collect()
}

/// `C:\…\pwsh.exe` is what gets started; `pwsh` is what the border says.
fn name_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn stem_of(program: &str) -> String {
    name_of(Path::new(program)).to_ascii_lowercase()
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What the pane says when nothing would start.
///
/// It names every program that was tried, because the useful next move depends
/// entirely on which list this was: a single name means `ABEAM_SHELL` is wrong,
/// and all three of [`SHELLS`] means this is not the Windows anyone expected.
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

/// Everything here starts a real child in a real pty, so it is Windows-only
/// like the rest of the pty-backed suite — including the two tests that only
/// read a table, because the table is a list of Windows program names.
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
        assert!(live.takes_input(), "the border promises alt+s as the way out");
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
        // ...so the app can read them as "give focus back to Claude", which is
        // the only route out of a pane that is no longer hosting anything.
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

        // And the one that used to be able to end the Claude session in the
        // *other* pane: `App::draw` propagates what this returns.
        pane.on_resize(Rect::new(0, 0, 20, 5)).unwrap();
        wait_for_exit(&mut pane);
        pane.on_resize(Rect::new(0, 0, 25, 7)).unwrap();
    }

    #[test]
    fn the_exit_is_news_exactly_once() {
        // The frame this asks for is the only thing that puts "exited · enter
        // restarts" in the border. Reported forever, it would redraw Claude's
        // whole screen on every idle loop; reported never, the title would
        // still say `shell · cmd` over a dead pane until something else
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
    fn the_powershells_are_asked_not_to_print_a_banner_and_cmd_is_not() {
        assert_eq!(args_for("pwsh.exe"), ["-NoLogo"]);
        assert_eq!(args_for("powershell.exe"), ["-NoLogo"]);
        // An explicit ABEAM_SHELL is often a full path, and is still PowerShell.
        assert_eq!(
            args_for(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            ["-NoLogo"]
        );
        // Anything else is spawned bare: a flag a shell does not understand is
        // a shell that refuses to start.
        assert!(args_for("cmd.exe").is_empty());
        assert!(args_for("bash").is_empty());
        assert!(args_for("nu.exe").is_empty());
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

    #[test]
    fn a_shell_is_never_taken_from_the_directory_abeam_is_looking_at() {
        // The bug this whole resolution step exists for. portable-pty hands a
        // bare name it could not find on PATH to `CreateProcessW` unchanged,
        // and Windows resolves that against the *calling process's* current
        // directory — so a repository containing `pwsh.exe` used to be what ran
        // on the first Alt+S, on every machine without PowerShell 7.
        //
        // Demonstrated against a `PATH` handed in rather than by standing in
        // the planted directory: the current directory belongs to the whole
        // test binary, and two hundred other tests are running beside this one.
        // What has to be true is that no entry which could name the current
        // directory is ever looked in, and these are all of them.
        let dir = TempDir::new("shell-planted");
        dir.write("abeam-planted-shell.exe", b"MZ not really a program");
        let planted = "abeam-planted-shell.exe";

        for hostile in [";;", ";", ".", ".;", r".\tools", "..", ""] {
            assert_eq!(
                walk(std::ffi::OsStr::new(hostile), planted),
                None,
                "PATH {hostile:?} was searched, and it names the current directory"
            );
        }

        // ...and the planted file is genuinely there, so the `None`s above are
        // the filter doing its job rather than the file being absent.
        assert_eq!(
            walk(dir.path().as_os_str(), planted),
            Some(dir.path().join(planted)),
            "an absolute PATH entry is still searched"
        );

        // The end of the same story: nothing on PATH, so nothing is started —
        // where portable-pty would have passed the bare name through.
        assert!(resolve(planted).is_err());
    }

    #[test]
    fn a_relative_path_is_refused_rather_than_resolved() {
        // `ABEAM_SHELL=.\tools\sh.exe` would be resolved against the repository
        // on screen, which is the one directory in this question somebody else
        // gets to write to. Refusing says so; resolving would not.
        let refused = resolve(r".\tools\sh.exe").expect_err("a relative path is not a shell");
        assert!(refused.contains("relative"), "got: {refused}");
        assert!(refused.contains("absolute"), "the way out has to be named");
    }

    #[test]
    fn a_script_is_refused_with_the_reason_rather_than_a_win32_error() {
        // `CreateProcessW` cannot start a .cmd or a .ps1 at all, and says so as
        // "%1 is not a valid Win32 application", which names neither the file
        // nor the problem.
        let dir = TempDir::new("shell-script");
        let script = dir.write("wrapper.cmd", b"@echo off\r\n");
        let refused =
            resolve(&script.to_string_lossy()).expect_err("a batch file is not a shell");
        assert!(refused.contains("script"), "got: {refused}");
        assert!(refused.contains("wrapper.cmd"), "got: {refused}");
    }

    #[test]
    fn the_shells_windows_ships_resolve_to_the_ones_windows_shipped() {
        // Not "resolve to something": to the copy under %SystemRoot%. `cmd.exe`
        // taken from PATH is `cmd.exe` taken from whatever put itself at the
        // front of PATH.
        let system32 = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join("System32");
        assert_eq!(resolve("cmd.exe").unwrap(), system32.join("cmd.exe"));
        assert_eq!(
            resolve("powershell.exe").unwrap(),
            system32
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        );
        // ...and whatever comes back is absolute, always, which is the property
        // the whole module is for.
        assert!(resolve("cmd.exe").unwrap().is_absolute());
    }

    #[test]
    fn an_extensionless_script_never_wins_over_an_executable_beside_it() {
        // An npm install of Claude Code drops `claude`, `claude.cmd` and
        // `claude.ps1` in one directory, and the extensionless one is a POSIX
        // shell script `CreateProcessW` cannot run. portable-pty's own search
        // checks the exact name first and takes it.
        let dir = TempDir::new("shell-pathext");
        dir.write("abeam-probe", b"#!/bin/sh\n");
        dir.write("abeam-probe.exe", b"MZ");
        assert_eq!(
            probe(dir.path(), "abeam-probe"),
            Some(dir.path().join("abeam-probe.exe"))
        );

        // With no executable beside it, the script is still what is there —
        // `resolve` is what turns that into a sentence rather than a spawn.
        let only = TempDir::new("shell-pathext-only");
        only.write("abeam-probe", b"#!/bin/sh\n");
        assert_eq!(
            probe(only.path(), "abeam-probe"),
            Some(only.path().join("abeam-probe"))
        );
    }
}
