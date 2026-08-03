# abeam

One window for an AI coding session.

Your agent runs in the left pane — hosted in a pty, parsed, and drawn by abeam,
not passed through to your terminal. The right pane shows the state of the git
worktree, the document the agent just wrote, or a shell to run things in. A file
watcher drives the first two, so neither has to be asked.

It replaces a three-window setup: the agent in one terminal, git in another, and
an editor open purely to read the markdown it produced.

Two agents are known by name today — **Claude Code** and **GitHub Copilot CLI**
— and any other program on `PATH` can be hosted as `abeam +<name>`. Everything
else you type goes to the agent, so `abeam <claude args>` is `claude <claude
args>` with two panes beside it; "Running it" is the whole rule. Which one you got
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
why an npm-installed agent started on Windows by way of `cmd.exe` says its own
name rather than the interpreter's. `abeam +copilot` should therefore draw
`┌ copilot ─…` in the same place — "should" because that follows from the code
and from a test of the naming, and nobody has watched it happen. The `+` is how
abeam was told which agent to host and never part of what it was told, so it is
not on the border either.

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
is a compiled binary, so `uvx` fetches a few hundred kilobytes and runs it — no
Rust toolchain, no build step, and nothing to compile on your machine. Two
wheels are published, `win_amd64` and x86-64 `manylinux`; the section after next
says what that list leaves out, and what it does not prove about the half of it
that is new.

## Requirements

- **Windows or Linux, x86-64**, which is not the same sentence as "tested on
  both" — see **Platforms**.
- `git` on `PATH`, for the git pane. Its absence is reported in the pane, not
  fatal.
- **An agent to host**, which abeam does not install for you — see below for
  what each one wants.
- Rust 1.95+ — **only to build from source**. On Windows that means the MSVC
  toolchain and plain `cargo build`, with no vcvars wrapper; on Linux it means
  whatever your distribution calls a C toolchain, because that is what supplies
  the linker. Neither build looks for a system library.

## Platforms

**Windows x86-64 and Linux x86-64 are what ships, and the two are not equally
proven.**

abeam was Windows-only by construction until this version — `uvx abeam` on Linux
found no wheel because there was none to find — and the fix was a real port
rather than a build flag. `crates/abeam/src/launch/` is one shared question —
where a program may be found — in front of two answers to what may then be
started; `crates/abeam-pty/src/tree/` is one type whose `Drop` is a kill, with a
job object behind it on Windows and a process group on Unix; the shell pane's
candidate list is per-platform; and four places were folding case or reading `\`
as a path separator, which is right on Windows and a correctness bug anywhere
else. CI builds, tests and lints both targets on
every push, which is the only reason the second one is a claim at all: a suite
that names a platform is only ever run by that platform, and a `cfg` that
excludes one is invisible from the other.

**Nobody has driven abeam on Linux by hand.** That is the sentence to read
before installing it there. The six manual pass criteria in
`docs/conpty-findings.md` — renders, one character per keystroke, editing keys,
live resize, paste, `/exit` — have been confirmed on Windows only. They
generalise to a Unix pty without a word changing, which is why re-running them
there is the right acceptance gate rather than a second checklist, and nobody
has run them. Two further things are worse on Linux than on Windows and both are
about killing children rather than starting them; "Not done, and known" says
what they are.

**macOS and both ARM targets do not ship.** The Unix half compiles for macOS and
would probably work, and `aarch64-pc-windows-msvc` and
`aarch64-unknown-linux-gnu` are one more matrix entry each. None of the three
has ever been built or run by anyone on this project, and `uvx` picks a wheel
automatically — so shipping one would mean the first person to find out it does
not work is a user. That is the rule that already kept ARM Windows out, applied
unchanged: `uvx` reporting no matching wheel is one line, true and immediately
actionable. Add each target when someone can run it.

## Running it

**Everything on the command line belongs to the hosted agent, except a single
leading token beginning `+`, which is abeam's.** That is the whole rule, and
what it buys is that `uvx abeam <anything>` is the session `claude <anything>`
would have started, with two panes around it.

```
abeam                     # the default agent, in the current directory
abeam --resume            # ...which is `claude --resume`
abeam -p "fix the tests"  # ...and `claude -p "fix the tests"`
abeam agent               # ...and `claude agent`, subcommands included

