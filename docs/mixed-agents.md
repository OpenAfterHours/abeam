# More than one *kind* of agent in the window

> **This is a proposal, and phase 1 of it is built.** The rest is not, which is
> the opposite of `docs/multi-agent.md`'s banner and the reason this is a second
> document rather than a section of that one. Where a paragraph here describes
> code, it describes code that exists today and says so; where it describes
> behaviour, that behaviour does not exist yet.
>
> What landed is the seam and nothing a reader can see: `Agent::kind`,
> `Agent::is_claude`, a `poll_readiness` that asks each pane and a
> `roster_is_wanted` that asks whether any pane is Claude. `App::has_claude_state`
> is gone — the two sections below that quote it are describing the code it
> replaced, and are kept because the argument for the change is the reason it is
> written that way. There is still no chooser, no `A` key and no way to *get* a
> pane of a second kind, so every predicate still answers what it answered
> before.
>
> It is the argument `docs/multi-agent.md` said would be wanted and declined to
> make in passing. That document's "What moves out of `App` and into `Agent`"
> section takes `agent: String` *off* the list of fields that became per-pane,
> on the grounds that it is a fact about the session, and then withdraws a
> promise in the next paragraph:
>
> > The cost of leaving it on `App` is a promise withdrawn: **panes of different
> > kinds are not expressible.** Every agent abeam starts is the one it was
> > started with, in another directory. Hosting Claude in one pane and Codex in
> > the next is a coherent thing to want and this design does not deliver it —
> > `has_claude_state` is a session-wide predicate standing in front of a now
> > per-agent probe, and making it per-pane is a change to the readiness path
> > that wants its own argument rather than a field quietly moved during a
> > refactor.
>
> This is that argument.

## What is being asked for

`a` in the git pane starts another agent — in the worktree under the cursor in
the worktree list, or in the checkout on screen in the status view. It starts
*the agent abeam was started with*. The request is to be able to say which:
working in a Claude session, press a key, and get a Codex pane in the stack
beside it.

## What the reader does

**`A` where they would have pressed `a`, then `Enter`.** Everything before
that is what they press today.

```
Alt+G          the git pane takes the keys, showing this checkout's status
A              the agent list opens over it, cursor on the session's own agent
j j            down to `codex`
Enter          a Codex pane opens in this checkout and takes the keys
```

Or, when the agent belongs in a different worktree, in the list that is already
about worktrees:

```
Alt+G  w       the worktree list
j              down to the worktree the agent belongs in
A              the same agent list, asking about *that* worktree
Enter          the pane opens there
```

**`a` is unchanged and is still the fast path.** `Alt+G`, `a` starts the
session's own agent in the checkout on screen, with no list and no
confirmation, exactly as it does today. `A` is that key with a question in
front of it, and the question is only ever "which one".

What is on screen while it stands:

```
┌ git · start an agent in main ────┐
│ ▸ claude                  session│
│   copilot                        │
│   codex                          │
│   fleet                  → claude│
│   reviewer                → codex│
│                                  │
│ enter starts                     │
└──────────────────────────────────┘
```

Four things about that screen, each of which is the answer to a question the
rest of this document then argues:

- **The list is the names `+` already takes.** `claude`, `copilot` and `codex`
  are `crate::agent::AGENTS`; `fleet` and `reviewer` are this reader's
  `[preset.*]` blocks from `abeam.toml`. It is the same table that answers
  `abeam +codex` on the command line — `crate::config::Config::table` — so
  there is nothing new to learn and nothing new to configure. A machine with no
  presets sees three rows.
- **The border names the checkout**, because the question is about one. `A` in
  the worktree list is asking about the row the cursor was on; `A` in the status
  view is asking about the checkout the pane is showing. Neither is "wherever
  the cursor ends up" — the answer is captured on the keystroke that asked.
- **The cursor starts on the session's own agent**, marked `session` on the
  right. So `A` `Enter` is "another one of what I already have", and the common
  case stays two keystrokes. The *order* is the table's and never moves.
- **A preset says what it hosts.** `fleet → claude` is a Claude pane whatever it
  is called, and that is worth seeing before choosing, because it decides
  whether the queue will be able to type at the pane at all. `crate::agent::Agent::hosts`
  is a static field, so this costs nothing to draw.

The sketch marks the cursor with `▸`; the real list highlights the row's
background, as `worktree_lines` does today. While the question stands the
border's `exit_hint` reads `esc→list` rather than `esc→git`, because `Esc` here
puts the reader back in the list they pressed `A` in.

