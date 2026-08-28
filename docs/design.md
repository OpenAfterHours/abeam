# Design notes

> Moved out of `README.md` so that document could be something a new user reads
> start to finish. Nothing here was rewritten on the way.
>
> This is the *why*: the arguments behind decisions that look arbitrary until
> you know what went wrong first. Why one leading `+` token and nothing else on
> the command line is abeam's; why the config file is read from your profile and
> never from the repository; why two of the six views do not answer to the
> shared scroll vocabulary; why the obvious version of the worktree routing rule
> is a no-op dressed as a rule.
>
> The rest of the reasoning lives in the source, in the module documentation of
> the file that owns each decision — `crates/abeam/src/workspace.rs` for the
> routing rule, `crates/abeam/src/launch/` for what may be started,
> `crates/abeam/src/ask/` for the second agent. `docs/keymap.md` is the keyboard
> audit, and `docs/status.md` is what has and has not been proven.

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
abeam +codex [args]       # OpenAI Codex CLI; every argument is forwarded
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
abeam knows, `claude`, `copilot` or `codex`, or a preset out of your config
file, matched without regard to case, abeam looks that entry's executables up
on `PATH`; anything else is a program name and means what `abeam bash` used to
mean. Spaces
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

**`abeam claude`, `abeam copilot` and `abeam codex` are refused rather than
reinterpreted**, with exit code 2 and a message naming both readings and both
ways out. That refusal is permanent, not a migration aid, and it is a fixed
lookup in abeam's own table rather than a `PATH` probe: a refusal that depended
on what happened to be installed would accept a command line on your machine
and reject it on a build server.

Codex is a first-class **interactive host**, not a claim that Claude-specific
side channels generalise. `abeam +codex [args]` forwards the line unchanged to
the Codex TUI in the left pty. Ask is unavailable. The readiness probe reads
Claude's session records, so under Codex it deliberately reports `Unknown`:
queue send items cannot drain automatically **or** through `Enter`, and must be
typed in the left pane instead. Background dispatch and its roster are
Claude-only and stay unavailable. This fails closed rather than treating a
neighbouring Claude record in the same repository as Codex's state.

