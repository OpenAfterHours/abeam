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
use std::path::{Path, PathBuf};
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
use crate::paths;
use crate::watch::{Change, Watch};
use crate::workspace::{self, Worktree};

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

/// How often `git worktree list` is asked what worktrees exist.
///
/// Slower again than the roster, because nothing waits on it. The list decides
/// how watcher events are routed, and the thing that changes it is somebody
/// running `git worktree add` — which happens a few times a day, in another
/// terminal, and is worth learning about in ten seconds rather than in one.
///
/// The first pass runs it immediately all the same. Until it answers, abeam
/// knows about exactly one workspace and routes every change to it, which is
/// the old behaviour and therefore the old bug; ten seconds of that at startup
/// would be ten seconds of a neighbouring agent's writes landing in this
/// window before the fix switched on.
///
/// **This interval is how stale the routing rule is allowed to be**, and that
/// is inherent to polling rather than a gap in it. A worktree a neighbour
/// created has its whole checkout owned by the enclosing root until the next
/// pass, and one they removed keeps its former parents out of
/// [`workspace::is_evidence`] for the same window.
/// `crate::workspace::is_evidence` is where both directions are written out,
/// because that is the function a reader is standing in when it matters.
const WORKTREES_EVERY: Duration = Duration::from_secs(10);

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

/// One workspace the right pane can be pointed at.
///
/// **The left pane is not in here, and that is the whole shape of the feature.**
/// A live child's working directory belongs to the child: there is no call that
/// moves a running process to another directory, so the agent stays where it was
/// started for as long as it runs. The asymmetry is real, so it is surfaced
/// rather than hidden — the border names the workspace the *right* pane is on,
/// and the worktree list marks the agent's own root separately from the one
/// being read.
struct Space {
    root: PathBuf,
    label: String,
    /// One per workspace, cold until drawn. The pane that cannot be re-rooted:
    /// a live child's cwd belongs to the child.
    ///
    /// So switching workspaces with the command view up spawns a *second* child
    /// on the next frame, which is deliberate and is the same lazy rule abeam
    /// has always had — a session that never presses `Alt+S` never pays for a
    /// shell. The cost is named rather than denied: the number of shell
    /// processes grows with the number of workspaces somebody has typed in, and
    /// each of them holds abeam open at `Alt+Q` until it is finished with.
    shell: ShellPane,
    /// Whether the one watcher can see it. False for a worktree outside the
    /// agent's root, which falls back to the git pane's own two-second poll.
    watched: bool,
}

impl Space {
    fn new(root: PathBuf, label: String, watched: bool) -> Self {
        Self {
            // Read per space rather than once and cloned, so that the answer is
            // the same one `App::new` would have given: it is a setting, and
            // reading it twice from the same process cannot disagree.
            shell: ShellPane::new(root.clone(), std::env::var("ABEAM_SHELL").ok()),
            root,
            label,
            watched,
        }
    }
}

pub struct App {
    left: TerminalPane,
    git: GitPane,
    viewer: ViewerPane,
    /// Every workspace the right pane knows about, and which of them it is on.
    ///
    /// **Two invariants, upheld by [`App::sync_workspaces`] and
    /// [`App::set_workspace`] and relied on by every accessor below.**
    /// `spaces[0].root` is the agent's own root and is never removed — it is the
    /// one workspace that exists before git has said anything and the one that
    /// survives git saying nothing. And `at < spaces.len()`, so
    /// [`App::workspace`] can index rather than answer an `Option` nobody has a
    /// sensible fallback for.
    ///
    /// A `Vec` and an index rather than a map from root to workspace, and that
    /// is a borrow decision rather than a taste one: `at` is a `Copy` `usize`
    /// that can be read *before* the index, which is what keeps
    /// [`App::right_pane_ref`] a plain `&self` method. A map would need the key
    /// borrowed from `self` to look up in `self`.
    spaces: Vec<Space>,
    at: usize,
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
    /// When `git worktree list` was last asked for.
    worktrees_at: Instant,
    /// Every worktree of the repository on screen, as git last described them.
    ///
    /// Held here rather than in a pane because it is not any one pane's: it is
    /// what [`App::route`] uses to decide whose change a watcher event was, and
    /// the watcher is the shell's. Empty until the first discovery answers, and
    /// empty for ever on a machine with no git — both of which route exactly as
    /// abeam did before this existed, because [`App::workspace_roots`] puts the
    /// agent's own root in the list whatever git said.
    worktrees: Vec<Worktree>,
    /// The background-agent roster, kept here as well as handed to the queue.
    ///
    /// Two readers, one process: the queue reports what a dispatched task is
    /// doing, and the worktree list reports who is working in which checkout.
    /// A clone of a handful of small records is cheaper than a second
    /// `claude agents --json`, and asking the queue for it back would make the
    /// queue the owner of a fact that is not about the queue.
    roster: Vec<crate::agentstate::Session>,
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
    worktrees_running: bool,
    /// Something has been dispatched at least once, so the roster is worth
    /// asking for. Sticky, and the reason a session that only ever uses the
    /// first mode never starts a `claude agents` process.
    dispatched_any: bool,
    /// The worktree list has been asked for at least once, so the roster is
    /// worth asking for on its account too. Sticky, for the same reason
    /// [`dispatched_any`](Self::dispatched_any) is.
    ///
    /// Two flags rather than one, and not because they are hard to tell apart.
    /// `claude agents --json` is a *process*, and the rule the older flag was
    /// written to keep is that a session which never uses a feature never
    /// starts it. Occupancy — who is working in which worktree — needs the same
    /// roster, so the honest change is a second reason to want it, not the
    /// deletion of the first: `dispatched_any || worktrees_wanted`, where
    /// dropping the gate outright would have started that process in every
    /// session abeam has ever run.
    ///
    /// Set by [`App::pump`] once `GitPane::wants_worktrees` says the list has
    /// been opened. [`App::roster_is_wanted`] is the whole of the gate and has
    /// a test.
    worktrees_wanted: bool,
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
    /// What `git worktree list` said. Never an error: `crate::workspace`
    /// answers a failure with an empty list, because there is nothing here that
    /// could act on the difference — no git and no worktrees route identically.
    Worktrees(Vec<Worktree>),
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
        let watch_started = watch.is_some();
        let mut viewer = ViewerPane::new(root.clone());
        // Told rather than discovered, so a pane that will never update says so
        // on screen instead of looking like one that simply never notices.
        viewer.set_watching(watch_started);
        // Before the first frame, so the reader's page is right the first time
        // it is drawn rather than repainted a frame later.
        viewer.set_theme(opening.theme);

