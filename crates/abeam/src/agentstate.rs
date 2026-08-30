//! What abeam can learn about the agent it is hosting, without asking it.
//!
//! The queue's whole difficulty is one question â€” *has the agent finished?* â€”
//! and the honest answers available from a pty are all bad. Output quiescence
//! cannot tell a finished turn from one waiting on a permission prompt, and a
//! prompt typed into that dialog is answered by its first character. Screen
//! scraping rots on every release of every agent.
//!
//! Claude answers the question itself. It keeps one small JSON record per live
//! session under `~/.claude/sessions/<pid>.json`, rewritten as state changes:
//!
//! ```json
//! {"pid":46256,"sessionId":"â€¦","cwd":"C:\\â€¦\\forge","version":"2.1.220",
//!  "peerProtocol":1,"kind":"interactive","name":"forge-c5","status":"busy",
//!  "statusUpdatedAt":1785683809246}
//! ```
//!
//! `status` is one of `idle`, `busy`, `waiting` and `shell` on an interactive
//! session, which is the kind abeam hosts. That is the signal, it is exact, and
//! it is in a *file* â€” which is what makes it cheap enough to re-read that
//! abeam simply re-reads it, on the loop that is already running to draw the
//! agent's screen.
//!
//! A watcher was the obvious alternative and it was declined. `crate::watch`
//! runs one recursive watch of the repository root and argues for exactly one,
//! because a second doubles the OS-level event traffic for the same
//! information. This directory is not in the root at all â€” it is under the
//! user's profile, and it changes whenever *any* Claude on the machine changes
//! status, nearly all of them nothing to do with this window. A second watcher,
//! outside the repository, to save re-reading one small file a few times a
//! second, is a bad trade, and the poll is what drains the queue.
//!
//! `status` is exact about interactive sessions and about nothing else, and the
//! difference is worth stating rather than leaving to be discovered. The same
//! directory holds records for background agents â€” `claude -p --bg`, whose
//! records spell `kind` as `"bg"` â€” and those carry a `status` too, often with
//! no `state` beside it. The `state` vocabulary (`working`, `blocked`,
//! `failed`) is mostly a *roster* shape: `claude agents --json` is where an
//! entry with a `state` and no `status` turns up. [`Session::readiness`] is
//! where the two are reconciled and where the precedence rule is written down.
//!
//! ## Why this is read rather than configured
//!
//! The alternative was a `Stop` hook the user installs, which is a supported
//! interface and would work. It was not chosen because it asks somebody to
//! edit their settings before a feature in front of them works at all. Reading
//! a file that is already there asks nothing, adds nothing to the child's
//! argument list â€” `crate::agent` promises that and a test pins it â€” and
//! degrades to [`Readiness::Unknown`] rather than to a wrong answer.
//!
//! ## What is not promised
//!
//! This shape is Claude's private business, not a published API. `version` and
//! `peerProtocol` are in the record because Claude expects to change it. So:
//! every field is optional, an unreadable or unfamiliar record is
//! [`Readiness::Unknown`], and `Unknown` means queued Send items remain blocked
//! rather than guessing. The person at the keyboard can still type their text
//! in the left pane. A probe that guesses is worse than one
//! that admits it does not know â€” the failure it would cause is a prompt typed
//! into a busy agent.
//!
//! ## Why the record being current is the whole of the design
//!
//! One of Claude's four statuses is `waiting`, and it is what a session says
//! while a permission dialog is up. That is the answer to the question this
//! file opens with: a pty cannot tell a finished turn from a dialog, and the
//! record can. The design is therefore sound *exactly as far as the record
//! being read is current* â€” which makes everything that keeps it current
//! load-bearing, and makes anything that could hand back a record which is not
//! the most serious kind of bug there is here. A stale one, a neighbour's, a
//! half-written one read as though it were whole: each of those is a `status`
//! that was true of something else, and the one this module must never produce
//! is a false `idle`. [`Readiness::Unknown`] is always available and always
//! cheap, and it is the right answer to every one of them.
//!
//! None of this is true of any other agent. Copilot and Codex publish nothing
//! like it, so those sessions report `Unknown` forever and queued Send items
//! remain blocked. That is why readiness is asked of *this* module rather than
//! of `Pane`: it is knowledge about one agent, and the app/provider boundary is
//! where that per-agent knowledge belongs.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::launch::Launch;
use crate::text::clip;

/// The record shape this module understands.
///
/// A record that carries this is read. One that carries a *different* number is
/// refused outright and reported as [`Readiness::Unknown`], because a
/// `peerProtocol` that is present and unfamiliar is Claude saying, in the field
/// that exists to say it, that this record means something abeam has not been
/// taught â€” and reading that hopefully is how a `status` which has come to mean
/// something else gets read as `idle`.
///
/// A record that carries *no* `peerProtocol` is read on its other merits, and
/// that is deliberate rather than lax. The stamp is on every session file, but
/// it is not on the roster: not one entry of a real `claude agents --json
/// --all` has the field, including the entry for the very session whose file
/// has `"peerProtocol":1` in it on disk. Refusing an unstamped record would
/// therefore empty the roster on a machine where everything is working. So
/// absence is read as no claim being made, mismatch as a claim abeam cannot
/// honour, and the two are different pieces of evidence rather than one rule
/// applied twice.
pub const PEER_PROTOCOL: u64 = 1;

/// Whether the hosted agent is mid-turn.
///
/// **Four variants, and the fourth is the one [`Session::readiness`] predicted
/// would be worth having "on the day anything wants to explain itself".** That
/// day is the stacked left column: a pane the window has no room for collapses
/// to its title row, and that row is all a reader gets about the agent inside
/// it. `Unknown` on such a row draws nothing at all — abeam cannot say — which
/// left the agent *stopped on a permission dialog* as the one agent a reader
/// must go and look at and the one the row stayed silent about.
///
/// **Splitting a refusal is not widening an acceptance, and that distinction is
/// the whole safety argument.** Every gate in this program tests for `Idle` and
/// nothing else — `crate::panes::queue` compares against it and calls
/// [`is_idle`](Self::is_idle) — so a state that was `Unknown` and is now
/// `Waiting` is refused by exactly the same comparison it was refused by
/// before. What a border wanted was never a *looser* answer, which would need
/// its own function and its own argument; it was a **more specific** one, and
/// specificity cannot let a prompt through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The agent is at its prompt. A queued item may be sent.
    Idle,
    /// The agent is working, or waiting on something that is not us.
    Busy,
    /// The agent has **stopped and is waiting for a person**: Claude's
    /// `waiting` status, which is the permission dialog, or the `blocked`
    /// state, which is a background agent that has gone quiet on a question.
    ///
    /// A refusal exactly as `Unknown` is — nothing may be typed at it — and a
    /// different sentence about why. It is the one refusal the reader can *do*
    /// something about, which is why it is worth a word on a border where
    /// `Unknown` gets none: an agent stopped on a dialog stays stopped until
    /// somebody answers it, and a stack exists to show several agents at once
    /// precisely so that the one that has stopped can be noticed.
    Waiting,
    /// No record, an unreadable one, one from a protocol this does not know, or
    /// a session abeam cannot type at for a reason of its own — a shell over
    /// the agent, a child with no bracketed paste, a child that has gone.
    /// **Never treated as `Idle`.**
    Unknown,
}

impl Readiness {
    /// Whether this is a permission to type at the agent.
    ///
    /// **The one gate, and an exhaustive `match` rather than `self == Idle`,
    /// which is the whole of what makes it a mechanism.** Adding a variant to
    /// this enum is a change to what abeam might type into; written as an
    /// equality the new state would be silently refused — correct today, and
    /// correct only by luck, because nothing would have made anybody *decide*.
    /// Written as a `match` with no wildcard, `rustc` refuses the file until
    /// somebody has said, in the one place the answer matters, whether the new
    /// state lets a queued prompt through.
    ///
    /// [`Readiness::Waiting`] is the variant that proved the point. It split
    /// out of `Unknown` so that a border could name a stopped agent, and a
    /// reviewer had to enumerate every read of this enum by hand to establish
    /// that it was still refused everywhere. The next split should not need
    /// that: this is where it is answered.
    ///
    /// `crate::panes::queue` asks through here and nowhere else — both of its
    /// gates, which were two spellings of the same comparison until this
    /// existed to be the one.
    pub fn is_idle(self) -> bool {
        match self {
            Readiness::Idle => true,
            // Spelled out rather than left to a `_`, for the reason the arms in
            // [`Session::readiness`] are: a wildcard here would let a fifth
            // variant compile, which is exactly what this function is for.
            Readiness::Busy | Readiness::Waiting | Readiness::Unknown => false,
        }
    }
}

/// What kind of session a record describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A session somebody is typing at â€” including the one abeam hosts.
    Interactive,
    /// A `claude -p --bg` agent.
    Background,
    /// Something this version does not know about.
    Other,
}

/// One session, as Claude describes it.
///
/// Every field past the identifiers is optional because every field past the
/// identifiers is Claude's to change.
#[derive(Clone, Debug)]
pub struct Session {
    /// Which process wrote this record.
    ///
    /// Read by the tests and by nothing else yet, like the two below it, and
    /// kept rather than trimmed to what today's callers happen to want. This
    /// type is a *faithful* reading of a record abeam does not own: dropping a
    /// field it does carry would mean the next person to need one re-derives
    /// its `serde` spelling from a JSON sample, which is exactly the guesswork
    /// this module exists to have done once. They are asserted by
    /// `the_record_claude_writes_for_a_live_session_parses_field_for_field`, so
    /// they are covered even while they are unconsumed â€” which is the whole of
    /// what `dead_code` is objecting to here.
    ///
    /// **`#[allow]` and not `#[expect]`, and this is the case the crate root's
    /// rule is written around.** It looks like a waiver awaiting a consumer,
    /// which would be an `#[expect]`; it is really a waiver whose condition is
    /// a `cfg`. The tests above read it, so under
    /// `cargo clippy --all-targets` the lint does not fire and an `#[expect]`
    /// is *unfulfilled* — a warning of its own. See `crate`'s module docs.
    #[allow(
        dead_code,
        reason = "read by this module's tests; dead only in a build without them"
    )]
    pub pid: Option<u32>,
    /// The short id `claude agents` uses; absent on interactive sessions.
    pub id: Option<String>,
    /// The full session id, and the only field in this record that separates
    /// one session from another.
    ///
    /// **It was kept ahead of a consumer and has three now**, which is worth
    /// recording because a waiver is the shape of thing that outlives its
    /// argument. [`Probe::is_disowned`] matches it against the ids abeam
    /// minted for its own readers and the ids of panes it has closed;
    /// [`Found`] remembers it; and [`Probe::has_moved`] refuses to let a
    /// record have moved unless it carries the one that was ours. A pid could
    /// do none of that — the operating system hands those out again.
    pub session_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub kind: Kind,
    /// `busy` / `shell` / `idle` / `waiting` â€” see [`Session::readiness`] for
    /// what each of them means and where the list was read from.
    ///
    /// Not one half of a partition with [`state`](Self::state), which is what
    /// these two look like and what the real records disprove twice over. A
    /// background agent's *roster* entry can carry `"status":"busy"` and
    /// `"state":"working"` at once â€” and a `pid` and an `id` at once with them
    /// â€” while a background agent's *record file* carries a `status` and often
    /// no `state` at all. So a record may have either field, both, or neither,
    /// and [`Session::readiness`] has to say which wins. It reads this one
    /// first, because the session abeam hosts is interactive and this is the
    /// field it is written with.
    pub status: Option<String>,
    /// `working` / `blocked` / `failed`, and mostly a roster shape: this is
    /// where `claude agents --json` describes an agent, and a record file on
    /// disk frequently has no `state` in it at all.
    ///
    /// Read only where there is no `status`. It is read at all, rather than
    /// ignored as another agent's business, because of `blocked`: an agent that
    /// has stopped and gone quiet while waiting for a person is the exact shape
    /// a queue must not mistake for one that has finished.
    pub state: Option<String>,
    /// Milliseconds since the epoch, as Claude stamps it.
    pub started_at: Option<u64>,
}

impl Session {
    /// This session's readiness, from whichever of `status` and `state` it
    /// carries. Anything unrecognised is [`Readiness::Unknown`].
    ///
    /// `status` is read first where a record has both â€” a background agent's
    /// roster entry carries `"status":"busy"` and `"state":"working"` together
    /// â€” because `status` is the field the interactive record is written with,
    /// and an interactive session is what abeam hosts.
    ///
    /// ## The whole of Claude's vocabulary, and where it came from
    ///
    /// `status` is one of **`busy`, `shell`, `idle`, `waiting`**, and `state`
    /// is one of **`working`, `blocked`, `failed`**. Neither list is
    /// documented anywhere: they were read out of the shipped binary at version
    /// 2.1.220, which is why they are written down here â€” knowledge that
    /// expensive rots silently, and the next reader deserves to find it beside
    /// the code that acts on it rather than to go and get it again. Assume it
    /// is a version behind and treat the catch-all as the real contract.
    ///
    /// Two of the four are the interesting ones, and both are `Unknown`:
    ///
    /// **`waiting` is the permission dialog.** It is the finding that
    /// vindicates this module. The nightmare in the header â€” a queued prompt
    /// arriving at a dialog and being answered by its first character â€” cannot
    /// happen through a record that says `waiting`, because `waiting` is not
    /// `idle` and abeam only ever sends on `idle`. That is exactly what a pty
    /// could not tell us, and it is the reason a file is read at all.
    ///
    /// **`shell` is idle with a shell open over it.** The agent is not working,
    /// so `Busy` would be a lie, but what is in front of the keyboard is a
    /// shell: a prompt sent then goes to `bash`, not to Claude. `Unknown` is
    /// therefore the right answer rather than a shrug, and it is the one place
    /// where `Unknown` costs a reader something â€” the queue stops draining and
    /// cannot say why, because [`Readiness`] has three variants and no room for
    /// "not now, and here is the reason". Worth a fourth variant on the day
    /// anything wants to explain itself.
    ///
    /// And on the `state` side, the line worth reading twice is `blocked`. A
    /// blocked agent has stopped: its output has gone quiet, and every cheap
    /// heuristic abeam could reach for would call that finished. What it is
    /// actually doing is waiting for a person. So it is `Unknown` rather than
    /// `Idle`, and a queued Send remains blocked; the person at the keyboard can
    /// type its text in the left pane once they have made that decision.
    pub fn readiness(&self) -> Readiness {
        // Case-insensitively, because what is being matched is Claude's own
        // vocabulary in a record it wrote and not a name on a filesystem â€”
        // where whether case folds is the platform's business and
        // [`crate::paths`]'s, and on Unix the answer is that it does not. And
        // without lowercasing a copy of a word it is only comparing.
        match self.status.as_deref() {
            Some(word) if word.eq_ignore_ascii_case("idle") => Readiness::Idle,
            Some(word) if word.eq_ignore_ascii_case("busy") => Readiness::Busy,
            // Both spelled out rather than left to the catch-all below. What
            // the catch-all says is "abeam has never heard of this"; what these
            // two say is "abeam knows exactly what this is, and it is still not
            // a permission to type" — and only one of those survives Claude
            // adding a fifth status.
            //
            // **They stopped being the same answer when [`Readiness::Waiting`]
            // arrived, and the split is between what a person can act on and
            // what they cannot.** `waiting` is a dialog with somebody's name on
            // it: it will sit there until a human answers, and a border that
            // says so is a border that gets it answered. `shell` is the agent
            // being fine and abeam's own keystroke being aimed at `bash`
            // instead — nothing is stuck, nobody is being waited for, and the
            // reader has nothing to do about it. Refused identically; reported
            // differently.
            Some(word) if word.eq_ignore_ascii_case("waiting") => Readiness::Waiting,
            Some(word) if word.eq_ignore_ascii_case("shell") => Readiness::Unknown,
            // A status this version has never seen is not one it may guess at:
            // the guess that costs something is the one that guesses `Idle`.
            Some(_) => Readiness::Unknown,
            None => match self.state.as_deref() {
                Some(word) if word.eq_ignore_ascii_case("working") => Readiness::Busy,
                // `blocked` is the same shape of stop as `waiting` and gets the
                // same word: an agent that has gone quiet on a question, which
                // a person can end and nothing else can. `failed` arrives here
                // with everything else, because a failed background task is not
                // waiting for anybody.
                Some(word) if word.eq_ignore_ascii_case("blocked") => Readiness::Waiting,
                _ => Readiness::Unknown,
            },
        }
    }
}

/// Where Claude keeps the session records, honouring `CLAUDE_CONFIG_DIR`.
///
/// `None` when there is no such directory, which is the ordinary state on a
/// machine hosting some other agent.
pub fn sessions_dir() -> Option<PathBuf> {
    sessions_dir_from(
        std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
        home(),
    )
}

/// The profile directory `.claude` sits in, from the variable this platform's
/// Claude built its own path out of.
///
/// Two functions rather than one list of variables, because the same list read
/// in the same order is right on one platform and wrong on the other, and the
/// wrong answer is not a missing directory â€” it is a *different* directory,
/// which is how abeam ends up reading somebody's session records from
/// somewhere they were never written.
///
/// On Windows `USERPROFILE` is the variable the system sets and the one Claude
/// is installed under. `HOME` is behind it rather than absent because a machine
/// can have both: git-bash and MSYS set `HOME`, sometimes to a POSIX-shaped
/// path that names nothing a Windows program ever wrote to. So it is the
/// fallback for the machine that has only that one, not a second opinion about
/// a machine that has both.
///
/// Reachable from the crate rather than private to this module, because a
/// second question has the same answer:
/// `crate::panes::viewer::files::in_repository` stops its ancestry walk at the
/// home directory, so that a `git init ~` for dotfiles cannot switch the file
/// window's hidden-file guard off in the one directory that guard was written
/// for. Two readings of "where is home" is two places for the answer to drift,
/// and the platform reasoning above is the whole of why that drift would be
/// expensive.
#[cfg(windows)]
pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// `HOME` and nothing else, which is `USERPROFILE` dropped rather than demoted.
///
/// A Linux process can have a `USERPROFILE` in its environment and it never
/// means anything good: WSL's interop exports the Windows environment into
/// Linux shells in some configurations, so the variable arrives naming
/// `C:\Users\someone` â€” a string with no meaning to this kernel â€” or its
/// `/mnt/c` spelling, which is a real directory that no Linux Claude has ever
/// written a session record into. Behind `HOME` it would be harmless and
/// unreachable; in front of it, on exactly the machines where it is set, it
/// would beat the only variable that is right. A fallback that cannot be
/// correct is not a fallback, so there is one variable here.
#[cfg(unix)]
pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The directory itself, over the two variables handed in rather than read.
///
/// Split out for the reason `crate::launch::interpreter_from` is: the process
/// environment belongs to the whole test binary, and a test that set
/// `CLAUDE_CONFIG_DIR` to prove it is honoured would be setting it for the two
/// hundred tests running beside it â€” several of which spawn children that
/// inherit it.
fn sessions_dir_from(config: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let dir = sessions_path_from(config, home)?;
    // `is_dir` rather than `exists`: what the caller does next is a `read_dir`,
    // and a file of that name is not somewhere to read records from.
    dir.is_dir().then_some(dir)
}

