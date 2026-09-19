# Rendering pipeline and validation

Abeam renders hosted terminal output through `vt100` and `tui-term`, then emits
Ratatui buffer differences to the outer terminal. The rendering changes address
visible caret movement and incomplete child repaints at both boundaries.

The investigation reproduced two independent cursor owners, screen/cursor reads
from different moments, and three underlying writer calls for a small outer
frame. In the Windows ConPTY probe, synchronized-update markers survived but
the closing marker could precede the last rewritten screen bytes. Outer frame
buffering alone cannot repair an incomplete child screen.

**Outer frames.** `term::FrameBackend` stages backend writes until explicit
commit, including backend operations that normally flush. A frame emits begin
synchronized update, cursor hide, cell differences, final cursor position,
optional cursor show, and end synchronized update in that order. Ordinary
frames make one application-level write; short writes and interruptions may
require more and are counted. This does not guarantee that an OS or terminal
will receive the data in one chunk. Staging also handles frames larger than
64 KiB. Failed or unwinding frames discard staged bytes and make a best-effort
attempt to end synchronized mode and restore the cursor.

The terminal widget's software cursor is disabled. Rendering captures the native
cursor together with the committed cells under one screen guard. Only the
focused pane positions it. A scrolled-back pane hides the caret. Selection uses
the displayed screen; input encoding and the final exit transcript use live state.

**Child publication.** `abeam-pty::PtySession::screen()` remains the live parser,
used for input modes, diagnostics and DSR replies. `display_screen()` exposes a
separate persistent parser that replays committed bytes. This doubles stored
screen/scrollback state, but avoids copying the full history for each frame.

A streaming parser detects mode 2026 begin/end, reset and cursor queries across
read boundaries. A completed repaint preceding the next begin in the same read
can be published while the new repaint is held. Device Status Reports are
answered from the live cursor at each query, without waiting for publication.

Publication limits:

| Case | Policy |
| --- | --- |
| Synchronized repaint without an end marker | Recover after 100 ms, including when no further read arrives. Repeated begin markers do not extend the deadline. |
| Windows trailing/unmarked bytes | Coalesce until 4 ms of quiet, with a 16 ms maximum tail window. |
| Pending bytes reach 256 KiB | Publish and recover to bound buffered memory. |
| Reset, resize, scrollback change or child exit | Recover the pending state while preserving parser geometry and view consistency. |

The Windows quiet interval is a bounded heuristic for ConPTY's output ordering,
not a protocol guarantee. Unix unmarked output is published immediately.
A per-session Condvar worker handles deadlines independently of the blocking
reader. It is stopped on EOF or session drop. The UI and reader do not sleep to
coalesce output; notifications are coalesced through the published dirty flag.

**Scheduling and diagnostics.** The existing 8 ms minimum frame interval is
retained. Each wake batch processes at most 64 events or about 2 ms of work
(checked between events), so a continuously replenished channel cannot starve
drawing indefinitely. Unconsumed input remains queued.

The diagnostics pane (`F1`, then `D`) reports pane rendering, backend encoding/
diffing, output-write time, frame bytes/writes, pane maintenance and PTY resize
time. Reader lock wait and read parsing are separate PTY values; read parsing
does not include later replay performed by the publication worker. Publication
and synchronization-timeout counters help identify a child that leaves updates
open.

Wake-to-write latency starts at input receipt or published-output notification
and ends when the application finishes writing the frame. It excludes held
child updates and terminal display time. The p95/p99 values cover the last
120 wake-triggered frames; polled-only frames do not contribute samples and
report zero for the last wake delay.

**Repeatable checks.** The automatic regressions exercise cursor ordering and
focus, successive visible/hidden frames, large and short writes, injected I/O
failures and panic cleanup; split synchronized/UTF-8/escape sequences, false
markers inside OSC strings, completed-plus-pending updates, DSR during holds,
missing end markers without later reads, timer shutdown, oversized updates,
resize, scrollback and exit. A replenished-channel test checks that the wake
batch yields without discarding input.

Two release-mode probes use a filled 5,000-row history and require no agent
account or network:

```powershell
cargo run --release -p abeam --example render_bench
cargo test --release -p abeam-pty benchmark_publication -- --ignored --nocapture
```

The frame probe compares the old buffered writer with the production frame
adapter at 120 x 38 and 240 x 58 cells. The PTY probe compares the original
single-parser work with live/committed parsing for one-cell changes, full panes
and a 256 KiB burst. Both measure application work, not terminal display time.

Windows release-mode results from the implementation check (milliseconds per
frame, recording sink, 5,000 populated history rows):

| Pane | Update | Previous frame | Current frame | Application writes |
| --- | --- | ---: | ---: | --- |
| 120 x 38 | One cell | 0.184 | 0.182 | 3 to 1 |
| 120 x 38 | Full pane | 0.348 | 0.340 | 3 to 1 |
| 240 x 58 | One cell | 0.610 | 0.576 | 3 to 1 |
| 240 x 58 | Full pane | 1.114 | 1.069 | 3 to 1 |

The separate full-pane parsing/publication probe measured 0.105 ms and 0.233 ms
at those sizes, approximately twice the original single-parser work. Feeding
256 KiB in one benchmark call cost 6.6-8.3 ms; production reads are 8 KiB, but
publishing a full pending buffer still replays its bytes under the screen lock.
These measurements support retaining the existing frame cadence and avoiding
unnecessary whole-pane caching or display-rate changes in this patch.

A matched direct-Codex versus Abeam visual check remains useful after automated
validation: use the same CLI/terminal versions and child dimensions, stream a
long response, type during streaming, resize, change focus and scroll through
history. Check for duplicate or traveling carets, incomplete screen updates and
growing display delay. Synthetic results must not be presented as proof of
perceived smoothness in a live terminal.
