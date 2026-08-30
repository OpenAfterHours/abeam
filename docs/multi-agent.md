# More than one agent in the window

> **All four phases are built. This is a record, not a proposal.** It began as
> one, and the tense of a paragraph is not a safe way to date it, so: nothing
> below is waiting to be done. The argument is why the code is the shape it is;
> the parts that survived have moved into module documentation, and where the
> code disagrees with a paragraph here, the code is the answer and the
> paragraph says so.
>
> **What the four phases changed about this document**, so nobody has to diff
> it against the source:
>
> - `MIN_AGENT_ROWS` is **12**, written into `crate::layout` with the arithmetic
>   the section below asks for. The guess became a number with an argument
>   rather than a measurement; what would settle it is still unmeasured.
> - `stack` takes **three** arguments, not two — `stack(area, n, at)`. The
>   cursor is an input because the pane holding the keys must be one that is
>   drawn.
> - **Closing a pane is answered twice**, and both answers are `x`. Phase 3
>   took the easy half — `x` twice at a pane whose child has **exited**, in the
>   left column, safe because there is no child left to shadow. Phase 4 took
>   the other half and could not put it in the same place: `x` twice on a row
>   of the **worktree list** ends a *live* agent. The section below has the
>   argument for why the key had to move rooms.
> - **A zoom that shows one agent was declined**; the argument is in the section
>   below.
> - **Per-pane readiness arrived early**, in phase 3 rather than phase 4,
>   because a collapsed pane's title row has to say whether its agent is
>   working. `crate::agentstate::Readiness` grew a fourth variant for it.
> - **The queue's targeting landed in phase 4, as designed and later than
>   designed.** Phase 2 could not defer the *failure* it prevents, so it aimed
>   every send at `agents[0]` as an interim that was byte-identical for one
>   agent; phase 4 replaced the fixed point with a target on each item.
> - **`Row.agents_here` stayed a count and did not become a list of pane ids**,
>   which is one prediction below that did not come true. What the closing
>   gesture needed was not ids on a row but a *guaranteed* row: `workspace::rows`
>   now promises one for every agent's root, which the section on the roster
>   revises.
> - **Nothing was built for re-aiming a queued item.** It is written where it
>   was written and cannot be moved. That is a gap rather than a decision, and
>   it is named at the foot of the queue section.

## What is being asked for

Several hosted agents at once, visible together. Start one on a bug, start
another on a refactor in a second worktree, and watch both work without a
multiplexer and without a second copy of abeam.

The request came with two observations attached, and both are correct in a way
that shortens the design considerably:

- **The right pane already handles this.** It follows a *workspace*, and
  workspaces are worktrees; `w` in the git view lists them and `Enter` switches.
  Nothing about the git view, the reader, the shell or the ask needs a new
  concept to survive a second agent.
- **The pad can stay shared.** It is keyed by workspace root
  (`crate::panes::pad::store`), so two agents in one checkout already share one
  pad, and two agents in two checkouts already have one each. That is the
  behaviour anybody would have chosen, arrived at for free.

So this is not a rework of the right-hand half. It is one field on `App`.

## The field it started from

```rust
pub struct App {
    left: TerminalPane,   // ← this
    ...
}
```

That is what was there. One hosted child, owning the left column outright, with
twenty-one references to `self.left` outside the tests — and each of them a real
decision rather than plumbing: the readiness probe, the queue's send, the
selection hand-off, the resize, the title, the cursor, the exit. Sorting those
twenty-one into "every agent", "the session's" and "the current one" was phase
1, and it is why phase 1 shipped nothing a user could see.

## The shape to copy is already in the file

The right-hand half solved this problem once:

```rust
spaces: Vec<Space>,
at: usize,
```

and `spaces`' own documentation argues for a `Vec` and an index rather than a
map from root to workspace, on borrow grounds — `at` is a `Copy` `usize` that
can be read *before* the index, which is what keeps `App::right_pane_ref` a
plain `&self` method. Every one of those words is true again on the left.

```rust
agents: Vec<Agent>,
at_agent: usize,
```

Using the same shape is worth more than the code it saves. It means the
invariants are already written and already argued: index 0 is never removed,
`at_agent < agents.len()`, reconcile by identity and never by position because a
list can change length under a keystroke. `sync_workspaces` is a working example
of every one of those going wrong first.

## `Focus` stays two-valued, and this is the load-bearing decision

The obvious move is `Focus::Left(usize)`. It is the wrong one.

`Focus::Left` and `Focus::Right` appear 126 times across the crate. More than
the count, `focus = "left"` is a documented setting in the config file, and
`App::set_focus` is the single choke point that the whole "typing goes to the
agent" argument at the top of `app.rs` rests on — that module's promise is that
the callers of one function are the complete list of what can take your keys.
Widening the type re-opens every one of those call sites for review, to express
something that is not focus.

Because it genuinely is not. `Focus` answers *which side of the divider has the
keyboard*. Which agent within the left column is a cursor, exactly as `at` is a
cursor within the right. Two fields, two questions, and `set_focus` never
learns that a second agent exists.