/// Which directory those two variables name, before anything asks whether it is
/// there.
///
/// Split from the check above so that the rule below can be tested at all.
/// Whether the directory exists is a fact about the machine running the suite,
/// and the failure this rule prevents is invisible through it: a *relative*
/// path is only ever found on a machine that happens to have one beside the
/// test binary. The path is the whole of the decision, so the path is what the
/// test asserts.
fn sessions_path_from(config: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    // Absolute or not at all â€” both variables, for one reason, and the rule is
    // absoluteness rather than mere blankness because blank is only the loudest
    // way of being relative.
    //
    // What a relative one costs: joining onto it leaves a relative directory,
    // so the `is_dir` above stops being a question about anybody's profile and
    // becomes one about wherever this process is standing, which `main` has
    // deliberately moved to `%SystemRoot%` or `/`. A `/.claude/sessions`
    // belonging to somebody else would then become the search space for the
    // record that decides whether a queued prompt may be typed â€” which is the
    // one decision in this module that must never be made about the wrong
    // agent.
    //
    // Both shapes really do arrive. PowerShell leaves
    // `$env:CLAUDE_CONFIG_DIR = ""` behind when somebody clears it; a container
    // or a service unit can export an empty `HOME`, which on Unix is the only
    // variable there is; and a `CLAUDE_CONFIG_DIR=.claude` typed relative to a
    // repository is an ordinary enough mistake to make. That last one is worth
    // refusing rather than following even though it names something: Claude
    // resolves it against *its* working directory, which is the repository, and
    // abeam would resolve the same string against `/`. Two different
    // directories from one variable is not a case to guess in, and the safe
    // guess is to have no directory at all â€” `Readiness::Unknown` costs a
    // keystroke, and the other answer costs somebody's prompt going to the
    // wrong session.
    //
    // The same rule, and for the same reason, as `crate::launch::find`'s
    // refusal of a relative program path.
    let dir = match config.filter(|dir| dir.is_absolute()) {
        Some(config) => config.join("sessions"),
        None => home
            .filter(|dir| dir.is_absolute())?
            .join(".claude")
            .join("sessions"),
    };
    Some(dir)
}

/// The readiness probe abeam holds for the whole session.
///
/// Constructed once, from what the shell knows at spawn time, and then asked
/// on the frames that care. It holds no handle to the child and never blocks.
#[derive(Debug)]
pub struct Probe {
    dir: Option<PathBuf>,
    pid: Option<u32>,
    root: PathBuf,
    spawned_at: u64,
    /// The record the search settled on, so that the search runs about once
    /// per session instead of once per frame.
    ///
    /// A place and a name and nothing else — deliberately not a `Session`, and
    /// that is what makes it a cache that cannot go stale. What is remembered
    /// is *where to look and what was there*, both of which are facts about
    /// the machine and change about never; what is read is the file, every
    /// time, and it is checked against [`Probe::is_still_mine`] every time. A
    /// remembered record that has stopped being ours — the file gone, a pid
    /// handed to a Claude in another repository, a dispatched background agent
    /// written over it — is thrown away and the search runs again. So the
    /// memory can be wrong for exactly one read, and that read is the one that
    /// notices.
    found: Option<Found>,
    /// The other directories a session of ours may legitimately have moved to.
    ///
    /// Empty until `crate::app`'s discovery answers, which makes every failure
    /// of it harmless: no git, no repository, a git too old for the porcelain
    /// this needs, and the probe is exactly as strict as it was before this
    /// field existed.
    ///
    /// **Read by exactly one thing** — [`Probe::has_moved`], which revalidates
    /// a record this probe has already vouched for. It is never consulted by
    /// the pid shortcut or by the candidate filter in [`Probe::search`], and
    /// [`Probe::set_worktrees`] is where that separation is argued and where
    /// the three ways the other arrangement went wrong are written out.
    worktrees: Vec<PathBuf>,
    /// Sessions abeam started itself, which are therefore never the session
    /// abeam is *hosting*.
    ///
    /// [`Probe::disown`] is where this is argued. It is a list of `sessionId`s
    /// rather than pids because a pid is reused and a `sessionId` abeam chose
    /// is not.
    disowned: Vec<String>,
    /// The `cwd` of the last record this probe *accepted*, which is a
    /// different fact from [`root`](Self::root) and must stay one.
    ///
    /// `root` is the anchor: the directory the pane was spawned in, compared
    /// against by [`Probe::is_here`], never assigned to after [`Probe::new`].
    /// This is what the accepted record said about itself, which is the same
    /// directory in almost every session.
    ///
    /// **When it differs it is a worktree `git worktree list` named, and that
    /// is wider than the case this was built for.** The case is a session that
    /// made itself a worktree and moved in; what
    /// [`has_moved`](Self::has_moved) actually accepts is any root git printed,
    /// matched exactly — one somebody else added, one that has been there for
    /// months, a neighbour's. That is deliberate and it is safe for a reason
    /// that has nothing to do with how the directory came to exist: the record
    /// has to carry the `sessionId` that was ours. Describing the field by its
    /// motivating case would invite a later edit to narrow the check to
    /// directories abeam watched appear, which is a fact abeam does not have.
    /// Read [`standing_in`](Self::standing_in) before using either field.
    ///
    /// **Written only where a record has already been vouched for**, on both
    /// paths through [`Probe::session`], and every accepted record carries a
    /// `cwd` by construction — `is_here`, `is_mine` and `has_moved` each
    /// refuse a record without one, so there is no acceptance this can be
    /// assigned `None` by.
    ///
    /// **Never cleared, and the reason is a pane with no way out rather than a
    /// flickering border.** A record that goes missing or unreadable does not
    /// mean the session went back where it came from; it means abeam cannot see
    /// it this poll. Clearing would send `crate::app::Agent::standing` back to
    /// the directory the pane was *spawned* in — and that answer is what
    /// `workspace::rows` guarantees a row for and what
    /// `crate::app::App::agent_in` resolves `x` `x` against. So the row
    /// synthesised for the worktree the agent is working in would disappear,
    /// and the only gesture that can end that agent would start answering about
    /// a checkout it is not in. That is phase 4's "a pane whose root has no row
    /// is a pane with no way out", arriving through the very accessor built to
    /// keep the row where the work is. A border flapping between two names is
    /// the cosmetic half of the same thing and not the argument.
    ///
    /// `None` therefore means exactly one thing: this probe has never accepted
    /// a record at all, which is a pane whose child writes none.
    standing: Option<PathBuf>,
}

/// A record that was positively the session abeam hosts, and which session it
/// was when it was.
///
/// Two fields rather than a path, because a path here is a pid — the file is
/// `<pid>.json` — and a pid outlives the process it named. The name is what the
/// widened revalidation in [`Probe::has_moved`] is tied to, and there is where
/// the record it refuses is described.
#[derive(Debug)]
struct Found {
    path: PathBuf,
    /// The `sessionId` of the session that was ours at that path.
    ///
    /// `None` for a record that carried none. Every record a real Claude writes
    /// has one — `crate::agentstate`'s two captured fixtures both do — and a
    /// record that does not is simply never allowed to have moved, which is the
    /// strict answer rather than the convenient one.
    session_id: Option<String>,
}

impl Probe {
    /// `pid` is the child abeam spawned, when it knows it.
    ///
    /// It is not always the right pid, and that is the reason for the fallback
    /// below: on Windows a native install has abeam start `claude.exe` and hold
    /// its pid, but an npm install is a `claude.cmd` routed through `cmd.exe`
    /// (`crate::launch`), so the pid abeam holds is the interpreter's and the
    /// record is written by its child. Nothing is routed anywhere on Unix â€” the
    /// kernel runs a `#!` script itself â€” so the pid is the agent's own there,
    /// and the fallback is what covers a machine that has put a wrapper script
    /// in the way. `spawned_at` is milliseconds since the epoch, taken as close
    /// to the spawn as the caller can manage.
    pub fn new(root: PathBuf, pid: Option<u32>, spawned_at: u64) -> Self {
        Self {
            dir: sessions_dir(),
            pid,
            root,
            spawned_at,
            found: None,
            worktrees: Vec::new(),
            disowned: Vec::new(),
            standing: None,
        }
    }

    /// The worktrees of the repository on screen, as git last described them.
    ///
    /// ## The bug this closes
    ///
    /// A hosted session does not have to stay in the directory it started in.
    /// Claude Code makes git worktrees and moves into them, and when it does it
    /// rewrites its record with the new `cwd`. [`Probe::is_here`] compared that
    /// `cwd` with one directory, so the match failed, the record stopped being
    /// recognised as ours, [`Probe::readiness`] answered `Unknown` for ever, and
    /// the queue's automatic send stopped — **silently and permanently**. The
    /// pane goes on saying it is waiting for the agent to be idle and nothing
    /// anywhere says why, which is the worst shape a failure can have here: the
    /// feature looks present and is not.
    ///
    /// ## The fix that would have been worse than the bug
    ///
    /// `cwd.starts_with(root)`. It is one line, it fixes the symptom, and it is
    /// exactly what `crate::paths::parts` argues against at length: this is *the
    /// one function that decides whether a queued prompt may be typed into an
    /// agent*, and a loose comparison here "sends somebody's prompt to a session
    /// in another checkout, and it is not one they would see happen". A prefix
    /// test would accept any session anywhere under the repository — including
    /// the neighbouring agents Claude Code runs in `.claude/worktrees/`, which
    /// is the very layout that produced this bug.
    ///
    /// ## And the set alone was not narrow enough either
    ///
    /// A known set matched exactly is not a prefix, and it is still too wide for
    /// the question the *search* is asking. **Claude Code's neighbouring agents
    /// run at those worktree roots.** That is what a worktree is for. So a
    /// neighbour's `cwd` is not merely inside the set, it is an exact member of
    /// it — and letting the set answer "could this be our session" admitted
    /// precisely the sessions the paragraph above refuses. Three ways it went
    /// wrong, each of them answering where the code before the widening answered
    /// safely, and each with a test named after it:
    ///
    /// - a **recycled pid** landing on a neighbour's record — interactive,
    ///   started after abeam, in a directory on the list — was taken by the pid
    ///   shortcut outright. `Idle`, where it had been `Unknown`.
    /// - **clock skew**: our own record stamped a few milliseconds before
    ///   `spawned_at` makes [`Probe::search`]'s at-or-after filter find nothing,
    ///   so the documented `or_else` runs — and that fallback ignores
    ///   `spawned_at` and takes `max_by_key(started_at)` over the candidates. Over
    ///   a pool spanning every worktree, the newest thing in the repository wins,
    ///   which is whichever neighbour started last. [`Probe::found`] then
    ///   memoises it, because it satisfied the old `is_mine`. Stable, not
    ///   transient. `Idle`, where it had been `Busy`.
    /// - **two abeam windows on two worktrees of one repository**: git describes
    ///   the same repository to both, so window two spent its own startup window
    ///   — the second or two before its Claude writes a record — adopting window
    ///   one's session. `Idle`, where it had been `Unknown`.
    ///
    /// And `Idle` is exactly the answer that lets the queue type into a mid-turn
    /// agent. The exposure is not narrow either: the pid shortcut only covers a
    /// native install, and an npm install routes through `cmd.exe`, so the pid
    /// abeam holds is the interpreter's and the search is always what answers.
    ///
    /// ## The two questions, kept apart
    ///
    /// The widening exists for exactly one case — *our own* session moving into
    /// a worktree mid-flight — and by the time that happens the probe has
    /// already identified it. So the set answers only the second of these:
    ///
    /// - **Discovery is strict.** The pid shortcut and the candidate filter in
    ///   [`Probe::search`] match [`Probe::is_here`], which is an exact
    ///   comparison against the agent's own root and nothing else. A session
    ///   that has never been ours is never adopted, whatever directory it is
    ///   standing in.
    /// - **Revalidation is widened.** Once [`Probe::found`] names a record that
    ///   *was* positively ours, that session is allowed to have moved to a
    ///   directory on this list. [`Probe::has_moved`] is the whole of it, and it
    ///   is tied to the `sessionId` that was ours, so the recycled pid above
    ///   cannot walk back in through the revalidation door.
    ///
    /// Nothing about what counts as our session moved: `kind == Interactive` and
    /// `started_at >= spawned_at` are checked on both paths. What changed is that
    /// the set of places is asked about a session already known to be ours,
    /// rather than being offered as evidence that an unknown one is.
    ///
    /// The cost of the strict half is worth naming: a session that had *already*
    /// moved before the probe ever found it is not found at all, and the answer
    /// is `Unknown` for the session. That is the pre-widening behaviour for that
    /// one case — the queued Send stays blocked — and it is the direction this
    /// fails in on purpose.
    pub fn set_worktrees(&mut self, worktrees: Vec<PathBuf>) {
        self.worktrees = worktrees;
    }

    /// Never adopt this session, whatever else is true of its record.
    ///
    /// ## Why a second Claude in this directory is a hazard and not a curiosity
    ///
    /// `crate::ask` starts one. It is `claude -p` in streaming-JSON mode, it
    /// reads and cannot write, and it exists to answer questions about the file
    /// in the right pane. What it also does — verified, not assumed, on 2.1.222
    /// — is write `~/.claude/sessions/<pid>.json` with **`"kind":"interactive"`**
    /// and abeam's own `cwd`, started after abeam did.
    ///
    /// Read that against [`Probe::search`] and the problem is immediate: those
    /// are the three facts the candidate filter tests for. A reader the *user*
    /// started is admitted to the pool that decides whether the agent in the
    /// left pane is idle, and [`Probe::search`]'s documented `or_else` — the one
    /// that takes the newest record in the repository when clock skew leaves
    /// nothing at or after `spawned_at` — is where it would win, because it is
    /// always the newer of the two. The answer that comes back is that session's
    /// `status`, and a reader between questions is `idle`.
    ///
    /// `Idle` is the one answer that lets `crate::panes::queue` type into the
    /// left pane. So the failure is not "the ask pane reports the wrong state":
    /// it is a queued prompt spliced into a mid-turn agent, on the strength of a
    /// record belonging to a process abeam started itself. It is the fourth
    /// entry in the list under [`Probe::set_worktrees`], arriving by a door that
    /// list does not cover.
    ///
    /// ## Why the id, and not the pid
    ///
    /// abeam knows this child's pid, and the pid is the wrong key for the reason
    /// this whole module keeps repeating: it is handed out again. A disowned pid
    /// is a *future* Claude disowned by accident, and the direction that fails
    /// in is `Unknown` for ever with nothing on screen saying why.
    ///
    /// The `sessionId` is not the operating system's to reuse. abeam chooses it
    /// (`crate::ask::new_session_id`), passes it as `--session-id`, and the
    /// child writes it into the record — so the key here is one abeam minted,
    /// matched against a field only that child can be carrying. It is also the
    /// only field that separates the two records at all: `kind`, `cwd` and
    /// `startedAt` are the same shape for both.
    ///
    /// Consulted on **all three** paths — the pid shortcut, the candidate filter
    /// and revalidation — because a rule that holds on two of them is a rule
    /// that holds until the third one is the one that answers, which here is a
    /// difference between install shapes rather than a rare case.
    pub fn disown(&mut self, session_id: String) {
        if !self.disowned.contains(&session_id) {
            self.disowned.push(session_id);
        }
    }

    /// The `sessionId` this probe settled on, if it has settled on one.
    ///
    /// **For the moment a pane is destroyed, and it is the same hazard
    /// [`disown`](Self::disown) exists for arriving from the other end.** A
    /// record's lifetime belongs to Claude, not to abeam: a child that is
    /// killed does not tidy up after itself, so its `<pid>.json` can sit in the
    /// directory with `status` frozen at whatever it was. Start another agent
    /// in that worktree afterwards and the new probe finds no record of its own
    /// for a second or two — the pid shortcut misses, the at-or-after filter
    /// excludes the dead one on `startedAt` — and then falls through to
    /// [`Probe::search`]'s last resort, *the newest candidate there is*, which
    /// is the dead session. That fallback is there for a few milliseconds of
    /// clock skew and this is not clock skew; it hands the new pane a frozen
    /// `idle`, and `Idle` is the one answer that lets a queued prompt be typed.
    ///
    /// So `crate::app::App::close_agent` reads this as the pane goes and
    /// disowns what it finds, which **widens what the disowned list means**:
    /// it held ids abeam minted for its own readers, and it now also holds ids
    /// abeam has watched a child of its own stop carrying. Both are "records of
    /// something abeam is finished with", which is the sentence `is_disowned`
    /// was already written around.
    ///
    /// `None` when this probe never found a record — a pane closed in the first
    /// second of its life, or one whose agent was not Claude. There is then
    /// nothing to name, and the hazard survives for the case where the child
    /// wrote a record abeam never got as far as reading. That is a real gap and
    /// a small one: it needs a pane killed inside the window between its child
    /// writing its first record and the next quarter-second poll.
    pub fn found_session_id(&self) -> Option<&str> {
        self.found.as_ref()?.session_id.as_deref()
    }

    /// Where the record this probe has accepted says its session is standing.
    ///
    /// **It answers *where*, and it must never be asked *whether*.** Every
    /// question about whether a record is this pane's is settled before this is
    /// written: [`is_mine`](Self::is_mine) on discovery,
    /// [`is_still_mine`](Self::is_still_mine) on every read after it, and
    /// [`has_moved`](Self::has_moved) — gated on the `sessionId` that was ours
    /// — for the one widening in this module. What comes back is a fact about
    /// an identity check that has already passed, and a caller that used it to
    /// decide identity would be asking the answer to vouch for the question.
    ///
    /// It is therefore for **display and routing only**, and
    /// `crate::app::Agent::standing` is the whole of what reads it: a border
    /// that names the worktree an agent moved into, the occupancy count in the
    /// worktree list, and the row an `x` resolves a pane from. Not one of those
    /// can type at anything.
    ///
    /// **Not the directory the pane was spawned in**, which is
    /// [`root`](Self::root) and never changes — see [`standing`](Self::standing)
    /// for why the two cannot be collapsed, and why this is never cleared once
    /// it has an answer.
    pub fn standing_in(&self) -> Option<&Path> {
        self.standing.as_deref()
    }

    /// Whether this probe has been told to ignore `id`. A seam for
    /// `crate::app`, whose half of the promise is that every probe that exists
    /// is told and every probe made later is told out of a list.
    #[cfg(test)]
    pub fn is_disowned_for_tests(&self, id: &str) -> bool {
        self.disowned.iter().any(|mine| mine == id)
    }

