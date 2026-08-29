# The first frame

Four proposals for what abeam draws in the second before the agent speaks, and
what each one costs. Nothing here is implemented; this document exists to be
argued with and for three of the four to be closed.

## What the first frame is today

`main` resolves the agent to an absolute path, resolves the repository root to
its one true spelling, reads the config file, spawns the pty — and then throws
all of it away. The first frame is a border with the agent's name in it and
nothing inside.

The right pane is doing better. The git pane asks git a question on
construction, and the file viewer opens the newest markdown under the root, so
by the time the window appears the right half has something in it. The left
half, which is the half a person is looking at, is blank until Node has finished
starting — between one and four seconds, on the machines this has been watched
on.

That gap is not a bug and none of what follows is a fix for one. It is the only
unclaimed space abeam has, and there are four quite different things worth
putting in it.

## What any of these must not break

Every proposal below was written against this list, and where one strains a rule
it says so rather than hoping.

1. **It never eats a keystroke.** The agent is what the window is for. A splash
   that swallows the first key of `abeam -p "fix the tests"` has broken a
   scripted session for a picture.
2. **It is drawn from `ui()`, or it is not drawn.** One layout calculation per
   frame is the rule `crate::layout` exists to hold. A splash with its own idea
   of where the panes are is a second calculation, and a resize is where those
   two disagree.
3. **Nothing new goes in the repository.** `crate::config` refuses to read out
   of the workspace and `panes::pad::store` refuses to write into it. Anything
   below that needs to remember something remembers it in the profile.
4. **It survives 40 columns.** Below `MIN_SPLIT_COLS` the right pane is gone
   entirely, and the window can be narrower than any mockup here. Each proposal
   needs a degradation rather than a clipping.
5. **Every line on it is true.** If git is not on `PATH`, the row says so. A
   startup screen that quietly omits what went wrong is worse than no startup
   screen, because it was on screen at the one moment the user could still have
   fixed it.

## A — the boot log

Put the launch decision on screen, in the left pane, and let the agent's first
byte erase it.

```
┌ claude ──────────────────────────────────────────────┐┌ git · main ↑2 · 12 changed ──────┐
│                                                      ││  Staged (1)            +40 -6    │
│  abeam 0.8.1                                         ││    M crates/abeam/src/app.rs     │
│                                                      ││  Changed (1)            +7 -0    │
│  hosting    claude                                   ││    M docs/design.md              │
│             ~/.local/bin/claude                      ││  Untracked (1)                   │
│  in         ~/src/abeam    main ↑2                   ││    ? notes/                      │
│  config     ~/.config/abeam/abeam.toml               ││                                  │
│  watching   the repository root                      ││  Recent                          │
│  pty        52 × 12                                  ││    a1b2c3d  2m   parser skeleton │
│                                                      ││    9f0e1d2  1h   queue: arm sends│
│  F1  every key    F2  what the pty is doing          ││                                  │
│                                                      ││                                  │
│  ▍ starting…                                         ││                                  │
│                                                      ││                                  │
└──────────────────────────────────────────────────────┘└──────────────────────────────────┘
```

**Why.** Every row is something abeam worked out and then never told anybody.
`launch::resolve` spends four hundred lines deciding which `claude` is the real
one; `paths::resolve_root` picks the spelling of the root that the watcher, the
git pane and the readiness probe are all held to; `config::load` knows whether a
file was found and where it looked. Today the only way to see any of that is
`F2`, and by then you are debugging.

It also closes a papercut the README already documents: an `ABEAM_AGENT`
exported into a dotfile years ago silently redirects every command line that
does not lead with a `+`. The left border says `claude` either way. The boot log
says which *file*, which is the half a border cannot fit.

**How it disappears.** No timer and no state machine. The pane draws the log
whenever `Diagnostics::bytes_read == 0`, which is exactly the condition under
which the pty screen is blank anyway. An agent that prints in forty milliseconds
means nobody ever sees it; an agent that hangs for thirty seconds leaves it up,
which is the case where the config path and the resolved binary are what you
want in front of you.

