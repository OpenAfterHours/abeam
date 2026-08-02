# abeam

One window for a Claude Code session.

Claude runs in the left pane — hosted in a pty, parsed, and drawn by abeam, not
passed through to your terminal. The right pane shows the state of the git
worktree, the document Claude just wrote, or a shell to run things in. A file
watcher drives the first two, so neither has to be asked.

It replaces a three-window setup: Claude in one terminal, git in another, and an
editor open purely to read the markdown Claude produced.

```
┌ claude ──────────────────────────────┐┌ git · main ↑2 · 12 changed ──────┐
│                                      ││ Staged (1)          +40 -6       │
│  > implement the parser              ││   M crates/abeam/src/app.rs +40-6│
│                                      ││ Changed (1)          +7 -0       │
│  ● Writing crates/abeam/src/parse.rs ││   M docs/design.md          +7-0 │
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
reason abeam exists.

Everything else is a consequence of that. The panes are read-only, they never
take focus from Claude, and they never switch themselves — a pane that yanks
itself into view while you are reading is delightful twice and infuriating
thereafter.

## Installing

```
uvx abeam                 # run it without installing anything
uv tool install abeam     # or keep it on PATH
```

There is no Python in abeam. PyPI is the delivery van: the wheel's whole payload
is a compiled Windows binary, so `uvx` fetches a few hundred kilobytes and runs
it — no Rust toolchain, no build step, and nothing to compile on your machine.

## Requirements

- **Windows, x86-64.** Everything works because of specific ConPTY behaviour,
  and the pty test suites are `#![cfg(windows)]`. The code is not portable
  today, and on another platform the tests pass by saying nothing. Only x86-64
  wheels are published: ARM Windows would cross-compile in one line and would be
  a binary nobody has ever run, so `uvx` reporting no matching wheel is the more
  honest answer until someone can test it.
- `git` on `PATH`, for the git pane. Its absence is reported in the pane, not
  fatal.
- Rust 1.95+ and the MSVC toolchain — **only to build from source**. Plain
  `cargo build`, no vcvars wrapper.

## Running it

```
abeam            # hosts `claude` in the current directory
abeam pwsh       # hosts something else
```

The first argument is the program; everything after it is passed through. The
current directory is both the child's working directory and the root that the
git pane and the watcher use.

From a checkout, `cargo run -p abeam` and `cargo run -p abeam -- pwsh` do the
same.

## Keys

Everything abeam binds lives under `Alt` and the F-keys. **Nothing abeam claims
is a key Claude can act on** — the audit behind that is `docs/keymap.md`, and
`crates/abeam/src/keys.rs` has tests pinning it. Everything else you type goes
to Claude untouched.

