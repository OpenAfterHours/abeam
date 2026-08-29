# Keymap, and the collision audit behind it

> Provenance, for the Claude sections: extracted from
> `C:/Users/philm/.local/bin/claude.exe` — **Claude Code 2.1.220**, 265,720,480
> bytes, mtime 2026-07-25 — by the design pass on 2026-08-01 and re-checked on
> 2026-08-01 when the command view added `Alt+S`. Written down because it was
> expensive to obtain and will silently rot otherwise. The version and size are
> recorded as well as the date: a date alone cannot tell a re-install of the
> same build from a silent update.
>
> **That build is gone, and this is the block earning its keep.** The file at
> that path on 2026-08-29 is **Claude Code 2.1.251**, 217,360,032 bytes, mtime
> 2026-08-29 08:19 — a different binary, forty-eight megabytes smaller.
>
> **Three things below came out of 2.1.251, and the rest did not.** This block
> is worth nothing if it is vague about which, so: the first is the function-key
> claim, which is what `F9` needed — the declared `context`/`bindings` table in
> 2.1.251 contains no function key in any context, no `alt+`/`meta+` spelling of
> one appears anywhere in the binary, and the six occurrences of the literal
> `"f9"` are a `key.name = "f9"` in the keypress parser, the `"[20~"`
> escape-sequence table, and four `Set`s that list every F-key among the names a
> text field must *not* treat as input. The second is the `Alt+J` re-check under
> "Known gaps, against Claude", which is a **letter** rather than a function key
> — the one letter anybody had a live reason to look at, because Claude's own
> footer advertises it. The third is the census of declared actions under
> "Claude Code's bindings", which is the measurement of how far the rest may
> have drifted.
>
> Everything else — every `Ctrl`, every other `Alt`, every bare letter in every
> context — was extracted from 2.1.220 and has not been looked at since. Treat
> it as 2.1.220's until somebody repeats the whole extraction, and treat the
> difference as live rather than cosmetic: the short form of that census is that
> 2.1.251 declares 143 actions, thirty of which carry no default key at all,
> including a whole `strip:` family this document has never named.
>
> `~/.claude/keybindings.json` does not exist on this machine, so nothing here
> is overridden by user configuration. If it ever does, this whole document is
> describing defaults the user may not be using. Re-checked on 2026-08-29: still
> absent.
>
> The binary audited is a **Windows** build, and now that abeam runs on Linux
> that is worth saying out loud rather than leaving to be inferred from the
> path. Where Claude's own bindings are platform-conditional the inventory below
> says so — `alt+v` for image paste, the computed `meta+m` — but a Linux build
> has not been extracted, so "the table says so" is only as complete as one
> platform's binary made it.
>
> The GitHub Copilot CLI sections at the foot of this document carry their own
> provenance, and it is weaker. The two are not interchangeable and are kept
> apart for that reason.
>
> The Codex section carries a third provenance: the official npm package
> `@openai/codex@0.149.0-win32-x64`, downloaded but not installed on 2026-08-23.
> Its `codex.exe` reports **codex-cli 0.149.0**, is 297,362,224 bytes, and has
> SHA-256 `14B7E6B2356E82D1D9275579EAA588757B4E0A501B65DCC19FCCDF77BD83DC00`.
> The binary evidence was checked against OpenAI's complete `built_in_defaults`
> table at the matching `rust-v0.149.0` source tag. No authenticated session was
> available, so modal behaviour was not audited live.
>
> That artefact is **still on this machine**, at
> `%APPDATA%\npm\node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe`,
> and on 2026-08-29 its SHA-256 still matched the digit above. So `F9` was
> checked against the same bytes `F8` was, rather than against a later build
> standing in for them — which is the only reason the two rest on one standard
> and not on two.

## The invariant

**Against each agent's shipped default keymap, nothing abeam intercepts may be
a key that agent can act on.** The
plural is the whole difficulty: a binding is safe only if it is a no-op in
*every* agent abeam can host, so gaining an agent can retire a key that was
safe when there was one. It has already retired two: Copilot took the Alt
arrows, and Codex took `Alt+A`.