**Cost.** The least of the four. No config key, no new view, no persisted file,
no key to audit against three agents' keymaps. The work is a render branch in
`panes::terminal` and a struct of facts assembled in `main` and handed to it.
Under 40 columns the labels drop and the values stay.

## B — the curtain

A centred card over the whole window: wordmark, version, and the six keys worth
knowing.

```
┌ claude ──────────────────────────────────────────────┐┌ git · main ↑2 · 12 changed ──────┐
│                   ┌ abeam 0.8.1 ─────────────────────────────────────┐         +40 -6    │
│                   │    ▄▀█ █▄▄ █▀▀ ▄▀█ █▀▄▀█                         │eam/src/app.rs     │
│                   │    █▀█ █▄█ ██▄ █▀█ █ ▀ █                         │          +7 -0    │
│                   │                                                  │gn.md              │
│                   │    one window for an AI coding session           │                   │
│                   │                                                  │                   │
│                   │    Alt+G  git       F8  queue                    │                   │
│                   │    Alt+E  files     F9  pad                      │                   │
│                   │    Alt+S  shell     F1  every key                │   parser skeleton │
│                   │                                                  │   queue: arm sends│
│                   │    claude · ~/src/abeam · main ↑2                │                   │
│                   │                                                  │                   │
│                   │    ▍ waking claude…     any key dismisses        │                   │
│                   └──────────────────────────────────────────────────┘                   │
└──────────────────────────────────────────────────────┘└──────────────────────────────────┘
```

**Why.** The thing a new user does not know is that the right pane exists at
all, let alone that it is seven views. Nothing in A tells them. This does, in
the one moment they are certain to be looking at the screen, and it reuses
machinery that is already here: `help_overlay` is this widget with a different
list in it, centred with `Flex::Center` and cleared underneath.

**Where it strains.** abeam's stated rule is that the right pane never takes
focus and never switches itself. A curtain over the agent breaks the spirit of
that on frame one, and it has to be built so that it breaks nothing else: the
key that dismisses it is *forwarded* rather than consumed, or the first
character of the first prompt is gone. And it must not appear at all for
`abeam -p`, which is a one-shot nobody is watching.

The wordmark is five block-drawn characters wide at its narrowest. At 40 columns
the card becomes the three lines that matter: name, version, `F1`.

**Cost.** Small to build, and the most rules to get right. It is also the one
paid for on every launch forever, by a user who learned the six keys in week
one — which is the argument D exists to make.

## C — the start view

Not a splash. An eighth right-hand view that the session opens on, and that you
can come back to at eleven in the morning.

```
┌ claude ──────────────────────────────────────────────┐┌ start · abeam ───────────────────┐
│                                                      ││  abeam 0.8.1 · claude            │
│                                                      ││                                  │
│                                                      ││  ~/src/abeam                     │
│                                                      ││    main ↑2 · 12 changed          │
│                                                      ││    3 worktrees              w    │
│                                                      ││                                  │
│                                                      ││  Waiting for you                 │
│                                                      ││    queue   2 items         F8    │
│                                                      ││    pad     "the debounce…"  F9   │
│                                                      ││    newest  docs/status.md        │
│                                                      ││                                  │
│                                                      ││  F1 keys · Alt+H back here       │
│                                                      ││                                  │
│                                                      ││                                  │
└──────────────────────────────────────────────────────┘└──────────────────────────────────┘
```

**Why.** It never covers the agent, so rule 1 is not in play, and it never has
to decide when to go away — which is the hardest question the other three
answer. Everything on it is already gathered: the branch and the counts are the
git pane's own report, the worktrees are `workspace::discover`, the queue knows
its depth, the pad has already been loaded from the profile, and the newest
markdown is the file the viewer was about to open anyway.

It also answers a question the current opening cannot. On startup the file
viewer shows the newest markdown under the root, which is a fine answer on day
two hundred and a strange one on day one.