    /// Whether this record belongs to something abeam started for itself.
    ///
    /// A record carrying no `sessionId` is not ours by this test, and that is
    /// the strict direction: the ids in [`Probe::disowned`] were all minted by
    /// abeam, so a record with no id cannot be one of them, and treating an
    /// absent field as a match would disown the first stranger it met.
    fn is_disowned(&self, session: &Session) -> bool {
        session
            .session_id
            .as_deref()
            .is_some_and(|id| self.disowned.iter().any(|mine| mine == id))
    }

    /// [`Probe::new`], over a directory handed in rather than looked up.
    ///
    /// The test seam for everything downstream of this module, and it exists
    /// because `new` reads the machine's own `~/.claude`: without it, a test of
    /// anything that holds a `Probe` can only ever observe
    /// [`Readiness::Unknown`], which is the one answer that is indistinguishable
    /// from the feature being deleted. `#[cfg(test)]` and `pub` for the same
    /// reason `GitPane::stub_open_request` is â€” reachable from any test in the
    /// crate, and not part of what abeam ships.
    #[cfg(test)]
    pub fn over(dir: PathBuf, root: PathBuf, pid: Option<u32>, spawned_at: u64) -> Self {
        Self {
            dir: Some(dir),
            pid,
            root,
            spawned_at,
            found: None,
            worktrees: Vec::new(),
            disowned: Vec::new(),
            standing: None,
        }
    }

    /// The directory the pane this probe belongs to was spawned in — the
    /// [`root`](Self::root) field, under the name the arguments use for it.
    ///
    /// **A test seam for the one property that has no behaviour to observe:
    /// that this never moves.** [`standing_in`](Self::standing_in) follows the
    /// session and the anchor does not, and the whole safety of that pair is
    /// that only one of them feeds [`is_here`](Self::is_here). Nothing in this
    /// module assigns to the field, so the property holds by there being no
    /// writer — which is exactly the kind of property a later edit can take away
    /// without any test going red. This is the test that would.
    ///
    /// Not called `root`, which is the field's name and would be the obvious
    /// one: the two facts this module keeps apart are a directory that moves
    /// and a directory that does not, and a reader who meets `probe.root()`
    /// beside `probe.standing_in()` has to be told which is which. `anchor` is
    /// the word every argument here already uses.
    ///
    /// `#[cfg(test)]` and `pub` for [`disowned`](Self::disowned)'s reason:
    /// reachable from any test in the crate, and not part of what abeam ships.
    #[cfg(test)]
    pub fn anchor(&self) -> &Path {
        &self.root
    }

    /// What this probe has been told, for the one question a behavioural test
    /// cannot ask.
    ///
    /// **A pair of readers rather than a test that plants records, because the
    /// subject is a probe that was built with neither.** `crate::app` seeds a
    /// pane opened on a keystroke from what the session already knows — the
    /// repository's worktrees and the sessions abeam has disowned — and both
    /// arrive here as an absence when they are forgotten: an `Unknown` that
    /// looks exactly like the feature having been deleted, and an `Idle`
    /// belonging to somebody else. Neither absence can be told from the
    /// outside on a probe reading the machine's real `~/.claude`, and
    /// [`Probe::over`] cannot help, because replacing the probe is replacing
    /// the thing under test.
    ///
    /// `#[cfg(test)]` and `pub` for that function's reason: reachable from any
    /// test in the crate, and not part of what abeam ships.
    #[cfg(test)]
    pub fn disowned(&self) -> &[String] {
        &self.disowned
    }

    /// The other half of the pair above.
    #[cfg(test)]
    pub fn worktrees(&self) -> &[PathBuf] {
        &self.worktrees
    }

    /// The hosted agent's record, found by pid where that works and by
    /// `(kind, cwd, started_at)` where it does not.
    ///
    /// Those three are checked on the pid path too. The pid is a shortcut past
    /// the *search*, not past the evidence â€” every operating system abeam runs
    /// on hands the same number out again eventually, and the file it names may
    /// be a dead session's.
    ///
    /// The fallback must not match a *different* interactive session in the
    /// same directory â€” there are usually several â€” so it takes the one whose
    /// `startedAt` is nearest to, and not before, abeam's own spawn.
    ///
    /// **What this costs, and why it takes `&mut self`.** In the steady state,
    /// one `read_to_string` of one small file â€” on both installs. Getting there
    /// is what the `&mut` is for: the first call runs the search, which on a
    /// Windows npm install (where the pid abeam holds is `cmd.exe`'s) is a
    /// `read_dir` plus a read and a parse of every `*.json` in a directory that
    /// grows with every session the user has ever started. Paying that on the
    /// loop that draws the agent's screen, several times a second, for the
    /// whole session, is the thing [`Probe::found`] exists to stop.
    ///
    /// It is not a cache of the answer. The file is read on every call and
    /// checked on every call; what is remembered is only which file to read and
    /// which session was in it.
    pub fn session(&mut self) -> Option<Session> {
        // The remembered file first, and it is trusted for exactly as long as
        // it goes on being ours — which is the one question this module asks
        // that is allowed to know a session may have moved. See
        // [`Probe::set_worktrees`] for why that is this question and not the
        // search's.
        //
        // `Unreadable` keeps the memory rather than clearing it, which is the
        // one case where those two differ: a file that is there and half
        // written is *our* file, mid-rewrite, so there is nothing to look for
        // and nowhere better to look. The answer is `Unknown` for that poll and
        // the next one reads a whole file. A record that has *gone*, or that
        // has stopped being ours, is a different matter — the memory is wrong,
        // so it is dropped and the search runs again below.
        //
        // Decided before the memory is touched rather than inside the match,
        // because the borrow of `found` that reads the path is still live in
        // the arms that would clear it.
        //
        // The accepted record is carried out of the match for that same
        // reason, one field along: writing [`standing`](Self::standing) inside
        // the arm would be a `&mut self` while the borrow that read the path is
        // still live. Both writers of that field are below, and both are on the
        // far side of an acceptance.
        let mut forget = false;
        let mut accepted = None;
        if let Some(found) = self.found.as_ref() {
            match record(&found.path) {
                Record::Read(session) if self.is_still_mine(found, &session) => {
                    accepted = Some(session);
                }
                // **Still the session that was ours, refused for some other
                // reason: `Unknown` for this poll, and the memory kept.** This
                // arm is the whole of what makes a session moving into a
                // worktree survivable, and without it the feature essentially
                // never fired.
                //
                // The sequence, which is the ordinary one rather than a corner:
                // the agent runs `git worktree add`, moves in, and rewrites its
                // record. The next poll is 250 ms later; the *discovery* that
                // would put the new directory in [`worktrees`](Self::worktrees)
                // is on a ten-second timer, so at that poll `is_here` is false
                // and [`has_moved`](Self::has_moved) is false because the
                // destination is on no list yet. Dropping the memory there is
                // final: [`has_moved`] is reachable only through
                // [`is_still_mine`](Self::is_still_mine), which needs a
                // `found`, and [`search`](Self::search) is strict and matches
                // the spawn root exactly. So discovery catching up ten seconds
                // later could never help, and the session was `Unknown` — no
                // queued send, ever — for the rest of the run.
                //
                // **Keeping it costs no identity.** `found` is a path and a
                // name; what it must never do is go on naming a *different*
                // session, and [`is_ours_but_unplaced`](Self::is_ours_but_unplaced)
                // requires the file to still carry the `sessionId` that was
                // ours, which is exactly the evidence that it does not. Nothing
                // is admitted by this arm — it answers `None` — and nothing is
                // deferred: the next poll re-asks `is_still_mine` from scratch,
                // location included.
                //
                // **It is refused *only* on location, and the narrowness is
                // load-bearing rather than tidy.** The three other conditions
                // are re-asked here as well, and dropping `started_at` in
                // particular breaks something already tested: a record of ours
                // stamped a few milliseconds before `spawned_at` fails
                // `is_still_mine` on *every* call, and is re-found on every
                // call by [`search`](Self::search)'s clock-skew `or_else` —
                // which the memory path does not have. Keeping the memory there
                // would answer `Unknown` for ever in the one case the fallback
                // exists to rescue.
                Record::Read(session) if self.is_ours_but_unplaced(found, &session) => {
                    return None;
                }
                Record::Unreadable => return None,
                _ => forget = true,
            }
        }
        if let Some(session) = accepted {
            // The one place a *move* is noticed: `is_still_mine` has just
            // allowed a record whose `cwd` is a worktree of this repository,
            // on the strength of the `sessionId` that was ours. What that
            // record says about itself is what a border and the worktree list
            // want, and it is thrown away everywhere else.
            self.standing = session.cwd.clone();
            return Some(session);
        }
        if forget {
            self.found = None;
        }

        // Only what the search finds is remembered, so a search that finds
        // nothing leaves the memory empty and the next call searches again. An
        // agent that takes two seconds to write its first record must not be
        // `Unknown` for the rest of the session because of what was true on the
        // frame after the spawn.
        let (path, session) = self.search()?;
        // Discovery is strict, so what this record says about where it is
        // standing is the agent's own root — see [`Probe::search`]. It is
        // written all the same rather than left for the first *move* to set,
        // because a field that is only ever written by the unusual path is a
        // field nothing ordinary keeps honest.
        self.standing = session.cwd.clone();
        self.found = Some(Found {
            path,
            session_id: session.session_id.clone(),
        });
        Some(session)
    }

    /// Where the record is, when nothing is remembered â€” and the record, since
    /// finding it means reading it.
    ///
    /// **This is discovery, so every comparison in it is strict.** Both
    /// branches ask [`Probe::is_here`], which is an exact match on the agent's
    /// own root, and [`Probe::worktrees`] is not consulted anywhere below.
    /// [`Probe::set_worktrees`] is where that is argued; the argument in one
    /// line is that the neighbouring agents Claude Code starts are running *at*
    /// those roots, so a candidate pool spanning them hands the `or_else` at
    /// the bottom of this function the newest agent in the repository rather
    /// than ours.
    fn search(&self) -> Option<(PathBuf, Session)> {
        let dir = self.dir.as_deref()?;

        // The pid first, because when it is the right pid it is the only answer
        // here that cannot be confused by a second window on the same
        // repository â€” and with a native install it is the right pid.
        //
        // It is checked against the other three facts all the same, and that is
        // not belt and braces. Pids are reused on both platforms â€” Windows
        // hands a number back out as soon as it is free, and a Linux counter
        // wraps round at `pid_max` and does the same â€” so a Claude that died
        // without tidying up leaves its record behind, and the number naming it
        // eventually names something else: abeam's own child, on a machine that
        // has been up a while. The file would then be a dead session's,
        // its `status` frozen at whatever it last was, and a frozen `idle` is
        // not a stale reading. It is a queued prompt typed into a mid-turn
        // agent, on every item, for the whole session.
        //
        // So: our repository (`is_here`), an interactive session rather than a
        // dispatched `claude -p --bg` â€” which runs with our `cwd` and so passes
        // that check on its own â€” and started no earlier than abeam did, which
        // is the only one of the three that can tell a stale record in *this*
        // repository from the live one. That last check is what covers the
        // window between the spawn and Claude writing its own record over the
        // dead one, which is a second or two of every native start.
        //
        // A record that fails any of them falls through to the search below
        // rather than to `Unknown`, and that is where the clock skew between
        // abeam's stamp and Claude's is handled. `procStart` is the field that
        // would settle the whole question outright â€” see [`Wire`] for why it is
        // not here.
        if let Some(pid) = self.pid {
            let path = dir.join(format!("{pid}.json"));
            match record(&path) {
                Record::Read(session) if self.is_mine(&session) => return Some((path, session)),
                // A file that is *there* and did not parse is not permission to
                // go looking for a different one. Our own record is exactly the
                // one that goes unreadable â€” Claude rewrites it in place, so a
                // read can land mid-write, and a `peerProtocol` bump makes ours
                // unreadable while an older session's in the same repository
                // stays readable. Falling through would then answer with that
                // stranger's `status`, which on this machine is a `forge`
                // session frozen at `idle`. The refusal designed to fail safe
                // would have handed the queue a wrong `idle` for the whole run.
                Record::Unreadable => return None,
                _ => {}
            }
        }

        // And when the pid is not the answer: everything in the directory,
        // narrowed to what could be the session on screen. `kind` and `cwd` are
        // cheap and rule out most of it â€” a background agent is never what
        // abeam hosts, and a session in another repository is somebody else's
        // window.
        let mut unreadable = false;
        let mut candidates: Vec<(PathBuf, Session)> = Vec::new();
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                continue;
            }
            match record(&path) {
                // `is_disowned` first, and it is the cheapest of the three, but
                // that is not why it leads. A session abeam started for itself
                // is not a weak candidate to be outranked further down — it is
                // not a candidate, and the `or_else` below is precisely a place
                // where a weak candidate wins. It has to leave here, not lose
                // later.
                Record::Read(session)
                    if !self.is_disowned(&session)
                        && session.kind == Kind::Interactive
                        && self.is_here(&session) =>
                {
                    candidates.push((path, session));
                }
                // The same failure as above, arriving the other way round: with
                // no usable pid there is nothing to say *which* record is ours,
                // so one that cannot be read might be. Judged by the file name
                // rather than by its contents, because the contents are the
                // part that failed: this directory is named by pid, and a file
                // called something else is not a record whose unreadability
                // says anything about ours.
                Record::Unreadable if is_named_for_a_pid(&path) => unreadable = true,
                _ => {}
            }
        }
        if unreadable {
            return None;
        }

        // Of the candidates, the earliest start at or after abeam's own â€” the
        // session abeam caused. One that started earlier was already running
        // when abeam began, and one that started later is a window somebody
        // opened afterwards.
        candidates
            .iter()
            .filter(|(_, session)| session.started_at.is_some_and(|at| at >= self.spawned_at))
            .min_by_key(|(_, session)| session.started_at)
            // ...and when nothing is at or after it, the newest there is.
            // abeam's stamp comes from `SystemTime::now` on this side of the
            // spawn and Claude's from its own clock a moment later, so a few
            // milliseconds either way is ordinary â€” and an exact-or-later rule
            // that found nothing would leave the queue permanently `Unknown`
            // over a rounding difference. A record with no `startedAt` at all
            // sorts below every record that has one, so it wins only when it is
            // the only thing in the directory that could be ours.
            //
            // ## What this fallback will adopt, asked and answered
            //
            // **It is written for milliseconds and it accepts any age**, and
            // two callers can hand it a record that is minutes old. A pane that
            // has just been killed leaves its file behind, because a record's
            // lifetime is Claude's and a killed child does not tidy up. And a
            // *second* agent started in a root that already has one has no
            // record of its own for a second or two, during which the newest
            // candidate in that directory is its sibling's. Either way the
            // answer is somebody else's `status`, and if that somebody is idle
            // the answer is `Idle` â€” the one answer that lets
            // `crate::panes::queue` type.
            //
            // **Neither becomes permanent, and `is_still_mine` is why.** It
            // re-asks `started_at >= spawned_at` on every call, which an older
            // record fails, so a wrong answer is never memoised: the memory is
            // dropped and the search runs again. The moment this pane's own
            // record exists it wins outright â€” by the pid shortcut on a native
            // install, and by the at-or-after filter on an npm one, where the
            // pid abeam holds is the interpreter's. So the exposure is a
            // startup window and not a session.
            //
            // **It is closed from outside rather than narrowed here**, and that
            // is a decision rather than an oversight. `crate::app::App` disowns
            // a pane's record when the pane is closed, and tells a new pane's
            // probe about every record already claimed by a pane on screen â€”
            // both are "this record belongs to somebody else", said by the one
            // party that knows. Narrowing the fallback to a *window* around
            // `spawned_at` would help here too, and would be a change to the
            // discovery rule with its own argument to make: it would leave a
            // machine whose clocks disagree by more than the window `Unknown`
            // for the whole run, which is safe and silent, and it would still
            // not separate two agents started a fraction of a second apart. The
            // hazard the window would catch and the disowning does not is a
            // pane whose probe never settled on a record before it was closed
            // or before its sibling started.
            .or_else(|| {
                candidates
                    .iter()
                    .max_by_key(|(_, session)| session.started_at.unwrap_or(0))
            })
            .cloned()
    }

    /// Whether this record describes a session in the repository on screen.
    ///
    /// **One directory, matched exactly, and that is the whole of it.** Asked by
    /// both branches of the search, which is what makes a record carrying no
    /// `cwd` at all unmatchable by either: there is nothing in it to check, and
    /// an unchecked record is exactly the one this is here to refuse. Every
    /// record a real Claude writes has the field.
    ///
    /// The comparison itself is [`crate::paths`]'s rather than this module's,
    /// and it moved there when a second question started needing the same
    /// rule. A session's `cwd`, a watcher's event and a path out of
    /// `git worktree list` are three spellings of the same kind of thing, and
    /// on Windows the third arrives with forward slashes in it. Two modules
    /// each keeping their own idea of when two spellings are one directory is
    /// how they come to disagree about which repository is on screen, which is
    /// a disagreement neither of them can see.
    ///
    /// The worktrees are **not** consulted here, and that is the correction the
    /// widening needed rather than an omission. Claude Code's neighbouring
    /// agents run *at* those roots, so a set that vouched for a discovery would
    /// vouch for them — see [`Probe::set_worktrees`] for the three ways that
    /// went wrong and for where the set is consulted instead.
    fn is_here(&self, session: &Session) -> bool {
        session
            .cwd
            .as_deref()
            .is_some_and(|cwd| crate::paths::same_dir(cwd, &self.root))
    }

    /// Everything a record has to be before this probe will *adopt* it.
    ///
    /// Asked by the pid shortcut, which is taking a record on the strength of a
    /// number the operating system will hand out again, and by nothing else.
    /// This is discovery, so it is strict: the record has to name the agent's
    /// own root.
    ///
    /// It cannot catch everything, and the gap is worth naming: a record that
    /// is replaced by a *later* interactive session in the same repository
    /// passes all three checks, because all three are true of it. Nothing short
    /// of `procStart` separates those two — see [`Wire`] — and the search has
    /// the same blind spot, so the memory is no worse than the thing it stands
    /// in for.
    fn is_mine(&self, session: &Session) -> bool {
        !self.is_disowned(session)
            && self.is_here(session)
            && session.kind == Kind::Interactive
            && session.started_at.is_some_and(|at| at >= self.spawned_at)
    }

    /// Everything a *remembered* record has to be before this probe will go on
    /// answering with it.
    ///
    /// The same two facts about the session — `kind == Interactive` and
    /// `started_at >= spawned_at` — and a wider question about where it is
    /// standing, because this record has already been positively ours and the
    /// session in it is allowed to have moved since. [`Probe::set_worktrees`] is
    /// where the split between this and [`Probe::is_mine`] is argued.
    ///
    /// The three facts that are not about a place are
    /// [`is_a_session_of_ours`](Self::is_a_session_of_ours), because
    /// [`is_ours_but_unplaced`](Self::is_ours_but_unplaced) is this question
    /// with the place taken out and the two must not be able to drift.
    fn is_still_mine(&self, found: &Found, session: &Session) -> bool {
        self.is_a_session_of_ours(session)
            && (self.is_here(session) || self.has_moved(found, session))
    }

    /// Everything a remembered record has to be **apart from where it is
    /// standing**.
    ///
    /// Split out so that [`is_still_mine`](Self::is_still_mine) and
    /// [`is_ours_but_unplaced`](Self::is_ours_but_unplaced) ask one question
    /// rather than two that happen to agree — the second is defined as the
    /// first minus its location clause, and a copy of three conditions is a
    /// copy that gets edited once.
    fn is_a_session_of_ours(&self, session: &Session) -> bool {
        !self.is_disowned(session)
            && session.kind == Kind::Interactive
            && session.started_at.is_some_and(|at| at >= self.spawned_at)
    }

    /// Still this probe's session by every test except where it is standing.
    ///
    /// **The predicate that keeps a memory alive across a move discovery has
    /// not caught up with**, and the refusal arm in [`session`](Self::session)
    /// is its only caller and carries the argument. It is
    /// [`is_still_mine`](Self::is_still_mine) with the location clause replaced
    /// by an identity one: the record has to be the same *session* — the
    /// `sessionId` that was ours, by
    /// [`is_the_session_found`](Self::is_the_session_found) — and it has to be
    /// interactive, undisowned and no older than the spawn, exactly as it would
    /// to be answered with.
    ///
    /// **It is strictly narrower than `is_still_mine` on identity and strictly
    /// wider on place, and both halves matter.** Wider on place, or this would
    /// not keep the memory of a session standing in a worktree nobody has named
    /// yet, which is the whole point. Narrower on identity, because
    /// `is_still_mine` deliberately requires no `sessionId` when the record is
    /// at the agent's own root — see [`is_mine`](Self::is_mine)'s documented
    /// blind spot — and a memory kept on the strength of a *place* nobody has
    /// vouched for would be a memory kept for anybody's record.
    fn is_ours_but_unplaced(&self, found: &Found, session: &Session) -> bool {
        Self::is_the_session_found(found, session) && self.is_a_session_of_ours(session)
    }

    /// Whether the session that was ours has moved to a directory git named as
    /// a worktree of this repository.
    ///
    /// **The one place [`Probe::worktrees`] is read**, and the only widening in
    /// this module. Two conditions, and dropping either one gives back a bug
    /// that has already happened:
    ///
    /// *The same session*, which is [`is_the_session_found`](Self::is_the_session_found)
    /// and lives there because [`session`](Self::session)'s refusal arm needs
    /// exactly that half. Without it, a recycled pid landing on a neighbouring
    /// agent in a worktree passes every other check (interactive, started after
    /// abeam, in a directory on the list) and is then *memoised*: a wrong
    /// `Idle`, stably, for the rest of the run. That is the same failure
    /// `a_dead_sessions_record_is_never_read_as_the_agent_on_screen` pins for
    /// the root, arriving one worktree over.
    ///
    /// *A named directory, matched exactly.* A set, never a prefix, and never a
    /// directory merely inside a member of it — `crate::paths::parts` is explicit
    /// that a loose comparison in this decision "sends somebody's prompt to a
    /// session in another checkout, and it is not one they would see happen".
    ///
    /// **This answering `false` is not the end of the matter, and it used to
    /// be.** The list is ten seconds old at worst and a worktree the agent
    /// makes for itself is newer than that by construction, so the *first* poll
    /// after a move always lands here and always answers `false`. What that
    /// costs is a poll answered `Unknown`; what it used to cost was the memory,
    /// and therefore the session, for the rest of the run. See the refusal arm
    /// in [`session`](Self::session).
    fn has_moved(&self, found: &Found, session: &Session) -> bool {
        Self::is_the_session_found(found, session)
            && session.cwd.as_deref().is_some_and(|cwd| {
                self.worktrees
                    .iter()
                    .any(|worktree| crate::paths::same_dir(cwd, worktree))
            })
    }

    /// Whether the record at the remembered path is still the session that was
    /// remembered there.
    ///
    /// **The identity half of [`has_moved`](Self::has_moved), split out because
    /// a second caller needs exactly it and nothing else.** That caller is the
    /// refusal arm in [`session`](Self::session), which keeps a memory that has
    /// been refused for a reason that is not identity — a session standing in a
    /// worktree discovery has not named yet — so that a later
    /// [`set_worktrees`](Self::set_worktrees) can revalidate it. Splitting it
    /// is what stops that arm from being a second, looser idea of what "still
    /// ours" means.
    ///
    /// The remembered path is `<pid>.json` and a pid outlives the process it
    /// named, so a file that was ours can be rewritten by whatever got the
    /// number next. This is the only thing that separates the two, and it is a
    /// `sessionId` rather than a pid because the operating system does not hand
    /// those out again.
    ///
    /// A record carrying no `sessionId` is refused, and so is a memory that
    /// carries none. Every record a real Claude writes has one, so this costs
    /// nothing that exists, and what it buys is that the check cannot be
    /// skipped by a record that simply omits the field.
    ///
    /// No `&self`: it compares a record against a memory and consults nothing
    /// about this probe, which is the property that keeps both callers honest.
    fn is_the_session_found(found: &Found, session: &Session) -> bool {
        let Some(known) = found.session_id.as_deref() else {
            return false;
        };
        session.session_id.as_deref() == Some(known)
    }

    /// [`Session::readiness`], or `Unknown` when there is no record to read.
    ///
    /// `&mut` for [`Probe::session`]'s reason and no other: this is the call
    /// the draw loop makes, so it is the call that must not re-run the search.
    pub fn readiness(&mut self) -> Readiness {
        self.session()
            .map(|s| s.readiness())
            .unwrap_or(Readiness::Unknown)
    }
}

