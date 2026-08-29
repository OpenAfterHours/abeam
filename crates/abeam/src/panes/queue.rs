//! The queue: work lined up for the agent, in the two shapes that means.
//!
//! What it is for is the gap between having a thought and being able to act on
//! it. The agent is mid-task, you know the next three things you want, and the
//! choices today are to interrupt it, to hold them in your head, or to write
//! them somewhere abeam cannot see. This pane is the somewhere.
//!
//! ## The two modes, and why they are one pane
//!
//! - [`Mode::Send`] — typed into the session in the left pane the moment it
//!   goes idle. It *continues the conversation*: everything above it is still
//!   context. There is one left pane, so these are strictly sequential.
//! - [`Mode::Dispatch`] — started as its own background agent
//!   (`crate::dispatch`), with none of that context, running beside you. These
//!   are parallel, and there can be many.
//!
//! They are the same list because they are the same thought — "do this next" —
//! and differ only in whether the work needs the conversation you have already
//! had. Two panes would make you decide where a thought goes before you have
//! finished having it.
//!
//! ## Sending is never a guess
//!
//! A prompt typed into an agent at the wrong moment is not a cosmetic bug: a
//! permission dialog answers with the first character it is given, and a
//! half-written message in the composer is spliced, silently. So a send needs
//! **four** things to be true at once. They are stated here, once, because the
//! count is load-bearing — every argument in this file about what may be
//! skipped is an argument about which of these four it is:
//!
//! 1. **The queue is armed, or this item was asked for by hand.** `a` is the
//!    switch on the sender that runs unattended; `Enter` on a selected item is
//!    the attended path and does not need it. See [`QueuePane::now`].
//! 2. **`crate::agentstate` reports [`Readiness::Idle`]** — read from the
//!    agent's own record, never inferred from output going quiet.
//! 3. **Nothing is sitting unsubmitted in the agent's composer.** abeam
//!    forwards every keystroke, so typing sets that flag without asking
//!    anyone; only *watching the agent go busy* clears it. A submit cannot be
//!    inferred from the keystroke that looks like one — a bare `Enter` that
//!    Claude's inline autocomplete consumes accepts a completion and leaves the
//!    text sitting in the composer, with the record still reading `idle`. The
//!    consequence is deliberate and worth knowing: a draft typed and then
//!    abandoned holds the queue until something is actually submitted at the
//!    agent, which is the safe direction to be wrong in.
//! 4. **The item's own announcement has elapsed.** A by-hand ask is due at
//!    once; the automatic sender waits out [`ARM_DELAY`] first.
//!
//! That announcement is the fourth condition and the visible one: a pause drawn
//! in the left pane's title, during which a keystroke at the agent defers the
//! send — condition 3 goes false, the announcement is withdrawn, and the next
//! one is made from the beginning rather than resumed. It is the same
//! interaction as the quit confirmation in `crate::app`, and for the same
//! reason: a thing that types on your behalf must be interruptible by the
//! reflex of starting to type. A keystroke *here* is not one of those — this
//! pane is not the agent's composer — which is why the announcement belongs to
//! the item rather than to the pane.
//!
//! Dispatching needs none of this. Nothing is being typed at, so an item in
//! [`Mode::Dispatch`] goes the moment it is asked to — and pays for it in a
//! different currency, which the pane discloses rather than assumes: a
//! dispatched agent edits files without asking, while you are looking at
//! something else.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::agentstate::{Readiness, Session};
use crate::dispatch::{Dispatcher, Started, Unavailable};
use crate::pane::{Handled, Pane};
use crate::scroll::{self, Scroll};
use crate::text::{block, clip, clip_line, dim, elide_left, err};

/// How long an armed, ready send is announced before it happens.
///
/// Long enough to see and stop, short enough not to feel like a delay. A
/// keystroke at the agent during it defers the send to the next idle.
///
/// The default for [`QueuePane::arm_delay`] rather than a value read at the
/// point of use, so a test can drive a countdown it did not have to sit through
/// and still assert against the number on screen.
const ARM_DELAY: Duration = Duration::from_secs(3);

/// Narrower than this and the live state of a dispatched agent is worth less
/// than the columns it takes from the text it is annotating. The same threshold
/// `crate::scroll` uses to decide it can afford a scrollbar.
const ASIDE_MIN_WIDTH: usize = 24;

/// What the composer writes in front of the draft.
const PROMPT: &str = "› ";

/// What a dispatched agent does that a queued prompt does not, said in the pane
/// rather than left in this file's documentation.
///
/// The asymmetry is why it is on screen at all. The send path risks a spliced
/// prompt and gets a three-second countdown, announced in the left title and
/// cancellable by any keystroke. The dispatch path risks unsupervised writes to
/// the user's repository — and `m` moves an item between those two postures in
/// one keystroke, announcing the change with a changed sigil.
///
/// Short because it shares a row with the queue's live state, and that state is
/// what a reader watches: at four words this fits beside all three readiness
/// answers in a sixty-column pane, where "edits files without asking" pushed the
/// line over and clipped itself in half.
const DISPATCH_WARNING: &str = "⇉ edits files unattended";

/// Where a queued item is meant to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Into the live session in the left pane, when it is idle.
    Send,
    /// Into a background agent of its own.
    Dispatch,
}

/// What has become of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemState {
    /// Waiting its turn.
    Pending,
    /// Typed into the left pane. abeam cannot follow it further than that —
    /// once it is in the agent's composer it is the agent's.
    Sent,
    /// Started as a background agent. `id` joins it to a row in
    /// [`crate::agentstate::roster`], which is where its live state comes from.
    Dispatched { id: Option<String> },
    /// It could not be started, and this is what went wrong.
    Failed(String),
}

/// A send that is coming, and why.
///
/// It rides on the [`Item`] rather than on the pane, and that is a fix rather
/// than a preference: as a bare `Instant` on the pane it outlived its subject.
/// `Enter` on item B granted a due and promoted B; deleting B before the next
/// pass left the elapsed due sitting there for item **A**, which was then typed
/// at the agent with no countdown and nobody's consent. A due that is a field of
/// the item it was granted for cannot be inherited, because deleting the item
/// deletes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    /// The countdown. Announced because the queue is armed and everything else
    /// lined up, and withdrawn the moment any of that stops being true.
    Announced(Instant),
    /// Asked for by hand, with `Enter`, on the item that was selected.
    Asked,
}

/// One queued piece of work.
#[derive(Clone, Debug)]
pub struct Item {
    pub text: String,
    pub mode: Mode,
    pub state: ItemState,
    /// The announcement, which belongs to this item and to no other. Created
    /// and withdrawn in exactly one place, [`QueuePane::retime`].
    due: Option<Due>,
    /// This item has been handed to a dispatcher at least once. It never goes
    /// back to false: it is the record of an agent that may be running, and it
    /// is what stops `Enter` starting a second one. See
    /// [`QueuePane::note_dispatched`].
    started: bool,
}

/// A key that throws work away, pressed once and waiting to be meant.
///
/// Two of this pane's keys cannot be taken back by anyone who presses them: `d`
/// throws away something somebody wrote, and `r` throws away the record of what
/// abeam did with the rest. Both are single bare letters, because every key in
/// the list is. That was safe while the only way to reach them was to aim at
/// this pane, and it stopped being the only way when a view key stopped moving
/// focus: `F8` to glance at the queue from a shell now leaves the keys here,
/// so the rest of a half-typed command is read as commands. `cargo doc
/// --release` carries a `d` and two `r`s.
///
/// `Enter` cannot be taken back either, and is deliberately not guarded — it is
/// what the pane is *for*, it acts only on the row the user chose, and a guard
/// on it would be two presses on the pane's ordinary verb. That is a judgement
/// about cost, not a claim that it is safe: `Enter` ends every mistyped command
/// there is, and it grants a [`Due::Asked`], which is the one send that skips
/// the announced countdown. If that trade is ever revisited, the honest fix is
/// that delay rather than a second press.
///
/// **The guard is `Alt+Q`'s in shape and narrower in reach**, and the
/// difference is worth stating because the resemblance invites the assumption.
/// `crate::app` clears a pending quit before it matches anything at all, so
/// every key, every paste and every mouse press in the window is the answer no.
/// This one is pane-local: it sees the keys, pastes and clicks this pane is
/// offered, and nothing else — which is why [`QueuePane::cancel_confirm`]
/// exists for the shell to call when the pane leaves the screen.
///
/// It is a speed bump rather than a lock. Two `d`s in a row still delete, so
/// `add` and `ladder` get through, and that is the limit of a confirmation
/// sharing its key with the thing it confirms. What it buys is that the single
/// stray press — overwhelmingly the common one — changes the screen instead of
/// the list.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Confirm {
    /// The text of the item the question was asked about.
    ///
    /// Carried rather than trusted to `selected`, because `selected` moves
    /// under things that are not keys — a click, and a paste that appends an
    /// item and selects it. Without this, `d`, click on another row, `d`
    /// deleted a row nothing had asked about: one warning, shown about
    /// something else. Two items with the same text are indistinguishable
    /// here, and that is the one case where being wrong costs nothing.
    Delete(String),
    Clear,
}

/// The pane.
pub struct QueuePane {
    root: PathBuf,
    items: Vec<Item>,
    selected: usize,
    /// `Some` while composing a new item; the string is what has been typed.
    composing: Option<String>,
    /// A destructive key waiting to be pressed a second time. See [`Confirm`].
    confirm: Option<Confirm>,
    /// Whether [`Mode::Send`] items may go on their own. Off is not "paused
    /// work" — dispatching is unaffected, and so is `Enter` — it is "do not
    /// type at the agent unless I am asking for it".
    armed: bool,
    /// Last told by the shell, never worked out here: the pane does not know
    /// what a pty is.
    readiness: Readiness,
    /// Set by the shell when a key reaches the agent, and cleared when the
    /// agent is next seen to go *busy* — see condition 3 in the module docs. A
    /// send while this is true would splice into a half-written message.
    draft_open: bool,
    /// How long a countdown runs. [`ARM_DELAY`] in every real session; a field
    /// so a test can prove the number on screen came off the clock rather than
    /// out of the constant it is being compared with.
    arm_delay: Duration,
    /// The whole second [`QueuePane::title_note`] last put on screen.
    ///
    /// The countdown is the one thing in this pane that changes without anyone
    /// touching it, and [`Pane::tick`] runs at the speed of the loop rather
    /// than of a clock. Without somewhere to remember what was last drawn,
    /// "has the number changed?" is unanswerable and the honest fallback is to
    /// claim a frame every pass — which re-renders the agent's entire screen, a
    /// hundred and twenty-five times a second, to redraw a `3`.
    shown: Option<u64>,
    /// Live rows for the dispatched items, refreshed on a worker thread.
    roster: Vec<Session>,
    dispatcher: Result<Dispatcher, Unavailable>,
    /// The view onto the list. The selection and the offset are deliberately
    /// two things, as they are in `panes::git`: `Tab` chooses, `j` and the
    /// wheel look, and a glance from the other side of the window may never
    /// re-aim what `Enter` would act on.
    scroll: Scroll,
    drawn: Rect,
}

impl QueuePane {
    /// `agent` is the hosted agent's name, which decides whether
    /// [`Mode::Dispatch`] is available at all — see `crate::dispatch`.
    pub fn new(root: PathBuf, agent: &str) -> Self {
        let dispatcher = Dispatcher::new(root.clone(), agent);
        Self::with_dispatcher(root, dispatcher)
    }

