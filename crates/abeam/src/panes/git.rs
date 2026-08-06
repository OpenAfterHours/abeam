//! The git view: a read-only observer of the working tree.
//!
//! This replaces the second terminal window the user kept open purely to watch
//! git while the agent worked. It stages nothing, commits nothing, and runs no
//! command that can change the repository — every `git` invocation below is a
//! read, and that is a property worth preserving: the pane refreshes itself on
//! a timer, so anything it can do, it does unasked.
//!
//! ## Three things shape the design
//!
//! **It shells out to `git`.** The real binary already knows this user's
//! config, credentials, worktree layout and `.gitignore` rules; a library knows
//! none of that until you teach it. `--porcelain=v2` exists precisely so that
//! programs can read status, and it is stable.
//!
//! **Nothing git-shaped happens on the UI thread.** `git status` on a cold,
//! large repository can take a second, and a second spent here is a second of
//! the agent's keystrokes going nowhere. A worker thread owns every subprocess;
//! `tick` only ever does a `try_recv`.
//!
//! **The refresh timer is measured from the last *result*, not on a fixed
//! schedule.** A repo where status takes 900ms therefore backs itself off
//! instead of queueing work it cannot keep up with, and requests that arrive
//! while one is in flight collapse into a single follow-up. That is what makes
//! [`GitPane::request_refresh`] safe to wire to the file watcher later: a burst
//! of save events costs one extra refresh, not one per event.
//!
//! The parsing is the part with real edge cases in it — renames, unmerged
//! entries, paths with spaces — so it is pure functions over `&str` with
//! recorded fixtures in the tests, and no test in this file shells out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::pane::{Handled, Pane};
use crate::panes::AskRequest;
use crate::scroll::{self, Scroll};
use crate::text::{clip, clip_line, dim, elide_left, err};
use crate::workspace;

/// How long after a result lands before asking for another. Timed from
/// completion, so a slow repo throttles itself.
///
/// Most refreshes come from the shell's watcher, not from here. This is the
/// safety net for the changes the watcher deliberately cannot see — `.git` is
/// in its noise list, so a commit or a checkout made in another terminal shows
/// up on this timer instead. Two seconds is the right latency for that.
const REFRESH_AFTER: Duration = Duration::from_secs(2);

/// A refresh that outlives this is worth admitting to in the title. Anything
/// shorter would just make the border flicker every two seconds.
const SLOW_AFTER: Duration = Duration::from_millis(600);

/// Enough commits to answer "where am I", not enough to become a log viewer.
const RECENT_COMMITS: &str = "6";

// ---------------------------------------------------------------------------
// the pane
// ---------------------------------------------------------------------------

/// Which of the two lists this pane is showing.
///
/// A mode rather than a fifth right-hand view, and `crate::keys` is where the
/// argument for that lives: `Alt+G`, `Alt+E`, `Alt+S` and `Alt+A` are a set of
/// four workspace views, and a fifth spelled `F6` would be a key nobody groups
/// with the other three. The list of worktrees is not a peer of those — it is
/// how you point *this* pane somewhere else — so it is reached by a pane-local
/// key, which is exempt from the global invariant because it is only ever
/// delivered while the right pane has focus and this view is up.
enum Mode {
    Status,
    Worktrees,
}

pub struct GitPane {
    root: PathBuf,

    req: Sender<Ask>,
    res: Receiver<Answer>,
    /// Which root the answers still in flight were asked about.
    ///
    /// A monotonic counter rather than a comparison against [`root`](Self::root),
    /// and the difference shows up in one sequence: switching A → B → A within
    /// a single refresh cycle leaves an in-flight report about A that was asked
    /// for *before* the first switch, and a root comparison would accept it as
    /// current. It is not — it describes A as it was two workspaces ago. A
    /// number that only ever goes up cannot be confused that way.
    ///
    /// It is stamped on the envelope rather than inside [`Snapshot`], which is
    /// also deliberate. `Snapshot`'s equality is what keeps an idle repository
    /// at zero frames a second, and a generation inside it would make every
    /// report unequal to the last. On the envelope it also reaches `NotRepo`,
    /// `NoGit` and `Failed`, which carry no snapshot to hide a stamp in and are
    /// exactly the answers a wrong root produces.
    generation: u64,
    /// Cleared the first time the channel reports the worker gone, so a dead
    /// worker is reported once rather than spun on.
    worker_alive: bool,
    /// `Some(start)` while a refresh is out.
    inflight: Option<Instant>,
    /// A refresh was asked for while one was already running.
    again: bool,
    /// When the last result landed. The refresh timer counts from here.
    settled: Instant,
    /// The in-flight refresh has passed `SLOW_AFTER`. Title-only.
    slow: bool,

    report: Report,
    /// Built once per changed report, not per frame. Rows are width-agnostic;
    /// truncation happens at render, where the width is actually known.
    rows: Vec<Row>,
    /// Indices into `rows` of the rows a selection can land on.
    picks: Vec<usize>,
    sel: usize,
    /// The selection is remembered by path, so a background refresh cannot
    /// move it out from under someone who is reading.
    sel_path: Option<String>,
    open: Option<String>,
    /// `?` — the ask view, and the selected file to ask about if the selection
    /// names one. Drained by the shell like [`open`](Self::open) is.
    ask: Option<AskRequest>,

    scroll: Scroll,

    // --- the worktree list ------------------------------------------------
    mode: Mode,
    /// Every worktree of the repository, joined with what abeam knows about
    /// being in them. Pushed in by the shell — the discovery is on its worker
    /// channel and the occupancy comes from its roster, neither of which is
    /// this pane's to start.
    worktrees: Vec<workspace::Row>,
    wt_sel: usize,
    /// The list's own scroll, not the status list's. Sharing one would make
    /// `w` and `Esc` a round trip that loses the reader's place in whichever
    /// list they were not looking at.
    wt_scroll: Scroll,
    /// The workspace `Enter` asked for, waiting to be drained by the shell.
    workspace: Option<PathBuf>,
    /// The list has been opened at least once. Sticky, and read by the shell:
    /// occupancy comes from `claude agents --json`, which is a *process*, and
    /// the rule `crate::app` keeps is that a session which never uses a feature
    /// never starts it.
    worktrees_wanted: bool,
}

/// A refresh, and which root it is about.
///
/// The root travels with the request rather than being read from a variable the
/// worker set up once, because the pane it serves can be re-rooted underneath
/// it. A worker holding a stale root joins the porcelain paths of one worktree
/// onto the toplevel of another, so `Enter` opens a file in the wrong tree —
/// and reports no error at all, because the file it lands on exists.
struct Ask {
    generation: u64,
    root: PathBuf,
}

/// A report, and which request it answers.
struct Answer {
    generation: u64,
    report: Report,
}

impl GitPane {
    pub fn new(root: PathBuf) -> Self {
        let (req, res) = spawn_worker(root.clone());
        Self::over(root, req, res)
    }

    /// [`GitPane::new`], over channels handed in rather than a worker started.
    ///
    /// The test seam, and the reason it is worth having is the rule at the top
    /// of this file: no test here shells out. `new` starts a thread that runs
    /// `git` in whatever directory it was given, so a test about *this pane's*
    /// bookkeeping — which request went out, which answer was refused — would
    /// otherwise be a test with a subprocess in it. Handed both ends, a test
    /// can read exactly what was asked for and answer it by hand, which is
    /// stricter than a real worker rather than weaker.
    fn over(root: PathBuf, req: Sender<Ask>, res: Receiver<Answer>) -> Self {
        let mut pane = Self {
            root,
            req,
            res,
            generation: 0,
            worker_alive: true,
            inflight: None,
            again: false,
            settled: Instant::now(),
            slow: false,
            report: Report::Pending,
            rows: Vec::new(),
            picks: Vec::new(),
            sel: 0,
            sel_path: None,
            open: None,
            ask: None,
            scroll: Scroll::default(),
            mode: Mode::Status,
            worktrees: Vec::new(),
            wt_sel: 0,
            wt_scroll: Scroll::default(),
            workspace: None,
            worktrees_wanted: false,
        };
        pane.rebuild();
        pane.request();
        pane
    }

    /// A pane with no worker behind it, and the two ends a worker would have
    /// held. See [`GitPane::over`].
    #[cfg(test)]
    fn detached(root: PathBuf) -> (Self, Receiver<Ask>, Sender<Answer>) {
        let (req, asks) = mpsc::channel::<Ask>();
        let (answers, res) = mpsc::channel::<Answer>();
        (Self::over(root, req, res), asks, answers)
    }