## What moves out of `App` and into `Agent`

Six fields on `App` are secretly about the left pane, and finding them is most
of the refactor:

| today | why it is per-agent |
| --- | --- |
| `left: TerminalPane` | the child itself |
| `probe: Probe` | keyed by pid, cwd and disowned session ids — one session's record |
| `draft_open` | a half-written composer belongs to the agent it was typed at |
| `submit_pending` | the `Enter` owed to one composer |
| `agent_exit` | one child's status and last screen |
| `left_inner: Rect` | each pty is sized from the rect that drew it |

And one new field: `root: PathBuf`. Which brings up the constraint that decides
the rest of the design.

**`agent: String` was on that list and has been taken off it, which is a
decision rather than a correction.** It looked per-agent because it names an
agent. It is not: it holds `Hosted::agent` — the agent *behind* a preset, so a
preset called `fleet` hosting Claude answers `claude` — and what it decides is
whether `--bg` dispatch exists at all and what the per-workspace ask panes host.
Both are facts about the session, not about a pane. The border title is the
*other* string, `Hosted::name`, and `TerminalPane` has carried it since before
any of this, so a pane needs no name field of its own.

The cost of leaving it on `App` is a promise withdrawn: **panes of different
kinds are not expressible.** Every agent abeam starts is the one it was started
with, in another directory. Hosting Claude in one pane and Codex in the next is
a coherent thing to want and this design does not deliver it — `has_claude_state`
is a session-wide predicate standing in front of a now per-agent probe, and
making it per-pane is a change to the readiness path that wants its own argument
rather than a field quietly moved during a refactor. Written down here so the
next person reaches for it deliberately.

## An agent pane is pinned to a worktree, permanently

`Space`'s documentation says it twice already, once for the shell and once for
the ask: *a live child's working directory belongs to the child.* There is no
call that moves a running process to another directory. It is the reason the
left pane is not in `Space` in the first place — that comment opens with "the
left pane is not in here, and that is the whole shape of the feature".

Nothing about a second agent changes that. So an agent pane is born in a
directory and dies in it, and the honest thing is to name it in the pane's own
border rather than to hide the asymmetry. Which repairs something: today the
worktree list marks the agent's root separately from the one being read, because
there is one agent and it may be somewhere else. With a pane per agent, every
pane says where it is standing and the apology becomes a label.

**Where a new one starts: the workspace the right pane is on.** No new concept,
no path to type, and the gesture is already in the user's hands — `Alt+G`, `w`,
move to a worktree, and the new agent starts there. `Row.agent_here: bool`
became `agents_here: usize` — a count and not the list of pane ids this
paragraph offered as the alternative; the section below says why — and the list
stopped being able to say "the" agent, which it should never have been able to
say.

**That count is not a phase-4 nicety, and phase 2 pulled it forward on being
told so.** It is the load-bearing half of the argument for putting the key in
this list rather than anywhere else: the list is where you can already see who
is working in which checkout, so the gesture that fixes an empty worktree is
next to the evidence that it is empty. A row that went on reading empty after
`a` had started something takes that away — and takes with it the reason the key
needs no confirmation, since what a confirmation buys is that the second press
is an informed one.

**It is `agents_here: usize` and it stayed one, which is this paragraph's other
prediction not coming true.** A list of pane ids was going to arrive with the
queue's targeting, on the reasoning that targeting is what first needs to *name*
a pane rather than count them. Targeting does name panes — by
`crate::app::Agent::id`, on the item — and it never asks this list about them:
it asks the shell, which holds the vector. What the list needed turned out to be
something else entirely, and it came from the *closing* gesture rather than the
queue: `x` on a row has to resolve a checkout to a pane, and it resolves it in
`crate::app::App::agent_in` against the same vector. Ids on a row would have
been a second copy of what `crate::app` already knows, kept up to date on a
ten-second discovery timer, which is exactly the kind of mirror this design has
been deleting.

**What the list did owe, and now pays, is a row for every agent's root.**
`workspace::rows` guaranteed a row for the workspace on screen and for the one
abeam was started in, and for a while it declined a third guarantee on the
grounds that an agent is started *from* a row so its root has one by
construction. That was true and too narrow — an agent standing in a worktree
`git worktree list` has stopped naming had no row at all — and it stopped being
survivable when the close gesture arrived, because a pane whose root has no row
is a pane with no way out, in the very list that is meant to be its roster. It
is a guarantee now, and the cost is the one that argument named: a row for a
directory git no longer mentions, carrying a directory name because there is no
branch to carry.

## What `main` has to hand over

`main` owns the spawn today. It resolves the command line, builds the one
`PtyConfig` out of `hosted.launch.config()`, spawns the pane and hands the
*result* to `App::new(left, root, agent, opening)`. To start an agent on a
keystroke, `App` needs the recipe rather than the result — `hosted` itself,
kept, so a later pane is built from the same program with a different `.cwd()`.

