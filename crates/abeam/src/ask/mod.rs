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
//! - The child wrote `~/.claude/sessions/<pid>.json` with `"kind":"interactive"`
//!   and abeam's own `cwd`. See the section below, which is the most important
//!   thing in this file.
//! - Hooks still ran. `--strict-mcp-config` is about MCP servers and does not
//!   disable them.
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

mod proto;

use std::path::Path;

use anyhow::Result;

use crate::launch::Launch;

pub use proto::{AskEvent, new_session_id};

/// The tools the child is given, and the whole of the read-only claim.
///
/// An allowlist over the built-in set: what is not named here does not exist
/// for that session, so there is no `Write`, no `Edit`, no `Bash` to permit or
/// refuse. Stated as a constant rather than built at the call site so that the
/// one thing which must never drift — what authority a second agent is handed —
/// is a line a test can assert against, exactly as `crate::dispatch` does for
/// the authority *it* hands out.
pub const TOOLS: &str = "Read,Grep,Glob";

/// A live child, its pipes, and the reader threads draining them.
pub struct AskSession {
    _private: (),
}

impl AskSession {
    /// Start one. `root` becomes the child's working directory, and
    /// `session_id` becomes both the child's `--session-id` and the name the
    /// probe next door is told to disown.
    pub fn start(launch: &Launch, root: &Path, session_id: String) -> Result<Self> {
        let _ = (launch, root, session_id);
        todo!("implementer A")
    }

    /// The arguments [`start`](Self::start) will use, without starting
    /// anything. Split out for the same reason `crate::dispatch::plan` is: the
    /// authority handed to a second agent is the one thing here that must never
    /// drift, and this is what a test can hold still.
    pub fn args(session_id: &str) -> Vec<String> {
        let _ = session_id;
        todo!("implementer A")
    }

    /// Write one turn to the child's stdin.
    pub fn ask(&mut self, prompt: &str) -> Result<()> {
        let _ = prompt;
        todo!("implementer A")
    }

    /// Everything the readers have queued since the last call. **Must not
    /// block**: it is called from `Pane::tick`, on the loop that draws the
    /// agent's screen.
    pub fn poll(&mut self) -> Vec<AskEvent> {
        todo!("implementer A")
    }

    /// The id abeam gave it, which is what `crate::agentstate` disowns.
    pub fn session_id(&self) -> &str {
        todo!("implementer A")
    }

    pub fn is_live(&self) -> bool {
        todo!("implementer A")
    }
}