    /// Point this pane at another worktree.
    ///
    /// Everything the old root produced has to go, and the list below is not
    /// housekeeping — each line is a specific way for one repository's answer to
    /// be drawn under another repository's name.
    ///
    /// [`Report::Pending`] is the one worth reading twice. Without it the pane
    /// goes on drawing the *other* repository's branch, change count and file
    /// list under the new workspace's title until the first refresh lands, which
    /// on a cold repository is most of a second of a confidently wrong screen.
    /// "Reading the repository…" is slower to look at and true the whole time.
    /// It is true only for as long as something really is reading, which is why
    /// [`request`](Self::request) below owns the case where nothing is: written
    /// here unconditionally and left there by a request that quietly did
    /// nothing, "reading the repository…" is the pane's account of a worker
    /// that stopped before the switch, for the rest of the session.
    ///
    /// `open` goes for the reason [`Ask`] gives. It holds a porcelain path,
    /// which is relative to a worktree root, and the `Enter` that produced it
    /// was aimed at the toplevel this pane is about to stop having.
    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root;
        // Before the request, so the request carries the new stamp and every
        // answer already in flight is now stale by construction.
        self.generation = self.generation.wrapping_add(1);
        self.inflight = None;
        self.again = false;
        self.slow = false;
        self.open = None;
        self.sel = 0;
        self.sel_path = None;
        self.scroll.to(0);
        self.report = Report::Pending;
        self.rebuild();
        self.request();
    }

    /// The worktrees of the repository, as the shell last heard them described.
    ///
    /// Returns whether a frame is owed, on [`crate::panes::QueuePane::set_roster`]'s
    /// convention: only what is on screen can owe one, and this list is on
    /// screen only while it is the mode showing. Discovery runs every ten
    /// seconds for the whole session, so a bare "it changed" here would be a
    /// full re-render of the agent's screen for a list nobody has opened.
    pub fn set_worktree_rows(&mut self, rows: Vec<workspace::Row>) -> bool {
        if self.worktrees == rows {
            return false;
        }
        self.worktrees = rows;
        self.wt_sel = self.wt_sel.min(self.worktrees.len().saturating_sub(1));
        matches!(self.mode, Mode::Worktrees)
    }

    /// The workspace the user pressed `Enter` on, if any.
    ///
    /// Draining rather than peeking, and drained unconditionally by the shell,
    /// for [`take_open_request`](Self::take_open_request)'s reason: a request
    /// left sitting fires late, at whatever unrelated moment next reads it.
    pub fn take_workspace_request(&mut self) -> Option<PathBuf> {
        self.workspace.take()
    }

    /// Whether anything has asked to see the worktree list yet.
    ///
    /// The shell reads this to decide whether the occupancy column is worth a
    /// `claude agents --json` process. See
    /// [`worktrees_wanted`](Self::worktrees_wanted).
    pub fn wants_worktrees(&self) -> bool {
        self.worktrees_wanted
    }

    /// Ask for a refresh. Idempotent and non-blocking, by design: this is the
    /// hook the file watcher will pull, and it will pull it in bursts.
    pub fn request_refresh(&mut self) {
        if self.inflight.is_some() {
            self.again = true;
        } else {
            self.request();
        }
    }

    /// The path the user pressed Enter on, if any. The shell drains this every
    /// loop iteration and hands it to the viewer.
    ///
    /// Absolute, and resolved against the repository top level rather than
    /// `self.root`: porcelain paths are relative to the worktree root, so
    /// joining them onto the directory abeam happens to have been started in is
    /// right only at the top of the tree. The worker asks git once and every
    /// report carries the answer.
    pub fn take_open_request(&mut self) -> Option<PathBuf> {
        let base = self.base().to_path_buf();
        self.open.take().map(|p| base.join(p))
    }

    /// The file `?` was pressed on, if `?` has been pressed.
    ///
    /// Draining, and drained unconditionally, for
    /// [`take_open_request`](Self::take_open_request)'s reason — this is the
    /// same wire with a different destination. The `Some(None)` case is the
    /// interesting one and [`AskRequest`] carries the argument for it: `?` on a
    /// row that names nothing readable still opens the view, because there is
    /// no other key that does.
    pub fn take_ask_request(&mut self) -> Option<AskRequest> {
        self.ask.take()
    }

    /// Where a porcelain path is resolved from.
    ///
    /// The repository top level git reported, and `self.root` only until it has
    /// said: porcelain paths are relative to the worktree root, so joining them
    /// onto the directory abeam happens to have been started in is right only
    /// at the top of the tree. One function because two callers now need the
    /// same answer, and two copies of it would be two chances to open a file in
    /// the wrong tree with no error at all — the file it lands on exists.
    fn base(&self) -> &Path {
        match &self.report {
            Report::Ok(snap) => snap.toplevel.as_deref().unwrap_or(&self.root),
            _ => &self.root,
        }
    }

    /// Stand in for the user pressing Enter on a row. Reaching the real path
    /// needs a repository in a particular state, which is a lot of setup for a
    /// test whose subject is the wire out of this pane rather than the parsing
    /// into it.
    #[cfg(test)]
    pub fn stub_open_request(&mut self, path: &str) {
        self.open = Some(path.to_string());
    }

    /// Ask the worker for a report about the current root.
    ///
    /// Returns whether the pane's own content changed, which it only ever does
    /// when there is no worker to ask — see [`stopped`](Self::stopped). Callers
    /// that always owe a frame anyway (a switch, a watcher event) discard it;
    /// [`tick`](Self::tick) is the one that has to pass it on.
    fn request(&mut self) -> bool {
        // A worker that is gone is *said* rather than silently skipped, and
        // that is this line rather than the early return it replaced. Every
        // caller here sets `report` before it asks — `set_root` writes
        // `Pending`, and `Pending` renders as "reading the repository…" — so a
        // request that quietly did nothing left the pane saying it was busy
        // reading, for ever: `inflight` stays `None` so `slow` never fires, and
        // `tick`'s `Disconnected` arm is guarded on the worker still being
        // believed alive, so nothing ever put the message back.
        if !self.worker_alive {
            return self.stopped();
        }
        self.again = false;
        let ask = Ask {
            generation: self.generation,
            root: self.root.clone(),
        };
        if self.req.send(ask).is_err() {
            self.worker_alive = false;
            return self.stopped();
        }
        self.inflight = Some(Instant::now());
        false
    }

    /// What the pane says when there is no worker behind it.
    ///
    /// One place, reached from both directions — the channel reporting the
    /// worker gone, and a request finding it already gone — because the two
    /// used to disagree about who was responsible for saying so and the answer
    /// was nobody. Returns whether a frame is owed, which is `false` once the
    /// message is already up: `tick` comes back here every `REFRESH_AFTER` for
    /// the rest of the session, and a redraw on each of those would be a full
    /// re-render of the agent's screen every two seconds for a pane that is not
    /// going to change.
    fn stopped(&mut self) -> bool {
        self.inflight = None;
        self.again = false;
        self.slow = false;

        let report = Report::Failed("the git worker stopped".into());
        if self.report == report {
            return false;
        }
        self.report = report;
        self.rebuild();
        true
    }

    /// Take one answer, or drop it because it is about a root this pane has
    /// left.
    ///
    /// A stale answer must touch **nothing** — not `inflight`, not `settled`,
    /// not `slow`. [`set_root`](Self::set_root) has already cleared all three
    /// and made a fresh request; letting a report from the old root clear them
    /// again would mark the *new* request as settled, so the pane would sit on
    /// "reading the repository…" until the two-second poll rescued it.
    fn absorb(&mut self, answer: Answer) -> bool {
        if answer.generation != self.generation {
            return false;
        }

        self.inflight = None;
        self.settled = Instant::now();
        let mut dirty = std::mem::take(&mut self.slow);
        // Comparing reports rather than redrawing on arrival is what keeps an
        // idle repo at zero frames per second.
        if answer.report != self.report {
            self.report = answer.report;
            self.rebuild();
            dirty = true;
        }
        dirty
    }

    fn selected_row(&self) -> Option<usize> {
        self.picks.get(self.sel).copied()
    }

    fn selected_path(&self) -> Option<&str> {
        self.rows.get(self.selected_row()?).and_then(Row::path)
    }

    /// The selection, if it names something the viewer could actually open.
    /// The predicate itself is [`Row::openable`], next to the rows it judges.
    fn openable_path(&self) -> Option<&str> {
        self.rows.get(self.selected_row()?).and_then(Row::openable)
    }

    fn select(&mut self, delta: isize) {
        if self.picks.is_empty() {
            return;
        }
        let n = self.picks.len() as isize;
        // Wraps: with one screenful of files, Tab from the last back to the
        // first is what a reader means, not a dead key.
        self.sel = (((self.sel as isize + delta) % n + n) % n) as usize;
        self.sel_path = self.selected_path().map(str::to_owned);
        self.reveal();
    }

    /// Bring the selected row into view without recentring — a list that jumps
    /// under you is harder to read than one that scrolls by a line.
    fn reveal(&mut self) {
        let Some(row) = self.selected_row() else { return };
        let page = self.scroll.viewport().max(1);
        if row < self.scroll.offset {
            self.scroll.to(row);
        } else if row >= self.scroll.offset + page {
            self.scroll.to(row + 1 - page);
        }
    }

    // --- content ---------------------------------------------------------

    fn rebuild(&mut self) {
        self.rows.clear();
        self.picks.clear();
        report_rows(&self.report, &self.root, &mut self.rows, &mut self.picks);

        self.sel = refind(&self.rows, &self.picks, self.sel_path.as_deref(), self.sel);
        self.sel_path = self.selected_path().map(str::to_owned);
    }

    // --- the worktree list ------------------------------------------------

    fn open_worktrees(&mut self) {
        self.mode = Mode::Worktrees;
        // The one place this is ever set. Sticky, because the roster it turns
        // on is a process and a session that opened the list once has shown
        // that it wants the column.
        self.worktrees_wanted = true;
    }

    fn wt_select(&mut self, delta: isize) -> Handled {
        if self.worktrees.is_empty() {
            return Handled::No;
        }
        let n = self.worktrees.len() as isize;
        // Wraps, like the status list's own selection: Tab from the last row
        // back to the first is what a reader means, not a dead key.
        self.wt_sel = (((self.wt_sel as isize + delta) % n + n) % n) as usize;

        // Bring it into view without recentring, for `reveal`'s reason.
        let page = self.wt_scroll.viewport().max(1);
        if self.wt_sel < self.wt_scroll.offset {
            self.wt_scroll.to(self.wt_sel);
        } else if self.wt_sel >= self.wt_scroll.offset + page {
            self.wt_scroll.to(self.wt_sel + 1 - page);
        }
        Handled::Yes
    }

    /// The keys the list owns while it is up.
    ///
    /// `Esc` is claimed rather than left to fall through, which is the one place
    /// this pane differs from every other read-only view: `crate::pane`'s
    /// `exit_hint` spells out that a sub-mode's `Esc` gives you back the list
    /// you came from rather than the agent, and the border says so.
    ///
    /// Everything else falls through to the shared scroll vocabulary and then
    /// to `No`, including `q` — it means what it has always meant here, which is
    /// back to the agent.
    fn worktree_key(&mut self, key: KeyEvent) -> Handled {
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.wt_select(1),
            KeyCode::BackTab | KeyCode::Up => self.wt_select(-1),
            KeyCode::Enter => match self.worktrees.get(self.wt_sel) {
                Some(row) => {
                    self.workspace = Some(row.root.clone());
                    // Back to the status list, because what the switch is *for*
                    // is looking at the other worktree's git. The border's
                    // workspace label is what confirms it landed.
                    self.mode = Mode::Status;
                    Handled::Yes
                }
                None => Handled::No,
            },
            KeyCode::Char('w') | KeyCode::Esc => {
                self.mode = Mode::Status;
                Handled::Yes
            }
            _ => self.wt_scroll.key(key).unwrap_or(Handled::No),
        }
    }

    fn render_worktrees(&mut self, f: &mut Frame, inner: Rect) {
        // The scrollbar takes a column from the text rather than sitting on top
        // of it, as it does in the status list.
        let text_w = inner.width - scroll::bar_width(inner.width);
        let lines = worktree_lines(&self.worktrees, text_w, self.wt_sel);
        self.wt_scroll.measure(lines.len(), inner.height as usize);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let visible: Vec<Line> = lines
            .into_iter()
            .skip(self.wt_scroll.offset)
            .take(inner.height as usize)
            .collect();
        f.render_widget(
            Paragraph::new(visible),
            Rect {
                width: text_w,
                ..inner
            },
        );
        self.wt_scroll.render_bar(f, inner);
    }
}

/// Where the selection lands in a freshly built list.
///
/// By path first, falling back to holding the index when the file it pointed at
/// has gone. A path can appear *twice* — a file staged and then edited again is
/// listed under both Staged and Changed, which is what `git status` itself
/// shows — so the candidate nearest to where the selection already was wins.
/// Taking the first match would drag the selection from one section to the
/// other on every background refresh, which is the exact thing remembering it
/// by path is here to prevent.
fn refind(rows: &[Row], picks: &[usize], want: Option<&str>, was: usize) -> usize {
    let found = want.and_then(|want| {
        picks
            .iter()
            .enumerate()
            .filter(|&(_, &r)| rows[r].path() == Some(want))
            .min_by_key(|(pick, _)| pick.abs_diff(was))
            .map(|(pick, _)| pick)
    });
    found.unwrap_or(was).min(picks.len().saturating_sub(1))
}

/// Rows for a report, and the indices of the ones a selection can land on.
///
/// A free function rather than a method so the three failure reports can be
/// rendered in a test without a pane — and therefore without a git worker, a
/// subprocess or a repository. Nothing in this file's tests shells out, and
/// that is worth keeping.
fn report_rows(report: &Report, root: &Path, rows: &mut Vec<Row>, picks: &mut Vec<usize>) {
    match report {
        Report::Pending => rows.push(Row::note("reading the repository…", dim())),
        Report::NoGit => {
            rows.push(Row::note("git is not on PATH", err()));
            rows.push(Row::note("install it, or start abeam from a", dim()));
            rows.push(Row::note("shell that can see it", dim()));
        }
        Report::NotRepo => {
            rows.push(Row::note("not a git repository", err()));
            rows.push(Row::Blank);
            rows.push(Row::note(root.display().to_string(), dim()));
        }
        Report::Failed(msg) => {
            rows.push(Row::note("git failed", err()));
            rows.push(Row::Blank);
            rows.push(Row::note(msg.clone(), dim()));
        }
        Report::Ok(snap) => build_rows(snap, rows, picks),
    }
}