**What happens when the answer is a program that is not installed.** The list is
what abeam can *name*, not what is on the machine — see "Resolution happens on
the keystroke" below — so choosing `codex` on a box without Codex opens no pane
and puts `crate::agent::missing`'s paragraph on the left border, naming what was
looked for and how to install it. That is the same sentence, at the same
quality, that `abeam +codex` gives at startup.

## Why this is smaller than it sounds

Five things are already in the shape this needs, and none of them were built
for it.

- **The border already names the pane and not the session.**
  `abeam_pty::PtyConfig::title` is set per spawn and `TerminalPane::title`
  draws it, so a Codex pane says `codex` the moment one exists. This is the
  whole of what makes a mixed stack legible, and it costs nothing.
- **The readiness gate is already a parameter.**
  `crate::app::Agent::send_readiness` takes `claude: bool` and returns
  `Readiness::Unknown` before it touches the probe when that bool is false. It
  is called from one place, `App::poll_readiness`, which hoists one answer out
  of the loop. Making the answer per-pane is a change to the argument
  expression and nothing else.
- **The table is already `&'static` and already includes the user's presets.**
  `crate::config::Config::table` returns `&'static [crate::agent::Agent]` —
  built-ins first, then one row per `[preset.<name>]` — and `main` already
  holds it, because `crate::agent::parse` and `main::host` both take it. Handing
  it to `App` costs a field and no allocation.
- **Turning a table row into something startable is already one function.**
  `crate::agent::resolve_within(row, &[], table)` walks the row's candidates,
  and answers either a `Hosted` — `name`, `agent`, `launch` — or the sentence
  `crate::agent::missing` writes, which names what was looked for and how to
  install it. It is what `main` calls at startup.
- **There is already a surface for the refusal.** `App::agent_refused` is an
  `Option<String>` that `App::start_agent` writes and the left border draws.
  A Codex that is not installed lands there in the same sentence it would have
  landed in at startup.

## The field that has to move, and the four readers that decide what it means

`crate::app::App` holds one string:

```rust
/// The hosted agent's *kind*, for the same reason — and not the name on any
/// border.
agent: String,
```

and its own documentation already names this as the seam:

> **It is a session-wide answer gating a per-agent question, and that is the
> seam to watch.** The probe `has_claude_state` stands in front of now belongs
> to an [`Agent`]; this string does not. While every pane hosts what abeam was
> started to host the two cannot disagree.

The proposal is `Agent::kind: String`, written once in `Agent::new`, never
assigned afterwards — the same rule `Agent::root` is held to, and for a weaker
version of the same reason: a pty was opened to run one program and that spawn
is over.

`App::agent` **stays**, because three of its four readers are asking about the
session and would be wrong to ask about a pane. What follows is those four
readers, re-read one at a time, because the whole of this change is in which of
them moves.

### Readiness moves, and it is one line

`App::poll_readiness` today:

```rust
let claude = self.has_claude_state();
for agent in self.agents.iter_mut() {
    agent.readiness = agent.send_readiness(claude);
    moved |= agent.follow_record();
}
```

becomes `agent.send_readiness(agent.is_claude())`, where `Agent::is_claude` is
`self.kind == "claude"` — byte-identical to `App::has_claude_state` today, one
field along.

**The safety here is by construction rather than by care, and that is worth
saying outright.** A non-Claude pane's probe is never read, because
`send_readiness` returns before reaching it. So a Codex pane cannot report a
neighbouring Claude's `idle` as its own — not because anything checks, but
because it never asks. The sibling-disowning that `App::start_agent` already
does for same-kind panes stays exactly as necessary as it is now, and covers
the case this cannot: two Claude panes in one checkout.

### The roster widens from "the session" to "any pane"

`App::roster_is_wanted` is `self.has_claude_state() && (self.dispatched_any ||
self.worktrees_wanted)`, and what it protects is the rule that a session which
never uses a feature never starts `claude agents --json`.

Per-pane it becomes `self.agents.iter().any(Agent::is_claude) && (…)`.

**This is a behaviour change and not a refactor.** A session started under Codex
that opens a Claude pane and then opens the worktree list will start a process
it does not start today. That is correct — the occupancy column is about
background Claude agents in this repository, and whether they are worth naming
does not depend on which program `agents[0]` happens to be — but it is a new
`claude` invocation in a session that never typed the word, and it should be
introduced deliberately rather than discovered.

### The ask pane keeps the session's answer, and this is a decision

`crate::app::Space::new` is handed `agent` and builds an `AskPane` with it;
`crate::panes::ask::resolve` looks the name up in `ASKABLE` and gets a
`crate::ask::Flavour` — `Claude` or `Copilot`, two hand-written shapes.