    /// The same pane, with the dispatch decision handed in rather than looked
    /// up.
    ///
    /// [`Dispatcher::new`] resolves a real program on the real machine, and the
    /// answer changes this pane by a whole row: an unavailable dispatcher draws
    /// a notice above the list, which every row below it — and therefore every
    /// mouse click — is offset by. A test of the shell that built the pane the
    /// ordinary way would pass or fail depending on whether the machine running
    /// it has Claude installed, and that is not a property a test may have.
    pub fn with_dispatcher(root: PathBuf, dispatcher: Result<Dispatcher, Unavailable>) -> Self {
        Self {
            root,
            items: Vec::new(),
            selected: 0,
            composing: None,
            confirm: None,
            armed: false,
            readiness: Readiness::Unknown,
            draft_open: false,
            arm_delay: ARM_DELAY,
            shown: None,
            roster: Vec::new(),
            dispatcher,
            scroll: Scroll::default(),
            drawn: Rect::ZERO,
        }
    }

    // --- what the shell tells it -----------------------------------------

    /// Returns whether anything the next frame would show has changed, so the
    /// shell can decline to spend a frame. A frame re-renders the agent's
    /// whole screen; see `crate::app::App::tick_panes`.
    pub fn set_readiness(&mut self, readiness: Readiness) -> bool {
        if self.readiness == readiness {
            return false;
        }
        self.readiness = readiness;
        // Whatever it changed to, the announcement is now either owed or void.
        self.retime();
        true
    }

    /// Whether the user has an unsubmitted message in the agent's composer:
    /// condition 3 in the module docs, including why only the shell can know it.
    pub fn set_draft_open(&mut self, open: bool) -> bool {
        if self.draft_open == open {
            return false;
        }
        self.draft_open = open;
        self.retime();
        true
    }

    /// Live state for the dispatched items.
    ///
    /// Compared through what this pane would *draw* from it rather than field
    /// by field: the roster carries every session Claude knows about, most of
    /// which no item here joins to, and a stranger's session going from busy
    /// to idle must not cost a frame in a window showing none of it.
    pub fn set_roster(&mut self, roster: Vec<Session>) -> bool {
        let changed = self.items.iter().any(|item| match &item.state {
            ItemState::Dispatched { id: Some(id) } => {
                live_of(&self.roster, id) != live_of(&roster, id)
            }
            _ => false,
        });
        self.roster = roster;
        changed
    }

    // --- what the shell asks of it ---------------------------------------

    /// The next item to type into the left pane, if all four conditions in the
    /// module docs are met.
    ///
    /// Draining, not peeking: a request left sitting fires late, at whatever
    /// unrelated moment next reads it. The same contract as
    /// `GitPane::take_open_request`.
    ///
    /// This is the only path by which anything leaves the queue for the agent,
    /// and it has no exception in it. `Enter` does not bypass it — it grants the
    /// item a [`Due::Asked`], which is this same gate reached by a different
    /// route, so there is one place where a send is decided and one place to
    /// test it. The conditions are re-asked here through [`QueuePane::retime`]
    /// rather than trusted from the moment the announcement was made; that is
    /// the whole point of announcing it, and anything that has happened since —
    /// a keystroke at the agent, a turn starting, the item being deleted —
    /// voids it.
    pub fn take_send_request(&mut self) -> Option<String> {
        self.retime();
        let i = self.next_send()?;
        let elapsed = match self.items[i].due? {
            Due::Announced(at) => Instant::now() >= at,
            Due::Asked => true,
        };
        if !elapsed {
            return None;
        }
        // Marked before the text leaves the pane, and by the only path that can
        // hand it out. There is no state in which the same item is returned
        // twice, because after this line it is no longer `Pending` and
        // `next_send` cannot see it — and the due it was holding is withdrawn
        // by the next `retime` for exactly that reason, rather than by a second
        // line here that no test could ever fail without.
        self.items[i].state = ItemState::Sent;
        Some(self.items[i].text.clone())
    }

    /// The next item to start as a background agent. Independent of readiness
    /// — nothing is being typed at.
    ///
    /// Independent of [`Dispatcher`] too, and that is a judgement rather than
    /// an oversight: this pane holds the dispatcher but does not run it —
    /// starting a process blocks, and `Pane::tick` may not — so whoever does is
    /// also the one that can tell whether it is available. An item handed out
    /// and then found unstartable comes back through
    /// [`QueuePane::note_dispatched`] as a [`ItemState::Failed`] with the
    /// reason on it, which is a better answer than a queue that silently
    /// refuses to drain.
    pub fn take_dispatch_request(&mut self) -> Option<String> {
        let i = self.next_dispatch()?;
        // The placeholder is the whole safety mechanism, exactly as above: an
        // item awaiting its outcome is no longer `Pending`, so a second ask
        // cannot see it. `started` is the longer-lived half of the same fact —
        // see `note_dispatched`.
        self.items[i].state = ItemState::Dispatched { id: None };
        self.items[i].started = true;
        Some(self.items[i].text.clone())
    }

    /// How a dispatch turned out, once the worker thread knows.
    ///
    /// Applied to the oldest item still waiting for an outcome, because the
    /// signature carries no token to correlate on and the order requests were
    /// handed out in is the only ordering there is. One ambiguity survives
    /// that: an `Ok` whose output [`crate::dispatch::parse_started`] cannot read
    /// leaves the item with no id, which is indistinguishable from one still
    /// waiting, so a second outcome arriving while the first is in that state
    /// lands on the wrong row.
    ///
    /// What that costs is a misplaced status line, and saying so takes an
    /// argument this method cannot make on its own. It used to cost more: a
    /// mislanded `Err` marked a *started* item `Failed`, `Enter` on a `Failed`
    /// item put it back to `Pending`, and the queue dispatched it again — a
    /// second unattended agent on the same prompt, out of a bookkeeping slip.
    /// [`Item::started`] closes that: an item that has ever been handed to a
    /// dispatcher is never re-queued, whatever its state ends up saying.
    /// Correlating outcomes properly would need a token in this signature,
    /// which is a change to the shell rather than to this file.
    pub fn note_dispatched(&mut self, outcome: Result<Started>) {
        let Some(i) = self
            .items
            .iter()
            .position(|item| matches!(item.state, ItemState::Dispatched { id: None }))
        else {
            return;
        };
        self.items[i].state = match outcome {
            // `id` is what `claude agents` lists and therefore what the roster
            // can be joined on; the session id is the fallback, so an item is
            // at least identifiable when the short one is missing.
            Ok(started) => ItemState::Dispatched {
                id: started.id.or(started.session_id),
            },
            Err(e) => ItemState::Failed(clip(&e.to_string(), 200)),
        };
    }

    /// What the left pane's title should say about the queue, or `None` when
    /// there is nothing worth the columns. This is the only place a send is
    /// announced before it happens, so it must never be silent while an item is
    /// due.
    ///
    /// Ordered so that the countdown survives a clip: the title it joins is
    /// the *agent's*, which already carries a program name and whatever else
    /// the left border says, and the seconds are the half of this a reader
    /// must be able to act on.
    pub fn title_note(&self) -> Option<String> {
        let pending = self.pending();
        if let Some(secs) = self.countdown() {
            return Some(match pending {
                0 | 1 => format!("sending in {secs}s"),
                n => format!("sending in {secs}s · queue {n}"),
            });
        }
        let failed = self.failed();
        match (pending, failed) {
            (0, 0) => None,
            (0, f) => Some(format!("queue · {f} failed")),
            (p, 0) => Some(format!("queue {p}")),
            (p, f) => Some(format!("queue {p} · {f} failed")),
        }
    }