impl Pane for GitPane {
    fn title(&self) -> String {
        // The list says what it is and nothing about the repository: a branch
        // name and a change count belong to one worktree, and the whole subject
        // of this mode is the several of them.
        if matches!(self.mode, Mode::Worktrees) {
            return format!("git · worktrees ({})", self.worktrees.len());
        }

        let mut t = String::from("git");
        match &self.report {
            Report::Pending => t.push_str(" · reading"),
            Report::NoGit => t.push_str(" · no git"),
            Report::NotRepo => t.push_str(" · no repo"),
            Report::Failed(_) => t.push_str(" · error"),
            Report::Ok(snap) => {
                let b = &snap.status.branch;
                t.push_str(" · ");
                match (&b.head, &b.oid) {
                    (Some(head), _) => t.push_str(head),
                    (None, Some(oid)) => t.push_str(short_oid(oid)),
                    (None, None) => t.push_str("no branch"),
                }
                if b.ahead > 0 {
                    t.push_str(&format!(" ↑{}", b.ahead));
                }
                if b.behind > 0 {
                    t.push_str(&format!(" ↓{}", b.behind));
                }
                match snap.status.dirty() {
                    0 => t.push_str(" · clean"),
                    n => t.push_str(&format!(" · {n} changed")),
                }
            }
        }
        if self.slow {
            t.push_str(" …");
        }
        t
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        if matches!(self.mode, Mode::Worktrees) {
            self.render_worktrees(f, inner);
            return;
        }

        self.scroll.measure(self.rows.len(), inner.height as usize);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // The scrollbar takes a column from the text rather than sitting on top
        // of it: an elided path is worse than a narrower one.
        let text_w = inner.width - scroll::bar_width(inner.width);

        let now = now_unix();
        let selected = self.selected_row();
        let lines: Vec<Line> = self
            .rows
            .iter()
            .enumerate()
            .skip(self.scroll.offset)
            .take(inner.height as usize)
            .map(|(i, row)| row.to_line(text_w, Some(i) == selected, now))
            .collect();

        f.render_widget(
            Paragraph::new(lines),
            Rect {
                width: text_w,
                ..inner
            },
        );
        self.scroll.render_bar(f, inner);
    }

    fn tick(&mut self) -> bool {
        let mut dirty = false;

        loop {
            match self.res.try_recv() {
                Ok(answer) => dirty |= self.absorb(answer),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // We hold the request sender, so the worker cannot have
                    // finished normally: it either never started or it panicked.
                    if self.worker_alive {
                        self.worker_alive = false;
                        dirty |= self.stopped();
                    }
                    break;
                }
            }
        }

        if let Some(started) = self.inflight
            && !self.slow
            && started.elapsed() >= SLOW_AFTER
        {
            self.slow = true;
            dirty = true;
        }

        if self.inflight.is_none() && (self.again || self.settled.elapsed() >= REFRESH_AFTER) {
            dirty |= self.request();
        }

        dirty
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        // The list owns every key while it is up, the scroll ones included:
        // there `Down` moves a selection rather than an offset, and a pane
        // cannot hand one key to two vocabularies and hope.
        if matches!(self.mode, Mode::Worktrees) {
            return Ok(self.worktree_key(key));
        }

        // One shared scroll vocabulary, so the F1 table cannot be true here and
        // false in the pane next door.
        if let Some(handled) = self.scroll.key(key) {
            return Ok(handled);
        }

        match key.code {
            // `Ctrl` plus a letter is the agent's everywhere in this program,
            // and this is the arm that keeps it so here. `crate::scroll::key`
            // hands it *back* rather than declining it — deliberately, so that
            // a pane's own table is where the decision gets made — and the
            // plain-letter arms below would otherwise take it: `Ctrl+R` asked
            // git for a fresh diff and `Ctrl+W` opened the worktree list, for
            // chords aimed at the agent. The viewer's document view has the same
            // arm, which is what makes the claim below about the two panes
            // agreeing a true one.
            KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Handled::No);
            }

            KeyCode::Tab => self.select(1),
            KeyCode::BackTab => self.select(-1),
            KeyCode::Enter => self.open = self.openable_path().map(str::to_owned),
            KeyCode::Char('r') => self.request_refresh(),
            // `w` for worktree. Free in both vocabularies this pane already
            // matches — `crate::scroll` claims `j k b space g G` and Ctrl+D/U,
            // and the four arms above claim the rest — and pane-local, so it is
            // never in front of the agent. `Alt+W` is Claude's and is not what
            // this is.
            KeyCode::Char('w') => self.open_worktrees(),
            // `?` for the ask view, about the file under the selection. Free
            // in both vocabularies this pane matches, and pane-local for `w`'s
            // reason exactly: the *intercept* paragraph at the top of
            // `crate::keys`, which is about what `global` claims before a
            // focused pane is offered anything. There is no `Alt` spelling of
            // this and there is not meant to be — a question about the file you
            // are looking at is asked from where you are looking at it.
            //
            // Ctrl and Alt are excluded by name rather than by
            // `modifiers.is_empty()`: `?` is a shifted key on most layouts, so
            // SHIFT arrives with it and must not disqualify it. The viewer's
            // arm says the same thing the same way. (The CONTROL half is
            // already covered by the arm at the top of this match; naming it
            // here keeps the two panes' rule readable as one rule rather than
            // two that happen to agree — which is the viewer's own note,
            // repeated because the two arms now really do agree.)
            KeyCode::Char('?')
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                // The path, resolved here rather than at the drain, because
                // what `?` meant is what was selected at the moment it was
                // pressed — a refresh between the two can move the selection.
                let base = self.base().to_path_buf();
                let path = self.openable_path().map(|p| base.join(p));
                self.ask = Some(AskRequest(path));
            }
            // Esc and q fall through: the shell reads an unhandled one as
            // "give focus back to the agent".
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        if matches!(self.mode, Mode::Worktrees) {
            if let Some(handled) = self.wt_scroll.mouse(ev) {
                return Ok(handled);
            }
            if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
                let row = self.wt_scroll.offset + ev.row as usize;
                if row < self.worktrees.len() {
                    self.wt_sel = row;
                    return Ok(Handled::Yes);
                }
            }
            return Ok(Handled::No);
        }

        if let Some(handled) = self.scroll.mouse(ev) {
            return Ok(handled);
        }
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let row = self.scroll.offset + ev.row as usize;
                let Some(pick) = self.picks.iter().position(|&r| r == row) else {
                    return Ok(Handled::No);
                };
                self.sel = pick;
                self.sel_path = self.selected_path().map(str::to_owned);
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    }

    /// `Esc` in the worktree list gives you back the status list, which is one
    /// press short of the agent — so the border must not promise `esc→agent`
    /// there. `crate::pane` argues the three-answer rule this is the third
    /// answer to.
    fn exit_hint(&self) -> &'static str {
        match self.mode {
            Mode::Worktrees => " · esc→git",
            Mode::Status => " · esc→agent",
        }
    }
}

// ---------------------------------------------------------------------------
// rows
// ---------------------------------------------------------------------------

/// The worktree list, one line per worktree.
///
/// Built at render because the interesting decision is what to drop when the
/// pane is 46 columns wide, and that cannot be made before the width is known.
/// The label is what survives — it is the only part naming which worktree the
/// row is — so the notes are laid out first and the label takes what is left.
fn worktree_lines(rows: &[workspace::Row], width: u16, sel: usize) -> Vec<Line<'static>> {
    let w = width as usize;
    if rows.is_empty() {
        // Two states with one screen, and saying so beats an empty box: either
        // discovery has not answered yet or it answered with nothing, and from
        // here they look the same.
        return vec![
            Line::from(Span::styled("no worktrees to show", dim())),
            Line::default(),
            Line::from(Span::styled(
                "git worktree list has said nothing yet",
                dim(),
            )),
        ];
    }

    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let gutter = if row.here { " ▸ " } else { "   " };
            let note = worktree_note(row);
            let budget = w.saturating_sub(gutter.width() + note.width());
            let label = clip(&row.label, budget);
            let pad = budget.saturating_sub(label.width());

            let style = if row.here {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let mut spans = vec![
                Span::styled(gutter, style),
                Span::styled(label, style),
                Span::raw(" ".repeat(pad)),
            ];
            if !note.is_empty() {
                spans.push(Span::styled(note, dim()));
            }

            // Clipped here and nowhere else, exactly as `Row::to_line` is: a
            // pane that overflows its rect corrupts the frame rather than
            // merely looking wrong.
            let mut spans = clip_line(Line::from(spans), w).spans;
            if i == sel {
                let used: usize = spans.iter().map(|s| s.content.width()).sum();
                spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
                Line::from(spans).style(Style::default().bg(Color::DarkGray))
            } else {
                Line::from(spans)
            }
        })
        .collect()
}

/// Everything about a worktree that is not its name, in one string.
///
/// `unwatched` is the one that has to be said. One recursive watch covers the
/// agent's root, so a worktree somewhere else on the disk refreshes on the git
/// pane's own two-second timer instead — and a pane that is merely slow looks
/// exactly like a pane that is broken unless something admits which it is.
fn worktree_note(row: &workspace::Row) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if row.agent_here {
        parts.push("agent");
    }
    if let Some(who) = &row.occupant {
        parts.push(who);
    }
    if !row.watched {
        parts.push("unwatched");
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(" {}", parts.join(" · "))
}

/// One line of the view, held in a form that does not depend on the pane width.
///
/// Rows are rebuilt when the report changes; `to_line` is what runs per frame,
/// and only for the rows actually on screen. Truncation lives there because
/// the width is not known until render, and eliding a path is the one decision
/// that has to be made against the real width.
enum Row {
    Blank,
    /// Pre-styled and clipped as a unit: branch line, section headers, notes.
    Spans(Vec<Span<'static>>),
    File {
        mark: String,
        style: Style,
        path: String,
        /// Rename or copy source.
        from: Option<String>,
        /// Added / removed line counts, when a diff knows them.
        stat: Option<(u32, u32)>,
        /// git has already told us there is no file here to read.
        gone: bool,
    },
    Commit {
        hash: String,
        at: i64,
        subject: String,
    },
}

impl Row {
    fn note(text: impl Into<String>, style: Style) -> Self {
        Row::Spans(vec![Span::styled(text.into(), style)])
    }

