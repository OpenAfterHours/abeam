# abeam

One window for an AI coding session.

Your agent runs in the left pane — hosted in a pty, parsed, and drawn by abeam,
not passed through to your terminal. The right pane shows the state of the git
worktree, the document the agent just wrote, a shell to run things in, or a
second Claude you can ask about the file in front of you — one that may read and
may not write. A file watcher drives the first two, so neither has to be asked.

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

Read the ask half the same way, because it is the same kind of gap and it is
newer: **nobody has ever typed a question into that pane.** Every test drives
it against shims and fabricated launches, the only real `claude` runs were
hand-driven probes of the protocol rather than of the pane, and what comes back
from it is a model's answer — which can be confidently wrong about the file it
was pointed at. "Not done, and known" has the whole list.

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

One watcher over one root turned out not to be one workspace, and that is the
whole of "Worktrees" further down. Claude Code makes git worktrees *inside* the
directory abeam is watching and runs other agents in them, so "what the agent
just did" and "a file changed under this root" stopped being the same sentence
the day somebody ran two agents on one project. The pane still knows what the
agent just did. What it costs to keep that true is a routing rule with an
argument in it.

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
abeam -- +1 more thing    # `--` stops abeam reading, and goes to the agent too
```

**If you used to write `abeam <program>`, write `abeam +<program>` now.**
`abeam bash`, `abeam powershell`, `abeam nu` — anything this document used to
describe as "anything else on `PATH`" — needs the sigil in front of it, and the
next section says what those lines do without one.

A `+` token is read only in the first position and there is at most one: a
prompt may begin with a `+`, and `abeam config set +x` is a real command line.
`+name` is resolved exactly as the old positional was — if it names an agent
abeam knows, `claude` or `copilot`, or a preset out of your config file, matched
without regard to case, abeam looks that entry's executables up on `PATH`;
anything else is a program name and means what `abeam bash` used to mean. Spaces
around the name are trimmed, so `abeam "+claude "` is `abeam +claude` rather
than a hunt for a program with a space on the end of it. Two words behind the
sigil are reserved and only two, `+help` and `+version`, and like every other
name behind a `+` they are matched without regard to case: `+HELP` and
`+Version` are abeam's too. There are no `+h`/`+V` short forms, deliberately: a
short form is one more word that can never be a program name, and `-h`, `-V`,
`--help` and `--version` all go to the agent now, which is the help you wanted.

**`--` is not a second exception, and it used to be one.** A leading `--` stops
abeam reading the line — so a first argument beginning with `+` is safe behind
it — and then goes to the agent along with everything after it, exactly as you
typed it. That is a fix rather than a detail: abeam used to swallow the token,
so `abeam -- --resume` started `claude --resume` and *resumed the session*,
where `claude -- --resume` hands the literal string `--resume` over as a prompt.
One command line meant two things depending on whether abeam was in front of
it, which is the whole of what the rule above exists to prevent. So
`abeam -- claude agent` is `claude -- claude agent` today, and whether that is a
prompt or a complaint is Claude's business — the point is that it is the same
answer either way.

`ABEAM_AGENT` names what to host when no `+` token did — an agent name, a preset
name, or any program — and this change is what made it worth setting: `ABEAM_AGENT=copilot
abeam --resume` resumes Copilot, where before the variable stopped applying the
moment you had arguments to pass. An empty value counts as unset, because
PowerShell leaves one behind — and so does a profile that exports a variable it
then fails to fill — and `'' was not found on PATH` names nothing anyone can act
on. A `+` token overrides it.

**Its reach grew with that, and that is a cost as well as the feature.** The old
code read the variable only when you gave abeam no arguments at all, because
anything else was a positional that selected. It is read on every command line
that does not lead with a `+` now — so one exported in a dotfile three years ago
used to touch nothing but bare `abeam`, and today silently redirects `abeam -p
"commit my changes"` into a different agent. There is no version of this that is
only the good half. What stands against it is that a `+` overrides it for one
run, that the left border says which agent is taking the typing, and that when
what it names cannot be found abeam's message says the variable is why.

It holds a **name**, not a command line, so `ABEAM_AGENT=+copilot` — the
spelling every other line here teaches — is refused with the correction printed
rather than accepted with the `+` quietly dropped. The sigil says which token on
a command line is abeam's, and there is no token to mark inside a variable.

**`abeam claude` and `abeam copilot` are refused rather than reinterpreted**,
with exit code 2 and a message naming both readings and both ways out. They
hosted those agents for the whole of abeam's life before this, they are written
that way in every older copy of this document, and what you would otherwise get
is `claude claude` — the agent's own complaint about an argument it does not
have, on a screen that never mentions abeam. That refusal is permanent, not a
migration aid, and it is a fixed lookup in abeam's own table rather than a `PATH`
probe: a refusal that depended on what happened to be installed would accept a
command line on your machine and reject it on a build server.

The message rewrites the **first token** and leaves the rest of your line
described rather than quoted, which is a correction: it used to print your whole
command line back inside `` Write `abeam +…` ``, joined with single spaces,
having never seen your shell's quotes. For `abeam claude -p "fix the tests &
ship it"` that produced an instruction which in `bash` runs two commands. Only
the first token ever changes, so that is the only part worth spelling out
exactly.

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
`--help`. What the rule deletes is the *route* — a dashed token becoming a
program name with nobody having named one — so there is nothing left to check.

This paragraph used to claim something stronger and it was false, which is worth
correcting rather than quietly softening, since its own point is that "we added
a check for it" and "it stopped being expressible" are different guarantees. It
said a dashed token could never be a program name at all. It can:
`abeam +--help` names one, `ABEAM_AGENT=--help` names one with no `+` on the
line, and `abeam +./-weird` reaches a dash-named file by path. All three are you
asking for a dash-named program, which abeam allows on purpose. What keeps such
a name off the operating system's own spawn is `launch`, which answers only with
a path it has actually located and otherwise says `` `--help` was not found on
PATH `` in abeam's own voice — and that predates this change entirely.

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

