//! Asking a second Claude about the thing you are already looking at.
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
//! ## It reads, and the tool list is the proof rather than the promise
//!
//! `crate::ask` hands the child `--tools "Read,Grep,Glob"`, which is an
//! allowlist over the built-in set: what is not named there does not exist for
//! that session, so there is no `Write`, no `Edit` and no `Bash` to permit or
//! refuse. That is a claim about a *list*, and a claim about a list is worth
//! showing rather than asserting — so the tools that came back on
//! [`AskEvent::Ready`] are drawn along the bottom of the pane, and what is on
//! screen is what the child actually got. A pane that said "read-only" in its
//! own voice would be repeating abeam's intention back at the reader; this
//! repeats the child's answer.
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
//! child stands in the same directory and has `Read`, `Grep` and `Glob`, so
//! naming the file is enough: it fetches what it needs and skips what it does
//! not.
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

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::ask::AskEvent;
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

/// The one agent this is available for.
///
/// Written out rather than taken from [`crate::agent::DEFAULT`], for
/// `crate::dispatch`'s reason: the default is what abeam hosts when nobody
/// said, and this is whose streaming-JSON print mode the whole shape rests on.
/// The day the first of those changes must not be the day this pane starts
/// spawning something that has never heard of `--input-format stream-json`.
const AGENT: &str = "claude";

/// The cost nothing else on screen would mention.
///
/// Short because it shares a row with the tool list, and the tool list is the
/// half a reader is looking for. At twenty-three cells the pair fits the
/// forty-six columns a right pane is routinely given, where "shares your Claude
/// subscription with the agent in the left pane" clipped itself in half and
/// said nothing.
const QUOTA: &str = "same quota as the agent";

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
/// The pane resolves this once, while it is being built, and never runs it —
/// exactly as `QueuePane` holds a `Dispatcher` it never dispatches with. Two
/// things follow from resolving early. The pane can say *on the first frame*
/// that this session has nothing to ask, rather than looking ordinary until the
/// first question fails; and the thing that blocks stays off a type the shell
/// renders every frame.
#[derive(Debug)]
pub struct Ready {
    launch: Launch,
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
    /// Everything `Delta` has carried so far, replaced by `Turn`'s own text
    /// when one arrives with any.
    answer: String,
    cost_usd: Option<f64>,
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
    Note(String),
}


/// The pane.
pub struct AskPane {
    root: PathBuf,
    /// Whether there is anything to ask, decided once. See [`Ready`].
    ready: Result<Ready, Unavailable>,

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

    theme: theme::Mode,
    drawn: Rect,
}

