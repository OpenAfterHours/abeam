# What is done, what is not, and what nobody has watched work

> Moved out of `README.md` so that document could be something a new user reads
> start to finish. Nothing here was rewritten on the way: this is the same text,
> and it is the honest half of the project. If you are deciding whether to trust
> abeam with real work, this is the page to read.

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
every push to `main` and on every pull request, which is the only reason the
second one is a claim at all: a suite
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

## Status

Working, and used. Not finished.

**Done.** The pty host layer, proven by a spike that ran a real Claude session
against it on 2026-08-01. All seven right-hand views, the file list, the
rendered/source toggle — which now has something to toggle on a documented
`.rs` or `.py`, whose doc comments and docstrings are rendered where they stand
— the outline behind `o` and the breadcrumb it puts in the title, the watcher
driving what it should, and the three
searches under the reader — a file by its name, a phrase on the page, a phrase
in every file under the root except directories that are worktrees of another
repository, which the file list will still walk you into and marks `unindexed`
on its border while you stand there. Focus, zoom, help, the diagnostics view,
and the literal-next escape hatch. Agent selection for Claude, Copilot and Codex and the
launcher underneath it. The
Unix port, in the sense that the whole workspace builds, tests and lints clean
for both `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` — see
"Platforms" for the sense in which it is not done. Mermaid flowcharts and
sequence diagrams, drawn rather than shown as source. Copying rows out of the
right pane: a drag copies when you let go, `Ctrl+C` copies whatever is
highlighted, `F7` is the keyboard's way in, and `Enter` puts the rows in the
agent's composer without sending them — with the limits listed at the foot of
this document. The Windows test suite, and `clippy --all-targets` clean on both.