| Key | |
| --- | --- |
| `Alt+G` | right pane → git |
| `Alt+E` | right pane → files / markdown (again for the file list) |
| `Alt+S` | right pane → a shell, **and focus it** (again to hand focus back) |
| `F2` | right pane → pty diagnostics, and back to what it displaced |
| `Alt+Left` / `Alt+Right` | move focus |
| `Alt+J` / `Alt+K` | scroll the right pane a line — **without focusing it** |
| `Alt+PgDn` / `Alt+PgUp` | scroll the right pane a page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (twice while a child is live) |
| `F1` | key help overlay |
| `Ctrl+\` or `F12` | send the *next* key to Claude verbatim |

Reading the right pane costs nothing: switching views and scrolling both work
while Claude still has focus. You only need focus to drive a selection — or to
type, which is what `Alt+S` is for and the only reason a view key moves focus.

Once the right pane *is* focused, plain keys work, deliberately the same
vocabulary as Claude's own transcript view: `j`/`k` and arrows for a line,
`space`/`b` and PgDn/PgUp for a page, `Ctrl+D`/`Ctrl+U` for a half page, `g`/`G`
and Home/End for the ends, `Tab`/`Shift+Tab` for the selection, `Enter` to open,
`r` to refresh. `t` swaps rendered markdown for its source; in the file list,
`/` finds a file anywhere under the root and `Backspace` goes up a directory.
`Esc` or `q` hands focus back to Claude.

The shell view is the exception, and it has to be: `Esc` and `q` belong to
whatever is running in it. `Alt+S` or `Alt+←` is the way out, and its border
says so rather than leaving you to find out.

`Ctrl+\` exists so abeam can never permanently shadow a Claude binding. If a
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

**files** — read-only markdown and source, and a way to reach any of it.
Markdown is rendered, not shown as source: headings, lists, tables, quotes, GFM
alerts, footnotes, and syntax-highlighted fenced code. `t` swaps that for the
source it was rendered from, highlighted and numbered like any other file, and
back. Source files get highlighting and a line-number gutter. Everything is
pre-wrapped to the pane's exact width and scrolled by physical row, so
jump-to-end lands where you asked. On startup it opens the newest markdown under
the root; after that it follows what gets written. If something arrives while
another view is showing it waits, and the border says `◆ Alt+E` rather than
switching under you.

A second `Alt+E` opens the **file list**: a gitignore-aware directory browser
starting wherever the open file lives. `Enter` descends or opens, `Backspace`
climbs — and lands on the directory you just left, so walking back up is a place
rather than a reset. `/` finds a file by name anywhere under the root, matched
as a subsequence over its path and ranked so a hit in the file name beats one in
a directory name; `Esc` cancels the find and stays put, because being thrown out
of the pane by the key that means "never mind" is the worst thing a filter box
can do. The list is the second half of the never-switch-under-you rule: a
document arriving from the watcher waits while you are walking a tree, exactly
as it waits behind the git view.

**shell** — `Alt+S`, and the reason it is here rather than in another window:
`git branch`, `uv run ruff format`, `cargo test`, run in the directory abeam was
pointed at, next to the session that is about to be told what they printed. It
is a real pty — `pwsh`, falling back to `powershell` then `cmd`, or whatever
`ABEAM_SHELL` names — started the first time the view is drawn and never before,
so a session that never asks for one never pays for it. `Alt+J`/`Alt+K` scroll
its history without focusing it, which is why they are not the arrow keys the
shell would read as history. A child that exits leaves its last screen up with
`Enter` to start another, and while it is dead `Esc` means what it means
everywhere else.

Nor will abeam close out from under it: Claude exiting holds the door rather
than killing whatever is running, and says `shell open · Alt+Q to quit` in the
left title. That is "open", not "busy" — ConPTY cannot be asked whether a
command is running, so a shell sitting at a prompt holds the door exactly as a
build does. Type `exit` in it, or `Alt+Q` twice.

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
crates/abeam-pty/    the pty host layer: ConPTY session, input encoding, DSR
crates/abeam/        the binary: shell, layout, focus, panes
crates/abeam/tests/end_to_end.rs   abeam itself, hosted in a pty and typed at
docs/conpty-findings.md   what the spike learned. Read before touching the pty.
docs/keymap.md            the keybinding collision audit
pyproject.toml            maturin's instructions for wrapping the binary
.github/workflows/        ci on every push; release on a v* tag
```

## Releasing

Tag it and push the tag:

```
git tag v0.0.1 && git push origin v0.0.1
```

`release.yml` builds the wheel on Windows and uploads it to PyPI through
**trusted publishing** — an OIDC exchange between GitHub and PyPI, so there is
no API token in this repository to leak or rotate. PyPI is configured to trust
one repository, one workflow file and one environment (`pypi`); renaming any of
the three breaks the publish, deliberately.

The version lives in `[workspace.package]` in `Cargo.toml` and nowhere else. The
workflow refuses to run if the tag disagrees with it, because a release where
those two differ is discovered by whoever installs the wrong thing.

`cargo run -p abeam-pty --example host` is a complete pty host in one file, kept
as the manual regression harness and as proof that `PtySession` is sufficient
without abeam. The one thing that reproduces the ConPTY stall itself is the
ignore-DSR arm of `conpty_stalls_until_the_dsr_query_is_answered` in
`crates/abeam-pty/tests/conpty.rs`, which pins it in both directions.

Build and test with `--all-targets`, or the examples bit-rot:

```
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Status

Working, and used. Not finished.

**Done.** The pty host layer, proven by a spike that ran a real Claude session
against it on 2026-08-01. All three right-hand views, the file list and the find
under it, the rendered/source toggle, and the watcher driving what it should.
Focus, zoom, help, the diagnostics view, and the literal-next escape hatch.
248 tests, `clippy --all-targets` clean.

**Verified how.** The whole path — pty → parser → pane, git worker → real `git`
→ rendered rows, watcher → both panes, Enter in git → open in files — is
covered by tests that spawn a real ConPTY child and draw real frames.

On top of that, `crates/abeam/tests/end_to_end.rs` does to abeam what abeam does
to Claude: it spawns **the built binary** in a ConPTY, types at it as bytes, and
reads the screen that comes back. That is what proves the parts no in-process
test can reach — that abeam starts at all, that raw mode and the alternate
screen survive being someone else's child, that `Alt+S` written as `ESC s`
becomes the binding it should, and that a command typed into the shell view runs
in the right directory and puts its answer on screen. Three paths are pinned that
way today: type a command in the shell and read its output; reach a file nothing
pointed the pane at, by `Alt+E` `Alt+E` `/`; and a copy of a real shell planted
in the repository under the name abeam is about to look for, which abeam must
refuse to run. That last one is a test about an attack rather than a feature —
Windows resolves a bare program name against the *calling* process's directory
before it consults `PATH`, so a `pwsh.exe` committed to a cloned repo was one
keystroke from executing. For the whole session abeam now stands in
`%SystemRoot%` and hands its pty absolute paths only.

What none of it covers is a human at a terminal. The six pass criteria in
`docs/conpty-findings.md` (rendering, one character per keystroke, editing keys,
live resize, paste, `/exit`) were confirmed against the spike's host and have
not been re-run against `abeam` itself since the panes landed. Do that before
trusting it with real work.

**Not done, and known.**

- **The shell view has never been driven by a human.** A test types `set /a
  123*456` into the real binary and reads `56088` back off the screen, which is
  more than a smoke test and still less than use: the six pass criteria above
  were confirmed against Claude in the *left* pane, and a shell in a
  46-column right pane is not the same thing. Expect the first real `cargo test`
  run in there to find something about width, wrapping or the scrollback that no
  test thought to ask about.
- **`Alt+J` is on borrowed time.** Claude has a live `app:toggleTerminal` action
  with no default key, and its footer already prints `meta + j` as a fallback —
  so Claude's own UI advertises a key abeam has claimed. Nothing is bound today
  and the invariant holds; the day it is bound, abeam has to move.
  `docs/keymap.md` carries the details.
- **An npm-installed `claude` cannot be hosted.** `npm i -g` puts three files in
  `%APPDATA%\npm`: `claude` (a POSIX shell script, no extension), `claude.cmd`
  and `claude.ps1`. Windows starts only `.exe` and `.com` directly, so plain
  `abeam` fails at startup for those users — and the two obvious fixes are both
  wrong: preferring the `.cmd` does not help, because `CreateProcessW` cannot
  run that either, and going through `cmd.exe /c` hands abeam's own argv to a
  command-line re-parser that treats `&`, `|` and `^` as syntax. The native
  installer (`claude.exe`) is unaffected. Fixing it properly is a decision about
  how abeam launches its main program, not a patch.
- **Windows only.** Not a portability bug so much as an absence of work: nothing
  outside `abeam-pty` is platform-specific, but nothing has been tried.
- **No configuration.** Keybindings, the split ratio and the refresh interval are
  all constants. Claude's own bindings are user-configurable, so abeam's should
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
- **Two Claude features are unreachable inside abeam**, and both will be reported
  as abeam bugs: `Ctrl+Shift+B` and `Ctrl+Shift+C` are indistinguishable from
  `Ctrl+B` / `Ctrl+C` in legacy terminal encoding, and hold-to-talk voice needs
  key *release* events, which abeam drops for a load-bearing reason
  (`docs/conpty-findings.md`, constraint 3).
- **`EnableMouseCapture` disables your terminal's native text selection.**
  Copying out of abeam needs Shift+drag, and which terminals honour that varies.
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

## License

MIT. See [LICENSE](LICENSE).
