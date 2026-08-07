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
//!
//! ## Which of the six line shapes carries the answer, and why two do not
//!
//! Four `type` values reach the pane and the rest are dropped. Two of the drops
//! are decisions rather than gaps, and both are about the same text arriving
//! more than once.
//!
//! **`assistant` produces nothing.** A completed assistant message repeats,
//! whole, the text the `text_delta` fragments have already put on screen — so
//! turning it into an event would draw the answer twice, and turning it into a
//! *replacement* for the deltas would mean the transcript sat empty until the
//! model finished thinking, which is the entire thing `--include-partial-
//! messages` is passed to avoid. The authoritative final text is on the
//! `result` line, which arrives after it and which [`AskEvent::Turn`] carries.
//!
//! That last part is what makes the drop safe rather than merely tidy, and it
//! is worth being explicit about because it is the one place this file has a
//! single point of failure. If `--include-partial-messages` were ever refused
//! or quietly stopped being honoured, no `stream_event` would arrive at all and
//! the transcript would go empty — and `Turn.text` is what the pane falls back
//! to, in one piece at the end of the turn instead of a word at a time. So the
//! answer reaches the reader on either route, and it reaches them twice on
//! neither.
//!
//! **`user` produces nothing either**, for a nearer reason: on this session it
//! is a tool result coming back — the child has `Read`, `Grep` and `Glob` — or
//! the child's own echo of what was written to its standard input. Neither is
//! an answer to the question, and a transcript that showed the contents of
//! every file the child opened would bury the sentence the reader asked for.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::text::clip;

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

/// The delta that is an answer, out of the several that are not.
///
/// A `thinking_delta` is the model's reasoning and a `input_json_delta` is the
/// arguments of a tool call being assembled a character at a time — a path, a
/// regex, half a JSON object. Both would land in the transcript as text
/// somebody would read as the answer, and the tool-input one would show a
/// half-built call as though the child had said it. The filter is on this one
/// word and everything else is dropped, rather than the other way round: a
/// delta shape nobody has seen yet is far likelier to be another kind of
/// thinking than another kind of answer.
const TEXT_DELTA: &str = "text_delta";

/// Turn one line of stdout into an event, or `None` for a line that is
/// well-formed and simply not interesting.
///
/// The three answers are worth reading as a set, because which failure gets
/// which is the whole of the module's stance on a format abeam does not own:
///
/// - `None` — a line that parsed and that this version has no use for. An
///   unfamiliar `type`, a `system` line that is not the `init` one, a delta
///   that is not text, an `assistant` message, a blank line.
/// - `Some(Broke(_))` — a line that could not be read *as this format*: not
///   JSON at all, JSON carrying no `type`, or a `result` whose fields
///   contradict their names. Shown to the reader rather than dropped, because
///   the alternative is a pane that goes quiet and says nothing about why.
/// - anything else — a line this version understands.
pub fn parse_line(line: &str) -> Option<AskEvent> {
    // A blank line is not a message and never was. `BufRead::lines` hands one
    // back for the newline that terminates the last line of a stream, and a
    // child that prints a blank line between messages is not saying anything.
    if line.trim().is_empty() {
        return None;
    }

    // Through `Value` first rather than straight into a typed shape, because
    // the two failures below are different sentences and a single
    // `from_str::<Envelope>` cannot tell them apart: it fails identically for
    // `not json at all` and for `{"session_id":"…"}`, and only one of those
    // suggests the reader is looking at output from something that is not
    // Claude.
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(why) => return Some(AskEvent::Broke(not_json(line, &why))),
    };
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Some(AskEvent::Broke(no_type(line)));
    };

    match kind {
        "system" => init(&value),
        "stream_event" => delta(&value),
        "result" => Some(finished(&value)),
        "rate_limit_event" => Some(AskEvent::RateLimited(sayable(&value))),
        // The two that are dropped on purpose — see the module docs, which are
        // the argument rather than this line.
        "assistant" | "user" => None,
        // And everything else, which is how a format grows. A `type` this
        // version has never heard of is Claude having added a message, not
        // Claude having gone wrong, and the pane says nothing about it.
        _ => None,
    }
}