    fn pending(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.state == ItemState::Pending)
            .count()
    }

    /// Test seam, mirroring `GitPane::stub_open_request`.
    #[cfg(test)]
    pub fn stub_item(&mut self, text: &str, mode: Mode) {
        self.items.push(Item {
            text: text.to_string(),
            mode,
            state: ItemState::Pending,
            due: None,
            started: false,
        });
    }

    /// Forget a question this pane was in the middle of asking.
    ///
    /// Called by the shell when the pane leaves the screen, and it has to be
    /// the shell that calls it: a pane is not told it has been put away, and
    /// `tick` runs whether or not it is showing, so nothing in here can tell
    /// the difference. Without it a `d` could be asked on one screen and
    /// answered on another — glance at the queue, press `d`, `Alt+G` off to
    /// git, come back an hour later, press `d`, and an item goes on what the
    /// user experienced as a single press.
    ///
    /// This is the whole of the gap between this guard and `Alt+Q`'s, which
    /// `crate::app` clears before it matches any key at all. See [`Confirm`].
    pub fn cancel_confirm(&mut self) {
        self.confirm = None;
    }

    /// Whether the shell has told this pane about a draft. A test seam for the
    /// wiring in `crate::app`, which is the only thing that can set it — and the
    /// attribute comes off the day the shell needs the answer at runtime.
    #[cfg(test)]
    pub fn is_draft_open(&self) -> bool {
        self.draft_open
    }

    // --- the announcement -------------------------------------------------

    /// Announce a send, or withdraw the announcement.
    ///
    /// Called from everything that can change one of the four conditions, and
    /// from `tick`, so that no path can leave an announcement standing past the
    /// moment it stopped being true. Withdrawing *discards* the elapsed time
    /// rather than pausing it: a send that was two seconds into its warning
    /// when you started typing gets a fresh delay afterwards, because the
    /// warning is for the person who is about to be typed at and they have only
    /// just stopped doing something else.
    ///
    /// At most one item is ever due, and it is always the one next in line.
    /// That is what makes promotion safe: `Enter` on a later item moves it to
    /// the head, and the announcement the previous head was holding is
    /// withdrawn here rather than firing unannounced once the promoted item has
    /// gone.
    ///
    /// Returns whether any of that changed, which is exactly when the title and
    /// the status line differ from what is on screen.
    fn retime(&mut self) -> bool {
        // Conditions 2 and 3. They govern both kinds of due: a by-hand ask is
        // attended, not exempt, and the agent can go busy between the keystroke
        // that asked for it and the pass that would deliver it.
        let safe = self.readiness == Readiness::Idle && !self.draft_open;
        let armed = self.armed;
        let next = self.next_send();
        let mut changed = false;

        for (i, item) in self.items.iter_mut().enumerate() {
            let Some(due) = item.due else { continue };
            let keep = Some(i) == next
                && safe
                // Condition 1, and the only place `armed` is consulted: it
                // governs the sender that runs unattended, so it withdraws an
                // announcement and leaves an ask alone.
                && match due {
                    Due::Announced(_) => armed,
                    Due::Asked => true,
                };
            if !keep {
                item.due = None;
                changed = true;
            }
        }

        if safe
            && armed
            && let Some(i) = next
            && self.items[i].due.is_none()
        {
            self.items[i].due = Some(Due::Announced(Instant::now() + self.arm_delay));
            changed = true;
        }

        if changed {
            self.shown = self.countdown();
        }
        changed
    }

    /// Seconds until the send that is coming, if one is. A [`Due::Asked`] is due
    /// now and reads as zero, which is true for the fraction of a pass it exists
    /// for.
    fn countdown(&self) -> Option<u64> {
        match self.items.get(self.next_send()?)?.due? {
            Due::Announced(at) => Some(secs_left(at)),
            Due::Asked => Some(0),
        }
    }

    fn next_send(&self) -> Option<usize> {
        self.index_of(Mode::Send)
    }

    fn next_dispatch(&self) -> Option<usize> {
        self.index_of(Mode::Dispatch)
    }

    fn index_of(&self, mode: Mode) -> Option<usize> {
        self.items
            .iter()
            .position(|i| i.mode == mode && i.state == ItemState::Pending)
    }

    fn failed(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.state, ItemState::Failed(_)))
            .count()
    }

    /// Whether anything in the list would start an unattended agent. What the
    /// status line discloses, and the reason it is keyed on the list rather than
    /// on the selection: the warning has to be true of what is queued, not of
    /// what happens to be highlighted.
    fn dispatch_pending(&self) -> bool {
        self.next_dispatch().is_some()
    }

    // --- keys -------------------------------------------------------------

    /// The list. Every printable key here is a command, which is only safe
    /// because a pane reads keys when it has focus and at no other time.
    ///
    /// The scroll vocabulary comes first and keeps everything it claims,
    /// `space` included. Arming wanted that key — it is the switch that decides
    /// whether abeam types at the agent unasked, and it is the biggest key on
    /// the board — and it does not get it: `space` pages in four other panes
    /// and is promised by name in the F1 overlay, so a fifth pane where it
    /// toggles a mode is exactly the collision this program rebinds keys to
    /// avoid. Arming is `a` instead, which is free because `i` carries opening
    /// the composer on its own.
    fn list_key(&mut self, key: KeyEvent) -> Handled {
        // Taken before a single key is matched, so that *any* key is the answer
        // no -- which is the whole of the guard, and the same shape `Alt+Q`'s
        // double press has in `crate::app`. A confirmation only some keys
        // cancelled would be one a typist walks straight through.
        let cancelled = self.confirm.take();
        let was_asking = cancelled.is_some();
        let handled = self.list_key_after(key, cancelled);
        // A question that was on screen a moment ago and is not now costs a
        // frame even when the key itself did nothing: leaving `d again to
        // delete` under the press that cancelled it is the foot line lying
        // about what the next `d` would do.
        //
        // **Except the way out.** A bare `Esc` or `q` is how you leave this
        // pane — the shell reads an unhandled one as "give focus back to the
        // agent" — and claiming it to repaint a line would cost the user that
        // key at the one moment they are most likely to want it, which is
        // having just realised their keys are somewhere they did not put them.
        // No frame is lost by declining: handing focus back draws one.
        let chord = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let leaving = !chord && matches!(key.code, KeyCode::Esc | KeyCode::Char('q'));
        if was_asking && !leaving {
            return Handled::Yes;
        }
        handled
    }

    /// The list's keys, with whatever the press before this one was waiting to
    /// have confirmed.
    fn list_key_after(&mut self, key: KeyEvent, confirming: Option<Confirm>) -> Handled {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        // One shared scroll vocabulary, so the F1 table cannot be true here
        // and false in the pane next door.
        if let Some(handled) = self.scroll.key(key) {
            return handled;
        }
        // Ctrl+letter is the agent's everywhere else in the program, and only
        // the two half-page keys above are claimed inside a pane. Without this
        // `Ctrl+I` would open the composer.
        if ctrl || alt {
            return Handled::No;
        }

        match key.code {
            KeyCode::Tab => self.select(1),
            KeyCode::BackTab => self.select(-1),
            KeyCode::Char('i') => {
                self.composing = Some(String::new());
                Handled::Yes
            }
            // Arming. Not `space`, for the reason on this function.
            KeyCode::Char('a') => {
                self.armed = !self.armed;
                self.retime();
                Handled::Yes
            }
            KeyCode::Char('d') => {
                // The question has to have been asked about the row this press
                // would act on, or a click between the two presses turns one
                // warning into a different item's deletion.
                let confirmed = match &confirming {
                    Some(Confirm::Delete(asked)) => self
                        .items
                        .get(self.selected)
                        .is_some_and(|item| &item.text == asked),
                    _ => false,
                };
                self.delete(confirmed)
            }
            KeyCode::Char('m') => self.switch_mode(),
            KeyCode::Char('r') => self.clear_finished(matches!(confirming, Some(Confirm::Clear))),
            KeyCode::Enter => self.now(),
            // Esc and q are not ours. The shell reads an unhandled one as
            // "give focus back to the agent", which is the way out of here.
            _ => Handled::No,
        }
    }

    /// The composer. Every printable key is text — `q`, `j` and `g` included —
    /// which is the trade any type-into-a-pane makes, and the reason
    /// `takes_input` and `exit_hint` both have to change while one is open.
    fn compose_key(&mut self, key: KeyEvent) -> Handled {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            // Closes the composer and stays in the pane. Being thrown out to
            // the agent by the key that means "never mind" is the single most
            // annoying thing a text box can do, and `browse.rs` decided the
            // same thing about its find.
            KeyCode::Esc => {
                self.composing = None;
                Handled::Yes
            }
            // A newline in the item rather than a commit. Windows reports the
            // modifier on `Enter`, so abeam can tell these apart here; a
            // terminal that folds them into a bare `CR` leaves items
            // single-line, and a paste is then the way to a multi-line one.
            //
            // On Unix that second sentence is the usual case rather than the
            // exception, and it is worth being plain about it: a terminal only
            // distinguishes `Ctrl+Enter` from `Enter` if it speaks the Kitty
            // keyboard protocol, and abeam does not ask for it (`term::setup`
            // enables raw mode, the alternate screen, mouse capture and
            // bracketed paste, and nothing else). So this arm is mostly
            // unreachable on Linux and pasting is how a multi-line item gets
            // written. Left as it is rather than papered over: requesting the
            // protocol changes how *every* key arrives, which is a keymap
            // question — `docs/keymap.md` — and not one to answer as a side
            // effect of making the queue nicer.
            KeyCode::Enter if ctrl || alt => {
                if let Some(draft) = self.composing.as_mut() {
                    draft.push('\n');
                }
                Handled::Yes
            }
            KeyCode::Enter => self.commit(),
            KeyCode::Backspace => {
                let Some(draft) = self.composing.as_mut() else {
                    return Handled::No;
                };
                // Backspacing past the start deliberately does *not* close the
                // composer, which is where this parts company with the find in
                // `browse.rs`. There the next keystroke would move a cursor;
                // here `d` deletes an item, so a box that closes itself one
                // key early turns a typo into a lost item.
                draft.pop().is_some().into()
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                if let Some(draft) = self.composing.as_mut() {
                    draft.push(c);
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    /// `Enter` in the composer: add what has been typed and stay open.
    ///
    /// Staying open is what makes queueing three things three lines of typing
    /// rather than three round trips through `i`. An empty draft then means
    /// "that was the last one", which is also what `Enter` on a box you have
    /// typed nothing into ought to do.
    fn commit(&mut self) -> Handled {
        let text = self.composing.take().unwrap_or_default();
        let text = text.trim();
        if text.is_empty() {
            return Handled::Yes;
        }
        let text = text.to_string();
        self.composing = Some(String::new());
        self.push(text);
        Handled::Yes
    }

    /// A new item, at the end of the list and selected.
    ///
    /// Always [`Mode::Send`], because the two modes are not equally safe to
    /// guess at: a `Send` item waits for the queue to be armed and the agent to
    /// be idle, while a `Dispatch` item starts a process that edits files
    /// without asking. The mode that does nothing until told is the one a new
    /// item gets; `m` is one key away, and the status line says what that key
    /// has just changed.
    fn push(&mut self, text: String) {
        self.items.push(Item {
            text,
            mode: Mode::Send,
            state: ItemState::Pending,
            due: None,
            started: false,
        });
        self.selected = self.items.len() - 1;
        self.reveal();
        self.retime();
    }

    /// `Enter` in the list: do the selected item now.
    ///
    /// It skips two of the module docs' four conditions: the countdown (4) and
    /// the arming switch (1). The other two stand, and the readiness and draft
    /// checks are not a delay to be impatient with — they are the difference
    /// between a prompt and a permission dialog answered by its first
    /// character.
    ///
    /// ## Why `armed` is skipped, having first been kept
    ///
    /// This refused while disarmed for a while, on an argument worth recording
    /// because it is the one a reader will re-derive:
    /// [`QueuePane::take_send_request`] has four conditions and no exception to
    /// any of them, which is a property that can be read off one function, and a
    /// bypass would move part of the rule into a key handler — where the next
    /// person to add a way of sending something has to know to re-check it.
    ///
    /// That was a false dilemma. `armed` is consulted in [`QueuePane::retime`],
    /// not in the drain; the drain only asks whether the item in front of it is
    /// due and elapsed. So granting a [`Due::Asked`] that `retime` declines to
    /// withdraw leaves exactly one choke point, still with no exception in it.
    /// Nothing had to be given up for it.
    ///
    /// And keeping it cost more than the keystroke it looked like costing. There
    /// was no way to hand-drain a single item without first turning the
    /// unattended sender on, so somebody who wanted "just this one, now" pressed
    /// `a`, pressed `Enter`, and walked away with the queue **armed** — and the
    /// rest of the list then drained on its own, which is precisely what they
    /// did not ask for. A mode the user has to remember to switch back is a
    /// worse failure than the one the strictness was preventing.
    fn now(&mut self) -> Handled {
        let Some(item) = self.items.get(self.selected) else {
            return Handled::No;
        };
        let (mode, state, started) = (item.mode, item.state.clone(), item.started);

        match state {
            // Not a retry, and this is the second half of `note_dispatched`'s
            // problem: abeam cannot always tell a dispatch that failed from one
            // whose success it could not read, so re-queueing a failure could
            // put a second unattended agent on a prompt that already has one.
            // An item that never reached a dispatcher can go back in the queue;
            // one that did is finished, and writing it out again is a
            // deliberate act with a keystroke of its own.
            ItemState::Failed(_) if !started => {
                self.items[self.selected].state = ItemState::Pending;
                self.retime();
                Handled::Yes
            }
            ItemState::Pending => match mode {
                Mode::Send => {
                    if !self.readiness.is_idle() || self.draft_open {
                        // Nothing changed, so nothing is redrawn — and the
                        // status line already says which of the two it was.
                        return Handled::No;
                    }
                    self.selected = self.promote(self.selected);
                    // Due at once, and still drained by `take_send_request`:
                    // one path hands items to the shell and one path only.
                    self.items[self.selected].due = Some(Due::Asked);
                    self.shown = self.countdown();
                    Handled::Yes
                }
                // A dispatch is started as soon as it is asked for, so there is
                // no wait to skip: all this can do is put the item at the head
                // of its own queue.
                Mode::Dispatch => {
                    let to = self.promote(self.selected);
                    let moved = to != self.selected;
                    self.selected = to;
                    moved.into()
                }
            },
            _ => Handled::No,
        }
    }

    /// Move an item in front of every other pending item of its mode, and
    /// report where it landed. Finished items stay above it: the list is a
    /// record of what was asked for, in the order it was asked.
    fn promote(&mut self, idx: usize) -> usize {
        let Some(item) = self.items.get(idx) else {
            return idx;
        };
        let Some(head) = self.index_of(item.mode) else {
            return idx;
        };
        if head >= idx {
            return idx;
        }
        let item = self.items.remove(idx);
        self.items.insert(head, item);
        head
    }

    fn select(&mut self, delta: isize) -> Handled {
        let n = self.items.len() as isize;
        if n == 0 {
            return Handled::No;
        }
        // Wraps, as it does in the git view: with one screenful of items, Tab
        // from the last back to the first is what a reader means by it.
        let to = (((self.selected as isize + delta) % n + n) % n) as usize;
        if to == self.selected {
            return Handled::No;
        }
        self.selected = to;
        self.reveal();
        Handled::Yes
    }

    /// `d`, twice. The first press arms [`Confirm::Delete`] and says so; this
    /// is the second one.
    ///
    /// `confirmed` is the caller's answer to "was the question asked about
    /// *this* row", not merely "was a question asked" — see the arm that
    /// computes it. `selected` moves under a click and under a paste as well as
    /// under a key, so a confirmation that only remembered that it had been
    /// armed would delete whichever row happened to be under it by the time the
    /// second press arrived.
    fn delete(&mut self, confirmed: bool) -> Handled {
        let Some(item) = self.items.get(self.selected) else {
            return Handled::No;
        };
        if !confirmed {
            self.confirm = Some(Confirm::Delete(item.text.clone()));
            return Handled::Yes;
        }
        // The item's announcement goes with it, because it is a field of the
        // item. That is the whole reason `due` lives where it does.
        self.items.remove(self.selected);
        self.clamp();
        self.retime();
        Handled::Yes
    }

    /// `m`, and only on an item that has not gone anywhere yet. Re-labelling
    /// something already sent would be the pane lying about its own history.
    fn switch_mode(&mut self) -> Handled {
        let Some(item) = self.items.get_mut(self.selected) else {
            return Handled::No;
        };
        if item.state != ItemState::Pending {
            return Handled::No;
        }
        item.mode = match item.mode {
            Mode::Send => Mode::Dispatch,
            Mode::Dispatch => Mode::Send,
        };
        self.retime();
        Handled::Yes
    }

    /// `r`: everything abeam has already handed over or failed to hand over.
    /// A dispatched agent that is still running goes too — it is `claude
    /// agents`' to report from then on, and this pane was only ever showing a
    /// borrowed row.
    ///
    /// `r` twice, like `d`, and the emptiness check comes *first* so that `r`
    /// over a list with nothing finished in it stays the no-op it always was
    /// rather than arming a confirmation for nothing. What it would clear is
    /// counted at the second press, not the first: an item that finished in
    /// between is one more thing abeam has already handed over, which is what
    /// the key means.
    fn clear_finished(&mut self, confirmed: bool) -> Handled {
        if self.items.iter().all(|i| i.state == ItemState::Pending) {
            return Handled::No;
        }
        if !confirmed {
            self.confirm = Some(Confirm::Clear);
            return Handled::Yes;
        }
        self.items.retain(|i| i.state == ItemState::Pending);
        self.clamp();
        self.retime();
        Handled::Yes
    }

    fn clamp(&mut self) {
        self.selected = self.selected.min(self.items.len().saturating_sub(1));
        self.reveal();
    }

    /// Bring the selected row into view without recentring — a list that jumps
    /// under you is harder to read than one that scrolls by a line.
    fn reveal(&mut self) {
        let page = self.scroll.viewport().max(1);
        if self.selected < self.scroll.offset {
            self.scroll.to(self.selected);
        } else if self.selected >= self.scroll.offset + page {
            self.scroll.to(self.selected + 1 - page);
        }
    }

    // --- drawing ----------------------------------------------------------

    /// Rows the notice above the list takes, and therefore the offset every row
    /// of the list is drawn at.
    ///
    /// Read by `render` and by `handle_mouse`, from here and nowhere else: a
    /// click that landed on a different row from the one that was drawn would
    /// silently re-aim `Enter` at an item nobody pointed at, and two copies of
    /// this expression is exactly how that happens.
    fn notice_rows(&self) -> u16 {
        u16::from(self.dispatcher.is_err())
    }

    /// Rows of the pane the list itself gets: everything but the notice and the
    /// status line or composer at the bottom.
    fn list_height(&self) -> usize {
        (self.drawn.height as usize)
            .saturating_sub(self.notice_rows() as usize)
            .saturating_sub(1)
    }

    fn item_line(&self, i: usize, w: usize, selected: bool) -> Line<'static> {
        let item = &self.items[i];
        let (mark, mark_style) = marker(&item.state);
        let mut spans = vec![
            Span::styled(format!(" {} ", sigil(item.mode)), mode_style(item.mode)),
            Span::styled(format!("{mark} "), mark_style),
        ];
        let lead: usize = spans.iter().map(|s| s.content.width()).sum();

        // In a narrow pane the text is worth more than the annotation, and a
        // two-column budget for a prompt is not a row anybody can read.
        let aside = if w >= ASIDE_MIN_WIDTH {
            self.aside(item)
        } else {
            String::new()
        };
        let gap = if aside.is_empty() { 0 } else { 2 };
        let budget = w.saturating_sub(lead + aside.width() + gap);

        // The first line, and a count of the ones it is standing in for. A
        // pasted block is one item, and a row that showed only its first line
        // with no sign of the rest would misrepresent what is queued.
        let mut head = item.text.lines().next().unwrap_or_default().to_string();
        let rest = item.text.lines().count().saturating_sub(1);
        if rest > 0 {
            head.push_str(&format!("  +{rest}"));
        }
        // Measured in cells and clipped here, because `str::len` is wrong
        // about a CJK ideograph twice over and a row that overflows its rect
        // corrupts the frame rather than merely looking wrong.
        let head = clip(&head, budget);
        let pad = budget.saturating_sub(head.width());
        spans.push(Span::raw(head));
        if !aside.is_empty() {
            spans.push(Span::raw(" ".repeat(pad + gap)));
            spans.push(Span::styled(aside, dim()));
        }

        let mut spans = clip_line(Line::from(spans), w).spans;
        if !selected {
            return Line::from(spans);
        }
        // Padded to the full width, or the highlight would stop at the end of
        // the text instead of marking the row.
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        Line::from(spans).style(Style::default().bg(Color::DarkGray))
    }

    /// What an item says about itself at the right-hand end of its row.
    fn aside(&self, item: &Item) -> String {
        match &item.state {
            ItemState::Dispatched { id: Some(id) } => {
                live_of(&self.roster, id).unwrap_or("started").to_string()
            }
            // Handed to the shell, which has not come back yet — usually for
            // under a second, and permanently when Claude printed a start
            // `crate::dispatch::parse_started` could not read an id out of.
            // That agent is running; abeam simply cannot name it.
            ItemState::Dispatched { id: None } => "starting…".to_string(),
            ItemState::Failed(why) => why.clone(),
            _ => String::new(),
        }
    }

    /// The bottom row: what is being typed, or what the queue is doing.
    fn foot_line(&self, w: usize) -> Line<'static> {
        if self.composing.is_some() {
            let (shown, _) = self.composer_view(w);
            return Line::from(vec![Span::styled(PROMPT, dim()), Span::raw(shown)]);
        }

        let mut spans = vec![Span::raw(" ")];
        // **Leading**, for the reason the focus hint leads the border: this
        // line is clipped from the right, and a question somebody has to answer
        // with their next keystroke is the last thing on it that may be clipped
        // away. Yellow, the colour this program keeps for "abeam is in a state
        // you did not leave it in".
        if let Some(confirm) = &self.confirm {
            let said = match confirm {
                Confirm::Delete(_) => "d again to delete".to_string(),
                Confirm::Clear => {
                    let n = self
                        .items
                        .iter()
                        .filter(|i| i.state != ItemState::Pending)
                        .count();
                    format!("r again to clear {n} finished")
                }
            };
            spans.push(Span::styled(
                format!("{said} · "),
                Style::default().fg(Color::Yellow),
            ));
        }
        spans.push(if self.armed {
            Span::styled("armed", Style::default().fg(Color::Yellow))
        } else {
            Span::styled("disarmed", dim())
        });
        spans.push(Span::styled(" · agent ", dim()));
        spans.push(match self.readiness {
            Readiness::Idle => Span::styled("idle", Style::default().fg(Color::Green)),
            Readiness::Busy => Span::styled("busy", Style::default().fg(Color::Yellow)),
            // Named rather than hidden. It is not a worse `busy`, it is the
            // state in which abeam does not know, and nothing will be sent
            // until it does.
            Readiness::Unknown => Span::styled("state unknown", dim()),
        });
        if self.draft_open {
            spans.push(Span::styled(" · you are typing", dim()));
        }
        if let Some(secs) = self.countdown() {
            spans.push(Span::styled(
                format!(" · sending in {secs}s"),
                Style::default().fg(Color::Yellow),
            ));
        } else if !self.armed && self.next_send().is_some() {
            // The one key that is otherwise undiscoverable from in here: the
            // list looks identical whether or not anything will ever leave it.
            spans.push(Span::styled(" · a arms", dim()));
        }
        // Last, so that the volatile half of the line leads — but present
        // whenever the list holds one of these, which is the point. The
        // empty-state prose says the same thing and vanishes with the first
        // item; this does not.
        if self.dispatch_pending() {
            spans.push(Span::styled(
                format!(" · {DISPATCH_WARNING}"),
                Style::default().fg(Color::Magenta),
            ));
        }
        clip_line(Line::from(spans), w)
    }

    /// The draft as it fits on one row, and the column the cursor sits in.
    ///
    /// Elided from the *left*, because the end of the draft is where the
    /// typing is happening. One cell is held back for the cursor itself, or it
    /// would sit outside the pane on a full row.
    fn composer_view(&self, w: usize) -> (String, usize) {
        let draft = self.composing.as_deref().unwrap_or_default();
        // The line being typed, which for a multi-line item is the last one.
        let line = draft.rsplit('\n').next().unwrap_or_default();
        let avail = w.saturating_sub(PROMPT.width());
        let shown = elide_left(line, avail.saturating_sub(1));
        (shown.clone(), PROMPT.width() + shown.width())
    }

    /// What an empty list says. Never nothing: a blank pane is
    /// indistinguishable from a broken one, and this pane is empty every time
    /// it is opened for the first time.
    fn nothing(&self) -> String {
        format!(
            "Nothing queued yet.\n\
             \n\
             i writes an item, and a pasted block becomes one.\n\
             \n\
             → items are typed into the agent next door when it goes idle and the \
             queue is armed; a arms it. ⇉ items start as background agents of \
             their own, which edit files without asking while you are looking at \
             something else. m switches an item between the two, d deletes one, \
             enter does the selected one now, r clears what has finished.\n\
             \n\
             Work runs in {}",
            self.root.display()
        )
    }
}

