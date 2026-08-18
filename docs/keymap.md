# Keymap, and the collision audit behind it

> Provenance, for the Claude sections: extracted from
> `C:/Users/philm/.local/bin/claude.exe` — **Claude Code 2.1.220**, 265,720,480
> bytes, mtime 2026-07-25 — by the design pass on 2026-08-01 and re-checked on
> 2026-08-01 when the command view added `Alt+S`. Written down because it was
> expensive to obtain and will silently rot otherwise. The version and size are
> recorded as well as the date: a date alone cannot tell a re-install of the
> same build from a silent update.
>
> `~/.claude/keybindings.json` does not exist on this machine, so nothing here
> is overridden by user configuration. If it ever does, this whole document is
> describing defaults the user may not be using.
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

## The invariant

**Nothing abeam intercepts may be a key any supported agent can act on.** The
plural is the whole difficulty: a binding is safe only if it is a no-op in
*every* agent abeam can host, so gaining an agent can retire a key that was
safe when there was one. It has already retired one.

Every abeam binding below was checked against the Claude inventory below and is
a verified no-op in Claude today. Against Copilot CLI the same check has been
made from documentation and source rather than from the binary, it is honestly
weaker, and it found one collision: `Alt+Left` and `Alt+Right`, which GitHub
declares as word-motion in its own command reference. **abeam gave them up.**
Focus movement is `F4` and `F5`, and the arrows go to the agent.

That removed the one breach this audit knew about, and yielding a key verifies
nothing else, so be exact about what the invariant is worth in each agent. In
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
"The collision, and how it was settled" below.

Typing at the agent is byte-for-byte what the pty spike did.

## abeam's bindings

`crates/abeam/src/keys.rs` is the single table. Globals work at any focus.