The ask pane is **per workspace**, not per agent pane, and a workspace can hold
two agents of different kinds. So there is no answer to "which one is it a
second copy of" that is true in general.

Following the current pane — the obvious move, and cheap, because availability
is behind a `OnceCell` and is not decided until the pane is first used — is
declined for two reasons:

- The pane holds a **live conversation** with a running cost total on its title.
  A flavour that changed under `F4` would mean either killing that conversation
  when the reader moves between panes, or keeping it and having the pane's
  identity disagree with the thing it says it is a copy of. Neither is a pane
  anybody can reason about.
- With the `OnceCell`, the answer would be decided by *which pane had the keys
  the first time anyone pressed `?` in this workspace*, and pinned there for the
  session. That is a fact about a keystroke ordering, which is not something a
  reader can predict or repeat.

**Two call sites, not one**, and phase 3's checklist has to say so:
`Space::new` is called by `App::new` for the first workspace and again by
`sync_workspaces` for every worktree discovered afterwards. Both pass the
session's agent, which is the same decision applied consistently — but anyone
revisiting open question 1 who edits one and not the other gets a window where a
workspace's ask flavour depends on whether that worktree existed at startup.

So the ask pane goes on being the session's agent, and the sentence
`crate::panes::ask::elsewhere` already writes — "abeam is hosting `X`, and this
pane is a second copy of the agent you are already talking to" — becomes very
slightly less true in a mixed session, in that the agent you are talking to may
be in the other pane. Named here because nothing on screen will say it.

### Dispatch keeps the session's answer too, for now

`QueuePane::new(root, agent)` builds a `crate::dispatch::Dispatcher` eagerly,
and `Dispatcher::new` refuses anything but Claude because `--bg` is Claude's
alone. The queue is one pane for the whole window, so "can this session
dispatch" is a single question with a single answer, and it is the session's.

Widening it to "any pane hosts Claude" is coherent and is **not** in this
proposal, because it is not a predicate change: the dispatcher is built once, at
`App::new`, from a string that cannot change; making it arrive later means the
queue holding a `Result<Dispatcher, Unavailable>` that can turn from `Err` into
`Ok` when a pane opens, and every message it draws about being unavailable
becoming a statement with a shelf life. That is its own argument. It is phase 4
below, and until it lands the honest description is: **a Codex session that opens
a Claude pane can queue prompts to that pane and still cannot dispatch.**

## What a later pane is started from

`crate::app::Recipe` is the type that exists so a pane opened on a keystroke can
be built from the same program as the first one. It keeps two fields —
`target`, the file that does the work, and `name`, the border's word — and
derives a fresh `Launch` with `crate::launch::resolve_at(&self.target, &[])`.
Its documentation spends a page on why the command line is deliberately not in
there, and all of it stands: `abeam -p "fix the tests"` must not be re-run in a
worktree it was never written about.

For a *chosen* agent there is no `target` to keep, because nothing has been
resolved. The route is the one `main` already takes:

```rust
let hosted = crate::agent::resolve_within(row, &[], self.table)?;
```

which gives all three of the fields a new pane needs — `launch` to spawn from,
`name` for the border, `agent` for `Agent::kind` — and whose `Err` is the
install sentence, already written, already good, already having somewhere to go.

So `Recipe` grows a third field, `kind`, and `App::start_agent` grows a second
parameter:

```rust
fn start_agent(&mut self, root: &Path, choice: Option<&str>) -> bool
```

`None` is today's path exactly: the session's `Recipe`, the session's `kind`.
`Some(name)` is `find_within(name, self.table)` and then `resolve_within`.

### Resolution happens on the keystroke, and the list is not pre-flighted

The chooser lists what abeam can *name*. It does not list what is *installed*,
and it must not try: marking the missing rows would mean a `PATH` walk per row
per frame the list is open, and `crate::agent`'s module documentation is
explicit that abeam's answer to an agent it cannot find is a sentence rather
than a silence — the sentence names the candidates and the install command, and
a greyed-out row says none of that.

A row is chosen, the resolve fails, `agent_refused` gets `missing()`'s
paragraph, and the reader is told what to install. That is the same failure the
session's own agent has at startup, at the same quality, and it costs nothing to
reach.

### One thing found on the way, which is a gap in today's code

`Recipe::launch` re-resolves with **no arguments at all**, so a preset's own
`args` are dropped. In a session started `abeam +fleet`, where
`[preset.fleet] host = "claude", args = ["agent"]`, the first pane runs `claude
agent` and every pane opened with `a` runs plain `claude`. Two panes, one border
word, two different programs.