impl Pane for QueuePane {
    /// Clipped from the right in a 46-column pane, so it leads with the two
    /// things worth the last few columns: that this is the queue, and how much
    /// is in it waiting.
    fn title(&self) -> String {
        if self.composing.is_some() {
            return "queue · new item · enter adds".to_string();
        }
        if self.items.is_empty() {
            return "queue · empty".to_string();
        }
        let mut t = match self.pending() {
            0 => "queue · clear".to_string(),
            n => format!("queue · {n} pending"),
        };
        if self.armed {
            t.push_str(" · armed");
        }
        match self.failed() {
            0 => {}
            n => t.push_str(&format!(" · {n} failed")),
        }
        t
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        self.drawn = inner;
        if inner.width == 0 || inner.height == 0 {
            self.scroll.measure(self.items.len(), 0);
            return;
        }
        let mut area = inner;

        // Above the list rather than below it: a pane whose second mode cannot
        // work at all has to say so where somebody looking for that mode will
        // read it, and the bottom of a scrolled list is not that place.
        if let Err(Unavailable(why)) = &self.dispatcher {
            let line = clip_line(
                Line::from(vec![
                    Span::styled(" dispatch unavailable · ", err()),
                    Span::styled(why.clone(), dim()),
                ]),
                area.width as usize,
            );
            f.render_widget(Paragraph::new(line), Rect { height: 1, ..area });
        }
        let notice = self.notice_rows().min(area.height);
        area.y += notice;
        area.height -= notice;
        if area.height == 0 {
            self.scroll.measure(self.items.len(), 0);
            return;
        }

        // The last row belongs to the composer while one is open and to the
        // status line otherwise. One row, always spent, because both of them
        // answer the question a queue raises — is anything going to happen? —
        // and a pane that answers it only sometimes is one you have to learn.
        let foot = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        f.render_widget(Paragraph::new(self.foot_line(area.width as usize)), foot);

        let list = Rect {
            height: area.height - 1,
            ..area
        };
        // The other half of `notice_rows`: what was drawn and what a click will
        // be measured against are the same number, or they are a bug.
        debug_assert_eq!(list.height as usize, self.list_height());
        self.scroll.measure(self.items.len(), list.height as usize);
        if list.height == 0 {
            return;
        }

        // The scrollbar takes a column from the text rather than sitting on
        // top of it: an elided prompt is worse than a narrower one.
        let text_w = list.width - scroll::bar_width(list.width);
        let text = Rect {
            width: text_w,
            ..list
        };
        if self.items.is_empty() {
            f.render_widget(
                Paragraph::new(block(&self.nothing(), text_w as usize, dim())),
                text,
            );
            return;
        }

        let lines: Vec<Line> = (self.scroll.offset..self.items.len())
            .take(list.height as usize)
            .map(|i| self.item_line(i, text_w as usize, i == self.selected))
            .collect();
        f.render_widget(Paragraph::new(lines), text);
        self.scroll.render_bar(f, list);
    }

