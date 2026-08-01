# forge

One window for a Claude Code session.

Claude runs in the left pane — hosted in a pty, parsed, and drawn by forge, not
passed through to your terminal. The right pane shows either the state of the
git worktree or the document Claude just wrote. A file watcher drives both, so
neither has to be asked.

It replaces a three-window setup: Claude in one terminal, git in another, and an
editor open purely to read the markdown Claude produced.

```
┌ claude ──────────────────────────────┐┌ git · main ↑2 · 12 changed ──────┐
│                                      ││ Staged (1)          +40 -6       │
│  > implement the parser              ││   M crates/forge/src/app.rs +40-6│
│                                      ││ Changed (1)          +7 -0       │
│  ● Writing crates/forge/src/parse.rs ││   M docs/design.md          +7-0 │
│                                      ││ Untracked (1)                    │
│                                      ││   ? notes/                       │
│                                      ││ Recent                           │
│                                      ││   a1b2c3d  2m   parser skeleton  │
└──────────────────────────────────────┘└──────────────────────────────────┘
```

## Why this rather than wezterm + lazygit + glow

Because the right pane knows what the agent just did. A watcher on the
repository root feeds both views: markdown Claude writes becomes the document on
screen, and any file it touches refreshes git within one debounce interval. That
is the only thing here you cannot assemble out of existing tools, and it is the
reason forge exists.

Everything else is a consequence of that. The panes are read-only, they never
take focus from Claude, and they never switch themselves — a pane that yanks
itself into view while you are reading is delightful twice and infuriating
thereafter.

## Requirements

- **Windows.** Everything works because of specific ConPTY behaviour, and the
  pty test suites are `#![cfg(windows)]`. The code is not portable today, and on
  another platform the tests pass by saying nothing.
- Rust 1.95+, MSVC toolchain. Plain `cargo build` — no vcvars wrapper.
- `git` on `PATH`, for the git pane. Its absence is reported in the pane, not
  fatal.

## Running it

```
cargo run -p forge            # hosts `claude` in the current directory
cargo run -p forge -- pwsh    # hosts something else
```

The first argument is the program; everything after it is passed through. The
current directory is both the child's working directory and the root that the
git pane and the watcher use.

## Keys

Forge's own bindings live under `Alt` and the F-keys. **Nothing forge claims is
a key Claude can act on** — the audit behind that is `docs/keymap.md`, and
`crates/forge/src/keys.rs` has tests pinning it. Everything else you type goes
to Claude untouched.

| Key | |
| --- | --- |
| `Alt+G` | right pane → git |
| `Alt+E` | right pane → files / markdown (again to reload) |
| `F2` | right pane → pty diagnostics, and back to what it displaced |
| `Alt+Left` / `Alt+Right` | move focus |
| `Alt+J` / `Alt+K` | scroll the right pane a line — **without focusing it** |
| `Alt+PgDn` / `Alt+PgUp` | scroll the right pane a page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (twice while a child is live) |
| `F1` | key help overlay |
| `Ctrl+\` or `F12` | send the *next* key to Claude verbatim |

Reading the right pane costs nothing: switching views and scrolling both work
while Claude still has focus. You only need focus to drive a selection.

Once the right pane *is* focused, plain keys work, deliberately the same
vocabulary as Claude's own transcript view: `j`/`k` and arrows for a line,
`space`/`b` and PgDn/PgUp for a page, `Ctrl+D`/`Ctrl+U` for a half page, `g`/`G`
and Home/End for the ends, `Tab`/`Shift+Tab` for the selection, `Enter` to open,
`r` to refresh. `Esc` or `q` hands focus back to Claude.

`Ctrl+\` exists so forge can never permanently shadow a Claude binding. If a
future Claude release binds `Alt+G`, `Ctrl+\` then `Alt+G` still reaches it.

## The panes

**git** — read-only. Branch, ahead/behind, staged / unstaged / untracked files
with per-file line counts, and recent commits. Every `git` call is a read: it
stages nothing and commits nothing, which matters because it refreshes itself
unasked. Refreshes when the watcher sees a write, and on a two-second safety
timer for changes the watcher cannot see (`.git` is deliberately not watched, so
a commit made in another terminal arrives on the timer). `Enter` opens the
selected file in the viewer, when the row names one to open: git collapses an
untracked tree to a single `dir/` entry and there is no directory view to open
one in, and a deletion is a path git has just finished saying is gone. Enter
does nothing on those, rather than trading the view you are reading for a
viewer saying "no such file".

**files** — read-only markdown and source. Markdown is rendered, not shown as
source: headings, lists, tables, quotes, GFM alerts, footnotes, and
syntax-highlighted fenced code. Source files get highlighting and a line-number
gutter. Everything is pre-wrapped to the pane's exact width and scrolled by
physical row, so jump-to-end lands where you asked. On startup it opens the
newest markdown under the root; after that it follows what gets written. If
something arrives while the git view is showing it waits, and the border says
`◆ Alt+E` rather than switching under you.

**pty diagnostics** (`F2`) — what the emulation layer is doing: alt-screen,
application cursor, bracketed paste, mouse mode, byte counts, resize count, and
the pty and parser sizes side by side, to be compared with the pane you can see
rather than with each other. **DSR answered** is the one that matters:
ConPTY blocks until its opening cursor query is answered, so a red zero there
means the session is hung rather than slow. `docs/conpty-findings.md` explains
each field and why it is on screen.

## Layout

60/40 split, and below 60 columns the right pane collapses entirely rather than
squeezing Claude into 36. The pty is sized from exactly the rect that was drawn,
once per frame, which is also what coalesces a window drag into a single ConPTY
resize.

## Repository

```
crates/forge-pty/    the pty host layer: ConPTY session, input encoding, DSR
crates/forge/        the binary: shell, layout, focus, panes
docs/conpty-findings.md   what the spike learned. Read before touching the pty.
docs/keymap.md            the keybinding collision audit
```

`cargo run -p forge-pty --example host` is a complete pty host in one file, kept
as the manual regression harness and as proof that `PtySession` is sufficient
without forge. The one thing that reproduces the ConPTY stall itself is the
ignore-DSR arm of `conpty_stalls_until_the_dsr_query_is_answered` in
`crates/forge-pty/tests/conpty.rs`, which pins it in both directions.

Build and test with `--all-targets`, or the examples bit-rot:

```
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Status