abeam +copilot --resume   # GitHub Copilot CLI, with its own --resume
abeam +bash               # anything else on PATH
abeam +help               # abeam's own help; `--help` is the agent's
abeam -- +1 more thing    # `--` fences a leading `+` for the agent
```

A `+` token is read only in the first position and there is at most one: a
prompt may begin with a `+`, and `abeam config set +x` is a real command line.
`+name` is resolved exactly as the old positional was — if it names an agent
abeam knows, `claude` or `copilot`, or a preset out of your config file, matched
without regard to case, abeam looks that entry's executables up on `PATH`;
anything else is a program name and means what `abeam bash` used to mean. Two words behind the sigil are reserved and only
two, `+help` and `+version`. There are no `+h`/`+V` short forms, deliberately: a
short form is one more word that can never be a program name, and `-h`, `-V`,
`--help` and `--version` all go to the agent now, which is the help you wanted.

`ABEAM_AGENT` names what to host when no `+` token did — an agent name, a preset
name, or any program — and this change is what made it worth setting: `ABEAM_AGENT=copilot
abeam --resume` resumes Copilot, where before the variable stopped applying the
moment you had arguments to pass. An empty value counts as unset, because
PowerShell leaves one behind — and so does a profile that exports a variable it
then fails to fill — and `'' was not found on PATH` names nothing anyone can act
on. A `+` token overrides it.

**`abeam claude` and `abeam copilot` are refused rather than reinterpreted**,
with exit code 2 and a message naming both readings and both ways out. They
hosted those agents for the whole of abeam's life before this, they are written
that way in every older copy of this document, and what you would otherwise get
is `claude claude` — the agent's own complaint about an argument it does not
have, on a screen that never mentions abeam. That refusal is permanent, not a
migration aid, and it is a fixed lookup in abeam's own table rather than a `PATH`
probe: a refusal that depended on what happened to be installed would accept a
command line on your machine and reject it on a build server.

There is no `--agent` flag, and this document used to give a different reason
for that than it gives now. The old one was that the positional already
selected, so a flag beside it would make `abeam --agent copilot powershell`
expressible with no honest meaning. Nothing positional selects any more, so that
objection has gone with its premise — but the property it was defending is
exactly what `+` keeps: there is one place a selection can be written, and it is
impossible to write two.

What the positional cost was the point of abeam. `abeam agent` looked for a
program called `agent`, `abeam mcp list` looked for one called `mcp`, and
`abeam --help` was abeam's help rather than Claude's — every argument that
happened not to start with a dash was a trap, and the traps were whatever
subcommands the hosted agent grows next, which is not a list any README can
hold. The change also deleted a class of bug rather than guarding against it:
abeam used to refuse leading flags it did not recognise, because before that
check existed `abeam --help` reached `CreateProcessW` as a program named
`--help`. Under this rule a dashed token can never be a program name at all, so
there is nothing left to check.

An agent abeam cannot find is a sentence and never a download. Modern `gh
copilot` is a launcher rather than the retired suggest/explain extension: it
will fetch the Copilot CLI if it is missing, and abeam had that fallback for a
day before it was taken out on purpose. Typing `abeam +copilot` is a request to
run something, not consent for a network install, and a terminal border cannot
close that gap after the fact. So what you get instead is four things: the
names that were looked for, what the operating system said about the last of
them, then — when another agent abeam knows is already sitting on `PATH` — the
line that saves the ten minutes, `` `claude` is installed; `abeam +claude` would
host it. ``, and last one sentence on installing the one you asked for, `gh
copilot` included, as a command you run yourself. The third is the best line in
the message and the one most often needed: the default agent missing on a
machine where the other one is right there is a session one word away from
starting, and nothing else on that screen would say so.

What each agent wants:

- **Claude Code** — the native installer, or `npm i -g
  @anthropic-ai/claude-code`. On Windows those write `claude.exe` and a
  `claude.cmd` shim respectively; both are hostable, the shim was not until
  recently, and the story is under "Not done, and known". On Linux both write
  one extensionless file with a `#!` line on it, which abeam starts directly and
  which has no story.
