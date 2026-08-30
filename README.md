# abeam

One window for an AI coding session.

Your agent runs in the left column — hosted in a pty, parsed and drawn by
abeam, not passed through to your terminal — and you can start more than one
there. The right pane shows the state of the git
worktree, the document the agent just wrote, a shell to run things in, work
lined up for the agent, a pad to write your own notes in, or, where that
provider is supported, a second copy you
can ask about the file in front of you. A file watcher drives the first two, so
neither has to be asked.

The left column holds **more than one agent** when you want it to: start another
in a second worktree and they stack, whole where there are rows for them and a
title row each where there are not. See [More than one
agent](#more-than-one-agent) below — and read `docs/status.md` first, because
nobody has yet driven that part by hand.

It replaces a three-window setup: the agent in one terminal, git in another, and
an editor open purely to read the markdown it produced. The difference from
assembling that yourself out of a multiplexer and a git TUI is that **the right
pane knows what the agent just did** — one watcher on the repository root turns
the markdown it writes into the document on screen and refreshes git within a
debounce interval of any file it touches. The panes are read-only, they never
take focus from the agent, and they never switch themselves.

Text comes back the other way by dragging over it. Run something in the shell
pane, highlight what it printed, and it is on your clipboard when you let go —
or press `Enter` and the rows land in the agent's composer, unsent, ready for
you to say what you want done about them. That round trip is
[a section of its own](#copying-out-of-the-right-pane) below.

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
  - **OpenAI Codex CLI** — `npm i -g @openai/codex`, then run `codex` once and
    sign in with ChatGPT or an API key. See OpenAI's [CLI
    setup](https://learn.chatgpt.com/docs/codex/cli) and [authentication
    guide](https://learn.chatgpt.com/docs/auth).

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
abeam +codex [args]       # OpenAI Codex CLI; every argument is forwarded
abeam +bash               # anything else on PATH
abeam +help               # abeam's own help; `--help` is the agent's
abeam -- +1 more thing    # `--` stops abeam reading, and goes to the agent too
```

A `+` token is read only in first position, and there is at most one — so a
prompt may begin with a `+`, and `abeam config set +x` is a real command line.
Behind the sigil you can write an agent abeam knows (`claude`, `copilot`,
`codex`), a preset from your config file, or any program on `PATH`. Case does
not matter. Two names are reserved: `+help` and `+version`.

`abeam claude`, `abeam copilot` and `abeam codex` — without the sigil — are
**refused** with a message naming both readings, rather than being
reinterpreted. Use the corresponding `+name` form to select an agent.

Codex support is deliberately narrow: `abeam +codex [args]` hosts the ordinary
interactive Codex TUI in the left pty. Ask is unavailable. Claude's readiness
record cannot establish whether Codex is idle, so queue **send** items are
blocked both automatically and when you press `Enter`; type the item in the
left pane instead. Background dispatch is Claude-only and is unavailable too.

`ABEAM_AGENT` names what to host when no `+` token did, and a `+` overrides it
for one run. It holds a **name**, not a command line, so `ABEAM_AGENT=copilot`
rather than `ABEAM_AGENT=+copilot`. Be aware that it applies to every command
line that does not lead with a `+`, so one exported in a dotfile years ago will
quietly redirect `abeam -p "commit my changes"` as well as bare `abeam`. The
left border always says which agent is taking your typing.

The directory you start in is the first agent's working directory and the root
that the git pane, the watcher and the shell use. The right pane can later be
pointed at another worktree of the same repository; an agent pane cannot, ever,
because a running process cannot be moved — starting a second agent somewhere
else is what you do instead.

## The panes

The left column is your agent — or your agents, if you have started more than
one; see [More than one agent](#more-than-one-agent). The right pane is one of
seven views, and switching between them or scrolling them costs you nothing —
you only need to move focus to pick something out of a list, to type, or to copy
with the keyboard rather than the mouse.

**git** (`Alt+G`) — read-only. Branch, ahead/behind, staged / unstaged /
untracked files with per-file line counts, and recent commits. Every `git` call
is a read: it stages nothing and commits nothing. It refreshes when the watcher
sees a write, and on a two-second timer for changes the watcher cannot see, such
as a commit made in another terminal. `Enter` opens the selected file in the
reader. `a` starts another agent in this checkout. `w` lists the repository's
other worktrees, which is how the right pane is pointed at one.

**files** (`Alt+E`) — read-only markdown and source. Markdown is rendered rather
than shown as source: headings, lists, tables, quotes, GFM alerts, footnotes,
YAML front matter as a header rather than a slab of source,
syntax-highlighted code fences, and `graph`/`flowchart` and `sequenceDiagram`
mermaid blocks drawn in box-drawing characters. Nothing of the markup is left on
the page — a heading carries its level in a rule under it or a pip before it,
not in the `#` it was typed with.

**Docstrings and doc comments are rendered where they stand.** A `.rs` or a
`.py` is still highlighted code with a line-number gutter, but its `///`, `//!`
and `"""` blocks are markdown by the time you read them, laid out in place
between the code they describe. The gutter stays honest about it: a rendered
block carries the first and last file lines its words come from, with a `┊` on
the rows between, because N lines of docstring do not become N rows of prose and
a number per row would be inventing one.

`t` swaps any of that for the source it came from, and back — that is what it
means on a documented `.rs` now as much as on a `.md`. `o` opens the **outline**:
the document's headings, or a source file's definitions, indented by level;
`Enter` jumps there and `Esc` leaves you where you were. The title carries a
breadcrumb of the section you have scrolled into.

On startup it opens the newest markdown under the
root; after that it follows what the agent writes. A document arriving while you
are looking at something else waits, and the border says `◆ Alt+E` rather than
switching under you.

A second `Alt+E` opens the **file list**, a gitignore-aware browser starting
where the open file lives. `Enter` descends or opens, `Backspace` climbs.

Inside a repository it shows dot-named files too — `.claude`, `.github`,
`.gitignore` — because that is where a good deal of the work lives, and
gitignore is there to keep the rest out. Started somewhere that is *not* a
repository, gitignore is inert, so they stay hidden as before: nothing else
would keep `.ssh` off the list.

Three keys search, and they ask three different questions:

| Key | Where | Question |
| --- | --- | --- |
| `/` | file list | which file is called this |
| `/` | document | where is it on this page (`n` / `N` walk the matches) |
| `f` | either | which files say this — reads every file under the root |

`f` reads every file under the root *except* directories that are worktrees of
another repository, which the file list will still walk you into. Standing in
one, the file list's border says `unindexed`, because a search that answered
`0 matches` there would be answering about a corpus rather than about the
tree — use `w` to move the window to that worktree instead.

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

**Telling the agent what it printed is a drag and a keystroke.** Highlight the
output with the mouse — that copies it — and press `Enter` while the highlight
is up to put those rows in the agent's composer without sending them. Long lines
come back as the lines they were written as, not as the rows the pane was too
narrow to fit them in. See [Copying out of the right
pane](#copying-out-of-the-right-pane).

**queue** (`F8`) — work lined up for the agent, for the gap between having a
thought and being able to act on it. Items go one of two ways: **send**, typed
into one agent's session the moment *that* agent goes idle, continuing its
conversation; or **dispatch**, started as its own background agent with none of
that context, running beside you. `i` writes an item, `m` switches it between
the two, `a` arms unattended sending, `Enter` does the selected one now, `d`
deletes one and `r` clears the rows it has finished with. **Those last two ask
twice** — press again, and any other key, paste or click is the answer no —
because a view key leaves your keys in this pane and a command typed at it by
mistake is made of letters. `Enter` is not guarded: it is the pane's ordinary
verb and it acts only on the row you chose, but it does end every mistyped
command there is. A send waits for the agent's own record to say it is idle and for
nothing to be sitting unsubmitted in its composer, and announces itself in the
left title first — typing at the agent during that pause defers it.

**pad** (`F9`) — a scratch pad, one per workspace, and the only thing on screen
that nobody but you wrote. It is where the thought goes when the agent is
mid-task and interrupting it would cost you the turn. `F9` opens it **and gives
it your keys**, because a pad you have to press a second key to type into is a
picture of a pad; `F9` again hands them back. It holds markdown and opens on the
source with a caret in it, so every plain key is a letter rather than a command —
the arrows, `Home` and `End` move the caret, and `Alt+T` shows you the rendering
instead, which is read-only and where a bare `t` brings the source back. What
you type is saved a couple of seconds after you stop, in your own profile
directory rather than in the repository, so it is still there next week and it
never appears in `git status`. `Alt+T` here is the pad's own key rather than a
global, and it works from either `Alt` key like every other one. One thing to
know about it: it needs the pad
to have your keys. Hand them back with a second `F9` and the pad is still on
screen but no longer listening, so `Alt+T` goes to the agent instead and the
page does not turn over. `F9` again, or a click, and it does. `F7`, a drag and
`Enter` work here as they do in every other
right-hand view, which is how a line of it reaches the agent.

**ask** (`?` from the document or the git view, `F6` from anywhere) — for Claude
and Copilot, a second copy of your agent in the right pane which **may read and
may not write**. For
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
[status](docs/status.md) has the detail. Under Codex, Ask is unavailable rather
than starting a different provider or guessing at a safe non-interactive mode.

**pty diagnostics** (`F2`) — what the emulation layer is doing: alt-screen,
application cursor, bracketed paste, mouse mode, byte counts, sizes, and the
frame clock. **DSR answered** is the one that matters on Windows — a red zero
means the session is hung rather than slow. `docs/conpty-findings.md` explains
each field.

## Keys

Everything abeam binds lives under `Alt` and the F-keys, with one exception:
`Ctrl+\`, the escape hatch. `docs/keymap.md` is the audit behind the table.
Codex's shipped defaults leave `F8` unused, but Codex keymaps are configurable,
so a local `tui.keymap` can still collide. Press `Ctrl+\` or `F12`, then the key,
to send it to Codex untouched.

| Key | |
| --- | --- |
| `Alt+G` | right pane → git |
| `Alt+E` | right pane → files (again for the file list) |
| `Alt+S` | right pane → a shell, **and focus it** (again to hand focus back) |
| `F8` | right pane → the queue |
| `F6` | right pane → ask, nothing attached, **and focus it** (again for what it displaced) |
| `F2` | right pane → pty diagnostics, and back to what it displaced |
| `F3` | file reader → light / dark page |
| `F7` | select rows of the right pane by keyboard, **and focus it** (a drag copies on its own) |
| `F9` | right pane → the scratch pad, **and focus it** (again to hand focus back) |
| `F4` / `F5` | move focus left / right; `F4` again moves along the agents |
| `Alt+J` / `Alt+K` | scroll the right pane a line — **without focusing it** |
| `Alt+PgDn` / `Alt+PgUp` | scroll the right pane a page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (twice while a child is live) |
| `F1` | key help overlay |
| `Ctrl+\` or `F12` | send the *next* key to the agent verbatim |

**Either `Alt` key works.** Windows reports AltGr as Ctrl+Alt, so on a UK, Irish
or continental layout the right-hand `Alt` key arrives carrying an extra
modifier; abeam counts both, for every row above and for the pad's `Alt+T`. The
one place that is deliberately *not* true is `Ctrl+\`: on the layouts that put
`\` behind AltGr, `AltGr+\` is a backslash you were trying to type, so
literal-next declines it and `F12` is the alias to reach for there.

A view key leaves focus where it found it. `Alt+G`, `Alt+E`, `F8` and `F2`
change what the right pane is showing without moving your keys: if you were
typing at the agent you still are, and if the right pane had them the view that
arrives has them. Two other things go with the switch — a view key un-zooms, so
that asking to see something always shows it, and it drops any highlight you had
up rather than leaving one hanging over text it no longer names. And "keeps
them" is about the *slot*, not the pane: the shell you were typing at is not on
screen any more, so what you type next goes to whatever is.

You can always tell which it is. While the right pane holds your keys its
border *leads* with the way out — `esc→agent`, or `alt+s→agent` at a live
shell — ahead of the pane's own title, so a long branch name cannot clip it
off the end. Nothing else on screen says it: four of the seven views draw no
cursor, so a focused one leaves the window with no cursor anywhere at all. The
pad draws one while you are editing and none in the rendering, for the same
reason: there is nothing there to put it in front of.

Once the right pane has focus, plain keys work — deliberately the same
vocabulary as Claude's own transcript view:

```
j / k, arrows   a line        g / G, Home / End   the ends
space / b       a page        Tab / Shift+Tab     the selection
Ctrl+D / Ctrl+U a half page   Enter               open · queue: do it now
r  refresh      t  the rendering / what was typed  Esc or q   back to the agent
o  outline of this document, when it has one
```

`r` is the one of those that is not the same everywhere: it refreshes in the
files and git views, and clears the finished rows in the queue.

**Three views do not answer to that paragraph.** The **shell** takes every plain
key, because a pane you type into cannot also read what you typed. The **ask**
is the same: its composer is live the whole time the pane is, so `j`, `k`, `g`,
`G`, `space`, `b`, `r` and `q` are letters there, and what scrolls is the
arrows, PgUp/PgDn, Home/End and `Ctrl+D`/`Ctrl+U`. So is the **pad** while you
are editing, which is why `t` there is `Alt+T` — the letter is a letter. The
pad's rendering is read-only, so the vocabulary comes back to it, `t` included.
That is what the F1 overlay's mode rows are for: a pane whose keys mean
something else, with nothing on screen saying so, reads as a broken pane.

`Ctrl+\` exists so abeam can never permanently shadow a binding of the agent you
are typing at. If a future release of an agent binds `Alt+G`, `Ctrl+\` then
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

`w` in the git view lists every worktree git knows about, with a count of the
agents of yours working in each and `▸` where the right pane is standing.
`Enter` moves the right pane there and puts the status list back. The right
pane's border names its workspace whenever it is not the session's own root.

[Design notes](docs/design.md) has the argument for the routing rule, including
why the obvious version of it does not work.

## More than one agent

`a` in the git view starts another agent **here**, in the checkout you are
looking at. `a` on a row of that worktree list starts one **there**. Both are
the whole gesture: no path to type, no confirmation, and the row's own count
goes up on the frame you pressed it.

The first of those two is the one most sessions want, because Claude Code makes
its own worktrees: open a second agent where you already are, tell it to branch
off, and it runs `git worktree add` and moves into the result. abeam does not
make worktrees for you and will not — but it follows an agent that makes one, so
the list's count and the row `x` acts on move to the worktree it is actually
working in, and so does the pane's own border. The border names that worktree
only when it is *not* the checkout abeam was started in, which is the same rule
the right pane's label follows and for the same reason: 72 columns is not enough
to spend three of them on the answer that is true by default. So two agents in
one checkout read `claude · 1/2` and `claude · 2/2`, and one that has branched
off reads `claude · 2/2 · branch-name`.

What does **not** follow is the record abeam reads to tell whether that agent is
busy: it goes on being matched against the directory the pane started in, which
is what stops one pane being told another session's state. Following an agent
also waits on `git worktree list`, which runs every ten seconds — so expect the
border and the count to catch up a few seconds after the agent moves, not at
once.

`F4` gives your keys to the left column, and pressed again it moves along the
agents. The border of the pane that has them is highlighted and reads
`claude · 2/3`, which is the only thing on screen that says which session your
next sentence is going to.

They **stack**, top to bottom in the order you started them. A pane that will
not fit whole shrinks to its title row rather than disappearing, so the roster
and the busy / idle / waiting-on-you word stay on screen for every agent. A
whole pane wants twelve rows and a border, so two agents want about 28 rows of
terminal and three want 42; below that you get one pane and title rows. The
right pane does **not** follow the agent cursor: moving between agents never
costs you the thing you were reading.

The **queue** (`F8`) aims each item at the agent that had your keys when you
wrote it, and it stays aimed there — moving the cursor afterwards does not move
the prompt. The row says which pane it is for once there is more than one, the
three-second countdown appears on that pane's border, and if you close the pane
before the item goes, the item disarms and says so rather than being sent
somewhere else. There is no way to re-aim an item: press `F4` and write it at
the pane you meant.

**Closing.** `x` twice at an agent whose child has exited closes that pane; `x`
twice on its row in the worktree list ends it even if it is still running, and
the pane's own border asks first, in those words — the second press is refused
if those words were not actually on screen, so a fast double tap becomes the
question rather than the answer. Two agents in one checkout are one row, and
abeam refuses to guess between them: it says so and points at `F4`, which from
the list is `F4` `F4` `F5` and then `x` `x`. Prompts queued for a pane you close
are not sent anywhere else — each says which pane it was for, and the border
counts them. The agent abeam started with
is the session and never closes — its exit is what abeam exits with, so leaving
it is `Alt+Q`. While any agent or shell is still live, `Alt+Q` asks twice and
the title says which of them is holding the door.

**Read `docs/status.md` before relying on any of this.** It works in the tests
and nobody has yet watched it work.

## Configuration

There is one file, it is optional, and most machines will not have one.

```
Windows  %APPDATA%\abeam\abeam.toml
Linux    $XDG_CONFIG_HOME/abeam/abeam.toml, or ~/.config/abeam/abeam.toml
```

```toml
[defaults]
view  = "git"    # git | files | shell | queue | pad | ask
focus = "left"   # left | right               — which pane has the keyboard
zoom  = false
theme = "dark"   # light | dark               — the reader's page

[preset.fleet]
host  = "claude"     # an agent abeam knows, or any program on PATH
args  = ["agent"]
view  = "queue"
theme = "dark"

[preset.openai]
host  = "codex"
view  = "files"
```

`[defaults]` is how every session on the machine opens. A **preset** is a name
behind the sigil that behaves exactly like a built-in agent: `abeam +fleet
--resume` starts `claude agent --resume` with the queue showing, and `+help`
lists `fleet` and `openai` beside `claude`, `copilot` and `codex`. `abeam
+openai [args]` resolves the preset to the built-in Codex host and forwards the
typed arguments after any preset `args`. A preset's own `args` go in *front* of
what you typed, because a subcommand is the first word of its line.

**Migration:** `codex` is now a built-in, so an existing `[preset.codex]` is
reserved and the configuration is refused. Rename that preset (for example to
`[preset.openai]`) and invoke the new name behind `+`.

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
  The Codex exception below is an unauthenticated smoke test; every other
  combination ships on tests rather than on use.
- **Nobody has driven abeam on Linux by hand.** CI builds, tests and lints both
  targets on every push to `main` and on every pull request, and the six manual
  pass criteria in
  `docs/conpty-findings.md` have been confirmed on Windows only.
- **abeam has never been run with GitHub Copilot CLI.** Not once. The selection,
  the launcher and the failure messages are tested; a session is not.
- **Codex 0.149.0 has been hosted through abeam on Windows without signing
  in.** Its welcome/sign-in screen, navigation, resize and quit path worked.
  Authenticated modes and every Linux path remain untested.
- **Nobody has typed a question into the ask pane**, and the Copilot half of it
  has never been run by any process at all. What comes back from it is a model's
  answer, which can be fluent, specific and wrong about the file it read.
- **Nobody has run more than one agent in a real window.** The whole of [More
  than one agent](#more-than-one-agent) — starting, cycling, stacking, aiming
  the queue, closing a live pane — is built and tested and has never been used.
  Expect the row arithmetic to be the first thing that disappoints: three agents
  want a 42-row terminal before all three are drawn whole.
- **Nobody has typed into the scratch pad by hand either**, on either platform.
  It has tests and it has never had a user. It also has no undo and no
  selection, it holds 64 KiB, and it saves two seconds after you stop typing —
  so a machine that loses power in that gap loses that much.
- **A turn in the ask pane that never ends has no way out** but `Alt+Q`. There is
  no cancel key and no timeout.
- **Copying takes whole rows, of the right pane, that are on screen.** Not a
  column range, so a hash comes with the row around it; not the left pane, so
  what the agent drew cannot be selected at all; and not what has scrolled past,
  which has to be scrolled back to first. The clipboard is reached with OSC 52,
  which your terminal has to honour — and a drag over text replaces whatever was
  on it, as copy-on-select does everywhere.

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
git tag v0.8.1 && git push origin v0.8.1
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
crates/abeam/src/select.rs         the rows a drag chose, and the keys that
                                   choose them without one
crates/abeam/src/ask/              the second agent, and a wire format abeam
                                   does not own
crates/abeam/src/panes/            one file per view
crates/abeam/src/panes/pad/        the scratch pad: the text, the caret in it,
                                   and the one file abeam writes
crates/abeam/tests/end_to_end.rs   abeam itself, hosted in a pty and typed at
```

Most of the reasoning lives in the module documentation of the file that owns
each decision, rather than in this README.

| Document | |
| --- | --- |
| [docs/status.md](docs/status.md) | what is done, what is not, and what nobody has watched work |
| [docs/keymap.md](docs/keymap.md) | the keyboard audit against Claude, Copilot and Codex, including custom-map gaps |
| [docs/design.md](docs/design.md) | worktree routing, the draw loop, the layout |
| [docs/conpty-findings.md](docs/conpty-findings.md) | **read before touching the pty layer.** Five constraints that look like things to tidy up and are not |

## License

MIT. See [LICENSE](LICENSE).