/// The JSON line that carries one user turn to the child's stdin, **without**
/// its newline.
///
/// The prompt goes in as a serialised JSON string, which is the whole reason
/// this shape was chosen over `crate::dispatch`'s: a newline here is two bytes
/// inside a quoted string on one line of stdin, where on a `cmd.exe` command
/// line it is the end of the command and cannot be escaped at all.
///
/// Built through `serde_json` rather than by `format!`, and that is the point
/// of the two structs below existing for four fixed strings. A hand-written
/// `{{"type":"user",…,"content":"{prompt}"}}` is one line shorter and is wrong
/// for every prompt containing a double quote, a backslash, a newline or a tab
/// — which is to say for ordinary questions about code, since a quoted
/// identifier and a pasted snippet both carry two of those. What it would
/// produce is not a mangled question but a *malformed line*, which the child
/// would refuse whole; and the fix somebody would reach for is a hand-rolled
/// escape table, which is the thing this codebase pays a JSON dependency
/// precisely to avoid (see the manifest's note beside `serde_json`).
pub fn turn(prompt: &str) -> String {
    let line = UserTurn {
        kind: "user",
        message: Said {
            role: "user",
            content: prompt,
        },
    };
    // Infallible for this shape: serialisation fails on a map with non-string
    // keys, on a float that is not a number, or on an `io::Error` from a
    // writer, and there are none of the three here — every field is a `&str`
    // going into a `String`. `expect` rather than a fallback because a fallback
    // would be a line nobody could write a correct version of.
    serde_json::to_string(&line).expect("a struct of four strings always serialises")
}

/// A valid v4-shaped UUID, unique per call within this process.
///
/// Not a dependency: `uuid` would be a whole crate to format sixteen bytes, and
/// the only property needed here is that two abeam windows and two workspaces
/// in one window never collide. Built from the pid, the clock and a counter,
/// with the version and variant nibbles set so Claude's own validation accepts
/// it.
///
/// **This is an identifier, not a secret**, and the three sources are chosen
/// for the three ways two of these could otherwise meet rather than for
/// unpredictability. The counter separates two calls in one process; the pid
/// separates two processes running at the same moment, because an operating
/// system does not hand the same pid to two live processes; and the clock
/// separates this process from the one that had its pid an hour ago. A `rand`
/// would be a fourth answer to a question already answered three times. Nothing
/// downstream may treat this as unguessable: what it is used for is telling
/// abeam's own session records apart from the hosted agent's
/// (`crate::agentstate`), and a reader who can guess it can already read the
/// directory it names.
pub fn new_session_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&std::process::id().to_be_bytes());
    // The low 64 bits of the nanoseconds since the epoch. A clock set before
    // 1970 is the only way this fails, and it falls back to zero rather than
    // refusing: uniqueness within a run is the counter's job and uniqueness
    // between concurrent runs is the pid's, so a broken clock costs the one
    // guarantee that is about *past* runs of abeam on the same machine.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64);
    bytes[4..12].copy_from_slice(&nanos.to_be_bytes());
    bytes[12..16].copy_from_slice(&COUNTER.fetch_add(1, Ordering::Relaxed).to_be_bytes());

    // The two nibbles that make this a *v4-shaped* identifier rather than
    // sixteen bytes: version 4 in the high half of byte 6, and the RFC 4122
    // variant — `10xx`, which is the hex digits 8, 9, a and b — in the high
    // bits of byte 8. Claude validates the shape and takes the value, so these
    // four bits are the whole of what stands between this function and a
    // `--session-id` that is refused at startup.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let mut out = String::with_capacity(36);
    for (at, byte) in bytes.iter().enumerate() {
        if matches!(at, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        // Infallible: writing to a `String` cannot fail, and there is no other
        // error a `Display` of a `u8` can produce.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// ---------------------------------------------------------------------------
// the line shapes
// ---------------------------------------------------------------------------

/// The `Deserialize` shapes are private to this module on purpose: nothing
/// outside it should be able to grow an opinion about a format abeam does not
/// own. Every field is optional for `crate::agentstate`'s reason — a record
/// abeam does not own is one whose fields may move — and none of them is
/// `deny_unknown_fields`, because the real `init` line carries a dozen fields
/// beyond these four.
#[derive(Deserialize)]
struct Init {
    subtype: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    tools: Option<Vec<String>>,
}

/// The `result` line, which is the end of a turn and the only reliable signal
/// of one.
///
/// `api_error_status` is a [`Value`] rather than a `String` or a `u16` because
/// the one line anybody has captured has it as `null` and nothing says what it
/// is when it is not. Reading it as the wrong type would make an *error* line
/// unreadable, which is the worst line to lose: it is the one the reader is
/// waiting for an explanation from.
#[derive(Deserialize)]
struct Finish {
    result: Option<String>,
    total_cost_usd: Option<f64>,
    is_error: Option<bool>,
    subtype: Option<String>,
    api_error_status: Option<Value>,
}

/// One turn on the way in. Two structs so that `serde` decides the field order
/// and the escaping, and so the line this produces reads exactly like the shape
/// [`turn`]'s docs describe.
#[derive(Serialize)]
struct UserTurn<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    message: Said<'a>,
}

#[derive(Serialize)]
struct Said<'a> {
    role: &'a str,
    content: &'a str,
}