User overrides are outside that guarantee. Claude and Codex both support
custom keymaps, and abeam does not load or merge their configuration. No static
abeam table can be collision-free against arbitrary remappings; literal-next
(`Ctrl+\` or `F12`) is the recovery path when an override overlaps one.

Every abeam binding below was checked against the Claude inventory below, and
that inventory was extracted from 2.1.220 — a build no longer on this machine.
The standard is therefore narrower than this paragraph used to claim, and the
narrower version is the true one: **`F9` is the only binding verified against
the build the user is actually running.** Every `Alt` letter here is a claim
about a binary that no longer exists, unrefuted rather than re-checked, and the
note under "Claude Code's bindings" measures how far it may have drifted. That
is weaker than "a verified no-op in Claude today", and the sentence stating the
standard is the last place in this document that should have rounded it up.
Against Copilot CLI the same check has been
made from documentation and source rather than from the binary, it is honestly
weaker, and it found one collision: `Alt+Left` and `Alt+Right`, which GitHub
declares as word-motion in its own command reference. **abeam gave them up.**
Focus movement is `F4` and `F5`, and the arrows go to the agent.

Codex found the second collision. Its default global keymap binds `Alt+A` to
opening the shared agent-session overview. **abeam gave it up too.** The queue
is now `F8`; no `F8` collision surfaced in the audited shipped artefacts.

Those changes removed the two breaches this audit knew about, and yielding a
key verifies nothing else, so be exact about what the invariant is worth. In
Claude it is **verified**: the inventory came out of the installed binary,
which is the only reason it caught `Alt+F`. In Copilot it is **unrefuted**,
which is a weaker thing entirely — six of abeam's bindings, `Alt+G`, `Alt+S`,
`Alt+J`, `Alt+K`, `Alt+PageUp` and `Alt+PageDown`, each shadow a key Copilot
binds in its bare form somewhere, and Ink hands a handler the bare form
together with a `meta` flag the handler is free to ignore. Nothing a user can
reach is lost either way, because the bare keys still pass through; what is
unproven is the strict form, that an intercepted key is one the agent could not
have acted on. The change made that "unproven for one agent" instead of "known
broken for one agent", and not "verified for both". Only the Copilot binary can
settle the six; "Known gaps, against Copilot" at the foot of this document says
so again, and the README's disclosure bullet is the third place. The evidence
for the collision, and the reasoning about which side should yield, are under
"The collision, and how it was settled" below. Codex is a fourth confidence
level: its shipped Windows binary establishes the `Alt+A` collision and its
default F-key gap, but Codex also lets users remap TUI actions with `/keymap`.
The strict invariant therefore applies to Codex's defaults, not to an arbitrary
`tui.keymap` in a user's `config.toml`; literal-next remains the escape hatch.

Typing at the agent is byte-for-byte what the pty spike did.

## abeam's bindings

`crates/abeam/src/keys.rs` is the single table. Globals work at any focus.

| Key | Action |
| --- | --- |
| `Alt+G` | right pane → git view (focus unchanged) |
| `Alt+E` | right pane → files / markdown view (focus unchanged) |
| `Alt+S` | right pane → a shell, **and focus it**; again to hand focus back |
| `F8` | right pane → the queue of work for the agent (focus unchanged) |
| `F4` / `F5` | move focus left / right; `F4` again moves along the agents |
| `Alt+J` / `Alt+K` | scroll right pane one line — **without focusing it** |
| `Alt+PageDown` / `Alt+PageUp` | scroll right pane one page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (press twice while a child is live) |
| `F1` | key help overlay |
| `F2` | right pane → pty diagnostics, and back to what it displaced (focus unchanged) |
| `F3` | file reader → light / dark page (no other view changes) |
| `F6` | right pane → ask, **nothing attached, and focus it**; again for what it displaced |
| `F7` | select rows of the right pane by keyboard, **and focus it**; again to put the selection away |
| `F9` | right pane → the scratch pad, **and focus it**; again to hand focus back |
| `Ctrl+\` or `F12` | literal-next: send the following key to the agent verbatim |

**`F4`'s second meaning cost this document no audit, and that is the point of
it.** A press while the keys are already on the left used to do nothing at all,
so "again" is a meaning added to a dead press rather than a key taken from any
agent — the table above is one row longer in words and no rows longer in keys.
The gesture that *starts* another agent is `a` on a row of the worktree list,
which is not in this table either: it is a pane-local key, claimed inside a list
that is only up while the right pane has focus, and the exemption is
`crates/abeam/src/panes/git.rs`'s, stated above `enum Mode` and standing since
`w` and `Enter` were claimed there. There is no `Shift+F4` for the other
direction, deliberately: a modified F-key is a key abeam knows nothing about and
therefore the agent's, which `keys.rs` says in a comment about `Ctrl+F12`.

**The gesture that *closes* one is `q`, and it is not in the table for a
stronger reason than `a` is.** `a` leans on the worktree list's exemption — a
key claimed inside a view that is only up while the right pane has focus, so it
is never delivered while anything is being typed at an agent. `q` is claimed in
the **left** column, which is where typing at an agent happens, and it is
nevertheless outside this document's invariant: it is only ever offered to a
pane **whose child has exited**. There is no process left to have a binding for
it. Every key delivered to such a pane is already going into a closed pty and
doing nothing, so a letter intercepted there cannot shadow any agent's binding
in any session — which is a claim no audit could make about a live one, and the
reason the audits below are unaffected. It sits inside `handle_key`'s
`Focus::Left` arm and *after* the literal-next hatch, so `Ctrl+\` `q` still goes
to the child; `crates/abeam/src/app.rs`'s `close_key` is where that is written
down. Two presses, because what closing destroys is the frozen last screen and
the scrollback behind it, and nothing in abeam can get either back.

One consequence is worth disclosing rather than leaving to be discovered.
**While the right pane is hidden — `Alt+Z`, or a window narrower than
`MIN_SPLIT_COLS` — every `F4` cycles, including the first.** Focus cannot be on
the right when there is no right pane, so abeam holds it on the left, and a
press meaning "back to the agent" is a press that already has the keys. What
stops that silently handing your next sentence to another session is the agent
pane's own border, which reads `claude · 2/3` whenever there is more than one
agent. The position is the disclosure, and it is drawn for this reason rather
than as decoration.

**"Focus unchanged" holds in both directions.** The four rows marked so —
`Alt+G`, `Alt+E`, `F8` and `F2` — neither take focus nor hand it back:
`Alt+E` pressed while a shell has your keys leaves them on the right pane, now
showing the files view, rather than returning you to the agent. abeam used to
make that second move, leaving a pane you could type into for one you could not,
and it was the special case those parentheses deny. The argument for dropping it
is kept in one place — `App::set_right_view`, in `crates/abeam/src/app.rs` — and
in short it is that the rule turned on whether the pane you were leaving took
typing *at that instant*, so the same `Alt+E` moved focus while a shell's child
was alive and left it alone a second after that child exited.

`Alt+E` pressed while the files view is already showing opens the file list, so
it is never a key that does nothing. It used to reload the open file; reload is
`r` from inside the pane and the watcher does it unasked, so the second press
was spending a key on a job already done twice.

`Alt+S` and `F9` are the workspace view keys that move focus, and in both cases
the exception is deliberate and is the same exception: a command line you have
to press a second key to type into is a picture of a command line, and so is a
scratch pad. The other three views are read, so they are glanced at.
(`F6` and `F7` move focus too, and say so in the table; neither is one of the
five views `Alt+G`, `Alt+E`, `Alt+S`, `F8` and `F9` switch between.) `Alt+S` is
also the only right-hand view that keeps `Esc` and `q` — they belong to the
shell — so the border advertises the way out instead. The pad keeps neither:
`Esc` is not text, and `q` is text only in the edit form, where it is claimed
as a letter somebody is typing rather than as a way out.

**The queue moved from `Alt+A` to `F8` for Codex.** `Alt+A` really was clear in
Claude 2.1.220 — the earlier binary audit found zero `meta+a`/`alt+a` matches —
and Copilot's documentation-derived audit found none either. Codex 0.149.0's
shipped keymap instead places the literal `alt-a` beside
`tui.keymap.global.open_agents`, its shared agent-session overview. Keeping the
four view keys as an Alt-letter set is less important than preserving an action
in the hosted agent, so abeam yielded as it did for Copilot's Alt arrows.

`F8` and `F9` are unused in the three audited default keymaps. For Claude and
Copilot the old structural F-key arguments still apply; Codex can distinguish
F-keys and allows users to remap them. The claim for Codex is therefore only
about the default keymap. A custom Codex `tui.keymap` can collide with `F8` or
`F9` (or any other abeam key), and abeam does not currently read that file.

**`Alt+S` was verified the same way the rest of this table was**, against
2.1.220: absent from every declared `context/bindings` block (`meta+s` and
`alt+s` both), absent from the undeclared readline switch, and with no
hardcoded `meta && key === "s"` comparison anywhere in the binary. Claude's own
normaliser maps `meta`/`option`/`opt` → `alt`, which is what makes searching
both spellings sufficient.

`F2` is an F-key rather than a third Alt letter on purpose. The letters a
diagnostics binding would reach for — `Alt+D` for "diagnostics", `Alt+I` for
"instruments", `Alt+U`, `Alt+L`, `Alt+C` — are either known-taken or classic
readline word commands, and the audit below already caught one *undeclared*
readline binding (`Alt+F`). F-keys have no such ambiguity: none is bound by
Claude in any context, which is the same reason `F1` and `F12` are safe.

`F3` is an F-key for exactly the same reason, and the letters it would have
wanted are a worse set still: `Alt+T` for "theme" is Claude's outright, and
`Alt+L` for "light" and `Alt+D` for "dark" are both in the classic readline set
the audit below found Claude handling without declaring. It is global rather
than a key the viewer handles, so it works while Claude has focus — the reader
is a pane you glance at, and having to enter it before you could change how it
looks would defeat the point.

`F6` is an F-key by the same argument as `F2` and `F3`, and by one more that
neither of them had to make. The letters an "ask" binding would reach for are
gone twice over — `Alt+A` is Codex's, `Alt+Q` is quit, and `?` is a shifted
key whose `Alt` form no audit here has looked at in every agent. When this
landed there were two agents to clear; Codex's default now clears it too. The
Claude and Copilot F-key arguments are *structural* rather
than merely unrefuted (see "Why Ink settles the function keys and unsettles the
letters"). It joins `F2` rather than the view keys, and that grouping
is real: the diagnostics and the ask are the two views that displace another and
put it back, and neither is remembered as a workspace view. This is not one of
them; neither the queue's forced move to `F8` nor the pad's arrival on `F9`
changes that grouping. The count is left out on purpose, because it is the
number that goes stale every time a view lands.

It is the global half of a pane-local `?`, and the two are deliberately
different keys rather than two spellings of one. `?` means "about the file I am
reading" and is only ever delivered to a focused pane, which is what exempts it
from this file's invariant. `F6` means "about nothing in particular", is
reachable while the agent has focus — which is where the question usually comes
up — and attaches nothing, which also makes it the only way to take an
attachment back off.

`F7` is an F-key by a stronger argument than `F2`, `F3` and `F6` had to make,
and it is worth separating from theirs. Those three are F-keys because the
`Alt` letters they wanted turned out to be taken; this one could not have been a
letter under *any* namespace. The pane it acts on is the one pane that takes
every key it is given — a shell with a live child in it claims `Esc`, `q` and
every letter, which is why the border there advertises `Alt+S` as the way out —
so a pane-local key would be missing from the view the whole feature exists for.
You select what a command printed. That leaves only what `global` claims before
any pane is offered anything, and inside `global` only a key that the audited
shipped defaults for Claude, Copilot and Codex leave alone.

It is also not the way most people will copy anything, and the table above is
right to be the only place it looks central. **A drag in the right pane selects
and copies on its own**, with no key and no mode — the gesture the host
terminal's own selection used, kept doing what it did. `F7` is what a keyboard
has instead, and what anyone has when the right pane is running something that
asked for the mouse.

What it opens is a mode, and the mode is one of four places in abeam where the
ordinary vocabulary is suspended — the find boxes, the ask's composer and the
scratch pad's edit form being the others. Every one of them owes the F1 overlay
a row of its own, because a mode that changes what each key means and announces
it nowhere reads as a broken pane rather than as a mode. This one
is stated loudest because it swallows *every* key while it is up, over a pane
that may have a live child behind it: the scroll keys move a caret instead of a
view, `v` anchors, `y` copies to the host terminal's clipboard over OSC 52,
`Enter` puts the selected rows in the agent's composer unsent, and `Esc` or `q`
leaves. Nothing reaches the pane or the child until it does.

**`Ctrl+C` copies while a selection is up, and it is the one `Ctrl`+letter in
this program that is ever abeam's.** It is deliberately not in the table above,
and the distinction is this file's own: `global` claims nothing, so the invariant
at the top holds unchanged. The key is read inside a mode that is already
swallowing every keystroke, so what it costs the child is nothing it would have
received — and the state it is read in is exactly the one where a hand reaching
for `Ctrl+C` means "copy this", which is the rule Windows Terminal already
taught. `Esc` first is how you interrupt something instead, and the overlay says
so on the row beside it.

`F9` is the scratch pad, and it takes focus for `Alt+S`'s reason rather than
`F8`'s: a pad you have to press a second key to type into is a picture of a pad,
in the same way a command line you cannot type into is a picture of one. The
queue is glanced at while the agent works, so `F8` leaves your keys where it
found them; the pad exists to be written in during exactly that pause, so `F9`
brings them with it and a second `F9` gives them back. It is a workspace view
like the four above it, so `set_right_view` remembers it in
`last_workspace_view` — which is what `F2` comes back to when the diagnostics
view is put away, and what `Esc` restores from the ask. `Alt+G` returns to
nothing: it sets the git view whatever was showing before.

**Against Claude it is verified, and against a build this document had not seen
before.** The provenance block records that the audited 2.1.220 binary is gone
and 2.1.251 is standing at that path; `F9` was cleared against 2.1.251 rather
than against the older inventory, by the method that cleared the rest — extract
the printable runs from the binary, find the declared `context`/`bindings`
table, and then look for the comparisons the table does not declare, which is
the step that caught `Alt+F`. The declared table holds no function key in any
context. No `"alt+f9"`, `"meta+f9"` or any other modified spelling of an F-key
appears anywhere in the file. There is no hardcoded `=== "f9"` at all, which is
the shape the undeclared readline bindings take and the reason that search was
run. Every remaining occurrence of `"f9"` is Claude reading the key rather than
acting on it: the parser assigning `key.name`, the `"[20~"` sequence table, and
four sets that name every F-key precisely so a text field will not treat one as
typing. Claude's two lists of combinations a user is not allowed to rebind say
the same thing more weakly and are worth a glance rather than a paragraph — see
"What that means" below, where they are set out.

**Against Copilot it is inherited, and that is a different word from verified.**
No fresh evidence was gathered and none is needed: the structural argument under
"Why Ink settles the function keys and unsettles the letters" covers every
function key at once rather than one at a time. `useInput` hands its handler a
`Key` record with no field for a function key, and `f1` through `f12` sit in
Ink's `nonAlphanumericKeys`, so the `input` string is blanked as well — every
bare F-key arrives as an empty string with every flag false, indistinguishable
from every other F-key and from nothing having happened. `F9` is covered by that
sentence the moment it is bound, on the same footing as `F1` and `F4` and by the
same reasoning, and it would fall the same way if a future Ink release grew a
field for function keys. Nothing about `F9` in particular was established, and
the table below marks it inherited so that a reader cannot mistake the coverage
for a check.

**Against Codex it is the weakest of the three, and it is exactly as weak as
`F8`.** Codex can see a function key and can be told to act on one, so there is
no structural argument to inherit here and absence in a shipped default table is
the whole of what there is. The artefact is what makes even that worth saying:
the 0.149.0 binary the `F8` entry rests on is still on this machine and its
SHA-256 still matches the digit in the provenance block, so `F9` was run against
the same bytes rather than against a later build standing in for them. In those
bytes the only `alt-`prefixed binding literal in the whole 297 MB is `alt-a`, beside
`tui.keymap.global.open_agents` — which is the positive control, because a
method that finds the one collision Codex is known to have is a method that
would have found a second. No function-key literal appears at all: `f7` through
`f12` are absent as standalone tokens from every printable run in the file, and
the only byte sequences that look like `f9` are x86 instruction fragments. That
clears `F9` against 0.149.0's complete shipped defaults and against nothing
else. A user who binds `F9` in `tui.keymap` collides with abeam, abeam does not
read that file, and literal-next is the way through. What would settle it is
what would settle `F8`: an authenticated session, driven by hand, with every
abeam global sent through literal-next in the composer, the lists, the pager and
the approval UI.

**Why `F9` and not `F10` or `F11`, which is a different kind of reason and
should not be filed with the rest.** Every argument above is about an agent —
what Claude binds, what Ink can express, what Codex ships. This one is about the
terminal in front of the agent, and it disqualifies two keys before any agent is
consulted. `F11` is fullscreen in Windows Terminal and in most other emulators,
and `F10` activates the menu bar in several, so neither reliably *arrives* at
the application at all: the keystroke is consumed a layer above abeam, abeam
never sees an event, and the pad would fail to open on some machines and work on
others with nothing on screen to explain the difference. The invariant
at the top cannot catch that, because abeam intercepts nothing it is never
handed, which is why this is written here rather than added to the tables as a
fourth verdict. It is also the one claim in this document that no
strings audit can support, and `crates/abeam/examples/keyprobe.rs` is what would:
run it in the terminal abeam is launched from and see whether `F9` comes back as
`F(9)` with an empty modifier set. It has been run against Windows consoles only,
and never for `F9`.

`Alt+T` turns the pad over between its source and its rendering, and it is the
first `Alt` chord in abeam that a *pane* reads rather than `global`. That is
inside the exemption `keys.rs` states once — intercept means what `global`
claims before any pane is offered a key, and `global` returns `None` for
`Alt+T` — but it is worth naming rather than leaving to the general rule,
because this is the first time the exemption has been leaned on by anything but
a bare letter, and the key underneath it is not free. Claude binds `meta+t` to
`chat:thinkingToggle` in its Chat context, which is why `F3` is an F-key and not
`Alt+T` for "theme". Nothing is lost: `Alt+T` only means anything to abeam while
the pad has focus, and while the pad has focus the agent is not being typed at,
so the keystroke was never going to reach Claude whatever this file said.

`Alt+T` is also where this document learned that "the `Alt` namespace" was two
different sets, and the section below — "AltGr is Ctrl+Alt" — is what came of
it. The short version: the pad asked `alt && !ctrl` while `global` asked only
for `ALT`, so every global binding answered to both `Alt` keys and the pad's
answered to the left one alone.

The cost runs the other way instead, and it is the one direction that paragraph
would otherwise hide. After the round trip this document recommends — `F9` to
open the pad and take it, `F9` again to hand your keys back — the pad is still
on screen and no longer focused, and `Alt+T` pressed then is not abeam's at all.
It goes to the agent, Claude toggles thinking, and the pad does not turn over.
Nothing on screen says so, because from abeam's side nothing happened: a key it
declined is a key it never saw. The remedy is focus — `F9`, or a click — and
the pad's row in the F1 overlay is where that belongs.

**`Alt+T` must never be promoted to a global binding, and the reason is not
style.** The whole of what makes it safe is that `keys::global` returns `None`
for it, so it is delivered to a focused pane rather than claimed before any pane
is offered it. Move it into `global` and it is claimed while the *agent* has
focus — which is exactly where a Claude user presses it — and
`chat:thinkingToggle` stops working, for every Claude user, in every session,
whether or not they have ever opened the pad, with nothing on screen to say what
took it. It would be `Alt+F` again, made deliberately and by us.

**The test suite already refuses that edit**, which is a better thing to know
than the warning above it. `the_agents_alt_bindings_are_left_alone` in
`keys.rs` loops over `"bfdyvmpotw"` asserting that `global` returns `None` for
each of those letters under `Alt`. `t` is in that string, so an arm claiming it
fails at once, with `Alt+t is Claude's`. This is not a rule resting on somebody
having read a paragraph. One edge is worth knowing rather than discovering: the
loop presses lowercase only, so it catches a `Char('t')` arm and a
`Char('t') | Char('T')` arm both — on the `'t'` — while an arm matching `'T'`
alone would slip past. Whoever is reading this line while considering that edit:
the test is the argument, and it is one to answer rather than to delete.

`F4` and `F5` are the odd pair in this table: the only binding that was *taken
away* from abeam rather than chosen for it. Focus moved on `Alt+←`/`Alt+→` until
the Copilot audit below found GitHub declaring that pair as word-motion, at
which point they stopped being abeam's to bind. Two direct keys rather than one
toggle, for the reason the view keys are two direct keys — a toggle needs you to
know the current state before you press it, which fails exactly when you are
glancing rather than looking, and focus is glanced at the same way a view is.
Function keys rather than two more Alt letters because F-keys, alone in this
table, do not rest on an audit having been exhaustive: Claude binds none in any
context, and an Ink application *cannot* bind one. See "Why Ink settles the
function keys and unsettles the letters" below.

Right pane, only when focused. Plain keys, because the agent never sees them.
The vocabulary was taken from Claude's own transcript view, deliberately, so
that there is one scroll language in the application. Copilot's diff mode
shares most of it and differs in two places that are worth naming rather than
glossing: `space` pages nothing there, and `b` toggles the diff rather than
paging back. Neither difference reaches inside abeam, where these keys are the
pane's, but "one scroll language serves both" is a near-miss and not an
identity.

`j`/`k`, arrows — line · `space`/`b`, PgDn/PgUp — page · `Ctrl+D`/`Ctrl+U` —
half page · `g`/`G`, Home/End — ends · `Tab`/`Shift+Tab` — next/prev item ·
`Enter` — open · `r` — refresh · `Esc`/`q` — back to the agent, which is what
the border says.

The files view adds `t` — the rendering or what was typed, which is markdown
and, now, a Rust or Python file whose documentation is being drawn as prose —
`o` for the outline of the document, which declines on a file that has none
rather than opening an empty list, `Backspace` or `-`
to climb a directory in the file list, and three searches: `/` finds a file
anywhere under the root while the list is up and a phrase on the page while a
document is, `n` and `N` walk that document's matches, and `f` reads every file
under the root for a phrase. The queue view adds `i` to write an item, `a` to
arm or disarm sending, `d` to delete one, `m` to switch an item between being
typed into the live session and being dispatched as its own background agent,
`r` to clear the rows it has finished with, and `Enter` to do the selected one
now.

`d` and `r` ask twice — press again, with the foot line saying so, and *any*
other key, paste or click is the answer no. They are the two keys here that
throw work away, and they stopped being reachable only on purpose when a view
key stopped moving focus: `F8` to glance at the queue from a shell leaves the
keys in the pane, so the rest of a half-typed command is read as commands, and
`cargo doc --release` carries a `d` and two `r`s.

`Enter` cannot be taken back either and is **not** guarded, which is a judgement
about cost rather than a claim that it is safe — it is the pane's ordinary verb,
it acts only on the row you chose, and it ends every mistyped command there is.
The guard is `Alt+Q`'s in shape and narrower in reach: `Alt+Q`'s is cleared by
any key anywhere in the window, this one only by what the pane is offered, which
is why the shell drops it when the pane leaves the screen. It is a speed bump
rather than a lock — two `d`s in a row still delete.

`r` is the one place the shared vocabulary above means something else here: it
refreshes in the files and git views and clears the finished rows in this one.
None of the rest collides with the
vocabulary above, and none can reach the agent: the right pane has to be focused
for any of them to be seen. That exemption is stated once rather than argued
beside each key, and the place is the module doc at the top of `keys.rs`:
*intercept* means what `global` claims before any pane is offered a key, and
`global` returns `None` for every bare printable one of these.

`o` is the newest of them and was checked the same way regardless, because the
value of this document is the checking rather than the conclusion: no
`Char('o')` or `Char('O')` arm exists anywhere in `crates/`, and `keys::global`
claims only `Ctrl+\`, the function keys and `Alt` combinations — so nothing in
abeam took it first. It is **not** cleared against Claude's, Copilot's or
Codex's keymaps, and does not need to be, for the same reason `f`, `w` and `?`
are not: a bare letter that only ever arrives at a focused right-hand pane is a
letter no hosted agent is listening for.

Inside the queue's composer, `Enter` commits the item and `Ctrl+Enter` or
`Alt+Enter` puts a newline in it instead — and the first of that pair is a
Windows fact rather than a binding. Telling `Ctrl+Enter` from a bare `Enter`
requires the Kitty keyboard protocol, which abeam does not ask for (`term::setup`
enables raw mode, the alternate screen, mouse capture and bracketed paste, and
nothing else), so on most Unix terminals that arm is unreachable and what is
left is `Alt+Enter` — itself subject to the probe below — and pasting. Asking
for Kitty would fix it and is not a queue decision: it changes how *every* key
in this document arrives.

Arming is `a` and not `space`, which was the first implementation and was
wrong: `space` pages, in every pane, and the table above promises it by name. A
key that pages in three panes and toggles a mode in the fourth is a key nobody
can learn — the same argument that moved focus off `Alt+←`/`Alt+→` for every
agent rather than making it conditional on which one was running, and it costs
very much less to honour here.

Most of them are claimed *conditionally*, which is the thing to keep straight.
While a box is open the query eats every printable key, so `j`, `k`, `g`, `G`,
`b`, `q`, `r` and `f` are text rather than commands, the arrows and
`Ctrl+N`/`Ctrl+P` step whatever is behind it — a selection in the two lists, the
matches in a document — and `Esc` closes the box instead of leaving the pane.
There are three such boxes in the files view now — the file list's find, the
document's search and the repository sweep's — and they take keys the same way
on purpose. They differ in one place worth stating because it looks like a
fault: the first two answer on every keystroke, and the sweep's does nothing at
all until `Enter`, because it reads every file under the root. That is why
`Alt+J` and friends reach a pane through `Pane::scroll_key`
carrying the bare `Down`/`Up`/`PageDown`/`PageUp` a focused pane would have
seen, rather than the letters: a glance at the pane must never be able to type
into it.

The shell view keeps `Esc` and `q` — they belong to the hosted program — so it
is the one view the `Esc`-means-back rule does not apply to, and its border
advertises `Alt+S` instead. Once its child has exited it takes nothing, and the
ordinary rule comes back.

**The scratch pad suspends the vocabulary above rather than adding to it**, and
it is the widest of the suspensions in this document: in the edit form every
printable key is a character somebody is typing, so `j`, `k`, `g`, `G`, `b`,
`q`, `r`, `t` and the rest are letters, and what moves the caret is the arrows,
`Home` and `End`. That is the same shape as the ask's composer and the find
boxes, and it is more of it, because the pad has no state in which the letters
mean commands again. `Esc` is declined, which is how the composer and the boxes
differ from it: there is nothing for `Esc` to close, so it falls through and
`app.rs` hands focus back to the agent, which is the same thing `Esc` does from
every other view.

The rendering is the pad's other form and is read-only, so the vocabulary above
does come back there — including bare `t`, which returns to the source and is
the key `ViewerPane::toggle_raw` already taught for that question. `Alt+T` does
the same in both directions, and in the edit form it is the only key that can:
the letter is a letter there. That chord is read by the pane and never by
`global`, which is what keeps it out of the table at the top of this document
and out of the invariant — see the paragraph on `F9` above for why the
distinction is worth making out loud in this one case, when Claude binds
`meta+t` in its Chat context.

## Claude Code's bindings, as of the audited build

> **These bindings were not read out of the build on this machine.** They came
> out of 2.1.220. The file at that path is 2.1.251, and only the function keys
> below have been re-derived from it. Every `Ctrl`, every `Alt` and every bare
> letter in every context below is 2.1.220's — and the letters are the half this
> document has already been wrong about once, because `Alt+F` was word-motion in
> a binary and in no published table.
>
> **Repeating the extraction is a job to schedule, not a caveat to read past.**
> It is the method described throughout this document: pull the printable runs
> out of the binary, find the declared `context`/`bindings` table, then hunt the
> comparisons that table does not declare, and rewrite this section and the two
> that follow it against what comes back.
>
> Here is the measurement that says why it is worth somebody's afternoon rather
> than the usual shrug about software changing. **2.1.251 declares 143 actions.
> 113 of them carry at least one default key in the declared table. Thirty carry
> none at all** — and `app:toggleTerminal`, the action the `Alt+J` warning under
> "Known gaps" is built on, is one of the thirty. That warning is not a curiosity
> about one key, then: it is one member of a class thirty wide, every one of
> which is a single line away from acquiring a default, in a product that ships
> weekly. Thirteen of the thirty are one family this document has never
> mentioned — `strip:jump1` through `strip:jump9`, plus `strip:next`,
> `strip:previous`, `strip:toggle` and `strip:new` — and beside them sit
> `app:toggleReplTab`, `app:toggleDiffNoiseFilter`, `app:toggleDiffPreSession`,
> `app:redraw`, `chat:cycleProactivity`, `chat:attentionUp`, `chat:attentionDown`
> and an entire `proactivityMenu` context that is absent from the table below.
>
> Be exact about what that count is and is not, because overstating it would
> cost this document the thing it is for. "No default key in the declared table"
> is not "unreachable": an action can be driven from a menu, and it can be bound
> by precisely the sort of undeclared comparison that made `Alt+F` word-motion.
> Thirty is the size of the surface. It is not a prediction about any key on it.

**Global:** `ctrl+c` interrupt · `ctrl+d` exit · `ctrl+t` toggleTodos ·
`ctrl+o` toggleTranscript · `ctrl+shift+b` toggleBrief · `ctrl+r`
history:search · `ctrl+up`/`ctrl+down`/`meta+up`/`meta+down` diffFileList ·
**`ctrl+]` app:openArtifact**

**Chat:** `escape` cancel · `ctrl+l` clearInput · `ctrl+x ctrl+k` killAgents ·
`shift+tab` cycleMode · `meta+p` modelPicker · `meta+o` fastMode · `meta+t`
thinkingToggle · `meta+w` workflowKeywordToggle · `enter` submit · `ctrl+j`
newline · `up`/`down` history · `ctrl+_` / `ctrl+-` undo · `ctrl+x ctrl+e` +
`ctrl+g` externalEditor · `ctrl+s` stash · **`alt+v`** imagePaste (Windows;
`ctrl+v` elsewhere) · `space` voice:pushToTalk

**Transcript:** `ctrl+e` · `ctrl+c` · `esc` · `q` · `ctrl+u`/`ctrl+d` half-page ·
`ctrl+b`/`ctrl+f` full-page · `ctrl+n`/`ctrl+p` line · `g`/`shift+g` · `j`/`k` ·
`space`/`b` · arrows/home/end

**Task:** `ctrl+b`, `ctrl+x ctrl+b` background. **HistorySearch:** `ctrl+r`,
`ctrl+s`. **Scroll:** pageup/pagedown/wheel, `ctrl+home`/`ctrl+end`,
`ctrl+shift+c`, shift+arrows. **Settings / Select / Confirmation / Tabs /
DiffDialog / Footer / MessageSelector / ThemePicker / Plugin / Autocomplete /
Help** consume most bare letters, tab, space, enter, esc and the arrows.

**Undeclared** — a hardcoded readline switch in the prompt editor
(`if (F.meta) { switch (key) { case "b" … "f" … "d" … "y" } }`), absent from the
declared keybinding table: **`alt+b`** prevWord · **`alt+f`** nextWord ·
**`alt+d`** deleteWordAfter · **`alt+y`** yankPop · `alt+backspace`
deleteWordBefore.

## What that means

- **Every `Ctrl`+letter a–z is taken or reserved.** No Ctrl binding is safe,
  including any Ctrl *prefix*. `ctrl+x` is already a prefix *inside* Claude, so
  stealing it would break three bindings and create a double-prefix.
- **`Ctrl+]` — the pty spike's detach key — is `app:openArtifact`.** It is
  retired. Exiting the app when the user meant to open an artifact is the worst
  possible failure for a binding nobody chose. Replaced by `Alt+Q`.
- **No F-key is bound anywhere, in any context** — in 2.1.220, and re-derived
  in 2.1.251, which is the only claim in this list that has been. That is why
  `F1` is help, `F2` and `F3` are the instrument and the reader's page,
  `F4`/`F5` are focus, `F6` is the ad-hoc ask, `F7` the selection, `F8` the
  queue, `F9` the scratch pad, and `F12` the literal-next alias. The audit
  covered the *bare* keys, so abeam
  claims only those: `Ctrl+F12` and `Shift+F1` are keys nobody has checked, and
  they go to Claude. Swallowing `Ctrl+F12` would have been worse than a dead
  key — it arms literal-next with nothing on screen to say so, and the *next*
  keystroke is forwarded raw as well. What 2.1.251 adds to the picture is a
  second, weaker corroboration: Claude keeps two lists of combinations a user is
  not allowed to rebind, and the F-keys in them are `ctrl+f2`, `ctrl+f3`,
  `ctrl+f8`, `alt+f4` and `shift+f10` — the operating systems' keys, not
  Claude's. Bare F-keys are absent from those lists too.
- **Alt is only partly claimed**: `v m p o t w b f d y`, Up, Down, Backspace.
  Everything else under Alt is free, and Claude's prompt editor
  `preventDefault()`s then *discards* unmatched Alt keys — so those keystrokes
  are dead weight there today and abeam loses nothing by claiming them.
- **`Alt+F` is not free**, which is why the file view is `Alt+E` for "explorer".
  An audit reading only the documented keymap would have shipped that collision.
  Same trap for `Alt+B`, `Alt+D`, `Alt+Y`.

Excluded for other reasons: `Alt+Space` (the Windows system menu, and the window
menu in most Linux desktops), `Alt+Enter` (Windows Terminal fullscreen — and on
Linux, Copilot's own newline, which is the stronger reason of the two and is
argued below), `Ctrl+Alt+*` (AltGr on non-US Windows layouts, where AltGr *is*
Ctrl+Alt; on Linux AltGr is a modifier of its own and the combination is instead
what desktop environments take for switching workspaces), bare PageUp/PageDown
(Claude's Scroll context). Every one of them stays excluded on both platforms.
Only the reasons differ, which is worth noticing rather than smoothing over: an
exclusion that survives a change of platform for a *different* reason is one
nobody should reopen on the grounds that the original reason has gone.

## AltGr is Ctrl+Alt

Not a keymap decision so much as a fact about Windows that the keymap has to
survive: pressing AltGr sets `LEFT_CTRL_PRESSED` beside `RIGHT_ALT_PRESSED`, and
crossterm hands abeam the pair as `ALT | CONTROL`. On a UK, Irish or continental
layout the right-hand `Alt` key *is* AltGr, so **half the keyboard reports every
`Alt` binding with CONTROL set**. `keys::alt_chord` is abeam's one answer to
that, and `keys::global` and every pane-local `Alt` binding read it rather than
testing the modifiers themselves. `altgr_is_alt` walks the whole table and
asserts `Ctrl+Alt`+key resolves exactly as `Alt`+key does, declines included.

Three bugs came out of not having a single answer, and they are worth listing
because they are three different shapes of the same mistake:

- **`Alt+T` in the scratch pad** asked `alt && !ctrl`, so the pad turned over
  from the left `Alt` key only. Every global binding had always worked from
  both. A pane's private definition of a word the rest of the program had
  already defined.
- **The three composers** — the pad, the ask and the queue — guarded text with
  `!ctrl && !alt`, which is a guard against AltGr and therefore against every
  character behind it. `€` on a UK layout; `@`, `€`, `~`, `|`, `[`, `]`, `{`
  and `}` on a German one. Typed, and silently dropped. `keys::is_text` is the
  shared answer, and it reads `Ctrl` and `Alt` *together* as no modifier at all.
- **`Ctrl+\` literal-next** matched on `ctrl` alone. On the layouts that put `\`
  behind AltGr — German, Spanish, Italian — typing a backslash therefore armed
  literal-next and sent the *next* keystroke to the agent raw. The note beside
  that binding said the key was "awkward" on those layouts and offered `F12` as
  an alias; what was actually true is that the character was unreachable and the
  keystroke after it was misrouted. It now matches `ctrl && !alt`.

Nothing is given up by counting `Ctrl+Alt` as `Alt`, and the reason belongs to
crossterm rather than to this document: when an AltGr combination *produces* a
character, the reported `KeyCode` is that character — `€`, not the `e` under the
key — because `u_char` is non-zero and the keyboard-layout fallback never runs.
A binding letter only ever arrives from a combination that typed nothing, so a
binding and a layout's AltGr text cannot be the same event. What `is_text` does
give up is `Ctrl+Alt`+letter as a chord, which nothing binds and nothing hosted
can hear: all three composers are abeam's own, with no child in them.

Two things this does **not** fix, both of them outside abeam. A terminal that
takes `Alt`+letter for its own menu accelerators before the application sees it
— several IDE terminals do — cannot be reached from here; and a console that
reports `Alt` as an `Esc` prefix rather than as a modifier sends two events, the
first of which reads as a bare `Esc`. `crates/abeam/examples/keyprobe.rs` tells
the three cases apart: it prints the modifier set, names which `Alt` key
arrived, and names the binding each event would resolve to.

## Known gaps, against Claude

- Claude's keybindings are user-configurable (`~/.claude/keybindings.json`) and
  Anthropic ships new ones regularly. The Alt namespace is free *today*, not
  forever. `Ctrl+\` literal-next is the pressure-release valve, and abeam's own
  bindings should become configurable before 1.0.
- **`Alt+J` — watch this one.** Claude has a live `app:toggleTerminal` action
  with a Global handler and **no default key**, so its footer falls back to
  printing the literal `meta + j` (and fires a `tengu_keybinding_fallback_used`
  event each time it does). Nothing is bound, so abeam's invariant holds and
  `Alt+J` still scrolls the right pane — but **Claude's own UI is already
  telling the user that key toggles a terminal**, and Anthropic is one line from
  making that true. This is the first case of a Claude action whose *intended*
  key collides with an abeam binding, and it is exactly the scenario the "free
  today, not forever" caveat above was written for. If it lands, `Alt+↑`/`Alt+↓`
  are not available either (`app:diffFileList`), so the replacement is an F-key
  — a move this project has since made in earnest, for `Alt+←`/`Alt+→`, and on
  better evidence than a footer hint. The F-keys are the reserve, and the
  Copilot section below is why there is no other. Re-checked in 2.1.251 while
  clearing `F9`, because a five-week-old warning about a key that might be
  bound next week is worth thirty seconds: still exactly as described, an
  `app:toggleTerminal` in the Global context with `meta+j` supplied as the
  footer's fallback string and no `meta+j` anywhere in the declared bindings
  table. Unchanged is not the same as settled.
- **`Alt+M` is conditionally `chat:cycleMode`.** The Chat block binds a computed
  key: `shift+tab` normally, but `meta+m` on Windows when the runtime is outside
  a version range checked at startup. `m` is in the presumed-taken set already,
  so nothing is broken — the lesson is that a *computed* binding is invisible to
  a grep for the literal, and this table can only be trusted for the ones spelt
  out.
- **`Ctrl+\` is in Claude's own reserved-key table** as `severity: "error"`,
  reason "Terminal quit signal (SIGQUIT)". This entry used to end "on a Unix
  port that binding would signal the process group". The Unix port has arrived,
  so here is what is actually true rather than what was feared. abeam's own
  terminal is in raw mode, and raw mode clears `ISIG`, so `Ctrl+\` does not
  signal abeam; and abeam matches the key in `keys.rs` before anything is
  written to the pty, so it does not reach the agent either. The one live route
  is literal-next aimed at the key itself — `Ctrl+\` then `Ctrl+\` — which
  writes `0x1c` into the child's pty, where a program that has *not* put that
  pty into raw mode gets `SIGQUIT` for its foreground process group instead of a
  keystroke. Claude and Copilot put their terminal into raw mode, so both ought
  to read it as an ordinary byte; neither has been tried. `F12` is the alias that
  sidesteps all of it, and this is now a better half of why it exists than the
  AltGr layouts it was written for.
- `Ctrl+Shift+B` (toggleBrief) and `Ctrl+Shift+C` (selection:copy) are
  **unrepresentable in legacy terminal encoding** — no byte sequence
  distinguishes them from `Ctrl+B` / `Ctrl+C`. Those two Claude features are
  simply unavailable inside abeam until the Kitty keyboard protocol is
  implemented. Not caused by abeam, but users will report it as an abeam bug.
- Claude's hold-to-talk voice binds `space` in Chat and needs **key release**
  events to know when you let go. In abeam those releases are dropped
  (load-bearing — see `docs/conpty-findings.md`), and nothing it advertises
  reports them. Push-to-talk will not work inside abeam.
- ~~`ctrl_byte` in `input.rs` has no arm for `-`~~ — **fixed** during pane
  integration. `Ctrl+-` now encodes to `0x1f` alongside `_` and `/`, so Claude's
  declared `"ctrl+-": "chat:undo"` reaches it. Pinned by
  `ctrl_maps_to_control_codes`.
- `Alt+U`, `Alt+L` and `Alt+C` are treated here as **presumed taken**, not
  verified free. They are classic readline word-case commands and the audit
  found the prompt editor's readline switch is only partly declared. Nothing
  binds them today; nothing should, without re-reading that switch.
- **The inventory above is 2.1.220's and the installed build is 2.1.251.** The
  note under that section's heading has the whole of it — what was re-derived,
  what was not, and the thirty keyless actions that say how urgently the rest
  wants redoing. It is repeated here only because this is the list somebody
  reads when they are deciding what to trust.

## GitHub Copilot CLI's bindings, as of the audited release

> Provenance: **documentation and source, not a binary.** GitHub Copilot CLI
> **1.0.77** — the version published to npm as `@github/copilot`, registry
> mtime 2026-08-01, changelog entry dated 2026-07-30 — audited on 2026-08-02
> from GitHub's published shortcut tables
> (`docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference`,
> whose source lives at `github/docs`), the whole of `changelog.md` in
> `github/copilot-cli` (2025-09-26 through 2026-07-30, about 150 releases), that
> repository's issue tracker, and the source of Ink, the React-for-terminals
> renderer GitHub has said in print that Copilot CLI is built on. This is the
> standalone interactive `copilot`, not the retired `gh copilot suggest` and
> `explain` extension, which had no interactive keymap at all.
>
> **It is weaker evidence than the Claude section above, and the difference is
> not a formality.** The Claude inventory was extracted from the installed
> binary, which is the only reason it caught `Alt+F`, a binding Anthropic's own
> keymap does not declare. Nothing below can catch Copilot's equivalent, because
> nothing below reads Copilot's code. One undeclared Copilot binding has already
> surfaced by accident — `Alt+D`, announced in a changelog entry and absent from
> every published table — which is direct evidence that the tables are
> incomplete in exactly the way Claude's were.
>
> `copilot` is **not installed on this machine** and was not installed to write
> this. It is not on `PATH`; Node here is v20.14.0 with npm 10.9.3, below the
> Node 22 the npm route wants; and `pwsh` is absent, so Copilot's documented
> Windows prerequisite of PowerShell v6 or higher is unmet. Nothing was
> installed, and no strings audit was faked to stand in for one.

**Prompt and global:** `esc` cancel · `ctrl+c` cancel / clear input · `ctrl+d`
shutdown · `ctrl+g` external editor · `ctrl+l` clearScreen · `ctrl+enter` or
`ctrl+q` queue a message · `ctrl+r` history:search · `ctrl+s` stash and pop the
prompt · `ctrl+v` paste as attachment · `ctrl+z` suspend (Unix) · `shift+tab`
cycle standard / plan / autopilot · **`alt+enter`** newline on Windows and
Linux (`shift+enter` or `option+enter` on Mac) · the `@ # ! $ ?` prompt
prefixes, `?` being quick help on an empty prompt

