# abeam

One window for an AI coding session.

Your agent runs in the left pane — hosted in a pty, parsed and drawn by abeam,
not passed through to your terminal. The right pane shows the state of the git
worktree, the document the agent just wrote, a shell to run things in, work
lined up for the agent, or a second copy of your agent you can ask about the
file in front of you. A file watcher drives the first two, so neither has to be
asked.

It replaces a three-window setup: the agent in one terminal, git in another, and
an editor open purely to read the markdown it produced. The difference from
assembling that yourself out of a multiplexer and a git TUI is that **the right
pane knows what the agent just did** — one watcher on the repository root turns
the markdown it writes into the document on screen and refreshes git within a
debounce interval of any file it touches. The panes are read-only, they never
take focus from the agent, and they never switch themselves.

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

**Before you rely on it, read [what is done and what is
not](docs/status.md)** — or the short version under [Status](#status) at the
foot of this page. abeam is used daily on Windows with Claude Code; several
other combinations ship without anyone having watched them work, and that
document says which.

## Install

```
uvx abeam                 # run it without installing anything
uv tool install abeam     # or keep it on PATH
```

There is no Python in abeam. PyPI is the delivery van: the wheel's whole payload
is a compiled binary, so `uvx` fetches a few hundred kilobytes and runs it — no
Rust toolchain, no build step, nothing to compile.

You also need:

- **Windows or Linux, x86-64.** Those are the two published wheels. macOS and
  the ARM targets do not ship — see [status](docs/status.md#platforms).
- **`git` on `PATH`**, for the git pane. Its absence is reported in the pane
  rather than being fatal.
- **An agent to host.** abeam does not install one for you:
  - **Claude Code** — the native installer, or `npm i -g
    @anthropic-ai/claude-code`.
  - **GitHub Copilot CLI** — `winget install GitHub.Copilot` on Windows, `npm i
    -g @github/copilot` on Linux, or `gh copilot` once on either to fetch it.
    The npm package wants Node 22 or newer, which is why `gh copilot` is worth
    knowing: it is the route that works where the other two do not.

## Run it

Start it in the directory you want to work in:

```
abeam
```

**Everything on the command line belongs to the agent, except a single leading
token beginning `+`, which is abeam's.** That is the whole rule, and what it
buys is that `abeam <anything>` is the session `claude <anything>` would have
started, with two panes around it.

```
abeam                     # the default agent, in the current directory
abeam --resume            # ...which is `claude --resume`
abeam -p "fix the tests"  # ...and `claude -p "fix the tests"`
abeam agent               # ...and `claude agent`, subcommands included

abeam +copilot --resume   # GitHub Copilot CLI, with its own --resume
abeam +bash               # anything else on PATH
abeam +help               # abeam's own help; `--help` is the agent's
abeam -- +1 more thing    # `--` stops abeam reading, and goes to the agent too
```

A `+` token is read only in first position, and there is at most one — so a
prompt may begin with a `+`, and `abeam config set +x` is a real command line.
Behind the sigil you can write an agent abeam knows (`claude`, `copilot`), a
preset from your config file, or any program on `PATH`. Case does not matter.
Two names are reserved: `+help` and `+version`.

`abeam claude` and `abeam copilot` — without the sigil — are **refused** with a
message naming both readings, rather than being reinterpreted. They used to host
those agents, and what you would otherwise get now is `claude claude`.

`ABEAM_AGENT` names what to host when no `+` token did, and a `+` overrides it
for one run. It holds a **name**, not a command line, so `ABEAM_AGENT=copilot`
rather than `ABEAM_AGENT=+copilot`. Be aware that it applies to every command
line that does not lead with a `+`, so one exported in a dotfile years ago will
quietly redirect `abeam -p "commit my changes"` as well as bare `abeam`. The
left border always says which agent is taking your typing.

The directory you start in is the agent's working directory and the root that
the git pane, the watcher and the shell use. The right pane can later be pointed
at another worktree of the same repository; the left one cannot, ever, because a
running process cannot be moved.

## The panes

The left pane is your agent. The right pane is one of six views, and switching
between them or scrolling them costs you nothing — you only need to move focus
to drive a selection or to type.

**git** (`Alt+G`) — read-only. Branch, ahead/behind, staged / unstaged /
untracked files with per-file line counts, and recent commits. Every `git` call
is a read: it stages nothing and commits nothing. It refreshes when the watcher
sees a write, and on a two-second timer for changes the watcher cannot see, such
as a commit made in another terminal. `Enter` opens the selected file in the
reader. `w` lists the repository's other worktrees, which is how the right pane
is pointed at one.

**files** (`Alt+E`) — read-only markdown and source. Markdown is rendered rather
than shown as source: headings, lists, tables, quotes, GFM alerts, footnotes,
syntax-highlighted code fences, and `graph`/`flowchart` and `sequenceDiagram`
mermaid blocks drawn in box-drawing characters. `t` swaps the rendering for the
source it came from, and back. On startup it opens the newest markdown under the
root; after that it follows what the agent writes. A document arriving while you
are looking at something else waits, and the border says `◆ Alt+E` rather than
switching under you.

A second `Alt+E` opens the **file list**, a gitignore-aware browser starting
where the open file lives. `Enter` descends or opens, `Backspace` climbs.

Three keys search, and they ask three different questions:

| Key | Where | Question |
| --- | --- | --- |
| `/` | file list | which file is called this |
| `/` | document | where is it on this page (`n` / `N` walk the matches) |
| `f` | either | which files say this — reads every file under the root |

Only `f` touches the disk, and it is the only one whose box waits for `Enter`
rather than narrowing as you type. `Enter` on a result opens that file with the
document search already looking for the same phrase.

**shell** (`Alt+S`) — a real shell in the directory abeam was pointed at, next to
the session that is about to be told what it printed. `pwsh` on Windows, falling
back to `powershell` then `cmd`; `$SHELL` then `bash` then `sh` on Linux; or
whatever `ABEAM_SHELL` names. It starts the first time you open the view and
never before. This is the one view that keeps `Esc` and `q` — they belong to the
shell — so `Alt+S` or `F4` is the way out. abeam will not quit out from under a
running command: the agent exiting holds the door and the left title says
`shell open · Alt+Q to quit`.

**queue** (`Alt+A`) — work lined up for the agent, for the gap between having a
thought and being able to act on it. Items go one of two ways: **send**, typed
into the left pane's session the moment it goes idle, continuing the
conversation; or **dispatch**, started as its own background agent with none of
that context, running beside you. `i` writes an item, `m` switches it between
the two, `a` arms unattended sending, `Enter` does the selected one now, `d`
deletes. A send waits for the agent's own record to say it is idle and for
nothing to be sitting unsubmitted in its composer, and announces itself in the
left title first — typing at the agent during that pause defers it.

**ask** (`?` from the document or the git view, `F6` from anywhere) — a second
copy of your agent, in the right pane, which **may read and may not write**. For
the question that is about what is on screen — what does this call do, where is
this written, is this the only caller — without spending the conversation on the
left. `?` attaches the file you were looking at and shows you that it has; `F6`
attaches nothing, which is the question you have while typing at the agent, and
is also the only way to take an attachment back off.

What travels is a **path, never a payload**: the child stands in the same
directory and goes and reads what it needs, so nothing of your repository is
shipped off the machine beyond the file name shown above the composer. Answers
stream, and the row along the bottom counts the seconds so a long turn cannot be
mistaken for a dead pane. `Ctrl+L` ends the conversation and the child with it,
which is worth doing when you move on — every turn is re-sent as context with
the next question.

`Enter` never runs anything. `Tab` picks a single-line command out of an answer
and `Enter` on an empty composer types it at the shell **without submitting it**.
A block of more than one line is never offered, and neither is one carrying a
control character.

Under Claude the child gets `--tools "Read,Grep,Glob"` — an allowlist, so no
other tool exists for that session — and the pane draws the list the child
reports back, so what is on screen is what it actually got. Under Copilot the
guarantee is weaker and made the other way round, with `--deny-tool`; that half
has never been run by anyone, the pane says so on its opening screen, and
[status](docs/status.md) has the detail.

**pty diagnostics** (`F2`) — what the emulation layer is doing: alt-screen,
application cursor, bracketed paste, mouse mode, byte counts, sizes, and the
frame clock. **DSR answered** is the one that matters on Windows — a red zero
means the session is hung rather than slow. `docs/conpty-findings.md` explains
each field.

## Keys

Everything abeam binds lives under `Alt` and the F-keys, with one exception:
`Ctrl+\`, the escape hatch. **Nothing abeam claims is a key the hosted agent can
act on** — `docs/keymap.md` is the audit behind that, and there are tests
pinning it. Everything else you type goes to the agent untouched.

| Key | |
| --- | --- |
| `Alt+G` | right pane → git |
| `Alt+E` | right pane → files (again for the file list) |
| `Alt+S` | right pane → a shell, **and focus it** (again to hand focus back) |
| `Alt+A` | right pane → the queue |
| `F6` | right pane → ask, nothing attached, **and focus it** (again for what it displaced) |
| `F2` | right pane → pty diagnostics, and back to what it displaced |
| `F3` | file reader → light / dark page |
| `F7` | select rows of the right pane by keyboard (a drag copies on its own) |
| `F4` / `F5` | move focus left / right |
| `Alt+J` / `Alt+K` | scroll the right pane a line — **without focusing it** |
| `Alt+PgDn` / `Alt+PgUp` | scroll the right pane a page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (twice while a child is live) |
| `F1` | key help overlay |
| `Ctrl+\` or `F12` | send the *next* key to the agent verbatim |

Once the right pane has focus, plain keys work — deliberately the same
vocabulary as Claude's own transcript view:

```
j / k, arrows   a line        g / G, Home / End   the ends
space / b       a page        Tab / Shift+Tab     the selection
Ctrl+D / Ctrl+U a half page   Enter               open · queue: do it now
r  refresh      t  rendered markdown / source     Esc or q   back to the agent
```

**Two views do not answer to that paragraph.** The **shell** takes every plain
key, because a pane you type into cannot also read what you typed. The **ask**
is the same: its composer is live the whole time the pane is, so `j`, `k`, `g`,
`G`, `space`, `b`, `r` and `q` are letters there, and what scrolls is the
arrows, PgUp/PgDn, Home/End and `Ctrl+D`/`Ctrl+U`. The F1 overlay says so in a
row of its own.

`Ctrl+\` exists so abeam can never permanently shadow a binding of the agent you
are typing at. If a future release of either agent binds `Alt+G`, `Ctrl+\` then
`Alt+G` still reaches it.

### Copying out of the right pane

abeam turns mouse capture on, which takes your terminal's own drag-select away —
and a linear drag across a split window would hand you both panes and the border
between them anyway. So copying is abeam's job.

**Drag over what you want and let go. That copies it.** No key, no mode: on a
command line, highlighting something is what wanting to take it looks like, and
the border says `copied 3 rows · ⏎ agent` so you know it went. Press `Enter`
while the highlight is still up and **the rows go straight into the agent's
composer, unsent** — which is the round trip the feature exists for: run
something in the shell view, drag over what it printed, hand it over, and add a
sentence of your own before you press Enter yourself.

`Ctrl+C` copies too, whenever a highlight is up. It is the only `Ctrl`+letter
abeam ever takes, and only in that state — with nothing selected it is the
child's, as always. If you meant to interrupt something, `Esc` first.

`F7` is the same thing without a mouse: it puts a caret on the right pane, the
scroll keys move it, `v` anchors a selection, and `y` or `Ctrl+C` copies. That is
also the way in when the right pane is running something that wants the mouse
itself. While a selection is up nothing reaches the pane behind it, including a
live shell.

Two things worth knowing. The selection is whole **rows as they are on screen**,
not a range in the content: scroll under it and the highlight stays where it is,
naming whatever is there now — so what is highlighted is always exactly what
will be copied. And the clipboard is reached with OSC 52, which your terminal has
to honour: Windows Terminal, VS Code, iTerm2, kitty, WezTerm and Alacritty do, a
legacy Windows console does not, and tmux wants `set -g set-clipboard on`.
`Enter` needs none of that — it never goes near a clipboard.

## Worktrees

Claude Code makes git worktrees inside the directory abeam is watching — usually
`<root>/.claude/worktrees/<name>` — and runs agents in them, so one watched root
can hold several working trees. abeam routes every watched file to the workspace
that owns it, which is the innermost one containing it, exactly as `git status`
does. A neighbouring agent's writes do not refresh your git pane or pull its
scratch markdown into your reader.

`w` in the git view lists every worktree git knows about, with `agent` against
the one your agent is running in and `▸` where the right pane is standing.
`Enter` moves the right pane there and puts the status list back. The left pane
never moves, and the border names the right pane's workspace whenever it is not
the agent's own.

[Design notes](docs/design.md) has the argument for the routing rule, including
why the obvious version of it does not work.

## Configuration

There is one file, it is optional, and most machines will not have one.

```
Windows  %APPDATA%\abeam\abeam.toml
Linux    $XDG_CONFIG_HOME/abeam/abeam.toml, or ~/.config/abeam/abeam.toml
```

```toml
[defaults]
view  = "git"    # git | files | shell | queue | ask — which right-hand view opens
focus = "left"   # left | right                     — which pane has the keyboard
zoom  = false
theme = "dark"   # light | dark                     — the reader's page

[preset.fleet]
host  = "claude"     # an agent abeam knows, or any program on PATH
args  = ["agent"]
view  = "queue"
theme = "dark"
```

`[defaults]` is how every session on the machine opens. A **preset** is a name
behind the sigil that behaves exactly like a built-in agent: `abeam +fleet
--resume` starts `claude agent --resume` with the queue showing, and `+help`
lists `fleet` beside `claude` and `copilot`. A preset's own `args` go in *front*
of what you typed, because a subcommand is the first word of its line.

Three things are refused rather than quietly worked around: a preset whose
`host` names another preset (there is no chaining, so there is no cycle to
check), a preset taking a name abeam already answers, and a key abeam does not
recognise — a `[presets.fleet]` with the plural spelling is a line you wrote and
expected to work, so it is an error rather than a shrug.

**It is read from your profile and never from the repository**, which is a
security decision rather than a filing one. The repository on screen is the one
directory in this program somebody else gets to write to, and abeam goes to some
length to make sure a `claude.exe` committed to it can never be what starts. A
repo-local config would undo that in six lines of TOML.

## Status

Working, and used. Not finished. The full account is in
**[docs/status.md](docs/status.md)**; these are the things most worth knowing
before you install it.

- **Windows with Claude Code is the combination that gets used daily.**
  Everything else ships on tests rather than on use.
- **Nobody has driven abeam on Linux by hand.** CI builds, tests and lints both
  targets on every push, and the six manual pass criteria in
  `docs/conpty-findings.md` have been confirmed on Windows only.
- **abeam has never been run with GitHub Copilot CLI.** Not once. The selection,
  the launcher and the failure messages are tested; a session is not.
- **Nobody has typed a question into the ask pane**, and the Copilot half of it
  has never been run by any process at all. What comes back from it is a model's
  answer, which can be fluent, specific and wrong about the file it read.
- **A turn in the ask pane that never ends has no way out** but `Alt+Q`. There is
  no cancel key and no timeout.

## From source

```
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets
```

`--all-targets`, or the examples bit-rot. `cargo run -p abeam` and `cargo run -p
abeam -- +copilot` do what the installed binary does. Building needs Rust 1.95+,
the MSVC toolchain on Windows and a C toolchain on Linux for the linker; neither
build looks for a system library.

To release: bump `version` in `[workspace.package]` in `Cargo.toml` — it lives
there and nowhere else — commit it, then tag and push.

```
git tag v0.3.0 && git push origin v0.3.0
```

`release.yml` refuses to run if the tag disagrees with `Cargo.toml`, builds a
`win_amd64` and an x86-64 `manylinux` wheel, and publishes both to PyPI through
trusted publishing. `workflow_dispatch` builds the wheels without publishing, so
the build can be exercised without spending a version number.

## Where things are

```
crates/abeam-pty/                  the pty host layer: sessions, input, DSR
crates/abeam-pty/src/tree/         killing a child's children, per platform
crates/abeam/                      the binary: shell, layout, focus, panes
crates/abeam/src/agent.rs          the agents abeam knows, and how one is chosen
crates/abeam/src/config.rs         the one file abeam reads
crates/abeam/src/launch/           where a program may be found, and what may
                                   then be started
crates/abeam/src/workspace.rs      which workspace owns a watched path
crates/abeam/src/ask/              the second agent, and a wire format abeam
                                   does not own
crates/abeam/src/panes/            one file per view
crates/abeam/tests/end_to_end.rs   abeam itself, hosted in a pty and typed at
```

Most of the reasoning lives in the module documentation of the file that owns
each decision, rather than in this README.

| Document | |
| --- | --- |
| [docs/status.md](docs/status.md) | what is done, what is not, and what nobody has watched work |
| [docs/keymap.md](docs/keymap.md) | the keyboard audit: every key abeam claims, and why it is safe in both agents |
| [docs/design.md](docs/design.md) | worktree routing, the draw loop, the layout |
| [docs/conpty-findings.md](docs/conpty-findings.md) | **read before touching the pty layer.** Five constraints that look like things to tidy up and are not |

## License

MIT. See [LICENSE](LICENSE).
