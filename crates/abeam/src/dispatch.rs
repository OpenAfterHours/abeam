//! Starting a queued task as a background agent.
//!
//! This is the second half of the queue, and the half that does not type at
//! anybody. `claude -p --bg` starts a session that returns immediately and
//! runs on its own; `claude agents --json` reports what became of it
//! (`crate::agentstate::roster`). So a queued task can be *dispatched* rather
//! than *typed*, which sidesteps the whole question of when the pane in front
//! of you is ready for input.
//!
//! The two modes are deliberately not interchangeable, and the difference is
//! about context rather than mechanism:
//!
//! - **Sent** — typed into the session in the left pane when it goes idle. It
//!   continues the conversation you have been having: the agent remembers
//!   everything above it. Sequential by nature; there is one of it.
//! - **Dispatched** — a new session with none of that context, running beside
//!   yours. Parallel by nature; there can be many. For work that stands on its
//!   own, which is most of what people queue.
//!
//! ## What is added to the command line, and what is not
//!
//! Four things beyond the prompt, and only one of them is a decision about
//! authority. `-p` and `--bg` are the mode itself — a background agent has no
//! terminal to be interactive at, and `--bg` is the whole feature. A `--`
//! fences the prompt off from all three, for the reason given below.
//!
//! That leaves the one that is a choice: `--permission-mode acceptEdits`,
//! chosen by the user rather than by this module. A background agent that stops
//! to ask permission is a background agent that does nothing until somebody
//! notices, and the point of dispatching is not to have to notice. It is the
//! narrow form — edits land without asking, `Bash` and the rest still stop —
//! and it is *not* `--dangerously-skip-permissions`, which this module must
//! never pass.
//!
//! No `--worktree`, deliberately. Creating a git worktree in somebody's
//! repository as a side effect of queueing a task is a structural change to
//! their checkout that they did not ask for. It is the obvious next option and
//! it belongs behind an explicit per-item choice, not a default.
//!
//! The fence, then, which is not a flag at all: a `--` between abeam's
//! arguments and the prompt. The prompt is text somebody wrote — pasted,
//! usually, and `--- rewrite this section ---` is an ordinary way to begin one
//! — and without the fence Claude's own parser reads that leading dash as a
//! flag. It is the hazard `crate::agent`'s module docs are about one level up,
//! with the stakes turned around: there, a flag meant for the child was at risk
//! of being eaten by abeam, and here a *prompt* is at risk of becoming a flag on
//! a command line that then runs with nobody watching it. A pasted paragraph
//! must not be able to grant a process authority the user did not choose.
//!
//! ## What a command line cannot carry, and which install pays for it
//!
//! A prompt reaches a dispatched agent as an argument, and on one of the two
//! ways Claude gets onto a Windows machine that is a real constraint rather
//! than a formality.
//!
//! A native install is `claude.exe`, which abeam starts directly. Nothing below
//! applies to it.
//!
//! An npm install is `claude.cmd`, which `CreateProcessW` cannot run at all, so
//! `crate::launch` starts it through `cmd.exe` — and `cmd.exe` has limits that
//! are its own and not abeam's. **A prompt containing a newline cannot be put
//! on such a command line in any form**: a newline ends the command outright
//! and a carriage return truncates it, and there is no escape for either. A
//! prompt beyond roughly eight kilobytes is longer than `cmd` will run.
//!
//! The first of those is not an edge case here, which is why it is at the top
//! of the file rather than buried next to the code. `panes::queue` turns a
//! pasted block into a single item, so a multi-line prompt is the *ordinary*
//! shape of a queued task — meaning that on an npm install this refusal is
//! something a user meets on their first real dispatch. It is a refusal and
//! never a truncation: a command line quietly cut in half is indistinguishable,
//! from outside, from an argument injection that worked. The message names the
//! way through, and it is a good one — the same item can still be *sent* to the
//! session in the left pane, because that mode is typed at the agent rather
//! than quoted for a command processor.
//!
//! ### The route out of this, not taken yet
//!
//! `claude -p` reads a prompt from standard input when one is piped to it —
//! that is what `--print`'s own help means by "useful for pipes" — and a prompt
//! arriving that way is bytes on a pipe rather than text on a command line,
//! which dismisses both limits above at once, newline and length together. It
//! is the obvious next move for anyone who finds the npm route in their way.
//!
//! It is not taken here because of one unanswered question: `--bg` returns
//! immediately by design, and whether it drains standard input *before* it
//! detaches or leaves the pipe to a process that has already let go of it is
//! not something the flag's documentation says. Answering it means starting a
//! real background agent and watching what it does with the prompt, which costs
//! somebody's tokens and puts an unattended agent in a real repository — so it
//! wants to be a deliberate experiment rather than a thing tried on the way
//! past. Until then the prompt is an argument, and the refusals above are
//! honest about what that costs.
//!
//! ## Why this is Claude-only, and says so
//!
//! `--bg` is Claude's. Copilot publishes no equivalent, so hosting Copilot
//! leaves this mode unavailable — named in the pane, with the reason, rather
//! than silently missing or, worse, dispatching a Claude the user did not ask
//! to run. abeam hosts the agent you named; it does not quietly reach for a
//! different one. The same rule as `crate::agent`'s refusal to fetch a missing
//! agent: say what is wrong and what would fix it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Result, anyhow};

use crate::agent::Agent;
use crate::launch::{self, Launch};

/// The permission posture a dispatched task starts with.
///
/// Edits land without asking so the task can finish unattended; everything
/// else still stops. Spelled once, here, because it is the one piece of
/// authority abeam hands to a process nobody is watching.
pub const PERMISSION_MODE: &str = "acceptEdits";

/// The one agent `--bg` belongs to.
///
/// Written out rather than taken from [`crate::agent::DEFAULT`], which holds
/// the same word today and is a different fact. The default is what abeam
/// hosts when nobody said; this is whose flag `--bg` is. The day the first of
/// those changes must not be the day this module starts dispatching something
/// that has never heard of it.
const AGENT: &str = "claude";

