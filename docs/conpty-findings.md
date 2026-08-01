# ConPTY findings

> Provenance: `spike-pty/README.md`, a go/no-go spike that returned **go** on
> 2026-08-01. The spike directory is gone; this document and the code it
> describes are what survived it. Everything here was expensive to discover.

The question the spike answered:

> Can Claude Code run inside a pane we draw ourselves, on Windows ConPTY, and
> still look and behave correctly?

**Verdict: go.** Claude renders and behaves correctly hosted in a pane we draw
ourselves. Confirmed by automated tests and by the author running a real Claude
session on 2026-08-01. Everything else in forge — git pane, markdown viewer,
file watcher, packaging — is ordinary work with no comparable unknown in it.

## The finding that nearly sank it

ConPTY opens every session by writing a Device Status Report query —
`ESC [ 6 n`, *"where is the cursor?"* — and **blocks until the host answers**.

A host that ignores it sees four bytes of output and a child that never runs its
command, never exits, and never reports a status. It presents as a dead pty. The
spike burned several wrong diagnoses on it (blamed `read_to_end`, then `wait()`,
then slave-handle lifetime) before instrumenting the sequence and finding the
child had emitted exactly `\x1b[6n` and stopped.

Answering with `ESC [ row ; col R` fixes it completely: the same `cmd /c echo hi`
then completes in under half a second.

Three consequences, all still in the code:

- `DsrScanner` in `crates/forge-pty/src/input.rs` watches the output stream for
  the query, carrying bytes across read boundaries so a split sequence still
  matches.
- The pty **writer is shared** between callers and the reader thread, because
  the reader is what has to answer. This is why `PtySession` holds an
  `Arc<Mutex<..>>` writer and exposes no accessor to it.
- `conpty_stalls_until_the_dsr_query_is_answered` in
  `crates/forge-pty/tests/conpty.rs` pins the behaviour both ways. If the
  ignore-DSR arm ever starts passing, Windows has changed and the handling can
  be removed.

Had this surfaced during full integration instead of in a spike, it would have
looked like "Claude doesn't work in my terminal" with no obvious cause.

**This finding is written down in three places on purpose**: here, in the
`DsrScanner` doc comment, and in the `tests/conpty.rs` module header. Each is
reached by a different route — reading the docs, reading the code, reading a
test failure. That redundancy is deliberate and is not duplication to tidy away.

## Load-bearing constraints

Any refactor must keep all five. There are tests pinning every one of them.

1. **The writer is shared with the reader thread.** Do not "simplify" it back
   into sole ownership by whatever sends input. The reader is what answers DSR.
2. **Poll `try_wait()`. Never `wait()`** — it never returns. There is no
   reliable EOF on the master to block against, so the reader thread must be
   treated as an endless stream and **never joined**; let it die with the
   process. Joining it is the obvious-looking thing to add. Do not add it.
3. **Windows sends Press *and* Release for every key.** Forwarding both
   double-types everything. `encode_key` drops releases. Anything that matches
   its own bindings *before* calling `encode_key` — the app shell does — needs
   its own release filter, or every command fires twice.
4. **Mouse reports are gated on what the hosted app actually enabled.** Sending
   unrequested ones dumps escape sequences into Claude's prompt.
5. **`Screen::contents()` rejoins wrapped rows** into logical lines, so it tells
   you nothing about layout. Use `Screen::rows()` for anything positional. This
   one is not structurally enforceable — it is vt100's API, not ours.

## Other things learned

- **ConPTY does not wrap long lines.** It emits a 100-character run as-is and
  leaves layout to the host. Wrapping is the parser's job, and sizing the parser
  correctly is ours.
- **Only DSR is answered.** Other terminal queries (Device Attributes, colour
  reports) are ignored. If a hosted app turns out to need one, it will present
  exactly the same way: a stall with a tiny byte count. Check `dsr_replies` and
  `bytes_read` first.
- **`EnableMouseCapture` disables the host terminal's native text selection.**
  Copying out of forge needs Shift+drag, and which terminals honour that varies.

## Reading the diagnostics

`PtyStats` and the screen's mode flags exist so that the failure modes above are
visible instead of mysterious. Two places put them on screen: **`F2` inside
forge**, which is the one that matters because these failures are not
reproducible on demand and you want the instrument while the thing is going
wrong; and `cargo run -p forge-pty --example host`, which is the same view
without forge around it.

| Field | Why it matters |
| --- | --- |
| `alt screen` | Should flip to `on` once Claude's UI starts. |
| `app cursor` | DECCKM. When `on`, arrows must be sent as `ESC O A`. |
| `bracketed paste` | When `on`, pasted text is wrapped so Claude can tell paste from typing. |
| `mouse mode` / `encoding` | Whether Claude wants mouse reports, and in which dialect. We stay silent unless asked. |
| `dsr_replies` | Non-zero within the first moment. **Zero means an imminent hang.** |
| `pty size (set)` / `parser size` | Compare them with the *inner* area of the bordered pane — not with each other. See below. |
| `bytes_read` | Should climb steadily. Frozen = the reader thread died. |

The two size rows are not a cross-check, and the pane no longer pretends they
are. `portable_pty::ConPtyMasterPty::get_size` answers from a field it wrote
itself during the last successful `ResizePseudoConsole`, and `PtySession::resize`
updates the vt100 parser from that same call — so the two agree by construction
and their agreeing is evidence of nothing. What they are good for is the check
no code can make: reading them off the screen and comparing them with the pane
you are looking at. A hosted app wrapping in the wrong place is a size that does
not match the rect.

## Pass criteria

All confirmed passing on 2026-08-01, **against `examples/host.rs`**. Kept as the
manual regression checklist, because they have no automated equivalent — they
need a human at a terminal. Run either `forge` or
`cargo run -p forge-pty --example host`.

They have not been re-run against `forge` itself since the git and files panes
landed. The pty layer underneath is unchanged and its tests still pass, so a
failure now would be in the shell — most likely in key routing, since that is
the only part of the path the panes added anything to.

1. **Renders.** Claude's UI appears, colours intact, no stray escape sequences
   as literal text.
2. **Types.** One character per keystroke. *Two* means the release filter broke.
3. **Editing keys.** Arrows, Home/End, Backspace, Delete behave. Ctrl+C
   interrupts rather than inserting a control character.
4. **Resize.** Drag the window wider and narrower. Claude reflows, `resizes`
   increments, and the two size fields stay in agreement.
5. **Paste.** Multi-line paste arrives as one block, not a burst of Enters.
6. **Exit.** `/exit` closes the pty cleanly rather than hanging.

Steps 4 and 6 have automated coverage at the pty layer (resize accepted while a
child is attached; children reaped via `try_wait`), so failures there would be in
the UI layer rather than the plumbing.

The fallback plan — drive an existing multiplexer like WezTerm instead of hosting
the pty ourselves — is no longer needed, but is worth remembering if the pty
layer ever becomes a maintenance burden.

## Never tested

Not bugs; just never exercised, so do not assume they work:

- Wide-character and grapheme-cluster handling under resize.
- Any terminal query other than DSR.
- Anything but Windows. `tests/conpty.rs` and `tests/session.rs` are
  `#![cfg(windows)]`, so on another platform the suite passes by saying nothing.
