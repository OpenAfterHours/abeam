# Learning from the person at the keyboard

> **A proposal, not a record.** Nothing below is built. It is written in the
> shape `docs/multi-agent.md` was written in before its four phases landed, and
> it should be edited into a record the same way if it is taken up.
>
> The measurements in "What is already on this machine" were taken on
> **2026-08-30**, from this user's own `~/.claude` on this Windows machine.
> They are one person's corpus, not a population, and every number below should
> be read as evidence that a mechanism *exists* rather than as a rate anybody
> else would see. Where a number is load-bearing the section says so.

## What is being asked for

Users of abeam are, all day, doing the one thing that produces supervision for
free: reading what an agent wrote and saying what was wrong with it. That
feedback is spent the moment it is given. The next session starts from nothing,
the session after that makes the same mistake, and the same sentence gets typed
again — in this project, and in the twenty-two others.

So: capture it, and have the next session start knowing it. Within a project,
and across them.

## The short answer

**Yes, and abeam is the only component in the stack standing where it can be
done.** Not because it is clever, but because of where it sits: it is the one
process that sees the agent's turn boundary, the diff that turn produced,
whether the work survived, and the sentence the human typed next — at the
moment all four are true. Every other candidate for this job sees one of the
four and has to guess the rest.

Three findings shape the whole design, and each of them narrows it:

1. **The evidence has a thirty-day half-life.** Retroactive mining is not
   available. Capture has to happen live.
2. **A memory store already exists, is already profile-side, and is already
   loaded into every session.** Storage is not the gap. Capture, review and
   scope are.
3. **abeam's own rules — the untrusted repository, the one file it writes, the
   untouched argv — decide the design before a line is written.** They are not
   obstacles to route around; they produce a better answer than the obvious one.

## What is already on this machine

### The rich record evaporates; the thin one survives

`~/.claude/history.jsonl` holds **4,337 prompts, across 986 sessions and 23
projects, back to 2026-01-03**, in **1.5 MB**. It is the durable record, and it
is one-sided: what the user asked, never what the agent did or said.

`~/.claude/projects/<key>/<sessionId>.jsonl` holds the whole conversation —
every message, every tool call, the file-history deltas. It is the record worth
learning from. There are **178 of them, 313 MB, and the oldest is dated
2026-07-31** — thirty days before the measurement.

> **Transcripts survive for 152 of the 986 sessions. For 834 of them — 85% —
> the transcript is gone.**

That single number is the argument for building this into abeam rather than as
a nightly job that reads `~/.claude`. A miner that runs on a schedule can only
ever see the last month, and it will see less as the corpus grows: 313 MB for
one month against 1.5 MB for eight is not a ratio that survives being kept.
Whatever is going to be learned from a session has to be extracted while the
session is still alive — which is abeam's whole position in the stack.

### There is already a memory store, and it is unevenly fed

`~/.claude/projects/<key>/memory/*.md`, with a `MEMORY.md` index, loaded into
context at the start of every session in that project. It is profile-side —
`~/.claude`, not the repository — which, as the next section argues, is exactly
where a store like this has to be, and it got there without abeam.

What it holds today:

| | |
| --- | --- |
| Memories in total | **137** |
| Top-level project directories | 11, of which **10** have a memory directory |
| Held by two projects (`mooring`, `rwa-calculator`) | **115 of 137 — 84%** |
| `forge`, after 140 prompts | **3** |
| `kedge`, after 164 prompts | **0** |

The store is not broken. It is written when the agent happens to think of it,
which correlates with nothing — not with how much work a project has had, not
with how much correcting it took. Nobody is asked, nothing is reviewed, and the
user never sees a diff of what was remembered on their behalf. **That is the
gap, and it is a capture-and-review gap rather than a storage one.**

### Memory fragments exactly where abeam's own state does not

Of the 45 directories under `~/.claude/projects`, **34 are git worktrees or
scratchpads** — `TheCoachesApp--claude-worktrees-ci-master-flake` and its
kind, the directories Claude Code creates inside a repository and runs other
agents in. They hold **45 transcripts** between them. **Not one has a memory
directory**, and none appears in `history.jsonl` at all, because the sessions
running there are non-interactive and nobody is at the keyboard to teach them.

So the memory store is keyed by *current directory*, and a worktree is a
different current directory. An agent working in `.claude/worktrees/x` gets a
memory store of its own, which nothing else will ever read, for a repository
whose real store is one level up.