/// Every session Claude currently knows about, live and finished.
///
/// **Blocking** â€” it starts a process. Call it from a worker thread, never
/// from `Pane::tick` (`crate::pane`, and `docs/conpty-findings.md`
/// constraint 2).
///
/// `claude agents --json` rather than the record files, deliberately: it is
/// the documented scripting surface (`--help` says so, and says it needs no
/// TTY), it includes finished background agents that no longer have a file,
/// and being one process for the whole list it costs the same as reading one.
pub fn roster(root: &Path) -> Result<Vec<Session>> {
    let cwd = root.to_string_lossy().into_owned();
    // `--all` for the background agents that have finished, which no longer
    // have a record file to find, and `--cwd` because the question abeam is
    // asking is about the repository on screen rather than about every Claude
    // on the machine.
    let args: Vec<String> = ["agents", "--json", "--all", "--cwd", &cwd]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect();

    // Never `Command::new("claude")`. On Windows a bare name reaching
    // `CreateProcessW` is resolved against *this* process's current directory
    // before `PATH` is consulted, so a `claude.exe` committed to the repository
    // on screen would be what ran, with the user's full token. `crate::launch`
    // is the module that exists to stop that, and it is also what turns an npm
    // `claude.cmd` â€” which `CreateProcessW` cannot start at all â€” into
    // something that runs. Neither hazard exists on Unix, where `execvp` reads
    // `PATH` and nothing else, but the resolution goes through the same module
    // there: one code path that is right everywhere beats two, one of which is
    // only ever exercised by half the machines.
    //
    // Resolved *with* these arguments rather than with none, which is what
    // [`crate::launch::Launch::args`] documents: for a routed `.cmd` the
    // caller's arguments are quoted into the command line `cmd.exe` is pointed
    // at, so `launch.args` is the complete list and appending to it would send
    // them twice.
    let launch =
        crate::launch::resolve("claude", &args).map_err(|why| anyhow!(cannot_run(&why)))?;
    ask(&launch, root)
}

/// Run it and read what it printed, over a [`Launch`] handed in rather than
/// resolved.
///
/// Split out for the reason `crate::launch::interpreter_from` and
/// `crate::agent::resolve_within` are, and for a sharper version of it: the
/// rule below â€” *a non-zero exit is forgiven when the output still parses* â€” is
/// an argument about a program that may not be on the machine at all, so
/// against the real `claude` it is either untestable or untested depending on
/// who runs the suite. Handed a `Launch`, it can be proved against a `.cmd`
/// shim that prints a known array and exits 1, which is how `crate::dispatch`
/// pins its own copy of the same rule.
fn ask(launch: &Launch, root: &Path) -> Result<Vec<Session>> {
    let out = Command::new(&launch.program)
        .args(&launch.args)
        .envs(launch.env.iter().map(|(name, value)| (name, value)))
        .current_dir(root)
        // Nothing here may ever prompt. This runs on a worker thread with
        // nowhere to type an answer, so a child that stopped to ask one would
        // hold the thread for as long as abeam is running.
        .stdin(Stdio::null())
        .output()
        .map_err(|e| anyhow!("`{}` could not be started: {e}", launch.target.display()))?;

    // A non-zero exit is forgiven when the output still parses, and only then:
    // what abeam asked for is the array, and a Claude that printed it and then
    // complained about something else has answered the question. When there is
    // no array to read, the exit status and whatever went to standard error are
    // the whole of what abeam knows, so both go in the message.
    let printed = parse_roster(&String::from_utf8_lossy(&out.stdout));
    if printed.is_err() && !out.status.success() {
        return Err(anyhow!(
            "`claude agents --json --all` failed ({}) and printed nothing abeam \
             could read: {}",
            out.status,
            first_line(&String::from_utf8_lossy(&out.stderr))
        ));
    }
    printed
}

/// Parse the array `claude agents --json` prints. Split out so the parsing is
/// testable without a Claude on the machine.
pub fn parse_roster(json: &str) -> Result<Vec<Session>> {
    // Through `Value` rather than straight into `Vec<Wire>`, so that one entry
    // abeam cannot read does not cost the reader the other four. That is not
    // hypothetical: the roster is already three shapes today â€” a background
    // agent carries `id` and `state` and may have no `pid` or `status` at all,
    // an interactive one carries `status` and no `state`, and a background
    // agent that is running carries both â€” and it is the list of every agent on
    // the machine, so the oddest one is somebody else's.
    let entries: Vec<serde_json::Value> = serde_json::from_str(json)
        .context("`claude agents --json` did not print an array of sessions")?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<Wire>(entry).ok()?.into_session())
        .collect())
}

/// Parse one `<pid>.json` record. Same reason.
pub fn parse_session(json: &str) -> Option<Session> {
    serde_json::from_str::<Wire>(json).ok()?.into_session()
}

// ---------------------------------------------------------------------------
// the record, as it is on disk
// ---------------------------------------------------------------------------

/// The record in Claude's shape rather than abeam's.
///
/// Two types for one record because they are answerable to different people:
/// this one changes when Claude changes it, and [`Session`] is what the rest of
/// the crate reads. Mapping across at the door means a field that moves costs
/// an edit here and nothing anywhere else.
///
/// Every field is optional, and none of them is `#[serde(deny_unknown_fields)]`
/// â€” a record that has grown a field abeam has never heard of is still a
/// record. `procStart`, `version`, `entrypoint`, `nameSource`, `updatedAt` and
/// `statusUpdatedAt` are all in the real record and none of them is read below.
///
/// One of those six is worth naming, so that the next person does not have to
/// rediscover it. `procStart` â€” `"procStart":"639212808652473350"`, an 18-digit
/// process-start stamp, delivered as a *string* rather than a number â€” is the
/// canonical Windows answer to pid recycling: a pid identifies a process only
/// alongside the time it started, which is why `Probe::session` has to check a
/// `cwd` instead. It is absent from this struct because abeam cannot use it
/// yet: portable-pty hands back a pid and nothing else, so there is no start
/// time on abeam's side to compare it against, and a field nothing reads is a
/// field that quietly stops being true. If the pty ever reports one, this is
/// the field to add and the `cwd` check is what it would replace.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    pid: Option<u32>,
    id: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    kind: Option<String>,
    status: Option<String>,
    state: Option<String>,
    started_at: Option<u64>,
    /// The one field this module refuses a record over â€” see
    /// [`Wire::into_session`].
    peer_protocol: Option<u64>,
}

impl Wire {
    /// This record as abeam's [`Session`], or `None` when it is not a record
    /// abeam may read.
    ///
    /// Lenient about an absent `peerProtocol` and strict about a mismatched
    /// one, which is not the same rule twice â€” [`PEER_PROTOCOL`] is where that
    /// asymmetry is argued, and the real roster is the evidence for it.
    ///
    /// ## Why a record that names no session is not a record
    ///
    /// `{}` is valid JSON and every field on it is optional, so without the
    /// second check it would parse â€” into a session with no pid, no id and no
    /// status. That is a row in the roster describing nothing and a
    /// probe answering about nobody, which is worse than an empty list in both
    /// places. All three names are accepted because the roster's own entries
    /// disagree about which they carry: a background agent has `id` and
    /// `sessionId`, an interactive one has `pid` and `sessionId`.
    fn into_session(self) -> Option<Session> {
        if self.peer_protocol.is_some_and(|seen| seen != PEER_PROTOCOL) {
            return None;
        }
        if self.pid.is_none() && self.id.is_none() && self.session_id.is_none() {
            return None;
        }
        Some(Session {
            pid: self.pid,
            id: self.id,
            session_id: self.session_id,
            cwd: self.cwd.map(PathBuf::from),
            kind: kind_of(self.kind.as_deref()),
            status: self.status,
            // Kept as the strings they arrived as. `status` and `state` are
            // Claude's vocabulary and it is longer than abeam's â€” mapping them
            // to an enum here would mean throwing away the word before anything
            // has had a chance to show it.
            state: self.state,
            started_at: self.started_at,
        })
    }
}

/// `kind`, which is the one word this module does map, because it is the field
/// the fallback search filters on and a comparison spelled out at every call
/// site is one that eventually disagrees with itself.
///
/// **A background agent is `"bg"` on disk and `"background"` in the roster.**
/// The record file gets the raw word â€” the vocabulary is `interactive`, `bg`,
/// `daemon` and `daemon-worker` â€” and `claude agents --json` renames it on the
/// way out. Both spellings therefore have to be here, and only one of them is
/// ever written to a file: matching `"background"` alone made [`Kind`] a thing
/// only the roster could produce, so every record file on the machine came back
/// as `Interactive` or `Other` and the fallback search's whole reason for
/// filtering on `kind` quietly stopped working.
///
/// `daemon` and `daemon-worker` land in [`Kind::Other`] on purpose rather than
/// by oversight. Neither is an agent taking somebody's typing, so neither is a
/// thing abeam could be hosting, and `Other` is the answer that keeps them out
/// of the search without pretending to know what they are.
fn kind_of(word: Option<&str>) -> Kind {
    match word {
        Some(word) if word.eq_ignore_ascii_case("interactive") => Kind::Interactive,
        Some(word) if word.eq_ignore_ascii_case("bg") => Kind::Background,
        Some(word) if word.eq_ignore_ascii_case("background") => Kind::Background,
        _ => Kind::Other,
    }
}

/// What was at a path where a record might have been.
///
/// Three answers rather than two, because the two failures are not the same
/// failure and reading them alike is a bug this module has already had. A file
/// that is not there says nothing about the session abeam hosts. A file that is
/// there and cannot be read says there is a session abeam cannot see â€” and it
/// is very likely *ours*, because ours is the one being rewritten while we read
/// it and the one a `peerProtocol` bump would refuse first.
enum Record {
    /// Nothing at that path, or nothing readable as a file.
    Missing,
    /// A file, and not a record this version understands.
    Unreadable,
    Read(Session),
}

/// One record file.
///
/// A failure is ordinary rather than exceptional here: Claude rewrites the
/// record in place â€” `readFile` then `writeFile`, with no temp-and-rename â€” so
/// a read landing mid-write sees half of it, and that is likeliest at exactly
/// the moment the status is changing. Either failure becomes
/// [`Readiness::Unknown`], the queue waits, and the next poll reads a whole
/// file; what the caller does with the *distinction* is in [`Probe::session`].
fn record(path: &Path) -> Record {
    match std::fs::read_to_string(path) {
        Err(_) => Record::Missing,
        Ok(text) => match parse_session(&text) {
            Some(session) => Record::Read(session),
            None => Record::Unreadable,
        },
    }
}

/// Whether this file is named the way a session record is named.
///
/// `<pid>.json`, and nothing else counts. The question being asked is "could
/// this unreadable file have been ours", and the file name is the only part of
/// it left to ask â€” the contents are what failed. Anything Claude drops in this
/// directory that is not named for a process is not a session record, and its
/// being unreadable says nothing about the session on screen.
fn is_named_for_a_pid(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()))
}

/// What abeam says when there is no Claude to ask.
///
/// A sentence, and only a sentence. There is a version of this that notices
/// `claude` is missing and fetches it, and `crate::agent` records why abeam
/// does not have one: asking abeam to read an agent's state is not consent for
/// a network install, and the gap between those two is not one an error message
/// can close after the fact. So the whole of the offer is the command that was
/// tried and the operating system's own reason it could not be.
fn cannot_run(why: &str) -> String {
    format!("abeam could not run `claude agents --json --all`: {why}")
}

/// The one line of a child's standard error worth putting in a message â€”
/// `panes::git` holds its own git failures to the same standard.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("it printed nothing");
    clip(line, 200)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Nothing here starts Claude, and that is a rule rather than an accident. A