- **GitHub Copilot CLI** — `winget install GitHub.Copilot` on Windows, `npm i -g
  @github/copilot` on Linux, or `gh copilot` once on either to fetch it. The npm
  package wants Node 22 or newer, which is the reason `gh copilot` is worth
  naming: it is the route that works where the other two do not.

The current directory is both the child's working directory and the root that
the git pane and the watcher use.

From a checkout, `cargo run -p abeam` and `cargo run -p abeam -- +copilot` do
the same. Cargo's own `--` and abeam's are two different fences that happen to
be spelled alike: the first ends Cargo's arguments, and everything past it is
the line abeam reads under the rule above.

## Configuration

There is one file, it is optional, and most machines will not have one.

```
Windows  %APPDATA%\abeam\abeam.toml
Linux    $XDG_CONFIG_HOME/abeam/abeam.toml, or ~/.config/abeam/abeam.toml
```

**Your profile, and never the repository**, which is a security decision rather
than a filing one. The repository on screen is the one directory in this whole
program that somebody else gets to write to — it is a clone, it is somebody's
pull request, it is whatever `git checkout` just put there — and `launch` spends
four hundred lines making sure a `claude.exe` committed to it can never be what
starts. A `.abeam.toml` read out of that directory would undo all of that in six
lines of TOML: `[preset.claude]`, `host = "./tools/claude"`, with abeam's own
border obligingly printing the word `claude` over the top. So there is no
repo-local config and there is not going to be one. The usual mitigation is a
trust prompt, and a prompt that appears whenever a repository is fresh is a
prompt that gets answered yes.

No file is not an error; it is the ordinary state. A file that is there and does
not parse *is* one: abeam prints the path and the parser's own line and column
and exits 2, before it touches the terminal. This file names programs to start,
so half-reading it is the one outcome worse than refusing it.

```toml
[defaults]
view  = "git"    # git | files | shell | queue — which right-hand view opens
focus = "left"   # left | right               — which pane has the keyboard
zoom  = false
theme = "dark"   # light | dark               — the reader's page

[preset.fleet]
host  = "claude"     # an agent abeam knows, or any program on PATH
args  = ["agent"]
view  = "queue"
theme = "dark"
```

`[defaults]` is how every session on the machine opens. A **preset** is a name
behind the sigil that behaves exactly like a built-in agent: `abeam +fleet
--resume` starts `claude agent --resume` with the queue showing,
`ABEAM_AGENT=fleet abeam` does the same, and `+help` lists `fleet` beside
`claude` and `copilot`. A preset's own `args` go in *front* of what you typed,
because a subcommand is the first word of the line it belongs to — behind them,
`abeam +fleet --resume` would be `claude --resume agent`, which is a different
command in every agent abeam hosts. Its four opening keys override `[defaults]`
field by field, so the preset above moves the view and leaves the rest where the
defaults put them.

Three rules, each of them a refusal you will see rather than a surprise you
will not:

- **A preset's `host` is looked up in abeam's built-in table and on `PATH`,
  never among the other presets.** There is no preset chaining, deliberately.
  Without the rule, `[preset.claude] host = "claude"` is a row pointing at
  itself; with it, there is no edge from a preset to a preset to recurse along.
  What it costs is naming one preset from another, which is one saved line in a
  config file — bought, otherwise, with a cycle check on the path that decides
  which program starts.
- **A preset may not take a name abeam already answers** — `claude`, `copilot`,
  `help` or `version`. It would be a name with two meanings and one of them
  unreachable, with nothing on screen saying which of the two ran. Two presets
  whose names differ only in case are refused for the same reason, since every
  name behind a `+` is matched without regard to case.
- **A preset name is refused in front of the sigil too.** `abeam fleet` gets the
  same both-readings message `abeam claude` gets, because it is the same
  mistake — made, this time, by the one person on the machine most likely to
  believe the word means their preset.