**`ctrl+x` prefix:** `ctrl+x /` slash command · `ctrl+x e` and `ctrl+x ctrl+e`
editor · `ctrl+x b` background · `ctrl+x o` open the last link · `ctrl+x x`
close session · `ctrl+x h` hide the sidebar

**Editing:** `ctrl+a`/`ctrl+e` line ends · `ctrl+b`/`ctrl+f` by character ·
`ctrl+h` delete back · `ctrl+k`/`ctrl+u` kill to end / start · `ctrl+w` delete
word back · home/end, `ctrl+home`/`ctrl+end` · **`alt+left`/`alt+right` move by
word** on Windows and Linux (`option` on Mac) · up/down history · `tab` or
`ctrl+y` accept completion

**Timeline:** `ctrl+f` search · `ctrl+o` expand recent · `ctrl+e` expand all ·
`ctrl+t` reasoning · pageup/pagedown.
**Diff mode:** `j`/`k` and arrows · `h`/`l` file · `g`/`G`, home/end ·
pageup/pagedown · `ctrl+u`/`ctrl+d` half page · `c` comment · `s` summary ·
`b` diff toggle · `w` whitespace · `r` refresh · `enter` submit · `esc` or
`ctrl+c` exit. **Session picker:** arrows · `enter` · `s` sort · `tab`
local/remote · `d` delete · `esc` close.

