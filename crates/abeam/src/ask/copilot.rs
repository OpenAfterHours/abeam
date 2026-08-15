//! The same pane, asking GitHub Copilot CLI instead of Claude.
//!
//! [`super`]'s module docs describe a child that is started once, held open,
//! and fed one JSON line per turn down a pipe that stays open for the life of
//! the pane. **Almost none of that is available here**, and the differences are
//! not preferences — they are the shape Copilot CLI publishes. Reading this
//! file as "the Claude one, adapted" will mislead; the two agree on what the
//! pane is *for* and on nothing about how it is fed.
//!
//! ## Nothing in this file has been run
//!
//! Said first because it governs how to read everything after it. `copilot` is
//! not installed on the machine abeam is developed on, and the README has said
//! since the second agent landed that abeam has never been run with Copilot
//! CLI — not once. This module does not change that. Every flag below comes out
//! of GitHub's own documentation rather than out of a probe, which is the exact
//! opposite of [`super`], where every claim is a recorded observation with a
//! date and a version on it.
//!
//! So the honest status of this file is **plausible and unverified**, and the
//! places that would fail first are named where they are chosen rather than
//! collected in a caveat nobody reads. The one that matters most is
//! [`DENIED`], because it is the whole of the read-only claim.
//!
//! ## Four differences, and each one costs the pane something
//!
//! - **There is no streaming-JSON print mode.** Copilot publishes no
//!   `--output-format`, so what comes back is the prose a person would have
//!   read on their own terminal. The pane therefore has no `result` line, which
//!   means **no per-turn cost and no duration** — [`super::AskEvent::Turn`] is
//!   sent with both fields `None` rather than with abeam's own clock dressed up
//!   as the child's measurement.
//! - **There are no tool-call events**, so the `⋯ Read foo.rs · Grep …` line the
//!   Claude pane draws while it waits has nothing to draw. What is left is the
//!   answering counter, which is abeam's own and works either way.
//! - **There is no `system`/`init` line**, so the pane cannot show the tool list
//!   the child actually got. That is the one place this hurts a *promise* rather
//!   than a nicety: `crate::panes::ask`'s standing rule is that the capability
//!   row is the child's answer and never abeam's intention, and against Copilot
//!   there is no answer to show. The row says so in as many words rather than
//!   printing the denylist and letting it read as a confirmation.
//! - **stdin is not a conversation.** A `copilot -p` answers one question and
//!   exits. So a session here is not a process at all — it is a *name*, and the
//!   conversation is carried by `--resume`. See [`CopilotSession`].
//!
//! ## Why the prompt is an argument here and is not next door
//!
//! [`super`] goes to some length to keep the prompt off the command line, and
//! the reason is real: a newline cannot be put on a `cmd.exe` command line in
//! any form, so on a Windows npm install a multi-line question would be refused.
//! Copilot documents a piped-stdin form — `echo "…" | copilot` — that would have
//! the same virtue.
//!
//! It is not used, and the reason is which failure each choice buys:
//!
//! - `-p` is the documented programmatic switch. If it behaves as documented,
//!   everything works except a multi-line question on the one install route that
//!   goes through `cmd.exe`, and that case is **refused with a sentence** by
//!   `crate::launch` before anything starts.
//! - Piped stdin, if it does *not* put the CLI into programmatic mode, leaves a
//!   full-screen Ink application writing its interface into a pane that expects
//!   prose — on every platform and every install route, with nothing to say
//!   about why.
//!
//! One of those is bounded and legible and the other is not, and neither can be
//! checked from here. So the bounded one is taken. If somebody with a `copilot`
//! ever confirms that a pipe is read as a prompt, the pipe is the better shape
//! and this paragraph is the argument to overturn.
//!
//! ## What abeam leaves behind, which the Claude side does not
//!
//! A named Copilot session is written to `~/.copilot/session-state/` and is
//! offered by `copilot --resume` afterwards, so a reader who has used this pane
//! will find abeam's conversations in their own session picker. That is
//! disclosed rather than prevented, for two reasons: there is no documented way
//! to ask for a session that is not persisted, and being able to pick a question
//! up in a real terminal is arguably the better half of the trade. [`name`] is
//! why they are recognisable when they turn up there.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use anyhow::{Result, anyhow};

use super::proto::AskEvent;
use super::{Flavour, Voice, read};
use crate::launch::Launch;