A key abeam does not recognise is an error rather than a shrug. `[presets.fleet]`
with the plural spelling, or a `them = "dark"`, is a line you wrote and expected
to work, and a config file that quietly ignored it would behave exactly like a
config file abeam never found. The cost is forward compatibility — a file
written for a later abeam is refused by an earlier one rather than partly
honoured — and that is the right way round for a file that names programs to
start.

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
is a real pty — on Windows `pwsh`, falling back to `powershell` then `cmd`; on
Linux `$SHELL` when it is set to anything, then `bash`, then `sh`; or whatever
`ABEAM_SHELL` names, which replaces the list rather than heading it, because a
program you typed is a choice and falling back from it would hide the typo —
started the first time the view is drawn and never before, so a session that
never asks for one never pays for it. `Alt+J`/`Alt+K` scroll its history without
focusing it, which is why they are not the arrow keys the shell would read as
history. A child that exits leaves its last screen up with
`Enter` to start another, and while it is dead `Esc` means what it means
everywhere else.

Nor will abeam close out from under it: the agent exiting holds the door rather
than killing whatever is running, and says `shell open · Alt+Q to quit` in the
left title. That is "open", not "busy" — ConPTY cannot be asked whether a
command is running at all, and on Unix the question has an answer abeam does not
go and get (`tcgetpgrp` on the master says which process group holds the
terminal in the foreground) — so on both a shell sitting at a prompt holds the
door exactly as a build does. Type `exit` in it, or `Alt+Q` twice.

**pty diagnostics** (`F2`) — what the emulation layer is doing: alt-screen,
application cursor, bracketed paste, mouse mode, byte counts, resize count, and
the pty and parser sizes side by side, to be compared with the pane you can see
rather than with each other. **DSR answered** is the one that matters on
Windows: ConPTY blocks until its opening cursor query is answered, so a red zero
there means the session is hung rather than slow. Nothing opens a Unix pty with
a cursor query, so there the counter reads zero for the whole of a healthy
session and the pane neither reddens it nor says anything beside it — a
permanent alarm would cost this view the only thing it has, which is that a red
row means something. `docs/conpty-findings.md` explains each field and why it is
on screen.

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
pty resize.

## Repository

