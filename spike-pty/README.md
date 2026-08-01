# spike-pty

A go/no-go test for one question:

> Can Claude Code run inside a pane we draw ourselves, on Windows ConPTY,
> and still look and behave correctly?

Everything else in the `forge` idea — the git pane, the markdown viewer, the
file watcher, the PyPI packaging — is ordinary work. This is the part that
either works or sinks the design, so it gets tested first.

## Status

**Verdict: go.** Claude renders and behaves correctly hosted in a pane we draw
ourselves, on Windows ConPTY.

| | |
| --- | --- |
| Builds | yes, MSVC, no vcvars wrapper needed |
| Automated tests | 21 passing (16 input encoding, 5 live ConPTY) |
| Interactive behaviour | confirmed by the author running a real Claude session, 2026-08-01 |

The architecture is sound. Everything remaining in the `forge` idea — git pane,
markdown viewer, file watcher, packaging — is ordinary work with no comparable
unknown in it.

## Running it

```
cargo run                   :: hosts `claude`
cargo run -- powershell     :: hosts something else, for comparison
cargo test                  :: encoder + live ConPTY tests
cargo run --example diag    :: raw pty diagnostics, no UI
```

`Ctrl+]` detaches and exits.

## The finding that nearly sank it

ConPTY opens every session by writing a Device Status Report query —
`ESC [ 6 n`, *"where is the cursor?"* — and **blocks until the host answers**.

A host that ignores it sees four bytes of output and a child that never runs its
command, never exits, and never reports a status. It presents as a dead pty. I
spent several wrong diagnoses on it (blamed `read_to_end`, then `wait()`, then
slave-handle lifetime) before instrumenting the sequence and finding the child
had emitted exactly `\x1b[6n` and stopped.

Answering with `ESC [ row ; col R` fixes it completely: the same `cmd /c echo hi`
then completes in under half a second.

Three consequences, all now in the code:

- `DsrScanner` in `input.rs` watches the output stream for the query, carrying
  bytes across read boundaries so a split sequence still matches.
- The pty **writer is shared** between the input loop and the reader thread,
  because the reader is what has to answer.
- `conpty_stalls_until_the_dsr_query_is_answered` pins the behaviour both ways.
  If the ignore-DSR arm ever starts passing, Windows changed and the handling
  can be removed.

Had this surfaced during full integration instead of in a spike, it would have
looked like "Claude doesn't work in my terminal" with no obvious cause.

## Other things learned

- **`Child::wait()` is unusable here.** Poll `try_wait()` instead. There is no
  reliable EOF on the master to block against, so the reader thread must be
  treated as an endless stream and never joined — let it die with the process.
- **ConPTY does not wrap long lines.** It emits a 100-character run as-is and
  leaves layout to the host. Wrapping is the parser's job, and our job to size
  correctly.
- **`Screen::contents()` rejoins wrapped rows** into logical lines, so it tells
  you nothing about layout. Use `Screen::rows()` for anything positional.
- **Windows sends Press *and* Release for every key.** Forwarding both
  double-types everything. `input.rs` drops releases; there's a test for it.

## What you're looking at

The left pane is Claude, rendered by us: bytes come out of the pty, through a
`vt100` parser, into a `ratatui` widget. Nothing passes through to the real
terminal.

The right pane is the instrument. It stands where the git / file viewer would
eventually go, and meanwhile reports what Claude is asking the terminal to do:

| Field | Why it matters |
| --- | --- |
| `alt screen` | Should flip to `on` once Claude's UI starts. |
| `app cursor` | DECCKM. When `on`, arrows must be sent as `ESC O A`. |
| `bracketed paste` | When `on`, pasted text is wrapped so Claude can tell paste from typing. |
| `mouse mode` / `encoding` | Whether Claude wants mouse reports, and in which dialect. We stay silent unless asked. |
| `DSR answered` | Green and non-zero within the first moment. **Red zero means an imminent hang.** |
| `pty size` vs `parser size` | Must agree, and match the *inner* area of the bordered pane. |
| `bytes read` | Should climb steadily. Frozen = the reader thread died. |

## Pass criteria

All confirmed passing on 2026-08-01. Kept here as the regression checklist for
when the pty layer is lifted into the real project.

1. **Renders.** Claude's UI appears, colours intact, no stray escape sequences
   as literal text.
2. **Types.** One character per keystroke. *Two* means the release filter broke.
3. **Editing keys.** Arrows, Home/End, Backspace, Delete behave. Ctrl+C
   interrupts rather than inserting a control character.
4. **Resize.** Drag the window wider and narrower. Claude reflows, `resizes`
   increments, and the two size fields stay in agreement.
5. **Paste.** Multi-line paste arrives as one block, not a burst of Enters.
6. **Exit.** `/exit` closes the pty cleanly rather than hanging.

Steps 4 and 6 have automated coverage at the pty layer already (resize accepted
while a child is attached; children reaped via `try_wait`), so failures there
would be in the UI layer rather than the plumbing.

The fallback plan — drive an existing multiplexer like WezTerm instead of
hosting the pty ourselves — is no longer needed, but is worth remembering if the
pty layer ever becomes a maintenance burden.

## Deliberately out of scope

Not bugs — just not what a go/no-go needs:

- No focus model. Every keystroke goes to Claude; the right pane is read-only.
- No scrollback UI. 5000 lines are retained but nothing scrolls them.
- Right pane is static diagnostics, not git or files.
- No wide-character or grapheme-cluster stress testing.
- Only DSR is answered. Other terminal queries (Device Attributes, colour
  reports) are ignored; if Claude turns out to need one, it will present the
  same way — a stall with a tiny byte count.