/// The tools this child is refused, and the whole of the read-only claim.
///
/// **A denylist, where [`super::TOOLS`] is an allowlist, and the difference is
/// not cosmetic.** `--tools "Read,Grep,Glob"` means a Claude session in which no
/// other tool *exists*: there is nothing to permit or refuse, and a tool added
/// by a future release arrives switched off. Copilot publishes no equivalent —
/// `--allow-tool` widens a default rather than replacing it — so what abeam can
/// say here is "these five are refused", and a tool kind that ships next month
/// under a sixth name is not covered by this line.
///
/// What makes it worth having anyway is that GitHub documents `--deny-tool` as
/// taking precedence over both `--allow-tool` and `--allow-all-tools`, and abeam
/// never passes `--allow-all-tools` at all. Together with [`NO_ASK_USER`] —
/// which is what stops the child sitting on an approval prompt there is no
/// terminal to answer — the posture is: the destructive kinds are refused
/// outright, and anything else that would need approval cannot get one.
///
/// The five are the tool kinds GitHub's own documentation names. `shell`,
/// `write` and `edit` are the ones that can change this repository. `web_fetch`
/// and `web_search` cannot, and are here for a different reason that is worth
/// stating: the Claude session next door has exactly three tools and none of
/// them reaches the network, so a Copilot pane that could fetch a URL would be
/// quietly wider than the pane it is a second version of — and this pane's whole
/// disclosure is that what leaves the machine is the path above the composer.
///
/// **`str_replace_editor` is deliberately absent**, and that absence is the one
/// judgement call on this line. It appears in GitHub's prose as another name for
/// `edit` rather than as a kind of its own, and the failure mode of naming a
/// tool this release has never heard of is not a narrower session — it is a
/// child that refuses to start, which reads to the user as a broken pane. A
/// denylist entry that might cost the whole feature is not worth an alias that
/// is probably already covered.
pub const DENIED: &[&str] = &["shell", "write", "edit", "web_fetch", "web_search"];

/// What keeps the child from waiting on a person who is not there.
///
/// A `-p` run still has one way to block: asking a clarifying question. There is
/// no terminal behind this pane and no way to answer one, so without this the
/// failure is a turn that never ends and a counter that climbs for ever.
///
/// It is also half of the read-only posture — see [`DENIED`]. A tool that needs
/// approval cannot be approved by a session that may not ask.
const NO_ASK_USER: &str = "--no-ask-user";

/// What GitHub's programmatic-mode page recommends for capturing output:
/// "suppresses session metadata so you get clean text".
///
/// Wanted rather than needed. The transcript is rendered as markdown, so a
/// banner and a session footer would arrive as paragraphs of the answer — but
/// nothing downstream *depends* on this flag, and [`plain`] takes the escape
/// sequences out whether or not it is honoured. A short flag with no documented
/// long form is the weakest-attested thing on this command line, which is why it
/// is the one that costs nothing when it is wrong.
const SILENT: &str = "-s";

/// A conversation with Copilot, which is a name rather than a process.
///
/// The Claude session next door owns a child from the first question to the last
/// and is *over* when that child exits. Nothing of the sort is available here: a
/// `copilot -p` answers one question and goes. So this holds no long-lived
/// child, and [`CopilotSession::is_live`] answers `true` for as long as the
/// pane holds one — which is what stops `crate::app::App::pump_ask` throwing the
/// conversation away and starting a new one after every single turn.
///
/// What carries the conversation instead is the session name: the first question
/// creates it with `--name`, and every question after resumes it. `Ctrl+L` in
/// the pane drops this whole struct, and the next question makes a new name —
/// which is the same "there is no forget, so end the thing holding it" that the
/// Claude side arrives at from the other direction.
pub struct CopilotSession {
    /// The id abeam generated, shared with [`super::AskSession::session_id`] so
    /// that both flavours answer that question the same way. [`name`] is what
    /// actually reaches the command line.
    session_id: String,
    launch: Launch,
    root: PathBuf,
    /// Kept rather than dropped after `start`, because a reader thread is wired
    /// up per turn here rather than once per session.
    say: Sender<AskEvent>,
    events: Receiver<AskEvent>,
    /// The child answering the question in flight, if there is one.
    child: Option<Child>,
    /// A turn has been sent and its stdout has not yet ended. What keeps
    /// [`CopilotSession::poll`] from reporting an exit status before the answer
    /// that came with it: a process can be reaped while its pipe still holds
    /// lines nobody has read.
    answering: bool,
    /// How many questions have gone. Zero means the next one names the session;
    /// anything else means it resumes one.
    asked: usize,
    /// How many times a finished child has been waited on. A test's only way to
    /// prove a reap happened, exactly as the Claude session's is.
    #[cfg(test)]
    reaps: usize,
}

impl CopilotSession {
    /// Start one — which starts nothing.
    ///
    /// Infallible, and that is a real difference from
    /// [`super::AskSession::start`] rather than an oversight: there is no
    /// process to fail to spawn until there is a question to spawn it with. What
    /// would have been a failure at startup is a failure at
    /// [`CopilotSession::ask`] instead, where the pane already has a place to
    /// put the sentence.
    pub fn start(launch: &Launch, root: &Path, session_id: String) -> Self {
        let (say, events) = std::sync::mpsc::channel();
        Self {
            session_id,
            launch: launch.clone(),
            root: root.to_path_buf(),
            say,
            events,
            child: None,
            answering: false,
            asked: 0,
            #[cfg(test)]
            reaps: 0,
        }
    }