Working, and used. Not finished.

**Done.** The pty host layer, proven by a spike that ran a real Claude session
against it on 2026-08-01. Both right-hand views. The watcher driving both.
Focus, zoom, help, the diagnostics view, and the literal-next escape hatch.
168 tests, `clippy --all-targets` clean.

**Verified how.** The whole path — pty → parser → pane, git worker → real `git`
→ rendered rows, watcher → both panes, Enter in git → open in files — is
covered by tests that spawn a real ConPTY child and draw real frames, plus a
smoke run of the binary hosting `cmd.exe`. What that does *not* cover is a
human at a terminal: the six pass criteria in `docs/conpty-findings.md`
(rendering, one character per keystroke, editing keys, live resize, paste,
`/exit`) were confirmed against the spike's host, and have not been re-run
against `forge` itself since the panes landed. Do that before trusting it with
real work.

**Not done, and known.**

- **Windows only.** Not a portability bug so much as an absence of work: nothing
  outside `forge-pty` is platform-specific, but nothing has been tried.
- **No configuration.** Keybindings, the split ratio and the refresh interval are
  all constants. Claude's own bindings are user-configurable, so forge's should
  be too before anyone else uses it.
- **No diff view.** The git pane shows *which* files changed and by how many
  lines, not what changed in them.
- **The first source file with code in it costs a visible hitch** of 100–200 ms,
  while syntect deserialises its syntax and theme dumps. A session that only
  reads prose never pays it.
- **Laying a document out happens on the frame path**, and stalling a frame
  stalls Claude's keystrokes. Reading is the cheap half; parsing, highlighting
  and wrapping is the expensive one, and it runs again at every new width, so
  dragging the window border with a large document open pays it per frame. The
  only bound on it is the 512 KiB read cap — about 210 ms of layout in a release
  build, measured — and going over that is visible: the pane says where it
  stopped. Highlighting gives up above 64 KiB and shows plain text. A slow
  network share can still stall the frame that opens a file.
- **Source highlighting assumes a dark terminal** (base16-ocean.dark). A TUI
  cannot ask the terminal for its palette. Backgrounds are discarded so that a
  light theme is washed out rather than unreadable.
- **UTF-16 files are reported as binary.** The sniff is a NUL byte in the first
  8 KiB, which is what git does.
- **Two Claude features are unreachable inside forge**, and both will be reported
  as forge bugs: `Ctrl+Shift+B` and `Ctrl+Shift+C` are indistinguishable from
  `Ctrl+B` / `Ctrl+C` in legacy terminal encoding, and hold-to-talk voice needs
  key *release* events, which forge drops for a load-bearing reason
  (`docs/conpty-findings.md`, constraint 3).
- **`EnableMouseCapture` disables your terminal's native text selection.**
  Copying out of forge needs Shift+drag, and which terminals honour that varies.
- **The `Alt` namespace is free against one audited Claude build** (2026-07-25).
  It is not guaranteed free forever; `Ctrl+\` is the mitigation.

## A warning for anyone changing the pty layer

`docs/conpty-findings.md` lists five constraints that look like things to tidy
up and are not. The shortest version: ConPTY hangs forever if you do not answer
its opening `ESC [ 6 n`, `wait()` never returns so only `try_wait()` is safe,
Windows sends a Release event for every key and forwarding it double-types
everything, mouse reports must be gated on what the hosted app enabled, and
`Screen::contents()` tells you nothing about layout.

There are tests pinning all five. Read the document first.