/// `system`/`init`, and nothing else that calls itself `system`.
///
/// The subtype is checked *before* the shape is read, which is what keeps this
/// narrow: the same `type` carries `status`, `hook_started` and `hook_response`
/// lines, none of which has any of the four fields below and none of which the
/// pane wants. Only a line that says it is the init line and then cannot be
/// read as one is [`AskEvent::Broke`].
///
/// ## What the tool list is, and what abeam deliberately does not add to it
///
/// `tools` is the child's own account of what it can do, and it is drawn
/// unfiltered — the read-only claim is `--tools` having been honoured, so the
/// pane shows the answer rather than abeam's opinion of it. Nothing here
/// removes an entry it did not expect, and that is the point: an entry abeam
/// did not expect is exactly what a reader needs to see.
///
/// The `init` line also carries `mcp_servers`, which is not read and has
/// nowhere to go — [`AskEvent::Ready`] is frozen without it. That costs less
/// than it looks like it costs, on one reading of the observed line: an MCP
/// tool appears in `tools` under its own `mcp__server__tool` name, so a server
/// that slipped past `--strict-mcp-config` with a tool in it would show up in
/// the list the pane draws. A server with no tools cannot do anything. That
/// reading is from one run rather than from a specification, and it is the
/// reason `mcp_servers` is a comment here instead of a field — if it turns out
/// to be wrong, this is the line to come back to and `Ready` is the type that
/// has to grow.
fn init(value: &Value) -> Option<AskEvent> {
    if value.get("subtype").and_then(Value::as_str) != Some("init") {
        return None;
    }
    let Ok(init) = serde_json::from_value::<Init>(value.clone()) else {
        return Some(AskEvent::Broke(unreadable(
            "the `system`/`init` line, which is where the child says what tools \
             it has",
            value,
        )));
    };
    // Belt and braces: `from_value` accepted a `subtype` of some other type
    // only if the field is absent, and the check above already refused that.
    debug_assert_eq!(init.subtype.as_deref(), Some("init"));
    Some(AskEvent::Ready {
        // Absent rather than guessed at. An empty model or session id draws as
        // an empty model or session id, which is the truth about a line that
        // did not carry one — and neither is load-bearing here: the id abeam
        // disowns in `crate::agentstate` is the one abeam *chose* and passed on
        // the command line, not this echo of it.
        session_id: init.session_id.unwrap_or_default(),
        model: init.model.unwrap_or_default(),
        tools: init.tools.unwrap_or_default(),
    })
}

/// The one delta shape that is an answer.
///
/// Total, and never [`AskEvent::Broke`]. This is the highest-frequency line by
/// two orders of magnitude — one per few characters of every answer — so a
/// shape that stopped parsing would not produce *a* complaint, it would produce
/// hundreds of them, one per fragment, burying the transcript in the failure
/// rather than reporting it. And the cost of dropping them is bounded and
/// already paid for: the whole answer arrives again on the `result` line, so a
/// stream this function has stopped understanding degrades to an answer that
/// appears all at once at the end of the turn instead of a word at a time.
fn delta(value: &Value) -> Option<AskEvent> {
    let event = value.get("event")?;
    if event.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = event.get("delta")?;
    if delta.get("type").and_then(Value::as_str) != Some(TEXT_DELTA) {
        return None;
    }
    // A `text_delta` with no `text` is not a fragment of anything. It is
    // dropped rather than reported for the reason above, and rather than being
    // turned into an empty `Delta`, which the pane would append to the
    // transcript as nothing at all.
    let text = delta.get("text").and_then(Value::as_str)?;
    Some(AskEvent::Delta(text.to_string()))
}

/// The end of a turn.
///
/// The one line in this file whose failure is [`AskEvent::Broke`] rather than
/// silence, and the asymmetry with [`delta`] is deliberate. A `result` that
/// cannot be read is the *end of the turn* going missing: the pane would wait
/// for a signal that has already been and gone, the composer would stay shut,
/// and nothing on screen would say why. One complaint per turn is a price worth
/// paying to never be in that state; one complaint per fragment is not.
fn finished(value: &Value) -> AskEvent {
    let Ok(finish) = serde_json::from_value::<Finish>(value.clone()) else {
        return AskEvent::Broke(unreadable(
            "a `result` line, which is how a turn ends",
            value,
        ));
    };
    // Built before the fields are taken apart, because it reads three of them.
    // Only when the child says so, too: `subtype` is `success` on a good turn,
    // and abeam does not go looking for trouble in the other fields of a line
    // that has already said there was none.
    let error = finish.is_error.unwrap_or(false).then(|| refused(&finish));
    AskEvent::Turn {
        // What Claude said, and the fallback the whole design leans on: if
        // `--include-partial-messages` were ever not honoured, no delta would
        // have arrived and this is the entire answer. Empty when the field is
        // absent, which is the ordinary shape of a turn that failed — see
        // `error` above, which is where a failed turn says anything at all.
        text: finish.result.unwrap_or_default(),
        cost_usd: finish.total_cost_usd,
        error,
    }
}