abeam has met this exact problem and solved it. `crate::watch`'s module
documentation is about the day one recursive watch stopped being one workspace;
`crate::workspace` is the routing rule that came out of it; and the scratch pad
is keyed by **workspace root** precisely so that "two agents in one checkout
already share one pad". The routing rule this needs is written, argued and
tested already.

### Detecting feedback by keyword does not work

Worth reporting because it is the obvious first implementation and it should
not be built.

Classifying all 4,337 prompts with correction/preference/standing-rule patterns
matches **279 of them — 6.4%**. Reading the matches, a large share are false:
*"no, proceed with implementation"* classed as a correction, *"the delete
confirmation box is **always** visible"* classed as a standing rule, *"I want
to be able to publish this to PyPI"* classed as a preference. It is a bug
report, an instruction to continue, and a feature request.

The high-precision signals are **structural, not lexical**, and they are sparse:

| Signal | Occurrences | Sessions |
| --- | --- | --- |
| `[Request interrupted by user]` | 22 | 20 of 178 |
| Permission denied at a tool call | 9 | 9 of 178 |

Precision matters more than recall here, and the asymmetry is severe: a missed
lesson costs nothing, and a wrong lesson is injected into every future session
in that scope, invisibly, until somebody notices an agent behaving oddly and
goes looking. A regex would fill the store with bug reports rephrased as
policy.

## Why abeam, specifically

The signals this needs are already inside the process, and the reason to do it
here rather than in a hook is that abeam is the only place they are all true at
once.

| What extraction needs | Where abeam already has it |
| --- | --- |
| The agent's turn ended | `agentstate::Readiness` — Claude's own record, exact, already polled every draw |
| Which files that turn touched | `watch` carries paths; `workspace` decides whose they are |
| What changed, and whether it survived | `panes::git` — `--porcelain=v2`, staged/changed/untracked, recent commits |
| What the human said back | every keystroke passes through `app` on its way to the pty |
| Whether they ever actually sent it | the composer flag — `queue`'s third send condition |
| What they ran to check the work | the shell pane |
| What they thought worth keeping | `select` — the rows dragged out to the composer |
| What they wrote in their own words | the scratch pad |
| The session's identity, hence its transcript | `Session.session_id` + `cwd` |
| A language model to distil with — authenticated, read-only, metered | `ask::ClaudeSession` |

Two of those deserve a sentence each.

**`Session.session_id` is parsed today and consumed by nothing.** It carries an
`#[allow(dead_code)]` whose stated reason is "a faithful record, parsed and
tested ahead of a consumer". With `cwd` beside it, it names
`~/.claude/projects/<key>/<sessionId>.jsonl`. This proposal is that consumer.

**`ask` is already a working extraction engine.** Its module documentation
records what was measured: a prompt written to stdin as one line of JSON, so
there is no argv length limit and a newline is two ordinary bytes;
`--tools "Read,Grep,Glob"` honoured exactly, which is the read-only guarantee;
`--session-id` accepted; `total_cost_usd` per turn, which the pane already adds
up and displays. A distiller wants all of that and one thing less — an *empty*
tool list, because the evidence goes in the prompt and it has no business
reading the repository.

No new dependency, no new credential, no second watcher.

## The three rules that decide the design

### The repository is untrusted, so lessons live beside the profile

`crate::config` refuses to read a repository-local config file, and spends a
page on why: the repository is the one directory in this program that somebody
else writes to — a clone, a pull request, whatever `git checkout` just put
there. `crate::launch` spends four hundred lines making sure a `claude.exe`
sitting in it can never be what starts.

A learned-lesson store is **context injected into an agent**. If a repository
could contribute to it, it is a prompt-injection vector wearing a friendly
name, and it would arrive with abeam's own border printing the word *lesson*
over the top. The same rule therefore applies with the same force: **the store
is profile-side, and repository content can never promote itself into it.**

Claude's memory directory is already under `~/.claude`. The rule and the
existing mechanism agree, which is the cheapest kind of decision.

### abeam writes one file, and argued for a page about it

`panes::pad::store` opens by calling itself "the first file this program has
ever written", and everything after that is the careful reading of what makes
an exception worth making. It also contains, already written and tested, every
mechanism a second writer needs: a temporary file beside the real one, synced,
renamed over the top, so a save is never half of one and half of another; a
per-workspace key from `paths::workspace_key`; creation mode bits that make the
file readable only by the person who typed it.