The seventh view is the **scratch pad** on `F9`: a markdown pad per workspace
that opens on its source with a caret in it, turns over to the rendering on
`Alt+T` — from either `Alt` key — and writes itself to
`%APPDATA%\abeam\scratch\` on Windows and
`$XDG_DATA_HOME/abeam/scratch/` — falling back to `~/.local/share` — on Linux,
keyed by the workspace root. It is the first file abeam has ever written, and it
is written through a temporary file and a rename so that a crash mid-save leaves
the previous pad rather than half of one. What is done is the buffer, the two
forms, the persistence and the tests over all three; what is not done is
immediately below, and it is a longer list than most things in this document
arrive with.

Codex support means the interactive TUI in the left pty. The official Windows
Codex 0.149.0 binary was hosted through abeam with an isolated `CODEX_HOME`:
the welcome/sign-in UI rendered, Down-arrow navigation worked, a 120×40 →
100×32 outer resize completed with the UI still navigable, and the two-step
`Alt+Q` quit completed. No account was connected and no prompt was submitted.

One configuration name changes meaning on upgrade: `codex` is now built in, so
an existing `[preset.codex]` is refused and must be renamed, with its callers
changed to the new `+name`. A preset may still use `host = "codex"`; that
resolves to the built-in provider and keeps the capability boundaries described
above.

Two of those changed Windows behaviour on the way past, and both are worth
seeing before you upgrade rather than after.

**AltGr is Ctrl+Alt, and abeam now says so in one place instead of four.**
On a UK, Irish or continental layout the right-hand `Alt` key is AltGr, and
Windows reports it by setting the control bit as well — so half the keyboard
delivered every `Alt` binding with CONTROL set. `keys::global` had always
ignored that bit and three other places had not, which is why `Alt+S` reached
the shell from either key while `Alt+T` turned the scratch pad over from the
left one alone. `keys::alt_chord` is the single answer now, and `altgr_is_alt`
walks the whole table to keep it single.

Two more fell out of the same fact. The pad, the ask and the queue all guarded
typing with `!ctrl && !alt`, which is a guard against AltGr and so against every
character behind it — `€` on a UK layout, `@` and `€` on a German one — typed
and silently dropped; `keys::is_text` is their shared answer. And literal-next
matched `Ctrl+\` on the control bit alone, so on the layouts that put `\` behind
AltGr, typing a backslash armed it and sent the *next* keystroke to the agent
raw. It reads `ctrl && !alt` now, and `F12` is still the alias on those layouts.

None of that reaches a terminal that takes `Alt`+letter for its own menus before
abeam sees it, or one that reports `Alt` as an `Esc` prefix. `cargo run -p abeam
--example keyprobe` tells the three cases apart, names which `Alt` key arrived,
and names the binding each event resolves to; it covers all twenty globals and
the pad's `Alt+T`.

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
in the right directory and puts its answer on screen. Four paths are pinned that
way today: type a command in the shell and read its output; reach a file nothing
pointed the pane at, by `Alt+E` `Alt+E` `/`; select rows of the shell view with
`F7` and copy them, which is the only place `ESC [ 1 8 ~` is proved to come back
out of ConPTY as the function key it names *and* the only place the mode's
promise — that nothing reaches the child while a caret is up — is asked in front
of a real prompt; and a copy of a real shell planted in the repository under the
name abeam is about to look for, which abeam must refuse to run. That last one is
a test about an attack rather than a feature, and
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

**More than one agent in the window is built, and no human has ever used it.**
That sentence is the whole entry and the rest of this paragraph is detail. `a`
on a row of the worktree list (`Alt+G`, `w`) starts another agent in that
checkout; `F4` pressed again moves along them; they are stacked vertically, one
title row each for the ones there is no room to draw whole; the queue's `Send`
items carry the pane they were written for and are typed there and nowhere else;
`x` twice at a pane whose child has exited closes it, and `x` twice on a
worktree row ends a live one. The agent abeam was started with is still the
session: its exit is abeam's exit code, and it cannot be closed. `docs/multi-agent.md`
is the design and the record of what each phase cost.

What that is built on is tests — a few hundred of them, spawning real ptys and
drawing real frames, including a mutation audit that broke each new rule on
purpose to check something went red. What it is not built on is use. Nobody has
pressed `a` in a real terminal, watched two agents work at once, queued a prompt
for one of them and seen it arrive, or ended a running agent with `x` `x` and
watched the process go. Every number in the paragraph below about how it
degrades is arithmetic, not a measurement.

Four specific things to expect, in the order they are likely to bite.

- **The rows are tight, and the arithmetic is unmeasured.** A whole pane is
  `MIN_AGENT_ROWS` — twelve — plus its border, so two agents want 28 rows and
  three want 42. A 24-row terminal draws one agent and a title row whatever you
  do. Twelve is an argument (five rows of permanent furniture, seven of
  transcript, which is what a permission prompt needs to be on screen at all)
  rather than a number anybody has watched an agent use, and it is the first
  thing to change if a real session says otherwise.
- **A collapsed pane is not resized, on purpose, and the consequence is
  visible.** Resizing a live agent to one row would reflow its whole transcript
  and, on some agents, truncate the scrollback that reflow lands in — so a
  collapsed pane goes on believing it has the size it last had, and the first
  frame that draws it whole again is drawn at a size its child has not caught up
  with. That is one odd-looking frame per pane per return, by design, and it has
  never been seen by a person.
- **The queue's target is decided when you write the item and cannot be
  changed.** Write three prompts for the wrong pane and you write them again.
  This is deliberate — a key that re-points a prompt is a key that can
  misdeliver one — and it is the gap most likely to be the first complaint.
- **Two agents in one checkout are indistinguishable.** The worktree list counts
  them and cannot name them; the queue's rows call both by the same worktree
  label; and `x` on that row refuses rather than guessing, pointing at `F4`.
  A pane has no name a person chose, which is the missing piece.

Two costs are worth knowing before starting a fourth agent. Each pane is a whole
agent — its memory and its quota are the agent's, not abeam's — and each holds a
reader thread and a `vt100::Parser` with 5,000 lines of scrollback. Whether a
collapsed pane should keep 5,000 is a question for a measurement rather than an
opinion, and nobody has made one.

**Not done, and known.**

- **Two agents that are neither the session's can never be read together.** The
  stack hands out rects in list order and panes never swap places, so with three
  agents and room for two, the two you get are `agents[0]` and whichever has the
  keys — moving the cursor to the third collapses the second on the way past.
  Comparing two panes neither of which is the session's would need a second
  cursor or a pinned pane, and either is a feature rather than a fix. There is
  also no "show me only this agent": `Alt+Z` hides the *right* pane, which buys
  the left column columns and not one extra row, and a second zoom was declined
  because its whole effect would be to delete the roster of collapsed title rows
  that the feature exists to keep.
- **Ending a live agent is only reachable through the worktree list**, which is
  `Alt+G`, `w`, find the row, `x`, `x`. That is deliberate — `x` at the pane
  itself is that child's letter, and abeam may not take a key a live agent might
  bind — but it means the gesture is not discoverable from the pane a reader is
  looking at while wondering how to get rid of it. The `F1` overlay and
  `docs/keymap.md` are the only signposts, and neither is where the question is
  asked. If that turns out to be the common complaint, the honest fix is a
  sentence in the agent pane's own border rather than a key.
- **Nobody has typed into the scratch pad by hand, on either platform.** That is
  the sentence to read before trusting it with anything you would mind losing,
  and it is the same sentence "Platforms" says about Linux, said about a feature
  instead of an operating system. The pad has tests over the buffer, the two
  forms, the caret arithmetic and the file on disk; what none of them can do is
  press `F9` in a real terminal and find out whether the key arrives, whether
  the caret lands where a hand expects it, or whether the pad that comes back
  after a restart is the one that was typed. Until somebody does that, this is a
  feature that passes its tests.
- **There is no undo, no selection inside the pad, and no word motion.** It is a
  caret, the four arrows, `Home`, `End`, `Backspace`, `Delete` and typing —
  which is the whole editor, and less than any text field a user has met this
  decade. `Ctrl+Z` in particular does nothing, and the failure that buys is the
  ordinary one: a paste over the wrong place, or a `Backspace` held down a beat
  too long, is not recoverable and the file on disk will agree with the mistake
  two seconds later. It is deliberate to the extent that the first version of a
  text buffer should not also be the first version of an undo stack; it is not
  deliberate in the sense of being finished.
- **The pad holds 64 KiB, and the number belongs to the syntax highlighter.**
  Past that size syntect gives up and returns plain text, so a pad allowed to
  grow beyond it would go grey one keystroke after it was fine with nothing on
  screen explaining why — which is why what can be typed and what can be drawn
  in colour are one number rather than two. At the cap, typing and pasting are
  refused rather than truncated, and a paste that will not fit is refused whole:
  half a pasted paragraph is worse than none of it, because the user has to
  notice the cut and the place it happened is off the bottom of a pane they had
  already stopped looking at.
- **Saving is debounced by two seconds, so up to two seconds of typing can be
  lost.** The pad is written when the text has been still that long, and on
  quit; a machine that loses power between the last keystroke and the next save
  loses whatever was typed in that window. Writing on every keystroke was the
  alternative and it costs a file write per character on a pane somebody is
  typing prose into. Two seconds is a judgement about which of those hurts
  more, and it is a judgement rather than a guarantee.
- **A pad file already larger than the cap is readable and not editable.** The
  read stops at 64 KiB, the pane says so, and it then refuses to save for the
  rest of the session — because the buffer holds a prefix, and one ordinary
  autosave of a prefix over the whole file deletes everything past the cut. From
  the outside that deletion would look exactly like the pad working. Nothing
  abeam writes can reach that state, so the file has to have been grown by
  something else; the way out is to move it aside with an editor that can hold
  it. Refusing costs the user the feature for one session, and the other way
  costs them the file.
- **One of the persistence tests has a Linux expected value that was computed
  rather than run.** `a_root_is_written_down_under_the_same_name_it_was_last_year`
  freezes the name a workspace root is filed under, per platform, because a key
  that changed under somebody would leave every pad they have written on disk
  and unreachable at once. The Windows value was produced by running it here.
  The Unix one, `forge-94b2adccc4d44b8a`, was worked out from the same FNV-1a
  over the same components and has only ever been executed by CI — which does
  run it, on every push to `main` and on every pull request, and would fail if
  the arithmetic were wrong. It is
  listed because "computed and then confirmed by a machine somewhere else" is a
  different claim from "someone watched it pass", and this document is the place
  that keeps those apart.
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
  allowed to do. It is why the *flowchart* outline exists and where even that
  stops — named that way here because `o` in the files view now opens an
  outline of a different kind, and this bullet is about neither of the two
  things a reader would guess from the bare word.

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
  migration line in [design](design.md) is. The same shape, reversed, is the other
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
- **A nested worktree that `git worktree list` never names is pruned from the
  index and routed to this workspace anyway, permanently.** The file window
  keeps a worktree of another repository out of `/`, `f` and `Tab` by reading
  the `.git` file git left in it, which needs no discovery and so cannot be ten
  seconds late. `workspace::owner` routes by the discovered list. For a worktree
  of *this* repository the two agree as soon as the poll catches up. For one
  belonging to a **different** repository — somebody dropped a checkout of
  another project inside the root — git never names it here, so the two never
  agree: the index excludes it for ever while the router hands its writes to
  this workspace, and `follow` can put a document on screen that `/` says does
  not exist. That is the ten-second window above made permanent for a rare case.
  Worth writing down rather than discovering: the fix is the same one the border
  already points at, which is `w`.
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
  fires, and `Enter` cannot send the item either. The queue pane goes on saying
  the state is unknown; the way through is to type the item in the left pane.
  That is the direction this module fails in on purpose.
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
  that is the single resolution at startup described in [design](design.md), which
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
- **The Copilot half of the ask is weaker than that again: it has never been run
  at all.** The bullet above is about a pane no human has driven; this is about a
  pane no *process* has driven either. Not one `copilot` has been started by
  abeam, by hand or otherwise, because there is no `copilot` on the machine this
  was written on — every flag on that command line comes from GitHub's published
  documentation, and the tests hold the argument list still and drive the session
  against a shell script pretending to be Copilot. What that leaves unproven is
  named where each flag is chosen, and three are worth repeating here because
  they are the ones that would fail loudest. `-p` is assumed to be programmatic
  mode rather than a prompt typed into an interactive UI. `--name` and
  `--resume=` are assumed to be how a session is created and picked up, and if
  they are not, every answer will have forgotten the question before it while the
  pane goes on calling it a conversation. And `--deny-tool` is assumed to mean
  what GitHub says it means, which is the assumption the read-only claim rests
  on — the pane says `copilot · no tool list to show` rather than pretending
  otherwise, but a reader who wants a *guaranteed* read-only second agent should
  use the Claude one until somebody has watched this one refuse a write.
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
- **The outline cannot be filtered, and a big file needs it to be.** `o` lists a
  document's headings or a source file's definitions, and on this repository's
  own `crates/abeam/src/panes/viewer.rs` that is 223 entries of which 187 sit at
  one level, drawn at 46 columns where a name is cut at about 42 cells — six
  consecutive rows there begin `fn a_doc`. Paging across it is five keystrokes;
  telling two rows apart is the part that does not work. The three boxes this
  pane already has are the obvious answer and the reason it does not have a
  fourth is written down rather than assumed: every one of them adds a stage to
  `Esc`, and the outline is currently the one layer with a single unconditional
  way out. That is a real trade and it may well be the wrong side of it; what is
  not defensible is leaving the gap unwritten, which is why it is here.

- **`?` is inert in the file list, in the `f` results and in the outline.** It
  opens the ask from
  the document the reader is showing and from the git view, and nowhere else —
  those three each own every key while they are up, because a pane
  cannot hand the same key to two vocabularies and hope, and none of them has
  an arm for `?`. The outline is the newest of the three and the least excusable
  of them, because unlike the other two it is a layer over the very document
  `?` would have asked about. So `Alt+E` `Alt+E` reaches a view where the key the F1 overlay
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

  **Under Copilot it is once per *question*, and that is a stutter rather than a
  hitch.** A `copilot -p` exits when it has answered, so every turn spawns a
  child on the thread that draws — the one saving grace being that it is paid on
  a keystroke the reader made rather than on a bare pass of the loop. It has not
  been measured, because it has not been run. Moving the spawn onto a thread of
  its own is the fix for both, and is the same fix: what stopped it being taken
  is that a start which fails would then be reported a frame or two later, with
  nothing on screen connecting it to the `Enter` that caused it.
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
- **Codex support stops at the interactive pty boundary.** Install the official
  CLI with `npm i -g @openai/codex`, then run `codex` directly and authenticate
  with ChatGPT or an API key before `abeam +codex [args]`. abeam does not own or
  alter that authentication. Ask is unavailable; Claude's readiness record is
  inapplicable, so queue send items are blocked both automatically and on
  `Enter` and must be typed in the left pane; background dispatch is
  Claude-only. The Windows 0.149.0 sign-in screen was exercised as recorded
  above, but authenticated composer, approval, pager and agent-session modes
  remain untested. No Codex path has been run on Linux.

  Codex's shipped 0.149.0 defaults collided with the former `Alt+A` queue key,
  so abeam yielded it and the queue is now `F8`. Codex can bind `F8` through a
  custom `tui.keymap`; abeam does not parse that configuration, and
  literal-next (`Ctrl+\` or `F12`) is the recovery path. `docs/keymap.md` has the
  provenance and the remaining live-audit checklist.
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

  The ask pane learned to drive Copilot after that was written, which widens
  this bullet rather than narrowing it: there is now a second Copilot command
  line abeam has never watched run, and it is the one carrying a read-only
  claim. The bullet above about the ask has the detail.
- **The Copilot keymap audit is documentation- and source-derived**, and that is
  weaker evidence than the Claude half of `docs/keymap.md` rests on. Those
  bindings came out of strings extracted from the installed binary. Copilot's
  come from GitHub's published shortcut tables, about 150 changelog entries and
  Ink's source — which is exactly the kind of audit that would have cleared
  `Alt+F` in Claude, and `Alt+F` was bound. The document says so at length, and
  lists the two steps that would upgrade it.
- **Nine of abeam's `Alt` bindings are *probable* no-ops in Copilot, not
  verified ones, and six of the nine are worse than that.** The nine are every
  `Alt` key abeam claims **as a global**: `Alt+G`, `Alt+E`, `Alt+S`, `Alt+Q`,
  `Alt+Z`, `Alt+J`, `Alt+K`, `Alt+PageUp` and `Alt+PageDown`. `Alt+T` is a
  tenth `Alt` key abeam reads and is deliberately outside that list: the
  scratch pad claims it while that pane has focus, `keys::global` declines it,
  and no agent is listening for a key delivered to a focused pane. The
  distinction is the one `docs/keymap.md` argues at length; the reason to keep
  it out of this bullet rather than quietly counting it is that "every `Alt`
  key abeam claims" is a wider phrase than the audit earned. Each was looked for in
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
  and the invariant holds; the day it is bound, abeam has to move. Re-checked in
  2.1.251 on 2026-08-29, which is one build later than the audit and no
  different: still an action, still a fallback string, still no binding.
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
  runs the suite there on every push to `main` and on every pull request; the
  six pass criteria above have been
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
- **abeam's keybindings are not configurable**, and neither are the split ratio
  nor either drawing interval: those are constants in the source. There is a config
  file now — see "Configuration" — and it holds presets and the four things a
  session opens with, which is where the reader's light/dark choice went. It
  says nothing about keys. Claude's and Codex's bindings are user-configurable
  and abeam's should be too before anyone else uses it. Copilot's are not, which
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
  at all. It is reachable in an undocumented source file by dragging the pane
  narrower, and no width fixes it in rendered markdown — or in a rendered
  docstring, which puts a `.rs` and a `.py` in the same position — where the
  source syntax is not on the page at all. Rendering also makes the disagreement
  run the other way, which it did not before documentation was drawn as prose: a
  docstring whose two lines draw as one row holds a phrase for `/` that `f`
  cannot find in the file. A drawn mermaid diagram is the sharpest case of that same
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
- **`EnableMouseCapture` disables your terminal's native text selection**, which
  is why abeam has one of its own — a drag in the right pane, which copies when
  you let go, or `F7` for the same thing by keyboard. Shift+drag is still your
  terminal's way back to *its* selection and which terminals honour that still
  varies; what is worth knowing about abeam's is where it stops. **A drag over
  text replaces what was on your clipboard**, which is what copy-on-select costs
  everywhere it exists — a drag over blank rows writes nothing, so a stray
  gesture cannot silently empty it, but a stray gesture over output can still
  cost you what you copied a minute ago. It
  selects **whole rows**, never a column range, so a hash in the middle of a git
  row comes with the row around it. It selects only what is **on screen**, so
  anything above the top of the pane has to be scrolled to first — and because
  the rows it names are rows of the *pane*, scrolling under a selection leaves
  the highlight where it is rather than following the text. It is the **right
  pane only**: there is no way to select what the agent has drawn on the left.
  And `y` is OSC 52, which the host terminal has to honour — Windows Terminal,
  VS Code, iTerm2, kitty, WezTerm and Alacritty do, a legacy `conhost` does not,
  and tmux wants `set -g set-clipboard on`. There is no reply to such a write, so
  abeam reports what it did rather than what your terminal did with it. `Enter`,
  which puts the rows in the agent's composer unsent, needs none of that and is
  the route the feature was built for.
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
- **The `Alt` keys abeam claims are free in the audited shipped defaults** —
  verified against one Claude build (2026-07-25) and Codex 0.149.0, and merely
  not refuted by Copilot's published tables, which is one claim resting on
  different strengths of evidence. Codex custom keymaps are outside it. The
  `Alt` *namespace* is not free on Copilot's side, and saying so is the whole
  finding this change rests on: those tables declare `Alt+←`/`Alt+→` as
  word-motion and `Alt+Enter` as newline, and the changelog adds an undeclared
  `Alt+D`. abeam binds none of the three, so what is true is the narrower thing
  — free where abeam is standing, not free. No audit is a promise about the
  next release of an agent, and gaining an agent can retire a key that
  was safe while there was only one, as `Alt+←`/`Alt+→` has already
  demonstrated. `Ctrl+\` is the mitigation.
- **The Claude build that audit was read out of is no longer installed.** It was
  2.1.220; the file at that path on 2026-08-29 is 2.1.251, a different binary,
  and only the function-key half of the inventory has been re-derived from it —
  which is the half `F9` needed and the half that has never been wrong. The
  letters are the half that has been: `Alt+F` was found in a binary and not in
  any published table. So every `Alt` row in `docs/keymap.md` is now a claim
  about a build the user is not running. That document's provenance block exists
  to make exactly this visible rather than to prevent it, and this is the first
  time it has caught anything.

  **Repeating the extraction is work to schedule, and here is the number that
  argues for it.** 2.1.251 declares 143 keyboard actions; 113 have a default key
  and **thirty have none at all**, `app:toggleTerminal` — the one the `Alt+J`
  bullet above is about — among them. The `Alt+J` warning is therefore one
  instance of a class thirty wide, in a product that ships weekly, and thirteen
  of the thirty are a `strip:` family `docs/keymap.md` has never heard of. An
  action with no default key is not necessarily unreachable, so thirty is the
  size of the surface rather than a forecast; it is still the best available
  answer to "how urgent is this really".
- **`F9` has never been pressed in a real terminal.** `F10` and `F11` were
  passed over for the pad because `F11` is fullscreen in Windows Terminal and
  most other emulators and `F10` activates the menu bar in several, so neither
  reliably reaches an application at all — and that is a fact about terminals
  rather than about agents, which means no amount of reading an agent's source
  establishes that `F9` is any better. `crates/abeam/examples/keyprobe.rs` is
  what would, and it has been run against Windows consoles only and never for
  this key. Run it in the terminal you launch abeam from before assuming the pad
  opens there.

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