**Where it strains.** It needs a key, and `crate::keys` is explicit that a
binding is safe only if it is a no-op in *every* agent abeam can host. `Alt+H`
is clear of Claude's claimed Alt letters and is the natural spelling of "home",
but it is unaudited against Copilot's bare-key handlers and against a remapped
Codex. That audit is the real gate on this proposal, not the pane.

One row on the mockup does not exist yet: *what changed since you last had this
workspace open* needs something written down between sessions. That is a real
feature with a real argument behind it and it should not ride in on a start
view. Ship the view without it.

**Cost.** The most of the four — a `Pane` implementation, a `RightView`
variant, a name in `config::View`, a value in `Opening`, and a keymap audit. It
is also the only one still earning its space after the first ten seconds.

## D — the first-run card

The curtain, shown exactly once per machine, and then never again.

```
┌ abeam · first run ───────────────────────────────────────────────────────────────────────┐
│                                                                                          │
│    Your agent is in the left pane. Nothing on the right ever takes your                  │
│    keystrokes, and no view here switches itself.                                         │
│                                                                                          │
│    The right pane is seven views:                                                        │
│                                                                                          │
│      Alt+G   git      what the agent just changed, read-only                             │
│      Alt+E   files    the markdown it just wrote, rendered                               │
│      Alt+S   shell    a shell in the same directory                                      │
│      F8      queue    work lined up for the agent                                        │
│      F9      pad      notes, yours, kept out of the repository                           │
│      F6      ask      a second copy that may read and may not write                      │
│      F2      pty      what the terminal layer is doing                                   │
│                                                                                          │
│    Drag over anything on the right to copy it. Enter puts those rows in                  │
│    the agent's composer, unsent.                                                         │
│                                                                                          │
│    F1 shows every key.                    shown once · any key to begin                  │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

**Why.** Teaching is a first-run problem, not an every-launch problem, and
separating those two is what buys the space. Because it is shown once it can
afford eighteen rows and full sentences where the curtain affords eight and a
table — so it can say the thing that is genuinely hard to discover: that
dragging over the right pane copies it, and that `Enter` puts those rows in the
agent's composer unsent. That round trip is the feature, and nothing on screen
ever mentions it.

It is also the only proposal that may honestly consume the key that dismisses
it, because by definition there is no session in progress to lose it from.

**The marker.** abeam already writes to exactly one place —
`panes::pad::store`, under `$XDG_DATA_HOME/abeam` or `%APPDATA%\abeam` — and
that module has already argued out where a write belongs and why never in the
repository. An empty `seen` file beside `scratch/` rides that decision instead
of opening a new one.

**Cost.** The curtain's drawing code, one existence check and one file write.
The known failure is `uvx abeam` in a fresh container, where every run is a
first run — so the card must not draw for `-p`, and must not write the marker
there either, or a scripted session silently spends somebody's one showing.

## The four, side by side

| | covers the agent | seen every launch | needs a key | writes a file | teaches the views |
| --- | --- | --- | --- | --- | --- |
| A · boot log | no | only while it is slow | no | no | no |
| B · curtain | yes | yes | no | no | six keys |
| C · start view | no | yes, and after | yes | no | by being one |
| D · first-run card | once | once, ever | no | one marker | all seven |

A and C do not compete. A fills the left pane's dead second; C replaces what the
right pane opens on. Shipping both is one session's work and no new conflict.

## Recommendation

**A now.** It costs no new state, no new key and no new file, it deletes a
papercut already written down in the README, and its dismissal rule is a single
comparison against a counter that already exists. Nothing about it needs
deciding twice.

**Then D, not B.** They are the same drawing code; the difference is whether the
teaching bill is paid once or on every launch for the rest of the install's
life. Once is right, and once is also what lets the card be long enough to
mention the drag-and-`Enter` round trip.

**C is the one that compounds**, and the only one worth waiting on. Its gate is
not the pane — it is a binding cleared against Claude, Copilot and Codex, which
is the same audit `docs/keymap.md` has already made twice.

# The ones that move

A second pass, on the question of whether any of the above is allowed to be
fun. The short answer is yes, and cheaply, for a reason that is a fact about
the loop rather than an opinion about taste.

## Motion is already paid for

The usual objection to an animation in a TUI is that it drags the process awake
sixty times a second to redraw a screen nobody is reading. Here it does not.
`App::drive` already waits on `TICK` — 10 ms — to poll the panes that have no
doorbell: the git pane's channel, a viewer walk finishing, a shell child's
`try_wait`. An animation is one more thing for a wake that already happens to
advance, and it says so the same way those panes do, by returning `true` from
`tick()`.

| | |
| --- | --- |
| `TICK` | 10 ms — the wake that already happens |
| `MIN_FRAME` | 8 ms — the floor, 125 fps of headroom |
| a measured frame | ~0.75 ms, whole window |
| twelve frames a second, for one second | about 1% of a core |

And the cost does not have to be guessed at afterwards. `Frames` already records
worst-frame and fps and `F2` already puts both on screen, so this is the rare
flourish that arrives with its own instrument pointed at it.

One more thing makes a boat the on-theme choice rather than a pasted-on one:
*abeam* is a bearing. It is the direction at right angles to a vessel's keel —
straight off the side, which is where this program puts the right pane.

## The five

**The crossing.** A boat sails the width of the left pane while the agent
starts; it has arrived when the agent speaks. Its position is elapsed time at a
fixed speed, not a percentage, because abeam does not know how long Node will
take and a bar that creeps to 95% and stops is a lie told slowly. If the
crossing finishes and the agent still has not spoken, a second boat enters from
the left — two boats means slow, three means something is wrong, and that reads
correctly without anybody being told what it means.

```
┌ claude ──────────────────────────────────────────────┐┌ git · main ↑2 · 12 changed ──────┐
│ abeam 0.8.1                                          ││ Staged (1)            +40 -6     │
│                                                      ││   M crates/abeam/src/app.rs      │
│ hosting   claude                                     ││ Changed (1)            +7 -0     │
│           ~/.local/bin/claude                        ││   M docs/design.md               │
│ in        ~/src/abeam   main ↑2                      ││ Untracked (1)                    │
│                                                      ││   ? notes/                       │
│                                                      ││                                  │
│                                                      ││ Recent                           │
│                                                      ││   a1b2c3d  2m   parser skeleton  │
│                        |\                            ││   9f0e1d2  1h   queue: arm sends │
│                        | \                           ││                                  │
│                     ___|__\                          ││                                  │
│~~~~~_~~~~~~_~~~≈~≈~≈\______/~~~~_~~~~~~_~~~~~~_~~~~~~││                                  │
│ ▍ waking claude…      3.3s                           ││                                  │
└──────────────────────────────────────────────────────┘└──────────────────────────────────┘
```

**The regatta.** Four small craft, one per thing being made ready — the pty's
size, whether the config file was found, the git pane's first answer, the
watcher's first event. Each moors when its check lands and its lane turns into
the fact it went to fetch. This is proposal A in motion, and it keeps rule 5
because the motion *is* the truth: a boat still at sea is a check that has not
come back, and a lane that never docks is a diagnosis on screen at the one
moment somebody could still act on it. A spinner says a program is alive; this
says which part of it is not.

```
┌ claude ──────────────────────────────────────────────┐┌ git · main ↑2 · 12 changed ──────┐
│ abeam 0.8.1                                          ││ Staged (1)            +40 -6     │
│                                                      ││   M crates/abeam/src/app.rs      │
│                        |\                            ││ Changed (1)            +7 -0     │
│ pty      · · · · · · ·\__/▐ 52 × 12                  ││   M docs/design.md               │
│                                                      ││ Untracked (1)                    │
│                        |\                            ││   ? notes/                       │
│ config   · · · · · · ·\__/▐ …                        ││                                  │
│                                                      ││ Recent                           │
│                   |\                                 ││   a1b2c3d  2m   parser skeleton  │
│ git      · · · · \__/· · ·▐ …                        ││   9f0e1d2  1h   queue: arm sends │
│                                                      ││                                  │
│                 |\                                   ││                                  │
│ watcher  · · · \__/· · · ·▐ …                        ││                                  │
│ ▍ starting…                                          ││                                  │
└──────────────────────────────────────────────────────┘└──────────────────────────────────┘
```

**The beam.** The divider between the panes is the animation: a light travels it
while the agent works and rests when the agent is idle. The only one of the five
that outlives startup — `READINESS_EVERY` already re-reads the agent's own
idle/busy record every 250 ms for the queue's sake, and nothing on screen uses
it unless the queue is open. It costs one column and covers no character of
anything. It also carries the only real ergonomic risk here: a light moving in
peripheral vision for the whole of a four-minute turn is a light somebody will
come to hate, so it wants to be slow, dim, three cells of gradient, and probably
to fade out after the first thirty seconds of a long turn.

```
┌ claude ──────────────────────────────────────────────┐┌ git · main ↑2 · 12 changed ──────┐
│                                                      ││ Staged (1)            +40 -6     │
│ > rewrite the watcher's debounce                     ││   M crates/abeam/src/app.rs      │
│                                                      ││ Changed (1)            +7 -0     │
│ ● Editing crates/abeam/src/watch.rs                  ││   M docs/design.md               │
│   working…                                           ┃┃ Untracked (1)                    │
│                                                      ┃┃   ? notes/                       │
│                                                      ││                                  │
│                                                      ││ Recent                           │
│                                                      ││   a1b2c3d  2m   parser skeleton  │
│                                                      ││   9f0e1d2  1h   queue: arm sends │
│                                                      ││                                  │
│                                                      ││                                  │
│                                                      ││                                  │
│ readiness: busy                                      ││                                  │
└──────────────────────────────────────────────────────┘└──────────────────────────────────┘
```

**The wipe.** One bright column crosses the window and draws the panes as it
passes — four hundred milliseconds, once, never again. It holds no state past
the frame it is on, it never waits for the agent, and it makes the window feel
like something switched on rather than something that appeared. If the answer to
all of this is "something, but barely", this is that something.

**The horizon**, which is listed last because it should probably not ship on.
When the agent has been idle a long while and nobody has typed, the boat comes
back and drifts along the bottom of the pane. It is the most charming idea here
and the one most likely to be turned off in a week: the moment the agent goes
quiet is the moment its user starts thinking, and motion at the edge of vision is
what thinking cannot have near it. Off by default, behind `[defaults]`, and it
stops on the first keystroke rather than finishing its crossing.

## Four rules that keep this fun rather than annoying

1. **It stops the instant the agent speaks.** Not fades, not finishes the
   crossing — stops. The first byte out of the pty ends the animation, because
   from that moment the pane belongs to somebody else's output.
2. **Nothing moves during `abeam -p`.** A one-shot is a scripted session nobody
   is watching, and a boat drawn there is frames spent on an empty room.
3. **It is a redraw, never a wake.** It advances on the `TICK` that already
   happens. The moment it wants a timer of its own it has stopped being free.
4. **One line of config turns it off.** `[defaults]` already exists and already
   holds the light/dark choice. Somebody on a laptop on a train gets to say no,
   and gets the boot log instead.

## Recommendation

**The regatta, with the crossing as its finale.** They are the same sprite and
the same water: the four small craft dock as their checks land, and if the agent
is still starting after the last one moors, a full-sized boat sets out across the
pane. That is one feature, not two, and every frame of it is reporting something
true.

**The beam on its own schedule.** It is not a startup feature and should not be
judged as one — it is the readiness record finally appearing on screen. Slow and
dim, or not at all.

**The horizon goes in the config file, off.** The best idea here and the worst
default.