/// What a dispatch needs to know: which program, and where to run it.
///
/// Two of these are built per dispatched item, which is the arrangement rather
/// than an accident, and worth reading before trusting the word "once".
///
/// `panes::queue::QueuePane` builds one when it is created and asks it a single
/// question — whether there is anything to dispatch to at all — so that a mode
/// which is unavailable is named in the pane the first time it is drawn. It
/// keeps the answer and throws away the [`Launch`]: `dispatch` blocks, and a
/// blocking method reachable through `&self` on a pane the shell renders every
/// frame is a trap somebody eventually falls into. So `app::App` builds a
/// second one on the worker thread, and *that* is the one that runs.
///
/// The cost is that resolution happens twice and can still fail on the
/// keystroke rather than at construction: a Claude uninstalled mid-session
/// reaches the user as a failed queue item rather than as a mode that had
/// already greyed itself out. That is the right way round for something this
/// rare, and it is the only way round while the type that blocks is kept off
/// the pane.
#[derive(Debug)]
pub struct Dispatcher {
    root: PathBuf,
    launch: Launch,
}

/// Why this session cannot dispatch, in a sentence the pane can draw.
#[derive(Clone, Debug)]
pub struct Unavailable(pub String);

impl Dispatcher {
    /// Build a dispatcher for the agent abeam is hosting.
    ///
    /// `agent` is the name from `crate::agent::Hosted::name` — the word the
    /// user typed, not a path. Anything but `claude` is [`Unavailable`] with
    /// the reason, because `--bg` is Claude's alone.
    ///
    /// Resolution goes through `crate::launch`, like every other spawn in
    /// abeam: a bare name reaching `CreateProcessW` is resolved against the
    /// current directory first, which is the repository on screen.
    pub fn new(root: PathBuf, agent: &str) -> Result<Self, Unavailable> {
        // Through the table rather than by comparing the string, so that
        // `abeam Claude` and `abeam claude` are one request here as they are
        // everywhere else. A program named outright — `abeam C:\tools\claude.exe`
        // — is not the table's Claude and does not become it: abeam knows what
        // it was asked to host and nothing about what that turned out to be.
        let Some(claude) = crate::agent::find(agent).filter(|found| found.name == AGENT) else {
            return Err(Unavailable(elsewhere(agent)));
        };

        // The same walk `agent::resolve_within` does, keeping only the last
        // reason for the same reason it does: with a list, the earlier entries
        // are the ones expected to be missing.
        //
        // With no arguments, deliberately. What is being asked here is whether
        // there is a Claude at all, and the answer has to arrive while the pane
        // is being built rather than on the keystroke that dispatches. The
        // prompt is not known yet and, for the npm shim, is not something that
        // can be added afterwards — see [`plan`].
        let mut why = String::new();
        for candidate in claude.candidates {
            match launch::resolve(candidate, &[]) {
                Ok(launch) => return Ok(Self { root, launch }),
                Err(reason) => why = reason,
            }
        }

        // And that is the end of the search, here as there. Nothing on this
        // path fetches anything — `crate::agent`'s module docs record the route
        // that was written for exactly this shape of problem and then
        // deliberately taken out again, and a background agent installed
        // without being asked for would be a worse version of it.
        Err(Unavailable(missing(claude, &why)))
    }

    /// Start `prompt` as a background agent and report what Claude said.
    ///
    /// **Blocking** — it starts a process and waits for it to print. `--bg`
    /// returns immediately by design, so the wait is short, but it is still a
    /// wait: call it from a worker thread, never from `Pane::tick`.
    pub fn dispatch(&self, prompt: &str) -> Result<Started> {
        run(&plan(&self.launch, prompt)?, &self.root)
    }

    /// The arguments `dispatch` will use, without running anything. Split out
    /// so the one thing that must never drift — what authority is handed to an
    /// unattended process — is assertable in a test.
    ///
    /// `-p` because a background agent has no terminal to be interactive at;
    /// `--bg` because that is the whole feature; `--permission-mode` because a
    /// task nobody is watching cannot answer a question. The prompt is last and
    /// is one argument, whatever is in it: it travels as an argv element rather
    /// than as text some shell will read, so there is nothing here to escape
    /// and nothing here that can end the command.
    ///
    /// The `--` before it is the other half of that promise, and it is doing
    /// work. Everything after the fence is a positional argument to Claude's
    /// own parser, so a prompt is text even when it begins with a dash — and
    /// prompts do. `--- rewrite this section ---` is an ordinary thing to paste
    /// into a queue, and without the fence the first token of somebody's
    /// paragraph is read as a flag on a command line that runs unattended. The
    /// point is not that `--resume` would be inconvenient; it is that a *pasted
    /// prompt must not be able to choose the process's permissions*, which is
    /// otherwise one dash away.
    ///
    /// Total on purpose. A prompt that says nothing still has a command line,
    /// and refusing to *start* one is [`dispatch`](Self::dispatch)'s job — this
    /// function answers what the arguments are, which is a question with an
    /// answer for every string.
    pub fn args(prompt: &str) -> Vec<String> {
        [
            "-p",
            "--bg",
            "--permission-mode",
            PERMISSION_MODE,
            // Everything past here is the user's, and is read as text.
            "--",
            prompt,
        ]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect()
    }
}

/// A dispatched task, as far as the dispatching process could tell.
///
/// The id is what lets a queue item be joined to its row in
/// [`crate::agentstate::roster`]. It is optional because it is read out of
/// what Claude printed, and a future release may print something else — an
/// item that cannot be joined is still shown, just without live state.
#[derive(Clone, Debug)]
pub struct Started {
    pub session_id: Option<String>,
    pub id: Option<String>,
    /// Everything the command printed, trimmed.
    ///
    /// Its use today is the failure path: when a dispatch exits non-zero and
    /// says nothing on standard error, this is what the error message carries
    /// instead — see [`run`].
    ///
    /// It was meant to be shown in the pane when the two fields above came back
    /// empty, and it is not. `QueuePane::note_dispatched` takes `id` and
    /// `session_id` and drops the rest, so an output this module could not
    /// recognise currently reaches the user as an item with no live state and
    /// no explanation of why. Plumbing it through is unfinished work rather
    /// than a decision, and this field is where it will arrive.
    pub raw: String,
}