**Verified, including on Windows**, where it might have been expected not to
hold: the `.cmd` shim puts the command line in `ABEAM_LAUNCH` rather than in
argv, but `Recipe` never carried `Launch::env` either, so `through_cmd` simply
rebuilds the variable from the empty argument list. Both platforms drop the
preset's args by the same route.

**An earlier draft proposed fixing this by routing `Recipe::launch` through
`resolve_within`. That is wrong and would have been a regression.**
`resolve_within` searches by bare *name*, so it re-walks `PATH`, while
`Recipe::launch` uses `resolve_at` against an absolute path precisely so that a
keystroke-opened pane starts *the same file* the session did. A `PATH` that
changed since startup, or a nearer entry that appeared, would silently start a
different binary under the same border word — the asymmetry `Recipe`'s own doc
says it exists to remove.

The fix is one field, and it keeps `resolve_at`:

```rust
struct Recipe {
    target: PathBuf,
    name: String,
    kind: String,        // Hosted::agent
    args: Vec<String>,   // the row's own args, empty for a built-in
}

fn launch(&self) -> Result<Launch, String> {
    crate::launch::resolve_at(&self.target, &self.args)
}
```

**`args` and emphatically not `env`.** `Launch::env` is not an input to
resolution — it is *derived from* `(script, args)` on every resolve, by
`through_cmd`, which quotes the arguments back into `ABEAM_LAUNCH` itself. So
resolving the shim with `["agent"]` runs `claude agent` on Windows and puts the
same word in argv on Unix: one field, one code path, both platforms. Carrying a
stale `env` beside a blanked argv is the exact failure `Recipe`'s doc already
warns about — for `abeam -p "fix the tests"` the prompt lives in that variable,
so the result is a bare `cmd.exe` under a border reading `claude`.

`args` is the right thing to carry because it is the input the resolver
re-derives everything else from, and `crate::agent::Agent::args` is precisely
the preset's contribution with the typed command line excluded. A program named
outright still has no row and needs none: its `args` are empty and its `target`
is the whole recipe. It belongs in phase 2, and it wants a test named after the
disagreement.

## The keys

`docs/keymap.md`'s invariant is about **global** bindings, and this needs none.
`GitPane`'s `a`, `w`, `x` and `?` stand on the pane-local exemption — they are
delivered only under `Focus::Right` and only with this pane's view up, so they
are never in front of an agent that is listening — and a fifth letter claimed
there stands on exactly the same ground.

**`A` opens a chooser; `a` is untouched.**

`a`'s own comment argues that it needs no confirmation *because the gesture
reports itself on the frame it fires* — a row gains an agent in its occupancy
column, the new pane says `2/2`. Putting a list in front of `a` would cost that
argument and would make the common case two keystrokes; the status-view arm of
`a` exists precisely because a four-keystroke route to the ordinary request was
judged too long. `A` next to `a` is the smallest possible spelling of "the same
thing, but ask me first", and it is claimed in **both** lists, because `a` is.

### `Shift` may be tolerated and must never be required

`crates/abeam/src/panes/git.rs` has a test named
`a_chord_never_starts_or_kills_an_agent_in_either_list`, and its comment records
that the two `a` arms once disagreed about modifiers — `Ctrl+A` started an agent
in the worktree list while the status list declined it, three lines under a
comment saying the opposite. It refuses `CONTROL` and `ALT`. It does not refuse
`SHIFT`, because nothing needed it to.

**An earlier draft of this section said the chooser should be `a`-or-`A` *with
`SHIFT` set*. That is exactly backwards, and it would have shipped a key that
does nothing on most Unix terminals.** Crossterm's ANSI parser has no modifier
bits to recover from a bare `A` byte, so on Unix Shift+A arrives as
`KeyCode::Char('A')` with `KeyModifiers::NONE`. A rule that *requires* `SHIFT`
refuses it.

The repository has already settled this twice, in the opposite direction:

- `crate::keys::global` matches `Char('g') | Char('G')`, `Char('q') | Char('Q')`
  and `Char('k') | Char('K')`, and **never consults the `SHIFT` bit at all**.
- The `?` arm in `crate::panes::git` excludes `CONTROL` and `ALT` **by name**
  rather than by `modifiers.is_empty()`, and says why: `?` is a shifted key on
  most layouts, so `SHIFT` arrives with it and must not disqualify it. The
  right-hand fallthrough in `crate::app` carries the same note about `q`, and
  the words there are "*some* terminals report SHIFT for an uppercase letter".

*Some* is the whole point. `SHIFT`-on-uppercase is a variable, so it can be
tolerated and never required.

So this is **two independent edits, not one three-way match**:

```
Char('A')                                => choose   // Ctrl/Alt still refused by name
Char('a') if !modifiers.contains(SHIFT)  => start
```