    /// Must be false on the overwhelming majority of calls — including every
    /// call while a countdown is running that has not crossed a second
    /// boundary. Returning true each loop would re-render the agent's screen
    /// at the frame ceiling to redraw a number that has not changed.
    fn tick(&mut self) -> bool {
        // Cheap, and the belt to `set_readiness`'s braces: every path that
        // changes a condition already calls this, and a path that is added
        // later and forgets to is an announcement that outlives its promise.
        let mut dirty = self.retime();
        let now = self.countdown();
        if self.shown != now {
            self.shown = now;
            dirty = true;
        }
        dirty
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        Ok(match self.composing {
            Some(_) => self.compose_key(key),
            None => self.list_key(key),
        })
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        // Before the wheel is offered it, so that both ways of moving this list
        // give the same answer to a pending question: `Alt+J` arrives as a key
        // and cancels, and a wheel that did not would be the same gesture with
        // two meanings. `crate::app` cancels a pending quit on a mouse press
        // for the reason that covers both — the user has moved on to something
        // else — and this is that line, one pane down.
        self.confirm = None;
        if let Some(handled) = self.scroll.mouse(ev) {
            return Ok(handled);
        }
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(row) = ev.row.checked_sub(self.notice_rows()) else {
                    return Ok(Handled::No);
                };
                // Below the last row, or on the status line, there is nothing
                // to choose. Snapping to the end of the list instead would
                // mean a click on the empty half of a short pane silently
                // re-aiming `Enter`.
                if row as usize >= self.list_height() {
                    return Ok(Handled::No);
                }
                let i = self.scroll.offset + row as usize;
                if i >= self.items.len() || i == self.selected {
                    return Ok(Handled::No);
                }
                self.selected = i;
                self.reveal();
                Ok(Handled::Yes)
            }
            _ => Ok(Handled::No),
        }
    }

    /// True while composing, and false otherwise: the list is a read-only view
    /// with a scroll vocabulary, and `Esc` there means what it means in every
    /// other read-only pane. See `crate::pane::Pane::takes_input`.
    fn takes_input(&self) -> bool {
        self.composing.is_some()
    }

    /// While the composer is open `Esc` closes it and leaves you here, one
    /// press short of the agent — so the border must not promise otherwise.
    fn exit_hint(&self) -> &'static str {
        if self.composing.is_some() {
            "esc→list"
        } else {
            "esc→agent"
        }
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.composing.as_ref()?;
        if self.drawn.width == 0 || self.drawn.height == 0 {
            return None;
        }
        let (_, col) = self.composer_view(self.drawn.width as usize);
        Some((
            (col as u16).min(self.drawn.width - 1),
            self.drawn.height - 1,
        ))
    }

    /// A pasted block becomes one item — the fastest way to get a real prompt
    /// into the queue, and the reason this pane takes a paste while the other
    /// read-only views decline one.
    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        // A paste is a keystroke as far as a pending question is concerned, and
        // `crate::app` learned that about `Alt+Q` the hard way: "any other key
        // cancels it" has to include the ones that do not arrive as keys. Here
        // it is sharper than an ordinary cancel, because a paste *appends an
        // item and selects it* — so without this, `d`, paste, `d` destroyed the
        // text that had just been pasted while the row the question named
        // survived.
        self.confirm = None;
        // A Windows paste arrives with CRLF in it, and this text is on its way
        // to being typed at an agent one character at a time.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.composing.is_some() {
            if text.is_empty() {
                return Ok(Handled::No);
            }
            if let Some(draft) = self.composing.as_mut() {
                draft.push_str(&text);
            }
            return Ok(Handled::Yes);
        }
        let text = text.trim();
        if text.is_empty() {
            return Ok(Handled::No);
        }
        self.push(text.to_string());
        Ok(Handled::Yes)
    }
}

// ---------------------------------------------------------------------------
// rows
// ---------------------------------------------------------------------------

/// Where an item is going, in one column. A sigil rather than a colour,
/// because colour is the first thing a terminal takes away.
fn sigil(mode: Mode) -> &'static str {
    match mode {
        Mode::Send => "→",
        Mode::Dispatch => "⇉",
    }
}

fn mode_style(mode: Mode) -> Style {
    match mode {
        Mode::Send => Style::default().fg(Color::Cyan),
        Mode::Dispatch => Style::default().fg(Color::Magenta),
    }
}

fn marker(state: &ItemState) -> (&'static str, Style) {
    match state {
        ItemState::Pending => ("·", dim()),
        ItemState::Sent => ("✓", Style::default().fg(Color::Green)),
        ItemState::Dispatched { .. } => ("»", Style::default().fg(Color::Cyan)),
        ItemState::Failed(_) => ("✗", err()),
    }
}

/// What the roster says about one dispatched agent: its background `state`
/// where it has one, and its interactive `status` otherwise.
fn live_of<'a>(roster: &'a [Session], id: &str) -> Option<&'a str> {
    let session = roster.iter().find(|s| s.id.as_deref() == Some(id))?;
    session.state.as_deref().or(session.status.as_deref())
}