    /// The arguments one turn will use, without starting anything.
    ///
    /// Split out for [`super::ClaudeSession::args`]'s reason, and it matters more
    /// here rather than less: this is the complete authority abeam hands a
    /// second agent standing in somebody's repository, and unlike the Claude
    /// list it has never been checked against the program that will receive it.
    /// A test can at least hold it still.
    ///
    /// The order is deliberate and `-p` is last. Everything before it is a flag
    /// abeam wrote; the prompt is the one value on this line that is the user's,
    /// and it arrives as the *value of an option* rather than as a positional —
    /// so a question beginning with a hyphen is a question rather than a flag,
    /// and no `--` fence is needed to say so. `crate::dispatch` needs one
    /// precisely because its prompt is positional.
    ///
    /// `resuming` rather than a look at `asked`, so that the branch a test wants
    /// to pin is an argument rather than a field it has to arrange.
    pub fn args(session_id: &str, resuming: bool, prompt: &str) -> Vec<String> {
        let mut out = Vec::new();
        if resuming {
            // `--resume=<name>` with an equals rather than as two arguments,
            // because bare `--resume` is documented to open a session *picker* —
            // an interactive list, in a pane with no way to answer one. An
            // option whose value is optional is exactly the shape where a
            // separated value can be read as the next argument instead, and the
            // failure there would be abeam hanging on a chooser nobody can see.
            out.push(format!("--resume={}", name(session_id)));
        } else {
            out.push("--name".to_string());
            out.push(name(session_id));
        }
        out.push(SILENT.to_string());
        out.push(NO_ASK_USER.to_string());
        for tool in DENIED {
            out.push("--deny-tool".to_string());
            out.push((*tool).to_string());
        }
        out.push("-p".to_string());
        out.push(prompt.to_string());
        out
    }

    /// Ask one question, which here means starting one child.
    ///
    /// **Not "non-blocking enough" in the sense the Claude side means it.** That
    /// one writes a few hundred bytes onto a pipe with a 64 KiB kernel buffer
    /// and returns before the child has read a byte. This one spawns a process,
    /// on the thread that draws the screen, and a spawn is a syscall that goes to
    /// the filesystem. It is the same cost `crate::panes::shell` pays on the
    /// frame it first draws a shell, and it is paid on a keystroke the reader
    /// made rather than on a bare pass of the loop — but it is a cost, and
    /// pretending otherwise would be the wrong note for this file to end on.
    pub fn ask(&mut self, prompt: &str) -> Result<()> {
        if self.answering {
            // Refused rather than queued. The pane refuses a second question
            // mid-answer as well, for a reason of its own about which exchange
            // the fragments would be filed under; this is the boundary saying
            // the same thing, because two children answering into one channel
            // would interleave two answers with nothing to tell them apart.
            return Err(anyhow!(
                "a question is still being answered, and `copilot` answers one \
                 at a time. Wait for this turn to finish."
            ));
        }

        let planned = plan(&self.launch, &self.session_id, self.asked > 0, prompt)?;
        let mut command = Command::new(&planned.program);
        command
            .args(&planned.args)
            .current_dir(&self.root)
            // **Null rather than a pipe**, and the opposite of the Claude
            // session's choice for the same underlying reason. There the child
            // reads a pipe abeam owns, so it can never reach the console abeam
            // is typing at; here there is nothing to send it after the argument
            // list, and an inherited stdin is a second agent reading the
            // keystrokes meant for the first.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &planned.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|why| {
            anyhow!(
                "abeam could not start `{}` to ask it a question: {why}",
                planned.target.display()
            )
        })?;

        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            // Unreachable through `Stdio::piped()`, and handled rather than
            // unwrapped because the alternative is leaving a live Copilot behind
            // while panicking about not being able to hear it.
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "abeam started `{}` and was handed no pipes to it, so there is \
                 no way to hear the answer. The child has been stopped.",
                planned.target.display()
            ));
        };

        // End of file on stdout is the end of the *turn* here, where on the
        // Claude side it is the end of the session. That is the whole of the
        // difference between a child per conversation and a child per question,
        // and it is why `Ended` is never sent from this file: the conversation
        // outlives the process, in a name.
        read(
            stdout,
            self.say.clone(),
            Voice::Text,
            Some(AskEvent::Turn {
                text: String::new(),
                cost_usd: None,
                duration_ms: None,
                error: None,
            }),
        );
        read(
            stderr,
            self.say.clone(),
            Voice::Err(Flavour::Copilot.agent()),
            None,
        );

        self.child = Some(child);
        self.answering = true;
        self.asked += 1;
        Ok(())
    }

    /// Everything the readers have queued since the last call. **Must not
    /// block**: it is called from `Pane::tick`, on the loop that draws the
    /// agent's screen.
    pub fn poll(&mut self) -> Vec<AskEvent> {
        let mut arrived = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    // The turn's stdout has ended, so whatever the child has to
                    // say has been said and its exit status may now be read
                    // without racing the answer it belongs to.
                    if matches!(event, AskEvent::Turn { .. }) {
                        self.answering = false;
                    }
                    arrived.push(event);
                }
                Err(TryRecvError::Empty) => break,
                // Unreachable while this struct lives: `say` is a sender held
                // here, so the channel cannot lose its last one. Broken out of
                // rather than treated as an ending, because a session here does
                // not end when a pipe does.
                Err(TryRecvError::Disconnected) => break,
            }
        }

        // After the drain and only once the answer is complete. A child can be
        // reaped while its pipe still holds lines nobody has read, and an exit
        // status reported ahead of them would put "`copilot` exited 1" above the
        // sentence explaining why.
        if !self.answering
            && let Some(child) = self.child.as_mut()
            && let Ok(Some(status)) = child.try_wait()
        {
            #[cfg(test)]
            {
                self.reaps += 1;
            }
            self.child = None;
            if !status.success() {
                // Reported rather than swallowed, because a non-zero exit with
                // an empty answer is the shape of an expired credential or a
                // flag this release does not know — and the second of those is
                // the likeliest way this whole module is wrong. See the note at
                // the top about what has and has not been run.
                arrived.push(AskEvent::Broke(exited(&status)));
            }
        }
        arrived
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// **Always true while the session exists**, which is not the Claude
    /// session's answer and is not a stub.
    ///
    /// There, `live` tracks a process, and a dead one means the conversation is
    /// over. Here the conversation is a name in `~/.copilot/session-state/` and
    /// the process is a turn: it is *supposed* to be gone between questions.
    /// `crate::app::App::pump_ask` restarts a session that is not live, so
    /// answering honestly-about-the-process would throw away the name after
    /// every turn and give the reader a Copilot with no memory of the question
    /// before.
    pub fn is_live(&self) -> bool {
        true
    }

    /// Has the child answering the current turn exited? For the tests next door,
    /// which need to know without draining the channel that would say so.
    #[cfg(test)]
    pub fn exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(Some(_))),
            None => true,
        }
    }

    #[cfg(test)]
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    #[cfg(test)]
    pub fn reaps(&self) -> usize {
        self.reaps
    }
}