The first claims the letter the way `keys::global` claims letters. The second is
the real hazard the paragraph above was reaching for: on a terminal that *does*
set `SHIFT` for an uppercase letter, an unguarded `a` arm would start the
session's agent without asking, in the exact place the reader was trying to ask.

The existing test grows two rows, and the second is the one that matters: a
`SHIFT` row for `a`, **and a row asserting `Char('A')` with
`KeyModifiers::NONE` opens the chooser**. Without that second row the Unix bug
ships green. `Alt+A` stays refused, and that half is not symmetry —
`crate::keys` records that abeam yielded `Alt+A` to Codex, which after this
change is an agent any session can be hosting.

**And `A` is genuinely free in both lists, which was checked rather than
assumed.** `crate::scroll::key` claims `Ctrl+d`, `Ctrl+u`, `j`/`Down`, `k`/`Up`,
space/`PageDown`, `b`/`PageUp`, `g`/`Home` and — the one that matters —
**`G`/`End`**. So uppercase is *not* free in general; `A` simply is not in that
list. One asymmetry for whoever writes the two arms: in `Mode::Status` `scroll`
sees the key *before* the pane's match, and in `Mode::Worktrees` the pane's
match runs *before* `wt_scroll`. It does not change the answer for `A`, but the
two arms sit at different depths.

## The chooser, in code

What it looks like and what the reader presses is "What the reader does" above.
This is where that screen comes from.

**A standing question beside the mode, and not a third mode.** `GitPane` already
has the shape for this and it is not `Mode`: `kill: Option<PathBuf>` is the `x`
confirmation, held *over* whichever list is up and read by the shell through
`GitPane::closing` without the mode changing at all. The chooser is the same
kind of thing — a question the pane is asking about a checkout — and it is
reachable from both lists, which a mode would have had to encode a way back out
of.

```rust
/// The agent `A` is asking about, and which row of the table is under the
/// cursor while it asks.
choosing: Option<Choice>,

struct Choice { root: PathBuf, sel: usize }
```

Two things carried rather than looked up, and both have a precedent in this
file:

- **`root`, captured at the keystroke.** The `x` confirmation carries the row it
  asked about rather than trusting `wt_sel`, because `x`, `Tab`, `x` would
  otherwise warn about one worktree and kill in another. `A`, `Tab`, `Enter` is
  the same hazard one gesture along, and gets the same answer.
- **`sel`, starting on the session's own agent.** The list's *order* is the
  table's and never moves — a list that reorders itself by session is one nobody
  can build muscle memory in — but the cursor starts on the row the session was
  started from, so `A` `Enter` is "another of what I already have" and the
  common case stays two keystrokes.

The table itself is handed to the pane, once, at construction —
`GitPane::new(root, table)` with a `&'static [crate::agent::Agent]`, which is
what `crate::config::Config::table` already answers and what `main` already
holds. It borrows nothing and is never rebuilt, so this is a field and not a
`set_` call on the `set_worktree_rows` pattern: the rows change under a worker
thread and the table cannot change at all.

Movement is `crate::scroll`'s shared vocabulary, which this pane already routes.
`Enter` chooses. `Esc` clears the field, which puts the reader back in the list
they pressed `A` in because they never left it.

**The analogy to `kill` is about where the state lives and not about how keys
reach it**, and that difference is the one genuinely new thing here. A standing
`kill` changes nothing about routing — every other key simply cancels it,
through the `take` at the top of `worktree_key`. A standing `choosing` has to
*swallow* `j` and `k`, or the reader scrolls the worktree list while looking at
a list of agents. So `handle_key` grows one early branch, above the line that
dispatches to `worktree_key` — which is its second — and that one branch covers
both lists.

**Two requirements on that branch that an earlier draft did not state, and both
are load-bearing.**

**It must return `Handled::Yes` for every key, not only the four it acts on.**
The shell treats a `Handled::No` on a bare `Esc` *or* `q` as "the reader is done
with this pane" and moves focus to the left column. A branch that handles
`j`, `k`, `Enter` and `Esc` and falls through on the rest means a bare `q` hands
the keyboard to the agent with `choosing` still `Some` — an invisible standing
question over a `root` captured some time ago and still ageing. Coming back to
the pane later, the next `Enter` starts an agent in a checkout the reader has
forgotten they named. The `esc→list` hint is only an honest promise if the
branch consumes `Esc` itself.

