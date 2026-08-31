# Keyboard UX proposal

## Recommendation

Make `F1` the single, reliable entry point for **application commands**. Pressing
it opens a visible command hub; the next key is a mnemonic, not a simultaneous
chord. Keep only the few actions that need to be immediate as direct function
keys.

This is the recommended direction because it gives the application one learnable
global language, scales past the usable F-key slots, and does not make a
Windows-specific Alt/AltGr distinction part of normal use. Existing Alt and
direct-F-key bindings are removed in this release: the command hub is the one
supported global map. There is no Classic mode, compatibility switch, or
undocumented alias to maintain.

The proposal is deliberately about *global* commands. Focus-local keys such as
the file browser's `/`, the queue's `i`, and document navigation should remain
local: the focused right pane owns them, so they never steal a key from an
agent that is accepting text.

## What the retired map got right

The existing map has important constraints that the replacement must preserve:

- A global binding is intercepted in front of Claude, Copilot, or Codex. Bare
  letters and Ctrl-letter combinations therefore cannot safely be used as a
  global namespace.
- `F10` and `F11` are not dependable application keys in terminal emulators;
  they are commonly consumed by a menu bar or fullscreen. Function keys cannot
  be the whole answer.
- Some keys must work while a live shell has focus, so a right-pane-local
  shortcut is insufficient for selection and for returning control.
- `Ctrl+\\` / `F12` is a necessary escape hatch: it lets a user send a future
  colliding binding to the hosted agent verbatim.
- The current distinction between read-only views and text-entry views is
  sound: opening a shell, pad, or ask composer should make it immediately
  usable; opening git, files, queue, or diagnostics is usually a glance.

The issue is not that the choices are careless. It is that users have to infer
which of three unrelated namespaces a command belongs to:

| Namespace | Retired examples | User cost |
| --- | --- | --- |
| Alt mnemonic | `Alt+G`, `Alt+E`, `Alt+S`, `Alt+Q`, `Alt+Z` | Physical Alt behaviour is not dependable on Windows; some of the letters do not describe a shared category. |
| Numbered function key | `F2` through `F9`, plus `F12` | Numbers are safe but do not carry a mnemonic or a clear grouping once there are more commands than positions. |
| Focus-local letter | `/`, `t`, `w`, `?`, queue actions | These are appropriate in context, but are mixed into the same long help table as global controls. |

Several global keys also have state-dependent second meanings: `F4` can focus
the agent or cycle agents; `Alt+E` can open the file list; `Alt+S` and `F9` can
return focus. Those are individually defensible, but together make the map
hard to predict before pressing a key.

## Windows Alt and AltGr finding

The implementation intentionally treats an event containing `ALT` as an Alt
command even when Windows also reports `CONTROL` (its usual representation of
AltGr). That is why `AltGr+S` and `Alt+Shift+S` could reach the retired shell
binding. It is the right defensive rule once an event has reached the program.

It does not guarantee that physical left Alt and right Alt/AltGr arrive through
Windows Terminal, conhost, IDE terminals, and keyboard layouts in the same
form. The reported failure of left `Alt+S` is therefore a product UX failure
even if the key resolver behaves as designed. A person should not need to know
which physical Alt key their terminal happens to encode successfully in order
to open a shell.

Consequences for the new design:

- Do not make an Alt chord the only or primary route to any application action.
- Do not describe `Alt` and `AltGr` as interchangeable in the primary help.
  They are not application-command routes in this map.
- Keep `F12` as the portable literal-next route; `Ctrl+\\` must remain an alias
  only because a backslash can itself require AltGr on some layouts.
- Verify that ordinary Alt and AltGr text input still reaches the focused child;
  neither modifier should activate an application command.

## Option A - F1 command hub (recommended)

`F1` opens a small, non-search command hub and keeps it open until the user
chooses a command or presses `Esc`. The UI displays the mnemonic beside every
action. This is a **sequence**, written `F1, S`, not a chord: press and release
`F1`, then press `S`. There is no timeout and no hidden leader state.

The function row is then reserved for actions that are either safety-critical
or frequent enough to benefit from a single key:

| Key | Meaning | Why it remains direct |
| --- | --- | --- |
| `F1` | Open the command hub; `?` in the hub shows the full reference | One safe, discoverable application gateway. |
| `F4` | Focus the current agent | Returning to the agent should be immediate and reliable. |
| `F5` | Show and focus the right pane | The opposite focus action belongs beside `F4`. |
| `F7` | Start/end keyboard selection in the right pane | Must work over a live shell and is an active interaction, not a view. |
| `F12` | Send the next key verbatim to the agent | Recovery/safety mechanism; retain `Ctrl+\\` where available. |

`F4` should no longer silently cycle agents when focus is already left. Cycling
is explicit (`F1, N`) and the hub names the target. This removes the only
global key whose first and second presses answer different questions.

The hub owns the remaining global commands:

| Sequence | Action | Focus rule |
| --- | --- | --- |
| `F1, G` | Show git | Keep current focus: a glance. |
| `F1, E` | Show files / reader | Keep current focus: a glance. |
| `F1, B` | Open the file browser | Focus right: it accepts navigation input. |
| `F1, S` | Open the shell | Focus right: it accepts text. |
| `F1, W` | Show the work queue | Keep current focus: a glance. `W` avoids colliding with quit. |
| `F1, P` | Open the scratch pad | Focus right: it accepts text. |
| `F1, A` | Open ask with no attachment | Focus right: it accepts text. |
| `F1, D` | Toggle diagnostics | Keep current focus. |
| `F1, T` | Toggle reader theme | Keep current focus. |
| `F1, Z` | Hide/show the right pane | Keep focus on the meaningful existing pane. |
| `F1, J` / `F1, K` | Scroll the right pane one line | Keep current focus. |
| `F1, PageUp` / `F1, PageDown` | Page the right pane | Keep current focus. |
| `F1, N` | Focus the next agent | Explicit multi-agent navigation. |
| `F1, Q` | Quit; enter `F1, Q` again for the existing live-child confirmation | Destructive app action is visible before the second key. |
| `F1, Esc` | Dismiss the hub | Standard cancellation. |

The view command itself selects a stable view; it does not double as the way to
leave it or a second, hidden command. `F1, E` is always the reader and `F1, B`
is always the file browser. `F4` is always the way back to an agent and `F5` is
always the way to enter the right pane. Input-capable views still focus
themselves when opened because that is useful, but repeating their command
merely reopens or focuses that same view; it does not toggle focus away. This
preserves the useful read-versus-write distinction without asking users to
remember four unrelated "press it again" rules.

### Why this works

- It has one global mental model: `F1` means "application command", and the
  following letter says what. Mnemonics are visible rather than memorized.
- It gives every command a route even though only `F1`--`F9` and `F12` are
  dependable, and it leaves no reason to allocate `F10` or `F11`.
- It makes `F1, S` the canonical shell command on every Windows layout. Alt
  can fail without stranding a core workflow.
- It releases the wide Alt namespace back to the agent by default, reducing
  both collision risk and the support burden of per-terminal Alt behaviour.
- It lets new commands add a hub row rather than consuming an arbitrary F-key
  or adding another modifier family.

### Release decision: clean cutover

This release removes the former Alt bindings and direct `F2`, `F3`, `F6`, `F8`,
and `F9` bindings outright. They are not aliases, and there is no
`legacy_keymap` option. Existing users move directly to the visible `F1` hub;
the full reference at `F1, ?` makes the new map discoverable without sustaining
two competing vocabularies.

## Rejected alternative: command palette with classic shortcuts retained

This is the lower-risk alternative. Keep every current binding, but change `F1`
from a static long help overlay into a searchable command palette. Typing
`shell`, `queue`, `focus`, or `theme` filters commands; arrows and `Enter`
choose one. The palette also displays every current shortcut grouped as
**Application**, **Focus**, **Right-pane view**, and **Current view**.

Add `F1, S` as a direct palette mnemonic so that the core Windows problem has a
two-key route even before the user types a search.

This option improves discovery and gives a reliable shell route without
breaking muscle memory. It is not the recommended end state because the old
Alt/F-key split remains the primary map, and users must still learn which keys
are global versus focus-local. It is a good choice only if backwards
compatibility outweighs coherence for the next release.

## Help and on-screen guidance

The current help is comprehensive but flat. It should become progressive:

1. The permanent footer shows only `F1 commands`, `F4 agent`, `F5 right pane`,
   and `F12 pass next key`.
2. The hub/palette groups global actions by purpose rather than key number:
   **Views**, **Compose**, **Focus**, **Layout**, and **Application**.
3. A right-pane view shows only the local verbs valid in that view. A mode that
   changes the meaning of letters (selection, find, ask composer, pad editor)
   must lead with its exit key and its two or three most important actions.
4. Every command row states its focus consequence: `opens`, `opens and focuses`,
   or `keeps typing with agent`. This is more valuable than repeating the
   command's implementation detail.

The exhaustive reference can remain available from `F1, ?`; it should not be
the first thing a new user has to parse.

## What not to do

- Do not move every command to F-keys. The available keys are already
  exhausted, `F10`/`F11` are unreliable, and numbers do not communicate intent.
- Do not add a Ctrl leader. Ctrl-letter combinations belong to hosted agents,
  and Ctrl+Alt is AltGr on Windows.
- Do not use `Alt+Space`, `Alt+Enter`, modified F-keys, or desktop-reserved
  shortcuts as an escape hatch; they are not consistently delivered to a
  terminal application.
- Do not make the mapping agent-specific. A key that changes meaning when the
  active child changes is harder to learn than a two-key command sequence.

## Acceptance checks for the implementation

The command hub needs the existing collision audit extended, not replaced.

- Verify `F1` and `F12` reach abeam on Windows Terminal, conhost, and the IDE
  terminal(s) supported by the project.
- On US, UK, and a continental European layout, verify `F1, S` opens the shell;
  `F1, G/E/B/W/P/A` work; and real AltGr characters still type normally in the
  pad, queue, and ask composer.
- Verify that left `Alt+S`, right `AltGr+S`, and `Alt+Shift+S` do not activate
  a retired application command; the focused child receives them according to
  the terminal and keyboard layout.
- Re-run the Claude, Copilot, and Codex audit for every remaining direct key.
  A hub's second keystroke is safe only because `F1` has already deliberately
  entered application command mode.
- Test a live shell, a focused agent, selection mode, multiple agents, and a
  hidden right pane. In each state the hub must state where focus will go and
  `Esc` must cancel without sending its next key to a child.

## Suggested decision

Option A is adopted as a clean cutover in this release. It stops treating the
function row as a finite list of arbitrary commands and makes core workflows
independent of Windows Alt/AltGr delivery, without maintaining a second map.