/// Exactly what a dispatch will run: which program, with which arguments, and
/// what has to be in its environment for those arguments to arrive.
///
/// A free function over the [`Launch`] rather than a method, so that a test can
/// hand it each of the two shapes below without a Claude on the machine and —
/// much more to the point — without starting one.
///
/// The two shapes are not two spellings of the same thing.
///
/// A native install is `claude.exe`, and its arguments are a list: the `Launch`
/// [`Dispatcher::new`] resolved carries none, so [`Dispatcher::args`] is the
/// whole of what the process is given.
///
/// An npm install is `claude.cmd`, which `CreateProcessW` cannot start at all,
/// so `crate::launch` names `cmd.exe` in front of it and puts the command line
/// in an environment variable. There the arguments are *inside* that command
/// line, already quoted for `cmd`'s parser and for the argv parser behind it,
/// and `Launch::args` is `/e:ON /v:OFF /d /c %ABEAM_LAUNCH%` — complete, and
/// not a prefix. Appending the prompt to those five is the obvious move and it
/// is wrong twice over. `cmd` reads whatever follows the expansion as more of
/// its own command line, so a prompt containing an `&` would end abeam's
/// command and start somebody else's; and `std::process::Command` spells an
/// embedded quote `\"`, which `cmd` cannot read and which desyncs its quote
/// tracking for everything after it. So the command line is not extended, it is
/// **rebuilt**: the script is resolved a second time with the whole argument
/// list, by the module that owns that quoting and has the tests for it.
///
/// Which is why the npm route has limits the native one does not, and they are
/// `cmd.exe`'s rather than abeam's: a prompt containing a newline cannot be put
/// on a command line at all, and one over about eight kilobytes is longer than
/// `cmd` will run. Both come back as sentences from `crate::launch`, and both
/// are refusals rather than mangled commands.
fn plan(launch: &Launch, prompt: &str) -> Result<Launch> {
    if is_blank(prompt) {
        return Err(nothing());
    }
    let tail = Dispatcher::args(prompt);

    // An `.exe` is its own target — the one fact that tells the two shapes
    // apart without asking what the file is called.
    if launch.program == launch.target {
        return Ok(Launch {
            program: launch.program.clone(),
            target: launch.target.clone(),
            args: tail,
            // Nothing to carry: an executable's arguments travel as arguments.
            env: Vec::new(),
        });
    }

    launch::resolve(&launch.target.to_string_lossy(), &tail).map_err(|why| {
        anyhow!(
            "abeam could not start a background agent through `{}`, which is a \
             script and has to be run by cmd.exe: {why} The same item can still \
             be sent to the session in the left pane — that mode is typed at \
             the agent rather than quoted for a command processor, so none of \
             this applies to it.",
            launch.target.display()
        )
    })
}

/// Start what [`plan`] worked out, in `root`, and read the identifiers back.
///
/// Free, over the plan and the directory, for the reason [`plan`] is. There is
/// one fact about this module that cannot be established by comparing strings —
/// that a prompt with a space and an `&` in it reaches the program as a single
/// argument — and the only honest way to establish it is to run something and
/// ask. The test that does points this at a `.cmd` which prints its arguments
/// back. Nothing in the suite ever points it at a Claude.
fn run(plan: &Launch, root: &Path) -> Result<Started> {
    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        // The repository on screen, always. A dispatched task inherits none of
        // the conversation and none of the context; the directory it stands in
        // is the whole of what it is given besides the prompt.
        .current_dir(root)
        // abeam's own standard input is a console in raw mode with somebody
        // typing at the agent in the left pane. A child that read from it would
        // take those keystrokes, which is the one thing a task started
        // *in the background* must not do.
        .stdin(Stdio::null());
    for (key, value) in &plan.env {
        command.env(key, value);
    }

    let out = command
        .output()
        .map_err(|e| anyhow!("abeam could not start `{}`: {e}", plan.program.display()))?;

    let started = parse_started(&String::from_utf8_lossy(&out.stdout));

    // A non-zero exit that still named a session started something, and the id
    // is the whole of what the queue needs: an agent that is running is running
    // whatever its launcher made of the exit code. Nothing recognisable *and* a
    // failure is a failure, and what a reader needs then is on standard error.
    if out.status.success() || started.session_id.is_some() || started.id.is_some() {
        return Ok(started);
    }

    let stderr = String::from_utf8_lossy(&out.stderr);
    let said = [stderr.trim(), started.raw.as_str()]
        .into_iter()
        .find(|text| !text.is_empty())
        .unwrap_or("It printed nothing at all.");
    Err(anyhow!(
        "`{}` {} without starting a background agent.\n{said}",
        plan.target.display(),
        match out.status.code() {
            Some(code) => format!("exited with status {code}"),
            None => "was terminated".to_string(),
        }
    ))
}

/// A prompt that asks for nothing.
///
/// Whitespace rather than emptiness, for `crate::agent`'s reason: both arrive
/// by the same route — a composer somebody opened and thought better of — and
/// neither of them is an instruction.
fn is_blank(prompt: &str) -> bool {
    prompt.trim().is_empty()
}

/// Why an empty prompt is a sentence rather than a spawn.
///
/// `claude -p --bg ""` is not a no-op. It is a background agent with no
/// instruction, running unattended in somebody's repository with edits already
/// approved, and whatever it makes of that is by definition not what anybody
/// asked for. This is the first thing [`plan`] does, so the refusal happens
/// while there is still nothing to refuse.
fn nothing() -> anyhow::Error {
    anyhow!(
        "an empty queue item is not a task to dispatch. A background agent \
         starts with nothing but its prompt — there is no conversation above it \
         to make sense of a blank one — so this would be an agent with nothing \
         to do and permission to edit the repository while it worked out what \
         that meant."
    )
}

