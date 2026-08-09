//! A second Claude in the right pane, which may read and may not write.
//!
//! This is the third way abeam starts an agent, and the three differ in what
//! they are *for* rather than in mechanism. `crate::agent` hosts the session
//! you are working in: a pty, interactive, the whole conversation.
//! `crate::dispatch` starts work that outlives the keystroke: `--bg`, detached,
//! no terminal, deliberately in a session of its own. This one is neither. It
//! is a reader you can ask a question, tied to the window it belongs to and
//! killed with it, and the only one of the three that cannot change a file.
//!
//! ## Why the prompt is not on a command line
//!
//! `crate::dispatch`'s module docs spend a page on what a prompt costs as an
//! argv element, and the worst of it is not a length: **a newline cannot be put
//! on a `cmd.exe` command line in any form**, so on a Windows npm install a
//! multi-line prompt is refused outright. A question is multi-line more often
//! than a task is. That refusal is the right one there and would be unbearable
//! here.
//!
//! So the prompt never touches a command line. It is written to the child's
//! standard input as one line of JSON, with the newline inside a quoted string
//! where it is two ordinary bytes. There is no length limit, nothing that ends
//! a command, and nothing to escape but JSON itself — which `serde_json` does.
//! That is the whole reason this shape was chosen over the one next door.
//!
//! It is also *not* the experiment `dispatch` declines to run. The open
//! question there is whether `--bg` drains standard input before it detaches;
//! nothing here detaches, and the pipe stays open for the life of the pane.
//!
//! ## What was actually observed
//!
//! None of the following is a published contract. Claude's CLI reference
//! documents that these flags exist, and the SDK layered over them; the wire
//! format is not specified anywhere. This is one run, against **Claude Code
//! 2.1.222 on Windows, on 2026-08-05**, and it is written down so that the day
//! it stops being true somebody can see what it used to be:
//!
//! - Two turns down one stdin held a conversation — the second answer recalled
//!   the first. One `result` came back per turn. The child exited 0 when stdin
//!   closed.
//! - `--tools "Read,Grep,Glob"` was honoured exactly: `system`/`init` reported
//!   `["Glob","Grep","Read"]` and nothing else. **That flag is the read-only
//!   guarantee.** A permission mode is a mode; this is the tool list.
//! - `--session-id` was accepted and echoed on every line.
//! - **`total_cost_usd` is that turn's cost and not a running total**, which
//!   matters because the pane both prints it per turn and adds them up for the
//!   title, and those two are only both right on this reading. Two turns down
//!   one stdin came back $0.0544 and $0.0589; a cumulative field would have
//!   made the second ≈$0.113. `num_turns` resets with it — 3 then 2 on a
//!   second run — so neither field describes the session. It is recorded here
//!   because the name says the opposite and the company it keeps on the
//!   `result` line says the opposite, so the next reader will doubt it too.
//! - The child wrote `~/.claude/sessions/<pid>.json` with `"kind":"interactive"`
//!   and abeam's own `cwd`. See the section below, which is the most important
//!   thing in this file.
//!
//! ## What a second probe found, on 2026-08-09
//!
//! Two more runs, same version, driven the same way — one question each, both
//! with [`AskSession::args`]'s list as it stands. They were run because the
//! pane built on the first probe turned out to be showing people something
//! wrong, and every one of these is a fact that was being guessed at before:
//!
//! - **A turn is mostly not answer.** One ordinary question produced **123
//!   lines, ten of which were text**, over **30.7 seconds** — `duration_ms` on
//!   the `result` line — during which the child ran six tool calls. Time to the
//!   first fragment was 2.4 seconds. So the pane's old picture of a turn (a
//!   question, then text) described a tenth of one, and the other nine tenths
//!   were a blank space under the question for half a minute.
//! - **`result` carries the last text block, not the turn.** The two runs
//!   streamed 1144 and 2449 characters of text and reported 944 and 2278 on the
//!   `result` line. Both said something before reaching for a tool, and both
//!   times `result` began after the last tool call. The pane used to take
//!   `result` as the better copy and overwrite what had streamed, which deleted
//!   the opening of every answer that used a tool.
//! - **`rate_limit_event` is mostly not a rate limit.** Its shape is
//!   `rate_limit_info: {status, resetsAt, rateLimitType, overageStatus, …}`
//!   with `status` one of `allowed`, `allowed_warning`, `rejected` — and a
//!   perfectly healthy session emits one saying `allowed`. abeam was drawing a
//!   warning for it. `crate::ask::proto::limited` is that correction, and has
//!   the captured line.
//! - **`--permission-mode plan` was actively harmful here**, for reasons that
//!   are [`PERMISSION_MODE`]'s to give.
//! - The tool calls are on the `assistant` messages with their inputs complete,
//!   and reasoning blocks announce themselves on `content_block_start`. Both
//!   were being dropped. They are what the pane now draws while it waits.
//! - **Hooks still ran.** `--strict-mcp-config` is about MCP servers and does
//!   not disable them, and neither does a tool list: a hook is an arbitrary
//!   shell command the session runs on its own events, gated by nothing on
//!   [`AskSession::args`]'s list. That observation is what [`SETTING_SOURCES`]
//!   was added for — the repository's own `.claude/settings.json` no longer
//!   loads, so a checkout cannot configure the reader abeam started to read it.
//!   The user's own hooks, in `~/.claude`, still run. The claim this module
//!   makes is therefore "it has no tool that can change a file, and nothing the
//!   *repository* configured runs in it", which is narrower than "nothing runs
//!   in it" and is the true one.
//!
//! ## The read-only claim, tested by asking it to write
//!
//! The list above says what the child was *given*. A second run, on the same
//! day and with [`AskSession::args`]'s list exactly as it stands, asked it the
//! other question: it was pointed at a throwaway directory holding one file and
//! told to create a second one.
//!
//! It could not. The directory still held one file afterwards, and the child
//! said so itself — "the only tools available this session are Glob, Grep and
//! Read". `mcp_servers` came back `[]`, which is the half of `--strict-mcp-config`
//! worth having: an MCP tool would arrive in `tools` as `mcp__server__tool` and
//! nothing here filters that list, so the flag mattering is a fact to check
//! rather than assume. It read the existing file correctly in the same session,
//! so this is a session that reads and cannot write, rather than one that is
//! merely broken.
//!
//! **One nuance, and it is the reason this paragraph is not shorter: the model
//! still emitted a `Write` tool call.** Being given three tools did not stop it
//! *asking* for a fourth; what stopped the file appearing is that nothing
//! executed the call. So the guarantee is enforcement, not the model's
//! cooperation — which is the right way round, and is why [`TOOLS`] rather than
//! any prompt is what this module points at when it says "read-only". It also
//! means abeam cannot see the attempt: `permission_denials` on the `result`
//! line came back empty, so there is nothing here to show a reader and nothing
//! to count. A pane that promised to report refused writes would be promising
//! something this format does not carry.
//!
//! ## The session record, and the bug it would otherwise be
//!
//! A print-mode child is **indistinguishable from the hosted agent by `kind`**.
//! It writes an interactive record, in abeam's `cwd`, started after abeam did —
//! which is precisely the shape `crate::agentstate`'s search is looking for.
//! Its fallback takes the newest interactive record in the repository when
//! clock skew leaves nothing else, and the wrong answer there is a wrong
//! `idle`, and a wrong `idle` is a queued prompt typed into an agent that is
//! mid-turn. That is the one mistake `crate::panes::queue` exists to prevent.
//!
//! So abeam chooses the child's session id rather than letting it invent one,
//! and tells the probe to disown it. The id is the only field that separates
//! the two records; the pid does not, because a pid is reused.
//!
//! ## Why this child is on a leash and the dispatched one is not
//!
//! `crate::dispatch` goes out of its way to *detach*: a `setsid` on Unix, so
//! that a `kill %1` at the shell abeam was started from, or the `SIGHUP` a
//! closed terminal sends its session, does not sweep up a task that was started
//! precisely to outlive them. **Nothing here does any of that, and the omission
//! is the decision.** This child is a reader attached to a pane. It has no work
//! of its own to finish, nothing to write, and nothing to report to anybody
//! afterwards — so a copy of it left running once abeam is gone is a process
//! burning somebody's quota to answer a question nobody will ever see. It is
//! not a task that survives its window; it is part of the window.
//!
//! Two things follow, one on each platform, and they point the same way for
//! once. On Unix the child stays in abeam's process group and session, so every
//! signal aimed at the job the user started reaches it too — which is what is
//! wanted. On Windows it inherits abeam's console, so `CTRL_CLOSE_EVENT` on a
//! closed window reaches it as well. [`AskSession`]'s `Drop` is the ordinary
//! route and those are the backstops for the exits a `Drop` never runs on.
//!
//! ## What abeam cannot promise about the grandchild
//!
//! One gap, named rather than left to be discovered. On a Windows npm install
//! `crate::launch` starts `claude.cmd` through `cmd.exe`, so the process abeam
//! holds is the interpreter and the Claude is *its* child. `Drop` kills what it
//! holds, and killing `cmd.exe` does not kill a node underneath it — the same
//! limitation `abeam_pty` answers with a job object for the pane on the left,
//! which is machinery this module does not have.
//!
//! What closes it in practice is the observation above: **the child exits 0
//! when its standard input closes**, and `Drop` drops the write end of that
//! pipe before it kills anything. So an orphaned node reaches end of file on
//! the next read and leaves of its own accord. That is a mitigation resting on
//! observed behaviour rather than a guarantee resting on the operating system,
//! which is exactly the distinction worth writing down: if this ever needs to
//! be a guarantee, the answer is `abeam_pty`'s job object rather than a longer
//! kill.

