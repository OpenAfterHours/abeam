# More than one *kind* of agent in the window

> **This is a proposal. Nothing below is built**, which is the opposite of
> `docs/multi-agent.md`'s banner and the reason this is a second document
> rather than a section of that one. Where a paragraph here describes code, it
> describes code that exists today and says so; where it describes behaviour,
> that behaviour does not exist yet.
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

That is not this feature, and this feature makes it visible: a chooser routed
through `resolve_within` *would* keep a preset's args, so `A` → `fleet` and `a`
in a `+fleet` session would disagree.

The fix is to make `Recipe` carry the table row when there is one — the session
was started from a name, `find_within` finds it, and `None` is kept for the case
that has no row and never will: a program named outright, `abeam +pwsh` or
`abeam +C:\tools\claude.exe`. Then `a` and `A → the same name` are one program
by construction rather than by coincidence. It is small, it belongs in phase 2,
and it wants a test named after the disagreement.

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

### `Shift` has to become part of the gesture, and today it is not

`crates/abeam/src/panes/git.rs` has a test named
`a_chord_never_starts_or_kills_an_agent_in_either_list`, and its comment records
that the two `a` arms once disagreed about modifiers — `Ctrl+A` started an agent
in the worktree list while the status list declined it, three lines under a
comment saying the opposite. It refuses `CONTROL` and `ALT`. It does not refuse
`SHIFT`, because nothing needed it to.

Crossterm reports Shift+A as `KeyCode::Char('A')`, which falls through both arms
today and is why `A` is free. But a terminal that reports `Char('a')` with
`SHIFT` set — and the modifier reporting in this program has already had one
platform surprise, `crate::keys`' AltGr paragraph — would land on the `a` arm
and start the session's agent without asking, in the exact place a reader had
just tried to ask.

So the arms become one gesture with three cases, and the existing test grows a
`SHIFT` row: `a` with no modifiers starts; `a`-or-`A` with `SHIFT` chooses;
anything with `CONTROL` or `ALT` is `Handled::No`. `Alt+A` in particular stays
refused, and that half is not symmetry — `crate::keys` records that abeam
yielded `Alt+A` to Codex, which after this change is an agent any session can be
hosting.

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
a list of agents. So `handle_key` grows one early branch, ahead of either
list's match, and that branch is the whole of the chooser's key handling.

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

### A queued prompt aimed at a non-Claude pane never sends, and nothing says why

This is the largest hazard in the proposal and it is a disclosure problem rather
than a mechanism one.

`QueuePane::gate` is `readiness.is_idle() && !draft_open`, and `next_send` will
not name an item whose gate is not `Some(true)`. `Readiness::Unknown` is not
idle. `Enter` does not bypass it — a `Due::Asked` still has to survive
`next_send` — and that is deliberate: `retime`'s comment says a by-hand ask is
attended, not exempt.

So a non-Claude pane can never be typed at by the queue, ever, by design.

Today that is a whole-session fact: `has_claude_state` is false, every pane is
`Unknown`, and the queue says so once. After this change it is one row in a
stack — and `App::start_agent` moves the queue's aim to the pane it just
started, deliberately, because "somebody who presses `a` and then goes to write
a prompt means it for the pane they have just started". Press `A`, choose Codex,
write a prompt: it is aimed at a pane that will never take it, and
`QueuePane::gate_state` will draw the pane's label and `Unknown`, which is a
word about abeam's ignorance rather than about the pane's kind.

**The fix is in `Target`.** It carries `id`, `label`, `readiness` and
`draft_open`; it gains the kind, and the status line distinguishes "abeam cannot
say" from "this pane is not one the queue can type at". Those are two different
sentences and only one of them is worth waiting for. That is phase 3, and phase
2 should not ship without it — a queue that silently holds a prompt for ever is
the failure shape `crate::agentstate` exists to refuse.

The aim still moves. Refusing to move it would mean a keystroke whose effect
depends on the kind of thing it started, and the reader who wants the note aimed
at the new Codex pane — to read later, by hand — is not doing anything wrong.

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

**Phase 1 — the seam, with no user-visible change.** `Agent::kind`, written in
`Agent::new` from the `Hosted` that spawned the pane; `Agent::is_claude`;
`poll_readiness` asks each pane; `roster_is_wanted` asks whether any pane is
Claude. Every pane's kind is the session's, so every predicate answers today's
answer. The test is that it still does.

**Phase 2 — the feature.** `App` holds the table; `Recipe` carries the row and
the kind; `start_agent` takes a choice; the `choosing` question and the `A`/`Shift`
gesture; `AgentRequest`. The preset-args fix and its test go here, because this
is the phase that would otherwise make the disagreement worse.

**Phase 3 — the disclosure.** `Target` carries the kind; the queue's status line
says "this pane cannot be typed at" rather than `Unknown`; the README says what
a non-Claude pane does not do. Phase 2 should not ship to anyone without this.

**Phase 4 — dispatch, if it is wanted.** The queue's dispatcher becomes
something that can arrive after a pane opens. Its own argument, its own
document if it turns out to want one.

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