**The two `A` arms stay inside each list's own match and must not be hoisted
into that branch.** `worktree_key` takes the standing `kill` before it matches
anything, which is what the `x` arm's own comment relies on: *any* other key in
that list is the answer "no". An `A` hoisted above that `take` skips it, and
`x`, `A`, `Esc`, `x` becomes a kill of a live agent with **one** visible
warning, dismissed in between by an unrelated full-pane list. That is the most
destructive action in the program losing half its guard. As written — the branch
fires only when `choosing.is_some()`, and `A` is matched by each list — the
sequence is safe, because the `A` press itself has already taken the kill. It is
safe by ordering rather than by construction, so it is worth a test: `x`, `A`,
`Esc`, `x` must close nothing.

**And the same early return is owed to `handle_mouse`.** Without it the wheel
scrolls a list nobody can see and a click moves the selection underneath the
chooser. Not a correctness bug — `Choice` carries its own `root`, so the
captured checkout cannot change under it — but "one branch is the whole of the
chooser's key handling" is true of keys and there is a second input path.

**Four other sites read the mode and each needs an answer while the question
stands:** `render` (branch before the status list is measured), `title` (this is
where `git · start an agent in main` comes from, and the checkout name must come
from `Choice::root` — never from `self.root` or the selection, or the title
disagrees with what `Enter` will do), `exit_hint` (a pre-check returning
`esc→list`; it answers `&'static str`, so a third answer is free), and
`set_worktree_rows`, whose "a frame is owed" return may optionally be suppressed
while a chooser is up, since nothing the chooser draws can change. Nothing
matches `Mode` exhaustively, so none of this needs a third variant — which would
additionally have had to encode which list to return to.

### The rows say what a preset hosts

The `→ claude` column in the sketch above is one field read straight off the
table. `crate::agent::Agent::hosts` is a static field — no filesystem, no cost — and it
is the field that decides whether a pane will have readiness at all. A reader
choosing `fleet` should be able to see that they are getting a Claude pane, and
therefore a pane the queue can type at; a reader choosing `reviewer` should be
able to see that they are not. **This is the only place in abeam where the kind
of a pane is visible before it exists**, and it is the reason the chooser is
worth having even for a reader who only ever picks the session's own agent.

### What the git pane hands back

`GitPane::take_agent_request` returns `Option<PathBuf>` today. It becomes:

```rust
pub struct AgentRequest {
    pub root: PathBuf,
    /// The name from the table, or `None` for the session's own agent.
    pub agent: Option<&'static str>,
}
```

`&'static str` because the table is `&'static`, so the request borrows nothing
and outlives nothing. `None` keeps `a` byte-identical, which is what makes phase
2 shippable without re-testing the gesture that already works.

The pane keeps owning this, rather than the chooser living in `App` as an
overlay, because the pane already owns `a`, `x`, the close confirmation and the
worktree rows — and because routing keys to an app-level overlay means `App`
intercepting before a focused pane is offered anything, which is the one thing
`crate::keys`' intercept paragraph is written to keep rare.

## What goes wrong, and what it costs

### A queued prompt aimed at a pane that cannot receive never sends, and nothing says why

This is the largest hazard in the proposal, it is a disclosure problem rather
than a mechanism one, and **it is not a hazard this feature creates** — see the
end of this section, which is the reason phase 3 now ships first.

`QueuePane::gate` is `readiness.is_idle() && !draft_open`, and `next_send` will
not name an item whose gate is not `Some(true)`. `Readiness::Unknown` is not
idle. So a pane that reports `Unknown` can never be typed at by the queue, ever,
by design. That mechanism is right and nothing in it should be relaxed.

**`Enter` does not bypass it, and the refusal is completely silent.** A by-hand
ask only grants a due, which still has to survive `next_send`; and before it
gets that far, `QueuePane::now` refuses outright when the gate is not
`Some(true)` — no send, no redraw, no message. The comment beside that refusal
says "the status line already says which of the two it was", meaning *busy* or
*you are typing*. For a pane that can never receive it is neither, so **that
comment becomes false and the key becomes a dead one with a lie next to it.**
Phase 3 must give that path a sentence.

**And the status line describes exactly one pane, which may not be the one that
is stuck.** `gate_state` picks its subject through `gate_target`: the pane about
to be typed at, else the pane the first pending `Send` is waiting on, else the
aim. `index_of(Mode::Send)` returns the first pending item *regardless of
target*. So: two panes, the Claude one busy, a Claude-aimed item ahead of a
Codex-aimed one — the footer reads `claude · busy`, and the item that can never
go is named nowhere on screen. A fix confined to the footer does not reach its
own case.

