# More than one agent in the window

> A proposal, not a record. Nothing in here is implemented. It is written in
> this directory's voice because the point of it is the argument, and because
> the parts that survive should end up in module documentation rather than in a
> file called `proposal.md` that nobody reads twice.

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

## The field

```rust
pub struct App {
    left: TerminalPane,   // ← this
    ...
}
```

One hosted child, owning the left column outright. Twenty-one references to
`self.left` outside the tests, and each of them is a real decision rather than
plumbing: the readiness probe, the queue's send, the selection hand-off, the
resize, the title, the cursor, the exit.

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
becomes a count or a list of pane ids, and the list stops being able to say
"the" agent, which it should never have been able to say.

## What `main` has to hand over

`main` owns the spawn today. It resolves the command line, builds the one
`PtyConfig` out of `hosted.launch.config()`, spawns the pane and hands the
*result* to `App::new(left, root, agent, opening)`. To start an agent on a
keystroke, `App` needs the recipe rather than the result — `hosted` itself,
kept, so a later pane is built from the same `Launch` with a different `.cwd()`.

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
  the closing gesture is going to need.
- **Cycling is `F4` pressed again.** `F4` means "give the keys to the left" and
  today a second press does nothing at all. "Again" meaning "the next one down
  the stack" collides with nothing, costs no audit, and is a no-op in a session
  with one agent — which is every session that exists now. `F5` keeps its
  meaning untouched.

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
what makes hosted apps wrap strangely" comes from. A third function joins it —
`stack(left: Rect, n: usize) -> Vec<Rect>` — under exactly that rule, since each
pty is resized from the rect that drew it.

It needs the floor that `MIN_SPLIT_COLS` is on the other axis, and for the same
stated reason: *collapsing is the right degradation; squeezing is not.* Below
some rows per agent — twelve inside the border is a guess worth measuring, not a
number to write into the code on my say-so — the stack must stop expanding and
start collapsing.

**Collapsed, not hidden.** A pane that is not the current one shrinks to its
title row rather than disappearing. One row per agent keeps the roster and the
busy/idle signal on screen — which is most of what "see what multiple agents are
doing" actually means — and it degrades continuously as N grows instead of
falling off a cliff at the point where the window runs out. It also means the
one-visible-at-a-time mode and the two-visible mode are the same code path with
a different number in it.

`Alt+Z` is untouched. It answers "is the right pane here", which is orthogonal.

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

## The exit contract

Today the agent leaving ends abeam, `main` prints its last screen to the primary
buffer and exits with its status. `abeam -p "fix the tests" && next-step` depends
on that, and `Outcome::Exited` is single-valued because there is one child.

**The agent abeam was started with is the one that ends the session.** Panes
opened afterwards that exit freeze on their last screen under a title saying so,
until they are closed; they are not the session and their status is not the exit
code. The alternative — last one out — makes the exit code of a scripted run
depend on a pane somebody opened by hand, which is the kind of thing that is
noticed months later.

`any_shell_live`'s rule extends unchanged and should: a live agent holds the
door open at quit for exactly the reason a live shell does, and the title
already has a place to say why abeam is still here.

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
  registered once, on `self.left`, inside `App::run`. Every pane spawned later
  needs the same sender, and a pane whose waker was forgotten does not look
  slow, it looks frozen.
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

1. **The refactor alone.** `agents: Vec<Agent>` and `at_agent`, with exactly one
   agent in the vector and no user-visible change whatsoever. The seven fields
   move, `Focus` does not, and the existing tests are the proof. All of the
   architectural risk is here and none of the design risk.
2. **A second agent, one visible at a time.** `a` in the worktree list starts
   one; `F4` again cycles. Already the feature, minus the simultaneity — several
   sessions on the go, each with its own workspace, switched between in one
   keystroke.
3. **The stack.** `layout::stack`, the rows floor, collapsed title rows. This is
   the part the request actually asked for, and it lands on a foundation that
   two phases of tests have already been run against.
4. **The rest of the contract.** Queue targeting, the exit rule, `agent_here`
   becoming per-pane, `docs/keymap.md` and `docs/status.md` updated.

Each phase ships something on its own, and phase 1 ships nothing, which is the
point of it.

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

- **Closing a pane.** Killing a live agent is the most destructive thing in the
  program and there is no precedent for it — `Alt+Q` asks twice, which is the
  model, but it asks about the whole session. Probably a key in whatever list
  phase 2 gives the panes, with the same double-press.
- **How many is too many?** A hard cap is easy to defend and easy to resent. The
  rows floor may be the only cap needed, since it already refuses to expand a
  stack that cannot be read.
- **The diag pane** reads `self.left.diagnostics()`. It becomes the current
  agent's, which is almost certainly right and is worth one sentence in its
  border saying which.