/// A turn in flight dies with the pane.
///
/// The same decision [`super`] argues at length — this child is part of a
/// window rather than a task that outlives one — and it is a smaller promise
/// here, because between turns there is nothing to kill. What it cannot cover is
/// the same gap named there: on a Windows npm install the process abeam holds is
/// `cmd.exe` and the node underneath it is out of reach of the kill. Unlike the
/// Claude session there is no closed stdin to bring it down either, since stdin
/// was never a pipe. What is left is that a `-p` run finishes on its own, which
/// is a mitigation and not a guarantee — and `abeam_pty`'s job object is the
/// answer on the day it has to be one.
impl Drop for CopilotSession {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// What the session is called on the command line and in the reader's own
/// `copilot --resume` picker afterwards.
///
/// Prefixed rather than bare, because these are persisted where the reader's own
/// sessions are and an unexplained UUID in that list is a thing somebody has to
/// work out. `abeam-ask-` says which program made it and what it was for.
///
/// The id itself is `crate::ask::new_session_id`'s, whose own docs are worth
/// reading for what it is *not*: an identifier, never a secret.
fn name(session_id: &str) -> String {
    format!("abeam-ask-{session_id}")
}

/// Exactly what will be started for one turn: which program, with which
/// arguments, and what has to be in its environment for those arguments to
/// arrive.
///
/// The same shape as [`super::plan`] and `crate::dispatch::plan`, and the same
/// reason for rebuilding a Windows command line rather than appending to it:
/// `Launch::args` for a routed `.cmd` is `/e:ON /v:OFF /d /c %ABEAM_LAUNCH%`
/// and the real command line is inside that variable.
///
/// **This one can fail on the user's own text, and the other two cannot.** The
/// prompt is on this command line, so on a Windows npm install it goes through
/// `cmd.exe` — which cannot carry a newline in any form and runs nothing at all
/// past 8191 characters. `crate::launch` refuses both with a sentence; what is
/// added here is the half that sentence cannot know, which is that the argument
/// in question is the question, and that the same question asked of Claude would
/// have gone through.
fn plan(launch: &Launch, session_id: &str, resuming: bool, prompt: &str) -> Result<Launch> {
    let args = CopilotSession::args(session_id, resuming, prompt);

    // Windows-only for `crate::dispatch`'s reason: on Unix `crate::launch`
    // cannot produce a program that is not its own target, so an arm left
    // always-false there would be an arm that cannot carry an error message
    // about `cmd.exe` to a reader who has none.
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
                 script and has to be run by cmd.exe: {why}\n\nYour question is \
                 on that command line, because Copilot CLI publishes no way to \
                 read one from anywhere else. A newline is what usually does \
                 this: ask it as one line, or install `copilot` as a program \
                 rather than through npm.",
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

/// One line of a Copilot answer, as the transcript can hold it.
///
/// **The escape sequences come out here rather than being asked away with a
/// flag**, and that is a decision rather than a belt-and-braces afterthought.
/// `--no-color` is not a flag this module can point at documentation for, and a
/// flag `copilot` does not accept is not a slightly noisier pane — it is a child
/// that refuses to start. So the one thing abeam can do without guessing is done
/// on the bytes it has already received.
///
/// Two things need it, and only one of them is cosmetic. The transcript is
/// rendered as markdown, so a stray `ESC[1m` arrives as text nobody wrote. The
/// other is `crate::panes::ask::scan`, which refuses to offer any fenced block
/// carrying a control character — for a good reason about bracketed paste, and
/// with the consequence that a coloured answer would have every command in it
/// silently refused. Taking the escapes out before either sees the line is what
/// keeps that refusal about dangerous blocks rather than about ANSI.
///
/// A CSI sequence is `ESC [` then parameter and intermediate bytes then one
/// byte in `@`–`~`; an OSC is `ESC ]` up to a `BEL` or an `ESC \`. Anything
/// else beginning `ESC` has its `ESC` dropped and its text kept, which is the
/// lossy-but-visible direction: a reader seeing one stray letter can tell what
/// happened, where a swallowed line looks like an answer that stopped.
fn plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            // Every other control character goes too, and the tab stays: a
            // vertical tab or a backspace in a transcript is a row that does not
            // say what it looks like, and a tab is indentation.
            if !c.is_control() || c == '\t' {
                out.push(c);
            }
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // Up to and including the final byte, which is the first one in
                // `@`–`~`. A sequence that simply ends — a truncated line — takes
                // the rest with it, which is the same answer as reading it as
                // text a terminal would have eaten.
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // A bare `ESC`, or one in front of something this does not model.
            // The `ESC` goes and what follows stays.
            _ => {}
        }
    }
    out
}

