//! The shell: layout, focus, and the event loop.
//!
//! Two rules run through all of it.
//!
//! **Typing goes to the agent.** It is over 95% of keystrokes, so it is the
//! state the app lives in and the one it must never leave by accident. Nothing
//! moves focus implicitly — not the file watcher, not a view switch, not a git
//! refresh. Only `F4`/`F5`, a mouse click, or `Esc` from the right pane.
//!
//! **Reading the right pane costs nothing.** Switching views and scrolling both
//! work while the agent still has focus. Focus is needed only to drive a
//! selection. A two-pane multiplexer that makes you switch modes to *look* at
//! something has already lost.
//!
//! The third thing the shell does, which neither pane can do for itself, is
//! carry messages between them: the watcher's events go out to both, and a file
//! chosen in the git view opens in the viewer. Panes are deliberately ignorant
//! of each other — that is what makes them individually testable — so every
//! wire between them is here, in [`App::pump`].

use std::io::{BufWriter, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use abeam_pty::ExitStatus;
use anyhow::Result;
use crossterm::QueueableCommand;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use crate::agentstate::Probe;
use crate::config::Opening;
use crate::keys::{self, Action};
use crate::layout as abeam_layout;
use crate::pane::{Focus, Pane};
use crate::panes::{
    DiagPane, FrameStats, GitPane, QueuePane, RightView, ShellPane, TerminalPane, ViewerPane,
};
use crate::watch::Watch;

pub type Tui = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

/// The longest the loop will sleep when nothing has woken it.
///
/// This is no longer the thing that paces drawing — output rings a doorbell now
/// — so it is only what the panes without one are polled at: the git pane's
/// channel, a viewer walk finishing, a shell child's `try_wait`.
const TICK: Duration = Duration::from_millis(10);

/// The shortest gap between two frames, start to start.
///
/// A floor, not a target. An agent produces output far faster than any screen
/// can show it, and drawing every time it does spends the whole budget on
/// frames nobody sees while the console write queue backs up — which is what
/// jitter *is*. Holding frames to an even cadence and always drawing the
/// newest state coalesces a burst into one frame and makes the ones that do go
/// out land evenly, which is what reads as smooth.
///
/// 8 ms is 125 fps: comfortably above any display abeam will be looked at on,
/// and a smaller worst-case delay than the 10 ms poll it replaces, so nothing
/// about typing got slower.
const MIN_FRAME: Duration = Duration::from_millis(8);

/// How often the agent's own idle/busy record is re-read. See
/// [`App::poll_readiness`] for why this is a poll and not a watch.
const READINESS_EVERY: Duration = Duration::from_millis(250);

/// How often the background-agent roster is refreshed while the queue holds
/// something dispatched.
///
/// Two orders of magnitude slower than the readiness read, because it is two
/// orders of magnitude more expensive: it starts a process. Nothing waits on
/// it — it updates rows that are already on screen — so the only cost of being
/// late is a status a few seconds stale.
const ROSTER_EVERY: Duration = Duration::from_secs(3);

/// Why the loop woke up.
enum Wake {
    /// The console had something to say. Carried rather than re-read, because
    /// the thread that reads them is the only one that may.
    Input(Event),
    /// The agent produced output. No payload — the news is the pty's sticky
    /// dirty flag, and this only says "go and look".
    Output,
}

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
    Continue {
        redraw: bool,
    },
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

/// The frame clock: what decides when the next frame may go out, and what F2
/// reports about the ones that already did.
///
/// It is one struct because those are the same question. The pacing is only
/// defensible if you can see what it costs, and "is the renderer keeping up"
/// was, until this existed, a thing that could only be guessed at from the
/// outside — which is exactly how a 10 ms poll in front of a sub-millisecond
/// renderer survived as long as it did.
struct Frames {
    /// When the last frame *began*. Start-to-start, so a slow frame does not
    /// push the next one further out and turn one hitch into a stutter.
    started: Instant,
    cost: Duration,
    drawn: u64,
    /// Reset every [`Frames::WINDOW`]; the worst frame in the window that just
    /// closed is what [`stats`](Frames::stats) reports, because an average
    /// frame time hides precisely the hitch you are looking for.
    window: Instant,
    in_window: u32,
    worst: Duration,
    last_worst: Duration,
    fps: f32,
}

impl Frames {
    const WINDOW: Duration = Duration::from_secs(1);

    fn new() -> Self {
        let now = Instant::now();
        Self {
            // Far enough back that the first frame is owed immediately.
            started: now - MIN_FRAME,
            cost: Duration::ZERO,
            drawn: 0,
            window: now,
            in_window: 0,
            worst: Duration::ZERO,
            last_worst: Duration::ZERO,
            fps: 0.0,
        }
    }

    /// How long until a frame may go out. Zero once one is owed.
    fn due_in(&self) -> Duration {
        MIN_FRAME.saturating_sub(self.started.elapsed())
    }

    fn due(&self) -> bool {
        self.due_in().is_zero()
    }

    fn record(&mut self, began: Instant) {
        self.started = began;
        self.cost = began.elapsed();
        self.drawn += 1;
        self.in_window += 1;
        self.worst = self.worst.max(self.cost);

        let open = self.window.elapsed();
        if open >= Self::WINDOW {
            self.fps = self.in_window as f32 / open.as_secs_f32();
            self.last_worst = self.worst;
            self.window = Instant::now();
            self.in_window = 0;
            self.worst = Duration::ZERO;
        }
    }

    fn stats(&self) -> FrameStats {
        let ms = |d: Duration| d.as_secs_f32() * 1e3;
        FrameStats {
            drawn: self.drawn,
            last_ms: ms(self.cost),
            // Before the first window closes there is nothing to report but the
            // window in progress, which is better than reporting zero.
            worst_ms: ms(self.last_worst.max(self.worst)),
            fps: self.fps,
        }
    }
}

pub struct App {
    left: TerminalPane,
    git: GitPane,
    viewer: ViewerPane,
    shell: ShellPane,
    diag: DiagPane,
    queue: QueuePane,
    /// Reads the agent's own record of whether it is mid-turn. See
    /// `crate::agentstate` — this is the only thing standing between a queued
    /// prompt and a permission dialog.
    probe: Probe,
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
    /// The agent's exit, and the screen it left behind, held until abeam is
    /// actually willing to go. Normally that is immediately; with a command
    /// still running in the shell view it is when the user says so.
    agent_exit: Option<(ExitStatus, Vec<String>)>,
    /// Whichever pane owned the last mouse press keeps drag and motion events
    /// even once the pointer leaves it. Without this, dragging a selection in
    /// the agent and crossing the divider silently retargets mid-gesture.
    mouse_owner: Option<Focus>,
    /// Stashed by the last frame. Panes are sized from exactly the rects that
    /// were drawn, so the two can never disagree.
    left_inner: Rect,
    right_inner: Option<Rect>,
    /// The whole window, as of the last frame. Kept because the split is a pure
    /// function of it: a key can ask whether there *would* be a right pane
    /// without waiting for a frame to find out.
    area: Rect,
    /// Paces the drawing and answers for it in the F2 view.
    frames: Frames,
    /// When the agent's idle/busy record was last read.
    readiness_at: Instant,
    /// When the background-agent roster was last asked for.
    roster_at: Instant,
    /// Whether the user has typed something at the agent that they have not
    /// submitted.
    ///
    /// Tracked here rather than read from the screen because the shell is the
    /// one party that already knows: every keystroke bound for the left pane
    /// passes through [`App::handle_key`]. A queued prompt sent while this is
    /// true would be spliced into the middle of a half-written message, which
    /// is the failure nobody would think to look for.
    draft_open: bool,
    /// Results from work that had to leave this thread. Both of the queue's
    /// outward actions start a process, and `Pane::tick` may not block.
    work_tx: SyncSender<Work>,
    work_rx: mpsc::Receiver<Work>,
    /// One at a time, each: a slow `claude agents --json` must not stack up a
    /// thread per loop iteration behind itself.
    roster_running: bool,
    dispatch_running: bool,
    /// Something has been dispatched at least once, so the roster is worth
    /// asking for. Sticky, and the reason a session that only ever uses the
    /// first mode never starts a `claude agents` process.
    dispatched_any: bool,
    /// A sent prompt is sitting in the composer, waiting for the `Enter` that
    /// submits it on the next pass. See [`App::pump_queue`].
    submit_pending: bool,
    /// The repository on screen, kept because the workers need it and they
    /// cannot borrow from the panes that were built with it.
    root: PathBuf,
    /// The hosted agent's name, for the same reason.
    agent: String,
}

/// What a worker thread has finished doing.
enum Work {
    Roster(Vec<crate::agentstate::Session>),
    Dispatched(Result<crate::dispatch::Started>),
}

impl App {
    /// `agent` is the hosted agent's name — `crate::agent::Hosted::agent`, so
    /// the agent behind a preset rather than the preset's own word, and never a
    /// path. It decides whether background dispatch is available at all,
    /// because `--bg` is Claude's and abeam will not reach for a different
    /// agent than the one it was asked to host. See `crate::dispatch`.
    ///
    /// `opening` is where the session starts: which right-hand view, which
    /// pane has the keyboard, whether it is zoomed, and which page the reader
    /// is on. Those four were literals on the lines below until there was
    /// somewhere to write an answer down — `crate::config` is that somewhere,
    /// and its [`Opening::default`] is exactly what they used to say.
    pub fn new(left: TerminalPane, root: PathBuf, agent: &str, opening: Opening) -> Self {
        // Taken here rather than inside the probe: the record abeam is looking
        // for is the one written *after* this moment, and a clock read at
        // construction is the closest to the spawn anything gets.
        let spawned_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let probe = Probe::new(root.clone(), left.process_id(), spawned_at);
        // Bounded and small: at most one roster refresh and one dispatch are
        // ever in flight, so anything deeper would be a queue nobody drains.
        let (work_tx, work_rx) = mpsc::sync_channel::<Work>(8);
        let watch = Watch::start(&root);
        let mut viewer = ViewerPane::new(root.clone());
        // Told rather than discovered, so a pane that will never update says so
        // on screen instead of looking like one that simply never notices.
        viewer.set_watching(watch.is_some());
        // Before the first frame, so the reader's page is right the first time
        // it is drawn rather than repainted a frame later.
        viewer.set_theme(opening.theme);

        Self {
            left,
            git: GitPane::new(root.clone()),
            viewer,
            // No child yet. It is spawned by the first frame that draws it, so
            // a session that never asks for a command line never pays for one.
            queue: QueuePane::new(root.clone(), agent),
            probe,
            shell: ShellPane::new(root.clone(), std::env::var("ABEAM_SHELL").ok()),
            diag: DiagPane::new(),
            right_view: opening.view,
            // The same view, because F2 puts back what it displaced and
            // nothing has displaced anything yet. `Opening` cannot name the
            // diagnostics view — `crate::config` leaves it out of the
            // vocabulary — so this stays the "never `Diag`" the field promises.
            last_workspace_view: opening.view,
            watch,
            focus: opening.focus,
            zoom: opening.zoom,
            help: false,
            literal_next: false,
            pending_quit: false,
            agent_exit: None,
            mouse_owner: None,
            left_inner: Rect::ZERO,
            right_inner: None,
            area: Rect::ZERO,
            frames: Frames::new(),
            // Far enough back that the first pass reads the record rather than
            // waiting a quarter second to find out what it is looking at.
            readiness_at: Instant::now() - READINESS_EVERY,
            roster_at: Instant::now() - ROSTER_EVERY,
            draft_open: false,
            work_tx,
            work_rx,
            roster_running: false,
            dispatch_running: false,
            dispatched_any: false,
            submit_pending: false,
            root,
            agent: agent.to_string(),
        }
    }

    /// The loop.
    ///
    /// It waits to be told rather than asking on a timer, and then declines to
    /// draw more often than a screen can show. Those are two halves of one
    /// idea: the old loop blocked 10 ms on the console before it would so much
    /// as look at the pty, which put a 10 ms floor under a renderer measured at
    /// 0.75 ms and quantised the agent's output onto a grid unrelated to it.
    /// Now both sources of news arrive on one channel — the console from a
    /// thread that may block on it, the agent from the pty reader — and
    /// [`MIN_FRAME`] is the only thing deciding when a frame goes out.
    pub fn run(mut self, terminal: &mut Tui) -> Result<Outcome> {
        // Bounded, because it is a doorbell and not a queue. Input uses the
        // blocking `send` — a keystroke may never be dropped — while output
        // uses `try_send` and lets a full channel swallow the ring, which is
        // correct: the flag it announces is sticky and the loop is by then
        // provably on its way to read it.
        let (tx, rx) = mpsc::sync_channel::<Wake>(64);
        // Deliberately only the left pane, and not the shell's pty. A
        // `cargo build` behind the git view can produce output thousands of
        // times a second, and every one of those rings would be a loop
        // iteration that goes on to draw nothing — `tick_panes` already
        // declines to spend a frame on a shell nobody is looking at. It keeps
        // the tick it has always had.
        self.left.wake_on_output({
            let tx = tx.clone();
            move || {
                let _ = tx.try_send(Wake::Output);
            }
        });
        spawn_input(tx);

        self.draw(terminal)?;
        // Something has changed that the screen does not show yet. Sticky
        // across iterations: news that arrives inside the frame floor is not
        // lost, it is waiting for the floor to lift.
        let mut redraw = false;

        loop {
            // Sleep until there is news, or until the frame we already owe is
            // allowed out, or until the panes without a doorbell want polling.
            // Whichever comes first.
            let wait = if redraw { self.frames.due_in() } else { TICK };

            match rx.recv_timeout(wait) {
                Ok(first) => {
                    // Drain everything queued before drawing. Windows floods
                    // Resize events during a window drag and ConPTY resize is
                    // the flakiest operation in the stack; one batch is one
                    // resize.
                    let mut next = Some(first);
                    while let Some(wake) = next.take() {
                        match wake {
                            Wake::Output => redraw = true,
                            Wake::Input(ev) => match self.handle_event(ev)? {
                                Flow::Quit => return Ok(self.finish()),
                                Flow::Continue { redraw: wanted } => redraw |= wanted,
                            },
                        }
                        next = rx.try_recv().ok();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                // Unreachable while this loop is running: the waker holds a
                // sender for as long as the left pane exists, and the left pane
                // outlives this function. Treated as the console having gone
                // rather than ignored, because the alternative to leaving is
                // spinning on a channel that will never speak again.
                Err(RecvTimeoutError::Disconnected) => return Ok(self.finish()),
            }

            // Everything below happens only when a frame could actually go out.
            // Under a flood, the agent rings this loop far more often than 125
            // times a second, and running the periodic work on every one of
            // those wakes would put a `try_wait`, three channel polls and a
            // watcher drain on the hot path of its output — to learn things
            // that could not be shown until the floor lifts anyway.
            if !self.frames.due() {
                continue;
            }

            if self.agent_exit.is_none()
                && let Some(status) = self.left.poll_exit()?
            {
                // try_wait can report an exit while the last of the output is
                // still in flight. Let the reader drain, then take the screen
                // it drained into — that is what makes the wait worth 50 ms.
                std::thread::sleep(Duration::from_millis(50));
                let screen = self.left.last_screen();
                self.agent_exit = Some((status, screen));
                // The left title now says the session has ended, and on the
                // path where abeam stays up that is the only thing announcing
                // it.
                redraw = true;
            }

            // The agent leaving normally ends abeam with it — that is what
            // abeam is. The exception is an open shell session: leaving kills
            // it, and killing someone's `cargo build` because the *other* pane
            // finished is not a decision this program gets to make on its own.
            // So it waits, says so in the title, and Alt+Q is the answer.
            //
            // "Open", not "busy", and the difference is worth knowing: ConPTY
            // cannot be asked whether a command is running, so a shell sitting
            // at a prompt holds the door exactly as a build does. The cost is
            // that pressing Alt+S once, early, changes how the session ends —
            // which is why the title names the shell rather than just saying
            // abeam is still here.
            if self.agent_exit.is_some() && !self.shell.is_live() {
                return Ok(self.finish());
            }

            redraw |= self.poll_readiness();
            redraw |= self.tick_panes();
            redraw |= self.pump();

            if redraw {
                self.draw(terminal)?;
                redraw = false;
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
    /// loop, and honouring it would re-render the agent's entire screen at the
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

        // Unlike the shell's, this pane's news counts while it is hidden, and
        // it has to: the countdown before an automatic send is drawn in the
        // *left* title, so the one thing that must never go unredrawn is the
        // announcement of a keystroke abeam is about to make on your behalf.
        // It is only affordable because the pane is disciplined about saying
        // no — see the frame-cost note on `QueuePane::tick`.
        redraw |= self.queue.tick();

        redraw
    }

    /// Ask the agent's own record whether it is mid-turn, and tell the queue.
    ///
    /// Rate-limited rather than watched, and the trade is worth stating. A
    /// `notify` watch on the sessions directory would be event-driven and is
    /// what this wants to be eventually; it would also be a second watcher
    /// thread, on a directory outside the repository, for a signal that is
    /// already cheap to ask for — one `stat` and a sub-kilobyte read. At
    /// [`READINESS_EVERY`] the worst-case lag is a quarter of a second in front
    /// of a countdown measured in seconds, which nobody can perceive.
    ///
    /// Returns whether the answer changed, because a changed answer is the
    /// thing that starts and stops that countdown.
    fn poll_readiness(&mut self) -> bool {
        if self.readiness_at.elapsed() < READINESS_EVERY {
            return false;
        }
        self.readiness_at = Instant::now();

        let mut readiness = self.probe.readiness();
        // Downgraded rather than reported separately, because `Unknown` already
        // means exactly this: abeam cannot establish that a send would be safe.
        // Without bracketed paste every newline in a sent block submits, so a
        // three-line prompt arrives as three — the second and third typed at an
        // agent already busy with the first. Every agent abeam hosts enables it,
        // so this is a floor rather than a case anyone will meet.
        if !self.left.bracketed_paste() {
            readiness = crate::agentstate::Readiness::Unknown;
        }
        // A session that has gone cannot be typed at, and its last record can
        // sit at `idle` forever — a dead agent is the most convincingly idle
        // thing there is.
        if self.left.has_exited() {
            readiness = crate::agentstate::Readiness::Unknown;
        }

        // The one event that ends a draft, and the only place this flag is ever
        // cleared. See [`note_left_key`](Self::note_left_key) for why it is this
        // and not a keystroke: a message that was really submitted makes the
        // agent work, and nothing else the user can press does.
        let mut redraw = false;
        if readiness == crate::agentstate::Readiness::Busy && self.draft_open {
            self.draft_open = false;
            redraw |= self.queue.set_draft_open(false);
        }
        redraw | self.queue.set_readiness(readiness)
    }

    /// Remember that the user may have an unsubmitted message at the agent.
    ///
    /// This only ever *sets* the flag. Clearing it is
    /// [`poll_readiness`](Self::poll_readiness)'s job and happens on one event:
    /// the agent being observed **busy**. That asymmetry is the whole design,
    /// and it replaced a version that tried to infer a submit from the keystroke
    /// — which cannot be done, because the keystroke that submits and the
    /// keystroke that does not are the same key.
    ///
    /// `Enter` is the case that proves it. A bare `Enter` submits, *except* when
    /// Claude's inline autocomplete has a completion open, where it accepts the
    /// completion and leaves the text sitting in the composer — and the
    /// autocomplete is not on the dialog stack Claude derives its status from,
    /// so the record still reads `idle`. Reading that `Enter` as a submit
    /// cleared this flag over a live draft and pasted a queued prompt into the
    /// middle of it three seconds later. `Esc` dismissing the same popup is the
    /// same bug. Modifiers do not separate them: `Shift+Enter` inserts a
    /// newline here and `Alt+Enter` is Copilot's only newline.
    ///
    /// Waiting to see the agent *busy* asks a question that has one answer. A
    /// message that was really submitted makes the agent work; an accepted
    /// completion does not.
    ///
    /// The cost is that a draft the user types and then silently abandons holds
    /// the queue until they submit something at the agent. Submitting is the
    /// *only* way past it — `Enter` in the queue pane grants an item its turn
    /// and does not overrule this, because a hand-picked item pasted into a
    /// half-written message is the same splice as an automatic one. It is at
    /// least visible: the pane says what it is waiting for. That is the safe
    /// direction to be wrong in.
    fn note_left_key(&mut self, key: &KeyEvent) {
        // Anything that could be putting text in front of the user, including
        // the two that do it without printing: `Up` and `Down` walk the agent's
        // history, and an idle agent holding a recalled message looks exactly
        // like one holding nothing from out here.
        let typing = matches!(
            key.code,
            KeyCode::Char(_)
                | KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::Tab
                | KeyCode::Enter
                | KeyCode::Up
                | KeyCode::Down
        );
        if typing {
            self.draft_open = true;
            // Told rather than asked for, so the countdown is withdrawn on the
            // keystroke itself rather than up to a quarter second later.
            self.queue.set_draft_open(true);
            // And the submit abeam still owes is abandoned. Between the paste
            // and the `Enter` that submits it there is one pass in which the
            // composer holds a queued prompt and the user can type into it;
            // pressing on with the `Enter` would submit their keystrokes along
            // with it. Dropping it strands the prompt in the composer instead,
            // where it is visible and one backspace from gone — which is the
            // recoverable half of the two.
            self.submit_pending = false;
        }
    }

    /// What to report on the way out.
    ///
    /// `Alt+Q` after the agent has gone is still the agent's exit — it is the
    /// same session ending, delayed by however long the shell was busy — and
    /// reporting it as a detach would throw away both the transcript `main`
    /// prints and the status code anything scripting abeam reads.
    fn finish(&mut self) -> Outcome {
        match self.agent_exit.take() {
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

        redraw |= self.pump_queue();
        redraw
    }

    /// The queue's two wires out, and the results of both coming back.
    ///
    /// This is the only place in abeam that produces input the user did not
    /// type, so it is worth being explicit about what protects it. The
    /// decision to send is not made here — [`QueuePane::take_send_request`]
    /// makes it, against the agent's own idle record, the draft flag this
    /// struct maintains, and a countdown that has been visible in the left
    /// title for seconds. By the time a request arrives here it has already
    /// been announced and not cancelled. What is left for this function is to
    /// deliver it without inventing a second way to get it wrong.
    fn pump_queue(&mut self) -> bool {
        let mut redraw = false;

        // Nothing new is taken while a submit is still owed. Two sends in
        // consecutive passes would paste the second on top of the first and
        // then overwrite the pending flag, so both prompts would go to the
        // agent as one message with one `Enter` — and both items would show as
        // sent. The `Enter` below is the only way out of this state.
        // The bracketed-paste check comes *before* the item is taken, not after
        // it. `take_send_request` is a drain: it marks the item `Sent` on the
        // way out, so refusing afterwards would leave a queue reading "sent"
        // over a prompt that was never typed. Asked here, a pty that cannot
        // carry the text simply leaves the item pending, which is what it is.
        // `poll_readiness` downgrades to `Unknown` on the same condition, so
        // this is unreachable in practice — it is the check that makes the
        // unreachability structural rather than a coincidence of ordering.
        if !self.submit_pending
            && self.left.bracketed_paste()
            && let Some(text) = self.queue.take_send_request()
        {
            // The `Enter` is armed only by a write that actually succeeded. A
            // pty that refused the paste and then got a bare `\r` would submit
            // whatever the user had in the composer, which is a stray keystroke
            // abeam invented out of its own failure.
            self.submit_pending = self.left.send_text(&text).is_ok();
            redraw = true;
        } else if std::mem::take(&mut self.submit_pending) {
            let _ = self
                .left
                .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            redraw = true;
        }

        if !self.dispatch_running
            && let Some(text) = self.queue.take_dispatch_request()
        {
            self.dispatch_running = true;
            let tx = self.work_tx.clone();
            let root = self.root.clone();
            let agent = self.agent.clone();
            std::thread::spawn(move || {
                let started = crate::dispatch::Dispatcher::new(root, &agent)
                    .map_err(|why| anyhow::anyhow!(why.0))
                    .and_then(|d| d.dispatch(&text));
                let _ = tx.send(Work::Dispatched(started));
            });
            redraw = true;
        }

        // Only while there is something dispatched to report on. A session
        // that never uses the second mode never starts this process.
        if !self.roster_running && self.dispatched_any && self.roster_at.elapsed() >= ROSTER_EVERY {
            self.roster_running = true;
            self.roster_at = Instant::now();
            let tx = self.work_tx.clone();
            let root = self.root.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Work::Roster(
                    crate::agentstate::roster(&root).unwrap_or_default(),
                ));
            });
        }

        while let Ok(work) = self.work_rx.try_recv() {
            match work {
                Work::Roster(rows) => {
                    self.roster_running = false;
                    redraw |= self.queue.set_roster(rows);
                }
                Work::Dispatched(outcome) => {
                    self.dispatch_running = false;
                    // Sticky, and only ever set here: it is what turns on the
                    // roster refresh, so a session that never dispatches
                    // anything never starts that process at all.
                    self.dispatched_any |= outcome.is_ok();
                    self.queue.note_dispatched(outcome);
                    redraw = true;
                }
            }
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
                    Focus::Left => {
                        // A paste into the composer is a draft like any other.
                        self.draft_open = true;
                        self.queue.set_draft_open(true);
                        self.left.handle_paste(&text)?
                    }
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
            // To whichever pane has focus, not to the agent. The hatch exists
            // so abeam can never permanently shadow a binding of the program you
            // are typing at, and once the right pane can host a shell that is
            // two programs. Sending it left regardless would deliver a
            // keystroke into the agent while the user is looking at the shell
            // they aimed it at, invisibly.
            match self.focus {
                Focus::Left => {
                    // Counts as typing like any other key: the escape hatch
                    // changes who reads the keystroke, not whether it landed in
                    // the composer.
                    self.note_left_key(&key);
                    self.left.handle_key(key)?
                }
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
                self.note_left_key(&key);
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
                // declines them and lands back on the agent, which is the way
                // out the other three views taught. A second predicate asking
                // the pane's *type* whether it takes typing would have to be
                // kept in sync with what its `handle_key` actually did, and
                // would be wrong for exactly the states that matter: a dead
                // child, and a read-only pane with a filter box open.
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
                // still running a command counts, even once the agent has
                // gone — that is the whole reason abeam is still on screen.
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
            // A workspace view like git and the reader, and pointedly not like
            // the shell: it does not take focus. The common case is glancing
            // at what is still queued while the agent works and you keep
            // typing at it, which is the rule the whole shell is built on.
            Action::ShowQueue => self.set_right_view(RightView::Queue),
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

            // Deliberately does not switch to the viewer or take focus. Unlike
            // the view keys, this changes nothing about *what* is on screen, so
            // dragging the reader into view to restyle it would be a surprise —
            // and the common case is pressing it while already looking at a
            // document from the left pane.
            Action::ToggleReaderTheme => self.viewer.toggle_theme(),

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
        let began = Instant::now();

        // The frame goes out between a begin/end pair, so a host terminal that
        // understands DEC 2026 shows all of it or none of it. Without this a
        // frame is composited whenever the terminal next feels like it, which
        // for a full-pane repaint means a visible seam partway down — the
        // half-updated screen that reads as tearing rather than as slowness.
        //
        // Queued, not executed: `execute!` would flush here and put the begin
        // in a syscall of its own. A terminal that does not know the sequence
        // ignores it, which is the whole reason private modes are shaped this
        // way.
        terminal.backend_mut().queue(BeginSynchronizedUpdate)?;
        terminal.draw(|f| self.ui(f))?;
        terminal.backend_mut().queue(EndSynchronizedUpdate)?;
        std::io::Write::flush(terminal.backend_mut())?;

        self.frames.record(began);

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
        // Appended rather than chosen between, and that is the fix for a real
        // failure rather than a tidy-up. These used to be arms of one `if`, so
        // a pending quit or an exited agent took the title and the queue's
        // countdown vanished from it — leaving abeam three seconds from typing
        // at the agent with nothing on screen saying so. `title_note`'s own
        // contract is that it is never silent while a send is due; the pane
        // kept that promise and the shell broke it. Two facts about the left
        // pane are two pieces of one title.
        let state = if self.pending_quit {
            Some("Alt+Q again to quit".to_string())
        } else if self.agent_exit.is_some() {
            // This is abeam outliving the session it exists for, which happens
            // only because a shell is open. Naming it matters: without that
            // word the window looks stuck, and the one thing the user needs to
            // know is that something of theirs is still alive in the other pane.
            Some("shell open · Alt+Q to quit".to_string())
        } else {
            None
        };
        // The queue reports in the *left* title because everything it says is
        // about the left pane: how much is waiting to be typed there, and — the
        // part that has to be impossible to miss — that abeam is about to type
        // it. Last, so that a title clipped at 46 columns loses the count
        // before it loses the announcement.
        let left_title = [state, self.queue.title_note()]
            .into_iter()
            .flatten()
            .fold(format!(" {}", self.left.title()), |title, part| {
                format!("{title} · {part}")
            })
            + " ";
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
                // The frame clock reports on the loop, not on the pty, so it
                // comes from here rather than out of `diagnostics()`. Same
                // rule: only on the frames that show it.
                self.diag.update_frames(self.frames.stats());
            }
            f.render_widget(block_line(self.right_title(focused), focused), outer);
            self.right_pane().render(f, inner);
        }

        // The real cursor sits in whichever focused pane has one — the agent,
        // or the shell view. It is the strongest focus signal there is,
        // because it is what a typist is already looking at, and it costs no
        // screen space.
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
    /// resource in a two-pane TUI and an agent's UI is hungry for them.
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
        // hands focus back to the agent. Without this, `Alt+G` means two
        // different things depending on where you were: "show git, keep typing
        // at the agent" from the left, and "show git and you are now driving
        // it" from the shell — where the next thing typed would be read as
        // scroll keys. One keystroke, one meaning, from everywhere.
        if was_typing && !self.right_pane().takes_input() {
            self.focus = Focus::Left;
        }
    }

    fn right_pane(&mut self) -> &mut dyn Pane {
        match self.right_view {
            RightView::Git => &mut self.git,
            RightView::Viewer => &mut self.viewer,
            RightView::Shell => &mut self.shell,
            RightView::Queue => &mut self.queue,
            RightView::Diag => &mut self.diag,
        }
    }

    fn right_pane_ref(&self) -> &dyn Pane {
        match self.right_view {
            RightView::Git => &self.git,
            RightView::Viewer => &self.viewer,
            RightView::Shell => &self.shell,
            RightView::Queue => &self.queue,
            RightView::Diag => &self.diag,
        }
    }
}

/// Reads the console forever and forwards what it finds.
///
/// A thread because `event::read` is the only way to be *told* about a
/// keystroke rather than to ask, and the loop it used to be called from now has
/// a second thing to wait on. One reader, here, and nowhere else: crossterm's
/// event source is global, and two threads competing for it lose keys.
///
/// It is never joined, for the same reason the pty reader is not — there is no
/// way to interrupt a blocking read, and a session that hangs on the way out to
/// wait for a keystroke nobody is going to type is worse than a thread that
/// dies with the process.
///
/// A read that errors ends the thread rather than the program. The console
/// having gone is not something the loop can do anything useful about, and it
/// will find out on its own the moment it tries to draw.
fn spawn_input(tx: SyncSender<Wake>) {
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            // Blocking, unlike the output doorbell: a dropped keystroke is a
            // character missing from what somebody typed. The loop drains this
            // channel on every pass, so the queue is short and the wait is not.
            if tx.send(Wake::Input(ev)).is_err() {
                break;
            }
        }
    });
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
/// Everything here needs a real `App`, which needs a real pty and a real child
/// at the end of it — but almost none of it needs a *particular* child. These
/// are tests about what the shell does with its panes: queue to pty, draft and
/// submit, view switching, focus, zoom, the double confirm, the overlay, frame
/// pacing. What the child prints is scenery for all but four of them, which is
/// why the whole module ran on Windows for as long as abeam did and why
/// un-gating it needed one platform-selected child rather than thirty-one.
///
/// The four that do care say so on themselves: two need a child that has asked
/// for bracketed paste, one needs a child that provably never will, and one is
/// about ConPTY's opening handshake and has no Unix half to be about. The panes
/// themselves are tested in their own modules, with none of this.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentstate::Readiness;
    use crate::panes::queue::Mode;
    use crate::testutil::TempDir;
    use abeam_pty::PtyConfig;
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;

    // --- the children these tests spawn ------------------------------------
    //
    // Chosen here rather than at every call site, and written out rather than
    // reduced to "a shell", because the difference between them is the whole
    // subject of three tests below. None of them is an agent and none of them
    // needs to be.

    /// A child that starts and leaves immediately.
    ///
    /// The fixture's default, and most of why these tests are quick. It is also
    /// why three of them replace it: an agent that has already gone cannot be
    /// typed at, and `poll_readiness` is careful to say so.
    #[cfg(windows)]
    const EXITS: (&str, &[&str]) = ("cmd.exe", &["/c", "exit"]);
    #[cfg(unix)]
    const EXITS: (&str, &[&str]) = ("/bin/sh", &["-c", "exit"]);

    /// The shell for the one test that is about a *pane's* bookkeeping rather
    /// than about any shell in particular.
    ///
    /// Named outright rather than left to `ABEAM_SHELL` or the pane's candidate
    /// search, which would pick `pwsh` on one machine and `fish` on another and
    /// charge a second of startup to prove something neither of them is
    /// involved in.
    #[cfg(windows)]
    const A_PLAIN_SHELL: &str = "cmd.exe";
    #[cfg(unix)]
    const A_PLAIN_SHELL: &str = "/bin/sh";

    // The three children below are all `cmd.exe` on Windows and all `cat` on
    // Unix, and the second half of that is a decision rather than a shortage of
    // ideas.
    //
    // Whether a child asks for bracketed paste is load-bearing for all three:
    // `pump_queue` refuses to write to a pty that has not asked, so a child
    // that asks by accident and a child that refuses to ask would each make one
    // of these tests pass while proving the opposite of what it says. A shell
    // cannot be trusted either way here. Any readline-backed shell enables the
    // mode on an interactive pty — bash certainly does — and on Linux `/bin/sh`
    // is bash on some distributions and dash on others, so the answer would
    // depend on which image CI pulled. `cat` has no line editor, no prompt and
    // no opinion about terminals: it copies bytes. So the mode is handed over
    // in a *file* when it is wanted and simply absent when it is not, which
    // makes each of these children say one thing.
    //
    // Named as `/bin/cat` rather than looked up on `PATH`, and that is the same
    // trade `dispatch`'s shims make when they ask for `#!/bin/sh` and refuse to
    // ask for `#!/bin/bash`: name what POSIX requires every system to have, at
    // the path every mainstream distribution puts it at, and do not go
    // searching. It is a choice rather than an oversight — a system that puts
    // neither at those paths (NixOS, where everything lives under `/nix/store`)
    // fails these at the spawn, and the answer there is a `PATH` lookup in the
    // fixture rather than a different program. That is worth writing when
    // somebody runs the suite on such a machine, and not before.
    //
    // They are deliberately wide. A prompt is most of fifty columns, and a
    // queued prompt wrapping onto a second row is a needle split in half and a
    // test that fails for a reason nobody is interested in.

    /// A child that asks for bracketed paste and then stays.
    ///
    /// The `DECSET` is typed out of a file on the way in rather than asked for
    /// by the child — the pty forwards it and the parser behind the pane picks
    /// it up, which is the same route a real agent's takes.
    #[cfg(windows)]
    fn asks_and_stays(dir: &TempDir) -> PtyConfig {
        dir.write("bracketed.txt", b"\x1b[?2004h");
        PtyConfig::new("cmd.exe")
            .args(["/k".to_string(), "type bracketed.txt".to_string()])
            .cwd(dir.path())
            .size(20, 200)
    }
    #[cfg(unix)]
    fn asks_and_stays(dir: &TempDir) -> PtyConfig {
        dir.write("bracketed.txt", b"\x1b[?2004h");
        // `cat file -`: the file first, then standard input for ever, which is
        // the whole of "emits the mode and then stays" in one process.
        PtyConfig::new("/bin/cat")
            .args(["bracketed.txt".to_string(), "-".to_string()])
            .cwd(dir.path())
            .size(20, 200)
    }

    /// A child that asks for bracketed paste and *then* leaves.
    ///
    /// The distinction that makes the test about an agent that has gone mean
    /// anything: the ordinary fixture child would give the same `Unknown` for
    /// the wrong reason, because it never enables the mode at all.
    #[cfg(windows)]
    fn asks_and_goes(dir: &TempDir) -> PtyConfig {
        dir.write("bracketed.txt", b"\x1b[?2004h");
        PtyConfig::new("cmd.exe")
            .args(["/c".to_string(), "type bracketed.txt".to_string()])
            .cwd(dir.path())
            .size(20, 200)
    }
    #[cfg(unix)]
    fn asks_and_goes(dir: &TempDir) -> PtyConfig {
        dir.write("bracketed.txt", b"\x1b[?2004h");
        // The same thing without the `-`, so it runs out of input and exits.
        PtyConfig::new("/bin/cat")
            .args(["bracketed.txt".to_string()])
            .cwd(dir.path())
            .size(20, 200)
    }

    /// A child that stays, prints enough to be known to have started, and
    /// **provably never asks for bracketed paste**.
    ///
    /// The printing is not decoration. The test that uses this waits for the
    /// pane to have read something before it looks, and on Windows ConPTY's own
    /// opening sequence supplies that whatever the child does — on Unix nothing
    /// reaches the pty unless the child puts it there, so a child that only
    /// listened would leave that wait spinning until its deadline and then pass
    /// anyway. So the file is ordinary text with no escape in it: enough to
    /// prove the child ran, and nothing a parser could take for a request.
    #[cfg(windows)]
    fn never_asks(dir: &TempDir) -> PtyConfig {
        PtyConfig::new("cmd.exe")
            .args(["/k".to_string()])
            .cwd(dir.path())
            .size(20, 200)
    }
    #[cfg(unix)]
    fn never_asks(dir: &TempDir) -> PtyConfig {
        dir.write("plain.txt", b"nothing here is an escape sequence\n");
        PtyConfig::new("/bin/cat")
            .args(["plain.txt".to_string(), "-".to_string()])
            .cwd(dir.path())
            .size(20, 200)
    }

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
        let (program, args) = EXITS;
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let left = TerminalPane::spawn(program, &args, 20, 60).expect("spawn a child in a pty");
        let app = App::new(
            left,
            dir.path().to_path_buf(),
            "claude",
            crate::config::Opening::default(),
        );
        Fixture { app, dir }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    /// The same frame, as background colours. The reader is the one pane that
    /// paints its own, so this is how a test sees a palette rather than a
    /// layout.
    fn page(app: &mut App, width: u16, height: u16) -> Vec<ratatui::style::Color> {
        let mut term = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| app.ui(f)).unwrap();
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.bg)
            .collect()
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

    // --- the queue's two wires, and what stands between them and the pty ---
    //
    // Everything below is about `pump_queue`, `poll_readiness`, `note_left_key`
    // and the left title: the four places where the queue, the probe and the
    // agent's pty are joined up. Each of those three is well covered on its
    // own — which is exactly why the joins were not, and why a mutation audit
    // could delete the pty write from `pump_queue` and watch every queued item
    // still report itself as sent.

    /// The pid the planted records are named for. Any number does: nothing
    /// looks for the process, and [`Probe::over`] is told which to expect.
    const RECORD_PID: u32 = 4242;

    /// A directory holding one record for the session this app is hosting, with
    /// the app's probe aimed at it.
    ///
    /// `Probe::new` reads the machine's own `~/.claude`, where the only honest
    /// answer about a shell is `Unknown` — and `Unknown` is the one answer
    /// indistinguishable from the whole feature having been deleted. Returned
    /// rather than dropped so the directory outlives the probe reading it.
    fn records(fx: &mut Fixture, status: &str) -> TempDir {
        let dir = TempDir::new("records");
        say(&dir, fx.dir.path(), status);
        fx.app.probe = Probe::over(
            dir.path().to_path_buf(),
            fx.dir.path().to_path_buf(),
            Some(RECORD_PID),
            // Spawned at the epoch, so the record's own `startedAt` is always
            // at or after it. Which record belongs to which session is
            // `agentstate`'s question and is settled there.
            0,
        );
        dir
    }

    /// Write the record, or write it again. Claude replaces it in place as the
    /// session changes state and `Probe` re-reads the file on every poll, so
    /// this is what a turn starting looks like from out here.
    fn say(dir: &TempDir, root: &std::path::Path, status: &str) {
        // The `cwd` goes through serde rather than being pasted in: a path in
        // JSON is a string, and on Windows it is one full of escapes.
        let cwd = serde_json::to_string(&root.to_string_lossy()).expect("a JSON string");
        let record = format!(
            r#"{{"pid":{RECORD_PID},"sessionId":"s","cwd":{cwd},"startedAt":1,"peerProtocol":1,"kind":"interactive","name":"fixture","status":"{status}"}}"#
        );
        dir.write(&format!("{RECORD_PID}.json"), record.as_bytes());
    }

    /// Put [`asks_and_stays`] in the left pane and wait for the mode to land.
    ///
    /// Both halves are load-bearing and neither is free. The fixture's own
    /// child is gone before a send could reach it, and `poll_readiness`
    /// downgrades a departed agent to `Unknown`; and a child that never enabled
    /// bracketed paste is refused by `pump_queue` at the instant of the write.
    /// So a test that skipped this would pass for two wrong reasons at once.
    fn stays(fx: &mut Fixture) {
        let config = asks_and_stays(&fx.dir);
        fx.app.left = TerminalPane::spawn_with(config).expect("a child in a pty");

        let deadline = Instant::now() + Duration::from_secs(20);
        while !fx.app.left.bracketed_paste() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            fx.app.left.bracketed_paste(),
            "the child never asked for bracketed paste, so nothing would ever be sent"
        );
    }

    /// A readiness poll without the quarter-second rate limit in front of it.
    ///
    /// [`READINESS_EVERY`] is a decision about what re-reading the record
    /// costs, not about what it says; a test that slept through it would be
    /// timing the poll rather than reading its answer.
    fn polled(fx: &mut Fixture) -> bool {
        fx.app.readiness_at = Instant::now() - READINESS_EVERY;
        fx.app.poll_readiness()
    }

    /// The agent's screen, flattened.
    fn agent_screen(fx: &Fixture) -> String {
        fx.app.left.last_screen().join("\n")
    }

    /// Block until `text` is on the agent's screen, or say what was there
    /// instead. The echo comes back through a pty on another thread, so a test
    /// that looked once would be a test of thread scheduling.
    fn reaches_the_agent(fx: &mut Fixture, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let screen = agent_screen(fx);
            if screen.contains(text) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{text:?} never reached the agent. The screen says:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// How many keystrokes abeam has produced on the user's behalf.
    ///
    /// The pty counts these itself, and counts *only* keys: `send_text` writes
    /// its paste without touching the tally. So this is exactly the number of
    /// `Enter`s the queue has submitted, minus whatever the test typed on
    /// purpose — which is the only way to tell a prompt left sitting in the
    /// composer from one that was sent.
    fn keys_sent(fx: &Fixture) -> u64 {
        fx.app.left.diagnostics().keys_sent
    }

    #[test]
    fn a_queued_prompt_reaches_the_pty_and_is_submitted_on_the_pass_after_it() {
        // The survivor that alarmed the audit most: delete the `send_text` from
        // `pump_queue` and the whole suite stayed green, because the item was
        // marked `Sent` by the pane before the text ever left it and nothing
        // anywhere asked the pty what it had actually been given.
        //
        // The second half is the other one. The paste and the `Enter` are two
        // decisions a pass apart — the first is one backspace from gone and the
        // second is not — so a test that only checked "the prompt arrived"
        // would let both the missing `Enter` and an `Enter` sent along with the
        // paste through.
        let mut fx = app();
        let _records = records(&mut fx, "idle");
        stays(&mut fx);
        polled(&mut fx);

        fx.app.queue.stub_item("wire-check-alpha", Mode::Send);
        // Armed, which is condition 1 of the four...
        fx.app.queue.handle_key(key(KeyCode::Char('a'))).unwrap();
        // ...and then asked for by hand, which is the same drain reached
        // without sitting through the three-second announcement. The countdown
        // is the pane's own field and the pane's own tests pin it; what is
        // under test here is everything downstream of `take_send_request`.
        fx.app.queue.handle_key(key(KeyCode::Enter)).unwrap();

        assert!(fx.app.pump_queue(), "a send is worth a frame");
        reaches_the_agent(&mut fx, "wire-check-alpha");
        assert_eq!(
            keys_sent(&fx),
            0,
            "the prompt was submitted on the same pass that typed it"
        );
        assert!(fx.app.submit_pending, "nothing owes the agent an Enter");

        assert!(fx.app.pump_queue(), "the submit is worth a frame too");
        assert_eq!(keys_sent(&fx), 1, "the Enter that submits it never went out");
        assert!(!fx.app.submit_pending);

        // And nothing is owed after that, or the queue would type a bare
        // newline at the agent on every idle pass for the rest of the session.
        fx.app.pump_queue();
        assert_eq!(keys_sent(&fx), 1);
    }

    #[test]
    fn anything_typed_at_the_agent_opens_a_draft_and_only_a_busy_agent_ends_one() {
        let mut fx = app();
        let records = records(&mut fx, "idle");
        stays(&mut fx);

        // Bare `Enter` is the one to read twice, and the reason this list is a
        // list rather than a guess at which key submits: Claude's inline
        // autocomplete consumes an `Enter` without submitting, and the
        // autocomplete is not on the dialog stack the record is derived from,
        // so the record still reads `idle`. Reading that `Enter` as a submit
        // cleared this flag over a live draft and pasted a queued prompt into
        // the middle of it three seconds later.
        for pressed in [
            key(KeyCode::Char('x')),
            key(KeyCode::Backspace),
            key(KeyCode::Delete),
            key(KeyCode::Tab),
            // Neither of these prints anything, and both walk the agent's
            // history: an idle agent holding a recalled message looks exactly
            // like one holding nothing from out here.
            key(KeyCode::Up),
            key(KeyCode::Down),
            key(KeyCode::Enter),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
            alt(KeyCode::Enter),
        ] {
            fx.app.draft_open = false;
            fx.app.queue.set_draft_open(false);
            fx.app.note_left_key(&pressed);
            assert!(
                fx.app.draft_open,
                "{pressed:?} left the shell believing nothing had been typed"
            );
            assert!(
                fx.app.queue.is_draft_open(),
                "{pressed:?} never reached the queue, so the countdown would run on"
            );
        }

        // ...and the keys that cannot put text in front of anybody do not open
        // one, or the flag would be set by every glance at the screen and the
        // queue would never drain at all.
        for pressed in [
            key(KeyCode::Esc),
            key(KeyCode::Left),
            key(KeyCode::PageUp),
            key(KeyCode::F(6)),
        ] {
            fx.app.draft_open = false;
            fx.app.queue.set_draft_open(false);
            fx.app.note_left_key(&pressed);
            assert!(!fx.app.draft_open, "{pressed:?} is not typing");
        }

        // One event ends a draft and it is not a keystroke. An idle agent is
        // the state a draft *lives* in, so a poll that read the record and
        // cleared on it would clear on the very next pass.
        fx.app.note_left_key(&key(KeyCode::Char('h')));
        polled(&mut fx);
        assert!(fx.app.draft_open, "an idle agent must not end a draft");

        say(&records, fx.dir.path(), "busy");
        polled(&mut fx);
        assert!(
            !fx.app.draft_open,
            "the agent going busy is the only thing that ends a draft"
        );
        assert!(
            !fx.app.queue.is_draft_open(),
            "the shell forgot the draft without telling the pane"
        );
    }

    #[test]
    fn the_announcement_survives_every_state_the_left_title_can_be_in() {
        // A real bug, found and fixed: the title was an `if`/`else if` chain,
        // so a pending quit or a departed agent took the whole of it and the
        // queue's countdown vanished — leaving abeam three seconds from typing
        // at the agent with nothing on screen saying so. `title_note`'s
        // contract is that it is never silent while a send is due; the pane
        // kept that promise and the shell broke it.
        let mut fx = app();
        fx.app.queue.stub_item("announce me", Mode::Send);
        // Told outright rather than polled for. Which of the four conditions
        // hold is `poll_readiness`'s business and is tested above; this is
        // about what the title does once one of them has produced a countdown.
        fx.app.queue.set_readiness(Readiness::Idle);
        fx.app.queue.handle_key(key(KeyCode::Char('a'))).unwrap();
        assert!(
            fx.app
                .queue
                .title_note()
                .is_some_and(|note| note.contains("sending in")),
            "the pane is not announcing a send, so this test proves nothing"
        );

        // Rendered wide on purpose. A title clipped at the border is a
        // different failure with its own rule — `title_note` is appended last
        // precisely so a 46-column pane loses the count before it loses the
        // announcement — and what is under test here is the assembly.
        let plain = screen(&mut fx, 300, 24);
        assert!(plain.contains("sending in"), "got: {plain}");

        fx.app.pending_quit = true;
        let quitting = screen(&mut fx, 300, 24);
        assert!(quitting.contains("Alt+Q again to quit"), "got: {quitting}");
        assert!(
            quitting.contains("sending in"),
            "Alt+Q removed the only warning on screen that abeam was about to \
             type at the agent: {quitting}"
        );
        fx.app.pending_quit = false;

        // The fixture's own child leaves on its own, which is the other state
        // that used to take the title.
        let deadline = Instant::now() + Duration::from_secs(20);
        while fx.app.left.poll_exit().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = fx
            .app
            .left
            .poll_exit()
            .unwrap()
            .expect("the fixture's child exits on its own");
        fx.app.agent_exit = Some((status, Vec::new()));

        let ended = screen(&mut fx, 300, 24);
        assert!(ended.contains("shell open"), "got: {ended}");
        assert!(
            ended.contains("sending in"),
            "the agent leaving removed the announcement: {ended}"
        );
    }

    #[test]
    fn a_pending_submit_is_abandoned_the_moment_the_user_types() {
        // Between the paste and the `Enter` there is exactly one pass in which
        // the composer holds a queued prompt and the user can type into it.
        // Pressing on with the `Enter` would submit their keystrokes along with
        // it; dropping it strands the prompt in the composer instead, where it
        // is visible and one backspace from gone.
        let mut fx = app();
        let _records = records(&mut fx, "idle");
        stays(&mut fx);
        polled(&mut fx);

        fx.app.queue.stub_item("wire-check-bravo", Mode::Send);
        fx.app.queue.handle_key(key(KeyCode::Char('a'))).unwrap();
        fx.app.queue.handle_key(key(KeyCode::Enter)).unwrap();
        // Through the shell's own pump rather than straight at `pump_queue`,
        // which the tests either side of this one do. Nothing else covers the
        // one line joining the two, and a queue the loop never pumps is a queue
        // that silently stops draining.
        fx.app.pump();
        reaches_the_agent(&mut fx, "wire-check-bravo");
        assert!(fx.app.submit_pending, "nothing owes the agent an Enter");

        fx.app.handle_key(key(KeyCode::Char('!'))).unwrap();
        assert!(
            !fx.app.submit_pending,
            "a keystroke at the agent left the submit standing"
        );

        let theirs = keys_sent(&fx);
        assert_eq!(theirs, 1, "the user's own keystroke did not reach the pty");
        fx.app.pump();
        assert_eq!(
            keys_sent(&fx),
            theirs,
            "abeam submitted a message the user was still writing"
        );
    }

    #[test]
    fn the_queue_never_types_at_an_agent_that_has_gone() {
        // A session that has gone cannot be typed at, and its last record can
        // sit at `idle` forever: a dead agent is the most convincingly idle
        // thing there is.
        let mut fx = app();
        let _records = records(&mut fx, "idle");

        // A child that asks for bracketed paste and *then* leaves. The ordinary
        // fixture child would give the same `Unknown` for the wrong reason — it
        // never enables the mode at all — and a test that passes for the wrong
        // reason is a test of nothing.
        let config = asks_and_goes(&fx.dir);
        fx.app.left = TerminalPane::spawn_with(config).expect("a child in a pty");

        let deadline = Instant::now() + Duration::from_secs(20);
        while (!fx.app.left.bracketed_paste() || fx.app.left.poll_exit().unwrap().is_none())
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            fx.app.left.bracketed_paste(),
            "the child never asked for bracketed paste, so `Unknown` would be free"
        );
        assert!(fx.app.left.has_exited(), "the child was meant to leave");
        assert_eq!(
            fx.app.probe.readiness(),
            Readiness::Idle,
            "the record itself has to say `idle`, or there is nothing to override"
        );

        fx.app.queue.stub_item("never sent", Mode::Send);
        fx.app.queue.handle_key(key(KeyCode::Char('a'))).unwrap();
        polled(&mut fx);
        assert_eq!(
            fx.app.queue.title_note().as_deref(),
            Some("queue 1"),
            "a send was announced at an agent that is not there"
        );
        assert!(!fx.app.pump_queue(), "there was nothing to do");
        assert_eq!(keys_sent(&fx), 0);
    }

    #[test]
    fn readiness_is_unknown_while_the_agent_has_not_asked_for_bracketed_paste() {
        // Without the mode every newline in a sent block is a submit, so a
        // three-line prompt arrives as three — the second and third typed at an
        // agent already busy with the first. Every agent abeam hosts enables
        // it, so this is a floor rather than a case anyone will meet, and a
        // floor with nothing standing on it is one that quietly goes away.
        let mut fx = app();
        let _records = records(&mut fx, "idle");

        // Stays, so the departed-agent downgrade cannot be what answers, and
        // never enables the mode, so the downgrade under test is the only one
        // left. The record says `idle`: without that this assertion would hold
        // just as well with the probe deleted outright.
        //
        // Which child is not a detail here — see [`never_asks`]. A shell is
        // specifically the wrong one: an interactive readline shell *does* ask
        // for bracketed paste, and on Linux whether `/bin/sh` is one of those
        // is a fact about the distribution, so this test would assert the
        // opposite of itself on somebody else's CI image.
        let config = never_asks(&fx.dir);
        fx.app.left = TerminalPane::spawn_with(config).expect("a child in a pty");
        let deadline = Instant::now() + Duration::from_secs(20);
        while fx.app.left.diagnostics().bytes_read == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(!fx.app.left.has_exited(), "this child stays at its prompt");
        assert!(
            fx.app.left.diagnostics().bytes_read > 0,
            "the child never produced anything, so it was never really up"
        );
        assert!(
            !fx.app.left.bracketed_paste(),
            "the child asked for bracketed paste, so this test proves nothing"
        );
        assert_eq!(
            fx.app.probe.readiness(),
            Readiness::Idle,
            "the record itself has to say `idle`, or there is nothing to override"
        );

        fx.app.queue.stub_item("never sent", Mode::Send);
        fx.app.queue.handle_key(key(KeyCode::Char('a'))).unwrap();
        polled(&mut fx);
        assert_eq!(
            fx.app.queue.title_note().as_deref(),
            Some("queue 1"),
            "a send was announced at a pty that would submit every line of it"
        );
        assert!(!fx.app.pump_queue(), "there was nothing to do");
        assert_eq!(keys_sent(&fx), 0);

        // And the same fact re-asked at the instant of the write, which is a
        // separate check for a reason: the pane is told what was true a quarter
        // of a second ago, and this is the last thing standing between a stale
        // answer and a prompt submitted one line at a time. Told outright here,
        // because the poll above will never hand this pty an `Idle` to be stale
        // about.
        fx.app.queue.set_readiness(Readiness::Idle);
        fx.app.queue.handle_key(key(KeyCode::Enter)).unwrap();
        fx.app.pump_queue();
        assert!(
            !agent_screen(&fx).contains("never sent"),
            "a prompt was written to a pty that would have submitted every line of it"
        );
        assert!(
            !fx.app.submit_pending,
            "an Enter was armed by a write that was never made"
        );
        fx.app.pump_queue();
        assert_eq!(keys_sent(&fx), 0, "a bare Enter went out on its own");
    }

    #[test]
    fn two_sends_in_consecutive_passes_cannot_be_run_together() {
        // Taking a second item while a submit is still owed would paste it on
        // top of the first and then overwrite the pending flag, so both prompts
        // would go to the agent as one message with one `Enter` — and both
        // items would show as sent.
        let mut fx = app();
        let _records = records(&mut fx, "idle");
        stays(&mut fx);
        polled(&mut fx);

        fx.app.queue.stub_item("wire-check-first", Mode::Send);
        fx.app.queue.stub_item("wire-check-second", Mode::Send);
        fx.app.queue.handle_key(key(KeyCode::Char('a'))).unwrap();
        fx.app.queue.handle_key(key(KeyCode::Enter)).unwrap();

        fx.app.pump_queue();
        reaches_the_agent(&mut fx, "wire-check-first");

        // The second item is made due *before* the pass that owes an `Enter`,
        // or the pass would decline it for want of a countdown rather than
        // because a submit is outstanding, and the test would prove nothing.
        fx.app.queue.handle_key(key(KeyCode::Tab)).unwrap();
        fx.app.queue.handle_key(key(KeyCode::Enter)).unwrap();

        fx.app.pump_queue();
        assert_eq!(keys_sent(&fx), 1, "the first prompt was never submitted");
        assert!(
            !agent_screen(&fx).contains("wire-check-second"),
            "the second prompt was pasted on top of the first: {}",
            agent_screen(&fx)
        );

        // ...and the pass after that is free to take it.
        fx.app.pump_queue();
        reaches_the_agent(&mut fx, "wire-check-second");
        assert_eq!(
            keys_sent(&fx),
            1,
            "the second send inherited the first one's Enter instead of owing its own"
        );
    }

    #[test]
    fn the_queue_is_a_workspace_view_and_f2_remembers_it() {
        let mut app = app();
        screen(&mut app, 120, 24);

        app.handle_key(alt(KeyCode::Char('a'))).unwrap();
        assert_eq!(app.right_view, RightView::Queue);
        // Pointedly not like Alt+S: the common case is glancing at what is
        // still queued while the agent works and you keep typing at it.
        assert_eq!(app.focus, Focus::Left);

        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Diag);
        app.handle_key(key(KeyCode::F(2))).unwrap();
        assert_eq!(app.right_view, RightView::Queue);
    }

    /// The whole wire for F3: the global table resolves it, `act` dispatches it,
    /// and the reader repaints. Each end is tested on its own — this is the only
    /// thing that would catch the dispatch arm being dropped in the middle.
    #[test]
    fn f3_repaints_the_reader_and_leaves_the_rest_of_the_frame_alone() {
        let mut fx = app();
        fx.handle_key(alt(KeyCode::Char('e'))).unwrap();

        let before = page(&mut fx, 120, 24);
        fx.handle_key(key(KeyCode::F(3))).unwrap();
        let after = page(&mut fx, 120, 24);
        assert_ne!(before, after, "F3 never reached the reader");

        // The left pane and the borders are not the reader's to repaint, so
        // most of the frame is untouched — this would fail if the palette had
        // leaked out of the viewer's rect into the shell's chrome.
        let same = before.iter().zip(&after).filter(|(a, b)| a == b).count();
        assert!(
            same > before.len() / 2,
            "F3 repainted {} of {} cells — that is more than the reader",
            before.len() - same,
            before.len()
        );

        // And back, so the key is a toggle rather than a one-way trip.
        fx.handle_key(key(KeyCode::F(3))).unwrap();
        assert_eq!(page(&mut fx, 120, 24), before);
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
    fn a_shell_running_behind_another_view_does_not_redraw_the_agent() {
        // The expensive mistake this app can make: a build running in the
        // command view while you read git, asking for a frame every time it
        // prints a line. A frame re-renders the agent's whole screen, so that
        // is the agent's typing latency spent on a pane nobody is looking at.
        let mut app = app();
        // The platform's plainest shell rather than whatever `ABEAM_SHELL` or
        // the candidate search would pick: this test is about the pane's
        // bookkeeping, and `pwsh` costs a second of startup to prove the same
        // thing.
        app.app.shell = ShellPane::new(app.dir.path().to_path_buf(), Some(A_PLAIN_SHELL.into()));

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
        //
        // There is nothing to wait for on Unix, and that is the point rather
        // than an omission: no opening DSR is ever sent there, so the counter
        // stays at zero for the whole of a healthy session. Waiting would spend
        // five seconds establishing that. See `panes::diag` — the row keeps its
        // counter and loses its alarm, which is what makes the last assertion
        // below true on both platforms for opposite reasons: on Windows because
        // the query was answered, on Linux because there is no alarm to draw.
        if cfg!(windows) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while app.left.diagnostics().dsr_replies == 0 && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let text = screen(&mut app, 120, 24);
        assert!(text.contains("DSR answered"), "got: {text}");
        assert!(text.contains("pty size"), "got: {text}");
        // On Windows the alarm is what makes this pane worth having: it means
        // the session is hung on the opening handshake, not merely slow. See
        // docs/conpty-findings.md.
        //
        // Both halves of it, because they are spelled differently and only the
        // title used to be pinned: the title reads `pty · no DSR reply` and the
        // body note opens `No DSR reply yet.`, so a case-sensitive check for
        // the title passes with the note sitting on screen underneath it. The
        // second assertion is on `ConPTY` rather than on a phrase because it is
        // the one word in that note which wrapping cannot split in half.
        assert!(!text.to_lowercase().contains("no dsr reply"), "got: {text}");
        assert!(!text.contains("ConPTY"), "got: {text}");
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
        // The rule the whole design rests on: typing goes to the agent, and
        // nothing that happens in the background may quietly change that.
        let mut app = app();
        app.viewer.follow(PathBuf::from("whatever.md"));
        app.git.request_refresh();

        screen(&mut app, 120, 24);
        assert_eq!(app.focus, Focus::Left);
        assert_eq!(app.right_view, RightView::Git);
    }

    #[test]
    fn esc_in_the_right_pane_gives_focus_back_and_esc_in_the_agent_does_not() {
        let mut app = app();
        // Focus only moves if there is a right pane to move to, which the last
        // drawn frame is what decides.
        screen(&mut app, 120, 24);
        app.handle_key(key(KeyCode::F(5))).unwrap();
        assert_eq!(app.focus, Focus::Right);

        app.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.focus, Focus::Left);

        // ...and now Esc is the agent's, as it must be: it is how you leave a
        // prompt in both the agents abeam knows, and abeam stealing it would be
        // unusable.
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
    fn a_narrow_window_gives_the_whole_screen_to_the_agent() {
        let mut app = app();
        screen(&mut app, 120, 24);
        app.handle_key(key(KeyCode::F(5))).unwrap();
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
    fn the_next_key_after_the_escape_hatch_reaches_the_agent_verbatim() {
        // It must never be possible for abeam to permanently shadow a binding
        // of the program it is hosting, whichever one that is.
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
            key(KeyCode::F(5)),
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
        // re-renders the agent's whole screen, so a key a pane declined must
        // not cost one. Release events matter most — Windows sends one for every
        // keystroke, so treating them as news doubles the frame rate of typing.
        let mut app = app();
        let redraws = |flow: Flow| matches!(flow, Flow::Continue { redraw: true });

        let mut release = alt(KeyCode::Char('e'));
        release.kind = KeyEventKind::Release;
        assert!(!redraws(app.handle_event(Event::Key(release)).unwrap()));

        assert!(redraws(
            app.handle_event(Event::Key(alt(KeyCode::Char('g'))))
                .unwrap()
        ));
        assert!(redraws(app.handle_event(Event::Resize(80, 24)).unwrap()));

        // Focused on the git view with nothing to scroll, `j` changes nothing.
        screen(&mut app, 120, 24);
        app.handle_key(key(KeyCode::F(5))).unwrap();
        assert_eq!(app.focus, Focus::Right);
        assert!(!redraws(
            app.handle_event(Event::Key(key(KeyCode::Char('j'))))
                .unwrap()
        ));
    }

    #[test]
    fn a_release_event_is_dropped_before_it_can_fire_a_binding_twice() {
        // Windows sends Press and Release for every key. `encode_key` filters
        // releases, but the shell matches its own bindings first — without the
        // filter in `handle_event`, Alt+E would switch to the viewer and Alt+G
        // straight back on the release of the same press.
        //
        // The release is built here rather than typed, so this runs everywhere
        // even though only one platform produces one unasked. That is the right
        // way round: the filter is unconditional, a terminal that negotiates
        // the kitty keyboard protocol reports releases on any platform, and a
        // guard tested on one of two is a guard half tested.
        let mut app = app();
        let mut release = alt(KeyCode::Char('e'));
        release.kind = KeyEventKind::Release;
        app.handle_event(Event::Key(release)).unwrap();
        assert_eq!(app.right_view, RightView::Git);
    }

    #[test]
    fn the_first_frame_is_owed_immediately() {
        // The floor exists to stop frames piling up, not to delay the one that
        // opens the session. A new clock that made the caller wait 8 ms would
        // put that straight back into startup.
        assert!(Frames::new().due());
    }

    #[test]
    fn a_frame_holds_the_floor_and_then_lifts_it() {
        let mut f = Frames::new();
        f.record(Instant::now());
        assert!(
            !f.due(),
            "two frames back to back is what the floor forbids"
        );
        assert!(f.due_in() <= MIN_FRAME);

        std::thread::sleep(MIN_FRAME);
        assert!(f.due(), "and it has to lift on its own");
        assert!(f.due_in().is_zero());
    }

    #[test]
    fn pacing_is_start_to_start_so_one_slow_frame_is_not_two() {
        // If the floor were measured from when a frame *finished*, a frame that
        // overran would push the next one out by its own cost on top of the
        // gap — turning a single hitch into a visible stutter, which is the
        // exact thing this whole mechanism exists to avoid.
        let mut f = Frames::new();
        f.record(Instant::now() - (MIN_FRAME * 3));
        assert!(f.due(), "the gap has already elapsed; do not wait it again");
    }

    #[test]
    fn the_clock_reports_the_worst_frame_rather_than_an_average() {
        // An average is the wrong statistic here: a hitch is one frame in a
        // hundred, and the mean of a hundred good frames and one bad one looks
        // like a hundred and one good frames.
        let mut f = Frames::new();
        f.record(Instant::now());
        let quick = f.stats().worst_ms;

        f.record(Instant::now() - Duration::from_millis(40));
        let s = f.stats();
        assert_eq!(s.drawn, 2);
        assert!(s.worst_ms >= 40.0, "worst was {} ms", s.worst_ms);
        assert!(s.worst_ms > quick);
        // The window has not closed yet, so what it reports is the window in
        // progress — which beats a confident zero for the first second of a
        // session, when somebody watching for a stall is most likely looking.
        assert!(s.last_ms >= 40.0, "last was {} ms", s.last_ms);
    }
}