```
crates/abeam-pty/    the pty host layer: sessions, input encoding, DSR
crates/abeam-pty/src/tree/         killing a child's children, per platform
crates/abeam/        the binary: shell, layout, focus, panes
crates/abeam/src/agent.rs          the agents abeam knows, and how one is chosen
crates/abeam/src/config.rs         the one file abeam reads: presets, and how a
                                   session opens. The profile, never the repo.
crates/abeam/src/launch/           where a program may be found, and what may
                                   then be started: one shared half, two others
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

`release.yml` builds two wheels — `win_amd64` on a Windows runner, and the
x86-64 `manylinux` one inside a manylinux container on a Linux runner, because
the container is what fixes the wheel's glibc floor at something old rather than
at whatever the runner image happened to ship this month — and uploads both to
PyPI through **trusted publishing** — an OIDC exchange between GitHub and PyPI,
so there is no API token in this repository to leak or rotate. PyPI is
configured to trust one repository, one workflow file and one environment
(`pypi`); renaming any of the three breaks the publish, deliberately.

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
Agent selection and the launcher underneath it. The Unix port, in the sense that
the whole workspace builds, tests and lints clean for both
`x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` — see "Platforms" for
the sense in which it is not done. 379 tests on Windows, and
`clippy --all-targets` clean on both.

Two of those changed Windows behaviour on the way past, and both are worth
seeing before you upgrade rather than after.

**An earlier `PATH` entry holding something abeam cannot start no longer hides
the program behind it.** A `claude.ps1` with no `.cmd` beside it, or the
extensionless npm shim on its own, used to end the search where it sat and
produce the error about *that* file — with a perfectly good `claude.exe` one
`PATH` entry further along. The walk now takes two passes, the first for a file
this platform can actually start and the second only to name something in the
error if nothing can be, and it is the same code on both platforms because the
question is the same one. It arrived as a Unix change: there a directory holds
one candidate, so the preference `PATHEXT` expresses inside a directory on
Windows had to be lifted to the walk. It has a test of its own on this platform,
`a_file_windows_cannot_start_does_not_shadow_the_program_further_along_path`,
because without one the whole two-pass could be deleted and every Windows test
would still pass.

**`launch/unix.rs` asks the kernel whether *this process* may execute the file**
— `access(X_OK)` — rather than reading the mode bits, which answer "may
anybody". A `--x------` owned by somebody else passed the old check and then
failed at the spawn with a raw `EACCES`; it is now refused by abeam, with
abeam's own sentence.

**Verified how.** The whole path — pty → parser → pane, git worker → real `git`
→ rendered rows, watcher → both panes, Enter in git → open in files — is
covered by tests that spawn a real pty child and draw real frames, on both
platforms.

On top of that, `crates/abeam/tests/end_to_end.rs` does to abeam what abeam does
to an agent: it spawns **the built binary** in a pty, types at it as bytes,
and reads the screen that comes back. That is what proves the parts no in-process
test can reach — that abeam starts at all, that raw mode and the alternate
screen survive being someone else's child, that `Alt+S` written as `ESC s`
becomes the binding it should, and that a command typed into the shell view runs
in the right directory and puts its answer on screen. Three paths are pinned that
way today: type a command in the shell and read its output; reach a file nothing
pointed the pane at, by `Alt+E` `Alt+E` `/`; and a copy of a real shell planted
in the repository under the name abeam is about to look for, which abeam must
refuse to run. That last one is a test about an attack rather than a feature, and
the two platforms arrive at it down different roads, which is worth stating twice
rather than generalising once. Windows resolves a bare program name against the
*calling* process's directory before it consults `PATH`, so a `pwsh.exe`
committed to a cloned repo was one keystroke from executing and nothing the user
did was wrong. `execvp` has no rule of that kind, and hands the same hole back
through `PATH` instead: a `.`, a `..`, any relative entry, and the empty string
that a leading, trailing or doubled `:` leaves behind all name the directory the
process is standing in, which under abeam is the repository on screen — the one
directory in the whole question somebody else gets to write to. `PATH=:$PATH` is
one typo, and an empty entry is what a shell profile appending to an unset `PATH`
produces on its own, so it is not an exotic road. On both, the answer is the same
one thing: the `PATH` walk drops every entry that is not absolute, and nothing
leaves `launch` that is not an absolute path.

**The second line of defence is not symmetric, and this document used to imply
it was.** abeam stands in `%SystemRoot%` on Windows and in `/` on Unix for the
whole session, and only the first of those buys anything against this hazard. On
Windows it is real: `CreateProcessW` consults the calling process's directory
before `PATH`, so where abeam is standing is part of the answer, and standing in
`%SystemRoot%` is what makes it a harmless one. On Unix it buys nothing, because
abeam never reaches `execvp`'s own `PATH` walk holding this process's directory —
every spawn goes through `portable_pty::CommandBuilder::search_path`, which
resolves a bare name against `PtyConfig.cwd`, and every pty abeam opens is given
the repository on screen as its `cwd`. So a future call site that built a
`PtyConfig` with a bare name would resolve it against the repository however far
this process had walked from it, and `launch::resolve` returning absolute paths
is the whole of what stands behind that. The residual value is smaller again than
`/` suggests: `uvx abeam` inside a container commonly runs as root, where `/` is
writable like anywhere else. The chdir stays on both because it costs nothing and
still covers anything that does consult the process's own directory — but a Linux
reader should not read it as a spare defence they have.

What none of it covers is a human at a terminal. The six pass criteria in
`docs/conpty-findings.md` (rendering, one character per keystroke, editing keys,
live resize, paste, `/exit`) were confirmed against the spike's host and have
not been re-run against `abeam` itself since the panes landed — and have never
been run on Linux at all, against anything. Do that before trusting it with real
work, on either platform.

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
  not be.** All of what follows is about Windows, and a Linux reader can stop at
  the end of this sentence: the same install there is one file that already
  runs, and the paragraph after this one says why that is the whole of it. `npm
  i -g` puts three files in `%APPDATA%\npm`: `claude` (a POSIX shell script, no
  extension), `claude.cmd` and `claude.ps1`. Windows starts
  `.exe` and `.com` and nothing else, so plain `abeam` did fail at startup for
  those users. The entry that said so also called `cmd.exe /c` one of "two
  obvious fixes [that] are both wrong", on the grounds that it hands abeam's own
  argv to a command-line re-parser treating `&`, `|` and `^` as syntax. That
  half was itself wrong, and deleting it quietly would be the worse correction.
  `crates/abeam/src/launch/windows.rs` routes a `.cmd` or a `.bat` by naming
  `cmd.exe` in front of it, and going through the re-parser is safe *because*
  of the argument quoting rather than despite it: the command line is built the
  way Rust's own `make_bat_command_line` builds one — the fix for CVE-2024-24576,
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

  **On Linux the whole of that inverts, and `launch/unix.rs` is three
  functions.** The same `npm i -g` writes one file, extensionless, with
  `#!/usr/bin/env node` on its first line and the execute bit set — the very
  shape Windows cannot start at all — and the *kernel* reads that line: `execve`
  finds the interpreter, rewrites the argument vector and starts it before any
  of abeam's code would have had a chance to. So there is nothing to route, and
  because there is nothing to route there is no second parser between abeam and
  the child: nothing to quote against, no character that has to be refused
  because it cannot be escaped, and no command-line length belonging to a shell
  that is not running. What the platform does ask is one question — whether the
  file may be executed at all — and a file that may not gets its own sentence
  naming `chmod +x`, rather than the search's "was not found on PATH", which
  would be a lie about a file the user can see with `ls`. Putting `sh -c` in
  front of things is the obvious improvement to a module that short, and it is
  refused for the reason the paragraph above exists: it would invent on Linux,
  deliberately, the entire problem `windows.rs` spends four hundred lines
  solving, in exchange for nothing.