mod proto;

use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread;

use anyhow::{Result, anyhow};

// The module itself is named where it is used rather than imported here, and
// that is a fact about Unix rather than a style: the only non-test caller of
// `launch::resolve` below is inside a `#[cfg(windows)]` arm, so an import at
// the top of this file would be an unused one on every Linux build.
use crate::launch::Launch;

pub use proto::{AskEvent, Step, new_session_id};

/// The tools the child is given, and the load-bearing half of the read-only
/// claim.
///
/// An allowlist over the built-in set: what is not named here does not exist
/// for that session, so there is no `Write`, no `Edit`, no `Bash` to permit or
/// refuse. Stated as a constant rather than built at the call site so that the
/// one thing which must never drift — what authority a second agent is handed —
/// is a line a test can assert against, exactly as `crate::dispatch` does for
/// the authority *it* hands out.
///
/// **Not the whole of it, and saying so is the point of this paragraph.** A
/// tool list governs what the model may ask for. It says nothing about what the
/// session loads on the way up: the observations above record that hooks still
/// ran, and a hook is an arbitrary shell command gated by no tool list at all.
/// [`SETTING_SOURCES`] is what narrows that, and it narrows rather than closes
/// it. The honest claim is the pair of them together, with the remainder named
/// where the pair is assembled — see [`AskSession::args`].
pub const TOOLS: &str = "Read,Grep,Glob";

/// The permission posture this child starts in, which is not the load-bearing
/// flag and is here so that nobody mistakes it for one.
///
/// It is a *mode*, and a mode is a policy the agent applies to the tools it
/// has. [`TOOLS`] is what decides which tools those are, and it is the
/// guarantee. Change this and the child answers differently; change that and
/// the child can write to the repository.
///
/// ## It was `plan`, and `plan` was breaking the answers
///
/// The argument for `plan` was that a read-and-propose mode matches a read-only
/// tool list, and it was wrong in a way nothing here could see until somebody
/// read the pane's output: **plan mode is not a posture, it is a set of
/// instructions**. Claude Code wraps the session prompt with a plan-mode
/// preamble and an `ExitPlanMode` protocol footer — the model is told to finish
/// by calling `ExitPlanMode` with its plan — and `ExitPlanMode` is not on
/// [`TOOLS`]. So the child would answer the question, reach the end, look for a
/// tool that did not exist, and spend its last paragraph explaining that it was
/// in read-only mode and had no way to present a plan. On the transcript that
/// paragraph is the one thing a reader was left holding, because it is the last
/// text block of the turn and the pane used to keep only that. Two bugs
/// compounding: one produced a confused epilogue, the other threw the answer
/// away and kept it.
///
/// `default` is what a read-only session should have been given all along.
/// Nothing is widened by it: the mode governs what the agent may do with the
/// tools it has, and this one has three that read. What goes away is a page of
/// instructions about a workflow this session cannot take part in.
///
/// ## Checked against a real Claude rather than reasoned about
///
/// A mode change is exactly the sort of thing that would look fine and then
/// block on a permission dialog nobody can see — there is no terminal here to
/// answer one at — so it was run rather than argued. On 2.1.222, 2026-08-09,
/// with [`AskSession::args`]'s list as it stands and only this value changed:
/// `system`/`init` reported `permissionMode` as `default` and the same
/// `["Glob","Grep","Read"]`; the turn ran six tool calls — a `Read` of an
/// absolute path and five `Grep`s — and came back with `permission_denials`
/// **empty**; the child exited 0 and nothing waited on anything. The answer
/// contained no mention of plan mode, of `ExitPlanMode`, or of being unable to
/// do what it had been asked.
const PERMISSION_MODE: &str = "default";

/// The tools named again, from the other side.
///
/// Redundant by construction — nothing in this list is in [`TOOLS`], so an
/// allowlist that is honoured makes this a no-op — and that is the reason it is
/// here rather than an argument against it. `--tools` is an allowlist over the
/// *built-in* set and abeam has one observation of it working, on one version;
/// `--disallowedTools` with a bare tool name is documented to take the tool out
/// of the session's context as well. Two mechanisms, one of them documented,
/// both saying the same four words. The cost is four words on a command line
/// nobody reads, and what it buys is that a release which changes what `--tools`
/// means does not silently hand a pane in the corner of the screen the ability
/// to edit somebody's repository.
const DISALLOWED: &str = "Write,Edit,NotebookEdit,Bash";

/// Whose settings this child loads, which is the flag that keeps a repository
/// from configuring the reader abeam started to read it.
///
/// `user` and not `user,project`, so `.claude/settings.json` in the repository
/// on screen — and `.claude/settings.local.json` beside it — do not load.
/// **The reason is hooks.** A hook is an arbitrary shell command, run by the
/// session on events like `PreToolUse`, and it is gated by *no* tool list:
/// [`TOOLS`] can be honoured exactly, as it was observed to be, and a
/// `SessionStart` hook still runs whatever it likes. Without this flag a
/// checkout could put a command in a file and have abeam run it in a pane that
/// tells the reader nothing here can change anything. `--strict-mcp-config`
/// closes the same door for MCP servers and does not touch hooks; the module
/// docs record `Hooks still ran` from the run that found this out.
///
/// **It narrows the claim rather than making it absolute, and that is worth
/// reading twice.** The user's *own* hooks — the ones in `~/.claude` — still
/// run, because `user` is exactly what is kept. That is the right side of the
/// line to come down on: those are the reader's own settings, on the reader's
/// own machine, which they configured and can inspect, and dropping them too
/// would mean a pane that quietly ignores the machine it is running on. What is
/// excluded is the half nobody chose: whatever happens to be committed in the
/// repository somebody has just opened.
///
/// The flag itself was checked against a real Claude rather than taken from the
/// help text, and that check is worth naming because of how it would fail: an
/// argument this CLI did not accept would not degrade to a session with wider
/// settings, it would be a child that refuses to start at all, and the pane
/// would report a reader that never arrives. On 2.1.222 it is accepted —
/// `system`/`init` came back with `["Glob","Grep","Read"]`, `mcp_servers`
/// empty, and the turn answered normally.
const SETTING_SOURCES: &str = "user";