**Undeclared** — absent from every published table, found only by reading the
changelog: **`alt+d`** delete the word in front of the cursor (1.0.25,
2026-04-13) · **`meta+v`** image paste alongside `ctrl+v` on all platforms
(1.0.30, 2026-04-16) · `alt+backspace` registers as backspace rather than
delete (0.0.417, 2026-02-25) · hold `alt` while scrolling for line-at-a-time
(1.0.69, 2026-07-07). The declared `alt+left`/`alt+right` arrived earlier still,
as "better support for UNIX keyboard bindings (Ctrl+A/E/W/U/K, Alt+arrows)"
(0.0.400, 2026-01-30).

## What that means for Copilot

Three verdicts are used below, and the middle one is the honest one.
**Collision** means GitHub documents the key as doing something. **No-op**
means three independent lines agree it does nothing and one of them is
structural — Copilot could not bind the key even if it wanted to. **Probable
no-op** means nothing was found, and nothing found is not the same as nothing
there: it is precisely the evidence that would have missed `Alt+F` in Claude.
Three rows are marked **inherited** on top of their verdict, and that is a note
about where the evidence came from rather than a fourth grade. `F7`, `F8` and
`F9` were all bound after this audit was taken; the structural argument covers
each without being re-run; and nobody has re-read GitHub's tables since, which
matters more here than elsewhere because GitHub ships roughly weekly. They are
marked identically because their evidence is identical — grading one of three
alike rows differently would have been worse than marking none of them.