/// Why a turn failed, out of the three fields that are allowed to say.
///
/// All three are optional and the sentence is built from whichever arrived, so
/// the worst case is a true sentence with no detail in it rather than a missing
/// error. `subtype` is named whatever it says — including the `success` that
/// would contradict `is_error` — because a contradiction between two fields of
/// one line is itself the most useful thing abeam could report about it.
fn refused(finish: &Finish) -> String {
    let mut said = String::from("Claude ended the turn with an error");
    if let Some(subtype) = finish.subtype.as_deref() {
        let _ = write!(said, " (`{}`)", clip(subtype, 60));
    }
    if let Some(status) = finish
        .api_error_status
        .as_ref()
        .filter(|status| !status.is_null())
    {
        let _ = write!(said, ", API status {}", clip(&rendered(status), 60));
    }
    said.push('.');
    said
}

/// A rate limit, reduced to something sayable — out of a line nobody has
/// captured.
///
/// `rate_limit_event` is in `crate::ask`'s list of observed types and its
/// *fields* are not: the probe that found it recorded the `type` and an
/// ellipsis. So this is written to be right about a shape it has never seen,
/// which means never failing and never inventing. It looks for the three field
/// names such an event would plausibly carry a sentence in, and when it finds
/// none it hands over what the line actually said, minus the two fields every
/// line of this stream carries and which would say nothing.
///
/// The alternative — a fixed sentence of abeam's own, ignoring the line — reads
/// better and is worse: a rate limit is the one event where the reader needs
/// the detail (which limit, and when it lifts) and where abeam has none of it.
fn sayable(value: &Value) -> String {
    for field in ["message", "reason", "status"] {
        if let Some(said) = value
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|said| !said.is_empty())
        {
            return format!("Claude reported a rate limit: {}", clip(said, 200));
        }
    }

    let rest: serde_json::Map<String, Value> = value
        .as_object()
        .map(|fields| {
            fields
                .iter()
                .filter(|(name, _)| !matches!(name.as_str(), "type" | "session_id" | "uuid"))
                .map(|(name, said)| (name.clone(), said.clone()))
                .collect()
        })
        .unwrap_or_default();

    if rest.is_empty() {
        "Claude reported a rate limit and said nothing else about it.".to_string()
    } else {
        format!(
            "Claude reported a rate limit: {}",
            clip(&Value::Object(rest).to_string(), 300)
        )
    }
}

/// A JSON value as a person would want to read it: a string without its quotes,
/// anything else as it was written.
fn rendered(value: &Value) -> String {
    match value.as_str() {
        Some(text) => text.to_string(),
        None => value.to_string(),
    }
}

/// What abeam says about a line that is not JSON.
///
/// The line itself is in the message, clipped, because this is the failure that
/// is almost never about JSON: what lands here is a Node warning, a proxy's
/// HTML error page, or the first line of a stack trace — output from something
/// on the machine that is not the protocol at all, and unreadable without the
/// text in front of the reader.
fn not_json(line: &str, why: &serde_json::Error) -> String {
    format!(
        "`claude` printed a line abeam could not read as JSON ({why}): {}",
        clip(line.trim(), 200)
    )
}

/// What abeam says about JSON that is not one of these lines.
///
/// Separate from [`not_json`] because it points somewhere else entirely. A line
/// that parses and carries no `type` is well-formed JSON on a stream where
/// every message is tagged, which means either the format has changed shape or
/// what is on the other end of the pipe is not Claude.
fn no_type(line: &str) -> String {
    format!(
        "`claude` printed a JSON line with no `type` field, which every line of \
         this stream has carried: {}",
        clip(line.trim(), 200)
    )
}

