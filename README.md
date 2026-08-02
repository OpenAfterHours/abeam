# abeam

One window for an AI coding session.

Your agent runs in the left pane — hosted in a pty, parsed, and drawn by abeam,
not passed through to your terminal. The right pane shows the state of the git
worktree, the document the agent just wrote, or a shell to run things in. A file
watcher drives the first two, so neither has to be asked.

It replaces a three-window setup: the agent in one terminal, git in another, and
an editor open purely to read the markdown it produced.

Two agents are known by name today — **Claude Code** and **GitHub Copilot CLI**
— and any other program on `PATH` can be hosted by naming it. Which one you got
is not a thing to remember: the left border says. Read the Copilot half of this
document knowing that **abeam has never been run with Copilot CLI**, not once:
the selection, the launcher and the failure message are tested, a session is
not, and "Not done, and known" says exactly why and what that leaves unproven.

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

That title is the name you chose and never the path it resolved to, which is
why an npm-installed agent started by way of `cmd.exe` still says its own name
rather than the interpreter's. `abeam copilot` should therefore draw `┌ copilot
─…` in the same place — "should" because that follows from the code and from a
test of the naming, and nobody has watched it happen.

## Why this rather than wezterm + lazygit + glow

Because the right pane knows what the agent just did. A watcher on the
repository root feeds both views: markdown the agent writes becomes the document
on screen, and any file it touches refreshes git within one debounce interval.
That is the only thing here you cannot assemble out of existing tools, and it is
the reason abeam exists.

Everything else is a consequence of that. The panes are read-only, they never
take focus from the agent, and they never switch themselves — a pane that yanks
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
- **An agent to host**, which abeam does not install for you — see below for
  what each one wants.
- Rust 1.95+ and the MSVC toolchain — **only to build from source**. Plain
  `cargo build`, no vcvars wrapper.

## Running it

```
abeam                  # the default agent, in the current directory
abeam copilot          # GitHub Copilot CLI
abeam powershell       # anything else on PATH
abeam claude --resume  # everything after the name belongs to the agent
```

The first word that is not a flag is the whole of the selection. If it names an
agent abeam knows — `claude` or `copilot`, matched without regard to case —
abeam looks that agent's executables up on `PATH`; anything else is a program
name and means exactly what `abeam powershell` has always meant. `ABEAM_AGENT`
names the default for a shell where you always want the same one, and it is read
on precisely those terms: an agent name, or any program. An empty value counts
as unset, because PowerShell leaves one behind and `'' was not found on PATH`
names nothing anyone can act on.

**abeam parses its own flags only up to that first non-flag word**, and it has
two of them: `-h`/`--help` and `-V`/`--version`. Everything from the selector
onwards is the child's, `--help` included — `abeam claude --help` is a question
for Claude, and a multiplexer that quietly ate a flag meant for the thing it is
hosting would be wrong in a way that is very hard to see from the outside. `--`
first is the escape hatch, and covers the one case the rule cannot: a program
whose own name starts with a dash.

There is no `--agent` flag. The positional already selects, so a flag beside it
would make `abeam --agent copilot powershell` expressible, and there is no
honest answer to what that would mean.

An agent abeam cannot find is a sentence and never a download. Modern `gh
copilot` is a launcher rather than the retired suggest/explain extension: it
will fetch the Copilot CLI if it is missing, and abeam had that fallback for a
day before it was taken out on purpose. Typing `abeam copilot` is a request to
run something, not consent for a network install, and a terminal border cannot
close that gap after the fact. So what you get instead is four things: the
names that were looked for, what the operating system said about the last of
them, then — when another agent abeam knows is already sitting on `PATH` — the
line that saves the ten minutes, `` `claude` is installed; `abeam claude` would
host it. ``, and last one sentence on installing the one you asked for, `gh
copilot` included, as a command you run yourself. The third is the best line in
the message and the one most often needed: the default agent missing on a
machine where the other one is right there is a session one word away from
starting, and nothing else on that screen would say so.

What each agent wants:

- **Claude Code** — the native installer, which writes `claude.exe`, or `npm i
  -g @anthropic-ai/claude-code`, which writes a `claude.cmd` shim. Both are
  hostable; the shim was not until recently, and the story is under "Not done,
  and known".
- **GitHub Copilot CLI** — `winget install GitHub.Copilot`, or run `gh copilot`
  once to fetch it. There is an npm package too, `@github/copilot`, but it wants
  Node 22 or newer, which is the reason `gh copilot` is worth naming: it is the
  route that works where the other two do not.

The current directory is both the child's working directory and the root that
the git pane and the watcher use.

From a checkout, `cargo run -p abeam` and `cargo run -p abeam -- copilot` do the
same.

## Keys

Everything abeam binds lives under `Alt` and the F-keys, with one exception:
`Ctrl+\`, the escape hatch at the foot of the table, which has to be reachable
on the day an `Alt` key stops being safe and so cannot live in the namespace it
exists to rescue. (`F12` is its alias, for layouts that put backslash behind
AltGr.) **Nothing abeam claims is a key the hosted agent can act on** — the
audit behind that is `docs/keymap.md`, and `crates/abeam/src/keys.rs` has tests
pinning it. Everything else you type goes to the agent untouched.

| Key | |
| --- | --- |
| `Alt+G` | right pane → git |
| `Alt+E` | right pane → files / markdown (again for the file list) |
| `Alt+S` | right pane → a shell, **and focus it** (again to hand focus back) |
| `F2` | right pane → pty diagnostics, and back to what it displaced |
| `F3` | file reader → light / dark page |
| `F4` / `F5` | move focus left / right |
| `Alt+J` / `Alt+K` | scroll the right pane a line — **without focusing it** |
| `Alt+PgDn` / `Alt+PgUp` | scroll the right pane a page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (twice while a child is live) |
| `F1` | key help overlay |
| `Ctrl+\` or `F12` | send the *next* key to the agent verbatim |

Focus was `Alt+←`/`Alt+→` until abeam gained a second agent, and it is `F4`/`F5`
now because GitHub's own command reference declares that pair as word-motion in
Copilot CLI on Windows and Linux — a vendor-declared collision, not a suspicion,
and inside abeam it would have left a Copilot user with no way to move by a word
at all. The arrows go to the agent now. `docs/keymap.md` has the evidence, and
the argument about which side should yield.

Reading the right pane costs nothing: switching views and scrolling both work
while the agent still has focus. You only need focus to drive a selection — or
to type, which is what `Alt+S` is for and the only reason a view key moves
focus.

Once the right pane *is* focused, plain keys work, deliberately the same
vocabulary as Claude's own transcript view — which Copilot's diff mode shares
most of, on GitHub's published tables rather than on anything read out of a
binary, and differs from in two places worth naming: `space` pages nothing
there, and `b` toggles the diff rather than paging back. Neither reaches inside
abeam, where these keys are the pane's; one scroll language is close to serving
both rather than exactly serving both. The vocabulary itself: `j`/`k` and
arrows for a line, `space`/`b` and PgDn/PgUp for a page,
`Ctrl+D`/`Ctrl+U` for a half page, `g`/`G` and Home/End for the ends,
`Tab`/`Shift+Tab` for the selection, `Enter` to open, `r` to refresh. `t` swaps
rendered markdown for its source; in the file list, `/` finds a file anywhere
under the root and `Backspace` goes up a directory. `Esc` or `q` hands focus
back to the agent.

The shell view is the exception, and it has to be: `Esc` and `q` belong to
whatever is running in it. `Alt+S` or `F4` is the way out, and its border says
so rather than leaving you to find out.

`Ctrl+\` exists so abeam can never permanently shadow a binding of the agent you
are typing at. If a future Claude or Copilot release binds `Alt+G`, `Ctrl+\`
then `Alt+G` still reaches it.

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

Nor will abeam close out from under it: the agent exiting holds the door rather
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

It also reports the frame clock — frames drawn, the rate over the last second,
and the **worst** single frame in it rather than an average, because a stutter
is one slow frame in a hundred and a mean hides exactly that. While the agent is
producing output the rate should sit at the frame floor; a worst frame
approaching the gap between frames means the renderer is what is setting the
pace, and no amount of pacing will help.

## Drawing

Two intervals, with different jobs.

The agent's output does not wait to be noticed: the pty reader rings the draw
loop the moment bytes land, so nothing sits in a buffer waiting for a poll to
come round. What the loop does *not* do is draw every time it is rung — an agent
produces output far faster than a screen can show it, and drawing on every
chunk spends the whole budget on frames nobody sees while the console write
queue backs up. Frames are held to one every 8 ms, start to start, always
showing the newest state. A burst becomes one frame, and the frames that go out
land evenly, which is what reads as smooth; uneven frames read as jitter at any
rate.

The other interval is the 10 ms tick, which is now only what polls the things
without a doorbell of their own: the git worker's channel, a viewer walk
finishing, a shell child's exit.

Each frame is written between `BeginSynchronizedUpdate` and `End` (DEC 2026), so
a terminal that understands them composites the whole frame or none of it rather
than showing a seam partway down a repaint. A terminal that does not know the
sequence ignores it.

## Layout

60/40 split, and below 60 columns the right pane collapses entirely rather than
squeezing the agent into 36. The pty is sized from exactly the rect that was
drawn, once per frame, which is also what coalesces a window drag into a single
ConPTY resize.

## Repository

```
crates/abeam-pty/    the pty host layer: ConPTY session, input encoding, DSR
crates/abeam/        the binary: shell, layout, focus, panes
crates/abeam/src/agent.rs          the agents abeam knows, and how one is chosen
crates/abeam/src/launch.rs         what may be handed to CreateProcessW
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
Agent selection and the launcher underneath it. 299 tests, and
`clippy --all-targets` clean.