**Not from the same `Launch`, which is what this paragraph said and is the one
correction phase 2 made to it.** A `Launch` carries the command line, and
`abeam -p "fix the tests"` is in there: a pane opened on a keystroke and built
from it would re-run that prompt non-interactively in a worktree nobody wrote it
about and exit as soon as it had answered, and `--resume` would resume a
conversation belonging to somewhere else. Blanking the arguments is not the fix
either, and the reason is the install shape most people have: an npm `claude.cmd`
is routed through `cmd.exe`, so the arguments are the interpreter's wrapper and
the user's prompt is in the *environment* — dropping one and keeping the other
keeps the prompt and loses the agent. What is kept is `Launch::target`, the file
that does the work, and `crate::launch::resolve` is asked again with no
arguments at all. `crate::app::Recipe` is where that lives.

It is a small move and a safe one, and the safety is not incidental.
`crate::launch`'s guarantee is that nothing leaves that module which is not an
absolute path, which is what makes a spawn issued later, from a process that has
deliberately gone to stand somewhere unwritable, mean the same thing as the
spawn issued at startup. The `somewhere_unwritable()` defence in `main` is
untouched: every `PtyConfig` abeam opens is given an explicit `.cwd()`, and the
new ones are given the workspace root — the *resolved* spelling of it, for the
reason `main` already spends a paragraph on, since `crate::agentstate::Probe`
compares a path abeam holds against a path the child wrote into its own record.

`Probe::new` needs its clock read at the moment of the spawn — `App::new`'s
comment says the record it is looking for is the one written *after* that
moment. With panes created later that argument does not weaken, it just moves:
each `Agent` reads its own clock as it is constructed.

## The keys, which looked like the hard part and is not