| Key | Action |
| --- | --- |
| `Alt+G` | right pane → git view (focus unchanged) |
| `Alt+E` | right pane → files / markdown view (focus unchanged) |
| `Alt+S` | right pane → a shell, **and focus it**; again to hand focus back |
| `Alt+A` | right pane → the queue of work for the agent (focus unchanged) |
| `F4` / `F5` | move focus left / right |
| `Alt+J` / `Alt+K` | scroll right pane one line — **without focusing it** |
| `Alt+PageDown` / `Alt+PageUp` | scroll right pane one page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (press twice while a child is live) |
| `F1` | key help overlay |
| `F2` | right pane → pty diagnostics, and back to what it displaced (focus unchanged) |
| `F3` | file reader → light / dark page (no other view changes) |
| `F6` | right pane → ask, **nothing attached, and focus it**; again for what it displaced |
| `F7` | select rows of the right pane by keyboard, **and focus it**; again to put the selection away |
| `Ctrl+\` or `F12` | literal-next: send the following key to the agent verbatim |

**"Focus unchanged" holds in both directions.** The four rows marked so —
`Alt+G`, `Alt+E`, `Alt+A` and `F2` — neither take focus nor hand it back:
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

`Alt+S` is the one workspace view key that moves focus, and the exception is
deliberate: a command line you have to press a second key to type into is a
picture of a command line. (`F6` and `F7` move it too, and say so in the table;
neither is one of the four views `Alt+G`, `Alt+E`, `Alt+S` and `Alt+A` switch
between.) It is also the only right-hand view that keeps `Esc` and `q` —
they belong to the shell — so the border advertises the way out instead.

**`Alt+A` was verified the same way, on 2026-08-02, against the same 2.1.220
binary**: `rg -a 'meta\+a\b|alt\+a\b'` over `claude.exe` returns **zero
matches**, where the undeclared readline bindings that caught `Alt+F` do appear
as text — so this is the strong form of the test rather than the absence of
documentation that would have cleared `Alt+F`. `a` is also outside the classic
readline meta set (`b f d l u c t r y n p`) that Claude's prompt editor handles
without declaring.

It is a letter and not an F-key on purpose, and the reason is the set rather
than the key: `Alt+G`, `Alt+E`, `Alt+S` and `Alt+A` are the four workspace
views, and a fourth spelled `F6` would be a binding nobody groups with the
other three. The F-key argument in `keys.rs` is about what is *structurally*
safe in both agents, and it is still the right answer for a key that has no
set to join — which is why `F2` and `F3` are F-keys and this is not.

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
gone twice over — `Alt+A` is the queue, `Alt+Q` is quit, and `?` is a shifted
key whose `Alt` form no audit here has looked at in either agent — and by the
time this landed there were two agents to clear a binding against rather than
one. `Alt` is the namespace both of them actually use; the F-keys are the one
namespace both leave alone, and the Copilot half of that is *structural* rather
than merely unrefuted (see "Why Ink settles the function keys and unsettles the
letters"). It joins `F2` rather than the four `Alt` view keys, and that grouping
is real: the diagnostics and the ask are the two views that displace another and
put it back, and neither is remembered as a workspace view. The `Alt+A`
paragraph above still stands — a fifth *view* spelled `F6` would be a key nobody
groups with the other four — because this is not a fifth view.

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
any pane is offered anything, and inside `global` only the namespace both agents
leave alone.

It is also not the way most people will copy anything, and the table above is
right to be the only place it looks central. **A drag in the right pane selects
and copies on its own**, with no key and no mode — the gesture the host
terminal's own selection used, kept doing what it did. `F7` is what a keyboard
has instead, and what anyone has when the right pane is running something that
asked for the mouse.

What it opens is a mode, and the mode is the third place in abeam where the
ordinary vocabulary is suspended — the find boxes and the ask's composer being
the other two, both of which the F1 overlay states in a row of its own. This one
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

The files view adds `t` — rendered markdown or its source — `Backspace` or `-`
to climb a directory in the file list, and three searches: `/` finds a file
anywhere under the root while the list is up and a phrase on the page while a
document is, `n` and `N` walk that document's matches, and `f` reads every file
under the root for a phrase. The queue view adds `i` to write an item, `a` to
arm or disarm sending, `d` to delete one, `m` to switch an item between being
typed into the live session and being dispatched as its own background agent,
and `Enter` to do the selected one now. None of these collides with the
vocabulary above, and none can reach the agent: the right pane has to be focused
for any of them to be seen. That exemption is stated once rather than argued
beside each key, and the place is the module doc at the top of `keys.rs`:
*intercept* means what `global` claims before any pane is offered a key, and
`global` returns `None` for every bare printable one of these.

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

## Claude Code's bindings, as of the audited build

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
- **No F-key is bound anywhere, in any context.** That is why `F1` is help,
  `F2` and `F3` are the instrument and the reader's page, `F4`/`F5` are focus,
  `F6` is the ad-hoc ask, and `F12` is the literal-next alias. The audit covered
  the *bare* keys, so abeam
  claims only those: `Ctrl+F12` and `Shift+F1` are keys nobody has checked, and
  they go to Claude. Swallowing `Ctrl+F12` would have been worse than a dead
  key — it arms literal-next with nothing on screen to say so, and the *next*
  keystroke is forwarded raw as well.
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
  Copilot section below is why there is no other.
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
  keystroke. Both agents put their terminal into raw mode, so both ought to read
  it as an ordinary byte; neither has been tried. `F12` is the alias that
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
actually delivers for each of the sixteen keys abeam still binds, which is not
the same question as what Copilot would do with them. **It has been run
against Windows consoles only**, and on Linux it is the more urgent of these two
probes rather than the lesser: whether an `Alt` combination arrives as a
modifier, as an `Esc` prefix, or not at all because the window manager took it
first is a question about that terminal and that desktop, and nothing found by
reading an agent's source answers it. So run it first in the terminal abeam is
launched from, on the platform you are on, to confirm every one of the sixteen
arrives as abeam expects; then host `copilot` inside abeam and send each key
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
