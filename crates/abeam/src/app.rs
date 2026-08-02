//! The shell: layout, focus, and the event loop.
//!
//! Two rules run through all of it.
//!
//! **Typing goes to Claude.** It is over 95% of keystrokes, so it is the state
//! the app lives in and the one it must never leave by accident. Nothing moves
//! focus implicitly — not the file watcher, not a view switch, not a git
//! refresh. Only `Alt+Left`/`Alt+Right`, a mouse click, or `Esc` from the right
//! pane.
//!
//! **Reading the right pane costs nothing.** Switching views and scrolling both
//! work while Claude still has focus. Focus is needed only to drive a
//! selection. A two-pane multiplexer that makes you switch modes to *look* at
//! something has already lost.
//!
//! The third thing the shell does, which neither pane can do for itself, is
//! carry messages between them: the watcher's events go out to both, and a file
//! chosen in the git view opens in the viewer. Panes are deliberately ignorant
//! of each other — that is what makes them individually testable — so every
//! wire between them is here, in [`App::pump`].

use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;

use abeam_pty::ExitStatus;
use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use crate::keys::{self, Action};
use crate::layout as abeam_layout;
use crate::pane::{Focus, Pane};
use crate::panes::{DiagPane, GitPane, RightView, ShellPane, TerminalPane, ViewerPane};
use crate::watch::Watch;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub enum Outcome {
    /// The child finished. `screen` is what its last frame said — printed to
    /// the *primary* buffer by `main`, because leaving the alternate screen
    /// throws away everything drawn inside it, and a session whose transcript
    /// vanishes on `/exit` is a worse terminal than the one abeam replaced.
    Exited {
        status: ExitStatus,
        screen: Vec<String>,
    },
    Detached,
}

enum Flow {
    /// Keep going. `redraw` is whether anything the next frame would show has
    /// actually changed — the shell draws on it, so a key a pane declined
    /// costs nothing.
    Continue { redraw: bool },
    Quit,
}

impl Flow {
    fn idle() -> Flow {
        Flow::Continue { redraw: false }
    }

    fn redraw() -> Flow {
        Flow::Continue { redraw: true }
    }
}

pub struct App {
    left: TerminalPane,
    git: GitPane,
    viewer: ViewerPane,
    shell: ShellPane,
    diag: DiagPane,
    right_view: RightView,
    /// What F2 puts back. Never `Diag`.
    last_workspace_view: RightView,
    /// One watcher for the whole app; the shell splits its output between the
    /// two panes that care. `None` if the platform would not watch, in which
    /// case both panes fall back to their own refresh.
    watch: Option<Watch>,
    focus: Focus,
    zoom: bool,
    help: bool,
    /// The next keystroke bypasses every abeam binding. See `keys::Action`.
    literal_next: bool,
    /// Quitting kills a live session, so it asks twice. One bit rather than a
    /// modal dialog: any other key cancels it, which is the whole interaction.
    pending_quit: bool,
    /// Claude's exit, and the screen it left behind, held until abeam is
    /// actually willing to go. Normally that is immediately; with a command
    /// still running in the shell view it is when the user says so.
    claude_exit: Option<(ExitStatus, Vec<String>)>,
    /// Whichever pane owned the last mouse press keeps drag and motion events
    /// even once the pointer leaves it. Without this, dragging a selection in
    /// Claude and crossing the divider silently retargets mid-gesture.
    mouse_owner: Option<Focus>,
    /// Stashed by the last frame. Panes are sized from exactly the rects that
    /// were drawn, so the two can never disagree.
    left_inner: Rect,
    right_inner: Option<Rect>,
    /// The whole window, as of the last frame. Kept because the split is a pure
    /// function of it: a key can ask whether there *would* be a right pane
    /// without waiting for a frame to find out.
    area: Rect,
}

impl App {
    pub fn new(left: TerminalPane, root: PathBuf) -> Self {
        let watch = Watch::start(&root);
        let mut viewer = ViewerPane::new(root.clone());
        // Told rather than discovered, so a pane that will never update says so
        // on screen instead of looking like one that simply never notices.
        viewer.set_watching(watch.is_some());

        Self {
            left,
            git: GitPane::new(root.clone()),
            viewer,
            // No child yet. It is spawned by the first frame that draws it, so
            // a session that never asks for a command line never pays for one.
            shell: ShellPane::new(root, std::env::var("ABEAM_SHELL").ok()),
            diag: DiagPane::new(),
            right_view: RightView::Git,
            last_workspace_view: RightView::Git,
            watch,
            focus: Focus::Left,
            zoom: false,
            help: false,
            literal_next: false,
            pending_quit: false,
            claude_exit: None,
            mouse_owner: None,
            left_inner: Rect::ZERO,
            right_inner: None,
            area: Rect::ZERO,
        }
    }