The audit found exactly one collision. Its two rows are kept as they were found
and then marked settled, rather than deleted now that abeam no longer binds
those keys: the finding is the most valuable thing in this document, and a table
that only lists keys abeam still holds cannot explain why focus is on an F-key.

| Key | Verdict | Why |
| --- | --- | --- |
| `Alt+Left` | **collision — abeam yielded** | Declared: "move the cursor by a word", Windows and Linux, in the navigation table. It was abeam's focus-left until this audit; it is the agent's now, and focus-left is `F4`. |
| `Alt+Right` | **collision — abeam yielded** | The same binding, the other direction. Focus-right is `F5`. |
| `Alt+G` | probable no-op | Absent from the tables and the changelog. Bare `g` jumps to the first line in diff mode; see the meta-blind caveat below. |
| `Alt+E` | probable no-op | Absent everywhere, and no bare `e` is bound in any view. The cleanest of the letters. |
| `Alt+S` | probable no-op | Absent everywhere. Bare `s` is comments-summary in diff mode and cycle-sort in the session picker; meta-blind caveat. |
| `Alt+Q` | probable no-op | Absent everywhere. `Ctrl+Q` queues a message, which is a different key and one abeam does not take. |
| `Alt+Z` | probable no-op | Absent everywhere. `Ctrl+Z` suspends on Unix; not claimed. |
| `Alt+J` | probable no-op | Absent everywhere. Bare `j` moves down in diff mode; meta-blind caveat, and 1.0.71 fixed this exact class of leak elsewhere. |
| `Alt+K` | probable no-op | As `Alt+J`, with bare `k`. |
| `Alt+PageUp` | probable no-op | Absent everywhere. Bare PageUp scrolls the timeline and pages the diff, and Ink reports `Alt+PageUp` as PageUp with `meta` set. |
| `Alt+PageDown` | probable no-op | As `Alt+PageUp`. |
| `F1` | no-op | Absent from the tables, absent from the changelog, and an Ink `useInput` handler cannot tell one function key from another or from nothing at all. |
| `F2` | no-op | As `F1`. |
| `F3` | no-op | As `F1`. |
| `F4` | no-op | As `F1`, and now load-bearing: this is the argument focus movement rests on. |
| `F5` | no-op | As `F4`. |
| `F6` | no-op | As `F1`, and load-bearing for the same reason `F4` is: the ad-hoc ask rests on it. |
| `F7` | no-op — **inherited** | As `F1`. Bound after this audit was taken, so nothing about `F7` in particular was ever looked for: the Ink argument settles every function key in one move, and there was no per-key search to run. |
| `F8` | no-op — **inherited** | As `F7`. The queue's key since it left `Alt+A`. |
| `F9` | no-op — **inherited** | As `F7`. The scratch pad's key. None of these three was in this table until the pad landed and somebody counted its rows against `keys.rs`, which is the table's own small lesson about tables. |
| `F12` | no-op | As `F1`. |
| `Ctrl+\` | no-op | Absent from the tables and the changelog, and in legacy encoding it is a byte Ink's parser matches no branch of. |

`Esc` and `Shift+Tab` are Copilot's — cancel, and cycling standard / plan /
autopilot — and abeam claims neither. `keys.rs` returns `None` for a bare `Esc`
and for `Shift+Tab` in either spelling, `BackTab` or `Tab`+SHIFT, because all of
them fall past the `!alt` guard. `plain_typing_is_never_a_global` pins the bare
`Esc` and `Tab` halves of that; the modified spelling of `Shift+Tab` is pinned
by nothing and is safe by the same structural argument as every other
unmentioned key. The right pane's own `Esc` and `q` are reachable only when that
pane has focus, so Copilot never loses them.

`Alt+Enter` is Copilot's newline on Windows and Linux both, and abeam already
excluded it for a reason belonging to only one of those (Windows Terminal
fullscreen). Inside abeam that exclusion becomes load-bearing rather than
incidental, on either platform: Copilot's other newline, `Shift+Enter`, needs
the Kitty keyboard protocol, which abeam does not implement, so `Alt+Enter` is
the *only* way to get a newline into a Copilot prompt hosted here. On Linux that
also means the key has to *arrive* — an exclusion is worth nothing if the
terminal or the desktop eats the key on its way in — which is the keyprobe's
question and not this document's.

### Why Ink settles the function keys and unsettles the letters

Copilot CLI is an Ink application, and that is not a trivia point: it fixes what
Copilot is *able* to bind, and two of Ink's limits do most of the work here.

The first is that Ink cannot see which function key was pressed. Its
`parseKeypress` does recognise them — `\x1bOP` becomes the name `f1`,
`\x1b[24~` becomes `f12`, which is exactly what `abeam-pty` sends. But
`useInput`, the hook an Ink app handles keys with, exposes a fixed record with
fields for the arrows, PageUp/PageDown, Home/End, Return, Escape, Tab,
Backspace, Delete and the modifiers, and **no field for a function key at all**;
and because `f1` through `f12` are in Ink's `nonAlphanumericKeys`, the `input`
string is blanked as well. Every bare function key therefore arrives at every
handler as an empty string with every flag false — indistinguishable from any
other function key, and from nothing having happened. An Ink app cannot bind a
bare function key through `useInput`. That agrees with the two documentary
lines: no function key appears in any published Copilot shortcut table, and none
appears anywhere in the changelog across about 150 releases.

That argument used to carry `F1`, `F2`, `F3` and `F12` — help, an instrument, a
page colour and an alias for a key that already had one. It now also carries
focus movement, which after the view keys is the binding a user reaches for
most. Being *structural* rather than evidential is exactly why it was safe to
put focus there instead of on a fourth Alt letter: a letter would have been
cleared by the same kind of search that cleared `Alt+F` in Claude, and `Alt+F`
was bound.

`Ctrl+\` fails the same way and for a related reason. In legacy encoding it is
the single byte `0x1c`, and Ink's parser only turns a lone byte into
`ctrl`+letter for `0x01` through `0x1a`; `0x1c` falls past every branch, leaving
a nameless character with `ctrl`, `meta` and `shift` all false. Under the Kitty
keyboard protocol it would be distinguishable, but inside abeam the pty is a
legacy vt100, so the legacy branch is the one that runs. This is the same
argument the existing note about `Ctrl+Shift+B` makes, pointed the other way:
what a legacy terminal cannot express, the hosted program cannot bind.

The second limit cuts against abeam, and it is the one to keep in mind. Ink
parses `Alt`+letter with `/^(?:\x1b)([a-zA-Z0-9])$/`, setting the name to the
letter and `meta` to true, and `useInput` then hands the handler the sequence
with the escape prefix stripped. **`Alt+J` reaches an Ink handler as
`input === "j"` with `key.meta === true`** — so a handler written
`if (input === "j")`, which never tests `key.meta`, fires on `Alt+J` as readily
as on `j`. Copilot has shipped fixes for precisely this: 1.0.71 (2026-07-16)
records that "modified vim keys (Ctrl+K, uppercase J/K) no longer move the
selection in tool-permission prompts". Every bare-letter binding in Copilot's
diff mode and session picker is therefore a *possible* Alt binding, decided by a
handler this audit cannot read.

That matters less than it first looks, and it is worth being exact about why.
abeam intercepts `Alt+J`; Copilot's own way to move down a diff is bare `j`,
which abeam never touches, so nothing a user can reach is shadowed. What is not
true is the strict form of the invariant — that the intercepted key is one the
agent could not have acted on — and this document should not pretend otherwise.
Those six are *unverified*, though, not known broken. The one key pair this
audit knew to be broken was `Alt+←`/`Alt+→`, and abeam no longer binds it.

### The collision, and how it was settled

**`Alt+Left` and `Alt+Right` were a collision, and a declared one.** GitHub's
navigation table binds them to moving the cursor by a word on Windows and Linux,
and the changelog has carried them since 0.0.400 (2026-01-30). They were abeam's
focus-movement keys. Inside abeam a Copilot user would have had no word-left or
word-right in the prompt at all: `Ctrl+B` and `Ctrl+F` move by a single
character, `Ctrl+W` deletes a word backwards and the undeclared `Alt+D` deletes
one forwards, but nothing else *moves* by a word. No argument about Ink rescues
this one — it is spelt out in the vendor's own reference, in the same table as
the bindings abeam relies on being complete.

**abeam yielded.** Focus is `F4` and `F5`; `Alt+←` and `Alt+→` fall through to
the agent like any other key abeam does not claim. Three things about that are
worth writing down, because the next collision will need them.

*One table, every agent.* Making the arrows mean focus in front of Claude and
word-motion in front of Copilot was considered and rejected. A key that means
different things depending on what is hosted is a key nobody can learn, and it
turns the `F1` overlay from a statement of fact into a claim about which agent
is running. The invariant is about what abeam *intercepts*; abeam should
intercept the same set whoever is listening.

*abeam yielded rather than the agent, because the two losses are not
comparable.* The agent's prompt is where the user is actually typing, the
collision is in the middle of that, and there is no second way to move by a word
once those keys are gone. abeam's focus movement can be spelt however abeam
likes, and it is not even the only way to move focus — a mouse click and `Esc`
from the right pane both do it.

*The replacement is a function key rather than another Alt letter*, and that is
the part that generalises. Every "probable no-op" in the table above could be
wrong in the way `Alt+F` was wrong in Claude. `F4` and `F5` cannot be wrong that
way: an Ink `useInput` handler is not handed enough information to tell one
function key from another, or from nothing at all. When a letter and an F-key
are both apparently free, they are not equally free.

The rows above are kept rather than tidied away because this is the finding the
whole audit paid for: **a vendor's declared binding moved one of abeam's keys**,
and it was found by reading GitHub's own reference rather than by a user
reporting that word-motion was broken. The version and date are recorded
(0.0.400, 2026-01-30) so that the next person to ask why focus is on an F-key
gets the evidence and not just the conclusion.

### What would upgrade this evidence

Two steps, in this order, and neither has been taken.

**Install `copilot` and audit the binary.** On this machine that means clearing
the prerequisites first — Node 22 or newer for `npm install -g @github/copilot`,
or `winget install GitHub.Copilot`, which sidesteps npm entirely, plus
PowerShell 6 or higher for the documented Windows path. Note that the npm
package is a 13 KB loader (`npm-loader.js`) that fetches a platform binary, so
the artefact a strings audit must run against is that downloaded binary and not
anything npm unpacks. Then repeat the Claude method: search the extracted
strings for the declared shortcut tables, and then — the part that actually
earns the confidence — for the *undeclared* comparisons, `meta` or `alt` tested
against a single letter, in the prompt editor and in each modal view. Record the
version, the byte size and the mtime, as the Claude provenance above does.

An install now buys a second thing, and it is the larger of the two: the ask
pane can drive `copilot -p`, and **not one of those flags has ever been run**.
`crates/abeam/src/ask/copilot.rs` names beside each choice what would fail first
if the documentation is wrong, and the three worth checking on the day there is
a binary are whether `-p` is programmatic mode rather than a prompt typed into
the UI, whether `--name` and `--resume=<name>` create and pick up a session, and
whether `--deny-tool` refuses what GitHub says it refuses — that last being the
whole of a read-only claim abeam makes to the reader on screen. A keymap audit
is about keys abeam might shadow; this is about authority abeam hands out.

**Then probe both ends of the wire.** `crates/abeam/examples/keyprobe.rs`, run
as `cargo run -p abeam --example keyprobe`, puts the terminal into raw mode and
prints, for every event crossterm reports, the `KeyCode`, the modifier set and
the event kind; it flags any `Char` carrying ALT as one abeam would treat as an
Alt binding, and flags a lone `Esc` as the sign that this terminal sends Alt as
an escape prefix rather than as a modifier. Three presses of `Esc` exit. It
answers the half of the question a strings audit cannot: what *this* terminal
actually delivers for each of the twenty keys in the table at the top of this
document, which is not
the same question as what Copilot would do with them. (That number read
"sixteen" until the scratch pad landed, having been written before `F6`, `F7`,
`F8` and `F9` were; it is the smallest thing in this file that had gone quietly
out of date, and it is here rather than deleted because a count nobody
maintains is a count nobody should have written.) **It has been run
against Windows consoles only**, and on Linux it is the more urgent of these two
probes rather than the lesser: whether an `Alt` combination arrives as a
modifier, as an `Esc` prefix, or not at all because the window manager took it
first is a question about that terminal and that desktop, and nothing found by
reading an agent's source answers it. So run it first in the terminal abeam is
launched from, on the platform you are on, to confirm every one of the twenty
arrives as abeam expects — `F9` most of all, because it is the newest and
because `F10` and `F11` were passed over precisely on the grounds that a
terminal can eat a function key before an application sees it, and nobody has
checked that `F9` is not eaten too; then host `copilot` inside abeam and send each key
through to it with literal-next (`Ctrl+\` or `F12`, then the key), in the
prompt, in `/diff` and in the session picker, watching for any response. A key
that is dead in all three views is as close to proof as this project gets
without reading GitHub's code.

`Alt+←` and `Alt+→` want probing too, and in the opposite direction: they are no
longer abeam's, so what has to be confirmed is that they *arrive* — that a word
really does move in a Copilot prompt hosted here. There is a specific thing to
watch. `abeam-pty` encodes them as the xterm modified-arrow form, `\x1b[1;3D`
and `\x1b[1;3C` (`input::cursor_key`), rather than as an `Esc` prefix in front
of a bare arrow, and only Copilot's parser can say whether it reads that form as
`left` with `meta` set. Yielding a key is worth nothing if the key then fails to
work, and no test in this repository can tell:
`the_agents_alt_bindings_are_left_alone` pins that abeam does not claim them,
which is the whole of what abeam controls.

## Known gaps, against Copilot

- **Everything above is documentation-derived.** It is the kind of audit that
  would have cleared `Alt+F` in Claude, and `Alt+F` was bound. Treat every
  "probable no-op" as unproven, not as safe.
- **The meta-blind handler risk is unquantified.** `Alt+G`, `Alt+S`, `Alt+J`,
  `Alt+K`, `Alt+PageUp` and `Alt+PageDown` all shadow a key Copilot binds in its
  bare form somewhere, and Ink hands a handler the bare form plus a `meta` flag
  it is free to ignore. Nothing a user can reach is lost either way — the bare
  keys still pass through — but the strict invariant is unverified for those
  six, and only the binary can settle it.
- **Copilot's keybindings are not user-configurable**, which is the mirror image
  of the Claude gap rather than an absence of one. Issue #2259 asks for a
  `~/.copilot/keybindings.json` and is open, so today the table is the table for
  every user; but GitHub ships roughly weekly and has changed key bindings
  repeatedly, including adding `Alt+D` mid-life. This inventory rots faster than
  the Claude one, not slower.
- **The Ink version is unverifiable.** The reasoning above reads Ink's current
  source; Copilot ships a bundled binary whose npm manifest declares one
  unrelated dependency, so neither the Ink version it vendors nor any local
  patch to `useInput` can be checked from here.
- **Windows Alt handling is unsettled in Copilot itself.** Issues #1999 and
  #2424 report AltGr combinations being swallowed on German layouts, so users on
  those layouts already have Alt trouble that abeam neither causes nor can fix,
  and that abeam will nevertheless be blamed for. Both reports are about
  Windows, where AltGr *is* Ctrl+Alt; the Linux equivalent has not been looked
  for, and not looked for is not the same as not there.
- **`Ctrl+Z` has stopped being an inert line in the inventory.** Copilot's
  prompt list carries `ctrl+z` suspend, marked *(Unix)*, and until this port
  that annotation meant "not here". abeam does not claim the key — it encodes to
  `0x1a` and goes to the agent like any other byte, and abeam's own terminal is
  in raw mode so nothing suspends abeam — but what the agent does next is the
  agent's, and untested. abeam is not a job-control shell and has no `fg`, so an
  agent that stops itself in response stops with nothing on the abeam side able
  to continue it; `Alt+Q` is the way out. Claude's audited inventory has no
  `ctrl+z` at all.
- **The Kitty keyboard protocol is the shared assumption.** Copilot uses it when
  the terminal offers it, and several of its behaviours — `Shift+Enter` for
  newline, extended key reporting — exist only on that path. abeam does not
  implement it, so inside abeam Copilot falls back to legacy encoding. That is
  what makes the `Ctrl+\` and function-key reasoning above apply; it would need
  re-checking the day abeam gains Kitty support.

## Codex CLI's bindings, as of the audited build

The official OpenAI documentation establishes two facts that change the shape
of this audit: Codex exposes `/keymap`, and bindings persist under
`tui.keymap.<context>.<action>` in `config.toml`. Contexts include `global`,
`chat`, `composer`, `editor`, `pager`, `list` and `approval`; function keys are
valid remap targets. There is therefore no agent-independent claim that a key
left unused by Codex's defaults will remain unused in every Codex installation.

The static audit used the official Windows npm artefact recorded in the
provenance block above and OpenAI's complete `built_in_defaults` table at the
matching
[`rust-v0.149.0` source tag](https://github.com/openai/codex/blob/rust-v0.149.0/codex-rs/tui/src/keymap.rs).
The binary contains `alt-a` beside `tui.keymap.global.open_agents`; the source
confirms that binding and contains no function-key default. That makes `Alt+A`
a direct collision with abeam's former queue binding and clears bare `F8` as
its replacement against this version's complete shipped defaults.

`F9` was run against that same artefact on 2026-08-29, its SHA-256 checked
first, and the finding is the one `F8` already has rather than a new one. Two
searches, and the first of them is what makes the second worth anything.
`alt-a` beside `tui.keymap.global.open_agents` is the only `alt-`prefixed
binding literal in the whole binary, which is a method finding the one collision
Codex is known to have; run over the same strings, `f7` through `f12` appear
nowhere as standalone tokens, and the byte sequences that read as `f9` are
x86 instruction fragments in printable runs. So `F9` and `F8` rest on one
artefact and one standard, and it is the weakest standard in this document:
absence from a shipped default table, in an agent that can both see a function
key and be told to act on one.

| Key | Verdict against Codex 0.149.0 defaults | Why |
| --- | --- | --- |
| `Alt+A` | **collision — abeam yielded** | Codex opens its shared agent-session overview; abeam now forwards the key. |
| `F8` | no default binding | The complete matching default table contains no function-key binding; Codex can still bind it through a custom keymap. |
| `F9` | no default binding | The same artefact, re-hashed and re-searched for the scratch pad: no function-key literal at all. Remappable, exactly as `F8` is. |
| Other abeam globals | clear in the default table | No collision appears in the complete matching `built_in_defaults`; this was not a live authenticated-modal audit. |

### Known gaps, against Codex

- **The live audit stopped at authentication.** The official 0.149.0 binary was
  hosted through abeam on Windows with an isolated `CODEX_HOME`: its welcome
  and sign-in UI rendered, a 120×40 → 100×32 outer resize completed with the
  UI still navigable, Down-arrow selection worked, and the two-step `Alt+Q`
  quit completed. No account was
  connected and no prompt was submitted, so authenticated composer, approval,
  pager and agent-session modes remain untested.
- **Custom keymaps can invalidate the default audit.** A user can assign `F8`,
  `F9` or another abeam key to a Codex action. abeam does not parse Codex's
  layered configuration and cannot promise the strict invariant for arbitrary
  `tui.keymap` overrides. Literal-next (`Ctrl+\` or `F12`) is the recovery path.
- **The audit is Windows-only.** A Linux Codex binary and Linux terminal path
  were not inspected. Run `keyprobe`, then host Codex and use literal-next to
  exercise every abeam global in the composer, lists, pager and approval UI.
