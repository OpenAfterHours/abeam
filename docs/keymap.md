# Keymap, and the collision audit behind it

> Provenance: extracted from `C:/Users/philm/.local/bin/claude.exe` — **Claude
> Code 2.1.220**, 265,720,480 bytes, mtime 2026-07-25 — by the design pass on
> 2026-08-01 and re-checked on 2026-08-01 when the command view added `Alt+S`.
> Written down because it was expensive to obtain and will silently rot
> otherwise. The version and size are recorded as well as the date: a date alone
> cannot tell a re-install of the same build from a silent update.
>
> `~/.claude/keybindings.json` does not exist on this machine, so nothing here
> is overridden by user configuration. If it ever does, this whole document is
> describing defaults the user may not be using.

## The invariant

**Nothing abeam intercepts is a key Claude can act on.** Every abeam binding
below was checked against the inventory below and is a verified no-op in Claude
today. Typing at Claude is byte-for-byte what the pty spike did.

## abeam's bindings

`crates/abeam/src/keys.rs` is the single table. Globals work at any focus.

| Key | Action |
| --- | --- |
| `Alt+G` | right pane → git view (focus unchanged) |
| `Alt+E` | right pane → files / markdown view (focus unchanged) |
| `Alt+S` | right pane → a shell, **and focus it**; again to hand focus back |
| `Alt+Right` / `Alt+Left` | move focus right / left |
| `Alt+J` / `Alt+K` | scroll right pane one line — **without focusing it** |
| `Alt+PageDown` / `Alt+PageUp` | scroll right pane one page — without focusing it |
| `Alt+Z` | zoom: hide / show the right pane |
| `Alt+Q` | quit (press twice while a child is live) |
| `F1` | key help overlay |
| `F2` | right pane → pty diagnostics, and back to what it displaced |
| `Ctrl+\` or `F12` | literal-next: send the following key to Claude verbatim |

`Alt+E` pressed while the files view is already showing opens the file list, so
it is never a key that does nothing. It used to reload the open file; reload is
`r` from inside the pane and the watcher does it unasked, so the second press
was spending a key on a job already done twice.

`Alt+S` is the one binding that moves focus, and the exception is deliberate: a
command line you have to press a second key to type into is a picture of a
command line. It is also the only right-hand view that keeps `Esc` and `q` —
they belong to the shell — so the border advertises the way out instead.

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

Right pane, only when focused. Plain keys, because Claude never sees them.
Deliberately the same vocabulary as Claude's own transcript view, so there is
one scroll language in the application:

`j`/`k`, arrows — line · `space`/`b`, PgDn/PgUp — page · `Ctrl+D`/`Ctrl+U` —
half page · `g`/`G`, Home/End — ends · `Tab`/`Shift+Tab` — next/prev item ·
`Enter` — open · `r` — refresh · `Esc`/`q` — back to Claude.

The files view adds `t` — rendered markdown or its source — and, in the file
list, `/` to find a file anywhere under the root and `Backspace` or `-` to climb
a directory. None of these collides with the vocabulary above, and none can
reach Claude: the right pane has to be focused for any of them to be seen.

Two of them are claimed *conditionally*, which is the thing to keep straight.
While a find is open the query eats every printable key, so `j`, `k`, `g`, `G`,
`b`, `q` and `r` are text rather than commands, the selection moves on the
arrows and `Ctrl+N`/`Ctrl+P`, and `Esc` closes the find instead of leaving the
pane. That is why `Alt+J` and friends reach a pane through `Pane::scroll_key`
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
- **No F-key is bound anywhere, in any context.** That is why `F1` is help and
  `F12` is the literal-next alias. The audit covered the *bare* keys, so abeam
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

Excluded for other reasons: `Alt+Space` (Windows system menu), `Alt+Enter`
(Windows Terminal fullscreen), `Ctrl+Alt+*` (AltGr on non-US layouts), bare
PageUp/PageDown (Claude's Scroll context).

## Known gaps

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
  are not available either (`app:diffFileList`), so the replacement is an F-key.
- **`Alt+M` is conditionally `chat:cycleMode`.** The Chat block binds a computed
  key: `shift+tab` normally, but `meta+m` on Windows when the runtime is outside
  a version range checked at startup. `m` is in the presumed-taken set already,
  so nothing is broken — the lesson is that a *computed* binding is invisible to
  a grep for the literal, and this table can only be trusted for the ones spelt
  out.
- **`Ctrl+\` is in Claude's own reserved-key table** as `severity: "error"`,
  reason "Terminal quit signal (SIGQUIT)". Harmless on Windows — there is no
  SIGQUIT, and abeam intercepts the key before the pty — but on a Unix port that
  binding would signal the process group. `F12` is the alias that already covers
  it, and that is why it exists.
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