- **Nobody has driven abeam on Linux by hand.** CI compiles it, lints it and
  runs the suite there on every push; the six pass criteria above have been
  confirmed on Windows only. "Platforms" says this where somebody deciding
  whether to install it will read it, which is the more important of the two
  places.
- **On Linux, killing abeam itself leaves the process tree behind.** A Windows
  job object dies with its last handle and the handle goes when the process
  does, so even `taskkill /f` takes the hosted shell's `cargo build` with it. A
  process group is nobody's handle: `SIGKILL` to abeam runs no destructor, and
  what is left is the kernel's own `SIGHUP` to the foreground process group when
  the pty master closes. That reaches a shell sitting at a prompt. It does not
  reach a build that ignores it. Quit with `Alt+Q`, which is the path that runs
  the destructor.
- **A second, narrower hole in the same place.** `killpg` signals the group the
  child leads, and job control gives every job an interactive shell starts a
  group of its own — so a `cargo build &` backgrounded under `bash` in the shell
  view is outside it even when abeam exits normally. What covers the foreground
  case is the `SIGHUP` that goes first, which a job-control shell answers by
  hanging up its own jobs. `sh -c`, direct spawns and non-interactive shells are
  fully covered. Windows has no equivalent gap: everything the child starts
  joins the job with it.
- **On Unix, abeam signals a bare pid in two places, and a pid comes round
  again.** `tree::unix`'s `killpg` and `PtySession::drop`'s `child.kill()` both
  take the number portable-pty handed back at spawn, and abeam's own `try_wait`
  — called every frame — has usually already reaped it, which is what releases
  the number to be handed out again. Windows has neither problem: a job object
  and a process handle each name one process for as long as they are open, and
  cannot come to mean somebody else's. What makes this reachable rather than
  academic is that abeam deliberately outlives the hosted agent for as long as a
  shell pane is open — so the window is however long the user leaves that pane
  up, and it is the pane people run builds in, spawning thousands of pids
  against a default `pid_max` of 32768. To be wrong the group has to have emptied
  *and* the pid space to have wrapped inside that window. Nobody has hit it, a
  distribution that has raised `pid_max` into the millions puts it back out of
  reach, and nothing in this repository can close it: the number is the only
  handle Unix hands back. The fix is a `pidfd` taken at spawn time, it is
  Linux-only, and it belongs in portable-pty. `crates/abeam-pty/src/tree/unix.rs`
  is the long version and is worth reading before anyone tries to fix it here.