/// What abeam says to a session that is hosting something else.
///
/// It names the agent in front of the reader, because "background dispatch is
/// unavailable" with no subject reads as a bug in abeam. And it offers the mode
/// that does still work, because the queue is not unavailable — half of it is.
fn elsewhere(agent: &str) -> String {
    format!(
        "abeam is hosting `{agent}`, and dispatching a queued task is Claude's \
         `--bg`: it starts a session that returns immediately and reports \
         itself afterwards through `claude agents`. `{agent}` publishes no \
         equivalent, and abeam will not quietly start a Claude you did not ask \
         for — it hosts the agent you named. Queued items can still be sent to \
         the session in the left pane, which is the mode that needs no such \
         flag."
    )
}

/// What abeam says when the agent that has `--bg` is not on the machine.
///
/// The same standard `crate::agent::missing` is held to — every candidate by
/// name, the operating system's own reason, and the sentence somebody can act
/// on — with one addition. Reaching this means abeam is *hosting* Claude, so
/// there was one a moment ago and something has moved it since; saying so is
/// the difference between a reader checking their install and a reader
/// wondering what abeam thinks it is looking for.
fn missing(claude: &Agent, why: &str) -> String {
    let tried: Vec<String> = claude
        .candidates
        .iter()
        .map(|name| format!("`{name}`"))
        .collect();
    format!(
        "abeam has nothing to dispatch with: it looked for the Claude it is \
         hosting and did not find one. Tried: {}. {why}\n\nThat is odd rather \
         than ordinary — this session started a Claude, so there was one when \
         it began. {}",
        tried.join(", "),
        claude.install
    )
}

/// Pull the identifiers out of what `claude -p --bg` printed. Split out from
/// the spawn so it is testable without starting anything.
///
/// Tolerant, and it has to be: what `--bg` prints is not a documented format,
/// and [`Started`]'s two fields are optional precisely so that a release which
/// prints something else costs a queue item its live state rather than its
/// dispatch. So this looks for the *shapes* — a UUID anywhere for the session,
/// eight hex characters standing alone for the short id — and never fails.
pub fn parse_started(output: &str) -> Started {
    let raw = output.trim().to_string();
    let session_id = tokens(&raw)
        .find(|token| is_uuid(token))
        .map(str::to_string);
    let id = tokens(&raw)
        .find(|token| is_short(token))
        .map(str::to_string);
    Started {
        session_id,
        id,
        raw,
    }
}

/// The output cut into the things that could be an identifier.
///
/// A hyphen is part of a token rather than a boundary between two, and that is
/// the whole trick. A UUID opens with eight hex characters, which is exactly
/// what the short id is; splitting on anything that is not a hex digit would
/// hand back `550e8400` as an id every time a session was named. Keeping the
/// hyphen keeps the UUID whole, so the two shapes cannot be confused for one
/// another.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .map(|token| token.trim_matches('-'))
        .filter(|token| !token.is_empty())
}