`docs/keymap.md`'s invariant: nothing abeam intercepts may be a key any hosted
agent can act on. The namespace is close to spent. `F1`–`F9` and `F12` are
abeam's; `F10` activates the menu bar in several emulators and `F11` is
fullscreen in most, so both are eaten before an application sees them — that
ruling is about *terminals*, so no fresh audit can recover them. Under `Alt`,
`g e s q z k j` are abeam's, `a` is Codex's, `←`/`→` are Copilot's, and `t f b d
y u l c` are Claude's declared table or the readline set its prompt editor
handles without declaring. Three new global keys — next, new, close — is not a
budget that exists.

It does not need to. `docs/keymap.md`'s invariant is about *global* bindings,
and `GitPane::worktree_key` is the standing proof: `w` and `Enter` and `Esc` are
claimed there with no audit at all, "exempt from the global invariant because it
is only ever delivered while the right pane has focus and this view is up".

So:

- **Starting an agent is a key in the worktree list.** `a` on a row, which is
  free there. It is the right place on its own merits, not only the cheap one:
  the list already shows who is working in which checkout, via the roster's
  occupancy column, so "there is nobody in that worktree" is already on screen
  next to the gesture that fixes it. It also gets a confirmation surface, which
  the closing gesture turned out to need: `x` twice on a row is where a *live*
  agent is ended, and this paragraph is the reason there was somewhere to put
  it.
- **Cycling is `F4` pressed again.** `F4` means "give the keys to the left", and
  before this a second press did nothing at all. "Again" meaning "the next one
  down the stack" collides with nothing, costs no audit, and is a no-op in a
  session with one agent. `F5` keeps its meaning untouched.

That is one repurposed key and one pane-local letter, for a feature that looked
like it needed three globals. If cycling in one direction turns out not to be
enough, the answer is a row in a list, not `Shift+F4` — a modified F-key is
deliberately not abeam's, and `keys::global` says so in a comment about `Ctrl+F12`.

## The layout

**Stacked, not side by side.** At 120 columns the left column is 72; two agents
abreast is 36 each, which is below what any of these agents can draw. Rows are
the cheaper axis: a 40-row window gives two agents about 19 each.

`crate::layout` opens by saying there is one calculation and it is called once
per frame, because two calculations that must agree is where "off-by-one here is
what makes hosted apps wrap strangely" comes from. A third function joins it,
under exactly that rule, since each pty is resized from the rect that drew it.

**It was sketched here as `stack(left: Rect, n: usize)` and it is
`stack(area: Rect, n: usize, at: usize)`, which is phase 3's one correction to
this section.** Which panes collapse cannot be decided from `n` alone: the pane
holding the keys has to be one that is drawn, or the reader is typing into a
title row with no cursor and no screen. So the cursor is an input, and the rule
is *the pane with the keys first, then list order from the top*.

It needs the floor that `MIN_SPLIT_COLS` is on the other axis, and for the same
stated reason: *collapsing is the right degradation; squeezing is not.* Below
some rows per agent the stack must stop expanding and start collapsing.

**Twelve is what went in, and the caveat this paragraph made has been half
answered.** It said twelve was "a guess worth measuring, not a number to write
into the code on my say-so", and it is now in the code — as an argument rather
than a measurement. `crate::layout::MIN_AGENT_ROWS` adds up what the rows are
spent on: five of permanent furniture, seven of transcript, which is what a
permission prompt needs to be on screen at all. What would still settle it is
the measurement nobody has made, and that constant is where it goes.

The arithmetic it implies is worth having here too, because it decides how much
of this feature most people ever see: a whole pane is twelve plus its border, so
**two agents want 28 rows and three want 42**. A 24-row terminal draws one agent
and a title row, whatever the user does.

**Collapsed, not hidden.** A pane that is not the current one shrinks to its
title row rather than disappearing. One row per agent keeps the roster and the
busy/idle signal on screen — which is most of what "see what multiple agents are
doing" actually means — and it degrades continuously as N grows instead of
falling off a cliff at the point where the window runs out. It also means the
one-visible-at-a-time mode and the two-visible mode are the same code path with
a different number in it.

**That row is why `Readiness` grew a fourth variant, which is a phase-4 cost
paid in phase 3.** "The busy/idle signal" was the easy half. The signal that
matters is an agent *stopped on a permission dialog* — the one a reader has to
go and answer — and `waiting` mapped to `Unknown`, which a border draws as
nothing. So the agent you most need to be told about was the one the row stayed
silent about, in the feature whose floor is set by keeping permission prompts on
screen. `Readiness::Waiting` splits that refusal out. It cannot widen the send
gate: every gate in the program tests for `Idle` and nothing else, so splitting
a refusal in two is not loosening an acceptance.

## `Alt+Z` is untouched, and there is no second zoom

`Alt+Z` answers "is the right pane here", which is orthogonal — and, more to the
point for a *vertical* stack, it buys the left column **columns**. Hiding the
right pane makes every agent wider and not one of them taller.

The different question a stack raises is "show me only this agent", and phase 3
declined it. Three reasons, in order of weight:

- It would cost a **global binding** out of a namespace this document records as
  spent, and `F4` is already carrying two meanings for this feature.
- Its whole effect is to **delete the roster**, which is the thing collapsed
  rows were designed to keep. A mode that hides the other agents' busy signals
  is not obviously the feature it sounds like, in a feature about watching
  several agents.
- The stack already produces that shape at its floor, though not on demand: one
  pane whole and the rest as title rows is what a short window or a fourth agent
  gives you. That is a weaker argument than it first looks — arriving somewhere
  by resizing a terminal is not the same as a keystroke — which is why it is
  third rather than first.

If it comes back, the honest form is the one `keys::global` prescribes for the
reverse-cycle key: a row in a list, not a new chord.

## The sentence in the queue that becomes false

`crate::panes::queue`, on `Mode::Send`: *"There is one left pane, so these are
strictly sequential."*

That is not a passing remark. That module states the four conditions for a send
as a **count**, and says the count is load-bearing — every argument in the file
about what may be skipped is an argument about which of the four it is.
Conditions 2 and 3 are "`crate::agentstate` reports idle" and "nothing is
sitting unsubmitted in the composer", and with several agents both questions
have to name one.

So a `Send` item carries its target, and the target is decided when the item is
enqueued rather than when it fires — otherwise a prompt written for the agent
you were watching lands in whichever pane happened to be current when the
announcement elapsed, which is precisely the failure that module exists to
prevent. A **pane id, not an index**: panes come and go, and `sync_workspaces`
already has the argument about why an index does not survive a list changing
length underneath it.

If the target has gone, the item disarms with a note. It is not retargeted.
`Mode::Dispatch` needs none of this — it never types at anybody — and gains an
obvious neighbour it should probably not gain yet: "dispatch, then open it in a
pane" is a real feature and a different one.

**That is what was built, and four things about it were decided in the
building.**

- **The aim is the pane with the left column's cursor when the item is
  written.** "The agent you were watching" is what somebody writing a prompt
  means, and it is the one answer that needs no key of its own — which matters,
  because this document has already spent the key budget. It is stamped in one
  function, `QueuePane::push`, and nothing else assigns to it: `m` switching an
  item between the two modes and back leaves the agent it was written for
  alone.
- **The queue holds a roster, not a reading.** The pane cannot ask the shell
  anything — `Pane::tick` re-asks the four conditions on a loop the shell is not
  standing in — so `crate::app` pushes a row per agent, every quarter second and
  again on every keystroke that opens a draft. That is the shape that makes
  conditions 2 and 3 answerable *about an arbitrary pane*, and it also disarms
  the landmine phase 1 left: a per-agent draft flag whose only reader was
  `agents[0]`'s. Whatever polls a pane's record is what clears its draft, so a
  target nothing polls has no gate in front of it at all.
- **An orphan leaves `Pending`, and that is not cosmetic.** `next_send` takes
  the first pending `Send`, so an undeliverable item left pending parks itself
  at the head of the queue for ever and every item written afterwards waits
  behind a prompt that is never going anywhere — the automatic sender off for
  the rest of the session with `armed` still on the status line, which is the
  silent-stall shape `crate::agentstate` refuses by name.
- **A blocked item is walked past, not stopped at.** The queue took the first
  pending `Send` whatever it named, which is right for one destination and a
  permanent stall for several: an agent sitting on an unanswered permission
  dialog parked itself at the head and held every *other* agent's prompts behind
  it. The search asks each item's own target, so it yields the first one that
  could actually go. Three properties survive the skip and each was checked
  rather than assumed — at most one item due at a time, at most one send in
  flight per agent, and order preserved *within* a conversation, which holds
  because eligibility is a fact about the target and two items naming one agent
  are always eligible together.
- **The announcement moved with the send.** Phase 3 put the countdown at the
  *front* of `agents[0]`'s border, because appended it was clipped off the end
  by the pane's own name; phase 4 has to choose a border as well as a position,
  and it is the target's. A three-second warning on the wrong pane's title is a
  reader watching the wrong composer. The one case where it cannot be the
  target's is a window with fewer rows than agents, where `layout::stack` gives
  some panes no rows at all; it is borrowed onto a neighbour's border there and
  the shell adds the target's name, because which border a note ended up on is a
  fact about the layout and belongs to whatever chose the layout.

**What an orphan actually looks like, because "disarms with a note" is three
surfaces and not one.** The row's marker changes to `⊘` and its aside says
`<pane> closed`, naming the pane off the label snapshotted when the item was
written — which is the only thing left to say about an agent that no longer
exists. The pane's status line stops counting it as work still coming. And the
agent border's low-ranked note counts it separately from the failures:
`queue 2 · 3 undeliverable`. That third one is the one that was missing at
first, and leaving it out made loss look like progress — kill a pane with three
prompts queued for it and `queue 5` became `queue 2`, which is what a border
says when work has been *done*.

**Where a queued prompt is delivered is a fact about the borrow checker.** The
queue hands the shell whatever the shell needs in order to write — and what the
shell asks for is a `&mut` to the agent itself, not its position. The pane that
was vetted is therefore the pane that is typed into by construction: the borrow
is live across the write, so no statement can be inserted between the two that
pushes to, removes from or reorders the vector. With an index the property was
true and one line away from false, which is how the whole class of bug this
design is about got in the first time.

**A record belongs to one pane, and the two ways that went wrong are cousins.**
A killed child does not tidy up, so its session record can outlive it and be
adopted by the next agent started in that worktree; and an agent started
*beside* another in the same root has no record of its own for a second or two,
during which the newest one in that directory is its neighbour's. Both end with
a probe reporting somebody else's `status`, and if that somebody is idle the
answer is `Idle` — the one answer that lets a prompt be typed. Neither becomes
permanent, because `agentstate` re-asks `started_at >= spawned_at` on every call
and never memoises an older record. Both are closed the same way and by the one
party that can: `crate::app` disowns a pane's record as the pane closes, and
tells a new pane's probe about every record a pane on screen has already
claimed. Narrowing `Probe::search`'s clock-skew fallback to a *window* would
help with both and is a change to the discovery rule with its own argument to
make; the note at that `or_else` says what it would cost.

**Two things this feature did not break and did expose, which is the more
useful way to record them.** Condition 3 was tested as a *level* rather than an
edge — `busy && draft_open` clears, whenever both happen to be true — and Claude
takes typing while it is working, so a follow-up typed into a mid-turn composer
was cleared within a quarter second and the queue pasted on top of it. That was
already wrong with one agent; what phase 4 changed is that it is now wrong at
every pane rather than one. `Agent::draft_mid_turn` makes it an edge, and the
direction of failure is stated where it lives: no rule survives a turn that
begins and ends inside one poll interval, so the choice is between a splice and
a stall, and it takes the stall. And a `Send` was marked sent before the write,
with the write's answer discarded — so a pty that refused one left `✓` over a
prompt that was simply gone. The item goes `Failed` now, and the pane a child
has just left is refused before the write is attempted.

**What is not built: there is no way to re-aim an item once it is written.** The
target is decided at enqueue and nothing moves it, which is the property the
whole section is about — and the honest reading is that the property was easy to
get right by refusing to offer the feature. Re-aiming wants a key in this pane's
list, and a key here is cheap to add and impossible to take back. The way to aim
somewhere else today is `F4` to that pane and write it there.

**Phase 2 could not defer all of this, and what it did instead is worth writing
down because it is not the interim anyone would have guessed.** The failure
above arrives the moment a second pane exists, by two routes with no new feature
between them: `F4` during the three-second countdown, and `a` on another
worktree while an item is armed. The obvious interim — stand the queue down
whenever the cursor is elsewhere — makes a feature stop working for a reason
nobody can see. So the queue is aimed at **`agents[0]`, the session's agent,
outright**: not the pane with the keys, and not a target the item carries.

That is byte-identical while there is one agent, and it is defensible on its own
terms rather than as a placeholder — a `Send` continues *this session's*
conversation, and the session is the agent whose exit is abeam's exit. The rule
it makes explicit is the one the bug broke: **the queue's three inputs must name
one agent** — the readiness it reads, the `draft_open` it gates on, and the pty
it types into. It also deleted work rather than adding it, because the
per-agent draft flag no longer has to be kept in step with the queue's copy as
the cursor moves; that syncing was itself the mechanism of a second bug.

The cost is one sentence long and is a labelling problem: with one agent drawn
at a time, the countdown can appear in a border describing a pane the send is
not going to. The stack fixes it by drawing `agents[0]`'s border at the same
time as everybody else's. Everything above this paragraph still stands as the
end state; what changed is that phase 2 is no longer sitting on a live
misdelivery while it waits.

**Phase 4 promoted that interim rather than reversing it, and the distinction is
worth keeping.** Both versions refuse the same thing — a target read at the
moment of delivery — and phase 2's rule that *the queue's three inputs must name
one agent* survives word for word. What changed is that the one agent comes off
the item instead of out of a constant. A reader who finds `agents[0]` in this
file's history should not read it as a bug that was fixed; it was the same
invariant with a fixed point, and the fixed point is what went.

**Phase 3 paid that and found the bill was larger.** Putting the countdown on
`agents[0]`'s border is only worth anything if it is *legible* there, and it was
not: the note was appended to the end of a line clipped from the right, behind a
pane name, a position, a worktree label and — in the very state that produces
this — an exit status. So the announcement was on the line and off the screen.
It now **leads** that border, in front of the pane's own name, which is the
treatment `App::right_title` already gives the one thing on a border a reader
has to act on. `QueuePane` reports its note in two parts of different rank
rather than one string, because only that pane knows which is in play and only
the shell owns the columns.

## The exit contract

The agent leaving ends abeam, `main` prints its last screen to the primary
buffer and exits with its status. `abeam -p "fix the tests" && next-step` depends
on that, and `Outcome::Exited` is single-valued because there is one *session*.

**`agents[0]` alone ends the session, and it is the only one that can.** Not the
last one out, and not whichever pane has the keys. The loop reads
`App::session_agent` and so does `App::finish`, which are named that way so the
sites that mean "whose exit is abeam's exit" cannot be spelled the same as the
sites that mean "whose keys are these" — with one agent those are the same
object, so the two names are the whole of what stops a later edit picking the
wrong one. The alternative makes the exit code of a scripted run depend on a
pane somebody opened by hand, which is the kind of thing that is noticed months
later.

**A pane opened afterwards that exits freezes on its last screen under a title
saying so, and abeam stays up.** The screen is frozen because nothing clears a
`vt100::Parser` and nothing has to: the pane goes on rendering what the child
left. The title says so because `TerminalPane::title` appends `· exited (n)`
once `poll_exit` has reaped the child — which `App::reap` does for *every*
agent, and that is the load-bearing word. Nothing else in the loop calls
`try_wait`, so an agent nobody reaped could never be observed to have left, and
its border would go on naming a live session while `has_exited` answered `false`
to the readiness read and to the selection hand-off, both of which consult it
before they will type at a pty.

`any_shell_live`'s rule extends unchanged and should: a live agent holds the
door open at quit for exactly the reason a live shell does, and the title says
which — `another agent · Alt+Q to quit`, or `shell open · Alt+Q to quit`, one of
the two and never both, with the agent leading because it is the more expensive
thing to end. Without that word the window merely looks stuck, and the one thing
the reader needs to know is that something of theirs is still running.

**Closing a live agent is the exit contract's other half, and it is the most
destructive thing in the program.** `agents[0]` may never be closed — its exit
is the status code — and every other pane may, twice over, from the worktree
list. The kill itself is nobody's code: removing the element drops the `Agent`,
the `TerminalPane` and the `PtySession`, whose `Drop` kills the child and then
closes the process group or job object it was started in, so a `cargo build` the
agent had running goes with it rather than being orphaned onto `init`. That is
the same teardown every child gets at the end of a session and it has the
`abeam-pty` test named for it; a second explicit kill here would be a second
thing to keep correct.

**Why the key is in a list and not at the pane**, which is the one decision in
this phase that reads like an inconvenience and is not. `x` at an agent pane is
legal only because the child has exited: there is no process left to hold a
binding, so the letter cannot shadow anything. A live child *is* listening.
Intercepting `x` in front of one eats the letter out of every word typed at it;
forwarding it and arming a confirmation anyway makes `box`, or any second `x` in
a sentence, end a running session. And there is no global left — `docs/keymap.md`
records the `Alt` namespace as close to spent, and taking a new letter means
repeating that document's whole extraction against three agents' current builds,
which is a claim nobody can make on an afternoon. What remains is the exemption
the worktree list already stands on, which is where `a` lives, and the detour
turns out to be part of the guard: `Alt+G`, `w`, find the row, `x`, `x` is
harder than two presses at a dead pane by about the margin a running agent
deserves. The confirmation is drawn on the border of the pane it would destroy
rather than in the list, because `x` `x` in a list is two presses on a key with
no memory of what it destroyed.

Two panes in one checkout are one row, and abeam refuses rather than guessing —
the answer counts them and says `F4` to the one you mean, which makes the
gesture two-factor exactly where it is ambiguous. **What that costs is five
keystrokes and the message says so**, because the first version said "F4 to the
one you mean" and that reads as one: the keys are in the right pane, so `F4`
gives them to the left column, a second `F4` moves the agent cursor, `F5` brings
them back, and then `x` twice. A refusal that undercounts its own remedy by four
presses is a refusal nobody follows.

**And a confirmation nobody saw does not count.** abeam drains every queued
input event before it draws, so two `x`es in one batch — key repeat, a fast
double tap, a pasted `xx` — would arm and answer the question with the words
never on a border. The kill is refused unless the last frame actually painted
them, which also covers the two ways a border can fail to: a window with fewer
rows than agents paints nothing into some panes, and a countdown leads the same
slot and can take the columns. The refused press becomes the *first* press
rather than nothing, so the reader who meant it gets what they meant one press
later.

**How a refusal reaches the reader at all** is `agents[0]`'s border, which is
where every answer about the roster of panes goes — a pane that failed to start,
a pane that must not be closed, a row abeam cannot resolve. It is elided from
the left, so every one of those sentences is written with the half a reader can
act on at the *end*: `` `a` starts one ``, `` then x ``, `` Alt+Q is the way
out ``. Below thirty columns of spare border there is no room for a sentence at
all and it is replaced by `refused · widen to see why`, which is the only thing
true of all five writers — the string it replaced said `no agent started`, which
described one of them and was printed on the border of a visibly running agent
the first time somebody pressed `x` on the session's own row.

## What it costs, said out loud

- **Processes.** Each pane is a whole agent. This is a "start another agent"
  feature wearing a layout feature's clothes: the memory is the agent's, and so
  is the quota. `crate::dispatch` already discloses this about background
  agents; here it is at least visible.
- **A thread and a parser per pane.** `abeam_pty::spawn_reader` is one thread
  per session, and each session holds a `vt100::Parser` with `PtyConfig`'s
  default 5,000 lines of scrollback. Whether a collapsed pane should keep 5,000
  is a question to answer with a measurement, not with an opinion.
- **Frames.** Every pane rings the output doorbell, and `MIN_FRAME`'s coalescing
  is exactly the mechanism that makes several of them survivable — a burst
  becomes one frame either way. The trap is not throughput: `wake_on_output` is
  registered once, on `self.left`, inside `App::run` — which was the warning,
  and it is answered: `Agent::arm_waker` is the one call that arms one,
  `App::arm_wakers` makes it over every agent that exists, and `App::wake_tx`
  keeps the sender for the panes that do not exist yet. It is recorded rather
  than deleted because the failure it names is invisible — a pane whose waker
  was forgotten does not look slow, it looks frozen, and only under output that
  nothing else coincides with.
- **Rows.** `App::right_title` calls them the scarcest resource in a two-pane
  TUI, and the stack is what makes that a three-pane one. Each agent's border is
  a row gone.

## What this is not

It is not a multiplexer, and that has to be argued rather than asserted, because
`docs/design.md` opens by justifying abeam against wezterm and lazygit on
exactly one claim: **the right pane knows what the agent just did.**

Two agents in two worktrees keep that claim intact — `workspace::owner`'s
innermost-ownership rule already routes each write to the pane that owns it, and
it was written for this exact situation, since Claude Code makes worktrees
inside the watched root and runs agents in them. Two agents in *one* worktree
lose the ability to say which of them wrote a file. But the right pane never
claimed to: it describes a checkout, not an author, and `git status` cannot tell
you either. The central claim survives, which is the test this feature had to
pass.

## Phasing

1. ~~**The refactor alone.**~~ **Done.** `agents: Vec<Agent>` and `at_agent`,
   with exactly one agent in the vector and no user-visible change whatsoever.
   The seven fields moved, `Focus` did not, and the existing tests were the
   proof. All of the architectural risk was here and none of the design risk.
2. ~~**A second agent, one visible at a time.**~~ **Done.** `a` in the worktree
   list starts one; `F4` again cycles. Already the feature, minus the
   simultaneity. The occupancy count left phase 4 and landed here — a key whose
   only effect is invisible is a key nobody presses twice, and that was not a
   cost phase 2 could book and defer.
3. ~~**The stack.**~~ **Done.** `layout::stack`, the rows floor, collapsed title
   rows. It brought four things this list did not have: closing a pane with `x`
   (the exited ones only), per-pane readiness and `Readiness::Waiting` for the
   collapsed row, per-pane mouse routing, and the resize of *every* pane rather
   than the current one — which was a live bug from the moment phase 2 could
   make a pane you were not looking at.
4. ~~**The rest of the contract.**~~ **Done.** Queue targeting by pane id, with
   the target stamped at enqueue and an orphan when the pane it names has gone;
   the exit rule made explicit and tested, including what closing a *live* agent
   means, which phase 3 deliberately left refused. Two things this list did not
   have: the closing key had to move rooms — a live child is listening, so `x`
   ended up in the worktree list rather than at the pane — and
   `workspace::rows` gained a guaranteed row per agent, without which a pane in
   a worktree git has stopped naming would have had no way out. `docs/keymap.md`
   has been kept current as each key landed; `docs/status.md` is updated here,
   and its entry is mostly about what nobody has watched work.

Each phase ships something on its own, and phase 1 shipped nothing, which was
the point of it.

## The right pane does not follow, and this is settled

This was the last open question and it has an answer: **cycling agents must not
move the right pane.** Not to the new agent's worktree, not to a different view,
not at all.

The case for following was convenience, and it was wrong. Somebody watching an
agent is *reading* — the document it just wrote, a diff, a shell's output — and
the reason they reach for the agent cursor in the first place is that they have
noticed something they want to say to another agent. Making that keystroke throw
away the place they were reading punishes exactly the gesture the feature exists
to enable: they went to the keyboard to *add* something, and it cost them what
they were looking at.

It is also the rule the program already has. `app.rs` opens with it — the panes
"never switch themselves", and a pane that yanks itself into view while you are
reading "is delightful twice and infuriating thereafter". The precedent I nearly
leaned on is `Enter` on a file in the git view, which does switch the reader; the
difference is that there the switch *is* the request. Here the request is "give
me that agent's keyboard", and the right pane is not mentioned in it.

So: the agent cursor and the workspace cursor are independent, and neither
writes to the other. The cost is real and worth naming — you can be typing at an
agent in one worktree while reading the git status of another, with only the two
borders to tell you so. That is a labelling problem, and the borders already
solve it: each agent pane says which root it is standing in, and the right pane
has said which workspace it is on since the day there was more than one.

A test pins it: cycle the agent cursor, assert `at` and `right_view` are
untouched.

## Open questions

- ~~**Closing a pane.**~~ **Answered, in two places, and the section's own guess
  was half right in a way worth recording.** It guessed a key in a list; phase 3
  put the easy half in the left column instead — `x` twice at a pane whose child
  has **already exited**, `Alt+Q`'s double press one pane down — on a safety
  argument this section did not anticipate: the letter is legal there because
  *the child that would have received it has gone*. Phase 4 then found that the
  hard half could not go in the same room, because a live child is listening,
  and put `x` twice on a row of the worktree list, which is where this section
  guessed it would be all along. So the answer is a bare letter in two places
  with two different arguments for it, and the wording differs — `close this
  pane` against `kill this running agent` — because what the second press
  destroys differs: a frozen screen and its scrollback, or a turn somebody is
  paying for. `q` was refused in both: it is documented as the way *out* of the
  right pane, and one letter for "leave this" and "destroy this" is a shared
  vocabulary teaching a mistake.
- **How many is too many?** Still open, and now with two measurements against
  it. The rows floor is a soft cap that bites earlier than it sounds — 28 rows
  for two agents, 42 for three — so a 24-row terminal is capped at one whole
  pane by arithmetic. And each live pane costs a readiness poll: 66 µs in the
  steady state, which is nothing at ten panes, against 11 ms while a pane cannot
  find its session record, which is a startup transient per pane and would
  matter if it ever stopped being transient. `App::poll_readiness` carries both
  numbers and the shape that would go wrong.
- **Two later agents cannot be read together.** New, and a consequence of the
  stack rather than a gap in it: rects come out in list order and panes never
  swap places, so with three agents and room for two, the two are `agents[0]`
  and whichever has the keys. Moving the cursor to `agents[2]` collapses
  `agents[1]` on the way past. Comparing two panes neither of which is the
  session's would need a second cursor or pinning, and either is a feature.
- **The diag pane** reads the current agent's `diagnostics()`, which is almost
  certainly right and is still worth one sentence in its border saying which.
- **A queued item cannot be re-aimed.** New in phase 4 and named in the queue
  section: the target is stamped when the item is written and nothing moves it,
  which is the property that whole section exists to protect, and it was easy to
  hold because nothing offers the feature. Somebody who writes three prompts and
  then realises they meant the other pane has to write them again. The honest
  form is a key in the queue's own list, which needs no audit; the reason it is
  not there is that a key which re-points a prompt is a key that can misdeliver
  one, and it wants its own argument rather than a spare afternoon.
- **Two agents in one checkout are one row, and one name.** The worktree list
  counts them and cannot tell them apart, so `x` there refuses and points at
  `F4`; the queue's rows call both by the same worktree label. A position would
  distinguish them and is exactly what closing a pane changes, so it is the one
  thing that must not be used. What would fix it is a name a person chose —
  panes have none, and giving them one is a feature with a keystroke and a
  border in it.
- **Nobody has watched any of this work.** Not an open question about the
  design, and the most important line in the section all the same. See
  `docs/status.md`, which is the document that keeps tests and use apart.