/// machine without Claude on it â€” a build server, a laptop with Copilot â€” would
/// either fail the suite or skip the test silently, and a test suite that
/// spawns agents is not a test suite. So the parsing is tested on strings,
/// exactly as `panes::git` tests git's output without running git, and the two
/// fixtures below are real output captured from a real Claude rather than
/// invented.
///
/// One test does start a process, and it starts a four-line script that this
/// file wrote itself. It is the only way to ask what [`ask`] does with a child
/// that prints an answer and then exits non-zero, which is a rule with an
/// argument behind it and was until now the one such rule here with no test
/// under it. `crate::launch` and `crate::dispatch` prove their own claims about
/// spawning the same way.
///
/// The fixtures below are in two shapes, and which is which is worth keeping
/// straight. `RECORD` and `ROSTER` are captured output, so their Windows paths
/// stay Windows paths on every platform â€” what they prove is that a string in
/// the JSON reaches [`Session`] unaltered, and that is a fact about the parser.
/// [`ROOT`] and [`ELSEWHERE`] are *places*, compared by
/// [`crate::paths::same_dir`] under a
/// rule that differs by platform, so they are spelled for the platform running
/// the test.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// The record Claude had on disk for the session this module was written
    /// in, byte for byte. Six of its fields are ones abeam does not read, and
    /// they are kept because surviving them is the point.
    const RECORD: &str = r#"{"pid":46256,"sessionId":"9c634f25-b81f-4254-874b-e076c7116283","cwd":"C:\\Users\\philm\\PycharmProjects\\forge","startedAt":1785680468453,"procStart":"639212808652473350","version":"2.1.220","peerProtocol":1,"kind":"interactive","entrypoint":"cli","name":"forge-c5","nameSource":"derived","status":"busy","updatedAt":1785683809246,"statusUpdatedAt":1785683809246}"#;

    /// A real `claude agents --json --all`, with the mix of shapes that makes
    /// every field on [`Wire`] an `Option`: two finished background agents with
    /// no pid and no status, an interactive session with a status and no state,
    /// a running background agent carrying both, and no `peerProtocol` anywhere
    /// in the array.
    const ROSTER: &str = r#"[
      {"id":"279d4964","cwd":"C:\\Users\\philm\\PycharmProjects\\rwa_calculator","kind":"background","startedAt":1783362533693,"sessionId":"279d4964-01f8-4525-b544-7a28413f53a1","name":"Review compliance for articles 111-168","state":"blocked"},
      {"id":"f670f497","cwd":"C:\\Users\\philm\\PycharmProjects\\rwa_calculator","kind":"background","startedAt":1783462777258,"sessionId":"f670f497-aade-411c-bfc5-aca93d09afb0","name":"Determine risk class","state":"failed"},
      {"pid":11292,"cwd":"C:\\Users\\philm\\PycharmProjects\\forge","kind":"interactive","startedAt":1785602377880,"sessionId":"e8d66a2c-d65c-45ed-ae51-03e14016959d","name":"forge-14","status":"idle"},
      {"pid":45960,"id":"ed822d28","cwd":"C:\\Users\\philm\\PycharmProjects\\forge","kind":"background","startedAt":1785620172993,"sessionId":"d23ea32d-e42f-4b51-ae9a-9e7545320343","name":"Navigate between panes","status":"busy","state":"working"},
      {"pid":46256,"cwd":"C:\\Users\\philm\\PycharmProjects\\forge","kind":"interactive","startedAt":1785680468453,"sessionId":"9c634f25-b81f-4254-874b-e076c7116283","name":"forge-c5","status":"busy"}
    ]"#;

    /// The `cwd` inside [`RECORD`], spelled as that record spells it.
    ///
    /// A Windows path on every platform, because it is being compared with what
    /// came out of the parser rather than with a directory: the fixture is
    /// bytes captured from a real Claude, and a test that quietly rewrote them
    /// for the machine it is running on would no longer be checking that they
    /// survive the trip.
    const RECORD_CWD: &str = r"C:\Users\philm\PycharmProjects\forge";

    /// The repository on screen, and another one beside it to be mistaken for
    /// it â€” both as this platform writes a path.
    ///
    /// Every test that plants a record puts one of these in its `cwd`, and what
    /// they are for is [`crate::paths::same_dir`], whose rule is not the same rule
    /// on the two
    /// platforms. A Windows path planted on Linux would go through the
    /// case-sensitive comparison and pass, which is the shape of a test that
    /// runs everywhere and tests one thing in one place.
    #[cfg(windows)]
    const ROOT: &str = r"C:\Users\philm\PycharmProjects\forge";
    #[cfg(windows)]
    const ELSEWHERE: &str = r"C:\Users\philm\PycharmProjects\rwa_calculator";
    #[cfg(unix)]
    const ROOT: &str = "/home/philm/PycharmProjects/forge";
    #[cfg(unix)]
    const ELSEWHERE: &str = "/home/philm/PycharmProjects/rwa_calculator";

    /// The real record's `startedAt`, which every planted record below is
    /// placed around.
    const STARTED: u64 = 1_785_680_468_453;

    /// A record file with only the fields the search reads, so that a test
    /// about the search is not also a test about the parser.
    ///
    /// The `cwd` goes through serde rather than being pasted in: a Windows path
    /// in JSON is a string full of escapes, and a fixture that doubles its own
    /// backslashes is a fixture that is one edit away from being about
    /// something else.
    fn plant(dir: &TempDir, pid: u32, cwd: &str, started_at: u64, status: &str) {
        let record = format!(
            r#"{{"pid":{pid},"sessionId":"s-{pid}","cwd":{},"startedAt":{started_at},"peerProtocol":1,"kind":"interactive","name":"forge-{pid}","status":"{status}"}}"#,
            serde_json::to_string(cwd).expect("a JSON string")
        );
        dir.write(&format!("{pid}.json"), record.as_bytes());
    }

    /// A probe over a planted directory, through the seam rather than the
    /// machine's own `~/.claude`, which is what [`Probe::new`] would read.
    fn probe(dir: &TempDir, pid: Option<u32>, spawned_at: u64) -> Probe {
        Probe::over(
            dir.path().to_path_buf(),
            PathBuf::from(ROOT),
            pid,
            spawned_at,
        )
    }

    /// The readiness of a record with these fields and nothing else.
    fn readiness(fields: &str) -> Readiness {
        parse_session(&format!(r#"{{"sessionId":"s"{fields}}}"#))
            .expect("a record with a session id in it")
            .readiness()
    }

    // --- the two real fixtures --------------------------------------------

    #[test]
    fn the_record_claude_writes_for_a_live_session_parses_field_for_field() {
        let session = parse_session(RECORD).expect("the real record");

        assert_eq!(session.pid, Some(46256));
        assert_eq!(
            session.session_id.as_deref(),
            Some("9c634f25-b81f-4254-874b-e076c7116283")
        );
        assert_eq!(session.cwd.as_deref(), Some(Path::new(RECORD_CWD)));
        assert_eq!(session.kind, Kind::Interactive);
        assert_eq!(session.status.as_deref(), Some("busy"));
        assert_eq!(session.started_at, Some(STARTED));
        // An interactive session carries neither of the two fields a background
        // agent is identified and described by.
        assert_eq!(session.id, None);
        assert_eq!(session.state, None);

        assert_eq!(session.readiness(), Readiness::Busy);
        assert!(!session.readiness().is_idle());
    }

    #[test]
    fn every_shape_the_roster_prints_survives_the_same_parser() {
        let roster = parse_roster(ROSTER).expect("the real roster");
        assert_eq!(roster.len(), 5, "an entry was dropped");

        // A finished background agent: an `id` and a `state`, no `pid` and no
        // `status` at all. This is the shape that makes every field optional.
        let blocked = &roster[0];
        assert_eq!(blocked.pid, None);
        assert_eq!(blocked.id.as_deref(), Some("279d4964"));
        assert_eq!(blocked.kind, Kind::Background);
        assert_eq!(blocked.status, None);
        assert_eq!(blocked.state.as_deref(), Some("blocked"));
        assert_eq!(
            blocked.cwd.as_deref(),
            Some(Path::new(r"C:\Users\philm\PycharmProjects\rwa_calculator"))
        );

        // An interactive one: a `status` and no `state`.
        let interactive = &roster[2];
        assert_eq!(interactive.pid, Some(11292));
        assert_eq!(interactive.id, None);
        assert_eq!(interactive.kind, Kind::Interactive);
        assert_eq!(interactive.status.as_deref(), Some("idle"));
        assert_eq!(interactive.state, None);
        assert_eq!(interactive.readiness(), Readiness::Idle);

        // And a running background agent, which carries both â€” the case that
        // decides which of the two fields is read first.
        let working = &roster[3];
        assert_eq!(working.pid, Some(45960));
        assert_eq!(working.id.as_deref(), Some("ed822d28"));
        assert_eq!(working.status.as_deref(), Some("busy"));
        assert_eq!(working.state.as_deref(), Some("working"));
        assert_eq!(working.readiness(), Readiness::Busy);

        // Not one entry in the real array has a `peerProtocol`, which is the
        // whole reason absence is read as "no claim" rather than as a refusal:
        // reading it the other way would empty this list.
        assert!(!ROSTER.contains("peerProtocol"));
        assert_eq!(roster[4].started_at, Some(STARTED));
    }

    // --- readiness ---------------------------------------------------------

    #[test]
    fn a_status_is_read_before_a_state_and_anything_unfamiliar_is_unknown() {
        assert_eq!(readiness(r#","status":"idle""#), Readiness::Idle);
        assert_eq!(readiness(r#","status":"busy""#), Readiness::Busy);
        assert_eq!(readiness(r#","state":"working""#), Readiness::Busy);

        // Case-insensitively, because this is Claude's vocabulary in a record
        // rather than a path: `crate::paths` holds the one comparison
        // that case matters to, and on Unix it is strict on purpose.
        assert_eq!(readiness(r#","status":"IDLE""#), Readiness::Idle);
        assert_eq!(readiness(r#","status":"Busy""#), Readiness::Busy);
        assert_eq!(readiness(r#","state":"Working""#), Readiness::Busy);

        // The other two of Claude's four statuses. Both refuse a send and
        // neither does so by falling off the end of the list — and they stopped
        // being the *same* refusal when a collapsed pane's title row needed one
        // of them by name.
        //
        // `waiting` is the permission dialog â€” the state the module header's
        // nightmare is about, and the proof that reading this file rather than
        // watching the pty is what makes the nightmare impossible: a queued
        // prompt is only ever sent on `idle`. It is `Waiting` rather than
        // `Unknown` because a person can end it and a border can say so.
        assert_eq!(readiness(r#","status":"waiting""#), Readiness::Waiting);
        assert!(!readiness(r#","status":"waiting""#).is_idle());
        // `shell` is idle with a shell open over it. Not `Busy`, because
        // nothing is working; not `Idle`, because what is in front of the
        // keyboard is `bash` and a prompt sent now goes there; and not
        // `Waiting`, because nobody is being waited *for* — there is nothing
        // for a reader to go and answer.
        assert_eq!(readiness(r#","status":"shell""#), Readiness::Unknown);

        // A record with neither field knows nothing, and says so.
        assert_eq!(readiness(""), Readiness::Unknown);
        // A word from a Claude newer than this abeam is not guessed at. The
        // guess that costs something is the one that guesses `Idle`, so an
        // unrecognised status is `Unknown` in both directions.
        assert_eq!(readiness(r#","status":"compacting""#), Readiness::Unknown);
        assert_eq!(readiness(r#","status":"""#), Readiness::Unknown);
        assert_eq!(readiness(r#","state":"queued""#), Readiness::Unknown);

        // `status` wins where a record has both, which is the shape the real
        // roster's running background agent has.
        assert_eq!(
            readiness(r#","status":"busy","state":"idle""#),
            Readiness::Busy
        );
        assert_eq!(
            readiness(r#","status":"idle","state":"working""#),
            Readiness::Idle
        );

        // Only `Idle` is idle. Nothing else in this module may be treated as a
        // permission to type, and the list is exhaustive on purpose: a variant
        // added so that something can *describe* an agent has to be brought
        // past this line before it can describe one.
        assert!(Readiness::Idle.is_idle());
        assert!(!Readiness::Busy.is_idle());
        assert!(!Readiness::Waiting.is_idle());
        assert!(!Readiness::Unknown.is_idle());
    }

    #[test]
    fn a_blocked_background_session_is_never_reported_as_idle() {
        // The failure this whole module exists to prevent, in one assertion. A
        // blocked agent has stopped and gone quiet, so every heuristic short of
        // asking â€” output quiescence above all â€” reads it as finished. What it
        // is actually doing is waiting for a person to answer a permission
        // prompt, and the first character of a queued prompt answers that
        // prompt instead of arriving as a prompt. `failed` is here for the same
        // reason: it has stopped too, and it is no more at its own input.
        //
        // **The assertion that matters is `!is_idle`, and it is the one written
        // second.** The two states answer differently now — `blocked` is
        // somebody waiting for a person, which a border says out loud, and
        // `failed` is a task that has ended, which nobody can act on — and a
        // test that pinned the *word* rather than the refusal would have to be
        // rewritten every time a border learns to say something new. What may
        // never change is that neither is a permission to type.
        for (state, expected) in [
            ("blocked", Readiness::Waiting),
            ("failed", Readiness::Unknown),
            ("BLOCKED", Readiness::Waiting),
            ("Failed", Readiness::Unknown),
        ] {
            let session = parse_session(&format!(r#"{{"sessionId":"s","state":"{state}"}}"#))
                .expect("a record");
            assert_eq!(
                session.readiness(),
                expected,
                "`{state}` was not read as {expected:?}"
            );
            assert!(
                !session.readiness().is_idle(),
                "a `{state}` agent was offered a queued prompt"
            );
        }

        // And through the real roster, where the two blocked and failed
        // entries came from.
        let roster = parse_roster(ROSTER).expect("the real roster");
        assert_eq!(roster[0].state.as_deref(), Some("blocked"));
        assert_eq!(roster[0].readiness(), Readiness::Waiting);
        assert!(!roster[0].readiness().is_idle());
        assert_eq!(roster[1].state.as_deref(), Some("failed"));
        assert_eq!(roster[1].readiness(), Readiness::Unknown);
        assert!(!roster[1].readiness().is_idle());
    }

    // --- what is not a record ---------------------------------------------

    #[test]
    fn a_record_from_a_protocol_abeam_does_not_know_is_not_read_at_all() {
        // Present and different is Claude saying the record means something
        // else. There is no half-reading it: `status` is the field that
        // decides whether a prompt may be sent.
        assert!(parse_session(r#"{"sessionId":"s","peerProtocol":99,"status":"idle"}"#).is_none());
        assert!(parse_session(r#"{"sessionId":"s","peerProtocol":0,"status":"idle"}"#).is_none());
        assert!(parse_session(r#"{"sessionId":"s","peerProtocol":2,"status":"idle"}"#).is_none());

        // Present and matching, and absent altogether, are both records. The
        // second is the lenient half, and the real roster is why it is lenient:
        // not one entry in it carries the field.
        assert!(parse_session(r#"{"sessionId":"s","peerProtocol":1,"status":"idle"}"#).is_some());
        assert!(parse_session(r#"{"sessionId":"s","status":"idle"}"#).is_some());
        assert_eq!(PEER_PROTOCOL, 1);

        // ...and one bad entry in a roster costs that entry and nothing else.
        let mixed = r#"[{"sessionId":"a","peerProtocol":99,"status":"idle"},
                        {"sessionId":"b","status":"busy"}]"#;
        let roster = parse_roster(mixed).expect("an array");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].session_id.as_deref(), Some("b"));
    }

    #[test]
    fn nothing_that_is_not_a_record_is_ever_read_as_one() {
        // Every one of these is something a half-written file, an older Claude
        // or a file that is not Claude's at all could hand this module, and not
        // one of them may panic or come back as a session.
        for junk in [
            "",
            "   ",
            "{}",
            "[]",
            "null",
            "42",
            r#""a string""#,
            // The array where an object was expected: `claude agents --json`
            // output dropped into the sessions directory would look like this.
            ROSTER,
            // A read that landed mid-rewrite. Claude replaces the record in
            // place as the session's state changes.
            &RECORD[..RECORD.len() / 2],
            &RECORD[..RECORD.len() - 1],
            // Right shape, nothing in it that names a session.
            r#"{"status":"idle"}"#,
            r#"{"kind":"interactive","cwd":"C:\\forge","startedAt":1}"#,
            // Right names, wrong types.
            r#"{"sessionId":"s","pid":"46256"}"#,
            r#"{"sessionId":["s"],"status":"idle"}"#,
        ] {
            assert!(
                parse_session(junk).is_none(),
                "{junk:?} was read as a session"
            );
        }

        // The roster's side of the same question: an object where an array was
        // expected is an error rather than a panic, and an array of junk is an
        // empty list rather than five sessions of nothing.
        assert!(parse_roster(RECORD).is_err());
        assert!(parse_roster("").is_err());
        assert!(parse_roster("[").is_err());
        assert!(parse_roster("[]").expect("an empty array").is_empty());
        assert!(
            parse_roster(r#"[{}, null, 42, "x", []]"#)
                .expect("an array")
                .is_empty()
        );
    }

    // --- finding the session abeam hosts -----------------------------------

    #[test]
    fn the_hosted_session_is_found_by_the_pid_abeam_holds() {
        // The native-install case, and the only one that cannot be confused by
        // a second window on the same repository: abeam started `claude.exe`
        // itself and holds the pid that names the file.
        let dir = TempDir::new("agentstate-pid");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        plant(&dir, 11292, ROOT, STARTED - 78_000, "idle");

        let mut ours = probe(&dir, Some(46256), STARTED - 12);
        let found = ours.session().expect("the record the pid names");
        assert_eq!(found.pid, Some(46256));
        assert_eq!(ours.readiness(), Readiness::Busy);

        // The shortcut is a shortcut past the search and not past the checks.
        // A pid naming a record that started before this abeam did is a pid
        // that has been handed out twice, so it is not taken â€” and the search
        // behind it answers instead, with the session that did start when we
        // did.
        assert_eq!(
            probe(&dir, Some(11292), STARTED - 12)
                .session()
                .expect("the search behind the shortcut")
                .pid,
            Some(46256)
        );
    }

    #[test]
    fn a_dead_sessions_record_is_never_read_as_the_agent_on_screen() {
        // Pids are handed out again on both platforms. A Claude that died
        // without tidying up leaves its record behind, and the number naming it
        // eventually names something else â€” abeam's own child, on a machine
        // that has been up a while. Matched on the pid alone, that file would
        // be read as the session on screen for the rest of the run, with its
        // `status` frozen at whatever it last was. A frozen `idle` is not a
        // stale reading: it is a queued prompt typed into a mid-turn agent, on
        // every item, and it would never come right.
        let dir = TempDir::new("agentstate-recycled");
        plant(&dir, 46256, ELSEWHERE, STARTED - 900_000, "idle");

        let mut ours = probe(&dir, Some(46256), STARTED - 12);
        assert!(
            ours.session().is_none(),
            "a dead session in another repository was read as ours"
        );
        assert_eq!(ours.readiness(), Readiness::Unknown);

        // The fallback is still reached past it, so the check refuses the wrong
        // record rather than abandoning the search: the same pid, with our own
        // record beside the stale one, finds ours.
        plant(&dir, 51000, ROOT, STARTED, "busy");
        assert_eq!(
            probe(&dir, Some(46256), STARTED - 12)
                .session()
                .expect("the record that is ours")
                .pid,
            Some(51000)
        );

        // A record with no `cwd` in it has nothing to check, and an unchecked
        // record is the one this is here to refuse â€” by either branch.
        let bare = TempDir::new("agentstate-recycled-bare");
        bare.write(
            "46256.json",
            br#"{"pid":46256,"sessionId":"s","kind":"interactive","startedAt":1785680468453,"status":"idle"}"#,
        );
        assert_eq!(
            probe(&bare, Some(46256), STARTED - 12).readiness(),
            Readiness::Unknown
        );
    }

    #[test]
    fn a_dispatched_background_agent_is_not_the_session_on_screen() {
        // `crate::dispatch` starts `claude -p --bg` with `current_dir(root)`,
        // so its record carries abeam's own `cwd` and the cwd guard says
        // nothing about it at all. It is `"kind":"bg"` on disk â€” the roster's
        // `"background"` is a rename that happens on the way out of `claude
        // agents --json` â€” so a `kind` check that only knew the roster's
        // spelling would have let every dispatched agent through.
        let dir = TempDir::new("agentstate-bg");
        dir.write(
            "46256.json",
            format!(
                r#"{{"pid":46256,"id":"ed822d28","sessionId":"d23ea32d","cwd":{},"startedAt":{STARTED},"kind":"bg","status":"idle"}}"#,
                serde_json::to_string(ROOT).unwrap()
            )
            .as_bytes(),
        );

        // By pid â€” the branch that used to check only the `cwd` â€” and by the
        // search behind it, with nothing else in the directory to find.
        assert!(probe(&dir, Some(46256), STARTED - 12).session().is_none());
        assert!(probe(&dir, None, STARTED - 12).session().is_none());

        // Both spellings map to the same thing, which is what stops this from
        // being two rules that disagree.
        assert_eq!(kind_of(Some("bg")), Kind::Background);
        assert_eq!(kind_of(Some("background")), Kind::Background);
        assert_eq!(kind_of(Some("BG")), Kind::Background);
        assert_eq!(kind_of(Some("interactive")), Kind::Interactive);
        // A daemon is not an agent taking anybody's typing, so it is not
        // something abeam could be hosting â€” `Other` keeps it out of the search
        // without pretending to know what it is.
        assert_eq!(kind_of(Some("daemon")), Kind::Other);
        assert_eq!(kind_of(Some("daemon-worker")), Kind::Other);
        assert_eq!(kind_of(None), Kind::Other);
    }

    // --- the reader abeam starts for itself -------------------------------

    #[test]
    fn a_session_abeam_started_itself_is_never_the_session_abeam_hosts() {
        // `crate::ask` starts `claude -p` in this directory, and it writes an
        // *interactive* record with our `cwd`, started after us â€” which is
        // every fact the candidate filter tests for. Planted alone, so nothing
        // else can be answering: if the disown is not consulted this is the
        // only record there is, and `session()` returns it.
        let dir = TempDir::new("agentstate-ask-alone");
        plant(&dir, 99, ROOT, STARTED + 40, "idle");

        let mut probe = probe(&dir, None, STARTED);
        probe.disown("s-99".to_string());

        assert!(
            probe.session().is_none(),
            "the reader abeam started was adopted as the session abeam hosts"
        );
        // And `Unknown` rather than `Idle` is the whole point: `Idle` is the
        // answer that lets the queue type into the left pane.
        assert_eq!(probe.readiness(), Readiness::Unknown);
    }

    #[test]
    fn the_fallback_that_takes_the_newest_record_cannot_take_a_disowned_one() {
        // The documented `or_else` in `search`, which is where this would
        // actually have bitten: our own record stamped a few milliseconds
        // before `spawned_at` leaves the at-or-after filter with nothing, so
        // the fallback takes `max_by_key(started_at)` â€” and the reader is
        // always the newer of the two, because abeam starts it later by
        // construction.
        let dir = TempDir::new("agentstate-ask-newest");
        plant(&dir, 46256, ROOT, STARTED - 5, "busy"); // ours, skewed early
        plant(&dir, 99, ROOT, STARTED + 40, "idle"); // the reader, newer

        // Without the disown the newest wins, and the answer is a wrong `Idle`
        // about an agent that is mid-turn. Asserted rather than described, so
        // that deleting the disown fails this test with the real symptom.
        let mut naive = probe(&dir, None, STARTED);
        assert_eq!(naive.readiness(), Readiness::Idle);

        let mut ours = probe(&dir, None, STARTED);
        ours.disown("s-99".to_string());
        assert_eq!(
            ours.session().and_then(|s| s.pid),
            Some(46256),
            "the fallback took the reader over the agent it was skewed past"
        );
        assert_eq!(ours.readiness(), Readiness::Busy);
    }

    #[test]
    fn the_pid_shortcut_does_not_adopt_a_disowned_record_either() {
        // The third door, and the one an npm install never uses â€” which is
        // exactly why it needs its own test: a rule that holds on the two paths
        // this machine happens to take is a rule nobody notices is missing from
        // the third until somebody installs Claude the other way.
        let dir = TempDir::new("agentstate-ask-pid");
        plant(&dir, 99, ROOT, STARTED + 40, "idle");

        let mut probe = probe(&dir, Some(99), STARTED);
        probe.disown("s-99".to_string());

        assert!(probe.session().is_none());
        assert_eq!(probe.readiness(), Readiness::Unknown);
    }

    #[test]
    fn a_record_with_no_session_id_is_not_disowned_by_accident() {
        // The strict direction. Every id in the disowned list was minted by
        // abeam, so a record carrying none cannot be one of them â€” and treating
        // an absent field as a match would disown the first stranger it met,
        // which here would be the agent itself.
        let dir = TempDir::new("agentstate-ask-nameless");
        let record = format!(
            r#"{{"pid":77,"cwd":{},"startedAt":{},"peerProtocol":1,"kind":"interactive","status":"idle"}}"#,
            serde_json::to_string(ROOT).expect("a JSON string"),
            STARTED + 10
        );
        dir.write("77.json", record.as_bytes());

        let mut probe = probe(&dir, None, STARTED);
        probe.disown("s-99".to_string());

        assert_eq!(probe.readiness(), Readiness::Idle);
    }

    #[test]
    fn a_record_that_is_there_and_will_not_parse_is_never_answered_for_by_a_neighbour() {
        // The whole point of refusing a record is to fail safe, and until this
        // test the refusal did the opposite. Claude rewrites `<pid>.json` in
        // place â€” `readFile` then `writeFile`, no temp-and-rename â€” so a read
        // can land mid-write, and a future `peerProtocol` bump would make *our*
        // record unreadable while an older session's in the same repository
        // stayed readable. Either way the search behind the shortcut then
        // answered with the stranger's `status`, which on this machine is a
        // second `forge` session frozen at `idle`: a wrong `Idle`, for the
        // whole run, produced by the mechanism designed to prevent exactly
        // that.
        let dir = TempDir::new("agentstate-torn");
        plant(&dir, 11292, ROOT, STARTED - 78_000, "idle"); // the stranger
        dir.write("46256.json", &RECORD.as_bytes()[..RECORD.len() / 2]); // ours, mid-write

        // Found by pid: the file is *there*, so this is not a pid that names
        // nothing and there is nothing to fall back to.
        let mut ours = probe(&dir, Some(46256), STARTED - 12);
        assert!(
            ours.session().is_none(),
            "a torn read was answered with another session's record"
        );
        assert_eq!(ours.readiness(), Readiness::Unknown);

        // And with no usable pid â€” the npm case â€” the search cannot say which
        // record is ours, so an unreadable one might be, and it will not guess.
        let mut blind = probe(&dir, None, STARTED - 12);
        assert!(blind.session().is_none());
        assert_eq!(blind.readiness(), Readiness::Unknown);

        // A `peerProtocol` bump reads exactly the same way: refused here, and
        // so unreadable, while the older session beside it still parses.
        let bumped = TempDir::new("agentstate-bumped");
        plant(&bumped, 11292, ROOT, STARTED - 78_000, "idle");
        bumped.write(
            "46256.json",
            format!(
                r#"{{"pid":46256,"sessionId":"s","cwd":{},"startedAt":{STARTED},"peerProtocol":2,"kind":"interactive","status":"busy"}}"#,
                serde_json::to_string(ROOT).unwrap()
            )
            .as_bytes(),
        );
        assert_eq!(
            probe(&bumped, None, STARTED - 12).readiness(),
            Readiness::Unknown,
            "the stranger's `idle` was reported for a record abeam refused"
        );

        // The judgement is on the file name, because the contents are the part
        // that failed. This directory is named by pid; something else Claude
        // leaves here is not a record, and its being unreadable says nothing
        // about ours â€” so it must not take the queue down with it.
        let alongside = TempDir::new("agentstate-not-a-record");
        plant(&alongside, 46256, ROOT, STARTED, "busy");
        alongside.write("index.json", b"{ not a record, and not named for one");
        assert_eq!(
            probe(&alongside, None, STARTED - 12)
                .session()
                .expect("the search still answers")
                .pid,
            Some(46256)
        );
        assert!(is_named_for_a_pid(Path::new("46256.json")));
        assert!(!is_named_for_a_pid(Path::new("index.json")));
        assert!(!is_named_for_a_pid(Path::new("46256.json.tmp")));
        assert!(!is_named_for_a_pid(Path::new(".json")));
    }

    #[test]
    fn a_pid_that_names_no_record_falls_back_to_the_earliest_session_started_at_or_after_the_spawn()
    {
        // The Windows npm case, and any machine with a wrapper script in front
        // of the agent: `claude.cmd` is routed through `cmd.exe`, so the pid
        // abeam holds is the interpreter's and the record is written by its
        // child under a pid abeam never saw. There are three interactive
        // sessions on this repository and only one of them is the one abeam
        // just caused.
        //
        // *Earliest at or after*, not *nearest*: the record 12 ms before the
        // spawn below is nearer to it than ours is, and is excluded rather than
        // preferred. Anything already running when abeam started belongs to
        // somebody else's window whatever the arithmetic says.
        let dir = TempDir::new("agentstate-fallback");
        plant(&dir, 11292, ROOT, STARTED - 78_000, "idle"); // already running
        plant(&dir, 11293, ROOT, STARTED - 13, "idle"); // ...and only just
        plant(&dir, 46256, ROOT, STARTED, "busy"); // ours
        plant(&dir, 47001, ROOT, STARTED + 60_000, "idle"); // opened later
        // A dispatched `claude -p --bg`, started between abeam's spawn and
        // Claude's â€” so it is precisely what the earliest-at-or-after rule
        // would take if `kind` were not checked, and it runs with abeam's own
        // `cwd`, which is what makes the cwd guard useless against it. On disk
        // its `kind` is `"bg"`; `"background"` is the roster's spelling and is
        // never written to a file. It carries a `status` and no `state`, which
        // is the shape a record file has and the roster's entries do not.
        dir.write(
            "45960.json",
            format!(
                r#"{{"pid":45960,"id":"ed822d28","sessionId":"d23ea32d","cwd":{},"startedAt":{},"kind":"bg","status":"idle"}}"#,
                serde_json::to_string(ROOT).unwrap(),
                STARTED - 5
            )
            .as_bytes(),
        );

        for pid in [None, Some(4242)] {
            let found = probe(&dir, pid, STARTED - 12)
                .session()
                .expect("one of the four");
            assert_eq!(
                found.pid,
                Some(46256),
                "pid {pid:?} did not fall back to the earliest start at or after the spawn"
            );
        }

        // Not a file in the directory, and not a `.json` either: the search
        // reads what is there, so what is there has to be able to include a
        // lock file, a log, or a subdirectory.
        dir.write("notes.txt", b"not a record");
        dir.write("46256.json.tmp", b"{ half a rec");
        std::fs::create_dir_all(dir.path().join("archive")).expect("a subdirectory");
        assert_eq!(
            probe(&dir, None, STARTED - 12).session().unwrap().pid,
            Some(46256)
        );
    }

    #[test]
    fn a_session_in_another_repository_is_never_matched_by_the_fallback() {
        // Somebody else's window, with a perfect timestamp. Matching it would
        // report readiness for an agent on the other side of the machine â€” and
        // "idle" from it is a prompt typed into the agent on screen.
        let dir = TempDir::new("agentstate-elsewhere");
        plant(&dir, 11292, ELSEWHERE, STARTED, "idle");
        assert!(probe(&dir, None, STARTED - 12).session().is_none());
        assert_eq!(
            probe(&dir, None, STARTED - 12).readiness(),
            Readiness::Unknown
        );

        // And with ours beside it, ours is the one found even though the other
        // started nearer the spawn.
        plant(&dir, 46256, ROOT, STARTED + 400, "busy");
        assert_eq!(
            probe(&dir, None, STARTED - 12).session().unwrap().pid,
            Some(46256)
        );
    }

    /// Every way the two sides might have written [`ROOT`] down, each with
    /// whether this platform's filesystem calls it the same directory.
    ///
    /// A table per platform rather than a test per platform, because the rule
    /// being tested is one rule â€” a record is ours if its `cwd` is this
    /// repository â€” and all that changes underneath it is which spellings are
    /// that repository. Windows folds case, reads either separator, and does
    /// not mind a trailing one. Unix folds nothing: the trailing slash is the
    /// only one of the three that is a spelling there, and the other two name
    /// other places.
    #[cfg(windows)]
    fn spellings() -> Vec<(String, bool)> {
        vec![
            (ROOT.to_string(), true),
            (ROOT.to_ascii_lowercase(), true),
            (ROOT.to_ascii_uppercase(), true),
            (ROOT.replace('\\', "/"), true),
            (format!("{ROOT}\\"), true),
            (format!("{}/", ROOT.replace('\\', "/")), true),
        ]
    }

    #[cfg(unix)]
    fn spellings() -> Vec<(String, bool)> {
        vec![
            (ROOT.to_string(), true),
            (format!("{ROOT}/"), true),
            // Two directories, and this filesystem will happily hold both â€”
            // which is what the capital letters in [`ROOT`] are there for, and
            // why changing them would take these two assertions with them.
            (ROOT.to_ascii_lowercase(), false),
            (ROOT.to_ascii_uppercase(), false),
            // Not a separator here. `\` is an ordinary byte in a file name, so
            // this names one file with a very odd name and not a path at all.
            (ROOT.replace('/', "\\"), false),
        ]
    }

    #[test]
    fn a_cwd_is_matched_exactly_as_far_as_this_filesystem_would_match_it() {
        // abeam's `root` comes from `std::env::current_dir` and Claude's `cwd`
        // from whatever its own process was handed, so the two agree about the
        // directory and not always about how to write it down.
        //
        // Which of those disagreements is still the same place is the whole of
        // what `crate::paths` decides, and the answer is different on the two
        // platforms. This test is written to invert with it, and that is
        // deliberate: a version of it that merely passed on both â€” the Windows
        // spellings, asserted to match everywhere â€” would have been testing
        // nothing. It would go green against a spelling rule that lowercased a
        // Linux path, which is the bug where a queued prompt is typed into the
        // session in `/home/phil/work` because the one on screen is
        // `/home/phil/Work`.
        for (spelling, is_ours) in spellings() {
            let dir = TempDir::new("agentstate-cwd");
            plant(&dir, 46256, &spelling, STARTED, "idle");
            let expected = if is_ours {
                Readiness::Idle
            } else {
                Readiness::Unknown
            };
            assert_eq!(
                probe(&dir, None, STARTED - 12).readiness(),
                expected,
                "`{spelling}` should {}be the repository on screen",
                if is_ours { "" } else { "not " }
            );
        }

        // It is still a comparison of directories and not of prefixes, in both
        // directions: a sibling whose name merely begins with the same
        // characters is a different place, and so is a directory inside this
        // one â€” a Claude started in `crates/` is not the session on screen.
        // Joined rather than concatenated, so that the separator is this
        // platform's without a `cfg` for it.
        let inside = Path::new(ROOT).join("crates");
        let dir = TempDir::new("agentstate-cwd-near");
        plant(&dir, 46256, &format!("{ROOT}-old"), STARTED, "idle");
        plant(&dir, 46257, &inside.to_string_lossy(), STARTED, "idle");
        assert!(probe(&dir, None, STARTED - 12).session().is_none());
    }

    #[test]
    fn a_spawn_stamped_later_than_every_record_still_finds_the_newest_of_them() {
        // abeam's stamp is `SystemTime::now` on this side of the spawn and
        // Claude's is its own clock a moment later, so the two can disagree by
        // a few milliseconds in either direction. An exact-or-later rule that
        // found nothing would leave the queue permanently `Unknown` over a
        // rounding difference, which is a feature that silently does not work.
        let dir = TempDir::new("agentstate-skew");
        plant(&dir, 11292, ROOT, STARTED - 78_000, "idle");
        plant(&dir, 46256, ROOT, STARTED, "busy");

        let found = probe(&dir, None, STARTED + 3)
            .session()
            .expect("the newest one anyway");
        assert_eq!(found.pid, Some(46256));
        assert_eq!(found.readiness(), Readiness::Busy);

        // A record with no `startedAt` sorts below every record that has one,
        // so it is taken only when there is nothing else it could be.
        let bare = TempDir::new("agentstate-undated");
        bare.write(
            "51000.json",
            format!(
                r#"{{"pid":51000,"sessionId":"s-51000","cwd":{},"kind":"interactive","status":"idle"}}"#,
                serde_json::to_string(ROOT).unwrap()
            )
            .as_bytes(),
        );
        assert_eq!(
            probe(&bare, None, STARTED).session().unwrap().pid,
            Some(51000)
        );
        plant(&bare, 46256, ROOT, STARTED - 78_000, "busy");
        assert_eq!(
            probe(&bare, None, STARTED).session().unwrap().pid,
            Some(46256),
            "an undated record beat one that is dated"
        );
    }

    #[test]
    fn the_record_is_found_once_and_read_from_then_on() {
        // What the `&mut self` buys. On a Windows npm install the pid abeam
        // holds is `cmd.exe`'s, so every call used to be a `read_dir` plus a
        // parse of every file in a directory that grows with every session the
        // user has ever started â€” several times a second, on the loop that
        // draws the agent's screen.
        let dir = TempDir::new("agentstate-memo");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut warm = probe(&dir, None, STARTED - 12);
        assert_eq!(warm.session().expect("the first search").pid, Some(46256));

        // Proved by making the *search* impossible while leaving the record
        // alone: an unreadable pid-named file is the one thing that stops the
        // search answering at all, so a probe that still answers did not run
        // it. A cold probe over the same directory is the control.
        dir.write("11292.json", b"{ half a record");
        assert_eq!(
            warm.readiness(),
            Readiness::Busy,
            "the search was re-run when the record was already known"
        );
        assert_eq!(
            probe(&dir, None, STARTED - 12).readiness(),
            Readiness::Unknown,
            "the control found the directory readable, so the test above proves nothing"
        );

        // And it is a memory of *where*, not of what: the answer still tracks
        // the file, which is the whole reason this is safe.
        plant(&dir, 46256, ROOT, STARTED, "idle");
        assert_eq!(warm.readiness(), Readiness::Idle);

        // A record mid-rewrite is `Unknown` for that poll and no more. The
        // memory survives it, because a half-written file is still our file and
        // there is nowhere better to look.
        dir.write("46256.json", &RECORD.as_bytes()[..RECORD.len() / 2]);
        assert_eq!(warm.readiness(), Readiness::Unknown);
        plant(&dir, 46256, ROOT, STARTED, "busy");
        assert_eq!(warm.readiness(), Readiness::Busy);
    }

    #[test]
    fn a_remembered_record_that_stops_being_ours_is_noticed_rather_than_answered_from_memory() {
        // The memory is a path, and it is re-validated on every read: what
        // makes that necessary is that a pid outlives the process it named. A
        // file that is no longer ours has to send the search out again rather
        // than be answered from memory â€” otherwise the memoisation would
        // reintroduce, one indirection later, the exact stale-record bug the
        // `is_mine` guard was added to close.
        //
        // Three ways a remembered record stops being ours, and all three are
        // things `is_mine` can see. The fourth â€” the same pid recycled onto a
        // *later* interactive session in this same repository â€” passes all
        // three checks and is invisible without `procStart`. The search has the
        // same blind spot, so the memory is no worse than what it stands in
        // for, and saying so is more use than pretending otherwise.
        let elsewhere = format!(
            r#"{{"pid":46256,"sessionId":"s","cwd":{},"startedAt":{STARTED},"kind":"interactive","status":"idle"}}"#,
            serde_json::to_string(ELSEWHERE).unwrap()
        );
        let dispatched = format!(
            r#"{{"pid":46256,"sessionId":"s","cwd":{},"startedAt":{STARTED},"kind":"bg","status":"idle"}}"#,
            serde_json::to_string(ROOT).unwrap()
        );
        let older = format!(
            r#"{{"pid":46256,"sessionId":"s","cwd":{},"startedAt":{},"kind":"interactive","status":"idle"}}"#,
            serde_json::to_string(ROOT).unwrap(),
            STARTED - 900_000
        );

        for (what, replacement) in [
            ("another repository", &elsewhere),
            ("a dispatched background agent", &dispatched),
            ("a session that predates this abeam", &older),
        ] {
            let dir = TempDir::new("agentstate-restale");
            plant(&dir, 46256, ROOT, STARTED, "busy");
            let mut warm = probe(&dir, None, STARTED - 12);
            assert_eq!(warm.readiness(), Readiness::Busy, "{what}: not found once");

            // The pid is handed to something else and the record under it is
            // rewritten by whatever got it, while the session abeam actually
            // hosts turns up under a pid abeam never saw. Answered from memory,
            // the probe reports the stranger; re-validated, it goes and finds
            // ours â€” and *which file the answer came from* is the only thing
            // that tells those two apart, so that is what is asserted.
            dir.write("46256.json", replacement.as_bytes());
            plant(&dir, 51000, ROOT, STARTED, "idle");
            assert_eq!(
                warm.session().map(|found| found.pid),
                Some(Some(51000)),
                "{what} was answered out of the memory"
            );
        }

        // And with nothing else to find, noticing means saying so.
        let dir = TempDir::new("agentstate-restale-alone");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut warm = probe(&dir, None, STARTED - 12);
        assert_eq!(warm.readiness(), Readiness::Busy);
        dir.write("46256.json", elsewhere.as_bytes());
        assert_eq!(warm.readiness(), Readiness::Unknown);

        // The third replacement above is the one that would *not* have failed
        // this way. A record that predates the spawn is refused by `is_mine`,
        // so the memory is dropped â€” but the search behind it takes the newest
        // record in this repository when nothing started at or after the spawn,
        // and with one file in the directory that is the same record again.
        // That is the clock-skew rule doing exactly what it is for, and it is
        // why the loop above plants a better answer rather than asserting the
        // probe goes blind.
        let skewed = TempDir::new("agentstate-restale-skew");
        plant(&skewed, 46256, ROOT, STARTED, "busy");
        let mut warm = probe(&skewed, None, STARTED - 12);
        assert_eq!(warm.readiness(), Readiness::Busy);
        skewed.write("46256.json", older.as_bytes());
        assert_eq!(warm.readiness(), Readiness::Idle);
    }

    #[test]
    fn a_search_that_finds_nothing_is_not_remembered_as_nothing() {
        // Claude takes a second or two to write its first record, and abeam
        // asks several times a second â€” so the first several answers are always
        // "there is no record". Remembering that would make an agent that
        // starts slowly `Unknown` for the whole session, which is a feature
        // that silently does not work.
        let dir = TempDir::new("agentstate-slow-start");
        let mut waiting = probe(&dir, Some(46256), STARTED - 12);
        for _ in 0..3 {
            assert_eq!(waiting.readiness(), Readiness::Unknown);
        }

        plant(&dir, 46256, ROOT, STARTED, "idle");
        assert_eq!(
            waiting.readiness(),
            Readiness::Idle,
            "the empty directory was remembered as the answer"
        );
    }

    #[test]
    fn a_probe_with_nowhere_to_look_answers_unknown_rather_than_guessing() {
        // The ordinary state on a machine hosting some other agent: Copilot and
        // Codex publish nothing in Claude's session directory, so there is no
        // record abeam may use. Queue Send items remain blocked in both
        // automatic and Enter-triggered modes; the user types in the left pane.
        //
        // Built by hand rather than through [`Probe::over`], and this is the
        // one test that has to be: `over` takes a directory, and what is being
        // asserted here is the machine that has none. `Probe::new` finds that
        // out by reading the environment, which no test may touch.
        let mut nowhere = Probe {
            dir: None,
            pid: Some(46256),
            root: PathBuf::from(ROOT),
            spawned_at: STARTED,
            found: None,
            worktrees: Vec::new(),
            disowned: Vec::new(),
            standing: None,
        };
        assert!(nowhere.session().is_none());
        assert_eq!(nowhere.readiness(), Readiness::Unknown);
        assert_eq!(
            nowhere.standing_in(),
            None,
            "a probe that has accepted no record named a directory anyway"
        );

        // An empty directory is the same answer, and so is one that has gone
        // between construction and the frame that asks â€” a probe is held for
        // the whole session and the disk is not.
        let dir = TempDir::new("agentstate-empty");
        let mut empty = probe(&dir, Some(46256), STARTED);
        assert_eq!(empty.readiness(), Readiness::Unknown);
        drop(dir);
        assert_eq!(empty.readiness(), Readiness::Unknown);
    }

    // --- where the records are --------------------------------------------

    #[test]
    fn the_sessions_directory_is_claude_config_dirs_when_that_is_set() {
        let configured = TempDir::new("agentstate-config");
        let sessions = configured.path().join("sessions");
        std::fs::create_dir_all(&sessions).expect("a sessions directory");

        let home = TempDir::new("agentstate-home");
        let under_home = home.path().join(".claude").join("sessions");
        std::fs::create_dir_all(&under_home).expect("a .claude/sessions");

        let config = Some(configured.path().to_path_buf());
        let dot_claude = Some(home.path().to_path_buf());

        assert_eq!(
            sessions_dir_from(config.clone(), dot_claude.clone()),
            Some(sessions),
            "CLAUDE_CONFIG_DIR was set and the home directory won"
        );
        assert_eq!(
            sessions_dir_from(None, dot_claude.clone()),
            Some(under_home.clone())
        );

        // A blank variable counts as unset. PowerShell leaves one behind when
        // somebody clears it, and joining `sessions` onto nothing names a
        // relative directory â€” one that would be looked for wherever this
        // process happens to be standing.
        assert_eq!(
            sessions_dir_from(Some(PathBuf::new()), dot_claude.clone()),
            Some(under_home)
        );

        // And the same of the other variable, which is the half that had no
        // assertion under it. Asked of the path rather than of the answer,
        // because the answer goes through `is_dir` and that is a question about
        // the machine running the suite: a blank home names `.claude/sessions`,
        // which is only *found* on a machine that has one beside the test
        // binary, and is wrong on every machine. `main` leaves abeam standing
        // in `/` on Unix, so a container that exports an empty `HOME` â€” they
        // exist â€” would have the queue reading a stranger's `/.claude/sessions`
        // as the records of the agent on screen. Delete these and that comes
        // back silently, on the machines least able to notice.
        assert_eq!(sessions_path_from(None, Some(PathBuf::new())), None);
        assert_eq!(sessions_dir_from(None, Some(PathBuf::new())), None);

        // Blank is only the loudest way of being relative, so the rule is
        // absoluteness and these are the quiet ways. `CLAUDE_CONFIG_DIR=.claude`
        // is an ordinary thing to type at a repository, and it is the case that
        // reads as working: it names something, so a guard against emptiness
        // alone waves it through, and then Claude resolves it against the
        // repository while abeam resolves the same string against `/` â€” two
        // directories from one variable, and abeam reading the wrong one to
        // decide whether a queued prompt may be sent.
        for relative in [".claude", "claude/config", ".."] {
            assert_eq!(
                sessions_path_from(Some(PathBuf::from(relative)), dot_claude.clone()),
                Some(home.path().join(".claude").join("sessions")),
                "a relative CLAUDE_CONFIG_DIR was taken: {relative}"
            );
            assert_eq!(
                sessions_path_from(None, Some(PathBuf::from(relative))),
                None,
                "a relative home was taken: {relative}"
            );
        }

        // ...while a variable that does name somewhere still names it, which is
        // what stops the guard above from being a guard against everything.
        assert_eq!(
            sessions_path_from(None, dot_claude.clone()),
            Some(home.path().join(".claude").join("sessions"))
        );

        // Nothing to read is `None`, whichever half is missing: no variables at
        // all, a configured directory with no `sessions` in it, a home with no
        // `.claude`, and a *file* where the directory should be.
        assert_eq!(sessions_dir_from(None, None), None);
        assert_eq!(
            sessions_dir_from(Some(home.path().to_path_buf()), None),
            None
        );
        assert_eq!(
            sessions_dir_from(None, Some(configured.path().to_path_buf())),
            None
        );
        let file = TempDir::new("agentstate-file");
        file.write("sessions", b"not a directory");
        assert_eq!(
            sessions_dir_from(Some(file.path().to_path_buf()), None),
            None
        );
    }

    // --- the message when there is no Claude -------------------------------

    #[test]
    fn a_claude_that_cannot_be_found_is_a_sentence_naming_what_was_tried() {
        // `roster` is the one function here that starts a process, and no test
        // in this file runs it against a real Claude â€” a machine without one
        // would fail the suite, and a test suite that spawns agents is not a
        // test suite. What is pinned instead is the failure it produces on such
        // a machine, and the failure is a sentence: abeam does not install
        // anything, ever, so naming the command it could not run is the whole
        // of what it has to offer.
        let refused = crate::launch::resolve("abeam-no-such-claude", &[])
            .expect_err("that program is on no machine");
        let said = cannot_run(&refused);
        assert!(said.contains("claude agents --json --all"), "got: {said}");
        assert!(said.contains("not found on PATH"), "got: {said}");
        assert!(
            !said.contains("install") && !said.contains("download"),
            "the failure path must not offer to fetch anything: {said}"
        );
    }

    /// A program that prints `out` on standard output and `err` on standard
    /// error, then exits with `code`.
    ///
    /// A shim rather than a real Claude for the reason the module doc gives,
    /// and a script rather than a compiled program because a test cannot
    /// compile one. On Windows that is a `.cmd`, which has the side benefit of
    /// exercising the routed-script path `crate::launch` exists for, since that
    /// is what an npm-installed Claude is; on Unix it is a `#!` script, which
    /// the kernel runs itself with nothing in between.
    ///
    /// Both halves take the two streams as two arguments rather than having the
    /// caller smuggle a `1>&2` into the text to be printed, which is how this
    /// used to reach standard error: a test asking which of the two ends up in
    /// abeam's message should not also be asking how each shell parses a
    /// redirection sitting in the middle of an argument list. Neither half may
    /// be handed a payload containing a single quote â€” the Unix one fences the
    /// text with them and there is no escaping.
    #[cfg(windows)]
    fn shim(dir: &TempDir, name: &str, out: &str, err: &str, code: u8) -> Launch {
        let mut script = String::from("@echo off\r\n");
        if !out.is_empty() {
            script.push_str(&format!("echo {out}\r\n"));
        }
        if !err.is_empty() {
            script.push_str(&format!("echo {err} 1>&2\r\n"));
        }
        script.push_str(&format!("exit /b {code}\r\n"));
        let path = dir.write(&format!("{name}.cmd"), script.as_bytes());
        crate::launch::resolve(&path.to_string_lossy(), &[]).expect("a .cmd is launchable")
    }

    #[cfg(unix)]
    fn shim(dir: &TempDir, name: &str, out: &str, err: &str, code: u8) -> Launch {
        let mut script = String::from("#!/bin/sh\n");
        // `printf` with the payload in single quotes, because the payload is
        // JSON: `echo [{"sessionId":"s"}]` hands the shell four quotes to eat
        // and prints something that no longer parses, which would make this a
        // test of `sh` quoting wearing a test of `ask` as a hat.
        if !out.is_empty() {
            script.push_str(&format!("printf '%s\\n' '{out}'\n"));
        }
        if !err.is_empty() {
            script.push_str(&format!("printf '%s\\n' '{err}' 1>&2\n"));
        }
        script.push_str(&format!("exit {code}\n"));
        // With the execute bit, which is the whole difference between this and
        // a text file: without it `execve` refuses and the test would be about
        // `crate::launch`'s error path instead of about `ask`'s rule.
        let path = dir.write_exec(name, script.as_bytes());
        crate::launch::resolve(&path.to_string_lossy(), &[]).expect("a `#!` script is launchable")
    }

    #[test]
    fn an_agent_list_that_arrived_is_kept_however_the_process_exited() {
        // The rule `roster` argues for and had no test under: what abeam asked
        // for is the array, so a Claude that printed one and then exited
        // non-zero has answered the question. Anything else makes the roster
        // vanish on a warning about a stale background agent.
        let dir = TempDir::new("agentstate-shim");
        let root = dir.path().to_path_buf();

        let printed = shim(
            &dir,
            "abeam-roster-ok",
            r#"[{"sessionId":"s","kind":"interactive","status":"idle"}]"#,
            "",
            1,
        );
        let roster = ask(&printed, &root).expect("stdout parsed, so the exit code is forgiven");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].readiness(), Readiness::Idle);

        // And only then. With nothing readable on stdout, the exit status and
        // what the child said are the whole of what abeam knows, so both are in
        // the message rather than an empty list that looks like an answer.
        let broke = shim(&dir, "abeam-roster-bad", "", "Error: no such command", 3);
        let refused = ask(&broke, &root).expect_err("nothing parsed and it failed");
        let said = refused.to_string();
        assert!(said.contains("claude agents --json --all"), "got: {said}");
        assert!(
            said.contains('3'),
            "the exit status is missing from: {said}"
        );
        assert!(
            said.contains("no such command"),
            "what the child said is missing from: {said}"
        );

        // A zero exit with unreadable output is still a failure, and it is the
        // parser's error rather than a sentence about an exit code â€” there was
        // nothing wrong with the exit.
        let empty = shim(&dir, "abeam-roster-empty", "not json at all", "", 0);
        assert!(ask(&empty, &root).is_err());
    }

    #[test]
    fn a_failed_roster_is_described_by_its_first_line_of_standard_error() {
        assert_eq!(
            first_line("\n\n  error: no such command `agents`  \nusage: claude\n"),
            "error: no such command `agents`"
        );
        assert_eq!(first_line(""), "it printed nothing");
        assert_eq!(first_line("   \n \n"), "it printed nothing");
        assert_eq!(first_line(&"x".repeat(400)).chars().count(), 200);
    }

    // --- a session that moved --------------------------------------------

    /// A worktree of [`ROOT`], where Claude Code puts one, spelled for the
    /// platform running the test.
    fn worktree(name: &str) -> String {
        Path::new(ROOT)
            .join(".claude")
            .join("worktrees")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    /// The set `crate::app` really hands the probe.
    ///
    /// **Every root `git worktree list` printed**, which is the repository
    /// itself and every worktree of it — including the ones the neighbouring
    /// agents Claude Code starts are running in. That is the whole reason this
    /// helper exists rather than each test naming one directory: a test that
    /// seeds a set the production wiring never produces can only prove things
    /// about a set nobody has. The test below this one used to do exactly that,
    /// listing `review` while planting a record in `other`, and it passed
    /// against a rule that admits both.
    fn the_repository() -> Vec<PathBuf> {
        vec![
            PathBuf::from(ROOT),
            PathBuf::from(worktree("review")),
            PathBuf::from(worktree("other")),
        ]
    }

    /// Rewrite the record at `pid`, which is what Claude does when a session
    /// moves: the file is replaced in place and the `sessionId` in it does not
    /// change, because it is the same conversation in a different directory.
    fn moves_to(dir: &TempDir, pid: u32, cwd: &str, started_at: u64, status: &str) {
        let record = format!(
            r#"{{"pid":{pid},"sessionId":"s-{pid}","cwd":{},"startedAt":{started_at},"peerProtocol":1,"kind":"interactive","name":"forge-{pid}","status":"{status}"}}"#,
            serde_json::to_string(cwd).expect("a JSON string")
        );
        dir.write(&format!("{pid}.json"), record.as_bytes());
    }

    #[test]
    fn a_session_that_moved_into_a_worktree_is_still_the_one_on_screen() {
        // The live bug. Claude Code makes worktrees and moves into them, and
        // when it does it rewrites its record with the new `cwd`. A probe that
        // knew one directory stopped recognising its own session, answered
        // `Unknown` for ever, and the queue's automatic send stopped —
        // silently and permanently, with the pane still saying it was waiting
        // for the agent to be idle.
        //
        // Found *here* first, and that is the shape of the fix rather than a
        // convenience of the fixture: discovery is an exact match on the
        // agent's own root, and only a record this probe has already vouched
        // for is allowed to have moved.
        let dir = TempDir::new("agentstate-moved");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        probe.set_worktrees(the_repository());
        assert_eq!(probe.readiness(), Readiness::Busy);

        // The session moves. Same file, same `sessionId`, new `cwd`.
        moves_to(&dir, 46256, &worktree("review"), STARTED, "idle");
        assert_eq!(probe.readiness(), Readiness::Idle);

        // ...and again, which is what makes this a rule rather than one hop.
        moves_to(&dir, 46256, &worktree("other"), STARTED, "busy");
        assert_eq!(probe.readiness(), Readiness::Busy);

        // Back to the root it started in, which is `is_here` unchanged.
        moves_to(&dir, 46256, ROOT, STARTED, "idle");
        assert_eq!(probe.readiness(), Readiness::Idle);
    }

    #[test]
    fn a_session_in_a_worktree_that_was_never_ours_is_never_adopted() {
        // The half the widening got wrong. Before discovery answers there is
        // nothing vouching for any directory; *after* it answers, every
        // neighbouring agent's worktree is in the set — so a rule that widened
        // discovery would admit a session that has never been ours at any point
        // in its life. Nothing about a directory being a worktree of this
        // repository makes the agent in it the agent abeam hosts.
        let dir = TempDir::new("agentstate-never-ours");
        plant(&dir, 46256, &worktree("review"), STARTED, "idle");

        // By the pid shortcut...
        let mut by_pid = probe(&dir, Some(46256), STARTED);
        by_pid.set_worktrees(the_repository());
        assert_eq!(by_pid.readiness(), Readiness::Unknown);

        // ...and by the search behind it, which is the branch an npm install
        // always takes.
        let mut by_search = probe(&dir, None, STARTED);
        by_search.set_worktrees(the_repository());
        assert_eq!(by_search.readiness(), Readiness::Unknown);
    }

    #[test]
    fn a_worktree_nobody_named_is_not_ours_however_far_inside_the_repository_it_is() {
        // The one-line fix this refuses. `cwd.starts_with(root)` closes the bug
        // above and opens a worse one: `crate::paths` is explicit that a loose
        // comparison in *this* function "sends somebody's prompt to a session
        // in another checkout, and it is not one they would see happen" — and
        // the neighbouring agents Claude Code runs in `.claude/worktrees/` are
        // precisely the sessions a prefix test would accept.
        //
        // Seeded with the set `crate::app` really produces, which is what this
        // test used to get wrong: it listed `review` while planting the record
        // in `other`, so it passed for want of a set rather than for want of a
        // prefix. Every worktree here is named, the neighbour's among them, and
        // it is still not ours.
        let dir = TempDir::new("agentstate-neighbour");
        plant(&dir, 46256, &worktree("other"), STARTED, "idle");
        let mut named = probe(&dir, Some(46256), STARTED);
        named.set_worktrees(the_repository());
        assert_eq!(named.readiness(), Readiness::Unknown);

        // And a worktree that is not on the list at all is no more ours for
        // being deep inside the repository — the prefix rule, refused twice.
        let scratch = TempDir::new("agentstate-unlisted");
        plant(&scratch, 46256, &worktree("scratch"), STARTED, "idle");
        let mut unlisted = probe(&scratch, Some(46256), STARTED);
        unlisted.set_worktrees(the_repository());
        assert_eq!(unlisted.readiness(), Readiness::Unknown);
    }

    #[test]
    fn a_recycled_pid_landing_on_a_neighbours_record_is_not_the_session_on_screen() {
        // A pid is handed out again on both platforms, and once the set of
        // worktrees was allowed to vouch for a *discovery*, the number could
        // land on a neighbouring agent's record and be taken outright: an
        // interactive session, started after abeam, in a directory on the list.
        // Every check passes and not one of them is about our session.
        //
        // `Idle` is what that answers, and `Idle` is the one answer that lets
        // the queue type into somebody else's mid-turn agent.
        let dir = TempDir::new("agentstate-recycled-worktree");
        plant(&dir, 46256, &worktree("other"), STARTED, "idle");

        let mut alone = probe(&dir, Some(46256), STARTED - 12);
        alone.set_worktrees(the_repository());
        assert_eq!(alone.readiness(), Readiness::Unknown);

        // The shortcut refuses the wrong record rather than abandoning the
        // search, exactly as it does for a recycled pid in another repository:
        // our own record, under a pid abeam never saw, is still found.
        plant(&dir, 51000, ROOT, STARTED, "busy");
        let mut beside = probe(&dir, Some(46256), STARTED - 12);
        beside.set_worktrees(the_repository());
        assert_eq!(beside.readiness(), Readiness::Busy);
    }

    #[test]
    fn clock_skew_does_not_hand_the_probe_the_newest_agent_in_the_repository() {
        // The documented `or_else` in `search`, which exists because abeam's
        // stamp comes from `SystemTime::now` on this side of the spawn and
        // Claude's from its own clock a moment later — so a record of ours can
        // land a few milliseconds *before* `spawned_at` and the
        // at-or-after filter can find nothing.
        //
        // That fallback ignores `spawned_at` entirely and takes
        // `max_by_key(started_at)` over the candidate pool. Widen the pool to
        // every worktree of the repository and the newest thing in it wins,
        // which is whichever neighbouring agent was started last — and `found`
        // then *memoises* it, because it satisfies `is_mine`. Stable, not
        // transient, and `Idle`.
        let dir = TempDir::new("agentstate-skew-neighbour");
        plant(&dir, 46256, ROOT, STARTED - 3, "busy"); // ours, stamped early
        plant(&dir, 47001, &worktree("other"), STARTED - 1, "idle"); // a neighbour, newer

        let mut probe = probe(&dir, None, STARTED);
        probe.set_worktrees(the_repository());
        assert_eq!(probe.readiness(), Readiness::Busy);
        // Twice, because the failure this is about is the memoised one: an
        // answer that is wrong on the first read and then wrong for ever.
        assert_eq!(probe.readiness(), Readiness::Busy);
    }

    #[test]
    fn a_second_abeam_on_a_second_worktree_does_not_adopt_the_first_ones_session() {
        // Two windows on two worktrees of one repository, which is the layout
        // this whole feature is about. Both probes see the same set — git
        // describes the same repository to both — so a rule that let the set
        // vouch for a discovery gives window two the run of window one's
        // sessions.
        //
        // The window it happens in is the startup one: for the second or two
        // before Claude writes window two's own record, the only interactive
        // session in the repository is window one's, it predates window two's
        // spawn, and the `or_else` above takes it.
        let dir = TempDir::new("agentstate-second-window");
        plant(&dir, 11292, ROOT, STARTED - 60_000, "idle"); // window one

        let mut window_two = Probe::over(
            dir.path().to_path_buf(),
            PathBuf::from(worktree("review")),
            None,
            STARTED,
        );
        window_two.set_worktrees(the_repository());
        assert_eq!(
            window_two.readiness(),
            Readiness::Unknown,
            "window two adopted window one's session during its own startup"
        );

        // ...and its own session, when it arrives, is the one it finds.
        plant(&dir, 46256, &worktree("review"), STARTED + 2, "busy");
        assert_eq!(window_two.readiness(), Readiness::Busy);
    }

    #[test]
    fn a_directory_inside_a_known_worktree_is_not_that_worktree() {
        // An exact match against a set, never a prefix — including against the
        // members of the set, and including on the one path where the set is
        // consulted at all.
        let dir = TempDir::new("agentstate-deeper");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        probe.set_worktrees(the_repository());
        assert_eq!(probe.readiness(), Readiness::Busy);

        // A session that moves *inside* a known worktree has moved somewhere
        // nobody named, and the probe stops answering for it rather than
        // guessing which worktree it meant.
        let deeper = Path::new(&worktree("review")).join("crates").join("abeam");
        moves_to(&dir, 46256, &deeper.to_string_lossy(), STARTED, "idle");
        assert_eq!(probe.readiness(), Readiness::Unknown);
    }

    #[test]
    fn widening_where_a_session_may_be_does_not_widen_what_counts_as_one() {
        // `is_mine`'s other checks are untouched on both paths, and the one
        // that matters most here is `kind`: a dispatched `claude -p --bg` runs
        // with our `cwd` — and in our worktrees too — so it passes the
        // directory check on its own and is still not the session abeam hosts.
        let dir = TempDir::new("agentstate-worktree-kind");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        probe.set_worktrees(the_repository());
        assert_eq!(probe.readiness(), Readiness::Busy);

        // The remembered file becomes a background agent's, in a worktree that
        // is on the list. Being somewhere our session may be is not being our
        // session.
        let cwd = serde_json::to_string(&worktree("review")).expect("a JSON string");
        dir.write(
            "46256.json",
            format!(
                r#"{{"pid":46256,"sessionId":"s-46256","cwd":{cwd},"startedAt":{},"peerProtocol":1,"kind":"bg","status":"idle"}}"#,
                STARTED + 1
            )
            .as_bytes(),
        );
        assert_eq!(probe.readiness(), Readiness::Unknown);
    }

    #[test]
    fn only_the_session_that_was_ours_is_allowed_to_have_moved() {
        // What the memory is a memory *of*. The remembered path is `<pid>.json`
        // and a pid outlives the process it named, so a file that was ours can
        // be rewritten by whatever got the number next — and if the widening
        // trusted the path alone, a recycled pid landing on a neighbouring
        // agent in a worktree would walk straight back in through the
        // revalidation door, memoised, with every other check passing.
        //
        // So the widened arm is tied to the session that was positively ours:
        // same file, same `sessionId`. At the agent's own root nothing is tied
        // to anything, which keeps the blind spot `is_mine` already documents
        // exactly the size it was.
        let dir = TempDir::new("agentstate-moved-identity");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        probe.set_worktrees(the_repository());
        assert_eq!(probe.readiness(), Readiness::Busy);

        // A different session, in a worktree of this repository, under the pid
        // ours had. Interactive, started after abeam, in a directory on the
        // list: every check but the identity passes.
        dir.write(
            "46256.json",
            format!(
                r#"{{"pid":46256,"sessionId":"somebody-else","cwd":{},"startedAt":{},"peerProtocol":1,"kind":"interactive","status":"idle"}}"#,
                serde_json::to_string(&worktree("other")).expect("a JSON string"),
                STARTED + 1
            )
            .as_bytes(),
        );
        assert_eq!(probe.readiness(), Readiness::Unknown);

        // ...and the memory is gone with it, which is the cost of the strict
        // half written as an assertion rather than as a promise. A session in a
        // worktree that nothing has vouched for cannot be *discovered*, so the
        // same session coming back under that pid is not found until it is
        // somewhere discovery can see it.
        moves_to(&dir, 46256, &worktree("other"), STARTED, "idle");
        assert_eq!(
            probe.readiness(),
            Readiness::Unknown,
            "a worktree nobody vouched for was rediscovered"
        );

        // At the agent's own root it is found again, and from there it is free
        // to move — which is what stops the identity check above from being a
        // check against everything.
        moves_to(&dir, 46256, ROOT, STARTED, "busy");
        assert_eq!(probe.readiness(), Readiness::Busy);
        moves_to(&dir, 46256, &worktree("other"), STARTED, "idle");
        assert_eq!(probe.readiness(), Readiness::Idle);
    }

    #[test]
    fn a_session_that_moves_before_discovery_names_the_worktree_is_recovered_when_it_does() {
        // **The order every real move happens in, and the one that made the
        // whole feature never fire.** `crate::app` refreshes this list from a
        // `git worktree list` on a ten-second timer; a worktree the agent has
        // just made for itself is newer than the last one of those by
        // construction. So the first poll after a move — 250 ms later — finds
        // `is_here` false and `has_moved` false, because the destination is on
        // no list yet.
        //
        // Dropping the memory there was final. `has_moved` is reachable only
        // through `is_still_mine`, which needs a `found`, and `search` matches
        // the spawn root exactly — so discovery catching up ten seconds later
        // could never put it back, and the session was `Unknown`, with no
        // queued send ever delivered to it, for the rest of the run.
        let dir = TempDir::new("agentstate-moved-early");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        // What discovery had said when the pane started: the repository, and
        // nothing that did not exist yet.
        probe.set_worktrees(vec![PathBuf::from(ROOT)]);
        assert_eq!(probe.readiness(), Readiness::Busy);

        // The move, into a directory the list does not have.
        moves_to(&dir, 46256, &worktree("review"), STARTED, "idle");
        assert_eq!(
            probe.readiness(),
            Readiness::Unknown,
            "a worktree nothing has vouched for was answered for anyway"
        );
        // Twice, because what this is really about is what the *second* poll
        // can still do — and a dozen polls happen before discovery next runs.
        assert_eq!(probe.readiness(), Readiness::Unknown);

        // Discovery catches up, and the session comes back rather than being
        // lost with the memory.
        probe.set_worktrees(the_repository());
        assert_eq!(
            probe.readiness(),
            Readiness::Idle,
            "the memory was thrown away, so the session could never be recovered"
        );
        assert!(
            probe
                .standing_in()
                .is_some_and(|at| crate::paths::same_dir(at, Path::new(&worktree("review")))),
            "the recovered session is not reported where it went: {:?}",
            probe.standing_in()
        );
    }

    #[test]
    fn a_memory_kept_across_an_unnamed_move_is_still_dropped_on_identity() {
        // The boundary of the arm above, and the reason it tests the
        // `sessionId` rather than the directory. Keeping a memory is keeping a
        // *name*; the moment the file under it stops carrying that name it is
        // somebody else's record, and no amount of discovery may bring it back.
        let dir = TempDir::new("agentstate-unplaced-identity");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        probe.set_worktrees(vec![PathBuf::from(ROOT)]);
        assert_eq!(probe.readiness(), Readiness::Busy);

        // A different session, under the pid ours had, in a worktree nothing
        // has named yet: interactive, started after abeam, and refused.
        dir.write(
            "46256.json",
            format!(
                r#"{{"pid":46256,"sessionId":"somebody-else","cwd":{},"startedAt":{},"peerProtocol":1,"kind":"interactive","status":"idle"}}"#,
                serde_json::to_string(&worktree("review")).expect("a JSON string"),
                STARTED + 1
            )
            .as_bytes(),
        );
        assert_eq!(probe.readiness(), Readiness::Unknown);

        // And the memory went with it, so naming that worktree does not hand
        // this probe a session it never identified. `search` is strict, and
        // this is the assertion that says the keeping arm did not quietly
        // widen it.
        probe.set_worktrees(the_repository());
        assert_eq!(
            probe.readiness(),
            Readiness::Unknown,
            "a stranger's record was revalidated by a list arriving later"
        );

        // **The step that makes the two versions of this tell apart**, and
        // without it the assertions above pass whether the identity is checked
        // or not: a kept memory and a dropped one both answer `Unknown` while
        // the file is a stranger's. So the file becomes ours again — the same
        // `sessionId`, in a worktree that is now on the list — and the answer
        // is still `Unknown`, because a memory that was dropped is dropped and
        // a session standing where discovery cannot vouch for it is not
        // rediscovered. That is the documented cost of the strict half, and it
        // is what a kept memory would silently buy back.
        moves_to(&dir, 46256, &worktree("review"), STARTED, "idle");
        assert_eq!(
            probe.readiness(),
            Readiness::Unknown,
            "the memory survived a stranger and let our own session back in \
             through a door `search` had closed"
        );
    }

    #[test]
    fn the_probe_names_the_directory_of_the_record_it_accepted() {
        // The display half of the bug the widening closed for readiness. A
        // session that moves into a worktree goes on being read — and nothing
        // outside this module could find out *where* it went, so a border named
        // the checkout the pane was spawned in and the worktree list credited
        // its occupancy to that same wrong row.
        let dir = TempDir::new("agentstate-standing");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        probe.set_worktrees(the_repository());

        // Nothing has been read yet, so there is nothing to say. `None` is also
        // the answer a pane hosting a Codex or a Copilot keeps for ever.
        assert_eq!(
            probe.standing_in(),
            None,
            "a probe named a directory before it had accepted a record"
        );

        assert_eq!(probe.readiness(), Readiness::Busy);
        assert!(
            probe
                .standing_in()
                .is_some_and(|at| crate::paths::same_dir(at, Path::new(ROOT))),
            "discovery is strict, so the first accepted record is at the root: {:?}",
            probe.standing_in()
        );

        // The session moves, and the answer moves with it.
        moves_to(&dir, 46256, &worktree("review"), STARTED, "idle");
        assert_eq!(probe.readiness(), Readiness::Idle);
        assert!(
            probe
                .standing_in()
                .is_some_and(|at| crate::paths::same_dir(at, Path::new(&worktree("review")))),
            "the probe went on naming the directory the pane was spawned in: {:?}",
            probe.standing_in()
        );

        // **And the anchor did not move**, which is the safety argument written
        // as an assertion rather than left as a property nothing observes.
        // `is_here` compares a record's `cwd` against this directory; an anchor
        // that followed the record would be the record vouching for itself, and
        // the revalidation `has_moved` gates on a `sessionId` would have nothing
        // left to be strict about.
        assert!(
            crate::paths::same_dir(probe.anchor(), Path::new(ROOT)),
            "the identity anchor followed the session it anchors: {:?}",
            probe.anchor()
        );

        // Back where it started, which is `is_here` again rather than the
        // widening — so this is the hop that proves the field is written by
        // every acceptance and not only by the interesting one.
        moves_to(&dir, 46256, ROOT, STARTED, "busy");
        assert_eq!(probe.readiness(), Readiness::Busy);
        assert!(
            probe
                .standing_in()
                .is_some_and(|at| crate::paths::same_dir(at, Path::new(ROOT))),
            "a session that came back was still reported one worktree over: {:?}",
            probe.standing_in()
        );
    }

    #[test]
    fn a_record_this_probe_refuses_does_not_move_where_it_says_the_session_is() {
        // The rule that keeps the accessor from becoming a second way of asking
        // whether a record is ours: it is written on the far side of an
        // acceptance and nowhere else. A refused record answers `Unknown` for
        // the readiness, and must leave the directory alone rather than
        // reporting where the stranger was standing.
        let dir = TempDir::new("agentstate-refused-standing");
        plant(&dir, 46256, ROOT, STARTED, "busy");
        let mut probe = probe(&dir, Some(46256), STARTED);
        probe.set_worktrees(the_repository());
        assert_eq!(probe.readiness(), Readiness::Busy);

        // A different session under the pid ours had, in a worktree on the
        // list: interactive, started after abeam, refused on the identity
        // alone. `only_the_session_that_was_ours_is_allowed_to_have_moved` is
        // where that refusal is pinned; this is what it must not leak.
        dir.write(
            "46256.json",
            format!(
                r#"{{"pid":46256,"sessionId":"somebody-else","cwd":{},"startedAt":{},"peerProtocol":1,"kind":"interactive","status":"idle"}}"#,
                serde_json::to_string(&worktree("other")).expect("a JSON string"),
                STARTED + 1
            )
            .as_bytes(),
        );
        assert_eq!(probe.readiness(), Readiness::Unknown);
        assert!(
            probe
                .standing_in()
                .is_some_and(|at| crate::paths::same_dir(at, Path::new(ROOT))),
            "a refused record moved the answer a border is drawn from: {:?}",
            probe.standing_in()
        );

        // And a record that has simply *gone* leaves it alone too, which is the
        // never-cleared rule: abeam cannot see the session this poll, and that
        // is not the same as the session having gone back where it came from.
        // The alternative is a border flapping between two names on a
        // transient.
        //
        // Through the root on the way, because the refusal above dropped the
        // memory and a session in a worktree cannot be *discovered* — the cost
        // the strict half of the widening pays, asserted in
        // `only_the_session_that_was_ours_is_allowed_to_have_moved`.
        moves_to(&dir, 46256, ROOT, STARTED, "busy");
        assert_eq!(probe.readiness(), Readiness::Busy);
        moves_to(&dir, 46256, &worktree("review"), STARTED, "idle");
        assert_eq!(probe.readiness(), Readiness::Idle);
        std::fs::remove_file(dir.path().join("46256.json")).expect("a record to remove");
        assert_eq!(probe.readiness(), Readiness::Unknown);
        assert!(
            probe
                .standing_in()
                .is_some_and(|at| crate::paths::same_dir(at, Path::new(&worktree("review")))),
            "a record going missing was read as the session moving home: {:?}",
            probe.standing_in()
        );
    }
}