/// A live child, its pipes, and the reader threads draining them.
pub struct AskSession {
    /// The id abeam chose and passed on the command line — **not** the one the
    /// child echoed back. `crate::agentstate` disowns records carrying this, so
    /// it has to be the value abeam knows it asked for rather than a value read
    /// out of a stream that may not have arrived yet.
    session_id: String,
    child: Child,
    /// `None` once the pipe has been closed or has failed. Dropping the handle
    /// is what sends the child its end of file, so this is also how a session
    /// is told there is nothing more coming.
    stdin: Option<ChildStdin>,
    events: Receiver<AskEvent>,
    /// What [`AskSession::is_live`] answers, and why that is a remembered
    /// answer rather than a fresh one: the frozen signature takes `&self` and
    /// asking the operating system needs `&mut`. It is set false by
    /// [`AskSession::poll`] — which runs every frame and does have `&mut` — on
    /// any of the three ways a session ends: stdout closing, the channel
    /// disconnecting, or the child having exited. So it is at most one frame
    /// behind, and the frame it is behind by is the one that notices.
    ///
    /// **Four setters and not three.** [`AskSession::ask`] is the fourth: a
    /// write that fails means the other end of the pipe has gone, and a session
    /// that cannot be written to is over whatever the reader threads have got
    /// round to saying. Enumerating three of them was how the guard on the reap
    /// below came to look safe.
    live: bool,
    /// How many times [`AskSession::poll`] has found the child gone and waited
    /// on it. A test's only way to prove a reap happened, since a `Child` keeps
    /// no note of it; the same seam `AskPane::builds` is for the frame path.
    #[cfg(test)]
    reaps: usize,
}

/// Which pipe a reader thread is draining.
///
/// The two are not symmetric and the difference is the whole reason this exists
/// rather than one loop over both handles: stdout is the protocol, and stderr
/// is whatever the program felt like saying.
enum Voice {
    /// Lines are JSON, parsed by [`proto::parse_line`], and end of file here is
    /// the end of the session.
    Out,
    /// Lines are prose, and end of file here means nothing at all — a child can
    /// perfectly well close standard error and go on answering.
    Err,
}

impl AskSession {
    /// Start one. `root` becomes the child's working directory, and
    /// `session_id` becomes both the child's `--session-id` and the name the
    /// probe next door is told to disown.
    ///
    /// Everything is a pipe: standard input because that is how a question
    /// arrives, standard output because that is the protocol, and standard
    /// error because a child that complains must be able to be heard.
    /// Deliberately **not** `Stdio::null()` for stdin, which is what
    /// `crate::dispatch` uses — there the child must never read the console
    /// abeam is typing at, and here the child reads a pipe abeam owns, which is
    /// the same guarantee reached the other way. Neither of them inherits the
    /// terminal.
    pub fn start(launch: &Launch, root: &Path, session_id: String) -> Result<Self> {
        let planned = plan(launch, &session_id)?;

        let mut command = Command::new(&planned.program);
        command
            .args(&planned.args)
            // The repository on screen. It is most of what the child is given:
            // the question names a path and the child stands where that path
            // means something. See `crate::panes::ask` — context is a pointer,
            // and this is the directory the pointer is relative to.
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &planned.env {
            command.env(key, value);
        }

        // No `pre_exec`, no `setsid`, no `creation_flags`. See the module docs:
        // the omission is the decision, and it is the opposite of the one
        // `crate::dispatch` argues for at length.
        let mut child = command.spawn().map_err(|why| {
            anyhow!(
                "abeam could not start `{}` to ask it a question: {why}",
                planned.target.display()
            )
        })?;

        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            // Unreachable through `Stdio::piped()`, and handled rather than
            // unwrapped because the alternative is leaving a live Claude behind
            // while panicking about not being able to talk to it.
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "abeam started `{}` and was handed no pipes to it, so there is \
                 no way to ask it anything. The child has been stopped.",
                planned.target.display()
            ));
        };

        // One channel, two senders. Ordering *within* a pipe is the reader
        // thread's, and ordering *between* the two pipes is nobody's — a
        // complaint on standard error and an answer on standard output are two
        // processes' worth of buffering apart, so the pane must not read the
        // arrival order of the two as a sequence of events.
        let (say, events) = std::sync::mpsc::channel();
        read(stdout, say.clone(), Voice::Out);
        read(stderr, say, Voice::Err);

        Ok(Self {
            session_id,
            child,
            stdin: Some(stdin),
            events,
            live: true,
            #[cfg(test)]
            reaps: 0,
        })
    }

    /// The arguments [`start`](Self::start) will use, without starting
    /// anything. Split out for the same reason `crate::dispatch::plan` is: the
    /// authority handed to a second agent is the one thing here that must never
    /// drift, and this is what a test can hold still.
    ///
    /// Eleven flags, and five of them are about authority — which is a
    /// correction rather than a count: this paragraph said "three" while the
    /// one below it named a fourth, and there is now a fifth.
    ///
    /// The six that are not are the shape. `-p` with the two stream formats and
    /// `--include-partial-messages` make a long-lived print-mode child, JSON in,
    /// JSON out, with the answer arriving while it is being written rather than
    /// at the end. `--verbose` is not an option there either: `--output-format
    /// stream-json` needs it under `-p`. `--session-id` is the whole of §4 next
    /// door, and is what `crate::agentstate` disowns.
    ///
    /// The five that decide what this child can do are [`TOOLS`],
    /// [`PERMISSION_MODE`], [`DISALLOWED`], `--strict-mcp-config` and
    /// [`SETTING_SOURCES`], and each has its own paragraph where it is defined.
    /// The last two are the ones worth reading twice, because both close doors
    /// a tool list does not reach. `--tools` is an allowlist over the *built-in*
    /// set: it says nothing about MCP servers, so without `--strict-mcp-config`
    /// a user's configured servers — a database, a deployment tool, a thing that
    /// posts messages — would be loaded into a session abeam has told the reader
    /// is read-only; and it says nothing about hooks, which are arbitrary shell
    /// commands the session runs on its own events, so without
    /// `--setting-sources` the repository on screen could configure the reader
    /// abeam started to read it. **The user's own hooks still run**, by design —
    /// see [`SETTING_SOURCES`] — so what this list buys is a narrower claim
    /// rather than an absolute one.
    ///
    /// **There is no prompt on this list.** That is the difference between this
    /// module and `crate::dispatch`, where the last argument is the user's text
    /// and a `--` fence is needed to stop a pasted paragraph from becoming a
    /// flag. Nothing here is the user's, so there is nothing here to fence: the
    /// question goes down standard input as JSON. The only value on this line
    /// that abeam did not write is the session id, which abeam generated
    /// itself.
    pub fn args(session_id: &str) -> Vec<String> {
        [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--verbose",
            "--tools",
            TOOLS,
            "--permission-mode",
            PERMISSION_MODE,
            "--strict-mcp-config",
            "--setting-sources",
            SETTING_SOURCES,
            "--session-id",
            session_id,
            "--disallowedTools",
            DISALLOWED,
        ]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect()
    }

    /// Write one turn to the child's stdin.
    ///
    /// ## What "non-blocking enough" actually means here
    ///
    /// One `write_all` of one line onto a pipe, then a flush. A pipe has a
    /// kernel buffer — 64 KiB on Windows and on Linux — and a write that fits
    /// in it returns as soon as the bytes are copied, without waiting for the
    /// child to read a single one of them. A question is a few hundred bytes.
    /// So in every case anybody can construct on purpose this returns
    /// immediately, and the draw loop is never held up.
    ///
    /// The honest remainder: `ChildStdin` is a blocking handle and there is no
    /// portable way to make it anything else — `O_NONBLOCK` is Unix's and
    /// `PIPE_NOWAIT` is a Windows call abeam has no binding for and which
    /// Microsoft's own documentation advises against. So if the child stopped
    /// reading *and* somebody pasted 64 KiB into the composer, this would wait
    /// on it, and what would be waiting is the thread that draws the screen.
    /// That is a known gap rather than a solved problem, and the way through it
    /// is a writer thread with a queue of its own — not taken here, because it
    /// moves the failure off the keystroke that caused it: a question that
    /// could not be sent would be reported a frame or two later with nothing on
    /// screen connecting it to the `Enter` that sent it.
    ///
    /// An empty question is not refused, unlike `crate::dispatch`'s empty
    /// prompt. That refusal is about an unattended agent with edits
    /// pre-approved and nothing to do; this child cannot change anything, and
    /// an empty question costs one round trip. The pane refuses one anyway,
    /// because a composer somebody opened and thought better of is not a
    /// question — but that is a fact about a composer rather than about a pipe.
    pub fn ask(&mut self, prompt: &str) -> Result<()> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(anyhow!(
                "this session's standard input has been closed, so there is \
                 nowhere to put the question. The conversation is over; asking \
                 again needs a new session."
            ));
        };

        // The newline is the frame delimiter and is written here rather than
        // inside `proto::turn`, which answers what one turn *is* rather than
        // how a stream of them is punctuated.
        let mut line = proto::turn(prompt);
        line.push('\n');

        if let Err(why) = stdin.write_all(line.as_bytes()).and_then(|()| stdin.flush()) {
            // A failed write is the end of the conversation and not a hiccup: a
            // pipe fails when the other end has gone, and half a JSON line
            // followed by a whole one is a stream the child would refuse
            // anyway. Closing the handle here is what makes the next `ask` say
            // so plainly instead of failing the same way again.
            self.stdin = None;
            self.live = false;
            return Err(anyhow!(
                "abeam could not write the question to `claude`'s standard \
                 input: {why} The session has ended — that pipe fails when the \
                 other end of it is gone."
            ));
        }
        Ok(())
    }

    /// Everything the readers have queued since the last call. **Must not
    /// block**: it is called from `Pane::tick`, on the loop that draws the
    /// agent's screen.
    ///
    /// `try_recv` in a loop, which is the whole of it. The two ways the channel
    /// can end are both an ending of the session and neither is an error: a
    /// disconnect means both reader threads have finished, which happens after
    /// the stdout one has already sent [`AskEvent::Ended`].
    pub fn poll(&mut self) -> Vec<AskEvent> {
        let mut arrived = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    if matches!(event, AskEvent::Ended) {
                        self.live = false;
                    }
                    arrived.push(event);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.live = false;
                    break;
                }
            }
        }

        // And the child itself, asked once a frame. This is also what reaps it:
        // a child that has exited and never been waited on is a zombie on Unix
        // for as long as abeam runs, and the pane may sit on an ended session
        // for the rest of the day.
        //
        // After the drain rather than before, so that the events a child
        // produced on its way out are never dropped in favour of the news that
        // it is gone.
        //
        // **Unconditionally, and the `self.live &&` that used to be here is why
        // this comment is longer than the line.** The drain above sets `live`
        // false the moment `Ended` arrives, and `Ended` is sent when stdout
        // closes — which is the same pass on which a child usually exits. So the
        // guard skipped `try_wait` on exactly the pass where the wait was owed,
        // and on every pass after it: measured at zero reaps across five hundred
        // polls of a session whose child had gone. The line that claimed to
        // prevent a zombie guaranteed one. `try_wait` on an already-waited child
        // is a cached answer rather than a syscall, so the cost of asking after
        // the session has ended is nothing.
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            #[cfg(test)]
            {
                self.reaps += 1;
            }
            self.live = false;
        }
        arrived
    }

    /// The id abeam gave it, which is what `crate::agentstate` disowns.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The child's process id while there is a child, for a message or a
    /// diagnostic.
    ///
    /// **Not what identifies this session to `crate::agentstate`**, and the
    /// distinction is the reason §4 next door is about ids rather than pids.
    /// Two things are wrong with this number for that purpose: on a Windows npm
    /// install it is `cmd.exe`'s rather than the Claude's, because
    /// `crate::launch` routes a `.cmd` through an interpreter; and a pid is
    /// handed out again once its process is gone, so a record named after this
    /// one may belong to something else entirely. [`AskSession::session_id`] is
    /// the answer to that question.
    ///
    /// `None` once the session is not live — which is *nearly* "once the process
    /// has exited" and is not the same sentence. `live` also goes false when
    /// stdout closes and when the channel disconnects, and a child can close its
    /// standard output and go on running. So this answers "is there still a
    /// session whose pid means anything", and a `None` here is not evidence that
    /// the process is gone. The direction it is wrong in is the safe one: it
    /// stops naming the number slightly before the number stops naming this
    /// process, rather than after.
    // `#[cfg(test)]` because all three call sites are, and compiling it that way
    // is what makes that true rather than asserted. It used to carry an `allow`
    // and a claim of precedent from `crate::agentstate`'s `Wire`, which has no
    // field held ahead of a consumer and no `allow` anywhere in the file — the
    // comment was arguing from a thing that does not exist. What it is for is
    // the `Drop` test, which needs a number to ask the operating system about
    // after the session has been dropped.
    #[cfg(test)]
    pub fn pid(&self) -> Option<u32> {
        self.live.then(|| self.child.id())
    }

    /// Has the child exited, asked of the operating system and of nothing else?
    ///
    /// For the one test next door that has to know a child has gone *without*
    /// draining the channel that would say so — the whole subject there is what
    /// happens on the pass where an `Ended` is still queued. It touches neither
    /// `live` nor the reap count, so a test using it is not also arranging the
    /// answer to the question it is asking.
    #[cfg(test)]
    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn is_live(&self) -> bool {
        self.live
    }
}

