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