**Verified how.** The whole path — pty → parser → pane, git worker → real `git`
→ rendered rows, watcher → both panes, Enter in git → open in files — is
covered by tests that spawn a real ConPTY child and draw real frames.

On top of that, `crates/abeam/tests/end_to_end.rs` does to abeam what abeam does
to an agent: it spawns **the built binary** in a ConPTY, types at it as bytes,
and reads the screen that comes back. That is what proves the parts no in-process
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
- **abeam has never been run with Copilot CLI.** Not once, not for a minute. It
  is not installed on the machine abeam is developed on and cannot easily be:
  the npm package wants Node 22 and this box has v20.14.0, and neither `winget
  install GitHub.Copilot` nor `gh copilot` has been run to get around that. What
  *is* exercised is the selection and the failure message — against agent tables
  injected into the tests, because a real table exercises whichever branch the
  machine it runs on happens to make reachable, which on a build server is the
  other one — and the launcher, which is the same code path Claude and the
  command view's shells already take every day. None of that is a session. The
  first real one will find something.
- **The Copilot keymap audit is documentation- and source-derived**, and that is
  weaker evidence than the Claude half of `docs/keymap.md` rests on. Those
  bindings came out of strings extracted from the installed binary. Copilot's
  come from GitHub's published shortcut tables, about 150 changelog entries and
  Ink's source — which is exactly the kind of audit that would have cleared
  `Alt+F` in Claude, and `Alt+F` was bound. The document says so at length, and
  lists the two steps that would upgrade it.