/// The child dies with the pane. See the module docs for why this is the
/// opposite of what `crate::dispatch` wants, and for the one case it cannot
/// cover.
///
/// Standard input first, and that ordering is load-bearing rather than tidy:
/// dropping the write end is what gives the child end of file, and end of file
/// is what a `claude -p` exits 0 on. On the Windows npm route the process being
/// killed on the next line is `cmd.exe` and the Claude is *its* child, out of
/// reach of the kill and not out of reach of the closed pipe.
///
/// Then `wait`, which reaps rather than lingers. It cannot hang on a process
/// that has been killed — `SIGKILL` cannot be caught and `TerminateProcess`
/// does not ask — and it does not wait for the pipes, so a grandchild holding
/// them open does not hold abeam's exit.
impl Drop for AskSession {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The reader threads are not joined, deliberately. They are blocked on
        // a read of pipes whose other end has just gone, so they are about to
        // see end of file and finish; joining them would put a wait for two
        // threads in the path of closing a pane, to observe something that
        // nothing will ask about afterwards.
    }
}

/// Exactly what will be started: which program, with which arguments, and what
/// has to be in its environment for those arguments to arrive.
///
/// The same shape as `crate::dispatch::plan` and for the same reason, which is
/// worth reading there in full: **`Launch::args` is a complete argument list
/// and not a prefix.** On the Windows npm route those arguments are
/// `/e:ON /v:OFF /d /c %ABEAM_LAUNCH%` and the real command line is inside that
/// variable, already quoted for `cmd`'s parser — so appending to them would put
/// abeam's flags *after* the expansion, where `cmd` reads them as more of its
/// own command line. The command line is rebuilt rather than extended, by the
/// module that owns that quoting and has the tests for it.
///
/// What is *not* here is the whole point of this module. `crate::dispatch` has
/// to hand the user three refusals on that route — a newline, a carriage
/// return, an eight-kilobyte ceiling — because the prompt is on the command
/// line. Nothing on this command line is the user's: ten fixed flags and a
/// UUID abeam generated. `crate::launch` can still refuse it, and if it ever
/// does that is abeam's bug rather than something the reader typed, so the
/// message says so.
fn plan(launch: &Launch, session_id: &str) -> Result<Launch> {
    let args = AskSession::args(session_id);

    // Windows-only for `crate::dispatch`'s reason: on Unix `crate::launch`
    // cannot produce a program that is not its own target — the kernel reads
    // the `#!` line, so nothing is ever routed through an interpreter — and an
    // arm gated away rather than left always-false is an arm that cannot carry
    // an error message about `cmd.exe` to a reader who has none.
    #[cfg(unix)]
    debug_assert_eq!(
        launch.program, launch.target,
        "on Unix a program is always its own target, and `plan` discards the \
         args and env of any launch where it is not"
    );

    #[cfg(windows)]
    if launch.program != launch.target {
        return crate::launch::resolve(&launch.target.to_string_lossy(), &args).map_err(|why| {
            anyhow!(
                "abeam could not build a command line for `{}`, which is a \
                 script and has to be run by cmd.exe: {why} Nothing on that \
                 command line is yours — it is ten fixed flags and an \
                 identifier abeam generated — so this is a fault in abeam \
                 rather than in the question you asked.",
                launch.target.display()
            )
        });
    }

    Ok(Launch {
        program: launch.program.clone(),
        target: launch.target.clone(),
        args,
        // Nothing to carry: an executable's arguments travel as arguments.
        env: Vec::new(),
    })
}