        Self {
            left,
            git: GitPane::new(root.clone()),
            viewer,
            // The agent's own root, and the invariant that it is `spaces[0]`
            // and stays there. No child in it yet: it is spawned by the first
            // frame that draws it, so a session that never asks for a command
            // line never pays for one.
            spaces: vec![Space::new(
                root.clone(),
                workspace::dir_label(&root),
                watch_started,
            )],
            at: 0,
            queue: QueuePane::new(root.clone(), agent),
            probe,
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
            // And far enough back that the first pass asks git, for the reason
            // [`WORKTREES_EVERY`] gives: until it answers, every neighbouring
            // worktree's writes land in this window.
            worktrees_at: Instant::now() - WORKTREES_EVERY,
            worktrees: Vec::new(),
            roster: Vec::new(),
            draft_open: false,
            work_tx,
            work_rx,
            roster_running: false,
            dispatch_running: false,
            worktrees_running: false,
            dispatched_any: false,
            worktrees_wanted: false,
            submit_pending: false,
            root,
            agent: agent.to_string(),
        }
    }

    // --- the workspaces --------------------------------------------------

    /// The workspace the right pane is on.
    ///
    /// Indexes rather than answering an `Option`, on the invariant
    /// [`spaces`](Self::spaces) states: `at` is only ever set by
    /// [`set_workspace`](Self::set_workspace), which refuses an index it cannot
    /// use, and [`sync_workspaces`](Self::sync_workspaces) never removes the
    /// space `at` points at.
    fn workspace(&self) -> &Space {
        &self.spaces[self.at]
    }

    fn shell(&self) -> &ShellPane {
        &self.workspace().shell
    }

    /// The same, mutably. `at` is read into a local *before* the index, which
    /// is the whole reason [`spaces`](Self::spaces) is a `Vec` and an index
    /// rather than a map: a key borrowed from `self` cannot be used to look up
    /// in `self`.
    fn shell_mut(&mut self) -> &mut ShellPane {
        let at = self.at;
        &mut self.spaces[at].shell
    }

    /// Is there a live child in *any* workspace's command view?
    ///
    /// Every one of them, not just the one on screen, because that is what
    /// quitting has to ask. A `cargo build` left running in a worktree somebody
    /// has since switched away from is exactly as alive as one in front of them,
    /// and `Alt+Q` killing it without asking is the decision abeam does not get
    /// to make on its own.
    fn any_shell_live(&self) -> bool {
        self.spaces.iter().any(|space| space.shell.is_live())
    }

    /// Point the right pane at another workspace.
    ///
    /// Returns whether a frame is owed. Switching to the workspace already on
    /// screen is not one: re-rooting in place would throw away the document the
    /// reader has open and put the git pane back to "reading the repository…"
    /// to arrive at the state it is already in.
    ///
    /// **The probe, the queue and the dispatcher are deliberately untouched.**
    /// They are the *agent's*, not the view's. The probe reads the record of the
    /// session in the left pane, which has not moved and cannot; the queue types
    /// into that session; and `crate::dispatch` starts background agents beside
    /// it. Re-rooting any of them would mean a prompt queued for the agent being
    /// aimed at a directory the agent is not in — which `crate::agentstate` is
    /// explicit is the one mistake in this program nobody would see happen.
    fn set_workspace(&mut self, ix: usize) -> bool {
        if ix >= self.spaces.len() || ix == self.at {
            return false;
        }
        self.at = ix;

        let root = self.spaces[ix].root.clone();
        let watched = self.spaces[ix].watched;
        self.git.set_root(root.clone());
        self.viewer.set_root(root);
        // Told rather than discovered, and it is the honest answer for a
        // worktree the one watcher cannot reach: that pane will not update on
        // its own, and saying so is the difference between slow and broken.
        self.viewer.set_watching(self.watch.is_some() && watched);
        true
    }

    /// Fold a fresh discovery into the workspaces already open.
    ///
    /// **Reconciled by root, never by index, and `at` is never moved.**
    /// Discovery runs on a worker thread every ten seconds and a switch happens
    /// on a keystroke, so the two race by construction: a list built before the
    /// switch can land after it, and anything that identified a workspace by its
    /// position in the previous answer would silently re-point the right pane at
    /// a different worktree. `crate::paths::same_dir` is the comparison
    /// throughout, because git spells a path its own way — forward slashes on
    /// Windows — and `==` would answer that the directory abeam is standing in
    /// is not the one git just described.
    ///
    /// Three workspaces are never removed, and each refusal is a specific
    /// failure:
    ///
    /// - **index 0**, the agent's own root, because it is where the left pane
    ///   is and there would be nothing to fall back to.
    /// - **the one `at` points at**, because removing it is the invariant above
    ///   broken and the right pane pointing at a workspace that is gone.
    /// - **any workspace with a live child in it.** `git worktree remove` while
    ///   a `cargo build` is running there must mark the worktree stale, not kill
    ///   the build. A retained workspace drops off the list — the list is built
    ///   from what git said — so it is unlisted rather than unreachable-and-
    ///   still-running-invisibly, and switching away from it is a one-way trip
    ///   until the build finishes. That is the cost, and it is smaller than the
    ///   alternative.
    fn sync_workspaces(&mut self, found: &[Worktree]) {
        for worktree in found {
            let label = workspace::label_of(worktree);
            let watched = self.watch.is_some() && paths::under(&self.root, &worktree.root);
            match self
                .spaces
                .iter_mut()
                .find(|space| paths::same_dir(&space.root, &worktree.root))
            {
                // A branch name changes under a worktree somebody checked out
                // in another terminal, and the border is drawn from this.
                Some(space) => {
                    space.label = label;
                    space.watched = watched;
                }
                None => self
                    .spaces
                    .push(Space::new(worktree.root.clone(), label, watched)),
            }
        }

        // An empty answer is every failure discovery has: no git on the machine,
        // a directory that is not a repository, a git too old for the `-z` the
        // parser needs. None of them is evidence that a worktree has gone, and
        // treating them as such would close every workspace on screen the first
        // time git was busy.
        if found.is_empty() {
            return;
        }

        let keep: Vec<bool> = self
            .spaces
            .iter()
            .enumerate()
            .map(|(ix, space)| {
                ix == 0
                    || ix == self.at
                    || space.shell.is_live()
                    || found
                        .iter()
                        .any(|worktree| paths::same_dir(&worktree.root, &space.root))
            })
            .collect();
        // Remembered before the retain and looked up again after it, because
        // the whole point is that an index does not survive a list changing
        // length underneath it.
        let at_root = self.workspace().root.clone();
        let mut ix = 0;
        self.spaces.retain(|_| {
            let keeping = keep[ix];
            ix += 1;
            keeping
        });
        self.at = self
            .spaces
            .iter()
            .position(|space| paths::same_dir(&space.root, &at_root))
            // Unreachable: the space `at` pointed at is one of the three that
            // are never removed. Falling back to the agent's root rather than
            // panicking, because a wrong workspace is a thing somebody can see
            // and press a key about, and an aborted session is not.
            .unwrap_or(0);
    }

    /// Rebuild the worktree list the git pane draws, from everything that feeds
    /// it: what git said, what Claude said, and where this window is standing.
    ///
    /// Called wherever any of those three changes, rather than on a timer,
    /// because two of them already arrive on the worker channel and the third is
    /// a keystroke. Returns whether a frame is owed — the pane answers `false`
    /// unless the list is the mode actually on screen.
    fn refresh_worktree_rows(&mut self) -> bool {
        let rows = workspace::rows(
            &self.worktrees,
            &self.roster,
            &self.root,
            &self.workspace().root,
            self.watch.as_ref().map(|_| self.root.as_path()),
        );
        self.git.set_worktree_rows(rows)
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
            if self.agent_exit.is_some() && !self.any_shell_live() {
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

        // **Every** workspace's, not just the one on screen. A `try_wait` is
        // the only thing that reaps a child, so a shell nobody has switched
        // back to could never be observed to have exited — `any_shell_live`
        // would go on reporting a live child for the rest of the session, and
        // `Alt+Q` would go on asking twice about a process that finished
        // minutes ago.
        //
        // What does *not* change is which of them earns a frame. A `cargo
        // build` in the command view makes its pane dirty on almost every pass
        // of this loop, and a frame re-renders the agent's entire screen — so
        // output counts only while it is the view on screen, and now only while
        // it is also the workspace on screen.
        let at = self.at;
        let mut shell_dirty = false;
        for (ix, space) in self.spaces.iter_mut().enumerate() {
            let dirty = space.shell.tick();
            if ix == at {
                shell_dirty = dirty;
            }
        }
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
            redraw |= self.route(change);
        }

        // The list being opened once is what pays for the occupancy column, and
        // the flag is sticky on this side so the pane never has to be asked
        // twice about the same session. See [`App::roster_is_wanted`].
        self.worktrees_wanted |= self.git.wants_worktrees();

        // **Before the open request below, and the order is load-bearing.** The
        // loop drains every pending event before it pumps, so `Enter` on a file
        // and a switch to another workspace can arrive in the same batch —
        // press `Enter`, then `w`, then `Enter`, faster than one frame.
        // `GitPane::set_root` clears the open request precisely because it holds
        // a porcelain path aimed at the toplevel being left behind, and doing
        // the switch first is what lets that clearing reach a request made in
        // the same batch. Drained the other way round, the stale `Enter` would
        // be resolved against the *new* workspace's toplevel and open whatever
        // file happens to sit at that path there — with no error at all,
        // because the file exists.
        if let Some(root) = self.git.take_workspace_request() {
            // By root, never by index: the row was drawn from a discovery that
            // may already have been superseded.
            let found = self
                .spaces
                .iter()
                .position(|space| paths::same_dir(&space.root, &root));
            if let Some(ix) = found
                && self.set_workspace(ix)
            {
                self.refresh_worktree_rows();
                redraw = true;
            }
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

    /// Hand one batch of watcher news to the panes that own it, and to no
    /// others.
    ///
    /// One recursive watch covers the agent's root, and Claude Code makes git
    /// worktrees inside that root and runs other agents in them. So a batch is
    /// not automatically about the workspace on screen: before this, every file
    /// a neighbouring agent wrote in `<root>/.claude/worktrees/other` refreshed
    /// this window's git pane and pulled that agent's scratch markdown into
    /// this window's reader, with nothing saying where it came from.
    ///
    /// The rule is `crate::workspace`'s and the argument for it is there:
    /// **innermost ownership**. A path belongs to the longest known root that
    /// contains it, and a pane takes it only when that root is the one the pane
    /// is on. A prefix test cannot do this — a nested worktree's paths really
    /// do begin with the outer root, which is the whole of why the naive fix
    /// fixes nothing.
    ///
    /// **Nothing here is unconditional any more, and that is the second half of
    /// the change.** A batch used to earn a frame merely by being non-empty;
    /// now a neighbouring agent's churn must cost nothing at all, or the fix
    /// would trade a wrong refresh for a wasted one. A frame re-renders the
    /// agent's entire screen — the discipline `tick_panes` and `crate::pane`
    /// already keep.
    fn route(&mut self, change: Change) -> bool {
        let roots = self.workspace_roots();
        // The workspace the *right* pane is looking at, which is no longer the
        // agent's root by definition: the routing question is "is this workspace
        // the one on screen", and the answer is about the panes being fed rather
        // than about where the agent is standing.
        let at = self.workspace().root.clone();
        // Two questions, and the second one is not redundant. Ownership says
        // which workspace a path belongs to; `is_evidence` says whether it says
        // anything at all, because the parent directories of a nested worktree
        // change whenever that worktree does and are owned by the workspace
        // *above* it. Without the second half a neighbour's write is dropped by
        // name and let straight back in as `<root>/.claude`.
        let mine = |path: &Path| {
            workspace::is_evidence(&roots, path)
                && workspace::owner(&roots, path).is_some_and(|owner| paths::same_dir(owner, &at))
        };

        let mut redraw = false;

        // An overflowed batch threw away the paths that would have answered
        // this, so it is taken as "assume everything changed" — which is what
        // every pane assumed before any of them could tell one workspace from
        // another. Refreshing on somebody else's branch switch costs one
        // `git status`; not refreshing on our own costs a pane that is quietly
        // wrong until its two-second poll catches up.
        if change.overflowed || change.changed.iter().any(|path| mine(path)) {
            // Coalescing is the pane's, and it is deliberate: a burst of
            // saves costs one extra refresh rather than one per file.
            self.git.request_refresh();
            redraw = true;
        }

        for path in change.markdown {
            if !mine(&path) {
                continue;
            }
            // Queued, never shown from here. The viewer takes it up on the
            // frame it is actually drawn, so nothing pulls the pane out
            // from under someone reading git.
            self.viewer.follow(path);
            // The queue changed even if no pane's content did: the border's
            // unread mark is drawn from it.
            redraw = true;
        }

        redraw
    }

    /// Every workspace root abeam knows about, for [`workspace::owner`] to pick
    /// between.
    ///
    /// The agent's own root is first and is in the list unconditionally, which
    /// is what makes every failure of discovery harmless. No git on the
    /// machine, a directory that is not a repository, a git too old for the
    /// `-z` this parser needs: each of those leaves `worktrees` empty, and a
    /// list holding only the agent's root routes every watched path to the pane
    /// exactly as abeam did before any of this. The fix degrades to the bug it
    /// fixes, and never to a watcher that has silently stopped.
    ///
    /// It is also usually a duplicate — git names the main worktree too — and
    /// deliberately not deduplicated. The two spellings differ on Windows
    /// (`C:\…` here, `C:/…` from git), `owner` is answering with whichever it
    /// picked, and the caller compares that answer with `paths::same_dir`
    /// rather than `==`. Deduplicating would mean choosing which spelling wins,
    /// which is a decision with no right answer and no need to be made.
    fn workspace_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::with_capacity(self.worktrees.len() + 1);
        roots.push(self.root.clone());
        roots.extend(self.worktrees.iter().map(|worktree| worktree.root.clone()));
        roots
    }

    /// Whether anything wants the background-agent roster enough to pay for the
    /// process that produces it.
    ///
    /// A named predicate rather than two conditions in an `if`, because what it
    /// is protecting is not obvious from either of them: `crate::agentstate`'s
    /// roster starts `claude agents --json`, and the rule is that a session
    /// which never uses a feature never starts it. See
    /// [`worktrees_wanted`](Self::worktrees_wanted) for why there are two
    /// reasons now and why neither replaced the other.
    fn roster_is_wanted(&self) -> bool {
        self.dispatched_any || self.worktrees_wanted
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

        // Only while something wants it. A session that never dispatches and
        // never opens the worktree list never starts this process.
        if !self.roster_running
            && self.roster_is_wanted()
            && self.roster_at.elapsed() >= ROSTER_EVERY
        {
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

        // A sibling of the block above rather than of anything in this
        // function's subject, and it is here because this is where the worker
        // channel is started and drained: "one at a time, and not on this
        // thread" is a rule that is only visible if the calls it governs are
        // written together.
        //
        // Ungated, unlike the roster. It costs one `git worktree list` every
        // ten seconds, which is a read of `.git/worktrees` and no network, no
        // index and no lock — and unlike the roster, something *does* depend on
        // the answer: [`App::route`] cannot tell a nested worktree from the
        // repository until git has said which is which, and until then every
        // neighbouring agent's writes land in this window.
        if !self.worktrees_running && self.worktrees_at.elapsed() >= WORKTREES_EVERY {
            self.worktrees_running = true;
            self.worktrees_at = Instant::now();
            let tx = self.work_tx.clone();
            let root = self.root.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Work::Worktrees(crate::workspace::discover(&root)));
            });
        }

        while let Ok(work) = self.work_rx.try_recv() {
            match work {
                Work::Roster(rows) => {
                    self.roster_running = false;
                    self.roster = rows.clone();
                    redraw |= self.queue.set_roster(rows);
                    // Occupancy is a column of the worktree list, and it is the
                    // only thing on it that changes on the roster's clock.
                    redraw |= self.refresh_worktree_rows();
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
                Work::Worktrees(found) => {
                    self.worktrees_running = false;
                    // Before `found` is moved into the field, so the
                    // reconciliation reads one list rather than borrowing
                    // `self` twice.
                    self.sync_workspaces(&found);
                    // The set the probe will let a session it has *already*
                    // identified move to. A hosted Claude that moves into a
                    // worktree keeps writing records — with a different `cwd` —
                    // and without this the exact match fails, readiness goes
                    // `Unknown`, and the queue's automatic send stalls silently
                    // and permanently.
                    //
                    // Every root git printed, which includes the worktrees
                    // Claude Code's *neighbouring* agents are running at. That
                    // is not a leak: `crate::agentstate::Probe::set_worktrees`
                    // spells out that discovery is strict and only revalidation
                    // consults this list, precisely because what is being handed
                    // over here is a list with the neighbours on it.
                    self.probe.set_worktrees(
                        found.iter().map(|worktree| worktree.root.clone()).collect(),
                    );
                    self.worktrees = found;
                    // A frame only if the list is the thing on screen. This
                    // runs every ten seconds for the whole session, and a
                    // redraw is a full re-render of the agent's screen.
                    redraw |= self.refresh_worktree_rows();
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
                if confirming || (self.left.has_exited() && !self.any_shell_live()) {
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
    ///
    /// The workspace label sits between the mark and the pane's own title, and
    /// **only when the right pane is somewhere other than the agent's root**.
    /// That is not tidiness. The pane is 46 columns; a label on every title
    /// would spend three or four of them saying the one thing that is true by
    /// default, and it would push the branch name and change count that the git
    /// title is *for* off the end of the border. Suppressed at index 0, the
    /// label appears exactly when it is news — which is also what keeps the
    /// three `tests/end_to_end.rs` assertions byte-identical.
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

        if self.at != 0 {
            spans.push(Span::styled(
                format!("{} · ", self.workspace().label),
                Style::default()
                    .fg(Color::Magenta)
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
            RightView::Shell => self.shell_mut(),
            RightView::Queue => &mut self.queue,
            RightView::Diag => &mut self.diag,
        }
    }

    fn right_pane_ref(&self) -> &dyn Pane {
        match self.right_view {
            RightView::Git => &self.git,
            RightView::Viewer => &self.viewer,
            RightView::Shell => self.shell(),
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
        assert_eq!(
            keys_sent(&fx),
            1,
            "the Enter that submits it never went out"
        );
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
        app.app.spaces[0].shell =
            ShellPane::new(app.dir.path().to_path_buf(), Some(A_PLAIN_SHELL.into()));

        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        screen(&mut app, 120, 24); // the frame that spawns it

        // Wait for the banner, so the pane is known to have produced output at
        // all — otherwise the assertion below passes for the wrong reason.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !app.tick_panes() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.shell().is_live(), "the shell should be up by now");

        // Now hide it, and keep it printing. Asked repeatedly rather than once
        // because the other panes have opening news of their own — the startup
        // walk, the first git report — and under a loaded `cargo test` there is
        // no window this test can pick that is guaranteed to be quiet. What
        // *is* guaranteed: if a hidden shell's output counted, every one of
        // these would be true, because every one of them follows fresh output.
        app.handle_key(alt(KeyCode::Char('g'))).unwrap();
        let mut quiet = false;
        for _ in 0..8 {
            app.shell_mut()
                .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap();
            std::thread::sleep(Duration::from_millis(60));
            quiet |= !app.tick_panes();
        }
        assert!(quiet, "a hidden shell's output must not cost a frame");

        // ...and the same output does earn one when it is the view on screen,
        // or the pane would be frozen rather than merely quiet.
        app.handle_key(alt(KeyCode::Char('s'))).unwrap();
        app.shell_mut()
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

    // --- whose change was that ---------------------------------------------

    /// A worktree of the fixture's repository, made as a real directory.
    ///
    /// Real, and not a `PathBuf` built out of literals, because the whole
    /// subject is what a path looks like coming back from a filesystem: the
    /// separators are the platform's, the case is whatever the disk kept, and
    /// a fixture written from the code cannot be wrong about those in the same
    /// direction the code is wrong. `git worktree list` is not run here —
    /// `crate::workspace` proves that end against a real repository, and
    /// re-proving it would make this a test of git rather than of routing.
    fn worktree_at(fx: &Fixture, rel: &str) -> Worktree {
        let root = fx.dir.path().join(rel);
        std::fs::create_dir_all(&root).expect("create the worktree directory");
        worktree(root, rel.rsplit('/').next().unwrap_or(rel))
    }

    /// The repository itself, which `git worktree list` names alongside the
    /// nested ones — so the routing list normally holds the agent's own root
    /// twice, once as abeam spelled it and once as git did.
    fn the_main_worktree(fx: &Fixture) -> Worktree {
        worktree(fx.dir.path().to_path_buf(), "main")
    }

    fn worktree(root: PathBuf, branch: &str) -> Worktree {
        Worktree {
            root,
            branch: Some(branch.to_string()),
            head: None,
            detached: false,
            bare: false,
        }
    }

    /// A real file at `rel`, and the batch a watcher would have made of it.
    ///
    /// Built directly rather than by waiting on a live watcher, because the
    /// question here is what the *shell* does with a batch. `crate::watch`
    /// proves that a real debouncer produces one, and a test that waited for
    /// one would be timing the platform rather than reading the routing.
    fn wrote(fx: &Fixture, rel: &str) -> Change {
        let path = fx.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create");
        std::fs::write(&path, b"# hello\n").expect("write");
        Change {
            markdown: if crate::watch::is_markdown(&path) {
                vec![path.clone()]
            } else {
                Vec::new()
            },
            changed: vec![path],
            overflowed: false,
        }
    }

    #[test]
    fn a_write_in_a_neighbouring_worktree_reaches_neither_pane() {
        // The bug, from the shell's side. Claude Code makes worktrees under
        // `.claude/worktrees/` inside the repository abeam is watching and runs
        // other agents in them, so one recursive watch covers two people's
        // work. `<root>/.claude/worktrees/other/NOTES.md` has `<root>` as a
        // prefix — which is why routing by prefix fixes nothing — and it used
        // to refresh this window's git pane and put that agent's scratch note
        // in front of this window's reader.
        let mut fx = app();
        let mine = the_main_worktree(&fx);
        let theirs = worktree_at(&fx, ".claude/worktrees/other");
        fx.app.worktrees = vec![mine, theirs];

        let change = wrote(&fx, ".claude/worktrees/other/NOTES.md");
        assert!(
            !fx.app.route(change),
            "somebody else's work must not even cost a frame"
        );
        assert!(
            !fx.app.viewer.has_pending(),
            "the reader was handed another agent's document"
        );

        // ...and a source file in there is no different: it is the git pane
        // this used to wake on every single write.
        let change = wrote(&fx, ".claude/worktrees/other/src/lib.rs");
        assert!(!fx.app.route(change));
    }

    #[test]
    fn a_write_in_the_workspace_on_screen_still_reaches_both_panes() {
        // The other half, and the one that makes the test above mean something:
        // a routing rule that dropped everything would pass it. This is abeam's
        // whole reason to exist — the agent writes a file and both panes
        // already know — so it has to survive the fix intact.
        let mut fx = app();
        let mine = the_main_worktree(&fx);
        let theirs = worktree_at(&fx, ".claude/worktrees/other");
        fx.app.worktrees = vec![mine, theirs];

        let change = wrote(&fx, "docs/DESIGN.md");
        assert!(fx.app.route(change), "the reader has news to show");
        assert!(fx.app.viewer.has_pending());
    }

    #[test]
    fn a_repository_git_never_answered_about_routes_exactly_as_it_used_to() {
        // Discovery fails on a machine with no git, in a directory that is not
        // a repository, and on a git too old for the `-z` the parser needs. All
        // three leave the list empty, and an empty list must degrade to the old
        // behaviour rather than to a watcher that has silently stopped —
        // "nothing is mine" is the one failure that looks exactly like the
        // feature having been deleted.
        let mut fx = app();
        assert!(fx.app.worktrees.is_empty(), "nothing has discovered yet");

        let change = wrote(&fx, "notes.md");
        assert!(fx.app.route(change));
        assert!(fx.app.viewer.has_pending());

        // Including the paths that would have belonged to a nested worktree, if
        // anything had known there was one.
        let change = wrote(&fx, ".claude/worktrees/other/NOTES.md");
        assert!(fx.app.route(change));
    }

    #[test]
    fn a_batch_too_big_to_route_refreshes_rather_than_dropping_everything() {
        // `git checkout` of a large branch overflows the batch, and an
        // overflowed batch threw away the paths that would have said whose it
        // was. Assuming it was ours costs one `git status`; assuming it was not
        // costs a pane that is wrong until its own two-second poll notices.
        let mut fx = app();
        fx.app.worktrees = vec![worktree_at(&fx, ".claude/worktrees/other")];

        let flood = Change {
            markdown: Vec::new(),
            changed: Vec::new(),
            overflowed: true,
        };
        assert!(fx.app.route(flood), "the git pane was never told");
    }

    /// An `App` over a real git repository with a real nested worktree in it,
    /// laid out exactly as Claude Code lays one out.
    ///
    /// Built before the `App` rather than after, because `App::new` starts the
    /// watcher and there is no way to hand it a directory that grew a worktree
    /// afterwards — which is also the shape of the bug: the worktree is there
    /// first and the window opens into it.
    ///
    /// `None` when git is not on this machine. That is a skip rather than a
    /// failure, and it is the only one in this file: everything else here needs
    /// a pty and a child, which every machine has, and this needs a third-party
    /// program that a minimal container may not. The one test that uses it says
    /// so out loud rather than passing quietly.
    fn app_in_a_repository_with_a_neighbour() -> Option<Fixture> {
        let dir = TempDir::new("app-worktrees");
        let root = dir.path().to_path_buf();
        a_repository_with_a_neighbour(&root).then(|| app_over(dir, root))
    }

    /// A real repository at `root`, with a real worktree where Claude Code puts
    /// one. `false` when git is not on this machine.
    fn a_repository_with_a_neighbour(root: &Path) -> bool {
        let git = |args: &[&str]| -> bool {
            std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env("GIT_AUTHOR_NAME", "abeam")
                .env("GIT_AUTHOR_EMAIL", "abeam@example.invalid")
                .env("GIT_COMMITTER_NAME", "abeam")
                .env("GIT_COMMITTER_EMAIL", "abeam@example.invalid")
                .stdin(std::process::Stdio::null())
                .output()
                .is_ok_and(|out| out.status.success())
        };

        std::fs::write(root.join("README.md"), b"# repo\n").expect("write");
        git(&["init", "-q", "-b", "main", "."])
            && git(&["add", "-A"])
            && git(&["commit", "-qm", "first"])
            && git(&[
                "worktree",
                "add",
                "-q",
                "-b",
                "other",
                ".claude/worktrees/other",
            ])
    }

    /// An `App` on `root`, with `dir` held so the watcher outlives nothing.
    ///
    /// `root` is passed rather than taken from `dir` because the two are not
    /// always the same directory: `main` resolves what `current_dir` gave it
    /// before it builds anything, and the test below is about a root where that
    /// resolution changes the answer.
    fn app_over(dir: TempDir, root: PathBuf) -> Fixture {
        let (program, args) = EXITS;
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let left = TerminalPane::spawn(program, &args, 20, 60).expect("spawn a child in a pty");
        let app = App::new(left, root, "claude", crate::config::Opening::default());
        Fixture { app, dir }
    }

    /// Wait until discovery has answered with `n` worktrees, pumping as the
    /// loop would. `false` if it never did.
    ///
    /// Discovery runs on a worker thread, and until it answers abeam knows
    /// about one workspace and routes everything to it — the old behaviour, and
    /// the old bug. Waiting is not slack in a test; it is the window
    /// [`WORKTREES_EVERY`]'s first pass exists to keep short.
    fn discovered(fx: &mut Fixture, n: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while fx.app.worktrees.len() < n && Instant::now() < deadline {
            fx.app.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        fx.app.worktrees.len() >= n
    }

    #[test]
    fn a_real_watcher_over_a_real_worktree_hands_a_neighbours_writes_to_nobody() {
        // Every other test here builds the `Change` it routes. This one builds
        // nothing: a real repository on disk, a real `git worktree add` where
        // Claude Code puts one, a real `notify` watch, and real files written
        // into both trees. Each step of it belongs to somebody else — git
        // decides how it spells the worktree path, the platform decides how
        // `notify` spells the event — and a fixture cannot be wrong about those
        // in the same direction the code is wrong, which is exactly what a
        // fixture written from the code would do.
        let Some(mut fx) = app_in_a_repository_with_a_neighbour() else {
            panic!("this test needs git on PATH; without it the claim is untested");
        };

        assert!(
            discovered(&mut fx, 2),
            "git never described the two worktrees"
        );

        // The neighbouring agent writes a document. Nothing may come of it.
        let theirs = fx.dir.path().join(".claude/worktrees/other/THEIRS.md");
        std::fs::write(&theirs, b"# not yours\n").expect("write");

        // Several debounces, which is long enough that an event still on its
        // way would have arrived.
        let settle = Instant::now() + Duration::from_millis(1500);
        let mut leaked = false;
        while Instant::now() < settle {
            leaked |= fx.app.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !fx.app.viewer.has_pending(),
            "another agent's document was put in front of the reader"
        );
        assert!(
            !leaked,
            "another agent's write cost this window a frame it had no news for"
        );

        // ...and the watcher was alive the whole time, which is the half that
        // stops the assertions above from passing for the worst possible
        // reason. The same write, one directory up, still reaches both panes.
        let mine = fx.dir.path().join("MINE.md");
        std::fs::write(&mine, b"# yours\n").expect("write");

        let deadline = Instant::now() + Duration::from_secs(20);
        while !fx.app.viewer.has_pending() && Instant::now() < deadline {
            fx.app.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            fx.app.viewer.has_pending(),
            "the watcher never reported a write in the repository on screen"
        );
    }

    /// The same repository, reached through a real junction — which is how a
    /// person who keeps `C:\src\forge` pointed at a directory on another drive
    /// reaches it every day.
    ///
    /// `mklink /J` needs no elevation, which is why the fixture can make one;
    /// `mklink /D` would need it and is not what this is about.
    #[cfg(windows)]
    fn app_in_a_repository_reached_through_a_junction() -> Option<(Fixture, PathBuf, PathBuf)> {
        let dir = TempDir::new("app-junction");
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        std::fs::create_dir_all(&real).expect("create the repository directory");
        if !a_repository_with_a_neighbour(&real) {
            return None;
        }

        let made = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|out| out.status.success());
        assert!(
            made,
            "this test needs `mklink /J`, which needs no elevation"
        );

        // Exactly `main`'s line, and the whole subject of the test: what
        // `current_dir` would have handed back is `link`, and what every root
        // git prints is `real`.
        let root = paths::resolve_root(&link);
        Some((app_over(dir, root), link, real))
    }

    #[test]
    #[cfg(windows)]
    fn a_repository_reached_through_a_junction_still_knows_whose_worktree_is_whose() {
        // The routing rule defeated in one step, and not by anything exotic:
        // `GetCurrentDirectoryW` does not resolve a junction and
        // `git worktree list --porcelain` does. Started through one, abeam's own
        // root is `…\link` while every root git names is `…\real` — two
        // different directories to `crate::paths`, which is correct — so the
        // only root in `workspace_roots` containing any watched path is the
        // agent's own, `owner` hands it every path including the ones inside a
        // neighbour's worktree, `is_evidence` has nothing to suppress, and the
        // bug two commits exist to close is back with nothing on screen saying
        // so. A `subst` drive and an 8.3 short name do the same thing.
        //
        // Everything below is somebody else's answer rather than a fixture's:
        // Windows decides what a junction resolves to, git decides how it
        // spells a worktree, `notify` decides how an event arrives.
        let Some((mut fx, link, real)) = app_in_a_repository_reached_through_a_junction() else {
            panic!("this test needs git on PATH; without it the claim is untested");
        };

        // The fixture proves something only if the two spellings really are two
        // directories to the rule that routes on them. Asserted of the fixture
        // rather than of the app's root, so that it goes on being true — and
        // goes on being the reason this test is worth running — however the
        // resolution above answers.
        assert!(
            !paths::same_dir(&link, &real),
            "the junction resolved to itself, so this test is about nothing"
        );

        assert!(
            discovered(&mut fx, 2),
            "git never described the two worktrees"
        );

        // The agent's own root is the one git named, so the list that switches
        // workspaces can say where you are — and says it once rather than
        // twice.
        let roots = fx.app.workspace_roots();
        let named = roots
            .iter()
            .filter(|root| paths::same_dir(root, &fx.app.root))
            .count();
        assert_eq!(
            named, 2,
            "the agent's root and git's main worktree are one directory under \
             two spellings, which is the duplicate `workspace_roots` documents"
        );
        assert!(
            roots.iter().any(
                |root| paths::under(&fx.app.root, root) && !paths::same_dir(root, &fx.app.root)
            ),
            "git's nested worktree is not inside the root abeam is holding"
        );

        let rows = workspace::rows(
            &fx.app.worktrees,
            &fx.app.roster,
            &fx.app.root,
            &fx.app.workspace().root,
            Some(fx.app.root.as_path()),
        );
        assert!(
            rows.iter().any(|row| row.here && row.agent_here),
            "no row is where the agent is: {:#?}",
            rows.iter().map(|row| &row.label).collect::<Vec<_>>()
        );

        // And the whole point of it: the neighbouring agent's write reaches
        // nobody. Written through the junction, because a neighbour that
        // reached the repository the same way abeam did is the ordinary case
        // and the filesystem does not care which name was used.
        let theirs = link.join(".claude/worktrees/other/THEIRS.md");
        std::fs::write(&theirs, b"# not yours\n").expect("write");

        let settle = Instant::now() + Duration::from_millis(1500);
        let mut leaked = false;
        while Instant::now() < settle {
            leaked |= fx.app.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !fx.app.viewer.has_pending(),
            "another agent's document was put in front of the reader"
        );
        assert!(
            !leaked,
            "another agent's write cost this window a frame it had no news for"
        );

        // ...with the watcher alive the whole time, which is what stops the two
        // assertions above from passing for the worst possible reason.
        std::fs::write(link.join("MINE.md"), b"# yours\n").expect("write");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !fx.app.viewer.has_pending() && Instant::now() < deadline {
            fx.app.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            fx.app.viewer.has_pending(),
            "the watcher never reported a write in the repository on screen"
        );
    }

    #[test]
    fn a_session_started_below_the_repository_can_still_switch_back_to_where_it_is() {
        // `spaces[0]` is the agent's own root and is never removed, and the `w`
        // list is what switches between workspaces — so a `spaces[0]` with no
        // row on it is a workspace nobody can get back to. That is not an
        // exotic state: **abeam started in a subdirectory of the repository**
        // produces it every time, because `git worktree list` names the
        // repository and not the directory somebody was standing in. The git
        // pane fully supports being started there; it resolves `toplevel` for
        // every open.
        //
        // Real git, because the claim is about what git names rather than about
        // what a fixture says it names.
        let dir = TempDir::new("app-subdirectory");
        let repository = dir.path().to_path_buf();
        assert!(
            a_repository_with_a_neighbour(&repository),
            "this test needs git on PATH; without it the claim is untested"
        );
        let below = repository.join("crates").join("abeam");
        std::fs::create_dir_all(&below).expect("create the subdirectory");

        let mut fx = app_over(dir, paths::resolve_root(&below));
        assert!(
            discovered(&mut fx, 2),
            "git never described the two worktrees"
        );
        assert!(
            !fx.app
                .worktrees
                .iter()
                .any(|worktree| paths::same_dir(&worktree.root, &fx.app.root)),
            "git named the subdirectory, so this test is about nothing"
        );

        let rows = workspace::rows(
            &fx.app.worktrees,
            &fx.app.roster,
            &fx.app.root,
            &fx.app.workspace().root,
            Some(fx.app.root.as_path()),
        );

        // The invariant the list has to keep, asserted of every row rather than
        // of the one this test added: `pump` looks a chosen row up in `spaces`
        // by root, so a row naming a workspace that is not there is a row that
        // does nothing when it is pressed.
        for row in &rows {
            assert!(
                fx.app
                    .spaces
                    .iter()
                    .any(|space| paths::same_dir(&space.root, &row.root)),
                "`{}` is on the list and is no workspace",
                row.root.display()
            );
        }
        assert!(
            rows.iter().any(|row| row.here && row.agent_here),
            "no row is where the agent is, so the list says you are nowhere"
        );

        // ...and switching away from it is not a one-way trip.
        let repository = fx
            .app
            .spaces
            .iter()
            .position(|space| paths::same_dir(&space.root, &repository))
            .expect("the repository is a workspace of its own");
        assert!(fx.app.set_workspace(repository));
        assert!(fx.app.set_workspace(0), "there is no way back to the agent");
        assert!(paths::same_dir(&fx.app.workspace().root, &fx.app.root));
    }

    #[test]
    fn the_roster_process_starts_for_a_reason_or_not_at_all() {
        // `crate::agentstate::roster` starts `claude agents --json`, and the
        // rule it was gated with is that a session which never uses a feature
        // never starts it. Occupancy in the worktree list needs the same
        // roster, so the gate gained a second reason rather than losing the
        // first — the failure being prevented is a process started in every
        // session abeam has ever run, to fill in a column nobody opened.
        //
        // Asserted through the predicate rather than by pumping, deliberately:
        // opening this gate for real starts that process, and no test in this
        // crate starts an agent.
        let mut fx = app();
        assert!(!fx.app.roster_is_wanted(), "nothing has asked for anything");

        fx.app.worktrees_wanted = true;
        assert!(fx.app.roster_is_wanted());

        let mut fx = app();
        fx.app.dispatched_any = true;
        assert!(fx.app.roster_is_wanted(), "the older reason still counts");
    }

    // --- pointing the right pane somewhere else ----------------------------
    //
    // The left pane is never in any of this, and that is the feature rather
    // than an omission: there is no call that moves a running process to
    // another directory, so the agent stays where it was started. Every test
    // below is about the *right* half of the window.

    /// A real directory beside the fixture's root, and a `Space` over it.
    ///
    /// Real, because `ShellPane` will be handed it as a child's working
    /// directory, and `paths::same_dir` is comparing spellings that came off a
    /// filesystem.
    fn a_second_workspace(fx: &Fixture, rel: &str) -> PathBuf {
        let root = fx.dir.path().join(rel);
        std::fs::create_dir_all(&root).expect("create the worktree directory");
        root
    }

    fn wt_row(root: &Path, label: &str, here: bool) -> workspace::Row {
        workspace::Row {
            label: label.to_string(),
            root: root.to_path_buf(),
            here,
            agent_here: here,
            occupant: None,
            watched: true,
        }
    }

    #[test]
    fn switching_workspaces_moves_the_right_pane_and_leaves_the_agents_own_alone() {
        let mut fx = app();
        let other = a_second_workspace(&fx, ".claude/worktrees/other");
        fx.app
            .spaces
            .push(Space::new(other.clone(), "other".into(), true));
        let agent_root = fx.app.root.clone();

        assert!(fx.app.set_workspace(1), "a switch is worth a frame");
        assert_eq!(fx.app.at, 1);
        assert!(paths::same_dir(&fx.app.workspace().root, &other));
        assert!(
            !fx.app.set_workspace(1),
            "re-rooting in place would throw away the open document to arrive \
             at the state it is already in"
        );

        // The probe, the queue and the dispatcher are all aimed at `root`, and
        // all three are the *agent's* rather than the view's: a prompt queued
        // for the session in the left pane must never be aimed at a directory
        // that session is not in.
        assert!(paths::same_dir(&fx.app.root, &agent_root));
        assert!(paths::same_dir(&fx.app.spaces[0].root, &agent_root));

        // What did move is where the watcher's news goes. This is the routing
        // half of `set_workspace` observed from outside: the same batch that
        // used to be somebody else's is now ours, and vice versa.
        fx.app.worktrees = vec![the_main_worktree(&fx), worktree(other.clone(), "other")];
        let change = wrote(&fx, ".claude/worktrees/other/NOTES.md");
        assert!(
            fx.app.route(change),
            "a write in the workspace on screen reached neither pane"
        );
        assert!(fx.app.viewer.has_pending());

        let change = wrote(&fx, "MINE.md");
        assert!(
            !fx.app.route(change),
            "a write in the agent's root is now another workspace's news"
        );
    }

    #[test]
    fn the_border_names_the_workspace_only_when_it_is_not_the_agents() {
        let mut fx = app();
        let title = |fx: &Fixture| -> String {
            fx.app
                .right_title(false)
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        };

        // At the agent's own root the border is exactly what it always was. The
        // pane is 46 columns and a label on every title would push the branch
        // name and change count the git title exists for off the end of it —
        // and it is what keeps the three `tests/end_to_end.rs` assertions
        // byte-identical.
        let here = title(&fx);
        assert!(here.starts_with(" git"), "{here}");
        assert!(!here.contains(&fx.app.spaces[0].label), "{here}");

        // ...and it says where you are the moment you are somewhere else.
        let other = a_second_workspace(&fx, ".claude/worktrees/review");
        fx.app.spaces.push(Space::new(other, "review".into(), true));
        fx.app.set_workspace(1);
        let elsewhere = title(&fx);
        assert!(elsewhere.contains("review · "), "{elsewhere}");
        assert!(elsewhere.contains("git"), "{elsewhere}");
    }

    #[test]
    fn a_worktree_git_has_stopped_naming_is_dropped_unless_something_needs_it() {
        let mut fx = app();
        let mine = the_main_worktree(&fx);
        let gone = worktree_at(&fx, ".claude/worktrees/gone");
        let kept = worktree_at(&fx, ".claude/worktrees/kept");

        fx.app
            .sync_workspaces(&[mine.clone(), gone.clone(), kept.clone()]);
        assert_eq!(fx.app.spaces.len(), 3);
        assert!(
            paths::same_dir(&fx.app.spaces[0].root, &fx.app.root),
            "the agent's root stopped being spaces[0]"
        );

        // `git worktree remove` in another terminal, with nothing running in
        // either of them.
        fx.app.sync_workspaces(std::slice::from_ref(&mine));
        assert_eq!(fx.app.spaces.len(), 1);
        assert_eq!(fx.app.at, 0);

        // The workspace the right pane is *on* is never removed, whatever git
        // says: `at` pointing at a space that is gone is the invariant broken
        // and a right pane looking at nothing.
        fx.app.sync_workspaces(&[mine.clone(), kept.clone()]);
        let ix = fx
            .app
            .spaces
            .iter()
            .position(|space| paths::same_dir(&space.root, &kept.root))
            .expect("the worktree git just named");
        assert!(fx.app.set_workspace(ix));
        fx.app.sync_workspaces(std::slice::from_ref(&mine));
        assert_eq!(fx.app.spaces.len(), 2);
        assert!(
            paths::same_dir(&fx.app.workspace().root, &kept.root),
            "the right pane was left pointing at a workspace that is gone"
        );

        // And an empty answer removes nothing at all. It is what every failure
        // of discovery looks like — no git, not a repository, a git too old for
        // the porcelain — and none of them is evidence that a worktree has
        // gone.
        fx.app.sync_workspaces(&[]);
        assert_eq!(fx.app.spaces.len(), 2);
    }

    #[test]
    fn discovery_racing_a_switch_reconciles_by_root_and_never_moves_the_right_pane() {
        // Discovery runs on a worker thread every ten seconds and a switch
        // happens on a keystroke, so a list built before the switch can land
        // after it — and git is under no obligation to print the worktrees in
        // the same order twice. Anything identifying a workspace by its
        // position in the previous answer would silently re-point the right
        // pane at a different worktree, which is a wrong `git status` under a
        // confident title.
        let mut fx = app();
        let mine = the_main_worktree(&fx);
        let a = worktree_at(&fx, ".claude/worktrees/a");
        let b = worktree_at(&fx, ".claude/worktrees/b");

        fx.app
            .sync_workspaces(&[mine.clone(), a.clone(), b.clone()]);
        let ix = fx
            .app
            .spaces
            .iter()
            .position(|space| paths::same_dir(&space.root, &b.root))
            .expect("b is a workspace");
        assert!(fx.app.set_workspace(ix));
        let was = fx.app.workspace().root.clone();

        // The answer from before the switch, reordered and one worktree short.
        fx.app.sync_workspaces(&[b.clone(), mine.clone()]);
        assert!(
            paths::same_dir(&fx.app.workspace().root, &was),
            "the right pane moved to another worktree on its own"
        );
        assert!(fx.app.at < fx.app.spaces.len(), "the index is out of range");
        assert!(paths::same_dir(&fx.app.spaces[0].root, &fx.app.root));
    }

    #[test]
    fn an_enter_and_a_switch_in_one_batch_never_open_a_file_in_the_wrong_tree() {
        // The loop drains every pending event before it pumps, so `Enter` on a
        // file, `w`, and `Enter` on a worktree can all arrive before a single
        // frame. The first of those holds a porcelain path, which is relative
        // to a worktree root — resolved against the workspace being switched
        // *to* it opens whatever sits at that path there, and reports no error
        // at all, because the file exists.
        let mut fx = app();
        let other = a_second_workspace(&fx, ".claude/worktrees/other");
        std::fs::write(other.join("notes.md"), b"# theirs\n").expect("write");
        fx.app
            .spaces
            .push(Space::new(other.clone(), "other".into(), true));

        fx.app.git.stub_open_request("notes.md");
        fx.app.git.set_worktree_rows(vec![
            wt_row(&fx.app.root, "main", true),
            wt_row(&other, "other", false),
        ]);
        fx.app.git.handle_key(key(KeyCode::Char('w'))).unwrap();
        fx.app.git.handle_key(key(KeyCode::Tab)).unwrap();
        fx.app.git.handle_key(key(KeyCode::Enter)).unwrap();

        assert!(fx.app.pump(), "the switch is worth a frame");
        assert_eq!(fx.app.at, 1, "the switch never happened");
        assert_eq!(
            fx.app.right_view,
            RightView::Git,
            "the stale Enter dragged the reader into view"
        );
        assert!(
            fx.app.viewer.path().is_none(),
            "a file of the workspace that was left is on screen under the name \
             of the one that was switched to"
        );
    }

    #[test]
    fn opening_the_worktree_list_is_what_asks_for_the_roster() {
        // The wire the gate's own test cannot see: the flag exists, and
        // something has to set it.
        let mut fx = app();
        assert!(!fx.app.roster_is_wanted());

        // The roster's timer pushed out of reach first, because opening this
        // gate for real starts `claude agents --json` and no test in this crate
        // starts an agent.
        fx.app.roster_at = Instant::now();
        fx.app.git.handle_key(key(KeyCode::Char('w'))).unwrap();
        fx.app.pump();
        assert!(fx.app.roster_is_wanted());
        assert!(!fx.app.roster_running, "a process was started after all");
    }

    #[test]
    fn a_shell_in_a_hidden_workspace_is_still_polled_and_still_holds_the_door() {
        // Two failures in one line of code. A child nobody has switched back to
        // is never `try_wait`ed, so it could not be observed to exit and
        // `any_shell_live` would go on reporting it for the rest of the
        // session. And quitting that read only the visible workspace's shell
        // would kill a `cargo build` in another worktree without asking — which
        // is exactly the decision abeam refuses to make on its own.
        let mut fx = app();
        let other = a_second_workspace(&fx, ".claude/worktrees/other");
        fx.app
            .spaces
            .push(Space::new(other.clone(), "other".into(), true));
        // The platform's plainest shell rather than whatever the candidate
        // search would pick: this is about the app's bookkeeping.
        fx.app.spaces[1].shell = ShellPane::new(other, Some(A_PLAIN_SHELL.into()));

        fx.app.set_workspace(1);
        fx.app.set_right_view(RightView::Shell);
        screen(&mut fx, 120, 24); // the frame that spawns it
        assert!(fx.app.any_shell_live(), "the shell should be up by now");
        assert!(
            !fx.app.spaces[0].shell.is_live(),
            "the agent's own workspace has no shell in it, so the assertions \
             below are about the hidden one"
        );

        // Hide it, workspace and all. Nothing may be drawn with the shell view
        // showing after this, or the lazy spawn would put a *second* child in
        // the workspace switched to.
        fx.app.set_right_view(RightView::Git);
        fx.app.set_workspace(0);
        assert!(fx.app.any_shell_live());

        // The fixture's own child leaves on its own, so what is holding abeam
        // open from here is entirely the shell in the workspace nobody is
        // looking at.
        let deadline = Instant::now() + Duration::from_secs(20);
        while fx.app.left.poll_exit().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(fx.app.left.has_exited(), "the fixture's child stayed");
        assert!(matches!(
            fx.app.handle_key(alt(KeyCode::Char('q'))).unwrap(),
            Flow::Continue { .. }
        ));
        assert!(
            fx.app.pending_quit,
            "Alt+Q left without asking, and took a build in another worktree \
             down with it"
        );

        // ...and the hidden child is still polled, or nothing would ever notice
        // it finishing and the door above would stay held for ever.
        for pressed in "exit".chars() {
            fx.app.spaces[1]
                .shell
                .handle_key(key(KeyCode::Char(pressed)))
                .unwrap();
        }
        fx.app.spaces[1]
            .shell
            .handle_key(key(KeyCode::Enter))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        while fx.app.any_shell_live() && Instant::now() < deadline {
            fx.app.tick_panes();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !fx.app.any_shell_live(),
            "a hidden workspace's child was never polled, so it could never be \
             seen to leave"
        );
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