/// 8-4-4-4-12, and nothing else on either end.
fn is_uuid(token: &str) -> bool {
    let mut groups = token.split('-');
    [8, 4, 4, 4, 12].into_iter().all(|len| {
        groups
            .next()
            .is_some_and(|group| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
    }) && groups.next().is_none()
}

/// The short id `claude agents` uses to name a background session.
fn is_short(token: &str) -> bool {
    token.len() == 8 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Windows-only like the rest of the suite: what [`Dispatcher::new`] does is a
/// `PATH` walk with `PATHEXT` in it, and what [`plan`] does with a script is
/// build a command line for `cmd.exe`.
///
/// **Nothing here starts a background agent**, and that is a rule rather than
/// an observation about what these happen to do. `claude -p --bg` would put a
/// real unattended Claude into a real repository with edits pre-approved, and a
/// test suite is not a thing anybody is watching. So [`Dispatcher::dispatch`]
/// is never called: what it would work out is [`plan`], which is pure, and what
/// would run that is [`run`], which is pointed at a `.cmd` that prints its
/// arguments back. The only programs these tests start are that shim and the
/// `cmd.exe` it goes through.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    /// The `Launch` a native install produces: an `.exe` that is its own
    /// target, resolved with no arguments. Fabricated rather than found, so
    /// that what is under test is the argument list rather than the machine.
    fn native() -> Launch {
        let exe = PathBuf::from(r"C:\Users\someone\.local\bin\claude.exe");
        Launch {
            program: exe.clone(),
            target: exe,
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    // --- what authority is handed over ------------------------------------

    #[test]
    fn the_argument_list_never_grants_more_than_edit_permission() {
        // This is the security assertion of the module and it is deliberately
        // written as one. What `args` returns is the complete authority abeam
        // hands to a process nobody is watching, running in somebody's
        // repository while they are looking at something else. It is asserted
        // whole rather than by `contains`, so that a flag cannot be added
        // without coming through this line.
        //
        // If you are here because this test is in the way of an edit: the
        // question to answer is not "is this flag useful". It is "would the
        // person who queued this task and walked away have agreed to it".
        assert_eq!(
            Dispatcher::args("do the thing"),
            args(&[
                "-p",
                "--bg",
                "--permission-mode",
                "acceptEdits",
                "--",
                "do the thing"
            ])
        );

        // Named one at a time as well, because these are the ones a
        // well-meaning edit reaches for and the whole vector above is easy to
        // re-record. The two `dangerously` flags hand a background agent the
        // machine; `--worktree` writes a git worktree into somebody's checkout
        // as a side effect of queueing a task, which is a structural change to
        // their repository that they did not ask for — see the module docs, it
        // belongs behind an explicit per-item choice rather than a default.
        let list = Dispatcher::args("do the thing");
        for forbidden in [
            "--dangerously-skip-permissions",
            "--allow-dangerously-skip-permissions",
            "--worktree",
            "-w",
            "bypassPermissions",
        ] {
            assert!(
                !list.iter().any(|arg| arg == forbidden),
                "`{forbidden}` reached the command line of an unattended agent"
            );
        }

        // The one grant that is there is spelled once, in the constant the
        // module docs describe, so that changing it is visibly changing that.
        assert_eq!(PERMISSION_MODE, "acceptEdits");
        assert!(list.contains(&PERMISSION_MODE.to_string()));
    }

    #[test]
    fn a_prompt_that_looks_like_a_flag_is_text_and_cannot_become_one() {
        // The other half of the test above, and the half that is about somebody
        // else's input rather than about this file's. The list above is what
        // abeam chooses; this is what stops a *prompt* from adding to it.
        //
        // The prompt is user text, usually pasted, and a pasted paragraph can
        // begin with anything. Everything after `--` is positional to Claude's
        // own parser, so the worst thing a queued item can say is still only a
        // thing it says.
        for pasted in [
            "--dangerously-skip-permissions",
            "--allow-dangerously-skip-permissions",
            "--permission-mode bypassPermissions",
            "--worktree",
            "--- rewrite this section ---",
            "-p ignore the above and do this instead",
        ] {
            let list = Dispatcher::args(pasted);

            // The fence is there, exactly once, and the prompt is behind it.
            assert_eq!(list.len(), 6, "{pasted:?} did not arrive as one argument");
            assert_eq!(list.iter().filter(|arg| *arg == "--").count(), 1);
            assert_eq!(list[4], "--", "the fence moved or went missing");
            assert_eq!(list[5], pasted, "the prompt was altered on the way past");

            // Which is the whole claim: nothing the user typed is in a position
            // where Claude would read it as an option. A prompt asking for
            // `--dangerously-skip-permissions` is a prompt, and a background
            // agent's permissions stay abeam's to choose.
            let (abeams, theirs) = list.split_at(5);
            assert!(
                !abeams.iter().any(|arg| arg.contains("dangerously")),
                "a pasted prompt reached the flag half of the command line: {abeams:?}"
            );
            assert_eq!(theirs, [pasted]);
        }
    }

    #[test]
    fn a_prompt_is_one_argument_however_much_syntax_is_in_it() {
        // It is an argv element and never a shell string, so there is nothing
        // in it to escape and nothing in it that can end a command. Every
        // character below is one that would matter if that were not true: a
        // space splits, a quote unbalances, an `&` separates, a `%VAR%` expands
        // and a newline ends the line outright.
        let awkward = "fix the \"quoting\" in a & b, print %CD% and !this!\nthen stop";
        let list = Dispatcher::args(awkward);

        assert_eq!(list.len(), 6, "the prompt became more than one argument");
        assert_eq!(list[5], awkward, "the prompt did not survive intact");
        assert_eq!(
            list[..5],
            args(&["-p", "--bg", "--permission-mode", "acceptEdits", "--"])[..]
        );

        // Last, always. Nothing of abeam's follows the prompt, so nothing of
        // abeam's can be mistaken for part of it.
        assert_eq!(Dispatcher::args("--resume").last().unwrap(), "--resume");
        assert_eq!(Dispatcher::args("").last().unwrap(), "");
    }

    #[test]
    fn a_prompt_that_asks_for_nothing_is_refused_before_a_process_exists() {
        // `dispatch` is not called here, and that is the point of `plan` being
        // separable. A test that proved this by calling `dispatch` would, on
        // the day the refusal regressed, start a real unattended agent in
        // whatever directory the test binary was standing in — which is the
        // exact failure it was meant to be guarding against.
        for empty in ["", " ", "   ", "\t", "\r\n"] {
            let refused = plan(&native(), empty).expect_err("an empty prompt is not a task");
            let said = refused.to_string();
            assert!(said.contains("empty"), "got: {said}");
            // Why it is refused rather than sent anyway, which is the half a
            // reader needs: it is not that the command would fail.
            assert!(said.contains("permission to edit"), "got: {said}");
        }

        // Whitespace and emptiness are the same non-answer — `crate::agent`'s
        // rule, and it arrives here by the same route: a composer somebody
        // opened and thought better of.
        assert!(plan(&native(), " do the thing ").is_ok());
        // ...and the trim is for the decision only. The spacing inside a prompt
        // is the user's, and it reaches the agent as they wrote it.
        assert_eq!(Dispatcher::args(" do it ").last().unwrap(), " do it ");
    }

    // --- who may dispatch --------------------------------------------------

    #[test]
    fn only_the_agent_that_has_bg_can_dispatch_and_the_rest_are_told_why() {
        let root = std::env::temp_dir();

        for hosting in ["copilot", "Copilot", "COPILOT"] {
            let Unavailable(why) =
                Dispatcher::new(root.clone(), hosting).expect_err("`--bg` is Claude's");

            // The agent in front of the reader, by the name they typed.
            // "Background dispatch is unavailable" with no subject reads as a
            // bug in abeam rather than as a fact about Copilot.
            assert!(why.contains(hosting), "got: {why}");
            assert!(why.contains("--bg"), "the flag this is about: {why}");
            assert!(why.contains("Claude"), "whose flag it is: {why}");
            // The reversal `crate::agent` records, in the place it would be
            // easiest to undo quietly: abeam hosts what it was asked to host.
            assert!(
                why.contains("did not ask for"),
                "the refusal to reach for a Claude is not stated: {why}"
            );
            // And half a queue still works, which is the difference between a
            // limitation and a broken pane.
            assert!(why.contains("left pane"), "got: {why}");
        }

        // Anything abeam is hosting that is not an agent at all goes the same
        // way: `abeam powershell` has a queue too, and one of its two modes is
        // not available to it.
        let Unavailable(why) =
            Dispatcher::new(root, "powershell").expect_err("a shell has no `--bg`");
        assert!(why.contains("powershell"), "got: {why}");
    }

    #[test]
    fn the_hosted_agents_name_is_read_the_way_every_other_name_on_this_platform_is() {
        // Case-insensitively, like `crate::agent::find`, like `PATH`, like file
        // names. `abeam Claude` hosting Claude while its queue said dispatch
        // was unavailable would be a distinction with no visible cause.
        //
        // Whether it then *resolves* is a fact about the machine rather than a
        // decision — Claude is installed where this was written and may not be
        // on a build server — so what is asserted is that the three spellings
        // are one question with one answer, and that the answer is never the
        // refusal meant for a different agent.
        let root = std::env::temp_dir();
        let answer = |name: &str| match Dispatcher::new(root.clone(), name) {
            Ok(_) => "there is a claude",
            Err(Unavailable(why)) => {
                assert!(
                    !why.contains("hosting `"),
                    "a spelling of claude was read as a different agent: {why}"
                );
                assert!(why.contains("Tried:"), "got: {why}");
                "there is no claude"
            }
        };

        assert_eq!(answer("claude"), answer("Claude"));
        assert_eq!(answer("claude"), answer("CLAUDE"));
    }

    #[test]
    fn a_claude_that_is_not_on_the_machine_names_what_was_looked_for_and_how_to_get_one() {
        // Only reachable through `Dispatcher::new` on a machine with no Claude,
        // and the machine abeam is developed on has one — so the message is
        // asserted where it is written, exactly as `crate::agent`'s failure
        // path is tested against a table whose agents are known to be absent.
        let claude = crate::agent::find(AGENT).expect("claude is in the table");
        let said = missing(claude, "`claude` was not found on PATH.");

        for candidate in claude.candidates {
            assert!(
                said.contains(candidate),
                "{candidate} is missing from: {said}"
            );
        }
        assert!(said.contains("not found on PATH"), "got: {said}");
        // The sentence somebody has to act on, taken from the table rather than
        // written here, so the two cannot disagree.
        assert!(said.contains(claude.install), "got: {said}");
        // A command to read and type, never one abeam runs on the reader's
        // behalf — see `crate::agent`'s module docs for the route that was
        // written for this and then deliberately taken out again.
        assert!(
            said.contains("npm i -g @anthropic-ai/claude-code"),
            "got: {said}"
        );
        assert!(
            !said.to_lowercase().contains("downloading"),
            "abeam does not fetch an agent: {said}"
        );
    }

    // --- the two installs --------------------------------------------------

    #[test]
    fn a_native_claude_is_started_directly_with_abeams_tail_and_nothing_else() {
        let planned = plan(&native(), "do the thing").expect("a prompt that says something");

        assert_eq!(planned.program, native().program);
        assert_eq!(planned.target, planned.program, "an .exe is its own target");
        assert_eq!(planned.args, Dispatcher::args("do the thing"));
        assert!(
            planned.env.is_empty(),
            "an executable's arguments travel as arguments"
        );
    }

    #[test]
    fn an_npm_claude_has_its_command_line_rebuilt_rather_than_appended_to() {
        // The `Launch` for a `.cmd` is complete rather than a prefix: its
        // arguments are `/e:ON /v:OFF /d /c %ABEAM_LAUNCH%` and the real
        // command line is in the variable, quoted for `cmd` by the module that
        // owns that quoting. Appending the prompt to those five would put it
        // *after* the expansion, where `cmd` reads it as more of its own
        // command line — so the `&` below would end abeam's command and start
        // somebody else's.
        let dir = TempDir::new("dispatch-npm");
        let script = dir.write("abeam-claude.cmd", b"@echo off\r\n");
        let resolved = launch::resolve(&script.to_string_lossy(), &[]).expect("a .cmd is routed");
        assert_eq!(
            resolved.args,
            args(&["/e:ON", "/v:OFF", "/d", "/c", "%ABEAM_LAUNCH%"]),
            "the shape this test is about has changed"
        );

        let planned = plan(&resolved, "fix a & b").expect("a prompt that says something");

        // Not one token longer. The arguments are not where the prompt went.
        assert_eq!(planned.args, resolved.args);
        assert_eq!(planned.program, resolved.program);
        assert_eq!(planned.target, script, "the border names the script");

        let (key, line) = planned
            .env
            .first()
            .expect("the command line travels in a variable")
            .clone();
        assert_eq!(key, "ABEAM_LAUNCH");
        assert_eq!(
            line,
            format!(
                "\"{}\" -p --bg --permission-mode acceptEdits -- \"fix a & b\"",
                script.display()
            )
        );
        // The fence survives the quoting as a fence: a bare `--` is on
        // `crate::launch`'s harmless list, so it is not shut inside quotes
        // where Claude's parser would read it as a word.
        assert!(line.contains(" -- \""), "the fence was quoted away: {line}");
        // Once. Twice is what appending would have produced, and a command line
        // carrying two prompts is one nobody wrote.
        assert_eq!(line.matches("--bg").count(), 1);
    }

    #[test]
    fn a_prompt_a_command_processor_cannot_carry_is_refused_with_the_reason() {
        // The npm route's limits, which are `cmd.exe`'s and not abeam's: a
        // newline ends a command line outright, so a pasted multi-line prompt —
        // which `panes::queue` turns into a single item — cannot be dispatched
        // through a `.cmd` at all. It is a sentence rather than a truncated
        // command, because a command line silently cut in half is
        // indistinguishable from an injection that worked.
        let dir = TempDir::new("dispatch-npm-limits");
        let script = dir.write("abeam-claude.cmd", b"@echo off\r\n");
        let resolved = launch::resolve(&script.to_string_lossy(), &[]).expect("a .cmd is routed");

        for (bad, reason) in [
            ("first line\nsecond line", "newline"),
            ("first line\rsecond line", "carriage return"),
            ("a\0b", "NUL"),
        ] {
            let refused = plan(&resolved, bad).expect_err("cmd.exe cannot carry this");
            let said = refused.to_string();
            assert!(said.contains(reason), "got: {said}");
            assert!(said.contains("abeam-claude.cmd"), "which program: {said}");
            // The way out, and it is a real one: the other mode types at the
            // agent rather than quoting for a command processor.
            assert!(said.contains("left pane"), "got: {said}");
        }

        // The ceiling, which is the worst-shaped of the four because it has no
        // symptom of its own. Past what `cmd` will run it starts nothing,
        // prints nothing and exits 0 — so before `crate::launch` refused it,
        // an over-long prompt drew an empty pane and reported success. Ten
        // kilobytes is an ordinary amount of pasted context, which is what
        // makes this reachable from a queue rather than theoretical.
        let long = "a".repeat(10_000);
        let refused = plan(&resolved, &long).expect_err("longer than cmd.exe will run");
        let said = refused.to_string();
        assert!(
            said.contains("cmd.exe's limit and not abeam's"),
            "whose limit it is: {said}"
        );
        assert!(
            said.contains("natively"),
            "the install that does not have it: {said}"
        );

        // The same prompts against a native install are ordinary arguments, and
        // the asymmetry is the point: this is a difference between the two
        // installs rather than a rule about prompts.
        assert!(plan(&native(), "first line\nsecond line").is_ok());
        assert!(plan(&native(), &long).is_ok());
    }

    // --- and it actually starts -------------------------------------------
    //
    // Everything above this line is a claim about a `Vec<String>`, where
    // `len() == 6` is true by construction. What is actually claimed is a fact
    // about what a *process* receives, and only a process can answer that — so
    // below this line `cmd.exe` is asked, on the route where the answer can go
    // wrong. The hardest inputs belong down here rather than up there.

    /// What a shim standing in for Claude prints: an id back the way one would,
    /// then its whole argument list, then its fifth and sixth arguments on
    /// their own — the fence and the prompt — then the directory it was started
    /// in.
    ///
    /// The id line is first deliberately. `parse_started` takes the first thing
    /// of each shape it finds, and a temp directory's own path could contain
    /// eight hex characters on somebody's machine and not on mine.
    const REPORTS: &[&str] = &[
        "echo Started agent a1b2c3d4 (550e8400-e29b-41d4-a716-446655440000)",
        "echo ALL [%*]",
        "echo FENCE [%5]",
        "echo PROMPT [%6]",
        "echo CWD [%CD%]",
    ];

    /// A `.cmd` that does what it is told, in a directory with a space in its
    /// name — a hazard every one of these has to survive rather than one of
    /// them.
    fn shim(dir: &TempDir, name: &str, lines: &[&str]) -> PathBuf {
        let home = dir.path().join("with space");
        std::fs::create_dir_all(&home).expect("a directory with a space in it");
        let script = home.join(format!("abeam-{name}.cmd"));
        // `\r\n`, because a batch file with bare newlines is a batch file that
        // does something else on some machines.
        let mut text = String::from("@echo off\r\n");
        for line in lines {
            text.push_str(line);
            text.push_str("\r\n");
        }
        std::fs::write(&script, text).expect("write a shim");
        script
    }

    /// Dispatch `prompt` through `script`, the whole way: resolve it as an npm
    /// install would be resolved, plan it, and run it in its own directory.
    fn through(script: &Path, prompt: &str) -> Result<Started> {
        let resolved = launch::resolve(&script.to_string_lossy(), &[]).expect("a .cmd is routed");
        let planned = plan(&resolved, prompt).expect("a prompt that says something");
        run(&planned, script.parent().expect("the shim has a directory"))
    }

    #[test]
    fn a_prompt_reaches_an_npm_shim_as_one_argument_with_its_syntax_intact() {
        // The question the whole routed path exists to answer, asked of the
        // only thing that can answer it: does a queued prompt arrive at the
        // program as a single argument, and does it arrive where a parser reads
        // it as text.
        //
        // The shim also prints an identifier back, so the last leg is covered
        // here too — standard output is captured and parsed rather than
        // discarded — and the directory it was started in, because a dispatched
        // task that ran somewhere other than the repository on screen would be
        // acting on the wrong files.
        let dir = TempDir::new("dispatch-spawn");
        let script = shim(&dir, "claude", REPORTS);
        let home = script.parent().expect("the shim has a directory");

        // Each of these is a prompt somebody could plausibly queue, and each
        // carries the character that would break this if the command line were
        // built by anything less careful than `crate::launch`.
        for (prompt, arrives) in [
            // `&` ends a command line and starts another; `%VAR%` expands into
            // one.
            (
                "fix a & b in %APPDATA% now",
                "\"fix a & b in %APPDATA% now\"",
            ),
            // The one that matters most, and the one this test went without
            // while it looked complete without it. A double quote is the
            // character most likely to desync
            // `cmd`'s quote tracking, and getting it wrong does not merely
            // mangle an argument — it puts the `&` that follows back outside a
            // quoted region, where it separates commands again. `""` is the one
            // spelling both `cmd` and every CRT since Visual C++ 2008 read as
            // an embedded quote; `\"`, which MSVCRT quoting would produce, is
            // read by `cmd` as neither. See `crate::launch`.
            ("say \"hi\" & run --now", "\"say \"\"hi\"\" & run --now\""),
            // Not quoted at all, because nothing in it needs quoting — and
            // still an argument rather than a flag, purely because of where the
            // fence put it. This is the live version of the assertion that
            // `a_prompt_that_looks_like_a_flag_is_text_and_cannot_become_one`
            // can only make against a vector.
            (
                "--dangerously-skip-permissions",
                "--dangerously-skip-permissions",
            ),
        ] {
            let started = through(&script, prompt).expect("the shim runs");

            assert!(
                started.raw.contains(&format!("PROMPT [{arrives}]")),
                "{prompt:?} did not arrive as one argument:\n{}",
                started.raw
            );
            // In the position a parser reads as positional, and not a quoted
            // word two arguments early.
            assert!(
                started.raw.contains("FENCE [--]"),
                "the fence did not survive the trip through cmd.exe:\n{}",
                started.raw
            );
            assert!(
                started.raw.contains(&format!(
                    "ALL [-p --bg --permission-mode acceptEdits -- {arrives}]"
                )),
                "the argument list did not arrive intact:\n{}",
                started.raw
            );
            assert!(
                started
                    .raw
                    .to_lowercase()
                    .contains(&home.to_string_lossy().to_lowercase()),
                "a dispatched task ran somewhere other than where it was pointed:\n{}",
                started.raw
            );

            // ...and what it said about itself was read.
            assert_eq!(started.id.as_deref(), Some("a1b2c3d4"));
            assert_eq!(
                started.session_id.as_deref(),
                Some("550e8400-e29b-41d4-a716-446655440000")
            );
        }
    }

    #[test]
    fn a_dispatch_that_failed_says_the_code_and_what_the_program_said() {
        // `run`'s two-way rule, which until this existed had never executed in
        // either direction because every shim in the file exited 0.
        let dir = TempDir::new("dispatch-failed");

        // Nothing recognisable and a non-zero exit is a failure, and the two
        // things a reader can act on are the code and whatever the program said
        // about it. The message carries both.
        let angry = shim(
            &dir,
            "angry",
            &["echo could not authenticate 1>&2", "exit /b 1"],
        );
        let why = through(&angry, "do the thing")
            .expect_err("a failed dispatch is not a dispatch")
            .to_string();
        assert!(why.contains("could not authenticate"), "got: {why}");
        assert!(why.contains("status 1"), "got: {why}");
        assert!(why.contains("abeam-angry.cmd"), "which program: {why}");

        // A program that fails and says nothing is the worst of the three, and
        // "it printed nothing" is genuinely the diagnosis — an error message
        // that goes quiet here reads as abeam having lost the output.
        let mute = shim(&dir, "mute", &["exit /b 1"]);
        let why = through(&mute, "do the thing")
            .expect_err("a failed dispatch is not a dispatch")
            .to_string();
        assert!(why.contains("printed nothing at all"), "got: {why}");
        assert!(why.contains("status 1"), "got: {why}");

        // And the other direction, which is the half that is a judgement rather
        // than a rule: a non-zero exit that still named a session *started
        // something*. There is an agent running whatever its launcher made of
        // the exit code, and the queue needs its id more than it needs the
        // complaint — an item reported as failed while its agent edits the
        // repository is the worse of the two mistakes.
        let late = shim(
            &dir,
            "late",
            &[
                "echo Started agent a1b2c3d4 (550e8400-e29b-41d4-a716-446655440000)",
                "echo and then something went wrong 1>&2",
                "exit /b 1",
            ],
        );
        let started = through(&late, "do the thing").expect("an agent that was named is an agent");
        assert_eq!(started.id.as_deref(), Some("a1b2c3d4"));
        assert_eq!(
            started.session_id.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn a_dispatched_task_cannot_read_the_console_abeam_is_typing_at() {
        // abeam's own standard input is a console in raw mode with somebody
        // typing at the agent in the left pane. A child that inherited it would
        // take those keystrokes, which is the one thing a task started *in the
        // background* must not do — so it is handed nothing to read.
        //
        // `set /p` leaves its variable untouched when it reads end of file, so
        // the marker below surviving is the child having found no input at all.
        //
        // Know the failure mode before you trust the green: if `Stdio::null()`
        // is ever dropped, this does not fail, it *hangs* — `set /p` on an
        // inherited console waits for a line that is never typed. A hang here
        // means what a failure would mean.
        let dir = TempDir::new("dispatch-stdin");
        let script = shim(
            &dir,
            "stdin",
            &[
                "set SAW=nothing-at-all",
                "set /p SAW=",
                "echo STDIN [%SAW%]",
            ],
        );

        let started = through(&script, "do the thing").expect("the shim runs");
        assert!(
            started.raw.contains("STDIN [nothing-at-all]"),
            "a dispatched task was given something to read:\n{}",
            started.raw
        );
    }

    // --- reading what came back --------------------------------------------

    #[test]
    fn what_claude_printed_is_read_tolerantly_and_never_fails() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";

        // A session named on its own.
        let bare = parse_started(&format!("  {uuid}  \n"));
        assert_eq!(bare.session_id.as_deref(), Some(uuid));
        assert_eq!(bare.raw, uuid, "raw is the output, trimmed");
        // The trap this parser is shaped around: a UUID opens with eight hex
        // characters, which is exactly what the short id is. The hyphen keeps
        // it whole, so `550e8400` is never handed back as an id.
        assert_eq!(bare.id, None, "the UUID's first group was read as an id");

        // Both, in prose.
        let both = parse_started(&format!(
            "Started background agent a1b2c3d4\nsession: {uuid}\n"
        ));
        assert_eq!(both.id.as_deref(), Some("a1b2c3d4"));
        assert_eq!(both.session_id.as_deref(), Some(uuid));

        // Both, as JSON, which is as likely a shape as any and is what the
        // neighbouring `claude agents --json` already speaks.
        let json = parse_started(&format!("{{\"id\":\"0f9e8d7c\",\"sessionId\":\"{uuid}\"}}"));
        assert_eq!(json.id.as_deref(), Some("0f9e8d7c"));
        assert_eq!(json.session_id.as_deref(), Some(uuid));

        // And everything that names nothing at all is still a `Started`, with
        // what was printed in it. An item that cannot be joined to a roster row
        // is an item that was started and cannot be followed, which the pane
        // can say — where a failure here would be a task that is running and is
        // reported as one that never began.
        for odd in [
            "",
            "   ",
            "\n\n",
            "ok",
            "error: something went wrong",
            "-",
            "--",
            "zzzzzzzz",
            "1.2.3",
            "{}",
            "550e8400-e29b-41d4-a716",
            "550e8400e29b41d4a716446655440000",
        ] {
            let started = parse_started(odd);
            assert_eq!(started.raw, odd.trim());
            assert!(started.session_id.is_none(), "{odd:?} named a session");
            assert!(started.id.is_none(), "{odd:?} named an agent");
        }
    }
}