**An earlier draft over-claimed the harm and the correction is worth keeping.**
It said a queue silently holding a prompt for ever is "the failure shape
`crate::agentstate` exists to refuse". It is not: `next_send` deliberately
*walks past* a blocked item rather than stopping at it, and its own doc says so
— so a Codex-aimed item at the head of the list blocks nothing behind it, and
every other pane's prompts keep flowing. The real harm is one item silently
never going with nothing on screen saying so, which is enough of an argument on
its own.

**The smallest honest fix is three sites, not one.**

- **`Target` gains `can_receive: bool`** — not the kind string an earlier draft
  proposed. Two reasons. `Target` derives `PartialEq` and is compared *whole* in
  `set_targets`, once per pane per readiness poll, so a `String` per pane per
  quarter second is an allocation with nothing on the other side of it. And the
  kind is the wrong predicate: `send_readiness` answers `Unknown` for three
  causes in order — the child has exited, the child has not asked for bracketed
  paste, the pane is not Claude — and only the first and third are permanent.
  "Can ever be typed at" is the question the sentence needs, so it is
  `is_claude() && !pane.has_exited()`, filled in `sync_queue_targets` where the
  agent is already in hand.
- **The footer splits its `Unknown` arm** into "cannot receive" and today's
  "state unknown". Two different facts, and only one of them is worth waiting
  for.
- **The item's own row says it too**, which is the half that covers the case
  above. `aside` already draws the target's label for every `Send` item once
  there is more than one target — and a mixed session always has more than one
  by construction — so that arm gains the reason beside the name.

Nothing in `gate`, `next_send`, `retime` or `take_send_request` changes.

**This bug exists today, with no mixed agents anywhere, and that is why phase 3
goes first.** An **exited but unclosed** Claude pane produces it exactly:
`send_readiness` returns `Unknown` on `has_exited()` before it asks anything
else; the pane stays in `App::agents`, because `close_agent` is the only remover
and it is driven by `x` `x`; so it stays in `targets`, `orphan_lost_targets`
never fires — that only orphans items whose target has *gone from the list* —
and its items sit `Pending` for ever under `state unknown`. Phase 3 is therefore
independently justified and independently testable, and sequencing it ahead of
phase 2 is a choice rather than a dependency.

The aim still moves to a newly started pane, and that stays right. Refusing to
move it would make a keystroke's effect depend on the kind of thing it started,
and a reader who wants a note aimed at the new Codex pane — to read later, by
hand — is not doing anything wrong.

### A non-Claude pane does not follow its agent into a worktree

`Agent::follow_record` reads `Probe::standing_in`, and the probe for a
non-Claude pane is never populated, because `send_readiness` returns before
touching it. So `moved_to` stays `None` and `Agent::standing` answers
`Agent::root`.

`docs/multi-agent.md` describes at length what that used to cost, and it is
worth being exact about how much of it comes back, because the answer is "less
than it looks":

- The border names the checkout the pane was **spawned** in, for ever. For a
  Codex pane that makes its own worktree and moves in, that is a stale name.
- `workspace::rows`' guarantee — a row for every directory an agent is *working*
  in — holds against `standing()`, which for these panes is the spawn root. So
  the row exists, and `x` `x` on it still ends the pane. The phase-4 "a pane
  with no way out" bug does **not** come back.
- The occupancy column credits the spawn root rather than where the work is.

This is not fixable from abeam's side. Following a session into a worktree is
built on Claude writing its own `cwd` into a session record abeam can read and
identity-check by `sessionId`; abeam knows of no such record for Codex or
Copilot, and inventing one from `git worktree list` alone was declined once
already, in that document's own "The ten-second list" section.

So: **mixed sessions get the pre-`moved_to` behaviour for non-Claude panes**, and
the reader should be told once, in the README's section on this feature, rather
than discovering it from a border that has gone quietly wrong.

### The rest of the list, briefly

- **The exit contract is untouched.** `agents[0]` is the session's, is never
  removed, and is the only pane whose exit becomes abeam's status code. Nothing
  in this proposal can create or replace it — the chooser is reachable only from
  a keystroke, and a keystroke can only append.
- **The layout is untouched.** `MIN_AGENT_ROWS` is twelve rows of transcript and
  furniture, and no agent needs more or fewer of them for being a different
  program.
- **Two panes of two kinds in one checkout** is already handled by the
  sibling-disowning in `start_agent`, and is now additionally safe by
  construction, since a non-Claude pane never searches for a record at all.
- **`+program` panes are not in the chooser.** A session started `abeam +pwsh`
  can still open more `pwsh` panes with `a`, through the `Recipe` path; it just
  cannot be chosen by name, because the list is the table and a program named
  outright is not in it. A text field in the git pane is the thing that would
  fix that, and it is not worth a text field.