/// Drain one pipe into the channel, on a thread of its own, until it ends.
///
/// Line-delimited, and lines are read as bytes and converted lossily rather
/// than through `BufRead::lines`. That is not fussiness: `lines` hands back an
/// `Err` for a line that is not UTF-8, and the honest thing to do with an error
/// from a pipe is to stop reading it — so one stray byte, in one line, out of
/// something that is not even the protocol, would end a conversation.
/// `from_utf8_lossy` costs that line a replacement character instead.
///
/// There is no cap on how long a line may be, deliberately, and it is the same
/// argument `crate::dispatch` makes about not checking a prompt's length: the
/// biggest line this stream produces is the `result` carrying a whole answer,
/// and any ceiling abeam wrote here would be a guess about how much somebody's
/// question could be worth answering. What it costs is that a child which never
/// prints a newline grows this buffer until abeam runs out of memory. A cap
/// belongs here on the day anything has ever seen that happen, with the size
/// taken from the observation rather than from an opinion.
fn read<R: Read + Send + 'static>(source: R, events: Sender<AskEvent>, voice: Voice) {
    thread::spawn(move || {
        use std::io::BufRead;

        let mut pipe = BufReader::new(source);
        let mut line = Vec::new();
        loop {
            line.clear();
            match pipe.read_until(b'\n', &mut line) {
                // End of file. On stdout that is the session ending and the
                // pane is told; on stderr it is a child that has stopped
                // complaining, which is not news and must not be announced as
                // an ending — the two pipes close in whatever order they close
                // in, and only one of them is the protocol.
                Ok(0) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(trimmed(&line));
                    let event = match voice {
                        Voice::Out => proto::parse_line(&text),
                        Voice::Err => complaint(&text),
                    };
                    // A send that fails means the `AskSession` has been
                    // dropped, so there is nobody left to tell and nothing left
                    // to read for.
                    if let Some(event) = event
                        && events.send(event).is_err()
                    {
                        return;
                    }
                }
                Err(why) => {
                    // Reported and then stopped. A read error on a pipe is not
                    // a bad line to skip past — the pipe is what failed — and a
                    // loop that carried on would spin on the same error as fast
                    // as the thread can go.
                    let _ = events.send(AskEvent::Broke(format!(
                        "abeam stopped being able to read from the session: \
                         {why}"
                    )));
                    return;
                }
            }
        }

        if matches!(voice, Voice::Out) {
            let _ = events.send(AskEvent::Ended);
        }
    });
}

/// One line of standard error, as an event or as nothing.
///
/// [`AskEvent::Broke`] because there is no better variant and because the
/// alternative is worse than an imperfect label: what arrives here is a proxy
/// refusing a connection, an expired credential, a Node warning, or the reason
/// a turn produced no answer at all. A pane that swallowed those would sit
/// there saying nothing while the child explained itself into a void — which is
/// the failure this module is least able to recover from, because there is
/// nothing else on screen to suggest where to look.
///
/// Blank lines are dropped. A child that prints an empty line on standard error
/// has not said anything, and an empty complaint in a transcript is a reader
/// looking for the missing half of it.
fn complaint(line: &str) -> Option<AskEvent> {
    let said = line.trim();
    (!said.is_empty()).then(|| AskEvent::Broke(format!("`claude` said on standard error: {said}")))
}