    pub fn run(mut self, terminal: &mut Tui) -> Result<Outcome> {
        self.draw(terminal)?;

        loop {
            let mut redraw = false;

            if self.claude_exit.is_none()
                && let Some(status) = self.left.poll_exit()?
            {
                // try_wait can report an exit while the last of the output is
                // still in flight. Let the reader drain, then take the screen
                // it drained into — that is what makes the wait worth 50 ms.
                std::thread::sleep(Duration::from_millis(50));
                let screen = self.left.last_screen();
                self.claude_exit = Some((status, screen));
                // The left title now says the session has ended, and on the
                // path where abeam stays up that is the only thing announcing
                // it.
                redraw = true;
            }

            // Claude leaving normally ends abeam with it — that is what abeam
            // is. The exception is an open shell session: leaving kills it, and
            // killing someone's `cargo build` because the *other* pane finished
            // is not a decision this program gets to make on its own. So it
            // waits, says so in the title, and Alt+Q is the answer.
            //
            // "Open", not "busy", and the difference is worth knowing: ConPTY
            // cannot be asked whether a command is running, so a shell sitting
            // at a prompt holds the door exactly as a build does. The cost is
            // that pressing Alt+S once, early, changes how the session ends —
            // which is why the title names the shell rather than just saying
            // abeam is still here.
            if self.claude_exit.is_some() && !self.shell.is_live() {
                return Ok(self.finish());
            }

            if event::poll(Duration::from_millis(10))? {
                // Drain everything pending before drawing. Windows floods
                // Resize events during a window drag and ConPTY resize is the
                // flakiest operation in the stack; one batch is one resize.
                loop {
                    match self.handle_event(event::read()?)? {
                        Flow::Quit => return Ok(self.finish()),
                        Flow::Continue { redraw: wanted } => redraw |= wanted,
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }

            redraw |= self.tick_panes();
            redraw |= self.pump();

            if redraw {
                self.draw(terminal)?;
            }
        }
    }

    /// Every pane's periodic work, and the one place that decides whether any
    /// of it was worth a frame.
    ///
    /// All of them tick whether or not they are visible: the watcher has to
    /// notice new markdown while the git view is showing, and a child has to be
    /// able to exit behind one.
    ///
    /// What differs is whose news earns a redraw. For the three read-only views
    /// news is rare and cheap. The shell's is neither: a `cargo build` running
    /// behind the git view makes that pane dirty on almost every pass of this
    /// loop, and honouring it would re-render Claude's entire screen at the
    /// poll rate to show nobody anything. Its output only counts while it is
    /// the view on screen — and switching back to it redraws on the keystroke
    /// that switches, so nothing is missed.
    fn tick_panes(&mut self) -> bool {
        let mut redraw = false;
        redraw |= self.left.tick();
        redraw |= self.git.tick();
        redraw |= self.viewer.tick();

        let shell_dirty = self.shell.tick();
        redraw |= shell_dirty && self.right_view == RightView::Shell;

        redraw
    }

    /// What to report on the way out.
    ///
    /// `Alt+Q` after Claude has already gone is still Claude's exit — it is the
    /// same session ending, delayed by however long the shell was busy — and
    /// reporting it as a detach would throw away both the transcript `main`
    /// prints and the status code anything scripting abeam reads.
    fn finish(&mut self) -> Outcome {
        match self.claude_exit.take() {
            Some((status, screen)) => Outcome::Exited { status, screen },
            None => Outcome::Detached,
        }
    }

    /// Everything the panes cannot say to each other. Runs once per loop
    /// iteration, and does no work at all when the watcher is quiet.
    ///
    /// This is the reason abeam exists rather than three windows: an agent
    /// writes a file, and the git view and the document view both already know.
    fn pump(&mut self) -> bool {
        let mut redraw = false;

        if let Some(change) = self.watch.as_ref().map(Watch::drain)
            && !change.is_empty()
        {
            if change.worktree {
                // Coalescing is the pane's, and it is deliberate: a burst of
                // saves costs one extra refresh rather than one per file.
                self.git.request_refresh();
            }
            for path in change.markdown {
                // Queued, never shown from here. The viewer takes it up on the
                // frame it is actually drawn, so nothing pulls the pane out
                // from under someone reading git.
                self.viewer.follow(path);
            }
            // The queue changed even if no pane's content did: the border's
            // unread mark is drawn from it.
            redraw = true;
        }

        // Enter in the git view. Draining unconditionally matters — a request
        // left sitting fires late, at whatever unrelated moment next reads it.
        if let Some(path) = self.git.take_open_request() {
            self.viewer.show(path);
            // Switching views here is right where the watcher switching would
            // be wrong: this one is a key the user pressed asking for exactly
            // this. Focus stays on the right pane, where they already were.
            self.set_right_view(RightView::Viewer);
            redraw = true;
        }

        redraw
    }

    // --- events ----------------------------------------------------------

    fn handle_event(&mut self, ev: Event) -> Result<Flow> {
        match ev {
            Event::Key(key) => {
                // Windows sends Press *and* Release for every key. `encode_key`
                // drops releases, but the shell matches its own bindings before
                // reaching that — without this line Alt+G would toggle to git
                // and straight back, and Alt+Q would skip its confirmation.
                if key.kind == KeyEventKind::Release {
                    return Ok(Flow::idle());
                }
                self.handle_key(key)
            }
            Event::Paste(text) => {
                // Offered to the focused pane unconditionally. The read-only
                // views decline by returning `No` — which is the same mechanism
                // every other event uses, and it leaves room for a pane that
                // takes a paste into a filter box without claiming to be a
                // terminal.
                let handled = match self.focus {
                    Focus::Left => self.left.handle_paste(&text)?,
                    Focus::Right => self.right_pane().handle_paste(&text)?,
                };
                // A paste is a keystroke as far as the confirmation is
                // concerned: "any other key cancels it" has to include the ones
                // that do not arrive as keys, or Alt+Q, paste, Alt+Q quits.
                self.pending_quit = false;
                Ok(Flow::Continue {
                    redraw: handled.is_yes(),
                })
            }
            Event::Mouse(me) => {
                self.handle_mouse(me)?;
                Ok(Flow::redraw())
            }
            // The panes are resized from the rects the next frame draws, which
            // coalesces a drag into a single ConPTY resize — but there does
            // have to *be* a next frame.
            Event::Resize(_, _) => Ok(Flow::redraw()),
            _ => Ok(Flow::idle()),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Flow> {
        if std::mem::take(&mut self.literal_next) {
            // To whichever pane has focus, not to Claude. The hatch exists so
            // abeam can never permanently shadow a binding of the program you
            // are typing at, and once the right pane can host a shell that is
            // two programs. Sending it left regardless would deliver a keystroke
            // into Claude while the user is looking at the shell they aimed it
            // at, invisibly.
            match self.focus {
                Focus::Left => self.left.handle_key(key)?,
                Focus::Right => self.right_pane().handle_key(key)?,
            };
            return Ok(Flow::redraw());
        }

        let confirming = std::mem::take(&mut self.pending_quit);
        // Any key at all dismisses the help overlay: it says "any key to
        // dismiss" on it, and a reader who has started pressing keys has
        // stopped reading. Cleared *before* the bindings are matched, or Alt+G
        // would draw the overlay back over the git view it just asked for.
        let was_helping = std::mem::take(&mut self.help);

        if let Some(action) = keys::global(&key) {
            return self.act(action, confirming, was_helping);
        }

        match self.focus {
            Focus::Left => {
                self.left.handle_key(key)?;
                Ok(Flow::redraw())
            }
            Focus::Right => {
                if self.right_pane().handle_key(key)?.is_yes() {
                    return Ok(Flow::redraw());
                }
                // A right pane that does not want Esc or q is telling us the
                // user is done with it — and `Handled` is the whole of that
                // question. A live shell claims both keys by returning `Yes`
                // and never reaches here; a shell whose child has exited
                // declines them and lands back on Claude, which is the way out
                // the other three views taught. A second predicate asking the
                // pane's *type* whether it takes typing would have to be kept
                // in sync with what its `handle_key` actually did, and would be
                // wrong for exactly the states that matter: a dead child, and a
                // read-only pane with a filter box open.
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.focus = Focus::Left;
                    return Ok(Flow::redraw());
                }
                // Nothing came of it — `j` at the end of a document, a letter
                // this pane has no use for. The only thing that could still
                // need a frame is the overlay this keystroke dismissed.
                Ok(Flow::Continue {
                    redraw: was_helping,
                })
            }
        }
    }

    fn act(&mut self, action: Action, confirming: bool, was_helping: bool) -> Result<Flow> {
        match action {
            Action::Quit => {
                // Straight out when nothing would be killed by leaving. A shell
                // still running a command counts, even once Claude has gone —
                // that is the whole reason abeam is still on screen.
                if confirming || (self.left.has_exited() && !self.shell.is_live()) {
                    return Ok(Flow::Quit);
                }
                self.pending_quit = true;
            }
            // Direct selection, not a cycle: Alt+G always means "git is now
            // showing", whatever was there.
            Action::ShowGit => self.set_right_view(RightView::Git),
            Action::ShowViewer => {
                // Pressing it again while the viewer is already up would
                // otherwise be a key that does nothing. It used to reload;
                // reload is what `r` does from inside the pane and what the
                // watcher does unasked, so the second press is now the way to
                // the file list — the fastest route to a file nothing has
                // pointed the pane at.
                if self.right_view == RightView::Viewer {
                    self.viewer.toggle_browse();
                }
                self.set_right_view(RightView::Viewer);
            }
            Action::ShowShell => {
                // The one action that moves focus, because a command line you
                // have to press a second key to type into is not a command
                // line. Pressed again from inside, it is the way home — so the
                // whole round trip for `git branch` is Alt+S, type, Alt+S.
                if self.right_view == RightView::Shell && self.focus == Focus::Right {
                    self.focus = Focus::Left;
                } else {
                    self.set_right_view(RightView::Shell);
                    // Asked of the layout rather than of the last frame.
                    // `right_inner` is a frame behind, and on exactly this key
                    // it is behind in the way that matters: `set_right_view`
                    // has just un-zoomed, so the pane that is about to exist
                    // does not exist yet. Taking focus optimistically and
                    // letting the frame correct it is not good enough either —
                    // the loop drains every pending event before drawing, so
                    // `Alt+S` followed by a typed command in the same batch
                    // would route those keys at a pane that will never appear.
                    if abeam_layout::split(self.area, self.zoom).right.is_some() {
                        self.focus = Focus::Right;
                    }
                }
            }
            Action::ToggleDiag => {
                let target = if self.right_view == RightView::Diag {
                    self.last_workspace_view
                } else {
                    RightView::Diag
                };
                // A question about the pty is usually asked while typing into
                // it, so the instrument does not take focus.
                self.set_right_view(target);
            }

            Action::FocusLeft => self.focus = Focus::Left,
            Action::FocusRight => {
                if self.right_inner.is_some() {
                    self.focus = Focus::Right;
                }
            }
            Action::ScrollRight(code) => {
                // Delivered as the bare key the pane would have seen had it
                // been focused, so panes implement one scroll vocabulary — but
                // through `scroll_key`, so a pane with a child in it can tell
                // this apart from the same key typed at the child.
                let key = KeyEvent::new(code, KeyModifiers::NONE);
                self.right_pane().scroll_key(key)?;
            }
            Action::ToggleZoom => {
                self.zoom = !self.zoom;
                if self.zoom {
                    self.focus = Focus::Left;
                }
            }
            // Every other binding has already cleared it; this is the one that
            // brings it back.
            Action::ToggleHelp => self.help = !was_helping,
            Action::LiteralNext => self.literal_next = true,
        }
        Ok(Flow::redraw())
    }

    fn handle_mouse(&mut self, me: MouseEvent) -> Result<()> {
        let press = matches!(me.kind, MouseEventKind::Down(_));
        let release = matches!(me.kind, MouseEventKind::Up(_));

        let target = match self.mouse_owner {
            Some(owner) => Some(owner),
            None if hit(self.left_inner, &me) => Some(Focus::Left),
            None => match self.right_inner {
                Some(r) if hit(r, &me) => Some(Focus::Right),
                _ => None,
            },
        };
        let Some(target) = target else { return Ok(()) };

        if press {
            // Click to focus. Wheel deliberately does not — the whole point of
            // scrolling the right pane is that it does not disturb typing.
            self.focus = target;
            self.mouse_owner = Some(target);
            // And a click cancels a pending quit, for the same reason any other
            // key does: the user has moved on to something else.
            self.pending_quit = false;
        }

        match target {
            Focus::Left => {
                let ev = relative(&me, self.left_inner);
                self.left.handle_mouse(&ev)?;
            }
            Focus::Right => {
                if let Some(r) = self.right_inner {
                    let ev = relative(&me, r);
                    self.right_pane().handle_mouse(&ev)?;
                }
            }
        }

        if release {
            self.mouse_owner = None;
        }
        Ok(())
    }

    // --- drawing ---------------------------------------------------------

    fn draw(&mut self, terminal: &mut Tui) -> Result<()> {
        terminal.draw(|f| self.ui(f))?;

        // Sized from the rect that was just drawn, unconditionally, once per
        // frame. `on_resize` is a no-op when nothing changed, which is what
        // makes calling it every frame the cheap option rather than the
        // careless one. The views without a pty behind them ignore it and
        // learn their size inside `render`, from the same rect.
        let left_inner = self.left_inner;
        self.left.on_resize(left_inner)?;
        if let Some(right) = self.right_inner {
            self.right_pane().on_resize(right)?;
        }
        Ok(())
    }

    fn ui(&mut self, f: &mut Frame) {
        self.area = f.area();
        let split = abeam_layout::split(f.area(), self.zoom);
        self.left_inner = abeam_layout::inner(split.left);
        self.right_inner = split.right.map(abeam_layout::inner);

        // The right pane can vanish under a narrow window while focused.
        if self.right_inner.is_none() {
            self.focus = Focus::Left;
        }

        let left_focused = self.focus == Focus::Left;
        let left_title = if self.pending_quit {
            format!(" {} · Alt+Q again to quit ", self.left.title())
        } else if self.claude_exit.is_some() {
            // This is abeam outliving the session it exists for, which happens
            // only because a shell is open. Naming it matters: without that
            // word the window looks stuck, and the one thing the user needs to
            // know is that something of theirs is still alive in the other pane.
            format!(" {} · shell open · Alt+Q to quit ", self.left.title())
        } else {
            format!(" {} ", self.left.title())
        };
        f.render_widget(block(&left_title, left_focused), split.left);
        let left_inner = self.left_inner;
        self.left.render(f, left_inner);

        if let (Some(outer), Some(inner)) = (split.right, self.right_inner) {
            let focused = self.focus == Focus::Right;
            // The instrument reads the terminal pane, so it is refreshed from
            // here rather than holding a borrow of it. Only on the frames that
            // show it: `pty_size()` asks the pty, and nothing else needs to.
            if self.right_view == RightView::Diag {
                let state = self.left.diagnostics();
                self.diag.update(state);
            }
            f.render_widget(block_line(self.right_title(focused), focused), outer);
            self.right_pane().render(f, inner);
        }

        // The real cursor sits in whichever focused pane has one — Claude, or
        // the shell view. It is the strongest focus signal there is, because it
        // is what a typist is already looking at, and it costs no screen space.
        // The read-only views have nothing to point at and say so by returning
        // `None`, which is also what hides it while they are up.
        let (rect, at) = match self.focus {
            Focus::Left => (self.left_inner, self.left.cursor()),
            Focus::Right => match self.right_inner {
                Some(r) => (r, self.right_pane_ref().cursor()),
                None => (Rect::ZERO, None),
            },
        };
        if let Some((col, row)) = at
            && rect.width > 0
            && rect.height > 0
        {
            // Clamped: vt100 can report a cursor one past the last column.
            f.set_cursor_position((
                rect.x + col.min(rect.width - 1),
                rect.y + row.min(rect.height - 1),
            ));
        }

        if self.help {
            help_overlay(f);
        }
    }

    /// The right pane's border text.
    ///
    /// Hints live in the border, not a status bar: rows are the scarcest
    /// resource in a two-pane TUI and Claude's UI is hungry for them.
    ///
    /// The unread mark goes *first* because titles are clipped from the right.
    /// A git title with a branch name and a change count already fills a 46
    /// column pane, so a mark appended to it would be invisible exactly when
    /// the repository is busy — which is exactly when new documents appear.
    fn right_title(&self, focused: bool) -> Line<'static> {
        let mut spans = vec![Span::raw(" ")];

        if self.right_view != RightView::Viewer && self.viewer.has_pending() {
            spans.push(Span::styled(
                "◆ Alt+E · ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        spans.push(Span::raw(self.right_pane_ref().title()));
        if focused {
            // Asked of the pane rather than decided here. The way out differs
            // per view and, in two of them, per state — a shell keeps `Esc` for
            // its child until that child exits, and a filter box keeps it until
            // the box closes. The shell cannot know any of that.
            spans.push(Span::styled(
                self.right_pane_ref().exit_hint(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::raw(" "));
        Line::from(spans)
    }

    /// Switch views, remembering what F2 should put back. Only the two
    /// workspace views are ever remembered, so F2 out of diagnostics can never
    /// land back on diagnostics.
    fn set_right_view(&mut self, view: RightView) {
        // Asking for a view is asking to see it. Without this, every view key
        // is a dead key while zoomed, which is a worse surprise than the pane
        // reappearing — that at least is visible and one keystroke to undo.
        self.zoom = false;
        let was_typing = self.focus == Focus::Right && self.right_pane().takes_input();
        self.right_view = view;
        if view != RightView::Diag {
            self.last_workspace_view = view;
        }
        // Leaving a pane you were typing into for one you cannot type into
        // hands focus back to Claude. Without this, `Alt+G` means two different
        // things depending on where you were: "show git, keep typing at Claude"
        // from the left, and "show git and you are now driving it" from the
        // shell — where the next thing typed would be read as scroll keys. One
        // keystroke, one meaning, from everywhere.
        if was_typing && !self.right_pane().takes_input() {
            self.focus = Focus::Left;
        }
    }

    fn right_pane(&mut self) -> &mut dyn Pane {
        match self.right_view {
            RightView::Git => &mut self.git,
            RightView::Viewer => &mut self.viewer,
            RightView::Shell => &mut self.shell,
            RightView::Diag => &mut self.diag,
        }
    }

    fn right_pane_ref(&self) -> &dyn Pane {
        match self.right_view {
            RightView::Git => &self.git,
            RightView::Viewer => &self.viewer,
            RightView::Shell => &self.shell,
            RightView::Diag => &self.diag,
        }
    }
}

fn block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    block_line(Line::from(title), focused)
}

fn block_line<'a>(title: Line<'a>, focused: bool) -> Block<'a> {
    let colour = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    Block::bordered()
        .title(title)
        .border_style(Style::default().fg(colour))
}

fn hit(r: Rect, ev: &MouseEvent) -> bool {
    ev.column >= r.x && ev.column < r.x + r.width && ev.row >= r.y && ev.row < r.y + r.height
}

/// Rewrite a mouse event into a pane's own coordinates. Panes are told where
/// the pointer is *within them* and nothing else, so they cannot develop an
/// opinion about where they are on screen.
fn relative(ev: &MouseEvent, r: Rect) -> MouseEvent {
    let mut out = *ev;
    out.column = ev.column.saturating_sub(r.x).min(r.width.saturating_sub(1));
    out.row = ev.row.saturating_sub(r.y).min(r.height.saturating_sub(1));
    out
}

/// Width of the key column in the overlay. Every binding in `keys::HELP` fits.
const HELP_KEYS: usize = 24;

fn help_overlay(f: &mut Frame) {
    let lines: Vec<Line> = keys::HELP
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(
                    format!("{k:<HELP_KEYS$}"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(*what, Style::default().fg(Color::Gray)),
            ])
        })
        .collect();

    // Measured from the lines that are about to be drawn, in cells. A constant
    // here was already clipping the longest two rows — the overlay that
    // explains the keys is the last place that should quietly lose its own
    // text, and a fixed number silently rots every time a line is reworded.
    // Measuring the `Line`s rather than the table cannot drift from the padding
    // above, and cannot be fooled by the `↑ ↓ ◆ …` this UI uses freely: those
    // are three bytes each and one cell each, and `str::len` would size the box
    // for the wrong one.
    let widest = lines.iter().map(Line::width).max().unwrap_or(0);
    let w = (widest as u16 + 2).min(f.area().width);
    let h = (lines.len() as u16 + 2).min(f.area().height);
    let [row] = Layout::vertical([Constraint::Length(h)])
        .flex(Flex::Center)
        .areas(f.area());
    let [area] = Layout::horizontal([Constraint::Length(w)])
        .flex(Flex::Center)
        .areas(row);

    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(block(" keys · any key to dismiss ", true)),
        area,
    );
}

/// The wiring, tested where it lives.
///
/// Everything here needs a real `App`, which needs a real pty, so it is
/// Windows-only like the rest of the pty-backed suite. The child is
/// `cmd /c exit` — these tests are about what the shell does with its panes,
/// not about what the child prints. The panes themselves are tested in their
/// own modules, with none of this.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;

    /// An `App` and the directory it was pointed at, which has to outlive it.
    ///
    /// A scratch directory rather than the repository: every `App::new` starts
    /// a recursive watch and a git worker on whatever it is given, and eleven
    /// tests doing that to the developer's live worktree — while `cargo test`
    /// is writing to `target/` inside it — is a test suite coupled to whatever
    /// state the repo happens to be in.
    struct Fixture {
        // Declared first so the watcher is stopped before the directory it is
        // watching is removed.
        app: App,
        dir: TempDir,
    }

    impl std::ops::Deref for Fixture {
        type Target = App;
        fn deref(&self) -> &App {
            &self.app
        }
    }

    impl std::ops::DerefMut for Fixture {
        fn deref_mut(&mut self) -> &mut App {
            &mut self.app
        }
    }

    fn app() -> Fixture {
        let dir = TempDir::new("app");
        dir.write("notes.md", b"# notes\n");
        let left = TerminalPane::spawn("cmd.exe", &["/c".into(), "exit".into()], 20, 60)
            .expect("spawn a child in a pty");
        let app = App::new(left, dir.path().to_path_buf());
        Fixture { app, dir }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    /// Render one frame and flatten it, so a test can ask what is on screen
    /// rather than what the code meant to put there.
    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut term = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| app.ui(f)).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn the_instrument_is_a_detour_and_comes_back_to_where_you_were() {
        let mut app = app();
        app.handle_key(alt(KeyCode::Char('e'))).unwrap();
        assert_eq!(app.right_view, RightView::Viewer);

        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Diag);
        // F2 out of diagnostics must land on the view it displaced, never on
        // diagnostics again — which is why `last_workspace_view` refuses to
        // record it.
        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Viewer);