- **N kinds is N programs' worth of memory**, which is nothing new — the stack
  already allowed N panes — but the chooser makes reaching for a second and third
  agent easy, and the honest note is that they cost what they cost.

## Phasing

Each phase is shippable and each has something to test.

**Phase 1 — the seam, with no user-visible change. Built.** `Agent::kind`,
written in `Agent::new` from the `Hosted` that spawned the pane;
`Agent::is_claude`; `poll_readiness` asks each pane; `roster_is_wanted` asks
whether any pane is Claude. Every pane's kind is the session's, so every
predicate answers today's answer. The test is that it still does.

Two things landed differently from the sketch above and both are worth naming.
`send_readiness` lost its `claude: bool` rather than being handed a per-pane
one, because the parameter's only caller was the loop that hoisted the answer
and a function that reads its own field cannot be handed a neighbour's. And
`start_agent` takes the *session's* kind rather than the recipe's, because
`Recipe` has no kind until phase 2 gives it one — which is where the preset-args
disagreement is fixed too, and for the same reason.

**Phase 2 — the disclosure, and it now goes second rather than third.** `Target`
gains `can_receive`; the footer splits its `Unknown` arm; the item's own row
carries the reason; `QueuePane::now`'s silent refusal gets a sentence and its
comment stops claiming the status line already explained it.

**It was written as phase 3, behind the feature, and that was wrong.** The bug
it fixes needs no mixed agents at all — an exited but unclosed Claude pane
reproduces it exactly, and the section above has the chain. So it is not a
disclosure owed by phase 3's feature; it is a hole in today's program that
phase 3 would have widened. Fixing it first also means phase 3 lands into a
queue that can already say what it is doing, rather than shipping a feature and
its own caveat together.

**Phase 3 — the feature.** `App` holds the table; `Recipe` carries the kind and
the row's `args`; `start_agent` takes a choice; the `choosing` question, the two
`A` arms and the `SHIFT` guard on `a`; `AgentRequest`. The preset-args fix and
its test go here, because this is the phase that would otherwise make the
disagreement worse. The README gains the note about what a non-Claude pane does
not do.

**Phase 4 — dispatch, if it is wanted.** The queue's dispatcher becomes
something that can arrive after a pane opens. Note that there are **two**
`Dispatcher::new` sites, not one — `QueuePane::new` builds one eagerly and
`pump_queue` builds another on the thread for each `--bg` run — so this is a
larger phase than it looks. Its own argument, its own document if it turns out
to want one.

## What this is not

- **Not abeam creating worktrees.** `a`'s own comment refuses that, on
  `crate::dispatch`'s argument that writing a git worktree into somebody's
  repository as a side effect of a keystroke is a structural change they did not
  ask for. Choosing which agent starts changes nothing about that.
- **Not a way to install anything.** `crate::agent`'s module docs record a
  launcher fallback that was written, shipped for a day, and deliberately
  removed. A chooser is a list of names abeam can already start, and a name it
  cannot start is a sentence.
- **Not per-agent key handling.** abeam's global set is the union of what three
  agents claim, decided once in `crate::keys` and audited in `docs/keymap.md`.
  It does not vary by what is hosted today and must not start to — a key whose
  interception depended on which pane had focus is the thing that invariant
  exists to prevent.
- **Not a repository-local anything.** The table comes from the built-ins and
  the user's profile. `crate::config`'s module docs have the security argument,
  and a chooser reading a list of programs out of the checkout on screen would
  undo all of it.

## Open questions

1. **Should the ask pane follow the current pane after all?** Declined above on
   two arguments, both about the conversation it holds. If it changes, the
   honest shape is probably a per-*pane* ask rather than a per-workspace one,
   which is a much larger change than this document.
2. **Should dispatch widen to "any Claude pane"?** Phase 4. The question is
   really about whether `QueuePane` may hold a capability that arrives late.
3. **Should `[defaults]` be able to name the keystroke's agent** — a machine
   where `a` starts Codex in a Claude session? It would make the common case one
   keystroke for someone whose habit is fixed, and it makes the answer a fact
   about the machine rather than about the moment. Cheap to add later, and
   nothing in this design forecloses it.
4. **Does anything report Shift+letter as `Char(lower)` + `SHIFT`?** The keys
   section assumes it might and pays one test for the assumption. Whether it
   actually happens on any terminal abeam ships to is unmeasured, and the
   measurement is `crates/abeam/examples/keyprobe.rs`.
5. **What should the border say about a pane the queue cannot type at?** Phase 3
   answers it in the queue's status line, which is where the countdown already
   is. Whether the *left* border should say it too — next to the name that is
   already there — is a question about how much a title row can carry, and this
   document does not answer it.