/// The line without its terminator, whichever of the two this platform's child
/// wrote.
///
/// Both, unconditionally, and not because Windows children are the only ones
/// that produce a `\r\n`: what is on the other end of this pipe is a Node
/// process, and which line ending it emits is a fact about the program rather
/// than about the kernel it is running on. A `\r` left on the end of a JSON
/// line is harmless — `serde_json` skips trailing whitespace — and a `\r` left
/// on the end of a *complaint* is a carriage return in the middle of somebody's
/// transcript.
fn trimmed(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && matches!(line[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    &line[..end]
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// **Nothing here starts a `claude`**, and that is a rule rather than an
/// observation about what these happen to do. A real one would cost somebody's
/// tokens on every `cargo test` — the two-turn probe that produced this
/// module's observations cost $0.054 — and would put a second agent in whatever
/// repository the test binary was standing in. So the protocol is tested on
/// strings, in `proto`, and everything that has to be asked of a real process
/// is asked of a shim: a `.cmd` on Windows, a `#!/bin/sh` script on Unix,
/// exactly as `crate::dispatch`'s spawning tests do.
///
/// Three groups, the same way that module splits them:
///
/// - **Shared**: the argument list and the direct `plan`, which are claims
///   about a `Vec<String>`.
/// - **Twinned**: the spawning tests, where the shim and the way it blocks are
///   spelled per platform, and the fabricated `Launch`, which has to be a path
///   the platform could really have produced.
/// - **Windows-only**: the `%ABEAM_LAUNCH%` rebuild, which is a fact about a
///   command processor that a Linux build does not have.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch;
    use crate::testutil::TempDir;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    /// The `Launch` a direct install produces: a program that is its own
    /// target, resolved with no arguments. Fabricated rather than found, so
    /// that what is under test is the argument list rather than the machine.
    fn native() -> Launch {
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

    // --- what authority the second agent is handed --------------------------

    #[test]
    fn the_argument_list_is_the_read_only_claim_and_is_asserted_whole() {
        // The security assertion of this module, written as one. What `args`
        // returns is the complete authority abeam hands to a second agent
        // running in somebody's repository, and the pane tells the reader it
        // cannot write. It is asserted whole rather than by `contains`, so that
        // a flag cannot be added without coming through this line.
        //
        // If you are here because this test is in the way of an edit: the
        // question is not "is this flag useful". It is "would somebody who was
        // told this pane can only read agree that it still can".
        assert_eq!(
            AskSession::args("3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e"),
            args(&[
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--include-partial-messages",
                "--verbose",
                "--tools",
                "Read,Grep,Glob",
                "--permission-mode",
                "default",
                "--strict-mcp-config",
                "--setting-sources",
                "user",
                "--session-id",
                "3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e",
                "--disallowedTools",
                "Write,Edit,NotebookEdit,Bash",
            ])
        );

        // The four constants the module documents, spelled once each so that
        // changing one is visibly changing that rather than editing a vector.
        let list = AskSession::args("x");
        assert_eq!(TOOLS, "Read,Grep,Glob");
        assert!(list.contains(&TOOLS.to_string()));
        assert!(list.contains(&PERMISSION_MODE.to_string()));
        assert!(list.contains(&DISALLOWED.to_string()));
        assert!(list.contains(&SETTING_SOURCES.to_string()));

        // The repository's own settings do not load, which is the half of the
        // claim a tool list cannot make: a hook is an arbitrary shell command
        // and no allowlist on this line gates one. `project` and `local` are
        // named here rather than only `user` being asserted, because the way
        // this regresses is somebody widening the value rather than deleting
        // the flag.
        let sources = list
            .iter()
            .position(|arg| arg == "--setting-sources")
            .map(|at| list[at + 1].clone())
            .expect("the flag that keeps a checkout from configuring the reader");
        assert_eq!(sources, "user");
        for whose in ["project", "local"] {
            assert!(
                !sources.split(',').any(|source| source == whose),
                "`{whose}` settings load into a pane that says it can only read"
            );
        }

        // And the belt-and-braces claim, which is only worth anything if the
        // two lists really are disjoint: nothing that can change a file is on
        // the allowlist, and everything that can is named on the denylist.
        for writes in ["Write", "Edit", "NotebookEdit", "Bash"] {
            assert!(
                !TOOLS.split(',').any(|tool| tool == writes),
                "`{writes}` is on the allowlist"
            );
            assert!(
                DISALLOWED.split(',').any(|tool| tool == writes),
                "`{writes}` is not on the denylist"
            );
        }
    }

    #[test]
    fn nothing_on_that_command_line_hands_the_machine_over() {
        // Named one at a time as well as asserted whole above, because these
        // are the flags a well-meaning edit reaches for and the whole vector is
        // easy to re-record. `crate::dispatch` has the same test for the
        // authority *it* hands out, and the two are the pair worth reading
        // together: that one grants edits to an unattended agent on purpose and
        // draws the line at `dangerously`; this one grants nothing at all.
        let list = AskSession::args("3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e");
        for forbidden in [
            "--dangerously-skip-permissions",
            "--allow-dangerously-skip-permissions",
            "bypassPermissions",
            "acceptEdits",
            "--worktree",
            "-w",
            "--bg",
        ] {
            assert!(
                !list.iter().any(|arg| arg == forbidden),
                "`{forbidden}` reached the command line of the read-only pane"
            );
        }

        // The mode is spelled here as well as in the whole list, because
        // `--permission-mode` is the argument whose *value* is the thing that
        // matters and a diff that changed one word of it would otherwise pass
        // through one assertion.
        //
        // It is `default` and was `plan`, which is the one value change on this
        // line since the pane shipped. [`PERMISSION_MODE`] has the whole reason
        // and the run that checked it; the short version is that plan mode
        // instructs the model to finish by calling `ExitPlanMode`, which is not
        // a tool this session has, and the answers said so. Nothing about the
        // authority on this line moved: the two lists below are what decide
        // that, and they are asserted whole above.
        let mode = list
            .iter()
            .position(|arg| arg == "--permission-mode")
            .map(|at| list[at + 1].clone());
        assert_eq!(mode.as_deref(), Some("default"));
        // Named rather than only asserted, because the way this regresses is
        // somebody reaching for a mode that *sounds* narrower and getting one
        // that hands over the machine.
        for widest in ["bypassPermissions", "acceptEdits", "dangerously-skip"] {
            assert_ne!(mode.as_deref(), Some(widest));
        }

        // And no prompt: nothing on this line is the user's, which is why there
        // is no `--` fence on it. The only value abeam did not write is the
        // session id it generated.
        let id = new_session_id();
        let list = AskSession::args(&id);
        assert_eq!(
            list.iter().filter(|arg| arg.as_str() == "--").count(),
            0,
            "a fence appeared on a command line with nothing of the user's on it"
        );
        assert_eq!(list.last().map(String::as_str), Some(DISALLOWED));
        assert!(list.contains(&id));
    }

    // --- the installs -------------------------------------------------------

    #[test]
    fn a_direct_claude_is_started_with_abeams_own_arguments_and_nothing_else() {
        let planned = plan(&native(), "abcd-1234").expect("a fixed argument list always plans");

        assert_eq!(planned.program, native().program);
        assert_eq!(
            planned.target, planned.program,
            "a program started directly is its own target"
        );
        assert_eq!(planned.args, AskSession::args("abcd-1234"));
        assert!(
            planned.env.is_empty(),
            "an executable's arguments travel as arguments"
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_npm_claude_has_its_command_line_rebuilt_rather_than_appended_to() {
        // `Launch::args` for a `.cmd` is complete rather than a prefix: it is
        // `/e:ON /v:OFF /d /c %ABEAM_LAUNCH%`, and the real command line is in
        // the variable. Appending abeam's flags to those five would put them
        // after the expansion, where `cmd` reads them as more of its own
        // command line.
        let dir = TempDir::new("ask-npm");
        let script = dir.write("abeam-claude.cmd", b"@echo off\r\n");
        let resolved = launch::resolve(&script.to_string_lossy(), &[]).expect("a .cmd is routed");
        assert_eq!(
            resolved.args,
            args(&["/e:ON", "/v:OFF", "/d", "/c", "%ABEAM_LAUNCH%"]),
            "the shape this test is about has changed"
        );

        let planned = plan(&resolved, "3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e")
            .expect("a fixed argument list always plans");

        // Not one token longer. The flags are not where they went.
        assert_eq!(planned.args, resolved.args);
        assert_eq!(planned.program, resolved.program);
        assert_eq!(planned.target, script, "the border names the script");

        let (key, line) = planned
            .env
            .first()
            .expect("the command line travels in a variable")
            .clone();
        assert_eq!(key, "ABEAM_LAUNCH");
        assert!(line.contains(&script.display().to_string()));
        // The two tool lists arrive *quoted*, and that is `crate::launch` doing
        // its job rather than an oddity to tidy away: a comma is an argument
        // separator to `cmd`, so `Read,Grep,Glob` on a bare command line is
        // three arguments and the child would be handed a `--tools` of `Read`.
        // The quoting is eager by design there — anything not alphanumeric and
        // not on a short harmless list — and the CRT behind node strips the
        // quotes again, so what the child parses is one argument with two
        // commas in it. Spelled out here rather than asserted loosely, because
        // the difference between the quoted and the unquoted form is the
        // difference between a read-only session and one with every built-in
        // tool but `Grep` and `Glob`.
        for flag in [
            "-p",
            "--input-format stream-json",
            "--output-format stream-json",
            "--include-partial-messages",
            "--verbose",
            "--tools \"Read,Grep,Glob\"",
            "--permission-mode default",
            "--strict-mcp-config",
            "--session-id 3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e",
            "--disallowedTools \"Write,Edit,NotebookEdit,Bash\"",
        ] {
            assert!(line.contains(flag), "`{flag}` is missing from: {line}");
        }
        // Once each. Twice is what appending would have produced, and a command
        // line carrying two tool lists is one nobody wrote.
        assert_eq!(line.matches("--tools").count(), 1);
        assert_eq!(line.matches("--session-id").count(), 1);
    }

    // --- and it actually starts ---------------------------------------------
    //
    // Everything above is a claim about a `Vec<String>`. What is claimed below
    // is about a *process*: that it starts, that what it prints reaches the
    // pane, and — the one that no comparison of strings can reach — that it is
    // gone when the session is.

    /// A shim that does what it is told, in the shape the platform can start.
    ///
    /// The same arrangement as `crate::dispatch`'s: a `.cmd` in a directory
    /// with a space in its name on Windows, because the path goes onto a
    /// command line as text; a `#!/bin/sh` with the execute bit on Unix,
    /// through [`TempDir::write_exec`], because a `#!` file without that bit is
    /// `EACCES` at the spawn.
    #[cfg(windows)]
    fn shim(dir: &TempDir, name: &str, lines: &[String]) -> PathBuf {
        let home = dir.path().join("with space");
        std::fs::create_dir_all(&home).expect("a directory with a space in it");
        let script = home.join(format!("abeam-{name}.cmd"));
        let mut text = String::from("@echo off\r\n");
        for line in lines {
            text.push_str(line);
            text.push_str("\r\n");
        }
        std::fs::write(&script, text).expect("write a shim");
        script
    }
    #[cfg(unix)]
    fn shim(dir: &TempDir, name: &str, lines: &[String]) -> PathBuf {
        let mut text = String::from("#!/bin/sh\n");
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        dir.write_exec(&format!("abeam-{name}"), text.as_bytes())
    }

    /// A line the shim prints on standard output, quoted the way its own
    /// interpreter needs. `cmd` prints a double quote literally and `sh` does
    /// not, so the JSON below is bare on one platform and single-quoted on the
    /// other.
    ///
    /// `printf '%s\n'` rather than `echo` on Unix, which is not a stylistic
    /// preference: `/bin/sh` is `dash` on most of Debian's descendants, and
    /// `dash`'s built-in `echo` interprets backslash escapes in its argument
    /// with no flag asked for. Every line these shims carry is JSON, and JSON
    /// is made of backslash escapes — a `\n` inside a string would arrive at
    /// the parser as a real newline, which is to say as two broken lines. Only
    /// the *format* string of `printf` is scanned for escapes, and that one is
    /// abeam's.
    #[cfg(windows)]
    fn says(text: &str) -> String {
        format!("echo {text}")
    }
    #[cfg(unix)]
    fn says(text: &str) -> String {
        format!("printf '%s\\n' '{text}'")
    }

    /// The same, on standard error.
    #[cfg(windows)]
    fn complains(text: &str) -> String {
        format!("echo {text} 1>&2")
    }
    #[cfg(unix)]
    fn complains(text: &str) -> String {
        format!("printf '%s\\n' '{text}' >&2")
    }

    /// A shim that blocks on standard input for ever, without starting a single
    /// other process.
    ///
    /// The "without" is what makes the drop test mean anything. `ping -n 60` is
    /// the usual way to make a `.cmd` wait, and it would have this test
    /// asserting that `cmd.exe` was killed while a grandchild nobody looked at
    /// went on running. `set /p` and `read` both block *in the interpreter*, on
    /// the pipe abeam is holding open — which is also what the real child does
    /// between turns, so the shim is waiting the same way for the same reason.
    #[cfg(windows)]
    fn waits_for_input() -> Vec<String> {
        vec![
            says("READY"),
            ":loop".to_string(),
            "set /p LINE=".to_string(),
            "goto loop".to_string(),
        ]
    }
    #[cfg(unix)]
    fn waits_for_input() -> Vec<String> {
        vec![
            says("READY"),
            "while read -r LINE; do :; done".to_string(),
        ]
    }

    /// Whether a process is still there, asked of the operating system.
    ///
    /// `kill(pid, 0)` on Unix performs the existence and permission check and
    /// delivers no signal, which is what it is for. Windows has no such call
    /// without a binding this crate does not have, so the question goes to
    /// `tasklist` — named absolutely, out of `%SystemRoot%`, for
    /// `crate::launch`'s reason: a bare name reaching `CreateProcessW` is
    /// resolved against the current directory first, and under `cargo test`
    /// that is the crate being built.
    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        // SAFETY: `kill` with signal 0 touches no memory abeam owns and
        // delivers nothing. It is unsafe only because every function in `libc`
        // is.
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    fn alive(pid: u32) -> bool {
        let tasklist = PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()),
        )
        .join("System32")
        .join("tasklist.exe");
        let out = Command::new(tasklist)
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .expect("tasklist is a Windows component and is always there");
        // A match is a CSV row with the pid as its second field; no match is a
        // sentence beginning `INFO:`. Looking for the quoted number rather than
        // the number keeps a pid from matching a memory figure or a session id.
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    }

    /// Poll until everything `enough` is waiting for has arrived, or give up
    /// loudly.
    ///
    /// A bounded wait rather than a sleep of a fixed length: the reader threads
    /// and the child are both real, so how long anything takes is the machine's
    /// business. Ten seconds is long enough that only a genuine failure reaches
    /// the panic, and the panic says what was seen instead — a test that times
    /// out with no account of what arrived is a test somebody deletes.
    ///
    /// The predicate is over everything seen so far rather than over one event,
    /// and that is what keeps these tests from being flaky by construction.
    /// There are two pipes and two threads, and nothing orders a line on
    /// standard error against a line on standard output — so a test that
    /// stopped at the first event it cared about would routinely stop before
    /// the second one had been scheduled. Waiting for the *set* asks the
    /// question the test actually has.
    fn wait_for(
        session: &mut AskSession,
        what: &str,
        enough: impl Fn(&[AskEvent]) -> bool,
    ) -> Vec<AskEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            seen.extend(session.poll());
            if enough(&seen) {
                return seen;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("waited ten seconds for {what} and saw only: {seen:#?}");
    }

    /// Whether anything in `seen` is the event `wanted` recognises.
    fn any(seen: &[AskEvent], wanted: impl Fn(&AskEvent) -> bool) -> bool {
        seen.iter().any(wanted)
    }

    /// Whether the reader threads have carried a line of standard error saying
    /// `what`.
    fn complained(seen: &[AskEvent], what: &str) -> bool {
        any(seen, |event| {
            matches!(event, AskEvent::Broke(said) if said.contains(what))
        })
    }

    /// Start a shim as though it were the agent, through the same resolver
    /// every other spawn in abeam goes through.
    fn through(script: &Path, root: &Path) -> AskSession {
        let resolved = launch::resolve(&script.to_string_lossy(), &[]).expect("the shim resolves");
        AskSession::start(&resolved, root, new_session_id()).expect("the shim starts")
    }

    #[test]
    fn the_child_dies_with_the_session_because_nothing_here_detaches_it() {
        // The one property of this spawn that no comparison of strings can
        // reach, and the one `crate::dispatch` deliberately does not have: a
        // `claude` left running after the pane that started it has gone is a
        // process burning somebody's quota to answer a question nobody will
        // ever read.
        //
        // Delete the `Drop` impl and every other test in this file still
        // passes, because a child that has been abandoned prints exactly what a
        // child that will be killed prints.
        let dir = TempDir::new("ask-drop");
        let script = shim(&dir, "claude", &waits_for_input());

        let mut session = through(&script, dir.path());
        let pid = session.pid().expect("a live session has a child");

        // Waited for rather than assumed: the assertion below is only worth
        // anything about a process that really was running, and `spawn`
        // returning says the process exists rather than that it has got as far
        // as its first line.
        wait_for(&mut session, "the shim to start", |seen| {
            complained(seen, "READY")
        });
        assert!(alive(pid), "the shim was not running to begin with");
        assert!(session.is_live());

        drop(session);

        assert!(
            !alive(pid),
            "process {pid} outlived the session that started it"
        );
    }

    #[test]
    fn what_the_child_prints_reaches_the_pane_and_a_complaint_is_not_swallowed() {
        // The whole path, once: a real process, two real pipes, two reader
        // threads and the parser. Everything below the argument list is
        // exercised here and nowhere else.
        let dir = TempDir::new("ask-reads");
        let script = shim(
            &dir,
            "claude",
            &[
                says(
                    r#"{"type":"system","subtype":"init","tools":["Glob","Grep","Read"],"model":"claude-opus-4-5","session_id":"echoed"}"#,
                ),
                complains("Warning: something on standard error"),
                says(
                    r#"{"type":"result","subtype":"success","is_error":false,"result":"ALPHA","total_cost_usd":0.0544}"#,
                ),
            ],
        );

        let mut session = through(&script, dir.path());
        let chosen = session.session_id().to_string();

        // The shim says its three lines and exits, so all four of these arrive:
        // the two protocol lines, the complaint from the other pipe, and the
        // ending. Waited for as a set, because the two pipes are not ordered
        // against one another — see [`wait_for`].
        let seen = wait_for(&mut session, "the session to say everything", |seen| {
            any(seen, |event| matches!(event, AskEvent::Ended))
                && any(seen, |event| matches!(event, AskEvent::Ready { .. }))
                && any(seen, |event| matches!(event, AskEvent::Turn { .. }))
                // Standard error is not the protocol and is not dropped
                // either. What lands there is a proxy refusing a connection or
                // a credential that has expired, and a pane that swallowed it
                // would sit silent while the child explained itself into a
                // void.
                && complained(seen, "something on standard error")
        });

        assert!(
            any(&seen, |event| matches!(
                event,
                AskEvent::Ready { tools, .. } if tools.len() == 3
            )),
            "the init line arrived with the wrong tool list: {seen:#?}"
        );
        assert!(
            any(&seen, |event| matches!(
                event,
                AskEvent::Turn { text, cost_usd, error, .. }
                    if text == "ALPHA" && *cost_usd == Some(0.0544) && error.is_none()
            )),
            "the turn did not reach the pane whole: {seen:#?}"
        );
        assert!(
            complained(&seen, "standard error"),
            "a complaint arrived without saying where from: {seen:#?}"
        );

        // Exactly one ending, from stdout. Standard error closing is not news:
        // announcing it would offer the reader a restart while the child was
        // still answering.
        assert_eq!(
            seen.iter()
                .filter(|event| matches!(event, AskEvent::Ended))
                .count(),
            1,
            "the session ended more than once: {seen:#?}"
        );
        assert!(!session.is_live(), "an ended session still says it is live");
        assert_eq!(
            session.pid(),
            None,
            "a session that has ended still names a pid, which now names \
             whatever got the number next"
        );

        // The id abeam chose, not the one the child echoed. `crate::agentstate`
        // disowns records carrying this, so it has to be the value abeam knows
        // it passed rather than a value read back out of the stream.
        assert_eq!(session.session_id(), chosen);
        assert_ne!(session.session_id(), "echoed");
    }

    #[test]
    fn a_question_goes_down_the_pipe_as_one_line_however_many_it_was_typed_on() {
        // The claim the whole shape exists for, asked of a process rather than
        // of a string: a multi-line question — which on `crate::dispatch`'s
        // route cannot be sent at all through a Windows npm install — arrives
        // whole, on one line, with its quotes and its ampersand intact.
        //
        // The shim reads one line and prints it back inside a `result`, which
        // is the only way to see what the child received.
        let dir = TempDir::new("ask-writes");
        // `setlocal EnableDelayedExpansion` and `!LINE!` rather than `%LINE%`,
        // and it is the same hazard this whole module exists to route around
        // arriving inside the fixture. `%LINE%` is substituted *before* `cmd`
        // parses the line, so the `&` in the question would end the `echo` and
        // start a command out of the rest of somebody's sentence — which is
        // exactly what `crate::dispatch` refuses a prompt over. Delayed
        // expansion substitutes after parsing, so the `&` is a byte. The shim
        // needs this and abeam does not, because abeam never puts the question
        // on a command line at all.
        #[cfg(windows)]
        let reads_one_line = vec![
            "setlocal EnableDelayedExpansion".to_string(),
            "set /p LINE=".to_string(),
            "echo {\"type\":\"result\",\"result\":\"SAW\"}".to_string(),
            "echo !LINE! 1>&2".to_string(),
        ];
        // `read -r`, and the `-r` is the same hazard again from the other side:
        // without it `sh`'s `read` treats a backslash as an escape character
        // and removes it, so the shim would report a question with all of
        // JSON's escaping quietly stripped out of it and the test would be
        // asserting against a line that never existed.
        #[cfg(unix)]
        let reads_one_line = vec![
            "read -r LINE".to_string(),
            says(r#"{"type":"result","result":"SAW"}"#),
            "printf '%s\\n' \"$LINE\" >&2".to_string(),
        ];
        let script = shim(&dir, "claude", &reads_one_line);

        let mut session = through(&script, dir.path());
        session
            .ask("why does \"a & b\"\nfail?")
            .expect("a question with a newline in it is an ordinary question");

        // Waited for on the pipe that carries it. The shim prints its `result`
        // first and echoes the question afterwards, and the two travel down
        // different pipes — so waiting for the answer would routinely stop
        // before the thing this test is about had been read at all.
        let seen = wait_for(&mut session, "the shim to echo the question", |seen| {
            any(seen, |event| {
                matches!(event, AskEvent::Turn { text, .. } if text == "SAW")
            }) && complained(seen, r#""type":"user""#)
        });
        let echoed: String = seen
            .iter()
            .filter_map(|event| match event {
                AskEvent::Broke(said) => Some(said.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // What the shim read back is one line of JSON with the newline escaped
        // inside it: the question, the `&` that would have ended a `cmd`
        // command line had this been an argument, and the two-character `\n`
        // that is a real newline in the composer.
        assert!(
            echoed.contains(r#""type":"user""#),
            "the question did not arrive as one turn: {echoed}"
        );
        assert!(
            echoed.contains(r"fail?"),
            "the question did not arrive: {echoed}"
        );
        assert!(
            echoed.contains(r"\n"),
            "the newline did not arrive escaped, so it was not one line: {echoed}"
        );
        assert!(
            echoed.contains(" & "),
            "an ampersand did not survive the trip: {echoed}"
        );
        assert!(
            !echoed.contains("SAW"),
            "the answer was read as the question: {echoed}"
        );
    }

    #[test]
    fn asking_a_session_that_has_gone_says_so_rather_than_failing_twice() {
        // A shim that exits at once. The write may or may not reach a pipe that
        // is closing — a pipe with nobody at the other end fails on the write
        // on Unix and can succeed into a buffer on Windows — so what is pinned
        // is the state *after*: once a write has failed the handle is closed,
        // and the next `ask` says the conversation is over instead of failing
        // the same way again.
        let dir = TempDir::new("ask-gone");
        let script = shim(&dir, "claude", &[says("bye")]);

        let mut session = through(&script, dir.path());
        wait_for(&mut session, "the shim to exit", |seen| {
            any(seen, |event| matches!(event, AskEvent::Ended))
        });

        // One of the first two writes reaches a pipe with nobody at the other
        // end and fails; which one is the operating system's business.
        let mut why = session.ask("are you there?").err();
        if why.is_none() {
            why = session.ask("still there?").err();
        }
        let why = why
            .expect("a write to a pipe whose other end has gone does not succeed twice")
            .to_string();
        assert!(why.contains("session has ended"), "got: {why}");

        // And then the handle is closed, so every question after it is the
        // sentence rather than the same operating-system error again. That is
        // what the closing is for: a reader who keeps typing gets told the
        // conversation is over and what would start another one.
        let after = session
            .ask("hello?")
            .expect_err("the conversation is over")
            .to_string();
        assert!(after.contains("has been closed"), "got: {after}");
        assert!(after.contains("new session"), "the way through: {after}");
        assert!(!session.is_live());
    }

    #[test]
    fn a_child_that_has_gone_is_waited_on_rather_than_left_a_zombie() {
        // The reap, which used to be guarded by `self.live` — and `live` is set
        // false by the drain in the same call, on the very pass where the child
        // has just gone. So `try_wait` was skipped on the pass that owed the
        // wait and on every pass after it: measured at zero reaps across five
        // hundred polls. A child that has exited and never been waited on is a
        // zombie on Unix for as long as abeam runs, and the pane can sit on an
        // ended session for the rest of the day.
        let dir = TempDir::new("ask-reap");
        let script = shim(&dir, "claude", &[says("bye")]);

        let mut session = through(&script, dir.path());
        wait_for(&mut session, "the shim to exit", |seen| {
            any(seen, |event| matches!(event, AskEvent::Ended))
        });
        assert!(!session.is_live(), "the drain has already said it is over");

        // Bounded rather than a single call, because `Ended` says stdout has
        // closed and the process has a few instructions left to run after that.
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.reaps == 0 && Instant::now() < deadline {
            session.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            session.reaps > 0,
            "the child was never waited on, which is the zombie the comment \
             above that line claims it prevents"
        );
    }

    #[test]
    fn polling_an_idle_session_returns_at_once_and_says_nothing() {
        // `poll` is called from `Pane::tick`, on the loop that draws the
        // agent's screen, and a blocking `recv` there would freeze abeam
        // between turns — which is most of the time. Asserted by the clock,
        // because "does not block" is not a property a type signature has.
        let dir = TempDir::new("ask-idle");
        let script = shim(&dir, "claude", &waits_for_input());

        let mut session = through(&script, dir.path());
        wait_for(&mut session, "the shim to start", |seen| {
            complained(seen, "READY")
        });

        let began = Instant::now();
        for _ in 0..100 {
            assert!(session.poll().is_empty());
        }
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "a hundred polls of a quiet session took {:?}, which is a draw loop \
             waiting on a child",
            began.elapsed()
        );
        assert!(session.is_live(), "a session that is waiting is still live");
    }
}
