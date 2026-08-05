//! The wire, and nothing else.
//!
//! Every function here is pure over a `&str`, so the whole protocol is testable
//! without starting a process. That split is not tidiness: the shape below is
//! **not a published contract**. Claude's CLI reference documents that
//! `--input-format stream-json` exists and documents the SDK layered over it;
//! it does not specify the JSON. What is written here came off one run against
//! 2.1.222 on 2026-08-05, recorded in `crate::ask`'s module docs, and the honest
//! consequence is that a version bump can change it under us. A parser that is
//! one pure function per line shape is one somebody can re-point at a new
//! sample in an afternoon.
//!
//! The rule that follows from not owning the format: **an unknown line is not
//! an error.** New `type` values are how a format grows, and a pane that shows
//! a scary sentence every time Claude adds a message type would be wrong more
//! often than it was right. Only a line that is not JSON at all, or one whose
//! shape contradicts what it claims to be, is [`AskEvent::Broke`].

use serde::Deserialize;

/// One line of the child's stdout, reduced to what a pane can act on.
#[derive(Clone, Debug, PartialEq)]
pub enum AskEvent {
    /// `system`/`init`. Carries the tools the child *actually* got, which is
    /// what the pane draws — the read-only claim is a claim about this list,
    /// so it is shown rather than asserted.
    Ready {
        session_id: String,
        model: String,
        tools: Vec<String>,
    },
    /// A `text_delta` fragment. Never a thinking delta and never a tool-input
    /// delta; see [`parse_line`].
    Delta(String),
    /// `result` — the end of a turn, and the only reliable signal of one.
    Turn {
        text: String,
        cost_usd: Option<f64>,
        error: Option<String>,
    },
    /// A rate limit, reduced to something sayable.
    RateLimited(String),
    /// stdout closed: the child is gone.
    Ended,
    /// A line that could not be read, or the reader itself failing. Carries
    /// what can honestly be said about it and is never silently dropped.
    Broke(String),
}

/// Turn one line of stdout into an event, or `None` for a line that is
/// well-formed and simply not interesting.
pub fn parse_line(line: &str) -> Option<AskEvent> {
    let _ = line;
    todo!("implementer A")
}

/// The JSON line that carries one user turn to the child's stdin, **without**
/// its newline.
///
/// The prompt goes in as a serialised JSON string, which is the whole reason
/// this shape was chosen over `crate::dispatch`'s: a newline here is two bytes
/// inside a quoted string on one line of stdin, where on a `cmd.exe` command
/// line it is the end of the command and cannot be escaped at all.
pub fn turn(prompt: &str) -> String {
    let _ = prompt;
    todo!("implementer A")
}

/// A valid v4-shaped UUID, unique per call within this process.
///
/// Not a dependency: `uuid` would be a whole crate to format sixteen bytes, and
/// the only property needed here is that two abeam windows and two workspaces
/// in one window never collide. Built from the pid, the clock and a counter,
/// with the version and variant nibbles set so Claude's own validation accepts
/// it.
pub fn new_session_id() -> String {
    todo!("implementer A")
}

/// The `Deserialize` shapes are private to this module on purpose: nothing
/// outside it should be able to grow an opinion about a format abeam does not
/// own.
#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
}