/// One line of Copilot's stdout as an event, or as nothing.
///
/// Every line is answer, which is the whole of the protocol here and the reason
/// this function is three lines where `crate::ask::proto::parse_line` is a
/// hundred: there is no envelope to be unfamiliar with, no `type` to be
/// unrecognised, and nothing that can fail to parse. What that costs is that
/// abeam cannot tell an answer from a progress note the CLI happened to print —
/// see [`SILENT`], which is the flag that is supposed to mean there are none.
///
/// The newline goes back on, because [`super::read`] took it off and the
/// transcript is a markdown document where a line ending is the difference
/// between a list and a paragraph.
pub(super) fn said(line: &str) -> Option<AskEvent> {
    let text = plain(line);
    // A line that was nothing but escapes is not a blank line the child wrote,
    // it is a cursor move. Dropping it keeps a paragraph from growing a hole.
    (!text.is_empty() || !line.contains('\u{1b}')).then(|| AskEvent::Delta(format!("{text}\n")))
}

/// What abeam says about a child that answered and then failed.
fn exited(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!(
            "`copilot` exited {code} after that question. An answer above it, if \
             there is one, is still whatever it said before it stopped."
        ),
        // Unix, killed by a signal. `ExitStatus` has no portable way to name
        // which one and the honest sentence does not need it.
        None => "`copilot` was killed before it finished that question.".to_string(),
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// **Nothing here starts a `copilot`**, which is this file's version of a rule
/// [`super`] states for the same reason — a real one would cost somebody's quota
/// on every `cargo test` — and which holds here for a second reason that is
/// worth naming: there is no `copilot` on the machine this was written on, so a
/// test that needed one would not be slow, it would be a test that has never
/// run. What can be tested is what this module *decides*: an argument list, a
/// command line, and what it makes of a line of text.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    #[test]
    fn the_argument_list_is_the_read_only_claim_and_is_asserted_whole() {
        // The security assertion of this module, written as one — the same shape
        // and the same standard as `crate::ask`'s, and held to it more tightly
        // rather than less because this list has never been checked against the
        // program that will receive it.
        //
        // If you are here because this test is in the way of an edit: the
        // question is not "is this flag useful". It is "would somebody who was
        // told this pane can only read agree that it still can".
        assert_eq!(
            CopilotSession::args("3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e", false, "why?"),
            args(&[
                "--name",
                "abeam-ask-3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e",
                "-s",
                "--no-ask-user",
                "--deny-tool",
                "shell",
                "--deny-tool",
                "write",
                "--deny-tool",
                "edit",
                "--deny-tool",
                "web_fetch",
                "--deny-tool",
                "web_search",
                "-p",
                "why?",
            ])
        );

        // And the flag that would undo every `--deny-tool` above it, named so
        // that adding it has to come through this line. GitHub documents deny as
        // taking precedence over it, and abeam is not going to find out.
        let list = CopilotSession::args("x", false, "q");
        for forbidden in [
            "--allow-all-tools",
            "--allow-tool",
            "--allow-all-paths",
            "--experimental",
        ] {
            assert!(
                !list.iter().any(|arg| arg == forbidden),
                "`{forbidden}` reached the command line of the read-only pane"
            );
        }

        // Every kind that can change a file is refused, spelled here as well as
        // in the whole list above so that a diff which drops one is visibly
        // dropping *that* rather than editing a vector.
        for writes in ["shell", "write", "edit"] {
            assert!(
                DENIED.contains(&writes),
                "`{writes}` is not on the denylist"
            );
        }
    }

    #[test]
    fn the_second_question_resumes_the_first_ones_session() {
        // The same question both times, so that the only difference between the
        // two lists is the one this test is about.
        let first = CopilotSession::args("abcd", false, "why?");
        let again = CopilotSession::args("abcd", true, "why?");

        // The name is created once and resumed after, and both spellings carry
        // the same name — a `--resume` of something never named is a session
        // that does not exist, and Copilot would answer question two with no
        // memory of question one while the pane went on calling it a
        // conversation.
        assert_eq!(first[0], "--name");
        assert_eq!(first[1], "abeam-ask-abcd");
        assert_eq!(again[0], "--resume=abeam-ask-abcd");

        // With an equals, and never as two arguments: a bare `--resume` opens an
        // interactive session picker, and there is nobody here to pick.
        assert!(!again.iter().any(|arg| arg == "--resume"));

        // Everything after the first argument is the same on both, which is what
        // makes the resumed turn as narrow as the one that opened the session.
        assert_eq!(first[2..], again[1..]);
    }

    #[test]
    fn the_question_is_the_value_of_an_option_and_never_a_positional() {
        // Nothing on this line is the user's except the prompt, and the prompt
        // arrives as the value of `-p`. That is why there is no `--` fence here
        // and why `crate::dispatch` needs one: a positional prompt beginning
        // with a hyphen is a flag, and this one cannot be.
        let list = CopilotSession::args("x", false, "--allow-all-tools");
        assert_eq!(list.last().map(String::as_str), Some("--allow-all-tools"));
        assert_eq!(list[list.len() - 2], "-p");
        assert_eq!(
            list.iter().filter(|arg| arg.as_str() == "--").count(),
            0,
            "a fence appeared on a command line with no positional on it"
        );
        // And it is the *value*, so it has not become a flag on the way: the
        // only `--allow-all-tools` on this line is the one being quoted.
        assert_eq!(
            list.iter()
                .filter(|arg| arg.as_str() == "--allow-all-tools")
                .count(),
            1
        );
    }

    #[test]
    fn a_multi_line_question_survives_everywhere_a_command_line_can_carry_one() {
        // The one thing `-p` costs, pinned from the side that still works. A
        // newline is fine in an argument to anything started directly — on both
        // platforms — and is refused only on the Windows npm route, which is
        // `plan`'s subject and `crate::launch`'s message.
        let list = CopilotSession::args("x", false, "why is this\n\nslow?");
        assert_eq!(
            list.last().map(String::as_str),
            Some("why is this\n\nslow?")
        );
    }

    // --- what it makes of a line -------------------------------------------

    #[test]
    fn colour_comes_out_and_the_words_stay() {
        // The whole reason `plain` exists: an answer wearing SGR codes would
        // reach a markdown renderer as text nobody wrote, and would have every
        // fenced command in it refused by `panes::ask::scan` for carrying a
        // control character.
        assert_eq!(plain("\u{1b}[1mbold\u{1b}[0m words"), "bold words");
        assert_eq!(plain("\u{1b}[38;5;204mpink\u{1b}[m"), "pink");
        // An OSC, which ends at a BEL or at a string terminator rather than at a
        // letter — read as a CSI it would have swallowed the title and stopped
        // at the `s` of `set`.
        assert_eq!(plain("\u{1b}]0;set a title\u{7}after"), "after");
        assert_eq!(plain("\u{1b}]0;title\u{1b}\\after"), "after");
        // Ordinary text is untouched, tabs included: a tab is indentation and a
        // transcript is markdown.
        assert_eq!(plain("\tcargo test --all"), "\tcargo test --all");
        // And a control character that is not part of any sequence goes, because
        // a row that does not say what it looks like is the thing being
        // prevented.
        assert_eq!(plain("a\u{8}b\u{7}c"), "abc");
    }

    #[test]
    fn every_line_is_answer_and_a_line_of_pure_escapes_is_not_a_line() {
        // There is no envelope here, so the only judgement this makes is that a
        // line which was nothing but a cursor move is not a blank line the child
        // wrote — a paragraph with a hole in it is a worse answer than one
        // without.
        let Some(AskEvent::Delta(text)) = said("the answer") else {
            panic!("a line of prose is an answer");
        };
        assert_eq!(text, "the answer\n", "the newline the reader took off");

        assert_eq!(said("\u{1b}[2K\u{1b}[1G"), None);
        // A genuinely blank line *is* one: it is the difference between a list
        // and a paragraph in the document this becomes.
        assert_eq!(said(""), Some(AskEvent::Delta("\n".to_string())));
    }

    #[test]
    fn a_session_is_a_name_rather_than_a_process() {
        // The claim `is_live` makes, and the one `crate::app::App::pump_ask`
        // reads: a Copilot session between turns holds no child and is still a
        // conversation. Answering otherwise would have the app drop the name
        // after every turn, and the reader would be told they were in a
        // conversation with something that remembered nothing.
        let launch = fake();
        let mut session = CopilotSession::start(&launch, Path::new("/repo"), "abcd".to_string());
        assert!(session.is_live());
        assert_eq!(
            session.pid(),
            None,
            "nothing is started until a question is"
        );
        assert!(session.exited(), "there is no child to be waiting on");
        assert!(session.poll().is_empty());
        assert!(
            session.is_live(),
            "a poll with nothing in it is not an ending"
        );
        assert_eq!(session.session_id(), "abcd");
    }

    /// The `Launch` a direct install produces, fabricated rather than found: what
    /// is under test is the argument list, not the machine — and on this machine
    /// there is no `copilot` to find.
    fn fake() -> Launch {
        #[cfg(windows)]
        let exe = PathBuf::from(r"C:\Program Files\GitHub Copilot\copilot.exe");
        #[cfg(unix)]
        let exe = PathBuf::from("/usr/local/bin/copilot");
        Launch {
            program: exe.clone(),
            target: exe,
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    #[test]
    fn a_direct_copilot_is_started_with_abeams_own_arguments_and_nothing_else() {
        let planned = plan(&fake(), "abcd", false, "why?").expect("a direct launch always plans");
        assert_eq!(planned.program, fake().program);
        assert_eq!(
            planned.target, planned.program,
            "a program started directly is its own target"
        );
        assert_eq!(planned.args, CopilotSession::args("abcd", false, "why?"));
        assert!(
            planned.env.is_empty(),
            "an executable's arguments travel as arguments"
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_npm_copilot_has_its_command_line_rebuilt_and_refuses_a_newline() {
        use crate::launch;
        use crate::testutil::TempDir;

        let dir = TempDir::new("ask-copilot-npm");
        let script = dir.write("abeam-copilot.cmd", b"@echo off\r\n");
        let resolved = launch::resolve(&script.to_string_lossy(), &[]).expect("a .cmd is routed");

        let planned = plan(&resolved, "abcd", false, "why?").expect("one line always plans");
        // Not one token longer: the flags are inside the variable, not appended
        // after the expansion where `cmd` would read them as its own.
        assert_eq!(planned.args, resolved.args);
        let (key, line) = planned.env.first().expect("a command line in a variable");
        assert_eq!(key, "ABEAM_LAUNCH");
        assert!(line.contains("--deny-tool"), "got: {line}");
        assert!(line.contains("-p"), "got: {line}");
        assert_eq!(line.matches("--name").count(), 1);

        // And the bounded failure this whole shape was chosen for, said in the
        // words the reader needs: the question is on the command line, `cmd.exe`
        // cannot carry a newline in any form, and there is a way through.
        let refused = plan(&resolved, "abcd", false, "why is this\n\nslow?")
            .expect_err("a newline cannot go through cmd.exe");
        let said = format!("{refused:#}");
        assert!(said.contains("newline"), "got: {said}");
        assert!(said.contains("one line"), "got: {said}");
    }

    // --- and it actually starts, one child at a time ------------------------
    //
    // Everything above is a claim about a `Vec<String>`. What is claimed below
    // is about *processes*, plural, which is the whole difference between this
    // module and the one next door: there a session is one child and here it is
    // one per question. A shim stands in for `copilot`, exactly as
    // `crate::ask`'s spawning tests use one for `claude` — and here it is not
    // merely thrift, since there is no `copilot` on this machine to be thrifty
    // with.

    /// A shim that prints its own arguments and one line of prose, then exits.
    ///
    /// Printing the arguments is the point: it is the only way to see what the
    /// *second* child was started with, and the second child is where the whole
    /// conversation claim lives. A `.cmd` in a directory with a space in its
    /// name on Windows, because the path goes onto a command line as text; a
    /// `#!/bin/sh` with the execute bit on Unix, because a `#!` file without one
    /// is `EACCES` at the spawn.
    #[cfg(windows)]
    fn shim(dir: &TempDir, code: u8) -> PathBuf {
        let home = dir.path().join("with space");
        std::fs::create_dir_all(&home).expect("a directory with a space in it");
        let script = home.join("abeam-copilot.cmd");
        std::fs::write(
            &script,
            format!("@echo off\r\necho ARGS %*\r\necho the answer\r\nexit /b {code}\r\n"),
        )
        .expect("write a shim");
        script
    }
    #[cfg(unix)]
    fn shim(dir: &TempDir, code: u8) -> PathBuf {
        dir.write_exec(
            "abeam-copilot",
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"ARGS $*\"\nprintf '%s\\n' 'the answer'\nexit {code}\n"
            )
            .as_bytes(),
        )
    }

    /// Poll until `enough` is satisfied, or give up loudly.
    ///
    /// A bounded wait rather than a sleep of a fixed length, for the reason
    /// `crate::ask`'s own version gives: the reader threads and the child are
    /// both real, so how long anything takes is the machine's business, and a
    /// test that times out with no account of what arrived is one somebody
    /// deletes.
    fn wait_for(
        session: &mut CopilotSession,
        what: &str,
        enough: impl Fn(&[AskEvent]) -> bool,
    ) -> Vec<AskEvent> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut seen = Vec::new();
        while std::time::Instant::now() < deadline {
            seen.extend(session.poll());
            if enough(&seen) {
                return seen;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("waited ten seconds for {what} and saw only: {seen:#?}");
    }

    /// Everything the child said, joined the way the pane would show it.
    fn answer(seen: &[AskEvent]) -> String {
        seen.iter()
            .filter_map(|event| match event {
                AskEvent::Delta(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn through(script: &Path, root: &Path) -> CopilotSession {
        let resolved =
            crate::launch::resolve(&script.to_string_lossy(), &[]).expect("the shim resolves");
        CopilotSession::start(&resolved, root, "abcd".to_string())
    }

    #[test]
    fn a_conversation_survives_the_child_that_answered_the_last_question() {
        // The claim the whole shape rests on, asked of two real processes. A
        // `copilot -p` exits when it has answered; if `is_live` reported that,
        // `crate::app::App::pump_ask` would throw the session away and the
        // second question would go to a Copilot with no memory of the first.
        // What carries the conversation is the name, and this is where the two
        // spellings of it are seen on a command line rather than in a vector.
        let dir = crate::testutil::TempDir::new("copilot-turns");
        let script = shim(&dir, 0);
        let mut session = through(&script, dir.path());

        session.ask("one").expect("the shim starts");
        let seen = wait_for(&mut session, "the first turn to end", |seen| {
            seen.iter().any(|e| matches!(e, AskEvent::Turn { .. }))
        });
        let first = answer(&seen);
        assert!(first.contains("the answer"), "got: {first}");
        assert!(
            first.contains("--name"),
            "the session was not named: {first}"
        );
        assert!(
            first.contains("abeam-ask-abcd"),
            "the name did not reach the child: {first}"
        );

        // The turn ended and the conversation did not. This is the assertion
        // `is_live` exists for.
        assert!(
            session.is_live(),
            "one answered question ended the conversation"
        );
        // And exactly one `Turn`: end of file on stdout is the end of the turn,
        // and nothing else here sends one.
        assert_eq!(
            seen.iter()
                .filter(|e| matches!(e, AskEvent::Turn { .. }))
                .count(),
            1
        );
        // Neither field is invented. abeam has its own clock and does not dress
        // it up as the child's measurement, and Copilot reports no cost at all —
        // so a turn here is unlabelled rather than labelled with a guess.
        assert!(seen.iter().any(|e| matches!(
            e,
            AskEvent::Turn {
                cost_usd: None,
                duration_ms: None,
                ..
            }
        )));

        // Reaped, in a loop of its own rather than through `wait_for`: the
        // question is about the session rather than about what it has said, and
        // bounded rather than asked once because the `Turn` says stdout has
        // closed and the process has a few instructions left to run after that.
        // A child that has exited and never been waited on is a zombie on Unix
        // for as long as abeam runs — the failure `crate::ask` measured at zero
        // reaps across five hundred polls and documents at length.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while session.reaps() == 0 && std::time::Instant::now() < deadline {
            session.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            session.reaps() > 0,
            "the child that answered was never waited on"
        );

        session
            .ask("two")
            .expect("the second question starts another child");
        let seen = wait_for(&mut session, "the second turn to end", |seen| {
            seen.iter().any(|e| matches!(e, AskEvent::Turn { .. }))
        });
        let second = answer(&seen);
        assert!(
            second.contains("--resume=abeam-ask-abcd"),
            "the second question did not resume the first one's session: {second}"
        );
        assert!(
            !second.contains("--name"),
            "the second question named a session that already exists: {second}"
        );
    }

    #[test]
    fn a_child_that_answers_and_then_fails_says_so_after_the_answer() {
        // Ordering, which is the one thing `answering` exists for. A process can
        // be reaped while its pipe still holds lines nobody has read, so an exit
        // status reported eagerly would put "`copilot` exited 3" *above* the
        // sentence explaining why — and a reader would be looking at a complaint
        // with the answer to it underneath.
        let dir = crate::testutil::TempDir::new("copilot-exit");
        let script = shim(&dir, 3);
        let mut session = through(&script, dir.path());

        session.ask("one").expect("the shim starts");
        let seen = wait_for(&mut session, "the failure to be reported", |seen| {
            seen.iter()
                .any(|e| matches!(e, AskEvent::Broke(said) if said.contains("exited 3")))
        });

        let turn = seen
            .iter()
            .position(|e| matches!(e, AskEvent::Turn { .. }))
            .expect("the turn ended before its status was read");
        let broke = seen
            .iter()
            .position(|e| matches!(e, AskEvent::Broke(said) if said.contains("exited 3")))
            .expect("waited for it");
        assert!(
            broke > turn,
            "the exit status was reported before the answer it belongs to: {seen:#?}"
        );
        assert!(
            answer(&seen).contains("the answer"),
            "the answer was lost to the failure: {seen:#?}"
        );
        // A failed turn is not a failed conversation: the next question starts
        // another child, and `--resume` of a session that was named is still a
        // session.
        assert!(session.is_live());
    }

    #[test]
    fn one_question_at_a_time_is_refused_at_the_boundary_as_well_as_in_the_pane() {
        // Two children answering into one channel would interleave two answers
        // with nothing to tell them apart, and the `Turn` from the first would
        // close the second. The pane refuses a question mid-answer for a reason
        // of its own; this is the boundary saying the same thing, so that the
        // rule does not rest on the one caller that happens to keep it.
        let dir = crate::testutil::TempDir::new("copilot-busy");
        let script = shim(&dir, 0);
        let mut session = through(&script, dir.path());

        session.ask("one").expect("the shim starts");
        let why = session
            .ask("two")
            .expect_err("a second question during a turn is refused")
            .to_string();
        assert!(why.contains("one at a time"), "got: {why}");

        wait_for(&mut session, "the turn to end", |seen| {
            seen.iter().any(|e| matches!(e, AskEvent::Turn { .. }))
        });
        session
            .ask("two")
            .expect("and is taken once the turn has ended");
    }
}