- **A marked-executable file with no `#!` line runs on glibc and not on musl.**
  This is libc's doing rather than the kernel's — the kernel refuses the file
  with `ENOEXEC`, and glibc's `execvp` then retries it against `/bin/sh`, in
  `__execvpe`'s `maybe_script_execute`. musl declines to implement that retry at
  all, as a long-standing upstream decision. So on a musl system the same file is
  an `Exec format error`, and what the user sees is portable-pty's raw spawn
  failure rather than one of abeam's own sentences. It has no bearing on either
  wheel today, because the Linux wheel is `manylinux` and Alpine would find no
  matching wheel at all; it becomes live the day a `musllinux` wheel is published
  or somebody builds from source there. `launch/unix.rs` records it because that
  is where the next person will look.
- **A dispatched background task does not survive closing the terminal window on
  Windows.** A task sent off to run beside the session — rather than typed into
  it — is now detached into a session of its own on Linux, with `setsid` in the
  child between `fork` and `exec`, so a `kill %1`, a shell hangup or a dropped
  ssh leaves it running. Windows has no counterpart yet: the child inherits
  abeam's console, and closing the window delivers `CTRL_CLOSE_EVENT` to
  everything attached to that console and kills it after the usual grace period.
  Raw mode does not suppress that — it clears `ENABLE_PROCESSED_INPUT`, which
  suppresses `CTRL_C_EVENT` and nothing else. The fix is a `DETACHED_PROCESS`
  creation flag, giving the child no console at all, and specifically *not*
  `CREATE_NEW_PROCESS_GROUP`, because a console control event reaches every
  process attached to the console whatever group it is in. That is a feature with
  its own questions rather than a line to add, so for now the two platforms
  differ and this says which way.
- **`Ctrl+Enter` is unreachable on most Unix terminals.** Telling it from a bare
  `Enter` needs the Kitty keyboard protocol, and abeam asks for raw mode, the
  alternate screen, mouse capture and bracketed paste and nothing else. The one
  place abeam distinguishes the two is the composer in the queue view, where
  `Ctrl+Enter` puts a newline in the item being written instead of committing
  it; on Linux that binding is mostly dead and a multi-line item has to be
  pasted. `Alt+Enter` does the same job and probably does arrive there — which
  is exactly the next bullet's point.
- **The `Alt` table is unverified on Linux.** Every key in it was checked against
  what a Windows console delivers, and `crates/abeam/examples/keyprobe.rs` exists
  to answer precisely that question and has been run against Windows consoles
  only. Whether an `Alt` combination arrives as a modifier, as an `Esc` prefix,
  or not at all because the desktop took it first is a question about that
  terminal and that desktop, and no amount of reading an agent's source answers
  it. Run the probe in the terminal you launch abeam from before assuming the
  table holds; `docs/keymap.md` has the procedure.
- **Keybindings are not configurable**, and neither are the split ratio or
  either drawing interval: those are constants in the source. There is a config
  file now — see "Configuration" — and it holds presets and the four things a
  session opens with, which is where the reader's light/dark choice went. It
  says nothing about keys. Claude's own bindings are user-configurable and
  abeam's should be too before anyone else uses it. Copilot's are not, which
  makes that the mirror image of the same gap rather than a reason to be relaxed
  about it. The two environment variables are unchanged beside it: `ABEAM_AGENT`
  names the agent, preset or program to host when no `+` token did, and
  `ABEAM_SHELL` names what the command view starts.