Under all of that there is one more line, naming `abeam +help`, and it is there
because **this message is also what `abeam --help` prints on a machine with no
agent installed**. Trace it: no `+`, so `--help` belongs to the agent; the agent
is not there; and what comes back is a page about installing Claude to somebody
who was asking what abeam is. `F1` is not a route out of that — it needs a
running agent — so without the line the only way to find abeam's own command
line is this document.

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
the git pane and the watcher use — **resolved once, at startup, before any of
the three is handed it.** `GetCurrentDirectoryW` reports the path the process
was given and resolves neither a junction, nor a `subst` drive, nor an 8.3 short
name; `git worktree list` resolves all three. So a session started through any
of them stands in `…\link` while every root git names is `…/real`, and those are
two different directories to every comparison abeam makes about a path — which
would leave the routing rule under "Worktrees" wrong for the whole session
rather than wrong once. The child is given the resolved spelling too, and that
is not tidiness: the child writes its own working directory into the session
record abeam reads back out. The visible cost is one surprise, and it is worth
naming rather than describing the resolution as free — somebody who works on a
`subst` drive `X:` gets an agent whose `pwd` says `C:\src\forge`. Between a name
that surprises you once and a window that cannot tell your worktrees apart, the
name is the cheaper of the two.

The right pane can later be pointed at another worktree of the same repository.
The left one cannot, ever, and "Worktrees" is where that asymmetry is argued.

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
view  = "git"    # git | files | shell | queue | ask — which right-hand view opens
focus = "left"   # left | right                     — which pane has the keyboard
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
  same both-readings refusal `abeam claude` gets, because it is the same
  mistake — made, this time, by the one person on the machine most likely to
  believe the word means their preset. The sentence differs in one place and had
  to: `abeam claude` really did host Claude for years, and `abeam fleet` has
  never hosted anybody's preset, so it says what that line *did* mean instead —
  a `PATH` lookup for a program called `fleet`.

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
rendered markdown for its source, and `Backspace` climbs a directory in the file
list. Three keys search, and they ask three different questions rather than one
question three ways: `/` in the file list is *which file is called this*, `/` in
the document is *where is it on this page* — `n` and `N` walk the matches — and
`f`, from either of them, is *which files say this*. Only the third reads the
disk, and it is the only one of the three whose box waits for `Enter` rather
than narrowing as you type. `Esc` or `q` hands focus back to the agent.

**Two of the views do not answer to that paragraph, and both are named below.**
It used to be one.

`w` is the one key in that vocabulary that is not about reading: in the git view
it opens the repository's worktrees, `Enter` there points the right pane at the
selected one, and `Esc` gives you the status list back instead of the agent —
which is why the border says `esc→git` while the list is up. **It is pane-local
rather than an `Alt` key, and the table above is right to leave it out.**
`crates/abeam/src/keys.rs`'s `HELP` lists it below the split that separates
abeam's global bindings from the keys that only mean anything while the right
pane has focus and one particular view is showing, and it is exempt from that
file's invariant for exactly that reason: no agent can be listening for a key
that is only ever delivered to a focused pane. A global spelling was available
and refused twice over — `Alt+W` is Claude's, and a view key spelled `F6` would
be a key nobody groups with `Alt+G` and `Alt+E`. The list is not a peer of those
views anyway; it is how you point one of them somewhere else.

`?` is the second key in that vocabulary that is not about reading, and the table
above is right to leave it out for exactly the reason it leaves out `w`. In the
document the reader is showing, or in the git view, it opens **ask** — a second
Claude in the right pane, described under "The panes" — with the file you were
looking at attached to the first question, and `Esc` puts back the view it
displaced the way `F2` does out of the diagnostics. Those two views and no
others: the file list and the `f` results own every key while they are up, so
`?` is inert in both, which "Not done, and known" says is a gap rather than a
decision. A question about the file you are reading is asked from where you are
reading it, so the key is only ever delivered to a focused pane; that is
the whole of the exemption, and it is the same sentence `w` relies on rather than
a second argument that happens to agree. A global spelling was not on offer here
either: every plausible `Alt` letter is Claude's or Copilot's, and there is
nothing to switch *to* from the left pane — the pane is opened by pointing at
something, which is a thing you can only do from the pane holding it.

**The shell is the first exception, and it has to be**: `Esc` and `q` belong to
whatever is running in it, along with every other plain key, because a pane you
type into cannot also read what you typed. `Alt+S` or `F4` is the way out, and
its border says so rather than leaving you to find out. `Alt+J`/`Alt+K` are
what scroll its history, which is why they are not the arrow keys the shell
would read as history.

**The ask is the second, and it is the easier of the two to trip over**,
because it looks like something you read rather than something you type at. The
composer there is live the whole time the pane is, so `j`, `k`, `space`, `b`,
`g`, `G`, `r` and `q` are **letters** — they go into the question, exactly as
they do in a find box — and the scroll half of the paragraph above is simply
untrue there. What scrolls is the arrows, PgUp/PgDn, Home/End and
`Ctrl+D`/`Ctrl+U`, with `Alt+J`/`Alt+K` reaching it from outside as they reach
every other view. `Enter` is not "open" either: it sends the question, or —
with nothing typed — hands the selected command to the shell view, which types
it without submitting. And `Esc` clears a draft before it hands anything back,
so the key that means "never mind" costs you what you were writing rather than
the pane, which is what it costs in the find boxes too. The F1 overlay carries
that as a row of its own rather than leaving it to be discovered, because a
table promising `j` in every pane must not go on promising it in the one pane
where `j` types a letter.

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
viewer saying "no such file". `w` leaves the status list for the repository's
other worktrees, which is how the right pane is pointed at one; the section
after this is the whole of that.

**files** — read-only markdown and source, and a way to reach any of it.
Markdown is rendered, not shown as source: headings, lists, tables, quotes, GFM
alerts, footnotes, syntax-highlighted fenced code, and mermaid diagrams. `t`
swaps that for the source it was rendered from, highlighted and numbered like
any other file, and back.