- **Nine of abeam's `Alt` bindings are *probable* no-ops in Copilot, not
  verified ones, and six of the nine are worse than that.** The nine are every
  `Alt` key abeam claims: `Alt+G`, `Alt+E`, `Alt+S`, `Alt+Q`, `Alt+Z`,
  `Alt+J`, `Alt+K`, `Alt+PageUp` and `Alt+PageDown`. Each was looked for in
  GitHub's tables and in about 150 changelog entries and found nowhere, which
  is the best a documentation-derived audit can do and is exactly the evidence
  that would have cleared `Alt+F` in Claude.

  Six of them carry a second problem on top. Ink parses `Alt`+letter into the
  bare letter plus a `meta` flag, and reports `Alt+PageUp` as PageUp with
  `meta` set — either way the handler is handed the unmodified key together
  with a flag it is free to ignore, so `Alt+J` reaches a handler as
  `input === "j"` with `key.meta` true, and a handler written
  `if (input === "j")` fires on both. Copilot binds `g`, `s`, `j`, `k`, PageUp
  and PageDown bare somewhere — diff mode, the session picker, the timeline —
  so `Alt+G`, `Alt+S`, `Alt+J`, `Alt+K`, `Alt+PageUp` and `Alt+PageDown` each
  shadow a key it really does act on in some view. GitHub has shipped fixes for
  precisely this class of bug: 1.0.71 stopped modified vim keys moving the
  selection in tool-permission prompts. Nothing a user can reach is shadowed,
  because the bare keys still pass through untouched — but the strict form of
  abeam's invariant, that an intercepted key is one the agent could not have
  acted on, is unproven for those six, and only the binary can settle it.
  `docs/keymap.md` grades every one of the nine and says the same thing twice.