- **A scrolling pane is a full repaint.** ratatui diffs by cell and has no notion
  of a scroll region, so when the agent's output scrolls, every row has changed
  and the whole pane is rewritten — about 10 KB of escape sequences, measured
  at a 118×45 pane on Windows Terminal, which keeps up with it comfortably.
  There is no Linux counterpart to that second half and this bullet does not
  claim one: the byte count is the renderer's and does not change, but what a
  terminal does with 10 KB many times a second is the terminal's business and no
  Linux terminal has been measured. Either way it is the structural ceiling on
  how cheap a frame can get, and the thing to attack next if the F2 worst-frame
  figure ever says the renderer is the limit.
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
  question by owning every colour inside its own rect. The choice is still per
  session — `F3` flips it and the flip is not written back — but where it starts
  is now `[defaults] theme` in the config file rather than always dark.
- **UTF-16 files are reported as binary.** The sniff is a NUL byte in the first
  8 KiB, which is what git does.
- **A path that is not UTF-8 breaks `launch`'s chain of custody.**
  `PtyConfig::program` is a `String`, so a `PathBuf` that `launch` probed,
  checked and found absolute is converted lossily on the way into the pty — and
  a `PATH` entry that is not valid UTF-8 can therefore hand the spawn a path
  that was never the one probed. What it produces in practice is a confusing
  "does not exist" rather than anything worse, because a lossy rendering names a
  *different* file rather than a hostile one. The fix is an `OsString` on that
  field, and it is worth doing precisely because the module's whole value is
  that nothing leaves it unchecked.
- **A non-UTF-8 command-line argument aborts abeam before anything runs.**
  `std::env::args()` panics on one. That is unreachable on Windows and entirely
  reachable on Unix, where an argument is bytes; `args_os` is the fix.
- **The reader's title and the find list spell the same path differently on
  Windows.** The file index rewrites `\` to `/` so that typing `src/panes` finds
  something; the reader's own label does not, so a title can read
  `src\panes\git.rs` where the list beside it read `src/panes/git.rs`. It is
  cosmetic, it predates the port, and it is Windows-only — on Unix both are the
  walk's own `/`, and rewriting there would be a bug rather than a tidy-up.
- **A Claude session record is matched by a case-insensitive `.json`.** Right on
  Windows, wrong on Unix, where `46256.JSON` is a different file that would be
  read as a record anyway. Nothing writes that spelling, so it is listed for
  completeness rather than because anyone has met it.
- **Two Claude features are unreachable inside abeam**, and both will be reported
  as abeam bugs: `Ctrl+Shift+B` and `Ctrl+Shift+C` are indistinguishable from
  `Ctrl+B` / `Ctrl+C` in legacy terminal encoding, and hold-to-talk voice needs
  key *release* events, which abeam drops for a load-bearing reason
  (`docs/conpty-findings.md`, constraint 3).
- **`EnableMouseCapture` disables your terminal's native text selection.**
  Copying out of abeam needs Shift+drag, and which terminals honour that varies.
- **A routed script agent sees an abeam variable in its environment, on
  Windows.** The command line `cmd.exe` is asked to run travels in
  `%ABEAM_LAUNCH%` rather than on the wire, for the quoting reason above, and
  `cmd` is handed it as an environment variable — so an npm-installed agent, and
  every process it goes on to spawn, can read a variable abeam set, containing
  the full path abeam resolved and the arguments it was given. Nothing is known
  to care, and it is occasionally the fastest way to see what abeam actually
  ran; it is listed here because a program's environment is not abeam's to write
  in silently, and because an agent that reports its own environment somewhere
  would report this too. Only a routed `.cmd` or `.bat` is affected — a native
  `claude.exe` is started directly and gets no such variable, and on Linux
  nothing is ever routed, so nothing is ever set.
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
`Screen::contents()` tells you nothing about layout. Three of those five are
facts about Windows rather than about ptys, which matters now that there is a
Unix build; the document marks which, and none of the three stops being a rule
the code has to keep.

There are tests pinning all five. Read the document first.

## License

MIT. See [LICENSE](LICENSE).
