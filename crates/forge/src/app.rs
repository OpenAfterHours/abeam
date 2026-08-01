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

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use forge_pty::ExitStatus;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use crate::keys::{self, Action};
use crate::layout as forge_layout;
use crate::pane::{Focus, Pane};
use crate::panes::{DiagPane, GitPane, RightView, TerminalPane, ViewerPane};
use crate::watch::Watch;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub enum Outcome {
    /// The child finished. `screen` is what its last frame said — printed to
    /// the *primary* buffer by `main`, because leaving the alternate screen
    /// throws away everything drawn inside it, and a session whose transcript
    /// vanishes on `/exit` is a worse terminal than the one forge replaced.
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
    diag: DiagPane,
    right_view: RightView,
    /// What F2 puts back. Only ever `Git` or `Viewer`.
    last_workspace_view: RightView,
    /// One watcher for the whole app; the shell splits its output between the
    /// two panes that care. `None` if the platform would not watch, in which
    /// case both panes fall back to their own refresh.
    watch: Option<Watch>,
    focus: Focus,
    zoom: bool,
    help: bool,
    /// The next keystroke bypasses every forge binding. See `keys::Action`.
    literal_next: bool,
    /// Quitting kills a live session, so it asks twice. One bit rather than a
    /// modal dialog: any other key cancels it, which is the whole interaction.
    pending_quit: bool,
    /// Whichever pane owned the last mouse press keeps drag and motion events
    /// even once the pointer leaves it. Without this, dragging a selection in
    /// Claude and crossing the divider silently retargets mid-gesture.
    mouse_owner: Option<Focus>,
    /// Stashed by the last frame. Panes are sized from exactly the rects that
    /// were drawn, so the two can never disagree.
    left_inner: Rect,
    right_inner: Option<Rect>,
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
            git: GitPane::new(root),
            viewer,
            diag: DiagPane::new(),
            right_view: RightView::Git,
            last_workspace_view: RightView::Git,
            watch,
            focus: Focus::Left,
            zoom: false,
            help: false,
            literal_next: false,
            pending_quit: false,
            mouse_owner: None,
            left_inner: Rect::ZERO,
            right_inner: None,
        }
    }

    pub fn run(mut self, terminal: &mut Tui) -> Result<Outcome> {
        self.draw(terminal)?;

        loop {
            if let Some(status) = self.left.poll_exit()? {
                // try_wait can report an exit while the last of the output is
                // still in flight. Let the reader drain, then take the screen
                // it drained into — that is what makes the wait worth 50 ms.
                std::thread::sleep(Duration::from_millis(50));
                let screen = self.left.last_screen();
                return Ok(Outcome::Exited { status, screen });
            }

            let mut redraw = false;

            if event::poll(Duration::from_millis(10))? {
                // Drain everything pending before drawing. Windows floods
                // Resize events during a window drag and ConPTY resize is the
                // flakiest operation in the stack; one batch is one resize.
                loop {
                    match self.handle_event(event::read()?)? {
                        Flow::Quit => return Ok(Outcome::Detached),
                        Flow::Continue { redraw: wanted } => redraw |= wanted,
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }

            // Every pane ticks whether or not it is visible: the watcher has to
            // notice new markdown while the git view is showing.
            redraw |= self.left.tick();
            redraw |= self.git.tick();
            redraw |= self.viewer.tick();
            redraw |= self.pump();

            if redraw {
                self.draw(terminal)?;
            }
        }
    }

    /// Everything the panes cannot say to each other. Runs once per loop
    /// iteration, and does no work at all when the watcher is quiet.
    ///
    /// This is the reason forge exists rather than three windows: an agent
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
                // Only the terminal pane takes a paste; the right-hand views
                // are read-only and have nowhere to put one.
                if self.focus == Focus::Left {
                    self.left.handle_paste(&text)?;
                }
                Ok(Flow::redraw())
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
            self.left.handle_key(key)?;
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
                // user is done with it.
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
                if confirming || self.left.has_exited() {
                    return Ok(Flow::Quit);
                }
                self.pending_quit = true;
            }
            // Direct selection, not a cycle: Alt+G always means "git is now
            // showing", whatever was there.
            Action::ShowGit => self.set_right_view(RightView::Git),
            Action::ShowViewer => {
                // Pressing it again while the viewer is already up would
                // otherwise be a key that does nothing. Re-reading the open
                // file is the obvious thing it should mean, and it is what `r`
                // does from inside the pane.
                if self.right_view == RightView::Viewer {
                    self.viewer.reload();
                }
                self.set_right_view(RightView::Viewer);
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
                // been focused, so panes implement one scroll vocabulary.
                let key = KeyEvent::new(code, KeyModifiers::NONE);
                self.right_pane().handle_key(key)?;
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
        // careless one. Only the terminal pane has one: the right-hand views
        // learn their size inside `render`, from the same rect.
        let left_inner = self.left_inner;
        self.left.on_resize(left_inner)?;
        Ok(())
    }

    fn ui(&mut self, f: &mut Frame) {
        let split = forge_layout::split(f.area(), self.zoom);
        self.left_inner = forge_layout::inner(split.left);
        self.right_inner = split.right.map(forge_layout::inner);

        // The right pane can vanish under a narrow window while focused.
        if self.right_inner.is_none() {
            self.focus = Focus::Left;
        }

        let left_focused = self.focus == Focus::Left;
        let left_title = if self.pending_quit {
            format!(" {} · Alt+Q again to quit ", self.left.title())
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

        // The real cursor sits only in the terminal pane, and only when it has
        // focus. It is the strongest focus signal there is, because it is what
        // a typist is already looking at, and it costs no screen space. The
        // right-hand views are read-only: there is nothing there to point at.
        let rect = self.left_inner;
        if self.focus == Focus::Left
            && let Some((col, row)) = self.left.cursor()
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
            spans.push(Span::styled(
                " · esc→claude",
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
        self.right_view = view;
        if view != RightView::Diag {
            self.last_workspace_view = view;
        }
    }

    fn right_pane(&mut self) -> &mut dyn Pane {
        match self.right_view {
            RightView::Git => &mut self.git,
            RightView::Viewer => &mut self.viewer,
            RightView::Diag => &mut self.diag,
        }
    }

    fn right_pane_ref(&self) -> &dyn Pane {
        match self.right_view {
            RightView::Git => &self.git,
            RightView::Viewer => &self.viewer,
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
        // Claude prompt, and forge stealing it would be unusable.
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
        // Forge must never be able to permanently shadow a Claude binding.
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
        // It used to mean "any key forge has no binding for". Press F1 then
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