A ledger of things somebody said about their own work is the same category of
bytes as a pad. It should be held to the same standard and should reuse that
machinery rather than growing a second, slightly different one.

### abeam adds nothing to the agent's argv — so learning arrives through the composer

`crate::agent` promises that abeam's argv is byte for byte what the agent would
have received, and a test pins it. So the obvious delivery mechanism —
`--append-system-prompt`, or a flag pointing at a lessons file — **is not
available, and must not be made available.**

This is the best thing that happens to this design. The remaining route is the
one abeam already uses for everything else: put the text in the agent's
composer, unsent, where the person can read it, edit it, or delete it. That is
exactly the round trip `select`'s `Enter` makes today with rows from the shell
pane.

> **abeam proposes; the human sends.** No lesson ever enters an agent's context
> behind the user's back, and the mechanism that makes that true is a rule the
> project already had.

## The proposal

Five phases, sized the way `docs/multi-agent.md`'s four were — the first one
ships nothing a user can see, and that is correct.

### Phase 1 — Notice the moment. No UI, no writes.

A `crate::learn` module subscribing to what already exists: a `Busy → Idle`
transition from `agentstate`, the paths from `watch`, git's before and after.
It assembles *episodes* in memory and holds them for the session only:

```rust
struct Episode {
    turn_ended: Instant,
    touched: Vec<PathBuf>,      // from watch, routed by workspace
    stat: DiffStat,             // from the git pane's existing parse
    interrupted: bool,          // Esc during the turn
    next_prompt: Option<String>,// what the human typed, once they submitted
    gap: Duration,              // how long they took to type it
    survived: Survival,         // committed, still dirty, reverted, discarded
}
```

Triggers, in descending order of confidence:

1. **Interrupted.** The user pressed Esc mid-turn. Unambiguous, and rare — 22
   in this corpus.
2. **Permission denied.** `waiting`, then a refusal.
3. **Reverted or discarded.** The agent's edit to a file is overwritten by hand
   in the same session, or never committed. This is the strongest evidence
   available anywhere in the system, because it is a *pair*: what the agent
   wrote, and what it should have written, in the same file.
4. **Short follow-up after an editing turn.** The correction turn.
5. **Standing language in a prompt.** Lowest precision. Never a trigger on its
   own; only ever a tiebreak on an episode one of the four above already
   flagged.

Nothing is written and nothing is shown. The deliverable is a counter in the
`F2` diagnostics view: how many episodes today, of which kind. One week of that
answers the question this proposal cannot — *is there enough signal here to be
worth a pane?* — before a pane is built.

### Phase 2 — The ledger. `F10`.

An eighth right-hand view, built like the queue: a list of records, the shared
scroll vocabulary, `Enter` to act. Each row is one candidate lesson, in the
user's own words, with the episode behind it available on the row.

Three keys and no more: **`Enter` promotes, `d` discards, `e` edits the
wording.** Promotion is the only write in this design, and it is a keystroke
made by a person who is looking at the sentence.

A pane rather than a prompt, for the reason `config` gives about trust dialogs:
a dialog that appears at the moment somebody is trying to start work is a
dialog that gets answered yes. A list you visit when the agent is busy is a
list that gets read — which is the same observation the queue and the pad were
both built on.

`F10` is free under the keymap audit's own argument. The Ink reasoning settles
every function key at once for Copilot; Codex's complete default table contains
no function-key binding; Claude 2.1.251's declared bindings contain no function
key in any context. That is the argument that let `F7`, `F8` and `F9` be taken,
and `F10` is the next number. It should be added to the table in
`docs/keymap.md` in the same commit, and the audit's own small lesson about
tables — that three F-keys sat unlisted until somebody counted rows against
`keys.rs` — should not be re-learned.

### Phase 3 — Distil with the agent that is already running

Turning an episode into a sentence is a language problem, and there is a
language model already authenticated, already metered, already read-only, one
module away.

Reuse `ask::ClaudeSession` with **an empty tool list**. The evidence goes in the
prompt; the distiller has no business reading the repository, and an empty
`--tools` is a stronger guarantee than a permission mode because the pane's own
probe confirmed the flag is honoured exactly. Cost is disclosed in the ledger's
title bar, as `ask` already discloses its own.

**Phase 1's capture has to stand alone without this.** A verbatim correction,
stored unpolished, is already a lesson. Keeping the mechanical path complete
means a distiller that writes a bad sentence is a quality problem and never a
correctness one — and it means the feature still works for a user who does not
want a second model spending their money.