/// What abeam says about a line that named itself and then did not fit.
fn unreadable(what: &str, value: &Value) -> String {
    format!(
        "abeam could not read {what}. This is a format abeam does not own and \
         it may simply have changed; what arrived was: {}",
        clip(&value.to_string(), 300)
    )
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Nothing here starts a `claude`, and unlike `crate::ask`'s spawning tests
/// nothing here starts anything at all — which is the whole reason the parsing
/// is a set of free functions over a `&str`.
///
/// The fixtures marked *captured* are the lines recorded in `crate::ask`'s
/// module docs, from the run against 2.1.222 on 2026-08-05, with the elisions
/// in that record filled in: the probe wrote `"model":"…"` rather than a model
/// name, so a plausible one stands in its place and the rest of each line is
/// as it arrived. The fixtures marked *constructed* are shapes that were not
/// captured — a thinking delta, a tool-input delta, a failed turn — and they
/// are built here from the surrounding shape rather than invented whole. Which
/// is which matters: a test can only be evidence about the format to the extent
/// its input came from the format.
#[cfg(test)]
mod tests {
    use super::*;

    /// Captured. The line that says what the child can do.
    const INIT: &str = r#"{"type":"system","subtype":"init","tools":["Glob","Grep","Read"],"model":"claude-opus-4-5-20251101","permissionMode":"plan","session_id":"3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e","cwd":"C:\\Users\\someone\\forge","mcp_servers":[]}"#;

    /// Captured. One fragment of an answer.
    const DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"AL"}},"session_id":"3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e"}"#;

    /// Captured. The completed message, which repeats what the deltas carried.
    const ASSISTANT: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"ALPHA"}]},"session_id":"3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e"}"#;

    /// Captured. The end of a turn, and what it cost.
    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"ALPHA","total_cost_usd":0.0544,"stop_reason":"end_turn","terminal_reason":"completed","api_error_status":null,"session_id":"3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e"}"#;

    /// Constructed: the delta envelope above with the model's own reasoning in
    /// it. This is the negative that matters most — thinking is text, it is
    /// long, and it is not an answer.
    const THINKING: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"The user is asking about the viewer"}},"session_id":"x"}"#;

    /// Constructed: the same envelope carrying a tool call being assembled a
    /// character at a time. A transcript that showed this would show half a
    /// `Grep` argument as though the child had said it.
    const TOOL_INPUT: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pattern\":\"fn ma"}},"session_id":"x"}"#;

    fn text_of(event: Option<AskEvent>) -> String {
        match event {
            Some(AskEvent::Delta(text)) => text,
            other => panic!("expected a text fragment, got {other:?}"),
        }
    }

    // --- what reaches the transcript ---------------------------------------

    #[test]
    fn a_text_delta_is_the_only_delta_that_reaches_the_transcript() {
        assert_eq!(text_of(parse_line(DELTA)), "AL");

        // The two negatives, asserted one at a time and by name, because they
        // are the two ways this filter fails in a direction nobody notices
        // until it is on somebody's screen. A thinking delta in the transcript
        // reads exactly like an answer — it is fluent prose about the question
        // — so the reader has no way to tell that what they are reading was
        // never said to them. A tool-input delta is worse in a different way:
        // it is a *fragment of a call*, and half a path or half a regex on
        // screen looks like the child telling them something about their
        // repository.
        assert_eq!(
            parse_line(THINKING),
            None,
            "the model's reasoning reached the transcript"
        );
        assert_eq!(
            parse_line(TOOL_INPUT),
            None,
            "half a tool call reached the transcript"
        );

        // And the shapes around the delta, which arrive between the fragments
        // of every answer: the block opening and closing, and the message
        // stopping. None of them is a fragment and none of them is a
        // complaint.
        for quiet in [
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"session_id":"x"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0},"session_id":"x"}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"}},"session_id":"x"}"#,
            // A delta shape this version has never seen. Dropped rather than
            // reported, and this is where that costs the least: the answer
            // arrives again, whole, on the `result` line.
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}},"session_id":"x"}"#,
            // And an envelope with nothing in it. `delta` is what this reads
            // and there is none, so there is nothing to say.
            r#"{"type":"stream_event","session_id":"x"}"#,
        ] {
            assert_eq!(parse_line(quiet), None, "got an event from: {quiet}");
        }
    }

    #[test]
    fn an_assistant_message_adds_nothing_the_deltas_and_the_result_did_not() {
        // The decision the module docs argue: the completed message repeats the
        // text the fragments already drew, and the authoritative copy is on the
        // `result` line after it. An event here would draw the answer twice.
        assert_eq!(parse_line(ASSISTANT), None);

        // Which is only safe because the fallback is real, so it is asserted
        // here rather than left as a claim in a comment: with no delta ever
        // parsed, `Turn` still carries the whole answer.
        let AskEvent::Turn { text, .. } = parse_line(RESULT).expect("a result is an event") else {
            panic!("a result line is a turn");
        };
        assert_eq!(text, "ALPHA");

        // A tool result comes back as a `user` message on this stream — the
        // child has `Read`, `Grep` and `Glob` — and the contents of a file it
        // opened are not an answer to the question.
        assert_eq!(
            parse_line(
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"fn main() {}"}]},"session_id":"x"}"#
            ),
            None
        );
    }

    // --- the line that says what the child can do ---------------------------

    #[test]
    fn the_init_line_is_where_the_read_only_claim_is_checked() {
        let event = parse_line(INIT).expect("an init line is an event");
        assert_eq!(
            event,
            AskEvent::Ready {
                session_id: "3f2b1c9d-4e5a-4b6c-8d7e-9f0a1b2c3d4e".to_string(),
                model: "claude-opus-4-5-20251101".to_string(),
                tools: vec![
                    "Glob".to_string(),
                    "Grep".to_string(),
                    "Read".to_string()
                ],
            }
        );

        // The claim itself, in the form the pane will draw it: the tools that
        // write are absent from the list the child reported. This is evidence
        // about the *fixture* rather than about the parser — but the fixture is
        // captured output from a run made with `crate::ask::TOOLS`, so it is
        // the only record in the suite of what that flag actually did.
        let AskEvent::Ready { tools, .. } = event else {
            panic!("an init line is a Ready");
        };
        for writes in ["Write", "Edit", "NotebookEdit", "Bash"] {
            assert!(
                !tools.iter().any(|tool| tool == writes),
                "`{writes}` was in the tool list the child came back with: {tools:?}"
            );
        }

        // Nothing is filtered on the way through, which is the other half of
        // showing the list rather than asserting it. A tool abeam did not
        // expect — an MCP tool under its own name is the shape that would
        // arrive — reaches the pane, because a reader who is being told what
        // the child can do needs the entry that surprises them most.
        let sneaked = INIT.replace(
            r#""tools":["Glob","Grep","Read"]"#,
            r#""tools":["Glob","Grep","Read","mcp__notes__write","Bash"]"#,
        );
        let AskEvent::Ready { tools, .. } = parse_line(&sneaked).expect("still an init line") else {
            panic!("an init line is a Ready");
        };
        assert!(tools.iter().any(|tool| tool == "mcp__notes__write"));
        assert!(tools.iter().any(|tool| tool == "Bash"));
    }

    #[test]
    fn a_system_line_that_is_not_the_init_one_is_not_an_event() {
        // The same `type` carries the hook lines and the status lines, which
        // arrive during every turn. None of them has a tool list in it and none
        // of them is what the pane is waiting for.
        for other in [
            r#"{"type":"system","subtype":"status","session_id":"x"}"#,
            r#"{"type":"system","subtype":"hook_started","hook":"PreToolUse","session_id":"x"}"#,
            r#"{"type":"system","subtype":"hook_response","hook":"PreToolUse","session_id":"x"}"#,
            // No subtype at all: not the init line, and not a complaint either.
            r#"{"type":"system","session_id":"x"}"#,
        ] {
            assert_eq!(parse_line(other), None, "got an event from: {other}");
        }

        // But an init line that cannot be read *as one* is said out loud, since
        // this is the line the pane draws its whole account of the child's
        // authority from — going quiet here means drawing nothing and
        // explaining nothing.
        let broken = r#"{"type":"system","subtype":"init","tools":"Read,Grep,Glob","session_id":"x"}"#;
        let Some(AskEvent::Broke(said)) = parse_line(broken) else {
            panic!("an init line that is not one is not silence");
        };
        assert!(said.contains("init"), "which line: {said}");
        assert!(
            said.contains("does not own"),
            "whose format it is: {said}"
        );
    }

    // --- the end of a turn --------------------------------------------------

    #[test]
    fn a_result_ends_the_turn_and_says_what_it_cost() {
        assert_eq!(
            parse_line(RESULT),
            Some(AskEvent::Turn {
                text: "ALPHA".to_string(),
                cost_usd: Some(0.0544),
                error: None,
            })
        );

        // A turn with no cost on it is still a turn. The field is optional
        // because the pane's cost line is worth less than the end-of-turn
        // signal, and losing the second over the first would shut the composer
        // for good.
        assert_eq!(
            parse_line(r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#),
            Some(AskEvent::Turn {
                text: "ok".to_string(),
                cost_usd: None,
                error: None,
            })
        );
    }

    #[test]
    fn a_failed_turn_says_so_from_whichever_fields_were_set() {
        // Constructed from the captured shape: the same line with the three
        // error fields set the way a failed turn would set them.
        let Some(AskEvent::Turn { text, error, .. }) = parse_line(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"api_error_status":529,"total_cost_usd":0.01}"#,
        ) else {
            panic!("a result line is a turn even when the turn failed");
        };
        assert_eq!(text, "", "a failed turn carries no answer");
        let said = error.expect("a failed turn says why");
        assert!(said.contains("error_during_execution"), "got: {said}");
        assert!(said.contains("529"), "the status was dropped: {said}");

        // Whichever fields were set: no subtype, no status, and the sentence is
        // still true and still says the turn failed.
        let Some(AskEvent::Turn { error, .. }) =
            parse_line(r#"{"type":"result","is_error":true,"result":""}"#)
        else {
            panic!("a result line is a turn");
        };
        assert_eq!(
            error.as_deref(),
            Some("Claude ended the turn with an error.")
        );

        // A string status rather than a number, which is the other shape the
        // captured `null` leaves open — read without its quotes, and without
        // costing the line its readability.
        let Some(AskEvent::Turn { error, .. }) = parse_line(
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"api_error_status":"429 Too Many Requests"}"#,
        ) else {
            panic!("a result line is a turn");
        };
        let said = error.expect("a failed turn says why");
        assert!(said.contains("429 Too Many Requests"), "got: {said}");
        assert!(
            !said.contains('"'),
            "a JSON string was reported with its quotes on: {said}"
        );

        // And the contradiction, reported rather than smoothed over: `is_error`
        // with a `success` subtype is two fields of one line disagreeing, which
        // is the most useful thing abeam can say about it.
        let Some(AskEvent::Turn { error, .. }) =
            parse_line(r#"{"type":"result","subtype":"success","is_error":true,"result":"hm"}"#)
        else {
            panic!("a result line is a turn");
        };
        assert!(
            error.expect("is_error is what decides").contains("success"),
            "the contradiction was hidden"
        );
    }

    #[test]
    fn a_result_that_cannot_be_read_is_the_one_failure_that_is_never_silent() {
        // The asymmetry with the deltas, and the reason for it: a `result` is
        // the end of the turn, and a turn whose end went missing leaves the
        // pane waiting for a signal that has already been and gone. One
        // complaint per turn buys never being in that state.
        for contradicted in [
            // `result` is the answer text; an object there is not text.
            r#"{"type":"result","subtype":"success","is_error":false,"result":{"text":"ALPHA"}}"#,
            // A cost that is not a number.
            r#"{"type":"result","subtype":"success","total_cost_usd":"0.0544"}"#,
            // An `is_error` that is not a yes or a no.
            r#"{"type":"result","subtype":"success","is_error":"yes"}"#,
        ] {
            let Some(AskEvent::Broke(said)) = parse_line(contradicted) else {
                panic!("a result that cannot be read is not silence: {contradicted}");
            };
            assert!(said.contains("`result`"), "which line: {said}");
            // What arrived, so that whoever re-points this at a new sample can
            // see the sample from the pane rather than from a debugger.
            assert!(said.contains("type"), "the line itself is missing: {said}");
        }
    }

    // --- the shapes nobody captured -----------------------------------------

    #[test]
    fn a_rate_limit_is_reduced_to_something_sayable_whatever_it_arrives_as() {
        // The `type` was observed and its fields were not, so this is written
        // to be right about a shape it has never seen. Three plausible ones,
        // and none of them may produce silence or a panic.
        let said = match parse_line(
            r#"{"type":"rate_limit_event","message":"You have reached your usage limit. It resets at 5pm.","session_id":"x"}"#,
        ) {
            Some(AskEvent::RateLimited(said)) => said,
            other => panic!("a rate limit is a rate limit, got {other:?}"),
        };
        assert!(said.contains("resets at 5pm"), "got: {said}");

        // Nothing abeam recognises: what the line said, minus the two fields
        // every line carries and which would tell the reader nothing.
        let said = match parse_line(
            r#"{"type":"rate_limit_event","rate_limit":{"kind":"five_hour","resetsAt":1785683809},"session_id":"x"}"#,
        ) {
            Some(AskEvent::RateLimited(said)) => said,
            other => panic!("a rate limit is a rate limit, got {other:?}"),
        };
        assert!(said.contains("five_hour"), "got: {said}");
        assert!(
            !said.contains("session_id"),
            "the id every line carries was reported as news: {said}"
        );

        // And a line with nothing in it at all, which is still not silence: the
        // reader needs to know a limit was hit even when abeam cannot say which
        // one.
        let said = match parse_line(r#"{"type":"rate_limit_event"}"#) {
            Some(AskEvent::RateLimited(said)) => said,
            other => panic!("a rate limit is a rate limit, got {other:?}"),
        };
        assert!(said.contains("rate limit"), "got: {said}");
    }

    #[test]
    fn an_unknown_type_is_ignored_and_a_line_that_is_not_json_is_not() {
        // The rule that follows from not owning the format. A `type` this
        // version has never heard of is Claude having added a message, and a
        // pane that complained about it would be wrong on every release.
        for grown in [
            r#"{"type":"compact_boundary","compact_metadata":{"trigger":"auto"},"session_id":"x"}"#,
            r#"{"type":"something_from_2027","session_id":"x"}"#,
            r#"{"type":"control_response","response":{"subtype":"success"}}"#,
        ] {
            assert_eq!(parse_line(grown), None, "complained about: {grown}");
        }

        // A blank line is not a message. `BufRead::lines` produces one for the
        // newline at the end of a stream, and a child that prints one between
        // messages is not saying anything.
        for blank in ["", " ", "\t", "   \r"] {
            assert_eq!(parse_line(blank), None, "complained about {blank:?}");
        }

        // And what is not the protocol at all. This is what a Node warning, a
        // proxy's error page or a stack trace looks like arriving on this pipe,
        // and dropping it silently is how a pane comes to sit there saying
        // nothing while the child explains itself into a void.
        for foreign in [
            "(node:4213) ExperimentalWarning: buffer.File is an experimental feature",
            "Error: connect ETIMEDOUT 10.0.0.1:443",
            "<html><head><title>407 Proxy Authentication Required</title></head>",
            "{",
        ] {
            let Some(AskEvent::Broke(said)) = parse_line(foreign) else {
                panic!("a line that is not JSON was dropped: {foreign}");
            };
            assert!(said.contains("JSON"), "got: {said}");
            assert!(
                said.contains(&foreign[..foreign.len().min(10)]),
                "the line itself is missing from: {said}"
            );
        }

        // JSON that is not this format is a different sentence pointing
        // somewhere else: every line of this stream is tagged, so an untagged
        // one means either the format has changed shape or what is on the other
        // end of the pipe is not Claude.
        for untagged in [r#"{"session_id":"x","result":"hello"}"#, "[]", "42", r#""ok""#] {
            let Some(AskEvent::Broke(said)) = parse_line(untagged) else {
                panic!("JSON with no `type` was dropped: {untagged}");
            };
            assert!(said.contains("`type`"), "got: {said}");
        }
    }

    // --- what goes the other way --------------------------------------------

    #[test]
    fn a_prompt_reaches_stdin_as_json_with_nothing_hand_escaped() {
        // Every character here is one that would end, unbalance or mangle
        // something on `crate::dispatch`'s route: a newline ends a `cmd.exe`
        // command line outright and cannot be escaped at all, a double quote
        // desyncs its quote tracking, a backslash is what a hand-rolled escaper
        // gets wrong, and an `&` separates one command from the next. Here they
        // are four ordinary bytes inside a quoted string.
        let awkward = "why does \"a & b\"\nfail in C:\\Users\\x?\ttabbed\r\nand {braces}";
        let line = turn(awkward);

        // One line, which is the whole contract with the child's stdin: the
        // newline in the question is `\n` in the JSON and the only real newline
        // is the one `AskSession::ask` writes after it.
        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "the question ended the line it was written on: {line}"
        );

        // And it round-trips, byte for byte. Read back with the same library
        // the child reads it with, rather than compared against a
        // hand-written expectation — which would be a second escaper, and
        // therefore a second thing to get wrong.
        let read: Value = serde_json::from_str(&line).expect("the line is JSON");
        assert_eq!(read["type"], "user");
        assert_eq!(read["message"]["role"], "user");
        assert_eq!(
            read["message"]["content"].as_str(),
            Some(awkward),
            "the question did not survive the trip"
        );

        // The shape the module docs describe, spelled out once so that a change
        // to it is a visible change to this line rather than a silent change to
        // what the child is sent.
        assert_eq!(
            turn("hello"),
            r#"{"type":"user","message":{"role":"user","content":"hello"}}"#
        );

        // Nothing is refused, and that is a decision rather than an omission:
        // `crate::dispatch` refuses an empty prompt because a background agent
        // with no instruction runs unattended with edits pre-approved, and
        // nothing here can edit anything. An empty question costs a round trip.
        assert!(turn("").contains(r#""content":"""#));
        // Including a NUL, which is the one byte no argv can carry and is
        // ordinary inside a JSON string. It arrives escaped, and the line
        // stays one line.
        let nul = turn("a\0b");
        assert!(nul.contains(r"\u0000"), "got: {nul}");
        let read: Value = serde_json::from_str(&nul).expect("the line is JSON");
        assert_eq!(read["message"]["content"].as_str(), Some("a\0b"));
    }

    #[test]
    fn a_session_id_is_v4_shaped_and_no_two_are_the_same() {
        let id = new_session_id();

        // 8-4-4-4-12, lowercase hex, and nothing else — checked by walking it
        // rather than by a pattern, because the two nibbles below are the whole
        // point and a pattern that spelled them would be the same claim written
        // twice.
        assert_eq!(id.len(), 36, "not a UUID: {id}");
        let bytes = id.as_bytes();
        for (at, byte) in bytes.iter().enumerate() {
            if matches!(at, 8 | 13 | 18 | 23) {
                assert_eq!(*byte, b'-', "no hyphen at {at} in {id}");
            } else {
                assert!(
                    byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
                    "{:?} at {at} is not lowercase hex in {id}",
                    *byte as char
                );
            }
        }
        // The version nibble and the variant nibble, which are the four bits
        // and the two bits that stand between this and a `--session-id` Claude
        // refuses at startup.
        assert_eq!(bytes[14], b'4', "not version 4: {id}");
        assert!(
            matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
            "not an RFC 4122 variant: {id}"
        );

        // Two calls differ, which is the property the whole function exists
        // for: two workspaces in one window and two windows on one machine must
        // not hand `crate::agentstate` one id to disown for two children.
        // Asserted over a run rather than a pair, because a clock read twice in
        // a tight loop is exactly where a timestamp-only version would collide.
        let ids: std::collections::BTreeSet<String> =
            (0..1000).map(|_| new_session_id()).collect();
        assert_eq!(ids.len(), 1000, "two session ids collided");
    }
}