/// Whole seconds still to go, rounded up, so a three-second countdown reads
/// 3, 2, 1 and never shows a 0 it has not yet acted on.
fn secs_left(due: Instant) -> u64 {
    let left = due.saturating_duration_since(Instant::now());
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentstate::Kind;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// A pane with no dispatcher behind it.
    ///
    /// Nothing in this file starts a process or opens a pty, and a real
    /// `Dispatcher` cannot be built without one — which is also why the
    /// notice-less layout is not exercised here: `Ok(Dispatcher)` is not a value
    /// a test can construct. The two layouts differ by exactly
    /// [`QueuePane::notice_rows`], which `render` and `handle_mouse` both read
    /// and neither recomputes.
    fn pane() -> QueuePane {
        unavailable("no claude on PATH")
    }

    fn unavailable(why: &str) -> QueuePane {
        QueuePane::with_dispatcher(PathBuf::from("/repo"), Err(Unavailable(why.to_string())))
    }

    /// Armed, idle, one thing to send: the state every safety test starts from
    /// and then breaks one condition of.
    fn ready(text: &str) -> QueuePane {
        let mut p = pane();
        p.stub_item(text, Mode::Send);
        p.armed = true;
        p.readiness = Readiness::Idle;
        p
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn wheel_down() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// An announcement that has already elapsed, without a test sleeping for
    /// one. The path a send really arrives by — a `Due::Asked` skips the clock
    /// altogether and would not prove the same thing.
    fn elapsed() -> Option<Due> {
        Some(Due::Announced(
            Instant::now()
                .checked_sub(ARM_DELAY)
                .unwrap_or_else(Instant::now),
        ))
    }

    /// Render one frame and flatten it, so a test can ask what is on screen
    /// rather than what the code meant to put there.
    fn screen(p: &mut QueuePane, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("a test terminal");
        term.draw(|f| p.render(f, Rect::new(0, 0, w, h)))
            .expect("draw the queue");
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// One row of a flattened frame.
    fn row_of(screen: &str, w: u16, row: usize) -> String {
        screen
            .chars()
            .skip(row * w as usize)
            .take(w as usize)
            .collect()
    }

    fn states(p: &QueuePane) -> Vec<ItemState> {
        p.items.iter().map(|i| i.state.clone()).collect()
    }

    fn texts(p: &QueuePane) -> Vec<String> {
        p.items.iter().map(|i| i.text.clone()).collect()
    }

    // --- the one bug that would spam the agent ----------------------------

    #[test]
    fn an_item_is_never_sent_twice_however_often_the_shell_asks() {
        // The shell drains this every pass of its loop. An item that stayed
        // `Pending` after being handed over would be typed at the agent a
        // hundred times a second, and nothing downstream could tell the copies
        // apart from a person queueing the same thing twice.
        //
        // Two items, and both dues re-armed on every pass, so that every one of
        // the asks below reaches the de-duplication rather than stopping at
        // "nothing is due yet" — which is what made the first version of this
        // test decorative.
        let mut p = ready("the first thing");
        p.stub_item("the second thing", Mode::Send);
        p.items[0].due = elapsed();

        assert_eq!(p.take_send_request().as_deref(), Some("the first thing"));
        p.items[1].due = elapsed();
        assert_eq!(p.take_send_request().as_deref(), Some("the second thing"));

        for _ in 0..200 {
            p.items[0].due = elapsed();
            p.items[1].due = elapsed();
            assert_eq!(p.take_send_request(), None, "an item came back");
        }
        assert_eq!(states(&p), [ItemState::Sent, ItemState::Sent]);
    }

    #[test]
    fn a_dispatched_item_is_never_dispatched_twice_however_often_the_shell_asks() {
        let mut p = pane();
        p.stub_item("read the docs", Mode::Dispatch);

        assert_eq!(p.take_dispatch_request().as_deref(), Some("read the docs"));
        for _ in 0..200 {
            assert_eq!(p.take_dispatch_request(), None);
        }
        assert_eq!(states(&p), [ItemState::Dispatched { id: None }]);
    }

    #[test]
    fn a_dispatch_that_has_been_started_is_never_quietly_started_again() {
        // The compound failure this closes. An `Ok` whose output could not be
        // read leaves item one showing `Dispatched { id: None }`; item two's
        // `Err` lands on it; and `Enter` on the failure it now shows used to put
        // it back in the queue — a second unattended agent on a prompt that
        // already had one, out of a bookkeeping slip.
        let mut p = pane();
        p.stub_item("first", Mode::Dispatch);
        p.stub_item("second", Mode::Dispatch);
        p.take_dispatch_request();
        p.take_dispatch_request();
        p.note_dispatched(Err(anyhow::anyhow!("claude exited with 1")));

        p.selected = 0;
        assert_eq!(
            p.handle_key(key(KeyCode::Enter)).unwrap(),
            Handled::No,
            "an item that has been handed to a dispatcher is finished"
        );
        assert_eq!(p.take_dispatch_request(), None);

        // An item that never reached one is a different thing, and can go back
        // in the queue.
        let mut p = pane();
        p.stub_item("never started", Mode::Dispatch);
        p.items[0].state = ItemState::Failed("nothing tried it".into());
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(p.items[0].state, ItemState::Pending);
    }

    // --- when a send is refused -------------------------------------------

    #[test]
    fn nothing_is_sent_while_the_agent_is_busy_or_the_queue_is_disarmed() {
        for break_it in [
            |p: &mut QueuePane| p.readiness = Readiness::Busy,
            |p: &mut QueuePane| p.armed = false,
            |p: &mut QueuePane| p.draft_open = true,
        ] {
            let mut p = ready("do not send me");
            p.items[0].due = elapsed();
            break_it(&mut p);

            assert_eq!(p.take_send_request(), None);
            assert_eq!(states(&p), [ItemState::Pending]);
            // ...and the announcement is withdrawn with it, so the title stops
            // promising a send that is no longer coming.
            assert_eq!(p.items[0].due, None);
            assert_eq!(p.title_note().as_deref(), Some("queue 1"));
        }
    }

    #[test]
    fn an_unknown_readiness_is_treated_as_unsafely_as_a_busy_one() {
        // `Unknown` is the ordinary state in front of any agent that is not
        // Claude, and the whole reason `crate::agentstate` reports three states
        // rather than a bool: a probe that guesses idle when it cannot tell
        // types a prompt into a permission dialog.
        let mut p = ready("do not guess");
        p.readiness = Readiness::Unknown;
        p.items[0].due = elapsed();

        assert_eq!(p.take_send_request(), None);
        assert_eq!(states(&p), [ItemState::Pending]);

        // Not even by the explicit key, which skips the countdown and the
        // arming switch and nothing else.
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert_eq!(p.take_send_request(), None);

        // And it is unsafe in exactly the way `Busy` is: the same lines pass
        // for both.
        p.readiness = Readiness::Busy;
        p.items[0].due = elapsed();
        assert_eq!(p.take_send_request(), None);
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert_eq!(states(&p), [ItemState::Pending]);
    }

    #[test]
    fn a_keystroke_at_the_agent_defers_the_send_and_the_countdown_starts_again_from_full() {
        let mut p = ready("send me later");
        p.arm_delay = Duration::from_secs(9);
        assert!(p.tick(), "a countdown is announced when the agent goes idle");
        assert_eq!(p.countdown(), Some(9));

        // Most of the way through the announcement, and then somebody starts
        // typing at the agent.
        p.items[0].due = Some(Due::Announced(Instant::now() + Duration::from_millis(20)));
        assert!(p.set_draft_open(true));
        assert_eq!(p.items[0].due, None, "a keystroke withdraws it");
        assert_eq!(p.take_send_request(), None);

        // What comes back is a *whole* delay rather than the twenty
        // milliseconds that were left. The warning is for the person who has
        // just stopped doing something else.
        assert!(p.set_draft_open(false));
        assert_eq!(p.countdown(), Some(9));
        assert_eq!(p.take_send_request(), None, "and it has not elapsed");
        assert_eq!(states(&p), [ItemState::Pending]);
    }

    #[test]
    fn a_send_granted_by_hand_cannot_be_inherited_by_the_item_that_follows_it() {
        // The bug this shape exists to make impossible. `Enter` on the second
        // item granted the *pane* a bare due; deleting that item left the
        // elapsed due sitting there for the first, which was then typed at the
        // agent with no countdown and nobody's consent.
        let mut p = ready("the item nobody chose");
        p.stub_item("the item that was chosen", Mode::Send);
        p.armed = false;
        p.selected = 1;

        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        // Twice, because `d` asks first now. See [`Confirm`].
        assert_eq!(p.handle_key(key(KeyCode::Char('d'))).unwrap(), Handled::Yes);
        assert_eq!(p.handle_key(key(KeyCode::Char('d'))).unwrap(), Handled::Yes);
        assert_eq!(texts(&p), ["the item nobody chose"]);
        assert_eq!(
            p.take_send_request(),
            None,
            "a due outlived the item it was granted for"
        );
        assert_eq!(states(&p), [ItemState::Pending]);
    }

    #[test]
    fn a_send_is_announced_before_it_happens_and_never_silently() {
        let mut p = ready("announce me");
        assert!(p.title_note().is_some());
        p.tick();

        // Exact, because the number is what a reader acts on: a pane counting
        // down from thirty must not read as one counting down from three.
        assert_eq!(p.title_note().as_deref(), Some("sending in 3s"));

        // ...and it is read off the clock rather than printed from a constant.
        let mut p = ready("announce me");
        p.arm_delay = Duration::from_secs(9);
        p.tick();
        assert_eq!(p.title_note().as_deref(), Some("sending in 9s"));
        assert!(screen(&mut p, 60, 8).contains("sending in 9s"));

        // Nothing queued and nothing to say.
        let mut empty = pane();
        assert_eq!(empty.title_note(), None);
        assert!(!empty.tick());
    }

    #[test]
    fn no_countdown_is_announced_when_there_is_nothing_to_send() {
        // Only a dispatch item is queued, which needs no announcement — and a
        // title promising a send that will never come is worse than silence.
        let mut p = pane();
        p.stub_item("in the background", Mode::Dispatch);
        p.armed = true;
        p.readiness = Readiness::Idle;

        assert!(!p.tick());
        assert_eq!(p.countdown(), None);
        assert_eq!(p.title_note().as_deref(), Some("queue 1"));
    }

    // --- dispatching ------------------------------------------------------

    #[test]
    fn a_dispatch_asks_nothing_of_the_agent_and_takes_only_its_own_items() {
        let mut p = pane();
        p.stub_item("typed at the agent", Mode::Send);
        p.stub_item("started beside it", Mode::Dispatch);
        // Every condition a send needs is false, and none of them is a
        // dispatch's business: nothing is being typed at.
        p.armed = false;
        p.readiness = Readiness::Busy;
        p.draft_open = true;

        assert_eq!(
            p.take_dispatch_request().as_deref(),
            Some("started beside it")
        );
        assert_eq!(p.take_dispatch_request(), None);
        assert_eq!(
            p.items[0].state,
            ItemState::Pending,
            "the send item was taken by the wrong drain"
        );
        assert_eq!(p.take_send_request(), None);
    }

    #[test]
    fn note_dispatched_lands_on_the_item_waiting_for_it_and_a_failure_says_why() {
        let mut p = pane();
        p.stub_item("first", Mode::Dispatch);
        p.stub_item("second", Mode::Dispatch);

        p.take_dispatch_request();
        p.note_dispatched(Ok(Started {
            session_id: Some("s-1".into()),
            id: Some("a1".into()),
            raw: String::new(),
        }));
        assert_eq!(
            p.items[0].state,
            ItemState::Dispatched {
                id: Some("a1".into())
            }
        );

        p.take_dispatch_request();
        p.note_dispatched(Err(anyhow::anyhow!("claude exited with 1")));
        assert_eq!(
            p.items[1].state,
            ItemState::Failed("claude exited with 1".into())
        );

        // An outcome with nothing left waiting for it changes nothing.
        p.note_dispatched(Ok(Started {
            session_id: None,
            id: Some("a9".into()),
            raw: String::new(),
        }));
        assert_eq!(
            p.items[0].state,
            ItemState::Dispatched {
                id: Some("a1".into())
            }
        );
    }

    #[test]
    fn a_dispatched_item_shows_the_live_state_of_the_agent_it_started() {
        let session = |id: &str, state: &str| Session {
            pid: None,
            id: Some(id.to_string()),
            session_id: None,
            cwd: None,
            kind: Kind::Background,
            status: None,
            state: Some(state.to_string()),
            started_at: None,
        };

        let mut p = pane();
        p.stub_item("the background one", Mode::Dispatch);
        p.take_dispatch_request();
        p.note_dispatched(Ok(Started {
            session_id: None,
            id: Some("a1".into()),
            raw: String::new(),
        }));

        assert!(p.set_roster(vec![session("a1", "working")]));
        assert!(screen(&mut p, 60, 8).contains("working"));

        // A frame is the agent's whole screen re-rendered. The same answer is
        // not worth one...
        assert!(!p.set_roster(vec![session("a1", "working")]));
        // ...nor is a stranger's session changing under a pane showing none of
        // it...
        assert!(!p.set_roster(vec![session("a1", "working"), session("a2", "failed")]));
        // ...and a real change is.
        assert!(p.set_roster(vec![session("a1", "blocked")]));
        assert!(screen(&mut p, 60, 8).contains("blocked"));
    }

    #[test]
    fn what_a_dispatched_agent_will_do_is_said_where_a_full_list_cannot_hide_it() {
        // The empty-state prose says it too, and vanishes with the first item.
        // The asymmetry is the point: the send path announces itself for three
        // seconds in the left title, and this one writes to the repository with
        // nobody watching.
        let mut p = pane();
        p.stub_item("something", Mode::Send);
        assert!(!screen(&mut p, 60, 8).contains("edits files"));

        // One keystroke moves an item between those two postures.
        assert_eq!(p.handle_key(key(KeyCode::Char('m'))).unwrap(), Handled::Yes);
        assert!(screen(&mut p, 60, 8).contains(DISPATCH_WARNING));

        // Whole, beside every answer the readiness line can give — the warning
        // that clips in half is the one nobody reads.
        for readiness in [Readiness::Idle, Readiness::Busy, Readiness::Unknown] {
            p.readiness = readiness;
            for armed in [false, true] {
                p.armed = armed;
                let text = screen(&mut p, 60, 8);
                assert!(text.contains(DISPATCH_WARNING), "{readiness:?}: {text}");
            }
        }

        // ...and it stops being said once nothing is queued to do it.
        p.items[0].state = ItemState::Dispatched { id: None };
        assert!(!screen(&mut p, 60, 8).contains("edits files"));
    }

    // --- keys -------------------------------------------------------------

    #[test]
    fn enter_skips_the_countdown_and_still_refuses_while_the_agent_is_busy() {
        let mut p = ready("do it now");
        p.readiness = Readiness::Busy;
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert_eq!(p.take_send_request(), None);

        p.readiness = Readiness::Idle;
        p.draft_open = true;
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert_eq!(p.take_send_request(), None);

        // With both of them true it goes at once — no three seconds of warning,
        // because the warning is for a send nobody asked for and this one was
        // asked for by hand.
        p.draft_open = false;
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(p.take_send_request().as_deref(), Some("do it now"));
    }

    #[test]
    fn enter_on_a_disarmed_queue_sends_that_item_and_leaves_the_queue_disarmed() {
        // The hazard that decided this. With `Enter` refusing while disarmed
        // there was no way to drain one item by hand, so "just this one, now"
        // meant pressing `a` first — and walking away with the unattended
        // sender on, which then drained the rest of the list unasked.
        let mut p = pane();
        p.stub_item("not this one", Mode::Send);
        p.stub_item("this one", Mode::Send);
        p.readiness = Readiness::Idle;
        p.selected = 1;
        assert!(!p.armed);

        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(p.take_send_request().as_deref(), Some("this one"));
        assert!(!p.armed, "sending one item must not arm the queue");

        // And the rest of the list stays exactly where it was, however long the
        // shell goes on asking.
        for _ in 0..50 {
            assert_eq!(p.take_send_request(), None);
        }
        assert_eq!(p.pending(), 1);
        assert_eq!(p.title_note().as_deref(), Some("queue 1"));
    }

    #[test]
    fn enter_sends_the_item_that_is_selected_rather_than_the_one_at_the_top() {
        let mut p = ready("the first thing");
        p.stub_item("the urgent thing", Mode::Send);
        p.selected = 1;

        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(p.take_send_request().as_deref(), Some("the urgent thing"));
        assert_eq!(p.pending(), 1);
        assert_eq!(p.items[1].text, "the first thing");

        // The one it jumped waits for an announcement of its own rather than
        // inheriting the ask, and it is a whole one: the item that was promoted
        // over it had its countdown withdrawn, not handed on.
        assert_eq!(p.countdown(), None, "nothing is due until it is announced");
        assert!(p.tick());
        assert_eq!(p.countdown(), Some(3));
        assert_eq!(p.take_send_request(), None);
    }

    #[test]
    fn the_composer_reads_q_and_j_and_g_as_text_and_esc_closes_it_without_leaving_the_pane() {
        let mut p = pane();
        assert_eq!(p.handle_key(key(KeyCode::Char('i'))).unwrap(), Handled::Yes);

        // Space included: it pages the list, and it is a space in here.
        for c in "qjgar dm".chars() {
            assert_eq!(
                p.handle_key(key(KeyCode::Char(c))).unwrap(),
                Handled::Yes,
                "{c:?} was read as a command instead of as text"
            );
        }
        assert_eq!(p.composing.as_deref(), Some("qjgar dm"));
        assert!(p.items.is_empty(), "not one of those was a command");
        assert!(!p.armed, "the `a` was a letter, not the arming key");

        // Backspace edits rather than closing: the next keystroke in the list
        // would be `d`, and `d` deletes an item.
        p.handle_key(key(KeyCode::Backspace)).unwrap();
        assert_eq!(p.composing.as_deref(), Some("qjgar d"));

        assert_eq!(p.handle_key(key(KeyCode::Esc)).unwrap(), Handled::Yes);
        assert!(p.composing.is_none(), "esc closed the composer");
        assert!(p.items.is_empty(), "and threw the draft away");
    }

    #[test]
    fn the_composer_stays_open_for_the_next_item_and_an_empty_enter_ends_the_run() {
        let mut p = pane();
        p.handle_key(key(KeyCode::Char('i'))).unwrap();
        for c in "one".chars() {
            p.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        p.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].text, "one");
        assert_eq!(p.composing.as_deref(), Some(""), "still taking the next one");

        for c in "two".chars() {
            p.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        // Ctrl+Enter is a newline in the item rather than a commit, where the
        // terminal reports the modifier at all.
        p.handle_key(ctrl(KeyCode::Enter)).unwrap();
        for c in "and a half".chars() {
            p.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        p.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(p.items[1].text, "two\nand a half");

        p.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(p.composing.is_none(), "an empty enter ends the run");
        assert_eq!(p.items.len(), 2);

        // A new item is a `Send` one: it waits for the queue to be armed, where
        // a `Dispatch` one would have started a process by itself.
        assert!(p.items.iter().all(|i| i.mode == Mode::Send));
    }

    #[test]
    fn esc_and_q_fall_through_to_the_shell_from_the_list() {
        let mut p = pane();
        p.stub_item("something", Mode::Send);
        assert_eq!(p.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
        assert_eq!(p.handle_key(key(KeyCode::Char('q'))).unwrap(), Handled::No);
        // Ctrl+letter is the agent's everywhere else in the program, and must
        // not fall into the plain-letter commands.
        assert_eq!(p.handle_key(ctrl(KeyCode::Char('i'))).unwrap(), Handled::No);
        assert_eq!(p.handle_key(ctrl(KeyCode::Char('a'))).unwrap(), Handled::No);
        assert!(p.composing.is_none());
        assert!(!p.armed);
    }

    #[test]
    fn a_arms_and_disarms_and_m_switches_an_item_between_the_modes() {
        let mut p = pane();
        p.stub_item("a thing", Mode::Send);
        p.readiness = Readiness::Idle;

        assert_eq!(p.handle_key(key(KeyCode::Char('a'))).unwrap(), Handled::Yes);
        assert!(p.armed);
        assert!(p.countdown().is_some(), "arming announces what it enables");

        assert_eq!(p.handle_key(key(KeyCode::Char('m'))).unwrap(), Handled::Yes);
        assert_eq!(p.items[0].mode, Mode::Dispatch);
        assert_eq!(p.countdown(), None, "nothing left to type at the agent");

        assert_eq!(p.handle_key(key(KeyCode::Char('a'))).unwrap(), Handled::Yes);
        assert!(!p.armed);

        // Not on an item that has already gone: relabelling it would be the
        // pane lying about its own history.
        p.items[0].state = ItemState::Sent;
        assert_eq!(p.handle_key(key(KeyCode::Char('m'))).unwrap(), Handled::No);
        assert_eq!(p.items[0].mode, Mode::Dispatch);
    }

    #[test]
    fn deleting_and_clearing_leave_the_selection_somewhere_that_exists() {
        let mut p = pane();
        p.stub_item("one", Mode::Send);
        p.stub_item("two", Mode::Send);
        p.stub_item("three", Mode::Send);
        p.items[0].state = ItemState::Sent;

        p.selected = 2;
        // Twice for each, and the first press of each pair is asserted to have
        // changed nothing: `d` and `r` throw work away, and see [`Confirm`] for
        // why they are now one press further from a mistyped shell command.
        assert_eq!(p.handle_key(key(KeyCode::Char('d'))).unwrap(), Handled::Yes);
        assert_eq!(p.items.len(), 3, "the first press acted instead of asking");
        assert_eq!(p.handle_key(key(KeyCode::Char('d'))).unwrap(), Handled::Yes);
        assert_eq!(p.selected, 1);
        assert_eq!(p.items.len(), 2);

        assert_eq!(p.handle_key(key(KeyCode::Char('r'))).unwrap(), Handled::Yes);
        assert_eq!(p.items.len(), 2, "the first press acted instead of asking");
        assert_eq!(p.handle_key(key(KeyCode::Char('r'))).unwrap(), Handled::Yes);
        assert_eq!(p.items.len(), 1, "the sent one was cleared");
        assert_eq!(p.selected, 0);
        // Nothing finished to clear is not a frame — and not a question either,
        // because the emptiness check comes before the arming.
        assert_eq!(p.handle_key(key(KeyCode::Char('r'))).unwrap(), Handled::No);

        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert!(p.items.is_empty());
        assert_eq!(p.handle_key(key(KeyCode::Char('d'))).unwrap(), Handled::No);
        assert_eq!(p.handle_key(key(KeyCode::Tab)).unwrap(), Handled::No);
    }

    #[test]
    fn a_destructive_key_asks_first_and_any_other_key_is_the_answer_no() {
        // The half of the guard that is not "press it twice". A confirmation
        // only *some* keys cancelled would be one a typist walks straight
        // through, because the keys between two `d`s in a command are exactly
        // the keys nobody thought about.
        let mut p = pane();
        p.stub_item("one", Mode::Send);

        assert_eq!(p.handle_key(key(KeyCode::Char('d'))).unwrap(), Handled::Yes);
        assert_eq!(
            p.confirm,
            Some(Confirm::Delete("one".to_string())),
            "the first press did not ask, or asked about the wrong row"
        );
        assert_eq!(p.items.len(), 1);

        // `Tab` is one of this pane's own keys, which is the strongest version
        // of "any": if the vocabulary the user is deliberately using did not
        // cancel, nothing would. With one item it selects nothing new, so the
        // `Yes` is the wrapper buying a frame for the line it has just cleared
        // — which is the other half of what this asserts.
        assert_eq!(p.handle_key(key(KeyCode::Tab)).unwrap(), Handled::Yes);
        assert_eq!(p.confirm, None, "a key went past the question");
        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert_eq!(p.items.len(), 1, "the pair was broken and it deleted anyway");

        // ...and the second of an unbroken pair still does the thing, or this
        // would be a test that the key is dead.
        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert!(p.items.is_empty());
    }

    #[test]
    fn a_command_typed_at_the_queue_by_mistake_takes_no_work_with_it() {
        // The report the guard exists for. `F8` to glance at what is queued
        // used to hand the keys back to the agent; it leaves them here now, so
        // a command meant for the shell arrives as this pane's commands. Every
        // letter of this one is offered to the pane, and it carries a `d` and
        // two `r`s.
        let mut p = pane();
        p.stub_item("do not lose me", Mode::Send);
        p.stub_item("or me", Mode::Send);
        p.items[1].state = ItemState::Sent;

        for c in "cargo doc --release".chars() {
            p.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        assert_eq!(
            texts(&p),
            ["do not lose me", "or me"],
            "a mistyped command emptied the queue"
        );

        // What it *did* do is worth being exact about rather than leaving to be
        // assumed. `a` is a toggle, and this command has two of them, so the
        // switch is back where it started — with a three second countdown
        // announcing itself in the left title had it not been.
        assert!(!p.armed);

        // **The trailing `Enter` is deliberately not typed here, and that is a
        // limit of this test rather than of the command.** A command is not
        // mistyped at a shell until it is submitted, and `Enter` is unguarded:
        // it grants the selected item a `Due::Asked`, the one send that skips
        // the announced countdown. [`Confirm`] says why that key was left alone
        // and what the honest fix would be. Two `d`s in a row are the other
        // thing this does not cover — `add` and `ladder` get through.
    }

    #[test]
    fn the_question_leads_the_foot_line_where_it_cannot_be_clipped_away() {
        // The foot line is clipped from the right and already carries four
        // other things — armed, the agent's state, the countdown, the dispatch
        // warning — so a question appended to it is a question nobody is asked.
        // Narrow on purpose: at this width there is room for the question or
        // for the state behind it, and the question is the one that has to
        // survive.
        let mut p = pane();
        p.stub_item("one", Mode::Send);
        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        let asked = screen(&mut p, 30, 8);
        assert!(asked.contains("d again to delete"), "{asked}");

        p.handle_key(key(KeyCode::Tab)).unwrap();
        let quiet = screen(&mut p, 30, 8);
        assert!(
            !quiet.contains("d again"),
            "the question outlived the press that answered it:\n{quiet}"
        );
    }

    #[test]
    fn a_click_between_the_two_presses_is_the_answer_no() {
        // The question names a row, so the press that answers it has to act on
        // that row or the warning was about something else. A click moves the
        // selection and does not arrive as a key, which is how `d`, click, `d`
        // came to delete an item nothing had asked about — one warning, shown
        // about the row that survived.
        let mut p = pane();
        p.stub_item("keep me", Mode::Send);
        p.stub_item("and me", Mode::Send);
        screen(&mut p, 40, 12);

        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert_eq!(p.confirm, Some(Confirm::Delete("keep me".to_string())));

        // The second row of the list, which is a different item from the one
        // the question named.
        p.handle_mouse(&click(3, 2)).unwrap();
        assert_eq!(p.confirm, None, "a click walked past the question");
        assert_eq!(p.items[p.selected].text, "and me");

        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert_eq!(texts(&p), ["keep me", "and me"], "the click deleted a row");
    }

    #[test]
    fn a_paste_between_the_two_presses_does_not_destroy_what_it_pasted() {
        // Sharper than an ordinary cancel, because a paste appends an item
        // *and selects it*: before the question carried the row it was asked
        // about, `d`, paste, `d` threw away the text that had just arrived and
        // left the row the warning named sitting there.
        let mut p = pane();
        p.stub_item("the one it asked about", Mode::Send);

        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        p.handle_paste("the one it did not").unwrap();
        assert_eq!(p.confirm, None, "a paste walked past the question");

        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert_eq!(
            texts(&p),
            ["the one it asked about", "the one it did not"],
            "the paste was destroyed by the press before it"
        );
    }

    #[test]
    fn a_question_does_not_survive_the_pane_leaving_the_screen() {
        // The shell's half, and the whole of the gap between this guard and
        // `Alt+Q`'s: a pane is not told it has been put away, so without the
        // call this test stands in for, `d` could be asked on one screen and
        // answered on another an hour later — a deletion on what the user
        // experienced as a single press. `crate::app::App::set_right_view` is
        // the caller, and that wiring is pinned in `app.rs`.
        let mut p = pane();
        p.stub_item("one", Mode::Send);
        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert!(p.confirm.is_some());

        p.cancel_confirm();
        p.handle_key(key(KeyCode::Char('d'))).unwrap();
        assert_eq!(texts(&p), ["one"], "the question outlived the screen");
    }


    #[test]
    fn a_paste_becomes_one_item_in_the_list_and_more_text_in_the_composer() {
        let mut p = pane();
        assert_eq!(
            p.handle_paste("rewrite the parser\r\nand test it").unwrap(),
            Handled::Yes
        );
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].text, "rewrite the parser\nand test it");

        p.handle_key(key(KeyCode::Char('i'))).unwrap();
        p.handle_paste("more").unwrap();
        assert_eq!(p.composing.as_deref(), Some("more"));
        assert_eq!(p.handle_paste("").unwrap(), Handled::No);
    }

    #[test]
    fn the_border_never_promises_esc_will_reach_the_agent_while_the_composer_is_open() {
        let mut p = pane();
        assert_eq!(p.exit_hint(), "esc→agent");
        assert!(!p.takes_input());
        assert_eq!(p.cursor(), None);

        p.handle_key(key(KeyCode::Char('i'))).unwrap();
        assert_ne!(p.exit_hint(), "esc→agent");
        assert!(p.exit_hint().contains("esc"), "{}", p.exit_hint());
        assert!(p.takes_input(), "the open composer is what plain keys go into");

        // And there is a cursor to look at, inside the pane it was drawn in.
        screen(&mut p, 40, 6);
        let (col, row) = p.cursor().expect("a composer has a cursor");
        assert!(col < 40 && row < 6, "cursor at {col},{row}");
    }

    // --- what a frame costs -----------------------------------------------

    #[test]
    fn a_running_countdown_asks_for_a_frame_only_when_the_second_it_shows_changes() {
        // `tick` runs once per pass of the loop, and a frame re-renders the
        // agent's entire screen. Returning true from here on every pass would
        // do that at the frame ceiling to redraw a digit that has not changed.
        let mut p = ready("count me down");
        let mut asked = 0;
        for _ in 0..20_000 {
            if p.tick() {
                asked += 1;
            }
        }
        assert!(
            asked <= 2,
            "a live countdown asked for {asked} frames without the clock moving"
        );
        assert_eq!(p.countdown(), Some(3), "and it is still running");
    }

    #[test]
    fn being_told_the_same_thing_twice_is_never_worth_a_frame() {
        let mut p = ready("quiet please");
        assert!(!p.set_readiness(Readiness::Idle));
        assert!(p.set_readiness(Readiness::Busy));
        assert!(!p.set_readiness(Readiness::Busy));
        assert!(!p.set_draft_open(false));
        assert!(p.set_draft_open(true));
        assert!(!p.set_draft_open(true));
        assert!(p.is_draft_open());
        assert!(!p.set_roster(Vec::new()));
    }

    // --- drawing ----------------------------------------------------------

    #[test]
    fn an_empty_pane_explains_itself_and_an_unavailable_dispatcher_says_why() {
        // A blank box is indistinguishable from a broken one, and this pane is
        // empty the first time anybody opens it.
        let mut p = unavailable("copilot has no background agents");
        let text = screen(&mut p, 60, 14);
        assert!(text.contains("Nothing queued"), "{text}");
        assert!(text.contains("a arms"), "the key that arms it is named");
        // One word at a time: the prose is wrapped, so a phrase can be split
        // across two rows of the flattened frame while reading perfectly well.
        assert!(text.contains("asking"), "and what ⇉ does");

        // The reason, where somebody looking for the missing half will read it:
        // above the list, not below whatever has scrolled past.
        assert!(text.contains("dispatch unavailable"), "{text}");
        assert!(text.contains("copilot has no background agents"), "{text}");

        // ...and it stays there once there is a list to push it off.
        p.stub_item("something", Mode::Dispatch);
        let text = screen(&mut p, 60, 14);
        assert!(text.contains("copilot has no background agents"), "{text}");

        // Both of them draw at every size a split can produce, including the
        // ones where there is no room for either.
        let mut p = unavailable("no claude on PATH");
        for w in [1u16, 2, 8, 24, 46] {
            for h in [1u16, 2, 3, 30] {
                screen(&mut p, w, h);
            }
        }
    }

    #[test]
    fn the_pane_says_what_it_is_doing_whether_or_not_it_is_armed() {
        let mut p = ready("visible state");
        assert!(screen(&mut p, 60, 8).contains("armed"));
        assert!(screen(&mut p, 60, 8).contains("idle"));

        p.armed = false;
        p.readiness = Readiness::Unknown;
        let text = screen(&mut p, 60, 8);
        assert!(text.contains("disarmed"), "{text}");
        assert!(text.contains("state unknown"), "{text}");
        assert!(text.contains("a arms"), "{text}");
    }

    #[test]
    fn a_long_or_wide_item_never_spills_out_of_the_pane_it_is_drawn_in() {
        // A terminal measures in cells. `str::len` is wrong about a CJK
        // ideograph twice over and about an emoji as well, and a row that
        // overflows its rect corrupts the frame rather than merely looking
        // wrong.
        let mut p = pane();
        p.stub_item("設計文書を全部読んでから直してください", Mode::Send);
        p.stub_item("🎉 ship it 🎉", Mode::Dispatch);
        p.stub_item(&"x".repeat(400), Mode::Send);
        p.stub_item("first line\nsecond line\nthird", Mode::Send);
        p.items[1].state = ItemState::Dispatched {
            id: Some("a1".into()),
        };
        p.items[2].state = ItemState::Failed("something went badly wrong indeed".into());

        for w in [1usize, 2, 3, 7, 12, 24, 46, 120] {
            for i in 0..p.items.len() {
                for selected in [false, true] {
                    let line = p.item_line(i, w, selected);
                    let used: usize = line.spans.iter().map(|s| s.content.width()).sum();
                    assert!(used <= w, "item {i} is {used} cells wide in {w} columns");
                }
            }
            let used: usize = p.foot_line(w).spans.iter().map(|s| s.content.width()).sum();
            assert!(used <= w, "the status line is {used} cells wide in {w}");

            // And the whole pane draws, at every height a split can produce.
            for h in [1u16, 2, 3, 20] {
                screen(&mut p, w as u16, h);
            }
        }
    }

    #[test]
    fn a_long_draft_scrolls_under_the_cursor_rather_than_out_of_the_pane() {
        let mut p = pane();
        p.handle_key(key(KeyCode::Char('i'))).unwrap();
        for c in "a very long prompt indeed, far wider than this pane".chars() {
            p.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        for w in [4usize, 12, 20, 46] {
            let used: usize = p.foot_line(w).spans.iter().map(|s| s.content.width()).sum();
            assert!(used <= w, "the composer is {used} cells wide in {w}");
            let (_, col) = p.composer_view(w);
            assert!(col < w, "the cursor sits at {col} in {w} columns");
        }
        // The end of the draft is what is on screen, because that is where the
        // typing is happening.
        let (shown, _) = p.composer_view(24);
        assert!(shown.ends_with("this pane"), "{shown}");
    }

    #[test]
    fn the_title_leads_with_what_survives_a_clip() {
        let mut p = pane();
        assert_eq!(p.title(), "queue · empty");

        p.stub_item("one", Mode::Send);
        p.stub_item("two", Mode::Dispatch);
        assert_eq!(p.title(), "queue · 2 pending");

        p.armed = true;
        assert_eq!(p.title(), "queue · 2 pending · armed");

        p.items[1].state = ItemState::Failed("no".into());
        assert_eq!(p.title(), "queue · 1 pending · armed · 1 failed");

        p.items[0].state = ItemState::Sent;
        assert_eq!(p.title(), "queue · clear · armed · 1 failed");

        p.handle_key(key(KeyCode::Char('i'))).unwrap();
        assert!(p.title().starts_with("queue · new item"));

        // Every one of them fits the pane it is drawn in, so the leading words
        // are a courtesy rather than the only thing anybody ever reads.
        let mut p = pane();
        let mut titles = vec![p.title()];
        p.stub_item("one", Mode::Send);
        p.armed = true;
        p.items[0].state = ItemState::Failed("no".into());
        titles.push(p.title());
        p.handle_key(key(KeyCode::Char('i'))).unwrap();
        titles.push(p.title());
        for t in titles {
            assert!(t.width() < 46, "a title that never survives a clip: {t}");
        }
    }

    #[test]
    fn the_list_scrolls_and_the_wheel_never_re_aims_what_enter_would_do() {
        let mut p = pane();
        for i in 0..40 {
            p.stub_item(&format!("item {i}"), Mode::Send);
        }
        screen(&mut p, 40, 12);

        // The scroll vocabulary of `crate::scroll`, which is one table for the
        // whole program — space and b included. Arming wanted space and did not
        // get it, because a key that pages in four panes and toggles a mode in
        // the fifth is a key nobody can learn.
        assert_eq!(p.handle_key(key(KeyCode::Char('j'))).unwrap(), Handled::Yes);
        assert_eq!(p.scroll.offset, 1);
        assert_eq!(p.handle_key(key(KeyCode::Char(' '))).unwrap(), Handled::Yes);
        // Ten rows of list under the notice and above the status line.
        assert_eq!(p.scroll.offset, 1 + 9, "a page keeps one row of overlap");
        assert!(!p.armed, "space pages here as it does everywhere else");
        assert_eq!(p.handle_key(key(KeyCode::Char('b'))).unwrap(), Handled::Yes);
        assert_eq!(p.scroll.offset, 1);
        assert_eq!(p.handle_key(key(KeyCode::Char('G'))).unwrap(), Handled::Yes);
        assert!(p.scroll.offset > 1);
        assert_eq!(p.handle_key(key(KeyCode::Char('g'))).unwrap(), Handled::Yes);
        assert_eq!(p.scroll.offset, 0);
        assert_eq!(
            p.selected, 0,
            "looking through the list must not re-choose the row Enter acts on"
        );

        // Tab is what chooses, and it wraps.
        p.handle_key(key(KeyCode::Tab)).unwrap();
        assert_eq!(p.selected, 1);
        p.handle_key(key(KeyCode::BackTab)).unwrap();
        p.handle_key(key(KeyCode::BackTab)).unwrap();
        assert_eq!(p.selected, 39);
    }

    #[test]
    fn a_click_chooses_the_row_it_landed_on_and_the_wheel_chooses_nothing() {
        // The one input that *does* re-aim what `Enter` acts on, so its
        // arithmetic has to agree with what `render` drew — including the row
        // the unavailable-dispatcher notice takes off the top.
        let mut p = pane();
        for i in 0..40 {
            p.stub_item(&format!("item {i}"), Mode::Send);
        }
        let (w, h) = (40u16, 12u16);
        let s = screen(&mut p, w, h);
        assert!(row_of(&s, w, 0).contains("dispatch unavailable"));
        assert!(row_of(&s, w, 1).contains("item 0"), "{}", row_of(&s, w, 1));

        // Third row of the pane, which is the second row of the list.
        assert_eq!(p.handle_mouse(&click(3, 2)).unwrap(), Handled::Yes);
        assert_eq!(p.items[p.selected].text, "item 1");
        let s = screen(&mut p, w, h);
        assert!(row_of(&s, w, 2).contains("item 1"), "{}", row_of(&s, w, 2));

        // The wheel moves the view and nothing else — a read from the other
        // side of the window may not re-choose anything.
        assert_eq!(p.handle_mouse(&wheel_down()).unwrap(), Handled::Yes);
        assert_eq!(p.scroll.offset, 3);
        assert_eq!(p.items[p.selected].text, "item 1");

        // ...and a click after it lands on the row that is now drawn there.
        let s = screen(&mut p, w, h);
        assert!(row_of(&s, w, 1).contains("item 3"), "{}", row_of(&s, w, 1));
        assert_eq!(p.handle_mouse(&click(3, 1)).unwrap(), Handled::Yes);
        assert_eq!(p.items[p.selected].text, "item 3");

        // The status line is not a row of the list, and neither is the notice.
        let before = p.selected;
        assert_eq!(p.handle_mouse(&click(3, h - 1)).unwrap(), Handled::No);
        assert_eq!(p.handle_mouse(&click(3, 0)).unwrap(), Handled::No);
        assert_eq!(p.selected, before);

        // Nor is the empty half of a short pane.
        let mut p = pane();
        p.stub_item("the only one", Mode::Send);
        screen(&mut p, w, h);
        assert_eq!(p.handle_mouse(&click(3, 5)).unwrap(), Handled::No);
        assert_eq!(p.selected, 0);
    }
}