A ` ```mermaid ` fence is the one of those that is *redrawn* rather than
styled, and it is here because it was the conspicuous hole: syntect has no
grammar for mermaid, so the block came out verbatim and uncoloured, and nobody
writes `A --> B` to be read as `A --> B`. Two families are drawn, in
box-drawing characters, for the width the pane happens to be —
`graph`/`flowchart` in all four directions, and `sequenceDiagram`. A pane too
narrow for boxes gets the same diagram as an indented outline, or a sequence
diagram as a numbered list of its messages — the same trade a table too wide for
the pane already makes when it gives up its grid and goes out one field per
line: at forty columns a four-cell-wide box is not a diagram, it is a puzzle. Anything else — every other diagram type, a `subgraph`,
a node spelled in syntax this does not know — keeps the code block it has always
had, and `t` still reaches the source, which is the only way to read a diagram
abeam declined to draw. What never happens is the fourth thing: a drawing with
an edge missing from it. The source is always true, and a diagram that has
quietly lost a node is worse than one nobody drew. Source files get highlighting and a line-number gutter. Everything is
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

`/` in the document is the second question — where is it on this page — and it
is a different matcher from the one in the list, deliberately. A path is matched
as a subsequence, because typing `capv` to reach
`crates/abeam/src/panes/viewer.rs` is how anyone who has used a fuzzy finder
expects to get there. Prose is not: a subsequence over a paragraph matches
nearly every paragraph, so the answer is every row and the reader has been
handed noise dressed as results. Plain substring here, and smart case —
case-insensitive until the query contains a capital, which needs no key to turn
on and no explaining when it fires. `n` and `N` walk the matches and wrap round.
The query sits in the title beside the position rather than in a row of its own,
so the layout and the scroll arithmetic the pane already depends on do not move
to make room for it. `Enter` shuts the box and keeps the marks, which is the
state somebody reading is in; `Esc` does the same, a second `Esc` clears them,
and only a third hands focus back. A search with nothing marked skips the middle
press, so the border names what *this* press does rather than where the sequence
ends.

`f` — from the document or from the file list — is the third question, which
files say this, and the only one of the three that reads the disk to answer. Its
box does nothing at all while you type and runs on `Enter`. That is the one
place this pane behaves unlike itself and it is deliberate: the other two boxes
answer from something already in memory, where this one reads every file under
the root, so a box that ran per keystroke would sweep the repository for `n`,
`ne` and `nee` before you had finished typing `needle` and throw all three
answers away. The empty list says so in words, because a box that visibly does
nothing is otherwise indistinguishable from one that is broken. What comes back
is a selectable list of path, line number and the line the match is on, filling
as the sweep goes rather than arriving all at once at the end; `Enter` opens
that file with the document search already looking for the same phrase, at the
match that row counted, which is what makes the two one feature rather than two
that happen to rhyme. It does not walk the tree again — the startup walk's list
is shared, since a second gitignore walk would double the cost of startup to
re-derive what the first one had in its hand — so it inherits that walk's caps,
reads at most the first half-megabyte of any file, keeps at most a screenful of
matches from each, and stops at a total. Every one of those is visible when it
is hit: a count that is a prefix of the truth is written `137+` rather than
`137`, and the per-file cap is named outright as `· 18 files cut`, because it is
the only one of the four whose remedy is a key already on screen — what it left
out is in files the list did reach, and `Enter` on any row of one of them shows
all of them.

**The two searches do not agree about what a match is, and the pane says so
rather than hiding it.** `f` matches the lines in a file; `/` matches the rows
the pane drew, which is what lets one search serve rendered prose, its source
and a highlighted `.rs` without the markdown renderer having to grow a second
output — rendering reflows and drops syntax, so no offset into the file means
the same thing on both sides of `t`, and a mapping is what searching the source
would have needed. Those are the same text for most files and not for all. A
line too wide for the pane is wrapped, so a match that straddles the break is
one the drawn page does not have; that is not a markdown problem, and dragging
the pane narrower is enough to reach it in a plain source file. In rendered
markdown the source syntax is not on the page at any width, so `**` cannot be
found there at all. So a page with no match for the phrase names the way out
that is true for the body in front of you — `· t for source` on rendered
markdown, `· widen if a wrap split it` on anything else — and a result that
lands on a different match from the one you chose says `· not the 3rd` rather
than a confident `2/2` about a question nobody asked. Retiring the gap needs
rows grouped by the source line they came from: `source_lines` knows that
grouping and `markdown::render` does not, so it is either a second output from
the renderer or a rule true of one body form and not the other. It is written
down in `crates/abeam/src/panes/viewer/search.rs` rather than half-done, because
half of it is the inconsistency and not the fix.

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

**ask** — `?` from the document the reader has open or from the git view, and a
second Claude in the right pane which **may read and may not write**. The gap it
fills is narrow and constant: you are reading a file, or a diff, and a question
comes up that is *about* what is on screen — what does this call do, where is
this written, is this the only caller — and every way of answering it costs the
conversation in the left pane. You interrupt a turn, or you queue the question
and wait, or you open a second terminal. This is the fourth way.

**Nobody has ever asked it anything.** Everything in this section follows from
the code and from tests that drive the pane against shims and strings, with no
`claude` anywhere near them; no human has typed a question into it and read the
answer back. So read the present tense here as what abeam does rather than as
what somebody has watched happen — and read what comes out of it as a model's
answer, which can be fluent, specific and wrong. "Not done, and known" says
both of those again, with the rest of what this pane costs.

The read-only claim is a flag rather than a promise. The child is started with
`--tools "Read,Grep,Glob"`, which is an allowlist over the built-in set: what is
not named there does not exist for that session, so there is no `Write`, no
`Edit` and no `Bash` to permit or refuse. `--disallowedTools
"Write,Edit,NotebookEdit,Bash"` says the same four words from the other side, and
`--strict-mcp-config` is the one worth reading twice — `--tools` is an allowlist
over the *built-in* set and says nothing about MCP servers, so without it a
user's configured servers would be loaded into a session abeam had just called
read-only. The tools the child reports back on its opening line are drawn along
the bottom of the pane, so what is on screen is what it actually got rather than
what abeam meant it to get. A probe on 2026-08-05 asked it to create a file in a
throwaway directory and it could not; the model **did** emit a `Write` call, and
what stopped the file appearing is that nothing executed it. The guarantee is
enforcement, not the model's cooperation, which is the right way round.

What travels is a **path, never a payload**. `?` attaches the file the pane you
came from was showing, the pane draws `▸ viewer.rs` above the composer until the
question goes, and what the child is handed is one line naming that file. It
stands in the same directory and has `Read`, `Grep` and `Glob`, so naming the
file is enough: it fetches what it needs and skips what it does not. Sending the
body instead would mean a cap, a truncation notice, a decision about how much of
a four-thousand-line file to send, and a question that silently carries part of
somebody's repository off the machine — where a path costs one line on screen and
is the *whole* of what was sent, which is what makes that row complete rather
than a summary.

**A turn is mostly not answer, and the pane draws what it is instead.** Probed
on 2026-08-09: one ordinary question took **30.7 seconds**, produced 123 lines
of protocol, and ten of them were text. The rest was the child working — opening
a block of reasoning, assembling a tool call, waiting for a file to come back —
and a pane that draws only the text draws nothing at all for most of every
question. So the working is on screen too. A line under the question names each
tool as it is asked for and what it was asked about — `Read
crates/abeam/src/scroll.rs`, `Grep fn key`, with the repository root taken off
the front — a reasoning block opening reads `thinking`, and the row above the
composer counts the seconds since you pressed `Enter`. That counter is the
load-bearing part: `answering` on its own says the same thing at second one and
at second thirty, and second thirty is where you start to wonder whether the
pane has died. The tool line stays after the turn, because *which files did it
read* is the question you have about an answer you are not sure of, and the
finished turn is labelled with what it cost in both currencies — `30s ·
$0.1634`.

**What streamed is the answer, and the `result` line is not a better copy of
it.** The same probe found the pane deleting the thing it was there to show. A
turn ends with a `result` message carrying answer text, and abeam took that as
authoritative and replaced what it had drawn — but `result` carries the *last
text block* of the turn rather than the turn. Measured twice: 1144 characters
streamed against 944 reported, and 2449 against 2278. Both answers said
something before reaching for a tool, and both times that opening was thrown
away at the end. At its worst it threw away all of it, because
`--permission-mode plan` — which abeam passed on the theory that a
read-and-propose mode suits a read-only tool list — instructs the model to
finish by calling `ExitPlanMode`, a tool this session is deliberately not given.
So a long, correct explanation would arrive, and be replaced at the end by one
paragraph about being in read-only mode with no way to present a plan. The mode
is now `default`, which changes no authority — `--tools` is the guarantee and it
is untouched — and the streamed text is authoritative whenever there is any. A
`result` that is *not* how the streamed text ends is appended rather than
substituted, because a duplicated paragraph is something you can read past and a
deleted one is not. Keeping every block exposed one more thing the wire does not
carry: two text blocks either side of a tool call arrive with nothing between
them, so the pane puts the paragraph break back where the interruption was.

**`Enter` never runs anything.** An answer full of shell commands is the ordinary
shape of a useful answer, and the distance between reading one and running it is
where this could do real harm. So there is exactly one route out and it ends at a
prompt: `Tab` picks a single-line command out of the transcript, `Enter` on an
empty composer hands it to the shell view, and the shell **types it without
submitting**. A block of more than one line is never offered — not truncated, not
joined with `&&`, not offered as its first line — because a command assembled out
of several lines by a program is indistinguishable, once it is sitting at a
prompt, from one somebody read and approved. The refusal is drawn where the offer
would have been, and the way through is to copy it out of the answer.

The composer is live the whole time the pane is, which costs it half the scroll
vocabulary and is worth naming rather than papering over: `j`, `k`, `g`, `G`,
`space`, `b`, `r` and `q` are **letters** here, exactly as they are in a find
box, so the arrows, PgUp/PgDn, Home/End and `Ctrl+D`/`Ctrl+U` are what scroll,
and `Alt+J`/`Alt+K` still reach it from outside. `q` is in that list rather than
left out of it because it is the key somebody presses to leave, and here it
types a letter. The F1 overlay says all of that in its own row rather than
leaving you to discover it, and "Keys" names this pane as the second of the two
exceptions to the one scroll vocabulary. Answers stream and the view follows the
bottom until you scroll up, and stops until you come back to the end.

It is one session per pane and per workspace, held open across questions, so the
second answer can remember the first — that is the whole reason it is a
long-lived child rather than one process per question. `Esc` puts the view back
and leaves it running; what ends it is quitting, and nothing is persisted, so
`Alt+Q` and a crash lose the conversation equally and by design. Asking again
after that starts a fresh reader, which the pane says on screen rather than
letting you find out from an answer that has forgotten the question before it.
`Ctrl+L` ends the conversation and starts a fresh one, and **it ends the child
along with it** — which is the only version of that key worth having. What a
long conversation costs is not the rows on screen: every turn is sent again as
context with the next question, so the file you finished with half an hour ago
is still being paid for. Clearing the pane and keeping the reader would have
hidden that rather than fixed it. The composer keeps what you have typed and the
attached file stays attached, because starting again about *this* file is what
pressing `?` and then `Ctrl+L` means.

The title carries what the session has cost so far, in three decimal places,
because a trivial exchange is a few hundredths of a dollar and a title reporting
`$0.00` over something that cost money is worse than one reporting nothing. What
the pane does *not* do is warn you about it. It is the same Claude, on the same
account, started by the same person — a standing caution about that would read
as though abeam had found something alarming, and there is nothing alarming to
find. The row along the bottom is spent on what a reader can act on instead: the
tools the child actually got, and the key that ends the conversation.

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

## Worktrees, and whose change is whose

This section exists because of a bug, and the bug is the fastest way to explain
the feature. Claude Code does not stay in one directory: it makes git worktrees
— the usual place is `<root>/.claude/worktrees/<name>` — and runs agents in
them, so a machine with two agents on one project has two working trees inside
one watched directory. abeam runs a single recursive watch of the repository
root and is right to. What that watch could not do on its own was say *whose*
change it had just seen. So another agent, working in
`<root>/.claude/worktrees/other`, refreshed your git pane on every file it wrote
and pulled its scratch markdown into your reader, with nothing on screen
admitting where any of it came from. A pane that reports somebody else's work as
yours is worse than a pane that reports nothing, because it is not obviously
broken.

One line in the watcher's noise list would have stopped those events at the door
and closed that bug in an afternoon. It would also have blinded abeam inside its
own worktrees for ever, which is the half of the feature you can actually press
a key on: the right pane is pointed *into* those directories now, and a noise
entry means the watcher never wakes it. Noise is for directories nobody wants to
read. `.claude/worktrees` is where the work is.

**The obvious fix is not the fix, and that is the part worth reading.** Route by
path prefix — take the event if it starts with the workspace root. It does not
work. `<root>/.claude/worktrees/other/NOTES.md` **has `<root>` as a prefix**; it
genuinely is inside the repository, which is exactly what makes the worktree
layout convenient, so a prefix test hands it straight back to the workspace
rooted at `<root>`, which is the case being complained about. The naive fix is a
no-op dressed as a rule. What works is **innermost ownership**: a path belongs
to the *longest* known root that contains it, and a pane takes an event only
when that longest root is its own. That is not a convention invented here to
make a bug go away — it is git's own model. `git status` in the main worktree
does not report a nested worktree's modifications, because the nested tree has
its own index and its own HEAD, and a pane that mirrors `git status` should
agree with `git status` about whose changes those are.

Ownership alone was half a rule, and the missing half is not a corner case — it
is the ordinary shape of the very event the rule exists to route. **Writing one
file inside a nested worktree makes the watcher report the directories above it
in the same debounced batch.** The file is owned by the worktree and dropped
correctly; `<root>/.claude` and `<root>/.claude/worktrees` are owned by the
*enclosing* workspace, because nothing nested is an ancestor of them. Routed on
ownership alone they are indistinguishable from somebody editing in the root by
hand, so a neighbour's write still bought this window a frame and a `git status`
— the whole thing the rule was built to stop, arriving one directory up. So the
second half is that **a directory containing another workspace's root is not
evidence about its own**. The only way such a directory can have changed is that
something inside the nested workspace did, and that something has already been
reported under its own name and routed to whoever owns it. A path that *is* a
root stays evidence about itself, and that arm is load-bearing rather than tidy:
every root contains a nested one the moment a worktree exists, so without it the
agent's own workspace would go silent the first time anybody added one — the
routing bug arriving through the back door, in the one workspace that is always
on the list.

With that settled, the worktrees are worth showing. `w` in the git view — plain
`w`, with the right pane focused — lists every worktree git knows about: the
branch name, or the directory's own name where a detached or bare worktree has
no branch to use; `agent` against the one the hosted agent is running in; who is
working there, in Claude's own words rather than a vocabulary abeam invented;
and `unwatched` on anything the single watcher cannot see. A `▸` marks where the
right pane is standing. `Enter` moves the right pane to the selected row and
puts the status list back, because looking at that worktree's git is what the
switch was *for*. `Esc` — or `w` again — leaves the list without moving
anything, and the border says `esc→git` rather than `esc→agent` so you are not
left guessing which of the two it means. The workspace you are in and the
agent's own always have a row, discovered or not — the list is *how* the right
pane is switched, so a workspace with no row on it is a workspace nobody can get
back to, and neither absence is exotic: `git worktree list` names the repository
rather than the subdirectory you started abeam in.

**The left pane never moves, and that asymmetry is the design rather than an
unfinished half of it.** A live child's working directory belongs to the child;
there is no call that moves a running process to another directory, so the agent
stays where it was started for as long as it runs. The window therefore
deliberately disagrees with itself about where it is, and the border is what
keeps that honest — it names the workspace the *right* pane is on, and **only
when that is not the agent's own**. That suppression is not tidiness either. The
pane is 46 columns; a label on every title spends three or four of them saying
the one thing that is true by default, and pushes the branch name and change
count the git title exists for off the end of the border. Shown only when it is
news, it costs nothing and says everything. The list marks `agent` separately
from `▸` for the same reason: the point of opening it is to look at a workspace
the agent is *not* in.

Three views read a directory, and the switch reaches all three. The git pane
re-roots and says "reading the repository…" until the first refresh lands, in
preference to drawing the other repository's branch and change count under the
new workspace's name for most of a second. The reader rebuilds its whole index
and opens the newest markdown of the worktree it has moved to, exactly as it
does at startup. The command view is per workspace, and that is the one place
the asymmetry above reaches the right-hand side as well: a shell cannot be
re-rooted any more than the agent can, so switching with the shell up starts a
*second* child, in the new worktree, the first time it is drawn there. Every
one of them is ticked whether or not it is on screen, so a hidden workspace's
child can still exit, and `Alt+Q` consults all of them — otherwise quitting
would kill somebody's build in a workspace they were not looking at.

What deliberately does *not* follow is everything belonging to the agent: the
idle/busy probe, the queue and the background dispatcher stay where the agent
is. They are the session's, not the view's. Re-rooting any of them would mean a
prompt queued for the agent being aimed at a directory the agent is not in,
which is the one mistake in this program nobody would see happen.

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
crates/abeam/src/workspace.rs      the worktrees of the repository, and which of
                                   them owns a watched path. The routing rule
                                   and the argument for it live here.
crates/abeam/src/paths.rs          when two spellings are one directory, and the
                                   one spelling everything starts from
crates/abeam/src/ask/              the second Claude: what it is started with,
                                   and a wire format abeam does not own. `mod.rs`
                                   is the record of one run against one version.
crates/abeam/src/panes/viewer/mermaid/   a diagram on a character grid, or an
                                   honest refusal to draw one. `mod.rs` holds
                                   the rule both families answer to.
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
against it on 2026-08-01. All five right-hand views, the file list, the
rendered/source toggle, the watcher driving what it should, and the three
searches under the reader — a file by its name, a phrase on the page, a phrase
in every file under the root. Focus, zoom, help, the diagnostics view, and the
literal-next escape hatch. Agent selection and the launcher underneath it. The
Unix port, in the sense that the whole workspace builds, tests and lints clean
for both `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` — see
"Platforms" for the sense in which it is not done. Mermaid flowcharts and
sequence diagrams, drawn rather than shown as source. 752 tests on Windows, and
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

- **Most of mermaid, by diagram type, is still shown as source.** Two families
  are drawn — `graph`/`flowchart` and `sequenceDiagram` — and `stateDiagram`,
  `classDiagram`, `erDiagram`, `gantt`, `pie`, `mindmap`, `journey`,
  `quadrantChart`, `gitGraph` and the rest are not. Nothing is lost when one
  arrives: it renders exactly as every mermaid fence did before this existed,
  which is why the two were worth doing without the other nine. `stateDiagram`
  is the obvious next one, because it is a labelled graph and would reuse the
  flowchart's layout wholesale rather than needing its own.
- **A `subgraph` declines the whole diagram**, and so does `click`. Both carry
  text — a group's title, a URL — and there is no drawing of them here that
  keeps it, so under the rule the module is built around the fence keeps its
  source rather than showing a flattened graph with the grouping silently gone.
  This is the gap most likely to be met in practice, because a `subgraph` is how
  anyone draws a diagram with two halves. Also declined: mermaid 11's
  `A@{ shape: … }` node syntax and its `@{…}` edge form, which is newly written
  mermaid and so the second most likely to be met.
- **`actor` is drawn as a box, exactly like `participant`.** Mermaid draws a
  stick figure; a terminal has one cell to do it in and no glyph whose width
  every terminal agrees on. The name in the box already says which one it is,
  and nothing else in mermaid depends on the distinction, so it is accepted and
  drawn rather than declined.
- **Two edges that cross between the same pair of ranks cannot be told apart.**
  `A --> D`, `B --> C`, `A --> C`, `B --> D` draws two identical horizontal
  rules whose corners merge into the risers already in those columns, and a
  reader cannot recover which run belongs to which target. The ordering pass
  removes the simple crossing, so it takes that four-edge shape to see it.
  Fixing it means reserving a *column* of the drawing per crossing rather than a
  row of the band — a different layout, not a tidier version of this one — and
  the diagram is not wrong meanwhile, only unrecoverable at that one join.
- **A flowchart declines below the width of its own longest word.** The pane
  can be narrow enough that a diagram which parses draws nothing, and the fence
  falls back to source. That floor is a property of the document rather than of
  the pane: `Choice` is six cells, so no amount of arranging fits it into four,
  and breaking it in half to make it fit would be the one thing this is not
  allowed to do. It is why the outline exists and where even the outline stops.

- **`abeam bash` is a prompt now, and nothing says so.** This document used to
  advertise `abeam bash`, `abeam powershell` and "anything else on `PATH`", and
  under the rule above every one of those is a word handed to Claude, which will
  start answering it. That is strictly worse than what `abeam claude` gets: the
  refusal catches the two agent names and every preset name, so those at least
  produce a complaint with abeam's name on it. It cannot catch a bare program
  name, and deliberately: the only way to know that `bash` is a program on
  *this* machine is to probe `PATH`, and a refusal that depended on what happens
  to be installed would accept a command line on your laptop and reject it on a
  build server. So the cost is paid in documentation instead, which is what the
  migration line under "Running it" is. The same shape, reversed, is the other
  live cost of the flip: a prompt that genuinely begins with a `+` is read as a
  program name, because first position is exactly where a prompt lands.
  `abeam "+1 to shipping this"` looks for a program called `1 to shipping this`
  — the message names the `--` escape, and it is still a line somebody will type
  once before they learn it.
- **Worktree discovery polls, so the routing rule is allowed to be ten seconds
  out of date.** `git worktree list` runs immediately at startup and every ten
  seconds after it, and both directions of that lag are worth naming rather than
  leaving to be found. A worktree somebody has just added is not on the list
  yet, so for up to ten seconds its whole checkout has no innermost root of its
  own, every path in it is handed to the enclosing workspace, and all of it
  counts as evidence — which is the original routing bug, for ten seconds, in a
  worktree that has existed for ten seconds. A worktree somebody has just
  removed is still *on* the list, so its former parent directories go on being
  suppressed for the same window: a real edit at `<root>/.claude` in that window
  is dropped rather than misrouted. Neither is fixable by being cleverer about
  the rule, because both are the list being out of date rather than the rule
  being wrong, and the rule has nothing but the list. The git pane's own
  two-second poll makes both of them cost one wrong refresh rather than a screen
  that stays wrong; the reader has no such net, so in the first window a
  neighbour's document can still be pulled in front of you and in the second one
  of your own is quietly dropped. Shortening the window would mean a second
  watch, on `.git/worktrees`, and one recursive watch is a decision the watcher
  makes on purpose.
- **A worktree outside the repository root is listed, switchable, and not
  watched.** `git worktree add ../elsewhere` is ordinary, and abeam shows it and
  will point the right pane at it — but the single recursive watch covers the
  directory abeam was started in, so nothing written in that worktree ever
  reaches the router. The git pane falls back to its own two-second poll, which
  is the same net a commit made in another terminal lands in, and the reader
  does not follow anything at all: it opens the newest markdown once, on the
  switch, and then sits there. The row in the `w` list reads `unwatched`, and
  the reader says as much on the empty screen it shows when there is nothing
  open — because a pane that is merely slow looks exactly like a pane that is
  broken unless something admits which it is.
- **The occupancy column shows an id where it should show a name.** Claude's
  session records and roster entries both carry a `name` — `"forge-c5"`, or a
  dispatched task's own title — and it is the field that column wants;
  `crates/abeam/src/agentstate.rs`'s `Wire` does not parse it, so what is
  rendered is the roster's short `id` beside the status word. That is worse than
  it sounds, because the `id` is a *background* agent's and an interactive
  session does not have one: the common case, somebody typing in a worktree,
  renders as a bare `working` with nothing saying who. It is one field on
  `Wire`, one on `Session`, and one line of a struct literal in the queue pane's
  tests.
- **A session that had already moved into a worktree before the probe first read
  a record is never discovered.** The probe identifies the hosted session by an
  exact match on the agent's own root, and the worktrees are consulted nowhere
  in that search — which is a correction rather than an omission, and the reason
  is worth having. Claude's neighbouring agents
  run *at* the worktree roots git names, so a set consulted during discovery
  does not merely blur the question, it admits exactly the sessions it was meant
  to exclude: a recycled pid landing on a neighbour's record, a few milliseconds
  of clock skew sending the search down a fallback that takes the newest agent
  in the repository, or a second abeam window during the second before its own
  Claude writes anything. Each of those answered `Idle`, and `Idle` is the one
  answer that types a queued prompt into a mid-turn agent. Only *revalidation*
  of a record already established as ours is widened, and it is tied to the
  `sessionId` that was ours — without that the remembered path is a pid, and a
  pid comes round again. What the strictness costs is the case in the heading:
  readiness answers `Unknown` for the whole session, the automatic send never
  fires, the queue pane goes on saying it is waiting for the agent to be idle,
  and the queue drains by hand. That is the direction this module fails in on
  purpose.
- **`paths` compares spellings and does not normalise them**, and two things
  follow that somebody will otherwise reach for a fix to. `..` is left where it
  is, so `under(C:\a, C:\a\..\b\x.md)` is true and a containment rule can in
  principle be walked out of; nothing abeam has emits a `..` — `notify` reports
  resolved absolute paths, `git worktree list` prints absolute ones, and the one
  path a person types is flattened at startup — which makes it unreachable today
  and not unreachable by construction, and that is the difference worth writing
  down. And `\\?\C:\repo` compares unequal to `C:\repo`, because the verbatim
  prefix is a different first component to the platform's own parser. Both are
  the same refusal: canonicalising per comparison touches the disk, and `under`
  is asked once per watched path per known workspace — thousands of stat calls
  under a `git checkout` to answer a question about two strings. What pays for
  that is the single resolution at startup described under "Running it", which
  is also the only reason a session reached through a junction routes correctly
  at all; `resolve_root` strips the verbatim prefix back off rather than handing
  a third spelling to everything downstream. The fix for a `..`, if a source
  ever emits one, is to resolve it at that source — resolving it textually is
  wrong wherever there is a symlink, since `a/link/..` is not `a`.
- **Each workspace you visit gets its own shell, and any of them can hold the
  door.** A shell cannot be re-rooted for the same reason the agent cannot, so
  switching workspaces with the command view up starts a second child rather
  than moving the first; the number of shell processes grows with the number of
  worktrees somebody has typed in, and `Alt+Q` asks about every one of them, so
  a build left running in a workspace nobody is looking at still makes quitting
  ask twice. The related cost is a workspace `git worktree remove` has deleted
  while a child of yours is still running in it: abeam keeps it rather than
  killing the build, it drops off the list because the list is built from what
  git said, and switching away from it is a one-way trip until that child
  finishes. Unlisted-and-still-running is the smaller of the two failures, and
  it is the one deliberately chosen.
- **The shell view has never been driven by a human.** A test types `set /a
  123*456` into the real binary and reads `56088` back off the screen, which is
  more than a smoke test and still less than use: the six pass criteria above
  were confirmed against Claude in the *left* pane, and a shell in a
  46-column right pane is not the same thing. Expect the first real `cargo test`
  run in there to find something about width, wrapping or the scrollback that no
  test thought to ask about.
- **The ask pane has never been driven by a human, and it is newer than
  everything else here.** No person has typed a question into it and read the
  answer back. What exists is three probes — one `claude -p` started by hand on
  2026-08-05, asked two questions down one standard input, which recalled a word
  from the first in the second and cost $0.054; and two more on 2026-08-09,
  driven the same way, which are where the numbers above about turn length and
  the `result` line come from — plus a suite that drives every argument in the
  pane and every line of the protocol against strings and shims, with no
  `claude` anywhere near it. That is more than a smoke test and still less than
  use.

  The version of this bullet written on 2026-08-05 said to expect the first real
  question to find something about "what a long tool-using turn looks like when
  the only thing on screen is a growing paragraph". That is exactly what it
  found, three ways: half a minute of blank pane, a warning about a rate limit
  on every healthy session, and an answer deleted at the end of the turn that
  produced it. All three are fixed above and none of them was visible from
  inside the tests, because every one of them was the pane being *shown the
  wrong thing* rather than doing the wrong thing with it. The next such bug will
  be found the same way, so the prediction stands: expect the first sustained
  use to find something about the forty-six columns an answer has to wrap into,
  or about a shape of turn none of the three probes happened to produce.
- **The answer can be wrong, and nothing on screen says so.** What comes back is
  a model's answer about a file it went and read, which is exactly as reliable
  as the agent in the left pane and no more — it can name a caller that does not
  exist, miss the one that does, and be fluent and specific about both. Every
  other risk this pane carries is written down somewhere: what it may do, what
  it costs, what it shares. This one was not, and it is the one that most needs
  saying, because a confident paragraph in a 46-column pane beside the file it
  is about reads as a fact about that file. There is no fix here, only the
  admission: it is a second Claude, not a second opinion from something that
  cannot be wrong. Check what it tells you the way you would check the left
  pane, and the `Read`, `Grep` and `Glob` it was given are what it checked
  against — everything else it says came out of the model.
- **It reads a working tree the agent on the left is writing.** The child is
  handed a path and goes and reads it whenever it gets round to it, which can be
  halfway through the edit the left pane is making — so an answer can describe a
  file that never existed in that state before the read and does not exist in it
  after. Nothing on screen dates the read: the transcript says what was asked
  and what came back, and not which version of the file was underneath. Every
  other pane here that can be out of date looks out of date — the git pane says
  it is reading, the reader says what it has open — and an answer about a file
  caught mid-edit looks exactly like an answer about the file.
- **A turn that never ends has no way out.** There is no cancel key and no
  timeout: the pane draws `ask · answering` and goes on drawing it. That is a
  reachable state rather than a hypothetical one, because `parse_line` drops
  every message type it does not know — so a child that stops mid-turn to ask
  abeam for something abeam has never heard of gets no reply, never sends the
  `result` that is the only reliable end of a turn, and stays `answering` for
  the rest of the session. `Alt+Q` is the whole of the escape. What has changed
  is only that it is now *visible*: the composer row counts the seconds, so a
  wedged turn reads `answering 900s` rather than looking like a slow one. Being
  able to tell is not being able to stop it. The dropping is deliberate and the
  wire-format bullet below is the argument for it; what is missing is a way out
  beside it.
- **`?` is inert in the file list and in the `f` results.** It opens the ask from
  the document the reader is showing and from the git view, and nowhere else —
  the list and the results each own every key while they are up, because a pane
  cannot hand the same key to two vocabularies and hope, and neither of them has
  an arm for `?`. So `Alt+E` `Alt+E` reaches a view where the key the F1 overlay
  advertises two rows above does nothing at all, silently. The overlay says
  "document, git" rather than "files, git" for that reason, which is a correction
  and not a fix: the honest answer is that the list should ask about the file the
  selection is on, exactly as the git view asks about the row it is on.
- **The transcript costs more to draw the longer it gets.** It is laid out as
  one markdown document, on the frame path, exactly as the reader's own body is
  — and every fragment of a streaming answer changes that document, so while an
  answer is arriving the *whole* transcript is re-wrapped once per frame, every
  earlier turn included and not only the one growing. Measured in a release
  build at a 46-column pane: **2.25 ms per frame at 10 turns, 3.74 ms at 30 and
  6.51 ms at 40**, against the 8 ms a frame has and in a frame that still has to
  redraw the agent's whole screen beside it. An idle transcript costs nothing —
  the layout is kept until something changes it or the width moves — so the
  whole of the cost lands exactly while the pane is busiest. Forty turns is a
  long conversation rather than an absurd one and nothing stops it growing.
  Retiring it means laying each turn out once and keeping it, which is a
  per-turn cache with a width rule on it rather than a smaller constant.
- **Starting the reader blocks the loop that draws**, measured at **11.6 ms**
  for the first question, against the same 8 ms. So the frame carrying
  somebody's first question is a late one, and it is late on the keystroke most
  obviously theirs. It is once per workspace rather than once per question — the
  session is held open across questions — which is the only reason this is a
  hitch rather than a stutter, and it is the same shape as the shell starting
  its child on the first frame that draws it. Neither has been moved off that
  path.
- **The wire format this rests on is not published, and what is written down is
  one run on one version.** Claude's CLI reference documents that
  `--input-format stream-json`, `--output-format stream-json` and `--session-id`
  exist, and the SDK layered over them; the *shape of the lines* — that a
  `result` is the only reliable end of a turn, that `text_delta` is the fragment
  worth showing and thinking deltas are not, that a `system`/`init` reports the
  tools actually granted, that the child exits 0 when its standard input closes
  — is none of it specified anywhere. Every one of those was read off a single
  run against **Claude Code 2.1.222 on Windows**, and it is recorded that way in
  `crates/abeam/src/ask/mod.rs` so that the day it stops being true somebody can
  see what it used to be. A release that renames a field does not break a
  contract; it breaks an observation. The parser ignores line types it does not
  know rather than complaining, which is what makes that failure look like a
  pane that has gone quiet rather than one that has crashed — and the one line
  that would show it is `crate::ask`'s `Broke`, which reaches the transcript
  rather than a log.
- **On a Windows npm install, `Drop` cannot reach the grandchild.**
  `crate::launch` starts a `claude.cmd` by naming `cmd.exe` in front of it, so
  the process abeam holds is the interpreter and the Claude is *its* child.
  Killing `cmd.exe` does not kill a node underneath it — the same limitation
  `abeam_pty` answers with a job object for the pane on the left, which is
  machinery this module does not have. What closes it in practice is the
  observation above: the child exits 0 when its standard input closes, and
  `Drop` drops the write end of that pipe *before* it kills anything, so an
  orphaned node reaches end of file on its next read and leaves of its own
  accord. That is a mitigation resting on observed behaviour rather than a
  guarantee resting on the operating system, and the difference is the point: if
  it ever has to be a guarantee, the answer is `abeam_pty`'s job object rather
  than a longer kill. A native install and every Linux install are unaffected —
  there is no interpreter in between, and the process abeam holds is the one it
  kills.
- **Nothing caps what it spends, and a long conversation costs more per
  question than a short one.** It is the same account as the agent, which is
  what anybody would expect and is not the part worth writing down. What is
  worth writing down is the shape of the bill: context is re-sent with every
  turn, so the tenth question in a conversation carries the nine before it and
  costs accordingly. `Ctrl+L` is the whole of the answer abeam offers, and it is
  a manual one — nothing clears the conversation for you when you move to
  another file, and the title's running cost is disclosure rather than a
  defence. `--max-budget-usd`
  exists and is not passed: a ceiling abeam picked would be a number nobody
  chose, and a ceiling in the config file is a decision worth making after
  somebody has seen what a day of this actually costs. Every workspace gets its
  own session too, for the reason each gets its own shell — a child's working
  directory belongs to the child, and a reader still standing in the checkout
  you left would resolve a path against the wrong one and answer confidently
  about somebody else's file — so switching workspaces with the ask up starts a
  second `claude` on the next question rather than moving the first. Both are
  cold until a *question*, so a session that never presses `?` never starts one.
  **The pane is not quite free either, and this used to say it was.** Before it
  can tell you on screen that there is no `claude` here to ask, it has to find
  out, and finding out is a `PATH` walk. That walk happens once per workspace,
  on the frame that first draws the pane rather than when the workspace is made
  — so pressing `?` is what pays for it, a workspace you never ask in pays
  nothing, and every question after the first in one you have pays nothing
  again. A walk is not a process, which is why this is a clause rather than a
  bullet of its own; it is here because "pays nothing at all" was a stronger
  sentence than the code supports, and the strong version is the one somebody
  would have built on.
- **A command handed to the shell is refused when the shell is `cmd.exe`.**
  `Enter` on an empty composer types the selected command at the shell view
  without submitting it, and that write is a bracketed paste — which abeam will
  not send to a child that has not asked for the mode, because without it a
  newline in what abeam wrote would submit. PSReadLine asks; `cmd.exe` never
  does. So on the `cmd` fallback, and on any shell without a line editor, the
  hand-off waits ten seconds and then says so in the transcript rather than
  typing anything. Refusing is the safe direction — nothing appears, rather than
  a command running unread — and the command is still on screen in the answer
  above, which is what the message points at.
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
  plain text. A mermaid fence is laid out on that same path and has caps of its
  own — 32 KiB of source, 128 nodes, 256 edges, and a row count past which the
  drawing stops being a drawing — and those are set by legibility rather than by
  the clock: a graph at exactly the caps lays out in 4.7 ms, against the 170 ms
  that constant above it is a budget for, and draws 668 rows, which is fourteen
  screens. Past any of them the fence stays source. A slow network share can
  still stall the frame that opens a file.
- **The document search cannot always reach what the repository search found.**
  `f` matches the lines in a file and `/` matches the rows the pane drew, and
  those are the same text for most files and not for all: a line too wide for
  the pane is wrapped, and a match straddling the break is not on the drawn page
  at all. It is reachable in a plain source file by dragging the pane narrower,
  and no width fixes it in rendered markdown, where the source syntax is not on
  the page at all. A drawn mermaid diagram is the sharpest case of that same
  rule: a node label wrapped inside its own box is not one run of text on any
  row, and the mermaid source `f` matched is not on the page in any form, so
  `t` rather than the width is the whole remedy there. Neither is silent — the page names the remedy its own body
  form has, `t` or the width, and a result that lands on a neighbouring match
  says `· not the 3rd` rather than a confident `2/2` — but naming a cost is not
  the same as paying it. Retiring it means grouping rows by the source line they
  came from, which `source_lines` knows and `markdown::render` does not, so it
  is either a second output from the renderer or a rule true of one body form
  and not the other, and an inconsistency between the two forms is worse than
  one honest rule that costs something.
  `crates/abeam/src/panes/viewer/search.rs` has the argument, written down
  rather than half-done.
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
  reachable on Unix, where an argument is bytes; `args_os` is the fix. Its
  exposure grew when the command line was handed to the agent, which is the
  reason it is worth doing rather than noting: abeam's arguments used to be a
  program name and a handful of flags abeam had written down itself, and they
  are now *the prompt* — arbitrary text a user pasted, from an editor, an issue
  tracker or another program's output, which is where a stray byte comes from.
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