The prerequisite is the official CLI on `PATH` — `npm i -g @openai/codex` —
and a direct `codex` run signed in with ChatGPT or an API key. abeam neither
installs Codex nor owns its credentials; OpenAI's [CLI
setup](https://learn.chatgpt.com/docs/codex/cli) and [authentication
guide](https://learn.chatgpt.com/docs/auth) are authoritative for both.

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
  recently, and the story is in [status](status.md). On Linux both write
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

[preset.openai]
host  = "codex"
view  = "files"
```

`[defaults]` is how every session on the machine opens. A **preset** is a name
behind the sigil that behaves exactly like a built-in agent: `abeam +fleet
--resume` starts `claude agent --resume` with the queue showing,
`ABEAM_AGENT=fleet abeam` does the same, and `+help` lists `fleet` beside
`claude`, `copilot` and `codex`. The `openai` example resolves to the built-in
Codex host: `abeam +openai [args]` starts Codex and forwards those arguments
after any preset `args`. A preset's own `args` go in *front* of what you typed,
because a subcommand is the first word of the line it belongs to — behind them,
`abeam +fleet --resume` would be `claude --resume agent`, which is a different
command in every agent abeam hosts. Its four opening keys override `[defaults]`
field by field, so the preset above moves the view and leaves the rest where the
defaults put them.

`codex` joining the built-in table reserves that name. An existing
`[preset.codex]` is now refused as ambiguous and must be renamed —
`[preset.openai]` above is one possible migration — with callers changed to the
new `+name`.

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
  `codex`, `help` or `version`. It would be a name with two meanings and one of
  them unreachable, with nothing on screen saying which of the two ran. Two presets
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
AltGr.) The shipped defaults of each supported agent are audited in
`docs/keymap.md`, and `crates/abeam/src/keys.rs` pins abeam's side. Codex also
supports custom `tui.keymap` bindings, so a local configuration can assign an
abeam key such as `F8`; literal-next is the recovery path when it does.

| Key | |
| --- | --- |
| `Alt+G` | right pane → git |
| `Alt+E` | right pane → files / markdown (again for the file list) |
| `Alt+S` | right pane → a shell, **and focus it** (again to hand focus back) |
| `F8` | right pane → the queue |
| `F2` | right pane → pty diagnostics, and back to what it displaced |
| `F3` | file reader → light / dark page |
| `F4` / `F5` | move focus left / right |
| `F6` | right pane → **ask**, with nothing attached, **and focus it** (again for what it displaced) |
| `F7` | select rows of the right pane by keyboard, **and focus it** |
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
while the agent still has focus. You need focus to type into the pane — which is
what `Alt+S` and `F6` are for, and the only reason a view key ever moves focus —
or to drive a selection, which is `F7`'s, and `F7` switches no view: it acts on
whatever is already showing. Every other view key leaves focus exactly where it
found it, in both directions.

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
and refused twice over — `Alt+W` is Claude's, and a *view* key spelled `F6`
would be a key nobody groups with `Alt+G` and `Alt+E`. The list is not a peer of
those views anyway; it is how you point one of them somewhere else. (`F6` is
bound now, and to the ask rather than to a view: the ask displaces something and
puts it back, which is `F2`'s shape and not `Alt+G`'s. That argument is about
which set a key joins, and it still holds.)

`?` is the second key in that vocabulary that is not about reading, and the table
above is right to leave it out for exactly the reason it leaves out `w`. In the
document the reader is showing, or in the git view, it opens **ask** — a
second-agent pane where the hosted provider supports it, described under "The
panes" — with the
file you were looking at attached to the first question, and `Esc` puts back the
view it displaced the way `F2` does out of the diagnostics. Those two views and
no others: the file list and the `f` results own every key while they are up, so
`?` is inert in both, which [status](status.md) records as a gap rather than a
decision. A question about the file you are reading is asked from where you are
reading it, so the key is only ever delivered to a focused pane; that is
the whole of the exemption, and it is the same sentence `w` relies on rather than
a second argument that happens to agree.

**`F6` is the same pane with nothing attached, and it is in the table above
because it has to be reachable from the left pane.** `?` answers "about this
file", and the question you have while typing at the agent is usually not about
a file at all — it is about the repository, and there is no pane to press `?` in
without first switching to one you did not want to look at. So `F6` opens the
ask from anywhere, focused, with no context on it, and a second press puts back
what it displaced. It is an F-key rather than an `Alt` letter for the reason
`F2` and `F3` are: every plausible letter is already occupied somewhere in the
supported agents' default maps, while this F-key is clear in the audited
defaults. Showing it is also the only way to take an attached file back **off** — an attachment survives until the
question it rides on has gone, so before this, `?` on the wrong file left you
asking about it or clearing the whole conversation. Nothing is hidden by that:
the row naming the attachment is what disappears.

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

**Copying out of the right pane is abeam's job, and a drag is the whole of it.**
Two things make it abeam's rather than the terminal's. `EnableMouseCapture` takes
the host terminal's own drag-select away for the whole session, and even without
that, a linear drag across a split window is a drag across *both* panes: the two
share every screen row, so what came back would be the agent's text with the
shell's interleaved a column at a time. Only a rectangular selection cuts one
pane out, and which terminals offer one — and behind which modifier — is exactly
the sort of thing abeam cannot promise on somebody else's behalf.

So abeam takes the gesture over and keeps its meaning: **press, drag, let go, and
what was highlighted is on the clipboard.** No key and no mode, because on a
command line highlighting something *is* the request to take it — that is what
every terminal with copy-on-select already assumes, and what the host terminal
would have done here if abeam had not taken its mouse. `Ctrl+C` copies too while
a highlight is up, which is the same rule Windows Terminal applies: with a
selection it copies, without one it belongs to the child. It is the only
`Ctrl`+letter abeam ever takes and it is not in `crate::keys`'s table, because
`global` claims nothing — the state it is reached in is one where every key is
already being swallowed, so it costs the child nothing it was going to get.

A *press* deliberately starts nothing, because the git view, the queue and the
file list all pick a row on a click and a selection that took that gesture would
have taken it from them. It is the movement after the press that selects, and the
release that copies. The one cost is the one the convention carries: a drag over
text replaces what was on the clipboard. A drag over blank rows does not — there
is nothing to write, and silently emptying somebody's clipboard is worse than a
gesture that appears to do nothing.

**`F7` is the same thing for a keyboard**, and the reason it exists is not
symmetry: the mouse belongs to whatever the right pane is running the moment that
program asks for it, so a `lazygit` or a `vim` in the shell view leaves a drag
with nowhere to go. It puts a caret on the pane and the scroll vocabulary above
moves it — `v` anchors, `y` or `Ctrl+C` copies, `Enter` hands the rows to the
agent. Nothing else is new. Auto-copy stops at the keyboard on purpose: a drag
has an end and a caret does not, so copying on every `j` would rewrite the
clipboard on the way to what you actually wanted. While a selection is up
**nothing reaches the pane behind it**, which is not a nicety: that pane can have
a live shell in it, and a key that fell through would be a command typed at a
prompt nobody was looking at.

**What it selects is whole rows of the pane, as they are on screen** — not a
range in the content behind them. That is the one thing about it worth learning,
and everything else follows from it. Six views live in that pane and they are six
different kinds of thing: a terminal grid, wrapped markdown with quote gutters,
a column-aligned status list. A cell-precise selection means something different
in each, and three of them have no coordinate space to draw it in. A row means
the same thing in all six, and a row is what somebody copying a path, a hash, a
stack trace or a test failure is after. It also means the highlight stays put
when the pane scrolls under it, naming whatever is there now — which is the
honest consequence, and it keeps the property that matters: what is highlighted
is always exactly what will be copied, because the text is read at the moment you
press the key.

The **shell view is the one pane that improves on that**, and it is the reason
`Pane` has a `selected_text` at all rather than the app simply reading the frame
back. A terminal grid records which of its rows are continuations of the row
above, so a `cargo` diagnostic drawn over three rows of a 46-column pane comes
back as the one line it was written as. A frame cannot know that — a wrapped row
and a row that happens to be full look identical once drawn — and a path
rejoined with a newline through the middle of it is worse than not copying it.
The other five answer `None` and get what was drawn, which is genuinely all there
is to know about them.

The two destinations are not the same feature wearing two hats. The clipboard is
reached with **OSC 52**, which is the one mechanism that reaches the
clipboard of the machine you are *sitting at* when abeam is running over SSH —
and it costs a feature flag on a dependency abeam already had rather than a
per-platform clipboard stack. It also has no reply, so abeam says what it did and
not what your terminal did with it. `Enter` needs none of that: it is
`send_text`, the same bracketed paste the queue writes, so the rows arrive in the
composer as one insertion and stop there. **It never submits**, for the reason
the queue's `Enter` is a separate pass: rows off a screen are not a message
somebody wrote, and the one who decides they are is the one at the keyboard.

`Ctrl+\` exists so abeam can never permanently shadow a binding of the agent you
are typing at. If a future agent release or Codex custom keymap binds `Alt+G`,
`Ctrl+\` then `Alt+G` still reaches it.

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
rather than a reset.

Inside a repository the list shows dot-named files — `.claude`, `.github`,
`.gitignore` — because that is where a good deal of the work lives, and because
gitignore is there to keep the rest out. Started somewhere that is *not* a
repository, gitignore is inert and nothing else would keep `.ssh` off the list,
so they stay hidden exactly as before. The same rule governs the startup walk,
which means `.claude/*.md` and `.github/**/*.md` are in `Tab`'s recency list
now: on a fresh clone, where every file shares one checkout timestamp and
"newest" is whatever order the filesystem hands back, the pane can open on a
workflow file, an issue template or one of `.claude/commands/*.md`. The first
write of any kind sorts it out.

`/` finds a file by name anywhere under the root, matched as a subsequence over
its path and ranked so a hit in the file name beats one in a directory name; `Esc` cancels the find and stays put, because being thrown out
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
re-derive what the first one had in its hand — so it inherits that walk's caps
and its one exclusion, reads at most the first half-megabyte of any file, keeps
at most a screenful of matches from each, and stops at a total. The exclusion is
that the index holds the files of *this* workspace, so a directory that is a
worktree of another repository is not in it — while the file list will happily
walk you into one. Standing in such a directory, the list's border says
`unindexed`, for the same reason the `+` and `· 18 files cut` exist: a search
that came back `0 matches` there would be answering about the corpus rather than
about the tree, and a pane that is merely limited must never look like a pane
that is right. Every one of the caps is visible when it is hit too: a count that
is a prefix of the truth is written `137+` rather than `137`, and the per-file cap is named outright as `· 18 files cut`, because it is
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

**ask** — `?` from the document the reader has open or from the git view, `F6`
from anywhere, and a second copy of your agent in the right pane which **may
read and may not write**. The gap it fills is narrow and constant: you are
reading a file, or a diff, and a question comes up that is *about* what is on
screen — what does this call do, where is this written, is this the only caller
— and every way of answering it costs the conversation in the left pane. You
interrupt a turn, or you queue the question and wait, or you open a second
terminal. This is the fourth way. `F6` is the same pane for the question that is
not about a file: it attaches nothing, it is reachable while you are typing at
the agent, and it is the only way to take an attachment back off.

**Nobody has ever asked it anything.** Everything in this section follows from
the code and from tests that drive the pane against shims and strings, with no
`claude` anywhere near them; no human has typed a question into it and read the
answer back. So read the present tense here as what abeam does rather than as
what somebody has watched happen — and read what comes out of it as a model's
answer, which can be fluent, specific and wrong. [Status](status.md) says
both of those again, with the rest of what this pane costs.

**Ask supports Claude and Copilot, and no other host.** The two are not the same
pane under the skin. Everything from here to "`Enter` never runs anything"
describes the Claude one, which is the one that has been probed; "Asking Copilot
instead" below is the other, and is a shorter section because there is less that
can honestly be said about it. abeam will not quietly start a Claude for a Codex
session: under Codex, as under any unsupported program, the pane says Ask is
unavailable and names the providers it can drive.

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
long-lived child rather than one process per question. (Under Copilot the same
sentence is true of the *conversation* and false of the child; see below.) `Esc`
puts the view back
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

### Asking Copilot instead

**Everything in this subsection is documentation-derived and has never been
run.** That is the same footing as the rest of abeam's Copilot support and it
matters more here, because what is being described is the shape of a read-only
guarantee. GitHub's published flags are the whole of the evidence;
`crates/abeam/src/ask/copilot.rs` names, beside each choice, what would break
first if a flag turns out not to mean what the documentation says.

Copilot CLI publishes no streaming-JSON print mode — no `--output-format`, no
`--input-format`, no `--session-id`, no `--tools`. So the pane is driven a
different way, and four things follow that a reader of the Claude half should
not assume across:

- **A question is a process.** `copilot -p` answers one question and exits, so
  abeam runs one child per turn. What carries the conversation is a *name*: the
  first question creates a session with `--name abeam-ask-<id>` and every
  question after it resumes that session. `Ctrl+L` starts a new name.
- **The read-only claim is a denylist rather than an allowlist**, and that is
  weaker. Claude's `--tools` means the other tools do not exist for that session;
  Copilot has no equivalent, so abeam passes `--deny-tool` for `shell`, `write`,
  `edit`, `web_fetch` and `web_search`, never passes `--allow-all-tools`, and
  passes `--no-ask-user` so that a tool needing approval cannot get one. GitHub
  documents deny as taking precedence over both allow flags. A tool kind that
  ships next month under a sixth name is not covered by that line, and saying so
  is the point of putting it here.
- **The pane cannot show you what the child was given.** The Claude pane's
  standing rule is that the row along the bottom is the child's own answer and
  never abeam's intention — Copilot sends no line announcing its tools, so that
  row says `copilot · no tool list to show` rather than reprinting the denylist
  as though something had confirmed it. There is no cost or duration on the wire
  either, so a finished turn is unlabelled rather than labelled with a guess.
- **The question is on the command line**, where Claude's goes down a pipe as
  JSON. `-p` is the documented programmatic switch and abeam takes it, which
  costs exactly one case: a multi-line question on a Windows npm install, where
  the `.cmd` runs through `cmd.exe` and a newline cannot be put on that command
  line in any form. That is refused with a sentence naming the way through
  rather than mangled.

Two things abeam cannot close on this route, named rather than left to be
discovered: a repository's own Copilot instructions still load, and so do any
MCP servers you have configured — `--strict-mcp-config` and `--setting-sources`
are Claude's, and Copilot publishes nothing equivalent. And the named sessions
are persisted where your own are, so abeam's conversations turn up in your
`copilot --resume` list afterwards under `abeam-ask-`. The pane's opening screen
says all of this too, because somebody leaning on the read-only promise should
learn which version of it they have from the pane rather than from here.

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

The same argument decides what the **file window** does about them, and it
decides it two different ways on purpose. The index behind `/` and `f` means
"the files of this workspace", so it stops at a nested worktree exactly as
`git status` does. The file list means "what is on the disk here", and pruning
that would take away a place you can see and are walking to — so it descends,
and marks the border `unindexed` while you are standing there. The two
deliberately disagree, and `w` is how you stop having to care: move the window
to that worktree and its files are the index.

Recognising one is git's own marker rather than a guess about `.claude`. A
worktree root carries a `.git` **file** whose `gitdir:` ends `worktrees/<id>`;
a submodule's ends `modules/<path>` and stays in the index, because
`git worktree list` never names one and the router hands its writes to this
workspace. It is the penultimate component of that path and nothing else — the
obvious version, "a `worktrees` component somewhere", reads a path the user
chose, and `git worktree add` writes an absolute one, so a checkout living under
any directory called `modules` would have turned the rule off silently.

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
