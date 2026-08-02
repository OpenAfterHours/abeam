# ConPTY findings

> Provenance: `spike-pty/README.md`, a go/no-go spike that returned **go** on
> 2026-08-01. The spike directory is gone; this document and the code it
> describes are what survived it. Everything here was expensive to discover.

The question the spike answered:

> Can Claude Code run inside a pane we draw ourselves, on Windows ConPTY, and
> still look and behave correctly?

**Verdict: go.** Claude renders and behaves correctly hosted in a pane we draw
ourselves. Confirmed by automated tests and by the author running a real Claude
session on 2026-08-01. Everything else in abeam — git pane, markdown viewer,
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

- `DsrScanner` in `crates/abeam-pty/src/input.rs` watches the output stream for
  the query, carrying bytes across read boundaries so a split sequence still
  matches.
- The pty **writer is shared** between callers and the reader thread, because
  the reader is what has to answer. This is why `PtySession` holds an
  `Arc<Mutex<..>>` writer and exposes no accessor to it.
- `conpty_stalls_until_the_dsr_query_is_answered` in
  `crates/abeam-pty/tests/conpty.rs` pins the behaviour both ways. If the
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

Three of the five are facts about *this* platform and two are general, and now
that `abeam-pty` builds for Unix as well the difference has to be marked. A
reader on Linux who takes the whole list as gospel goes looking for a handshake
that never happens. Nothing here stops being a rule the code must keep — the
marks say where each rule comes from, not which ones may be dropped.

1. **The writer is shared with the reader thread.** *(ConPTY.)* Do not
   "simplify" it back into sole ownership by whatever sends input. The reader is
   what answers DSR. Nothing opens a Unix pty with a query, so there the sharing
   costs a mutex and buys nothing; it stays unconditional because one shape that
   is right on both platforms is worth more than a `cfg` inside the reader loop.
2. **Poll `try_wait()`. Never `wait()`** *(ConPTY)* — it never returns. There is
   no reliable EOF on ConPTY's master to block against, so the reader thread
   must be treated as an endless stream and **never joined**; let it die with
   the process. Joining it is the obvious-looking thing to add. Do not add it.
   A Unix master *does* give a real EOF — dropping the slave in the parent is
   what lets the reader see it — so on Linux this rule stops being load-bearing
   without stopping being correct, which is not a licence to join the thread on
   one platform and not the other.
3. **Windows sends Press *and* Release for every key.** *(Windows console.)*
   Forwarding both double-types everything. `encode_key` drops releases.
   Anything that matches its own bindings *before* calling `encode_key` — the
   app shell does — needs its own release filter, or every command fires twice.
   A Unix terminal reports presses only, and a terminal that negotiates the
   Kitty keyboard protocol reports releases on any platform, so the filter is
   unconditional and its test builds the release rather than typing one.
4. **Mouse reports are gated on what the hosted app actually enabled.**
   *(General.)* Sending unrequested ones dumps escape sequences into the agent's
   prompt.
5. **`Screen::contents()` rejoins wrapped rows** into logical lines, so it tells
   you nothing about layout. *(General.)* Use `Screen::rows()` for anything
   positional. This one is not structurally enforceable — it is vt100's API, not
   ours.

## Other things learned

- **ConPTY does not wrap long lines.** It emits a 100-character run as-is and
  leaves layout to the host. Wrapping is the parser's job, and sizing the parser
  correctly is ours.
- **Only DSR is answered.** Other terminal queries (Device Attributes, colour
  reports) are ignored. If a hosted app turns out to need one, it will present
  exactly the same way: a stall with a tiny byte count. Check `dsr_replies` and
  `bytes_read` first.
- **`EnableMouseCapture` disables the host terminal's native text selection.**
  Copying out of abeam needs Shift+drag, and which terminals honour that varies.

## Reading the diagnostics

`PtyStats` and the screen's mode flags exist so that the failure modes above are
visible instead of mysterious. Two places put them on screen: **`F2` inside
abeam**, which is the one that matters because these failures are not
reproducible on demand and you want the instrument while the thing is going
wrong; and `cargo run -p abeam-pty --example host`, which is the same view
without abeam around it.

| Field | Why it matters |
| --- | --- |
| `alt screen` | Should flip to `on` once the agent's UI starts. |
| `app cursor` | DECCKM. When `on`, arrows must be sent as `ESC O A`. |
| `bracketed paste` | When `on`, pasted text is wrapped so the agent can tell paste from typing. |
| `mouse mode` / `encoding` | Whether the agent wants mouse reports, and in which dialect. We stay silent unless asked. |
| `dsr_replies` | *Windows:* non-zero within the first moment — **zero means an imminent hang**, and the pane reddens it and says so. *Unix:* nothing opens a pty by asking where the cursor is, so zero is what a healthy session reads for its whole life; the row is drawn plain and carries no alarm. A number that climbs there is a hosted child querying for itself, and `abeam-pty` answering. |
| `pty size (set)` / `parser size` | Compare them with the *inner* area of the bordered pane — not with each other. See below. |
| `bytes_read` | Should climb steadily. Frozen = the reader thread died. |

The two size rows are not a cross-check, and the pane no longer pretends they
are. `portable_pty::ConPtyMasterPty::get_size` answers from a field it wrote
itself during the last successful `ResizePseudoConsole`, and `PtySession::resize`
updates the vt100 parser from that same call — so on Windows the two agree by
construction and their agreeing is evidence of nothing. What they are good for is
the check no code can make: reading them off the screen and comparing them with
the pane you are looking at. A hosted app wrapping in the wrong place is a size
that does not match the rect.

On Unix that argument does not hold, and the pane has not caught up. `get_size`
there is a real `TIOCGWINSZ` on the master — the kernel's answer rather than a
remembered one — so the two rows genuinely can disagree, and a resize the kernel
took and the parser did not would show up as two different numbers. The pane
does not flag it, on either platform. That is now a gap rather than a decision
that has been made, and it is recorded here so the next person to look does not
have to rediscover which half of the paragraph above is still true.

## Pass criteria

All confirmed passing on 2026-08-01, **against `examples/host.rs`**. Kept as the
manual regression checklist, because they have no automated equivalent — they
need a human at a terminal. Run either `abeam` or
`cargo run -p abeam-pty --example host`.

They have not been re-run against `abeam` itself since the git and files panes
landed. The pty layer underneath is unchanged and its tests still pass, so a
failure now would be in the shell — most likely in key routing, since that is
the only part of the path the panes added anything to.

They have also never been run on Linux, against anything. Every one of the six
is a question about a pty and a hosted program rather than about ConPTY, so they
transfer word for word and there is no second checklist to write — which is
exactly what makes running them there the acceptance gate the Linux wheel has
not passed. The README's "Platforms" says so where somebody about to install it
will see it.

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
- **Linux, by hand.** The suite no longer passes there by saying nothing:
  `tests/session.rs` used to be `#![cfg(windows)]` beside `conpty.rs` and is
  ungated now, because every claim in it is about our wrapper rather than about
  a pseudoconsole and all of them hold on a Unix pty — and it is the only
  coverage `src/tree/unix.rs` has, which is why its last test asserts on a real
  grandchild. CI runs the whole workspace on both platforms. What has not
  happened on Linux is a human at a terminal: the six pass criteria above.
- `tests/conpty.rs` stays `#![cfg(windows)]` for ever, and that is not the same
  admission. It pins ConPTY's own `ESC [ 6 n` handshake, and a Unix pty asks
  nothing at startup — so over there the file would not be the same test
  proving less, it would be a test of nothing.