- **`Alt+J` is on borrowed time.** Claude has a live `app:toggleTerminal` action
  with no default key, and its footer already prints `meta + j` as a fallback —
  so Claude's own UI advertises a key abeam has claimed. Nothing is bound today
  and the invariant holds; the day it is bound, abeam has to move.
  `docs/keymap.md` carries the details.
- **An npm-installed agent is hosted now, and this README used to say it could
  not be.** `npm i -g` puts three files in `%APPDATA%\npm`: `claude` (a POSIX
  shell script, no extension), `claude.cmd` and `claude.ps1`. Windows starts
  `.exe` and `.com` and nothing else, so plain `abeam` did fail at startup for
  those users. The entry that said so also called `cmd.exe /c` one of "two
  obvious fixes [that] are both wrong", on the grounds that it hands abeam's own
  argv to a command-line re-parser treating `&`, `|` and `^` as syntax. That
  half was itself wrong, and deleting it quietly would be the worse correction.
  `crates/abeam/src/launch.rs` routes a `.cmd` or a `.bat` by naming `cmd.exe`
  in front of it, and going through the re-parser is safe *because* of the
  argument quoting rather than despite it: the command line is built the way
  Rust's own `make_bat_command_line` builds one — the fix for CVE-2024-24576,
  "BatBadBut" — quoting eagerly rather than trying to enumerate `cmd`'s syntax
  and coming up one character short, doubling an embedded quote as `""` because
  that is the one spelling both `cmd` and every CRT since Visual C++ 2008 read,
  and refusing outright the three characters nothing escapes — a NUL, a
  carriage return and a newline, the last two because one truncates the command
  and the other ends it outright, and silently dropping the tail of an argument
  is what an injection reads like from the outside. Two more things are refused
  with a sentence rather than escaped. The script's own path may not contain a
  `"` or end with a `\`, either of which would close the quote around it and
  turn the rest of the line into syntax; Windows file names can hold neither,
  so nothing legitimate is turned away. And the finished line may not exceed
  8124 characters, which is `cmd`'s limit rather than abeam's and was measured
  rather than read off the documented 8191 — because 8191 is not where the
  behaviour changes, and the difference is the dangerous part: past it `cmd`
  starts nothing, prints nothing and exits 0, so an over-long argument drew an
  empty pane and reported success. A natively installed agent is an `.exe`
  abeam starts directly and takes roughly four times as much. The line reaches
  `cmd` in `%ABEAM_LAUNCH%` rather than on the wire,
  because portable-pty applies MSVCRT argv quoting to everything it is handed
  and `cmd` has no backslash escape with which to read what that produces. A
  real shim in a real pty, in a directory with a space in it and with an `&` in
  one of its arguments, is what pins the whole of it.

  Two things are still refused, and both deliberately. An *extensionless* POSIX
  shim on its own cannot be started, because there is no shell on Windows that
  is the right one to hand it to. A `.ps1` is refused rather than routed:
  `powershell` and `pwsh` load different profiles under different execution
  policies, so guessing is guessing which of your profiles runs. Neither is
  normally reached — npm writes a `.cmd` beside both, `PATHEXT` is probed ahead
  of the bare file name, and `.PS1` is not on `PATHEXT` at all — so arriving at
  either message means the sibling that would have worked is missing.
- **Windows only.** Not a portability bug so much as an absence of work: nothing
  outside `abeam-pty` is platform-specific, but nothing has been tried.
- **Almost no configuration**, and the "almost" is two environment variables.
  Keybindings, the split ratio and both drawing intervals are constants in the
  source. `ABEAM_AGENT` names the agent — or the program — to host when the
  command line names none, and `ABEAM_SHELL` names what the command view starts.
  There is no file, so nothing survives the session: the reader's light/dark
  choice starts dark every time. Claude's own bindings are user-configurable and
  abeam's should be too before anyone else uses it. Copilot's are not, which
  makes that the mirror image of the same gap rather than a reason to be relaxed
  about it.
- **A scrolling pane is a full repaint.** ratatui diffs by cell and has no notion
  of a scroll region, so when the agent's output scrolls, every row has changed
  and the whole pane is rewritten — about 10 KB of escape sequences, measured
  at a 118×45 pane. Windows Terminal keeps up with that comfortably; it is
  still the structural ceiling on how cheap a frame can get, and the thing to
  attack next if the F2 worst-frame figure ever says the renderer is the limit.
- **No diff view.** The git pane shows *which* files changed and by how many
  lines, not what changed in them.
- **The first source file with code in it costs a visible hitch** of 100–200 ms,
  while syntect deserialises its syntax and theme dumps. A session that only
  reads prose never pays it.
- **Laying a document out happens on the frame path**, and stalling a frame
  stalls the agent's keystrokes. Reading is the cheap half; parsing,
  highlighting and wrapping is the expensive one, and it runs again at every
  new width, so dragging the window border with a large document open pays it
  per frame. The only bound on it is the 512 KiB read cap — about 210 ms of
  layout in a release build, measured — and going over that is visible: the
  pane says where it stopped. Highlighting gives up above 64 KiB and shows
  plain text. A slow network share can still stall the frame that opens a file.
- **The reader is the only pane that paints its own background.** `F3` gives it
  a light or a dark page — with a matching syntax theme, since base16-ocean.dark
  on a white page is washed out — and that is what makes one key enough in a
  bright room. Everything else still draws in named ANSI colours on the
  terminal's own background, so the git and diagnostics views follow whatever
  profile the terminal has and the two pty views show whatever their child sent.
  A TUI still cannot ask the terminal for its palette; the reader sidesteps the
  question by owning every colour inside its own rect. The choice is per session
  and starts dark — there is nowhere to persist it until there is a config file.
- **UTF-16 files are reported as binary.** The sniff is a NUL byte in the first
  8 KiB, which is what git does.
- **Two Claude features are unreachable inside abeam**, and both will be reported
  as abeam bugs: `Ctrl+Shift+B` and `Ctrl+Shift+C` are indistinguishable from
  `Ctrl+B` / `Ctrl+C` in legacy terminal encoding, and hold-to-talk voice needs
  key *release* events, which abeam drops for a load-bearing reason
  (`docs/conpty-findings.md`, constraint 3).
- **`EnableMouseCapture` disables your terminal's native text selection.**
  Copying out of abeam needs Shift+drag, and which terminals honour that varies.
- **A routed script agent sees an abeam variable in its environment.** The
  command line `cmd.exe` is asked to run travels in `%ABEAM_LAUNCH%` rather
  than on the wire, for the quoting reason above, and `cmd` is handed it as an
  environment variable — so an npm-installed agent, and every process it goes
  on to spawn, can read a variable abeam set, containing the full path abeam
  resolved and the arguments it was given. Nothing is known to care, and it is
  occasionally the fastest way to see what abeam actually ran; it is listed
  here because a program's environment is not abeam's to write in silently, and
  because an agent that reports its own environment somewhere would report this
  too. Only a routed `.cmd` or `.bat` is affected — a native `claude.exe` is
  started directly and gets no such variable.
- **The `Alt` keys abeam claims are free** — verified against one audited Claude
  build (2026-07-25), and merely not refuted by Copilot's published tables,
  which is one claim resting on two quite different strengths of evidence. The
  `Alt` *namespace* is not free on Copilot's side, and saying so is the whole
  finding this change rests on: those tables declare `Alt+←`/`Alt+→` as
  word-motion and `Alt+Enter` as newline, and the changelog adds an undeclared
  `Alt+D`. abeam binds none of the three, so what is true is the narrower thing
  — free where abeam is standing, not free. Neither audit is a promise about
  the next release of either agent, and gaining an agent can retire a key that
  was safe while there was only one, as `Alt+←`/`Alt+→` has already
  demonstrated. `Ctrl+\` is the mitigation.

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