    fn path(&self) -> Option<&str> {
        match self {
            Row::File { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The path, when the viewer could actually open it.
    ///
    /// Two kinds of row name something that is not there to read, and both are
    /// listed on purpose. `-unormal` collapses an untracked tree into a single
    /// `dir/` entry, and a deletion is a path git has just finished telling us
    /// is gone. Handing either to the viewer switches the right pane away from
    /// git — the view the user is reading — to say "not a regular file" or "no
    /// such file", and leaves them there. That is a worse answer than Enter
    /// doing nothing.
    ///
    /// A trailing `/` and only a `/`, because every path in this module came
    /// out of `git … -z` and git spells a separator `/` on every platform it
    /// runs on — it is not the operating system's path syntax, it is git's.
    /// Accepting `\` as well would therefore never match anything extra on
    /// Windows, and on Unix would be wrong: a backslash is an ordinary byte in
    /// a file name there, so `awkward\name` is a file this would refuse to
    /// open on the grounds that it is a directory.
    fn openable(&self) -> Option<&str> {
        match self {
            Row::File {
                path, gone: false, ..
            } if !path.ends_with('/') => Some(path),
            _ => None,
        }
    }

    fn to_line(&self, width: u16, selected: bool, now: i64) -> Line<'static> {
        let w = width as usize;
        let spans = match self {
            Row::Blank => Vec::new(),

            Row::Spans(spans) => spans.clone(),

            Row::File {
                mark,
                style,
                path,
                from,
                stat,
                ..
            } => {
                let stat_text = match stat {
                    Some((a, r)) => format!(" +{a} -{r}"),
                    None => String::new(),
                };
                let name = match from {
                    Some(from) => format!("{} → {}", short_path(from), path),
                    None => path.clone(),
                };

                let gutter = format!(" {mark:<2} ");
                let budget = w.saturating_sub(gutter.width() + stat_text.width());
                // Elided from the left: the end of a path is the half that
                // identifies the file.
                let name = elide_left(&name, budget);
                let pad = budget.saturating_sub(name.width());

                let mut spans = vec![
                    Span::styled(gutter, *style),
                    Span::raw(name),
                    Span::raw(" ".repeat(pad)),
                ];
                if !stat_text.is_empty() {
                    spans.push(Span::styled(stat_text, dim()));
                }
                spans
            }

            Row::Commit { hash, at, subject } => {
                let head = format!(" {hash} {:>3} ", short_age(now - at));
                let subject = clip(subject, w.saturating_sub(head.width()));
                vec![
                    Span::styled(head, Style::default().fg(Color::Yellow)),
                    Span::styled(subject, Style::default().fg(Color::Gray)),
                ]
            }
        };

        // Every row is clipped here and nowhere else. A pane that overflows its
        // rect corrupts the frame rather than merely looking wrong, so the
        // guarantee is worth having in exactly one place.
        let mut spans = clip_line(Line::from(spans), w).spans;

        if selected {
            // Padded out to the full width, or the highlight would stop at the
            // end of the text instead of marking the row. A named colour rather
            // than an RGB one: this has to be legible on whatever terminal the
            // user has, and only file rows — which carry no dim text — are ever
            // selected.
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
            Line::from(spans).style(Style::default().bg(Color::DarkGray))
        } else {
            Line::from(spans)
        }
    }
}

fn build_rows(snap: &Snapshot, rows: &mut Vec<Row>, picks: &mut Vec<usize>) {
    rows.push(Row::Spans(head_spans(&snap.status.branch)));
    rows.push(Row::Blank);

    let status = &snap.status;

    section(
        rows,
        picks,
        "Conflicts",
        Color::Red,
        None,
        status
            .conflicts()
            .map(|e| e.listed(format!("{}{}", e.x, e.y), None))
            .collect(),
    );
    section(
        rows,
        picks,
        "Staged",
        Color::Green,
        Some(&snap.staged),
        status
            .staged()
            .map(|e| e.listed(e.x.to_string(), e.from.clone()))
            .collect(),
    );
    section(
        rows,
        picks,
        "Changed",
        Color::Yellow,
        Some(&snap.unstaged),
        // No rename source here: a worktree edit to a file that was renamed in
        // the index is not itself a rename, and the source belongs to the
        // staged entry next door.
        status
            .unstaged()
            .map(|e| e.listed(e.y.to_string(), None))
            .collect(),
    );
    section(
        rows,
        picks,
        "Untracked",
        Color::Cyan,
        None,
        status
            .untracked()
            .map(|e| e.listed("?".to_string(), None))
            .collect(),
    );

    if status.dirty() == 0 {
        rows.push(Row::note(
            "working tree clean",
            Style::default().fg(Color::Green),
        ));
        rows.push(Row::Blank);
    }

    if !snap.commits.is_empty() {
        rows.push(Row::note(
            "Recent",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
        for c in &snap.commits {
            rows.push(Row::Commit {
                hash: c.hash.clone(),
                at: c.at,
                subject: c.subject.clone(),
            });
        }
    }
}

/// One file as a section wants it. Built by [`Entry::listed`], because the mark
/// comes from a different column of the XY code depending on which side of the
/// index the section is showing.
struct Listed {
    mark: String,
    path: String,
    from: Option<String>,
    gone: bool,
}

/// One titled group of files, or nothing at all when the group is empty: a
/// heading over no rows is a row spent saying "no".
fn section(
    rows: &mut Vec<Row>,
    picks: &mut Vec<usize>,
    label: &str,
    colour: Color,
    stats: Option<&Diffstat>,
    files: Vec<Listed>,
) {
    if files.is_empty() {
        return;
    }

    let mut spans = vec![Span::styled(
        format!("{label} ({})", files.len()),
        Style::default().fg(colour).add_modifier(Modifier::BOLD),
    )];
    if let Some(d) = stats
        && (d.added > 0 || d.removed > 0)
    {
        spans.push(Span::styled(
            format!("  +{} -{}", d.added, d.removed),
            dim(),
        ));
    }
    rows.push(Row::Spans(spans));

    for f in files {
        let stat = stats.and_then(|d| d.files.get(&f.path).copied());
        picks.push(rows.len());
        rows.push(Row::File {
            mark: f.mark,
            style: Style::default().fg(colour),
            path: f.path,
            from: f.from,
            stat,
            gone: f.gone,
        });
    }
    rows.push(Row::Blank);
}

fn head_spans(b: &Branch) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    match (&b.head, &b.oid) {
        (Some(head), _) => spans.push(Span::styled(
            head.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        (None, Some(oid)) => spans.push(Span::styled(
            format!("detached at {}", short_oid(oid)),
            Style::default().fg(Color::Magenta),
        )),
        (None, None) => spans.push(Span::styled("no branch", dim())),
    }

    if b.oid.is_none() {
        spans.push(Span::styled("  no commits yet", dim()));
        return spans;
    }

    match &b.upstream {
        Some(up) => {
            spans.push(Span::styled(" → ", dim()));
            spans.push(Span::styled(up.clone(), dim()));
            if b.ahead == 0 && b.behind == 0 {
                spans.push(Span::styled("  in sync", dim()));
            }
            if b.ahead > 0 {
                spans.push(Span::styled(
                    format!("  ↑{}", b.ahead),
                    Style::default().fg(Color::Yellow),
                ));
            }
            if b.behind > 0 {
                spans.push(Span::styled(
                    format!("  ↓{}", b.behind),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
        None => spans.push(Span::styled("  no upstream", dim())),
    }
    spans
}

// ---------------------------------------------------------------------------
// the worker
// ---------------------------------------------------------------------------

/// What a refresh came back with. Compared for equality on every result, so an
/// idle repository produces no redraws at all.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Report {
    Pending,
    Ok(Box<Snapshot>),
    NotRepo,
    NoGit,
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Snapshot {
    status: Status,
    staged: Diffstat,
    unstaged: Diffstat,
    commits: Vec<Commit>,
    /// The worktree root every path in `status` is relative to. Constant for
    /// the session, so it never makes two reports compare unequal; it rides
    /// along here because the worker is the only thread allowed to ask git.
    toplevel: Option<PathBuf>,
}

fn spawn_worker(root: PathBuf) -> (Sender<Ask>, Receiver<Answer>) {
    let (req_tx, req_rx) = mpsc::channel::<Ask>();
    let (res_tx, res_rx) = mpsc::channel::<Answer>();

    // A failed spawn drops `res_tx` with the closure, and the pane's
    // disconnected-channel path reports it. Nothing here can panic the UI
    // thread, and there is deliberately no join: the thread ends when the pane
    // drops its sender.
    let _ = std::thread::Builder::new()
        .name("abeam-git".into())
        .spawn(move || {
            // Where the answers below are about. Owned by the worker rather
            // than captured once, because the pane can be pointed at another
            // worktree while a refresh is in flight — see [`Ask`] for what a
            // stale toplevel costs.
            let mut at = root;
            // Asked once per root, on the first refresh rather than at
            // construction, so that `GitPane::new` still returns without
            // waiting on a subprocess. `None` outside a repository, which is
            // also the case where nothing will ever be opened.
            let mut top = toplevel(&at);
            // The log only changes when HEAD moves, so it is cached against the
            // oid. In the common case — the agent editing files — a whole refresh
            // is one `git status`.
            let mut log_cache: Option<(String, Vec<Commit>)> = None;

            while let Ok(ask) = req_rx.recv() {
                if !crate::paths::same_dir(&ask.root, &at) {
                    at = ask.root;
                    top = toplevel(&at);
                    // Not because it would be wrong. The cache is keyed on the
                    // HEAD oid alone, and two worktrees sitting at the same oid
                    // genuinely have the same log — so leaving it would be
                    // *correct* today. It is cleared because the invariant
                    // worth keeping is "this cache is keyed on everything that
                    // varies", and the day a field enters `Commit` that depends
                    // on the worktree rather than on the commit, a cache that
                    // was quietly relying on an accident would start answering
                    // one worktree's question with another's, with nothing on
                    // screen saying so.
                    log_cache = None;
                }
                let report = collect(&at, top.clone(), &mut log_cache);
                let answer = Answer {
                    generation: ask.generation,
                    report,
                };
                if res_tx.send(answer).is_err() {
                    break;
                }
            }
        });

    (req_tx, res_rx)
}

fn toplevel(root: &Path) -> Option<PathBuf> {
    let out = run(root, &["rev-parse", "--show-toplevel"]).ok()?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

fn collect(
    root: &Path,
    toplevel: Option<PathBuf>,
    log_cache: &mut Option<(String, Vec<Commit>)>,
) -> Report {
    let raw = match run(root, &["status", "--porcelain=v2", "--branch", "-z"]) {
        Ok(out) => out,
        Err(report) => return report,
    };
    let status = parse_status(&raw);

    // Line counts cost a subprocess each, so they are only asked for when the
    // status we already have says there is something to count.
    let unstaged = if status.unstaged().next().is_some() {
        run(root, &["diff", "--numstat", "-z"])
            .map(|s| parse_numstat(&s))
            .unwrap_or_default()
    } else {
        Diffstat::default()
    };
    // `--cached` diffs against HEAD, which an unborn branch does not have.
    let staged = if status.branch.oid.is_some() && status.staged().next().is_some() {
        run(root, &["diff", "--cached", "--numstat", "-z"])
            .map(|s| parse_numstat(&s))
            .unwrap_or_default()
    } else {
        Diffstat::default()
    };

    let commits = match &status.branch.oid {
        None => Vec::new(),
        Some(oid) => match log_cache {
            Some((cached, commits)) if cached == oid => commits.clone(),
            _ => {
                // `%at` rather than `%ar`: a unix timestamp cannot be localised
                // out from under the parser, and it lets the age stay live
                // between refreshes.
                let args = ["log", "-n", RECENT_COMMITS, "--format=%h%x1f%at%x1f%s"];
                match run(root, &args) {
                    Ok(out) => {
                        let commits = parse_log(&out);
                        *log_cache = Some((oid.clone(), commits.clone()));
                        commits
                    }
                    // Not cached, so the next refresh tries again.
                    Err(_) => Vec::new(),
                }
            }
        },
    };

    Report::Ok(Box::new(Snapshot {
        status,
        staged,
        unstaged,
        commits,
        toplevel,
    }))
}

fn run(root: &Path, args: &[&str]) -> std::result::Result<String, Report> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        // `git status` refreshes the index and takes `.git/index.lock` to write
        // it back. Here abeam polls every couple of seconds; the agent runs real
        // git commands. Without this they collide, and the command that fails
        // is the *agent's*.
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Nothing here may ever prompt. A worker blocked on a credential
        // question would take every future refresh down with it, silently.
        .stdin(Stdio::null())
        .output();

    let out = match out {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Report::NoGit),
        Err(e) => return Err(Report::Failed(e.to_string())),
    };

    if !out.status.success() {
        return Err(classify(root, &String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Tell "there is no repository here" apart from "git broke".
///
/// The messages differ only in wording, and git localises its wording, so this
/// asks a question whose *exit code* answers instead. Only reached on failure,
/// which is why the extra subprocess costs nothing in the steady state.
fn classify(root: &Path, stderr: &str) -> Report {
    let probe = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output();
    match probe {
        Ok(out) if !out.status.success() => Report::NotRepo,
        _ => Report::Failed(first_line(stderr)),
    }
}

fn first_line(s: &str) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("git exited with an error");
    clip(line.trim_start_matches("fatal: "), 200)
}

// ---------------------------------------------------------------------------
// the model, and the parsers that fill it
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Status {
    branch: Branch,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Branch {
    /// `None` on an unborn branch — a repo with no commits yet.
    oid: Option<String>,
    /// `None` when HEAD is detached.
    head: Option<String>,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ordinary,
    Renamed,
    Unmerged,
    Untracked,
    Ignored,
}

impl Kind {
    fn tracked(self) -> bool {
        matches!(self, Kind::Ordinary | Kind::Renamed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    kind: Kind,
    /// Index status, `.` for unchanged. `?` / `!` for untracked / ignored.
    x: char,
    /// Worktree status.
    y: char,
    path: String,
    /// Rename or copy source.
    from: Option<String>,
}

impl Entry {
    /// This entry as a section wants it. The mark is passed in rather than
    /// derived: which column of the XY code it comes from is the caller's
    /// business, not the entry's.
    fn listed(&self, mark: String, from: Option<String>) -> Listed {
        Listed {
            mark,
            path: self.path.clone(),
            from,
            gone: self.gone(),
        }
    }

    /// There is no file at this path any more: a deletion on one side of the
    /// index or the other. `D` in *either* column, because `MD` — modified in
    /// the index, deleted in the worktree — is listed under Staged with an `M`
    /// mark and is just as absent from the disk as the `D` row beside it.
    ///
    /// Unmerged entries are deliberately not gone. `DU` and `UD` are the
    /// delete/modify conflicts, and git leaves the surviving version in the
    /// worktree for you to resolve — that file is very much there to read.
    fn gone(&self) -> bool {
        self.kind.tracked() && (self.x == 'D' || self.y == 'D')
    }
}

impl Status {
    /// A file changed in both the index and the worktree appears in both lists,
    /// which is what `git status` itself shows and what the two columns of the
    /// XY code mean.
    fn staged(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.kind.tracked() && e.x != '.')
    }

    fn unstaged(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.kind.tracked() && e.y != '.')
    }

    fn untracked(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == Kind::Untracked)
    }

    fn conflicts(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == Kind::Unmerged)
    }

    /// Files, not rows: a file staged *and* modified is one dirty file.
    fn dirty(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.kind != Kind::Ignored)
            .count()
    }
}

/// Parse `git status --porcelain=v2 --branch -z`.
///
/// `-z` rather than the newline form on purpose: with NUL terminators git never
/// C-quotes a path, so filenames with spaces, quotes or non-ASCII arrive
/// verbatim and there is no unquoting step to get wrong. The cost is that a
/// rename entry spans two records, which is the one thing this has to get
/// right — miss it and every subsequent record is read one out of step.
///
/// Unrecognised or malformed records are skipped rather than failing the parse:
/// a future git that adds a header line should cost the user nothing.
fn parse_status(out: &str) -> Status {
    let mut st = Status::default();
    let mut recs = out.split('\0').filter(|r| !r.is_empty());

    while let Some(rec) = recs.next() {
        match rec.as_bytes()[0] {
            b'#' => header(rec, &mut st.branch),
            b'1' => st.entries.extend(entry(rec, 9, Kind::Ordinary)),
            b'2' => {
                // Consume the source record unconditionally, even if the entry
                // line itself is malformed, or the stream desynchronises.
                let from = recs.next().map(str::to_owned);
                if let Some(mut e) = entry(rec, 10, Kind::Renamed) {
                    e.from = from;
                    st.entries.push(e);
                }
            }
            b'u' => st.entries.extend(entry(rec, 11, Kind::Unmerged)),
            b'?' => st.entries.extend(loose(rec, '?', Kind::Untracked)),
            b'!' => st.entries.extend(loose(rec, '!', Kind::Ignored)),
            _ => {}
        }
    }
    st
}

/// The three tracked-entry shapes differ only in how many fields sit between
/// the XY code and the path: 9 for ordinary, 10 for renamed (an extra score),
/// 11 for unmerged (three stages of mode and hash).
fn entry(rec: &str, fields: usize, kind: Kind) -> Option<Entry> {
    let mut f = rec.splitn(fields, ' ');
    f.next()?;
    let mut xy = f.next()?.chars();
    let (x, y) = (xy.next()?, xy.next()?);
    // Everything between is mode, hash and rename-score noise.
    for _ in 0..fields - 3 {
        f.next()?;
    }
    let path = f.next()?;
    if path.is_empty() {
        return None;
    }
    Some(Entry {
        kind,
        x,
        y,
        path: path.to_owned(),
        from: None,
    })
}

fn loose(rec: &str, code: char, kind: Kind) -> Option<Entry> {
    let path = rec.split_once(' ')?.1;
    if path.is_empty() {
        return None;
    }
    Some(Entry {
        kind,
        x: code,
        y: code,
        path: path.to_owned(),
        from: None,
    })
}

fn header(rec: &str, b: &mut Branch) {
    let Some((key, val)) = rec.strip_prefix("# ").and_then(|r| r.split_once(' ')) else {
        return;
    };
    match key {
        "branch.oid" => b.oid = (val != "(initial)").then(|| val.to_owned()),
        "branch.head" => b.head = (val != "(detached)").then(|| val.to_owned()),
        "branch.upstream" => b.upstream = Some(val.to_owned()),
        "branch.ab" => {
            for tok in val.split_whitespace() {
                if let Some(n) = tok.strip_prefix('+') {
                    b.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = tok.strip_prefix('-') {
                    b.behind = n.parse().unwrap_or(0);
                }
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Diffstat {
    files: HashMap<String, (u32, u32)>,
    added: u32,
    removed: u32,
}

/// Parse `git diff --numstat -z`.
///
/// Two shapes: `added \t removed \t path`, and for a rename an *empty* path
/// field followed by the old and new names as two further records. Binary files
/// report `-` for both counts, which parses to zero and so contributes nothing
/// to the totals while still listing the file.
fn parse_numstat(out: &str) -> Diffstat {
    let mut d = Diffstat::default();
    let mut recs = out.split('\0').filter(|r| !r.is_empty());

    while let Some(rec) = recs.next() {
        let mut f = rec.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            let _from = recs.next();
            match recs.next() {
                Some(to) => to,
                None => break,
            }
        } else {
            path
        };
        let (added, removed) = (
            added.parse::<u32>().unwrap_or(0),
            removed.parse::<u32>().unwrap_or(0),
        );
        d.added += added;
        d.removed += removed;
        d.files.insert(path.to_owned(), (added, removed));
    }
    d
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Commit {
    hash: String,
    /// Author timestamp, unix seconds. Stored rather than formatted so the age
    /// shown stays current between refreshes.
    at: i64,
    subject: String,
}

/// Parse `git log --format=%h%x1f%at%x1f%s`. A subject is single-line by
/// definition, so newlines separate commits and `\x1f` separates fields.
fn parse_log(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\x1f');
            let hash = f.next()?.trim();
            let at = f.next()?.trim().parse::<i64>().ok()?;
            if hash.is_empty() {
                return None;
            }
            Some(Commit {
                hash: hash.to_owned(),
                at,
                subject: f.next().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// formatting
// ---------------------------------------------------------------------------

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Three characters at most: the pane is narrow, and "2 hours ago" spends
/// eleven of them saying what "2h" says.
fn short_age(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => "now".to_string(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 2_592_000 => format!("{}d", s / 86_400),
        s if s < 31_536_000 => format!("{}mo", s / 2_592_000),
        s => format!("{}y", s / 31_536_000),
    }
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

/// Just the last two components of a path — used for the *source* of a rename,
/// where the destination is already spelled out in full next to it.
///
/// `/` only, for the reason [`Row::openable`] gives: these paths are git's
/// spelling rather than the platform's, and on Unix a `\` is a byte in a name
/// and not a separator to cut at.
fn short_path(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((head, tail)) => match head.rsplit_once('/') {
            Some((_, mid)) => &path[path.len() - tail.len() - mid.len() - 1..],
            None => path,
        },
        None => path,
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// porcelain v2 records are NUL-*terminated*, not NUL-separated, so the
    /// fixtures append one to every record rather than joining with them. That
    /// is what git emits, trailing NUL and all.
    fn z(records: &[&str]) -> String {
        records.iter().map(|r| format!("{r}\0")).collect()
    }

    const OID: &str = "8ab2f9c9e1b0d3f4a5c6d7e8f90123456789abcd";

    // --- headers ---------------------------------------------------------

    #[test]
    fn branch_header_with_upstream_and_divergence() {
        let st = parse_status(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
        ]));
        assert_eq!(st.branch.oid.as_deref(), Some(OID));
        assert_eq!(st.branch.head.as_deref(), Some("main"));
        assert_eq!(st.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.branch.ahead, st.branch.behind), (2, 1));
        assert!(st.entries.is_empty());
    }

    #[test]
    fn a_repo_with_no_commits_has_no_oid() {
        let st = parse_status(&z(&["# branch.oid (initial)", "# branch.head main"]));
        assert_eq!(st.branch.oid, None);
        assert_eq!(st.branch.head.as_deref(), Some("main"));
        assert_eq!(st.branch.upstream, None);
    }

    #[test]
    fn a_detached_head_has_no_branch_name() {
        let st = parse_status(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head (detached)",
        ]));
        assert_eq!(st.branch.head, None);
        assert_eq!(st.branch.oid.as_deref(), Some(OID));
        assert_eq!(short_oid(OID), "8ab2f9c");
    }

    #[test]
    fn unknown_and_malformed_headers_are_ignored() {
        let st = parse_status(&z(&[
            "# stash 3",
            "# branch.head main",
            "#",
            "# branch.ab nonsense",
            "# branch.ab +x -y",
        ]));
        assert_eq!(st.branch.head.as_deref(), Some("main"));
        assert_eq!((st.branch.ahead, st.branch.behind), (0, 0));
    }

    // --- ordinary entries ------------------------------------------------

    #[test]
    fn the_two_columns_of_xy_split_staged_from_unstaged() {
        let st = parse_status(&z(&[
            "1 M. N... 100644 100644 100644 aaaa bbbb staged.rs",
            "1 .M N... 100644 100644 100644 aaaa bbbb dirty.rs",
            "1 MM N... 100644 100644 100644 aaaa bbbb both.rs",
            "1 D. N... 100644 000000 000000 aaaa 0000 gone.rs",
        ]));
        assert_eq!(st.entries.len(), 4);

        let staged: Vec<_> = st.staged().map(|e| e.path.as_str()).collect();
        let unstaged: Vec<_> = st.unstaged().map(|e| e.path.as_str()).collect();
        assert_eq!(staged, ["staged.rs", "both.rs", "gone.rs"]);
        assert_eq!(unstaged, ["dirty.rs", "both.rs"]);

        // A file changed on both sides is one dirty file, listed twice.
        assert_eq!(st.dirty(), 4);
    }

    #[test]
    fn a_path_may_contain_spaces() {
        let st = parse_status(&z(&[
            "1 .M N... 100644 100644 100644 aaaa bbbb docs/my design notes.md",
            "? notes/another new file.md",
        ]));
        assert_eq!(st.entries[0].path, "docs/my design notes.md");
        assert_eq!(st.untracked().count(), 1);
        assert_eq!(st.untracked().next().unwrap().path, "notes/another new file.md");
    }

    // --- renames ---------------------------------------------------------

    #[test]
    fn a_rename_carries_its_source_and_stays_in_step() {
        // The source is a second record. If it is not consumed, the `? after.md`
        // below is read as the source and the untracked file disappears — which
        // is exactly the failure this pins.
        let st = parse_status(&z(&[
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 src/new name.rs",
            "src/old name.rs",
            "? after.md",
        ]));

        assert_eq!(st.entries.len(), 2);
        let renamed = &st.entries[0];
        assert_eq!(renamed.kind, Kind::Renamed);
        assert_eq!(renamed.path, "src/new name.rs");
        assert_eq!(renamed.from.as_deref(), Some("src/old name.rs"));
        assert_eq!(renamed.x, 'R');
        assert_eq!(st.staged().count(), 1);

        assert_eq!(st.entries[1].kind, Kind::Untracked);
        assert_eq!(st.entries[1].path, "after.md");
    }

    #[test]
    fn a_rename_edited_afterwards_appears_on_both_sides() {
        let st = parse_status(&z(&[
            "2 RM N... 100644 100644 100644 aaaa bbbb R087 new.rs",
            "old.rs",
        ]));
        assert_eq!(st.staged().count(), 1);
        assert_eq!(st.unstaged().count(), 1);
        // The source belongs to the staged rename, not to the worktree edit.
        assert_eq!(st.unstaged().next().unwrap().path, "new.rs");
    }

    #[test]
    fn a_truncated_rename_does_not_desynchronise_or_panic() {
        let st = parse_status(&z(&[
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 new.rs",
            // ...source record missing, stream ends.
        ]));
        assert_eq!(st.entries.len(), 1);
        assert_eq!(st.entries[0].from, None);
    }

    // --- unmerged --------------------------------------------------------

    #[test]
    fn unmerged_entries_are_conflicts_not_changes() {
        let st = parse_status(&z(&[
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc src/merge me.rs",
            "u AA N... 100644 100644 100644 100644 aaaa bbbb cccc both added.rs",
            "u DU N... 100644 100644 000000 100644 aaaa bbbb cccc deleted by us.rs",
        ]));
        assert_eq!(st.conflicts().count(), 3);
        // `U` in either column is a conflict, never a staged or unstaged change:
        // offering them alongside ordinary edits misreports the repo's state.
        assert_eq!(st.staged().count(), 0);
        assert_eq!(st.unstaged().count(), 0);

        let first = st.conflicts().next().unwrap();
        assert_eq!(first.path, "src/merge me.rs");
        assert_eq!((first.x, first.y), ('U', 'U'));
        assert_eq!(st.conflicts().nth(2).unwrap().path, "deleted by us.rs");
    }

    // --- tolerance -------------------------------------------------------

    #[test]
    fn short_and_empty_records_are_skipped() {
        let st = parse_status(&z(&[
            "1 M.",
            "1",
            "?",
            "",
            "x something entirely new",
            "1 .M N... 100644 100644 100644 aaaa bbbb good.rs",
        ]));
        assert_eq!(st.entries.len(), 1);
        assert_eq!(st.entries[0].path, "good.rs");
    }

    #[test]
    fn empty_output_is_a_clean_repo_not_an_error() {
        let st = parse_status("");
        assert_eq!(st.dirty(), 0);
        assert_eq!(st.branch, Branch::default());
    }

    #[test]
    fn ignored_entries_are_parsed_but_do_not_count_as_dirty() {
        let st = parse_status(&z(&["! target/debug/abeam.exe", "? real.rs"]));
        assert_eq!(st.entries.len(), 2);
        assert_eq!(st.dirty(), 1);
        assert_eq!(st.untracked().count(), 1);
    }

    /// One recorded capture with everything in it at once, because the parts
    /// interact: the rename's second record sits in the middle of the stream.
    #[test]
    fn a_whole_recorded_status() {
        let st = parse_status(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head workspace-restructure",
            "# branch.upstream origin/workspace-restructure",
            "# branch.ab +3 -0",
            "1 M. N... 100644 100644 100644 1111 2222 crates/abeam/src/app.rs",
            "1 .M N... 100644 100644 100644 3333 3333 crates/abeam/src/panes/git.rs",
            "1 A. N... 000000 100644 100644 0000 4444 docs/keymap.md",
            "2 R. N... 100644 100644 100644 5555 6666 R100 docs/conpty-findings.md",
            "spike-pty/README.md",
            "u UU N... 100644 100644 100644 100644 7777 8888 9999 Cargo.lock",
            "? crates/abeam/src/panes/viewer notes.md",
            "! target/",
        ]));

        assert_eq!(st.branch.head.as_deref(), Some("workspace-restructure"));
        assert_eq!((st.branch.ahead, st.branch.behind), (3, 0));
        assert_eq!(st.staged().count(), 3); // M., A., R.
        assert_eq!(st.unstaged().count(), 1); // .M
        assert_eq!(st.conflicts().count(), 1);
        assert_eq!(st.untracked().count(), 1);
        assert_eq!(st.dirty(), 6); // everything but the ignored entry
        assert_eq!(
            st.staged().nth(2).unwrap().from.as_deref(),
            Some("spike-pty/README.md")
        );
    }

    // --- numstat ---------------------------------------------------------

    #[test]
    fn numstat_sums_and_indexes_by_path() {
        let d = parse_numstat(&z(&["12\t3\tsrc/app.rs", "0\t40\tsrc/old thing.rs"]));
        assert_eq!((d.added, d.removed), (12, 43));
        assert_eq!(d.files.get("src/app.rs"), Some(&(12, 3)));
        assert_eq!(d.files.get("src/old thing.rs"), Some(&(0, 40)));
    }

    #[test]
    fn numstat_counts_a_binary_file_as_zero_lines() {
        let d = parse_numstat(&z(&["-\t-\tdocs/screenshot.png", "5\t5\ta.rs"]));
        assert_eq!((d.added, d.removed), (5, 5));
        assert_eq!(d.files.get("docs/screenshot.png"), Some(&(0, 0)));
    }

    #[test]
    fn numstat_keys_a_rename_on_its_new_path() {
        // The -z form of a rename: empty path field, then old and new.
        let d = parse_numstat(&z(&["3\t1\t", "old/name.rs", "new/name.rs", "2\t0\tother.rs"]));
        assert_eq!(d.files.get("new/name.rs"), Some(&(3, 1)));
        assert!(!d.files.contains_key("old/name.rs"));
        // The record after the rename must still be read correctly.
        assert_eq!(d.files.get("other.rs"), Some(&(2, 0)));
        assert_eq!((d.added, d.removed), (5, 1));
    }

    #[test]
    fn numstat_survives_a_truncated_rename() {
        let d = parse_numstat(&z(&["3\t1\t", "old/name.rs"]));
        assert!(d.files.is_empty());
    }

    // --- log -------------------------------------------------------------

    #[test]
    fn log_parses_hash_timestamp_and_subject() {
        let out = "a1b2c3d\u{1f}1754006400\u{1f}lift the pty host into its own crate\n\
                   e4f5a6b\u{1f}1753920000\u{1f}answer the DSR query: subject with \u{1f} in it\n";
        let commits = parse_log(out);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "a1b2c3d");
        assert_eq!(commits[0].at, 1_754_006_400);
        assert_eq!(commits[0].subject, "lift the pty host into its own crate");
        // splitn(3) keeps a separator inside the subject rather than eating it.
        assert!(commits[1].subject.contains('\u{1f}'));
    }

    #[test]
    fn log_skips_unparseable_lines_and_empty_output() {
        assert!(parse_log("").is_empty());
        assert!(parse_log("\n\n").is_empty());
        assert_eq!(parse_log("deadbee\u{1f}not-a-number\u{1f}x").len(), 0);
        assert_eq!(parse_log("deadbee\u{1f}1754006400").len(), 1);
    }

    // --- formatting ------------------------------------------------------

    #[test]
    fn ages_fit_in_three_columns() {
        assert_eq!(short_age(0), "now");
        assert_eq!(short_age(-5), "now"); // clock skew, not a panic
        assert_eq!(short_age(59), "now");
        assert_eq!(short_age(60), "1m");
        assert_eq!(short_age(3599), "59m");
        assert_eq!(short_age(3600), "1h");
        assert_eq!(short_age(86_399), "23h");
        assert_eq!(short_age(86_400), "1d");
        assert_eq!(short_age(2_591_999), "29d");
        assert_eq!(short_age(2_592_000), "1mo");
        assert_eq!(short_age(31_536_000), "1y");
    }

    #[test]
    fn a_rename_source_is_shortened_to_two_components() {
        assert_eq!(short_path("crates/abeam/src/panes/git.rs"), "panes/git.rs");
        assert_eq!(short_path("docs/keymap.md"), "docs/keymap.md");
        assert_eq!(short_path("README.md"), "README.md");
    }

    #[test]
    fn a_backslash_in_a_path_is_a_character_in_a_name_and_never_a_separator() {
        // Asserted on both platforms because the rule is git's, not the
        // operating system's: everything this module parses came out of
        // `git … -z`, and git spells a separator `/` wherever it runs. So `\`
        // can only ever be a byte somebody put in a file name — legal on Unix,
        // impossible on Windows — and cutting at one would shorten a path to
        // something that names nothing.
        assert_eq!(short_path(r"x/od\d/na\me.rs"), r"od\d/na\me.rs");

        // The same fact, on the other side of it, and driven through the
        // parser rather than by hand so that what is under test is a row git
        // could really produce. A trailing `/` is how `-unormal` collapses an
        // untracked tree into one row, and that row has nothing to open; a
        // trailing `\` is the last character of a file name, and that row has.
        let (listed, openable) = selectable(&z(&[r"? awkward\", "? notes/"]));
        assert_eq!(listed, [r"awkward\", "notes/"]);
        assert_eq!(openable, [r"awkward\"]);
    }

    #[test]
    fn a_git_error_is_reduced_to_one_readable_line() {
        let msg = first_line("fatal: not a git repository (or any of the parent directories): .git\n");
        assert_eq!(
            msg,
            "not a git repository (or any of the parent directories): .git"
        );
        assert_eq!(first_line(""), "git exited with an error");
    }

    // --- rows ------------------------------------------------------------

    #[test]
    fn rows_group_by_section_and_only_files_are_selectable() {
        let status = parse_status(&z(&[
            &format!("# branch.oid {OID}"),
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +1 -0",
            "1 M. N... 100644 100644 100644 aaaa bbbb staged.rs",
            "1 .M N... 100644 100644 100644 aaaa bbbb dirty.rs",
            "? new.md",
        ]));
        let snap = Snapshot {
            status,
            staged: parse_numstat(&z(&["10\t2\tstaged.rs"])),
            unstaged: parse_numstat(&z(&["1\t1\tdirty.rs"])),
            commits: parse_log("a1b2c3d\u{1f}1754006400\u{1f}a commit\n"),
            toplevel: None,
        };

        let (mut rows, mut picks) = (Vec::new(), Vec::new());
        build_rows(&snap, &mut rows, &mut picks);

        // Three files, three selectable rows, and every one of them is a File.
        assert_eq!(picks.len(), 3);
        let paths: Vec<_> = picks.iter().filter_map(|&i| rows[i].path()).collect();
        assert_eq!(paths, ["staged.rs", "dirty.rs", "new.md"]);

        // The line counts reach the row they belong to.
        let staged_row = &rows[picks[0]];
        match staged_row {
            Row::File { stat, .. } => assert_eq!(*stat, Some((10, 2))),
            _ => panic!("expected a file row"),
        }

        // Everything renders inside the width it is given, at any width.
        for width in [1u16, 8, 20, 80] {
            for (i, row) in rows.iter().enumerate() {
                let line = row.to_line(width, i == picks[0], 1_754_100_000);
                let w: usize = line.spans.iter().map(|s| s.content.width()).sum();
                assert!(w <= width as usize, "row {i} is {w} cells wide at {width}");
            }
        }
    }

    #[test]
    fn a_clean_tree_says_so_and_has_nothing_to_select() {
        let snap = Snapshot {
            status: parse_status(&z(&[&format!("# branch.oid {OID}"), "# branch.head main"])),
            ..Snapshot::default()
        };
        let (mut rows, mut picks) = (Vec::new(), Vec::new());
        build_rows(&snap, &mut rows, &mut picks);

        assert!(picks.is_empty());
        let text: String = rows
            .iter()
            .flat_map(|r| {
                r.to_line(60, false, 0)
                    .spans
                    .into_iter()
                    .map(|s| s.content.into_owned())
            })
            .collect();
        assert!(text.contains("working tree clean"), "{text}");
        assert!(text.contains("no upstream"), "{text}");
    }

    #[test]
    fn a_refresh_leaves_the_selection_on_the_row_the_reader_chose() {
        // `both.rs` is staged *and* edited, so it is listed twice — once under
        // Staged and once under Changed. A reader on the second copy must stay
        // on it when a background refresh rebuilds the list under them.
        let snap = || Snapshot {
            status: parse_status(&z(&[
                "1 M. N... 100644 100644 100644 aaaa bbbb first.rs",
                "1 MM N... 100644 100644 100644 aaaa bbbb both.rs",
            ])),
            ..Snapshot::default()
        };
        let (mut rows, mut picks) = (Vec::new(), Vec::new());
        build_rows(&snap(), &mut rows, &mut picks);

        let paths: Vec<_> = picks.iter().filter_map(|&i| rows[i].path()).collect();
        assert_eq!(paths, ["first.rs", "both.rs", "both.rs"]);

        // Sitting on the Changed copy (pick 2), a rebuild keeps it there...
        assert_eq!(refind(&rows, &picks, Some("both.rs"), 2), 2);
        // ...and sitting on the Staged copy (pick 1) keeps it there too.
        assert_eq!(refind(&rows, &picks, Some("both.rs"), 1), 1);

        // A file that has gone leaves the index where it was, clamped.
        assert_eq!(refind(&rows, &picks, Some("vanished.rs"), 2), 2);
        assert_eq!(refind(&rows, &[], Some("both.rs"), 2), 0);
    }

    /// Every row a selection can land on, and the subset of them Enter would
    /// actually open. The gap between the two lists is the whole point: a row
    /// you can select but not open is a deliberate state, not an oversight.
    fn selectable(status: &str) -> (Vec<String>, Vec<String>) {
        let snap = Snapshot {
            status: parse_status(status),
            ..Snapshot::default()
        };
        let (mut rows, mut picks) = (Vec::new(), Vec::new());
        build_rows(&snap, &mut rows, &mut picks);
        let listed = picks
            .iter()
            .filter_map(|&i| rows[i].path().map(str::to_owned))
            .collect();
        let openable = picks
            .iter()
            .filter_map(|&i| rows[i].openable().map(str::to_owned))
            .collect();
        (listed, openable)
    }

    #[test]
    fn an_untracked_directory_is_listed_but_cannot_be_opened() {
        // `-unormal` collapses an untracked tree to one `dir/` entry. Sending
        // that to the viewer switches the right pane away from git to say "not
        // a regular file", which is a worse answer than Enter doing nothing.
        let (listed, openable) = selectable(&z(&["? notes/", "? real.md"]));
        assert_eq!(
            listed,
            ["notes/", "real.md"],
            "the directory is still shown"
        );
        assert_eq!(openable, ["real.md"]);
    }

    #[test]
    fn a_deleted_file_is_listed_but_cannot_be_opened() {
        // The same failure as the directory above, and a more likely one: every
        // `D` row was a one-way trip out of the git view to a viewer saying "no
        // such file" about a file the user deleted on purpose.
        let (listed, openable) = selectable(&z(&[
            "1 .D N... 100644 100644 000000 aaaa bbbb worktree-gone.rs",
            "1 D. N... 100644 000000 000000 aaaa 0000 staged-delete.rs",
            // Staged as a modification and deleted from the worktree after, so
            // it is listed under Staged with an `M` mark — and is just as
            // absent as the `D` rows above it.
            "1 MD N... 100644 100644 000000 aaaa bbbb edited-then-gone.rs",
            "1 .M N... 100644 100644 100644 aaaa bbbb alive.rs",
        ]));
        assert_eq!(
            listed,
            [
                "staged-delete.rs",    // Staged
                "edited-then-gone.rs", //   "
                "worktree-gone.rs",    // Changed
                "edited-then-gone.rs", //   "
                "alive.rs",            //   "
            ],
            "every one of them is still shown"
        );
        assert_eq!(openable, ["alive.rs"]);
    }

    #[test]
    fn a_delete_modify_conflict_is_still_there_to_read() {
        // `DU` and `UD` carry a `D` without being deletions: git leaves the
        // surviving version in the worktree to be resolved, and that file is
        // exactly the one someone wants to look at.
        let (listed, openable) = selectable(&z(&[
            "u DU N... 100644 100644 000000 100644 aaaa bbbb cccc theirs.rs",
            "u UD N... 100644 100644 100644 000000 aaaa bbbb cccc ours.rs",
        ]));
        assert_eq!(listed, ["theirs.rs", "ours.rs"]);
        assert_eq!(openable, ["theirs.rs", "ours.rs"]);
    }

    /// The three failure reports the README promises. Rendered here rather than
    /// through a pane so no test in this file has to shell out to git.
    #[test]
    fn every_failure_report_renders_at_any_width() {
        let root = Path::new(r"C:\some\where\deep\enough\to\need\truncating");
        for report in [
            Report::Pending,
            Report::NoGit,
            Report::NotRepo,
            Report::Failed("could not read index: bad file descriptor".into()),
        ] {
            let (mut rows, mut picks) = (Vec::new(), Vec::new());
            report_rows(&report, root, &mut rows, &mut picks);
            assert!(!rows.is_empty(), "{report:?} says nothing at all");
            // Nothing to select: a failure report has no files in it, so Tab
            // and Enter must find nothing rather than an index into notes.
            assert!(picks.is_empty(), "{report:?} offered a selection");

            for width in [1u16, 2, 5, 22, 46, 120] {
                for row in &rows {
                    let line = row.to_line(width, false, 0);
                    let w: usize = line.spans.iter().map(|s| s.content.width()).sum();
                    assert!(w <= width as usize, "{report:?} overflows at {width}");
                }
            }
        }
    }

    #[test]
    fn the_failure_reports_say_which_failure_it_was() {
        let text = |report: &Report| -> String {
            let (mut rows, mut picks) = (Vec::new(), Vec::new());
            report_rows(report, Path::new("/repo"), &mut rows, &mut picks);
            rows.iter()
                .flat_map(|r| {
                    r.to_line(60, false, 0)
                        .spans
                        .into_iter()
                        .map(|s| s.content.into_owned())
                })
                .collect()
        };
        assert!(text(&Report::NoGit).contains("git is not on PATH"));
        assert!(text(&Report::NotRepo).contains("not a git repository"));
        assert!(text(&Report::Failed("boom".into())).contains("boom"));
        assert!(text(&Report::Pending).contains("reading the repository"));
    }

    // --- being pointed at another worktree ---------------------------------
    //
    // All of it through [`GitPane::detached`], so that a pane appears in this
    // file without a `git` process appearing with it.

    /// Two worktrees, spelled for the platform running the test.
    ///
    /// Places rather than strings, for `crate::paths`'s reason: what makes two
    /// spellings one directory is not the same rule on the two platforms, and a
    /// Windows path asserted on Linux would take the case-sensitive comparison
    /// without ever exercising it.
    #[cfg(windows)]
    const ONE: &str = r"C:\Users\philm\PycharmProjects\forge";
    #[cfg(windows)]
    const TWO: &str = r"C:\Users\philm\PycharmProjects\forge\.claude\worktrees\other";
    #[cfg(unix)]
    const ONE: &str = "/home/philm/PycharmProjects/forge";
    #[cfg(unix)]
    const TWO: &str = "/home/philm/PycharmProjects/forge/.claude/worktrees/other";

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn detached(root: &str) -> (GitPane, Receiver<Ask>, Sender<Answer>) {
        GitPane::detached(PathBuf::from(root))
    }

    /// A report with a branch name and a change count in it — the two things
    /// the title draws, and so the two things a stale report puts on screen
    /// under the wrong workspace's name.
    fn a_report(branch: &str) -> Report {
        Report::Ok(Box::new(Snapshot {
            status: parse_status(&z(&[
                &format!("# branch.oid {OID}"),
                &format!("# branch.head {branch}"),
                "1 .M N... 100644 100644 100644 aaaa bbbb dirty.rs",
            ])),
            ..Snapshot::default()
        }))
    }

    fn a_row(label: &str, root: &str, here: bool) -> workspace::Row {
        workspace::Row {
            label: label.to_string(),
            root: PathBuf::from(root),
            here,
            agent_here: false,
            occupant: None,
            watched: true,
        }
    }

    #[test]
    fn question_mark_offers_the_selected_file_and_opens_the_pane_either_way() {
        // Pane-local for `w`'s reason — the *intercept* paragraph at the top of
        // `crate::keys` — and drained like the open request beside it.
        //
        // The half worth writing a test for is the second one. `?` is the only
        // key that reaches the ask view, so a `?` pressed over a repository
        // with nothing changed in it has to open the pane with no context
        // rather than doing nothing at all, and that is the whole reason
        // `AskRequest` is a newtype over an `Option` and not an `Option`.
        let (mut pane, _asks, answers) = detached(ONE);
        assert_eq!(pane.take_ask_request().map(|r| r.0), None, "nothing asked");

        // Nothing has been reported yet, so there is nothing selectable...
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('?'))).unwrap(),
            Handled::Yes
        );
        let AskRequest(nothing) = pane.take_ask_request().expect("the view still opens");
        assert_eq!(nothing, None);
        assert!(
            pane.take_ask_request().is_none(),
            "a request left sitting fires late, at whatever unrelated moment \
             next reads it"
        );

        // ...and once there is, the path is absolute and resolved against the
        // toplevel git reported rather than the directory abeam was started in.
        let first = _asks.recv().expect("the pane asks on construction");
        answers
            .send(Answer {
                generation: first.generation,
                report: a_report("main"),
            })
            .expect("the pane is listening");
        assert!(pane.tick());
        pane.handle_key(key(KeyCode::Tab)).unwrap();

        pane.handle_key(key(KeyCode::Char('?'))).unwrap();
        let AskRequest(path) = pane.take_ask_request().expect("a request");
        let path = path.expect("a selected file is something to ask about");
        assert!(path.is_absolute());
        assert!(path.ends_with("dirty.rs"), "got: {}", path.display());
    }

    #[test]
    fn set_root_asks_the_new_root_and_forgets_what_the_old_one_produced() {
        let (mut pane, asks, answers) = detached(ONE);
        let first = asks.recv().expect("the pane asks on construction");
        assert!(crate::paths::same_dir(&first.root, Path::new(ONE)));

        answers
            .send(Answer {
                generation: first.generation,
                report: a_report("main"),
            })
            .expect("the pane is listening");
        assert!(pane.tick());

        // A reader who has been using the pane: a selection, a scroll position,
        // and an `Enter` on a file that has not been drained yet.
        pane.handle_key(key(KeyCode::Tab)).unwrap();
        pane.scroll.measure(100, 10);
        pane.scroll.by(5);
        pane.stub_open_request("dirty.rs");
        assert!(pane.sel_path.is_some());

        pane.set_root(PathBuf::from(TWO));

        assert_eq!(
            pane.report,
            Report::Pending,
            "the other repository's branch and change count stayed on screen \
             under the new workspace's name"
        );
        assert_eq!(
            pane.take_open_request(),
            None,
            "an Enter aimed at the old toplevel survived the switch, and would \
             have opened whatever sits at that path in the new one"
        );
        assert_eq!(pane.sel, 0);
        assert_eq!(pane.sel_path, None);
        assert_eq!(pane.scroll.offset, 0);
        assert!(!pane.slow);

        let second = asks.recv().expect("a refresh of the new root");
        assert!(crate::paths::same_dir(&second.root, Path::new(TWO)));
        assert_ne!(second.generation, first.generation);
        assert!(pane.inflight.is_some(), "nothing is out for the new root");
    }

    #[test]
    fn an_answer_from_before_the_switch_is_dropped_and_settles_nothing() {
        // The failure this guards is not a wrong screen for one frame. A stale
        // answer that cleared `inflight` and `settled` would mark the *new*
        // request as finished, so the pane would sit on "reading the
        // repository…" until the two-second poll happened to rescue it.
        let (mut pane, asks, answers) = detached(ONE);
        let first = asks.recv().expect("the pane asks on construction");
        pane.set_root(PathBuf::from(TWO));
        let second = asks.recv().expect("a refresh of the new root");

        let settled = pane.settled;
        // As if the new request had already outlived `SLOW_AFTER`, so the title
        // is saying so and a stale answer clearing it would be visible.
        pane.slow = true;

        answers
            .send(Answer {
                generation: first.generation,
                report: a_report("main"),
            })
            .expect("the pane is listening");
        assert!(
            !pane.tick(),
            "a report about the workspace we left cost the agent a frame"
        );
        assert_eq!(pane.report, Report::Pending);
        assert!(
            pane.inflight.is_some(),
            "the stale answer marked the new request as finished"
        );
        assert_eq!(pane.settled, settled, "and restarted the refresh timer");
        assert!(pane.slow, "and took the title's own note down with it");

        // ...and the answer that does belong to the new root is taken.
        answers
            .send(Answer {
                generation: second.generation,
                report: a_report("other"),
            })
            .expect("the pane is listening");
        assert!(pane.tick());
        assert!(pane.title().contains("other"), "{}", pane.title());
        assert!(pane.inflight.is_none());
    }

    #[test]
    fn switching_away_and_back_does_not_let_the_first_answer_pass_as_current() {
        // Why the stamp is a counter and not the root. A → B → A inside one
        // refresh cycle leaves a report about A that was asked for *before* the
        // first switch — it describes A as it was two workspaces ago — and a
        // root comparison would accept it as current.
        let (mut pane, asks, answers) = detached(ONE);
        let first = asks.recv().expect("the pane asks on construction");

        pane.set_root(PathBuf::from(TWO));
        asks.recv().expect("a refresh of the second root");
        pane.set_root(PathBuf::from(ONE));
        let third = asks.recv().expect("a refresh of the first root again");

        assert!(
            crate::paths::same_dir(&third.root, Path::new(ONE)),
            "back where we started, which is the whole trap"
        );
        assert_ne!(
            third.generation, first.generation,
            "a root comparison would call these two the same request"
        );

        answers
            .send(Answer {
                generation: first.generation,
                report: a_report("main"),
            })
            .expect("the pane is listening");
        assert!(!pane.tick());
        assert_eq!(pane.report, Report::Pending);
    }

    #[test]
    fn a_switch_with_no_worker_behind_it_says_so_rather_than_reading_for_ever() {
        // The wedge. `set_root` writes `Pending` unconditionally and then calls
        // `request`, which used to return early when the worker was gone —
        // without setting `inflight`, so nothing was out, and `tick`'s
        // `Disconnected` arm is guarded on the worker still being believed
        // alive, so the "worker stopped" report was overwritten and never
        // restored.
        //
        // What was left on screen is "reading the repository…", for the rest of
        // the session. `inflight` is `None`, so `slow` never fires and the
        // title never admits to anything either. A pane that is broken and says
        // it is busy is the one failure shape this file is otherwise careful
        // about.
        let (mut pane, _asks, answers) = detached(ONE);
        drop(answers);

        assert!(pane.tick(), "the worker going costs a frame");
        assert!(!pane.worker_alive);
        assert_eq!(
            pane.report,
            Report::Failed("the git worker stopped".into()),
            "the pane never noticed"
        );

        pane.set_root(PathBuf::from(TWO));
        assert_eq!(
            pane.report,
            Report::Failed("the git worker stopped".into()),
            "the switch left the pane reading a repository nobody is reading"
        );
        assert!(
            pane.inflight.is_none(),
            "nothing is out, and nothing can be"
        );

        // ...and it stays said. `tick` goes on asking on the refresh timer,
        // which is what keeps this from being a one-frame message, and the
        // report has to survive every one of those without costing a frame.
        for _ in 0..3 {
            assert!(!pane.tick(), "a dead worker cost the agent a frame");
            assert_eq!(pane.report, Report::Failed("the git worker stopped".into()));
        }
        assert!(!pane.slow, "nothing is out to be slow");

        // And what is actually drawn, which is the whole complaint: the rows
        // are rebuilt from the report, so a report nobody rebuilt from would
        // leave the old sentence on screen however right the field was.
        let shown: String = pane
            .rows
            .iter()
            .flat_map(|row| {
                row.to_line(60, false, 0)
                    .spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
            })
            .collect();
        assert!(shown.contains("worker stopped"), "{shown}");
        assert!(!shown.contains("reading the repository"), "{shown}");
    }

    #[test]
    fn ctrl_and_a_letter_is_the_agents_here_as_it_is_in_the_pane_next_door() {
        // `crate::scroll::key` hands `Ctrl` plus a letter *back* rather than
        // declining it, so a pane's own table is where the decision gets made —
        // and this pane's plain-letter arms took it. `Ctrl+W` opened the
        // worktree list and started a `claude agents --json` behind it, and
        // `Ctrl+R` asked git for a fresh diff, both for chords the reader aimed
        // at the agent. The viewer's document view has said this since it was
        // written; this pane claimed parity with it and did not have it.
        let (mut pane, _asks, _answers) = detached(ONE);
        for letter in ['w', 'r', 'a', 'e'] {
            assert_eq!(
                pane.handle_key(KeyEvent::new(
                    KeyCode::Char(letter),
                    KeyModifiers::CONTROL
                ))
                .unwrap(),
                Handled::No,
                "Ctrl+{letter} was eaten by the git pane"
            );
        }
        assert!(
            !pane.wants_worktrees(),
            "a chord aimed at the agent opened the worktree list"
        );

        // The bare letters still work, which is what makes the arm a rule about
        // the modifier rather than about the keys.
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('w'))).unwrap(),
            Handled::Yes
        );
        assert!(pane.wants_worktrees());
    }

    // --- the worktree list -------------------------------------------------

    #[test]
    fn w_opens_the_worktree_list_and_esc_gives_back_the_status_list() {
        let (mut pane, _asks, _answers) = detached(ONE);
        assert_eq!(pane.exit_hint(), " · esc→agent");
        assert!(!pane.wants_worktrees(), "nothing has asked for anything");

        assert_eq!(
            pane.handle_key(key(KeyCode::Char('w'))).unwrap(),
            Handled::Yes
        );
        assert!(
            pane.wants_worktrees(),
            "the roster stayed gated, so the occupancy column would never fill in"
        );
        assert!(pane.title().contains("worktrees"), "{}", pane.title());
        assert_eq!(
            pane.exit_hint(),
            " · esc→git",
            "Esc here is one press short of the agent, and the border is the \
             only place that is written down"
        );
        assert!(!pane.takes_input(), "nothing here is typed into");

        // Claimed rather than falling through to the shell as "back to the
        // agent", which is what every other Esc in this pane does.
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::Yes);
        assert_eq!(pane.exit_hint(), " · esc→agent");

        // ...and `w` is the way out as well as the way in.
        pane.handle_key(key(KeyCode::Char('w'))).unwrap();
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('w'))).unwrap(),
            Handled::Yes
        );
        assert_eq!(pane.exit_hint(), " · esc→agent");
        assert!(!pane.takes_input());
        assert!(pane.wants_worktrees(), "and the flag is sticky");
    }

    #[test]
    fn enter_on_a_row_asks_for_a_switch_once_and_comes_back_to_the_status_list() {
        let (mut pane, _asks, _answers) = detached(ONE);
        pane.set_worktree_rows(vec![a_row("main", ONE, true), a_row("other", TWO, false)]);
        pane.handle_key(key(KeyCode::Char('w'))).unwrap();

        assert_eq!(pane.handle_key(key(KeyCode::Tab)).unwrap(), Handled::Yes);
        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);

        let asked = pane
            .take_workspace_request()
            .expect("a switch was asked for");
        assert!(crate::paths::same_dir(&asked, Path::new(TWO)));
        // Drained, not left to fire late at whatever unrelated moment next
        // reads it — the same contract as `take_open_request`.
        assert_eq!(pane.take_workspace_request(), None);
        assert_eq!(
            pane.exit_hint(),
            " · esc→agent",
            "what a switch is for is the other worktree's git"
        );

        // BackTab walks the other way, and both wrap.
        pane.handle_key(key(KeyCode::Char('w'))).unwrap();
        pane.handle_key(key(KeyCode::BackTab)).unwrap();
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(crate::paths::same_dir(
            &pane.take_workspace_request().expect("a switch"),
            Path::new(ONE)
        ));
    }

    #[test]
    fn a_worktree_list_nobody_has_opened_never_costs_a_frame() {
        // Discovery runs every ten seconds for the whole session, and a frame
        // re-renders the agent's entire screen. `QueuePane::set_roster`'s
        // convention: only what is on screen can owe one.
        let (mut pane, _asks, _answers) = detached(ONE);
        let rows = vec![a_row("main", ONE, true)];
        assert!(!pane.set_worktree_rows(rows.clone()));
        assert!(
            !pane.set_worktree_rows(rows.clone()),
            "and unchanged is not news"
        );

        pane.handle_key(key(KeyCode::Char('w'))).unwrap();
        assert!(
            !pane.set_worktree_rows(rows),
            "an unchanged list is not news with the list open either"
        );
        assert!(pane.set_worktree_rows(vec![a_row("main", ONE, true), a_row("other", TWO, false)]));
    }

    #[test]
    fn every_row_of_the_worktree_list_fits_the_pane_it_is_drawn_in() {
        let rows = vec![
            workspace::Row {
                agent_here: true,
                ..a_row("hand-the-command-line-to-the-agent", ONE, true)
            },
            workspace::Row {
                occupant: Some("a1b2c3d4 · working".into()),
                watched: false,
                ..a_row("other", TWO, false)
            },
        ];

        for width in [1u16, 2, 5, 22, 46, 120] {
            for line in worktree_lines(&rows, width, 0) {
                let w: usize = line.spans.iter().map(|s| s.content.width()).sum();
                assert!(w <= width as usize, "a row is {w} cells wide at {width}");
            }
        }

        let text = |rows: &[workspace::Row]| -> String {
            worktree_lines(rows, 120, 0)
                .into_iter()
                .flat_map(|line| line.spans.into_iter().map(|s| s.content.into_owned()))
                .collect()
        };
        let drawn = text(&rows);
        assert!(drawn.contains("agent"), "{drawn}");
        assert!(drawn.contains("a1b2c3d4 · working"), "{drawn}");
        assert!(
            drawn.contains("unwatched"),
            "a worktree the one watcher cannot reach looks broken rather than \
             slow unless the row says which: {drawn}"
        );

        // An empty list is two states — discovery has not answered, or it
        // answered with nothing — and an empty box says neither.
        assert!(text(&[]).contains("no worktrees"), "{}", text(&[]));
    }
}