### Phase 4 — Scope, which is the cross-project half of the question

Every lesson gets exactly one scope, proposed by the distiller and **chosen by
the human at the moment of promotion**:

- **Workspace** — *"this repository is deliberately not rustfmt-clean; one run
  rewrites twenty-five unrelated files."* Writes to
  `~/.claude/projects/<key>/memory/`, which exists and is already loaded.
  Keyed by **workspace root**, not by current directory, which is what stops
  the worktree fragmentation measured above — the rule is `crate::workspace`'s
  and is already written.
- **User** — *"I want a plan before an implementation."* True in all 23
  projects. **There is no store for this today**; it is the actual gap behind
  the second half of the question. It goes in a new profile-side directory
  under abeam's own data root, beside the pads.
- **Provider or platform** — *"on Windows, `uv`, never `pip`."* Travels with
  the tool rather than with the repository or the person.

Scope is a human decision because getting it wrong is the expensive failure. A
repository-specific fact promoted to user scope becomes wrong advice in
twenty-two other projects, delivered confidently, and nothing will surface it.
The distiller may propose; it may not decide.

### Phase 5 — Application, through the composer and nowhere else

**Workspace-scope lessons need no delivery at all.** Claude already loads that
directory at session start. Writing there is the whole of the mechanism.

**User- and provider-scope lessons arrive in the composer**, unsent, at the
start of a session — the same round trip `select`'s `Enter` already makes,
subject to the same four conditions `queue` enumerates for anything abeam types
on your behalf, and withdrawn by the same reflex: a keystroke at the agent
defers it.

That asymmetry is the design and not an inconsistency. Where a mechanism
already exists, use it. Where none does, do not invent a covert one.

## What this does not solve

The house standard is that this section is the honest half, so:

- **Only Claude publishes readiness.** `agentstate` says it plainly: Copilot
  and Codex report `Unknown` forever. Triggers 1, 2 and 4 need turn boundaries
  and therefore do not exist for those agents. Trigger 3 — reverted or
  discarded — is pure git and works for all three, so the feature degrades to a
  weaker version rather than to nothing. It is still a Claude feature first,
  and the README's disclosure standard means saying so there too.

- **Claude's memory directory is not a published API.** Neither is the session
  record, and `agentstate` handles that by reading every field as optional,
  refusing an unfamiliar `peerProtocol`, and degrading to `Unknown` rather than
  to a wrong answer. **Writing into a private layout is a materially bigger
  commitment than reading a status field out of one**, and the `C--Users-philm-…`
  path encoding is Claude's own business that abeam would be reproducing by
  guess. This is the single riskiest line in the proposal. The mitigation that
  matches the existing standard: derive the path, verify a `memory/` directory
  and a `MEMORY.md` are already there, and **write nothing if the layout is not
  the one abeam was taught** — an unfamiliar layout means the user keeps their
  lesson in abeam's own ledger and loses only the automatic delivery.

- **Nothing here measures whether a lesson helps.** No phase proves a promoted
  sentence changes what an agent does. The only honest metric is recurrence:
  promote a lesson, then count whether the episode that produced it happens
  again. It is not in phases 1–5, it should be phase 6, and until it exists
  every claim about effectiveness in this document is a hypothesis.

- **A wrong lesson is expensive and quiet.** It is injected into every session
  in its scope until somebody notices. Three things contain it, and all three
  are load-bearing: promotion needs a keystroke, every lesson keeps the episode
  it came from so it can be judged later, and the pane that promotes is the
  pane that retracts.

- **The distiller reads agent output about repository files**, which in a fork
  or a pull-request checkout is attacker-controlled text. Containment is the
  empty tool list, the fact that its output is a *candidate* a person must
  approve, and a hard rule that it may never propose user scope for a lesson
  drawn from repository content. That last one is a real restriction and should
  be written into the module, not left to the prompt.

- **One machine, one user.** Every number here came from one corpus. The
  mechanisms are real — the files exist, the fields parse, the retention window
  is measurable. The *rates* are not evidence about anybody else.

## The smallest version worth shipping

If one thing ships: **phase 1, trigger 3 only, and a ledger that records
verbatim.** No language model, no scope, no cross-project delivery. Reverted
and discarded work, listed on `F10`, in the user's own words.

It answers *"what did I have to correct this week"* — which is a question
nobody can answer today, needs nothing that is not already in the process, and
makes the case for the other four phases better than this document can.