        // Alt+G from the instrument is still a direct selection, and it is what
        // F2 then remembers.
        app.handle_key(key(KeyCode::F(2))).unwrap();
        app.handle_key(alt(KeyCode::Char('g'))).unwrap();
        assert_eq!(app.right_view, RightView::Git);
        app.handle_key(key(KeyCode::F(2))).unwrap();
        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Git);
    }

    /// The command view is deliberately never *drawn* in these tests. Drawing
    /// it spawns a real shell — that is the contract, spawn on first draw — and
    /// what is under test here is the shell's routing, not the pane's child.
    #[test]
    fn the_command_view_takes_focus_and_the_same_key_hands_it_back() {
        let mut app = app();
        screen(&mut app, 120, 24);
        assert_eq!(app.focus, Focus::Left);

        // The one view key that moves focus. A command line you have to press a
        // second key to type into is not a command line.
        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        assert_eq!(app.right_view, RightView::Shell);
        assert_eq!(app.focus, Focus::Right);

        // ...and pressing it again is the way home, so the whole round trip for
        // `git branch` is Alt+S, type, Alt+S.
        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        assert_eq!(app.focus, Focus::Left);
        assert_eq!(
            app.right_view,
            RightView::Shell,
            "leaving is a focus move, not a view switch — what you ran is still there"
        );

        // From any other view it selects rather than toggles. Otherwise the
        // press that brings the view up would be the press that leaves it.
        app.handle_key(alt(KeyCode::Char('g'))).unwrap();
        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        assert_eq!(app.right_view, RightView::Shell);
        assert_eq!(app.focus, Focus::Right);
    }

    #[test]
    fn focus_taken_before_the_pane_is_drawn_is_corrected_by_the_frame() {
        // Alt+S takes focus optimistically, because the pane it un-zooms has
        // not been drawn yet and `right_inner` still says there is nowhere to
        // go. That is only safe because the next frame is the authority.
        let mut app = app();
        screen(&mut app, 120, 24);
        app.handle_key(alt(KeyCode::Char('z'))).unwrap();
        screen(&mut app, 120, 24);
        assert!(app.right_inner.is_none(), "zoom hides the right pane");

        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        assert!(!app.zoom, "asking for a view is asking to see it");
        assert_eq!(app.focus, Focus::Right);

        // ...and a window too narrow to draw a right pane at all takes it back,
        // rather than leaving focus pointing at something that is not there.
        screen(&mut app, 40, 24);
        assert_eq!(app.focus, Focus::Left);
    }

    #[test]
    fn a_shell_running_behind_another_view_does_not_redraw_claude() {
        // The expensive mistake this app can make: a build running in the
        // command view while you read git, asking for a frame every time it
        // prints a line. A frame re-renders Claude's whole screen, so that is
        // Claude's typing latency spent on a pane nobody is looking at.
        let mut app = app();
        // `cmd` rather than whatever `ABEAM_SHELL` or the candidate search
        // would pick: this test is about the shell's bookkeeping, and `pwsh`
        // costs a second of startup to prove the same thing.
        app.app.shell = ShellPane::new(app.dir.path().to_path_buf(), Some("cmd.exe".into()));

        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        screen(&mut app, 120, 24); // the frame that spawns it

        // Wait for the banner, so the pane is known to have produced output at
        // all — otherwise the assertion below passes for the wrong reason.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !app.tick_panes() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.shell.is_live(), "the shell should be up by now");

        // Now hide it, and keep it printing. Asked repeatedly rather than once
        // because the other panes have opening news of their own — the startup
        // walk, the first git report — and under a loaded `cargo test` there is
        // no window this test can pick that is guaranteed to be quiet. What
        // *is* guaranteed: if a hidden shell's output counted, every one of
        // these would be true, because every one of them follows fresh output.
        app.handle_key(alt(KeyCode::Char('g'))).unwrap();
        let mut quiet = false;
        for _ in 0..8 {
            app.shell
                .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap();
            std::thread::sleep(Duration::from_millis(60));
            quiet |= !app.tick_panes();
        }
        assert!(quiet, "a hidden shell's output must not cost a frame");

        // ...and the same output does earn one when it is the view on screen,
        // or the pane would be frozen rather than merely quiet.
        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        app.shell
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut asked = false;
        while !asked && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            asked = app.tick_panes();
        }
        assert!(asked, "a visible shell's output must ask for a frame");
    }

    #[test]
    fn the_instrument_comes_back_to_the_command_view_too() {
        // `last_workspace_view` records any view but the instrument itself, so
        // F2 out of diagnostics can never land back on diagnostics — and can
        // land on the shell, which is a place you leave a command running.
        let mut app = app();
        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Diag);
        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Shell);
    }

    #[test]
    fn the_instrument_reports_the_live_session_rather_than_a_blank() {
        let mut app = app();
        app.handle_key(key(KeyCode::F(2))).unwrap();

        // ConPTY asks where the cursor is before the child runs at all, and the
        // reader thread answers. That is a handshake between two threads, so
        // the test waits for it rather than assuming the first frame is late
        // enough — it is not; the first frame usually sees zero bytes read.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.left.diagnostics().dsr_replies == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        let text = screen(&mut app, 120, 24);
        assert!(text.contains("DSR answered"), "got: {text}");
        assert!(text.contains("pty size"), "got: {text}");
        // The alarm is what makes this pane worth having: it means the session
        // is hung on the opening handshake, not merely slow. See
        // docs/conpty-findings.md.
        assert!(!text.contains("no DSR reply"), "got: {text}");
    }

    #[test]
    fn a_file_chosen_in_the_git_view_opens_in_the_viewer() {
        let mut app = app();
        assert_eq!(app.right_view, RightView::Git);
        app.git.stub_open_request("notes.md");

        assert!(app.pump(), "opening a file is worth a redraw");
        assert_eq!(app.right_view, RightView::Viewer);
        assert_eq!(
            app.viewer.path().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("notes.md"))
        );
        // Drained, not left to fire again at some unrelated later moment.
        assert!(!app.pump());
    }

    #[test]
    fn a_document_arriving_behind_the_git_view_is_announced_in_the_border() {
        // The mark can only ever be drawn by the shell: by the time the viewer
        // renders its own title it has taken the file up, so a mark inside the
        // pane would be a mark nobody can see.
        let mut app = app();
        let doc = app.dir.path().join("notes.md");
        app.viewer.follow(doc);

        assert!(screen(&mut app, 120, 24).contains('◆'));

        // Switching to the viewer takes the file up, and the mark goes with it.
        app.handle_key(alt(KeyCode::Char('e'))).unwrap();
        let text = screen(&mut app, 120, 24);
        assert!(!text.contains('◆'), "got: {text}");
        assert!(!app.viewer.has_pending());
    }

    #[test]
    fn the_watcher_never_moves_focus_or_switches_the_view() {
        // The rule the whole design rests on: typing goes to Claude, and
        // nothing that happens in the background may quietly change that.
        let mut app = app();
        app.viewer.follow(PathBuf::from("whatever.md"));
        app.git.request_refresh();

        screen(&mut app, 120, 24);
        assert_eq!(app.focus, Focus::Left);
        assert_eq!(app.right_view, RightView::Git);
    }

    #[test]
    fn esc_in_the_right_pane_gives_focus_back_and_esc_in_claude_does_not() {
        let mut app = app();
        // Focus only moves if there is a right pane to move to, which the last
        // drawn frame is what decides.
        screen(&mut app, 120, 24);
        app.handle_key(alt(KeyCode::Right)).unwrap();
        assert_eq!(app.focus, Focus::Right);

        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.focus, Focus::Left);

        // ...and now Esc is Claude's, as it must be: it is how you leave a
        // Claude prompt, and abeam stealing it would be unusable.
        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.focus, Focus::Left);
    }

    #[test]
    fn asking_for_a_view_while_zoomed_brings_the_pane_back() {
        let mut app = app();
        app.handle_key(alt(KeyCode::Char('z'))).unwrap();
        assert!(app.zoom);
        screen(&mut app, 120, 24);
        assert!(app.right_inner.is_none(), "zoom hides the right pane");

        // Otherwise every view key is a dead key while zoomed, which is the
        // same trap as Alt+E doing nothing when the viewer is already up.
        app.handle_key(alt(KeyCode::Char('e'))).unwrap();
        assert!(!app.zoom);
        assert_eq!(app.right_view, RightView::Viewer);

        app.handle_key(alt(KeyCode::Char('z'))).unwrap();
        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert!(!app.zoom);
        assert_eq!(app.right_view, RightView::Diag);
        // ...and F2 still remembers what it displaced, through the un-zoom.
        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Viewer);
    }

    #[test]
    fn a_narrow_window_gives_the_whole_screen_to_claude() {
        let mut app = app();
        screen(&mut app, 120, 24);
        app.handle_key(alt(KeyCode::Right)).unwrap();
        assert_eq!(app.focus, Focus::Right);

        // The right pane vanishes below `MIN_SPLIT_COLS`, and focus cannot be
        // left pointing at something that is not drawn.
        screen(&mut app, 40, 24);
        assert!(app.right_inner.is_none());
        assert_eq!(app.focus, Focus::Left);
    }

    #[test]
    fn quitting_a_live_session_asks_twice() {
        let mut app = app();
        assert!(matches!(
            app.handle_key(alt(KeyCode::Char('q'))).unwrap(),
            Flow::Continue { .. }
        ));
        assert!(app.pending_quit);
        assert!(screen(&mut app, 120, 24).contains("Alt+Q again"));

        // Any other key is the cancel, and there is no other cancel to learn.
        app.handle_key(key(KeyCode::Char('x'))).unwrap();
        assert!(!app.pending_quit);
        assert!(matches!(
            app.handle_key(alt(KeyCode::Char('q'))).unwrap(),
            Flow::Continue { .. }
        ));
        assert!(matches!(
            app.handle_key(alt(KeyCode::Char('q'))).unwrap(),
            Flow::Quit
        ));
    }

    #[test]
    fn the_next_key_after_the_escape_hatch_reaches_claude_verbatim() {
        // It must never be possible for abeam to permanently shadow a Claude
        // binding.
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.literal_next);

        app.handle_key(alt(KeyCode::Char('e'))).unwrap();
        assert!(!app.literal_next);
        // Alt+E went to the pty instead of switching the view.
        assert_eq!(app.right_view, RightView::Git);
        app.handle_key(alt(KeyCode::Char('e'))).unwrap();
        assert_eq!(app.right_view, RightView::Viewer);
    }

    #[test]
    fn the_help_overlay_shows_every_binding_without_clipping_it() {
        let mut app = app();
        app.handle_key(key(KeyCode::F(1))).unwrap();
        let text = screen(&mut app, 120, 40);
        for (k, what) in keys::HELP {
            if k.is_empty() {
                continue;
            }
            assert!(text.contains(k), "{k} is missing from the overlay");
            assert!(text.contains(what), "'{what}' was clipped off the overlay");
        }

        // Any unbound key dismisses it: a reader who has started typing has
        // stopped reading.
        app.handle_key(key(KeyCode::Char('x'))).unwrap();
        assert!(!app.help);
    }

    #[test]
    fn the_help_overlay_says_any_key_and_means_any_key() {
        // It used to mean "any key abeam has no binding for". Press F1 then
        // Alt+G and the overlay was still drawn on top of the git view you had
        // just asked to see, and a third keystroke was needed to get it back.
        let mut app = app();
        for dismiss in [
            alt(KeyCode::Char('g')),
            alt(KeyCode::Char('e')),
            alt(KeyCode::Char('z')),
            alt(KeyCode::Right),
            key(KeyCode::F(2)),
        ] {
            app.handle_key(key(KeyCode::F(1))).unwrap();
            assert!(app.help, "F1 opens it");
            app.handle_key(dismiss).unwrap();
            assert!(!app.help, "{dismiss:?} left the overlay up");
        }

        // ...and F1 itself is still the toggle it always was.
        app.handle_key(key(KeyCode::F(1))).unwrap();
        assert!(app.help);
        app.handle_key(key(KeyCode::F(1))).unwrap();
        assert!(!app.help);
    }

    #[test]
    fn a_frame_is_drawn_for_events_that_changed_something_and_not_for_others() {
        // `Handled` is the redraw signal, and the loop acts on it: a frame
        // re-renders Claude's whole screen, so a key a pane declined must not
        // cost one. Release events matter most — Windows sends one for every
        // keystroke, so treating them as news doubles the frame rate of typing.
        let mut app = app();
        let redraws = |flow: Flow| matches!(flow, Flow::Continue { redraw: true });

        let mut release = alt(KeyCode::Char('e'));
        release.kind = KeyEventKind::Release;
        assert!(!redraws(app.handle_event(Event::Key(release)).unwrap()));

        assert!(redraws(app.handle_event(Event::Key(alt(KeyCode::Char('g')))).unwrap()));
        assert!(redraws(app.handle_event(Event::Resize(80, 24)).unwrap()));

        // Focused on the git view with nothing to scroll, `j` changes nothing.
        screen(&mut app, 120, 24);
        app.handle_key(alt(KeyCode::Right)).unwrap();
        assert_eq!(app.focus, Focus::Right);
        assert!(!redraws(app.handle_event(Event::Key(key(KeyCode::Char('j')))).unwrap()));
    }

    #[test]
    fn a_release_event_is_dropped_before_it_can_fire_a_binding_twice() {
        // Windows sends Press and Release for every key. `encode_key` filters
        // releases, but the shell matches its own bindings first — without the
        // filter in `handle_event`, Alt+E would switch to the viewer and Alt+G
        // straight back on the release of the same press.
        let mut app = app();
        let mut release = alt(KeyCode::Char('e'));
        release.kind = KeyEventKind::Release;
        app.handle_event(Event::Key(release)).unwrap();
        assert_eq!(app.right_view, RightView::Git);
    }
}