impl AskPane {
    /// `agent` is the hosted agent's name, which decides whether there is
    /// anything to ask at all.
    pub fn new(root: PathBuf, agent: &str) -> Self {
        let launch = resolve(agent);
        Self::with_launch(root, launch)
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
    pub fn with_launch(root: PathBuf, launch: Result<Launch, Unavailable>) -> Self {
        Self {
            root,
            ready: launch.map(|launch| Ready { launch }),
            entries: Vec::new(),
            composing: String::new(),
            context: None,
            tools: None,
            ended: false,
            pending: None,
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
        self.ready.as_ref().ok().map(|ready| &ready.launch)
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
    /// Compared by label and not by identity, because the app has every reason
    /// to say this more than once for the same thing — `?` pressed twice, a
    /// pane re-offering what it is showing — and re-attaching what is already
    /// attached would be a frame spent redrawing an identical row. A frame here
    /// re-renders the agent's whole screen.
    pub fn attach(&mut self, ctx: Option<AskContext>) {
        let now = ctx.as_ref().map(|c| c.label.as_str());
        let was = self.context.as_ref().map(|c| c.label.as_str());
        if now == was {
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
                            x.answer.push_str(&text);
                        }
                    }
                    // A fragment with no question above it is not something to
                    // throw away: it is either a bug in the reader or a turn
                    // abeam did not know it had started, and both are worth
                    // seeing.
                    None => self.entries.push(Entry::Note(format!(
                        "an answer arrived with no question above it: {}",
                        clip(&one_line(&text), 200)
                    ))),
                }
                self.bump();
            }
            AskEvent::Turn {
                text,
                cost_usd,
                error,
            } => {
                match self.open_exchange() {
                    Some(i) => {
                        if let Entry::Exchange(x) = &mut self.entries[i] {
                            // The `result` line carries the whole answer and
                            // the deltas carry it in pieces, so the two should
                            // agree — and when they do not, the complete one
                            // wins. An empty `result` does *not*: a turn that
                            // streamed four paragraphs and then reported no
                            // text is a turn whose text abeam already has, and
                            // blanking it would throw away the only copy.
                            if !text.trim().is_empty() {
                                x.answer = text;
                            }
                            x.cost_usd = cost_usd;
                            x.error = error;
                            x.done = true;
                        }
                    }
                    None => self.entries.push(Entry::Note(format!(
                        "a turn ended with no question above it: {}",
                        clip(&one_line(&text), 200)
                    ))),
                }
                self.bump();
            }
            AskEvent::RateLimited(why) => {
                // Worth its own entry rather than a colour on the title. It is
                // the one thing that happens to this pane *because* it shares
                // the agent's quota, which is the sentence along the bottom
                // coming true.
                self.note(format!(
                    "rate limited: {why} This session and the agent in the left \
                     pane draw on the same account, so waiting is the whole of \
                     what there is to do."
                ));
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
        self.entries.push(Entry::Note(text));
        self.bump();
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

    fn streaming(&self) -> bool {
        matches!(self.entries.last(), Some(Entry::Exchange(x)) if !x.done)
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
                Entry::Note(_) => None,
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
    fn submit(&mut self) -> Handled {
        let question = self.composing.trim().to_string();
        if question.is_empty() {
            return Handled::No;
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
                    if let Some(cost) = x.cost_usd {
                        // Per turn and not only in total, because the total in
                        // the title cannot say which question was the expensive
                        // one.
                        out.push_str(&format!("`${cost:.4}`\n\n"));
                    }
                }
                Entry::Note(text) => {
                    out.push_str("> [!WARNING]\n> ");
                    out.push_str(&one_line(text));
                    out.push_str("\n\n");
                }
            }
        }
        out
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
            "Ask a second Claude about what is in front of you. It can read this \
             repository — `Read`, `Grep` and `Glob`, and the row along the bottom \
             is the list it actually got — and it has no tool that can change a \
             file.\n\
             \n\
             It is the **same account** as the agent in the left pane, so every \
             answer here comes out of the same quota and costs the same money.\n\
             \n\
             - `enter` sends what you have typed; `ctrl+enter` starts a new line \
             inside it.\n\
             - `tab` picks a command out of an answer, and `enter` on an empty \
             box types it into the shell **without running it**. A block of more \
             than one line is never offered, because a command joined into one \
             line is a command nobody read.\n\
             - `esc` clears what you have typed, and hands focus back once there \
             is nothing left to clear.\n\
             - `↑ ↓ pgup pgdn home end` move the transcript. The letters are \
             letters in here, because this is a box you type into.\n\
             \n\
             Nothing is remembered between sessions, and closing abeam ends the \
             conversation.\n\
             \n\
             It reads `{}`\n",
            self.root.display()
        )
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
        match &self.ready {
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
        u16::from(self.ready.is_err())
    }

    /// Everything drawn below the transcript, top to bottom, ending with the
    /// composer.
    ///
    /// One function so that the height the body is given and the height the
    /// foot is drawn at are one number. The composer is always the last row,
    /// which is what lets [`Pane::cursor`] answer from the pane's height alone
    /// rather than from a second copy of this arithmetic.
    fn foot(&self, w: usize) -> Vec<Line<'static>> {
        if self.ready.is_err() {
            // Nothing to type into and nothing to offer. The notice and the
            // reason are the whole pane.
            return Vec::new();
        }
        let mut out = vec![self.capability_line(w)];
        out.extend(self.command_lines(w));
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
            // Not "read-only". Nothing has told this pane anything yet, and a
            // list printed before the child has reported one would be abeam's
            // intention wearing the child's clothes.
            _ => spans.push(Span::styled("no reader yet", t.dim())),
        }
        spans.push(Span::styled(" · ", t.dim()));
        spans.push(Span::styled(QUOTA, t.dim()));
        clip_line(Line::from(spans), w)
    }

    /// The command `Enter` would type into the shell, or the reason there is
    /// not one.
    ///
    /// Nothing at all when the transcript has no code in it, because a row that
    /// is empty most of the time teaches a reader to stop looking at it.
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
        if self.ready.is_err() {
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
        if let Err(Unavailable(why)) = &self.ready {
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
    /// app told this pane, changed what the next frame would show.
    ///
    /// Never on a bare pass of the loop. A frame re-renders the agent's whole
    /// screen, and a pane that claims one every time it is asked would do that
    /// at the frame ceiling for a transcript nobody is adding to.
    fn tick(&mut self) -> bool {
        std::mem::take(&mut self.owed)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        // Nothing to type into, so nothing is claimed — including `Esc` and
        // `q`, which the shell then reads as "the user is done with this pane".
        // That is the only way out of an ask that cannot ask anything.
        if self.ready.is_err() {
            return Ok(Handled::No);
        }
        self.sync();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

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
            // question.
            KeyCode::Char(c) if !ctrl && !alt => {
                self.composing.push(c);
                Handled::Yes
            }
            _ => Handled::No,
        })
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        if self.ready.is_err() {
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
        if self.ready.is_err() {
            return Ok(Handled::No);
        }
        Ok(self.scroll_only(key).unwrap_or(Handled::No))
    }

    /// True whenever there is a composer, which is whenever the pane is
    /// available.
    ///
    /// A question about this instant, per the trait docs, and here the instant
    /// that matters is the one where there is nothing to ask: an unavailable
    /// ask takes nothing, so leaving it hands focus back to the agent and a
    /// paste has nowhere to go — both of which are true of it and of no other
    /// state this pane has.
    fn takes_input(&self) -> bool {
        self.ready.is_ok()
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
        if self.ready.is_ok() && !self.composing.is_empty() {
            " · esc→clear"
        } else {
            " · esc→agent"
        }
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        if self.ready.is_err() || self.drawn.width == 0 || self.drawn.height == 0 {
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
        if self.ready.is_err() {
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
fn resolve(agent: &str) -> Result<Launch, Unavailable> {
    // Through the table rather than by comparing the string, so `abeam +Claude`
    // and `abeam +claude` are one request here as they are everywhere else. A
    // program named outright — `abeam +C:\tools\claude.exe` — is not the
    // table's Claude and does not become it.
    let Some(claude) = crate::agent::find(agent).filter(|found| found.name == AGENT) else {
        return Err(Unavailable(elsewhere(agent)));
    };
    let mut why = String::new();
    for candidate in claude.candidates {
        match launch::resolve(candidate, &[]) {
            Ok(launch) => return Ok(launch),
            Err(reason) => why = reason,
        }
    }
    // And that is the end of the search. Nothing on this path fetches anything;
    // `crate::agent`'s module docs record the route that was written for exactly
    // this problem and then deliberately taken out again.
    Err(Unavailable(missing(claude, &why)))
}

/// What abeam says to a session that is hosting something else.
///
/// It names the agent in front of the reader, because "ask is unavailable" with
/// no subject reads as a bug in abeam rather than as a fact about the agent
/// being hosted. And it names what is still there, because losing this pane is
/// not losing the ability to ask a question — it is losing the ability to ask
/// one *without spending the conversation on the left*.
fn elsewhere(agent: &str) -> String {
    format!(
        "abeam is hosting `{agent}`, and this pane is a second Claude in \
         streaming-JSON print mode: it needs `--input-format stream-json`, \
         `--tools` and `--session-id`, which are Claude's. `{agent}` publishes \
         no equivalent, and abeam will not quietly start a Claude you did not \
         ask for — it hosts the agent you named. The question can still be \
         asked of the session in the left pane, which is the agent you chose."
    )
}

/// What abeam says when the agent it is hosting is not on the machine.
///
/// The same standard `crate::agent::missing` is held to — every candidate by
/// name, the operating system's own reason, and a sentence somebody can act on
/// — plus the observation that makes this odd rather than ordinary: reaching
/// here means abeam is *hosting* Claude, so there was one when the session
/// started and something has moved it since.
fn missing(claude: &crate::agent::Agent, why: &str) -> String {
    let tried: Vec<String> = claude
        .candidates
        .iter()
        .map(|name| format!("`{name}`"))
        .collect();
    format!(
        "abeam has nothing to ask: it looked for the Claude it is hosting and \
         did not find one. Tried: {}. {why}\n\nThat is odd rather than ordinary \
         — this session started a Claude, so there was one when it began. {}",
        tried.join(", "),
        claude.install
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
                match lines.len() {
                    0 => {}
                    1 => out.push(lines[0].trim().to_string()),
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
fn refusal(skipped: usize) -> String {
    format!(
        "{COMMAND}{skipped} {} several lines long. Joining one into a single \
         command would put something at your prompt that nobody read, so none \
         is offered — copy it out of the answer above.",
        plural(skipped, "block is", "blocks are")
    )
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
        AskPane::with_launch(PathBuf::from("/repo"), Ok(fake_launch()))
    }

    fn unavailable(why: &str) -> AskPane {
        AskPane::with_launch(PathBuf::from("/repo"), Err(Unavailable(why.to_string())))
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
        assert!(!p.takes_input(), "a paste has nowhere to go");
        assert_eq!(p.cursor(), None, "and there is nothing to type into");
        assert_eq!(p.exit_hint(), " · esc→agent");
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
    fn only_the_agent_with_the_flags_can_be_asked_and_the_rest_are_told_why() {
        for hosting in ["copilot", "Copilot"] {
            let Unavailable(why) = resolve(hosting).expect_err("this shape is Claude's");
            assert!(why.contains(hosting), "the agent in front of them: {why}");
            assert!(why.contains("stream-json"), "which flags: {why}");
            assert!(why.contains("Claude"), "whose they are: {why}");
            // The reversal `crate::agent` records, in the place it would be
            // easiest to undo quietly.
            assert!(why.contains("did not ask for"), "{why}");
            // And the way through: the agent they chose can still be asked.
            assert!(why.contains("left pane"), "{why}");
        }

        // Whether Claude then resolves is a fact about the machine rather than
        // a decision, so what is asserted is that the spellings are one
        // question — and that the answer is never the refusal meant for a
        // different agent.
        let answer = |name: &str| match resolve(name) {
            Ok(_) => "there is a claude",
            Err(Unavailable(why)) => {
                assert!(!why.contains("is hosting `"), "read as another agent: {why}");
                assert!(why.contains("Tried:"), "{why}");
                "there is no claude"
            }
        };
        assert_eq!(answer("claude"), answer("Claude"));
        assert_eq!(answer("claude"), answer("CLAUDE"));
    }

    // --- the transcript ----------------------------------------------------

    #[test]
    fn an_empty_pane_explains_itself_and_says_whose_quota_it_spends() {
        // A blank box is indistinguishable from a broken one, and this pane is
        // empty the first time anybody opens it.
        let mut p = live();
        assert_eq!(p.title(), "ask");
        // Phrases against the document, single words against the frame: the
        // prose is wrapped, so a phrase reads perfectly well on screen while
        // being split across two rows of a flattened buffer.
        let doc = p.source();
        assert!(doc.contains("can read this repository"), "{doc}");
        assert!(doc.contains("same account"), "whose quota: {doc}");
        assert!(doc.contains("without running it"), "the promise: {doc}");
        assert!(doc.contains("nobody read"), "and why: {doc}");
        let text = screen(&mut p, 70, 24);
        assert!(text.contains("quota"), "{text}");
        assert!(text.contains("Glob"), "{text}");
        assert!(p.launch().is_some(), "it resolved something to start");
    }

    #[test]
    fn a_question_and_the_answer_arriving_under_it_are_both_on_screen() {
        let mut p = live();
        assert_eq!(ask(&mut p, "what does resolve do?").as_deref(), Some("what does resolve do?"));

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
        // ...and it says why a rate limit here is not a surprise.
        assert!(doc.contains("same account"), "{doc}");
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
            error: Some("stopped early".to_string()),
        });
        let text = screen(&mut p, 70, 20);
        assert!(text.contains("the whole answer"), "{text}");
        assert!(text.contains("stopped early"), "the reason is kept too: {text}");
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
            !p.commands.iter().any(|c| c.contains("&&") || c.contains("clippy")),
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
        assert!(said.contains("copy it out of the answer"), "the way out: {said}");
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
        assert_eq!(p.take_command(), None, "Enter sent a question and ran a command");
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
        assert!(p.scroll.max() > 0, "the transcript must overflow eight rows");

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

        // ...and asking a question is itself a reason to be at the bottom.
        p.handle_key(key(KeyCode::PageUp)).unwrap();
        assert!(!p.following);
        ask(&mut p, "and one more thing");
        assert!(p.following, "a question you asked is one you are waiting to see");
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
    fn a_question_is_never_sent_twice_however_often_the_app_asks() {
        let mut p = live();
        assert_eq!(ask(&mut p, "just the once").as_deref(), Some("just the once"));
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
        assert_eq!(p.exit_hint(), " · esc→agent");
        assert!(p.takes_input(), "the composer is live from the first frame");

        typed(&mut p, "half a question");
        assert_ne!(p.exit_hint(), " · esc→agent", "esc clears the draft first");
        assert!(p.exit_hint().contains("esc"), "{}", p.exit_hint());

        // And it goes back the moment the draft does, one press short of the
        // agent rather than a press away from it.
        p.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(p.exit_hint(), " · esc→agent");

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
        assert!(!text.contains("read-only"), "a claim with nothing behind it");
        assert!(text.contains(QUOTA), "{text}");

        p.on_event(AskEvent::Ready {
            session_id: "1e6a7c40-0000-4000-8000-000000000001".to_string(),
            model: "claude-sonnet-4".to_string(),
            tools: vec!["Glob".into(), "Grep".into(), "Read".into()],
        });
        let text = screen(&mut p, 70, 12);
        assert!(text.contains("Glob Grep Read"), "{text}");
        assert!(text.contains(QUOTA), "the other half of the row: {text}");
        // What is *not* there is the point of showing the list at all.
        for forbidden in ["Write", "Edit", "Bash"] {
            assert!(!text.contains(forbidden), "{forbidden} is on screen: {text}");
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
