//! The scratch pad: the one pane in abeam whose contents nobody but the user
//! wrote.
//!
//! Every other right-hand view reports something — a repository, a file tree, a
//! queue, a second agent. This one holds the sentence you had while the agent
//! was mid-task, and it holds it in markdown because that is the shape a note
//! about code takes on its own. [`crate::editor`] is the text, the caret in it,
//! undo, and the single layout that both the drawing and the cursor are read
//! out of — all of it the pad's own until the files view needed the same
//! things — and [`store`] is where the text goes between sessions. This file is
//! the pane around them: the two forms, the keys, the pad's [`PAD`] policy, and
//! when the text is written.
//!
//! ## Two forms, and two different toggles
//!
//! It opens in **edit**, showing the markdown source with a caret in it. The
//! file reader opens rendered because its job is reading; the pad's job is
//! writing, and a pad that opened on a page you cannot type into would need a
//! keystroke before it could be used for the thing it exists for.
//!
//! `Alt+T` turns it over, in both directions, and in edit mode it is the only
//! key that does — because in edit mode `t` is a letter somebody is trying to
//! type, along with `q`, `j`, `g` and every other key the read-only views spend
//! on navigation. `crate::keys::global` declines `Alt+T`, so the chord arrives
//! here untouched, and the agent is not listening while the pad has focus: that
//! is the interception exemption stated once at the top of `crate::keys` for
//! every pane-local key to point at rather than re-derive.
//!
//! What it does **not** get to decide for itself is what `Alt` means. That is
//! `crate::keys::alt_chord`, and this pane is the reason it is a function: it
//! used to ask `alt && !ctrl` and so turned over from the left `Alt` key and
//! not the right one, on every layout where the right one is AltGr — which
//! Windows spells as Ctrl+Alt. Every global binding worked from both keys the
//! whole time, because `crate::keys::global` never looked at CONTROL. One
//! pane's private answer to a question the rest of the program had already
//! answered is the whole of that bug, and `crate::keys`'s module doc is where
//! the argument now lives.
//!
//! In the **rendering**, bare `t` is an alias for the same toggle. The two
//! forms differ on purpose rather than by oversight: nothing in the rendering
//! can be typed, so the plain key is free there, and `t` is the key
//! `ViewerPane::toggle_raw` already taught for exactly this question — the
//! rendering, and what was typed. Reaching for a chord in the one form where
//! the letter costs nothing would be teaching a second thing for no gain.
//!
//! **The rendering is read-only and shows no cursor**, which is one decision
//! and not two. `viewer.rs`'s `toggle_raw` carries the argument: rendered rows
//! and source rows share nothing at all — rendering drops fence markers,
//! reflows prose and turns a table into a grid — so there is no cell that is
//! honestly "where the caret is" on that page. A caret drawn anyway would be
//! pointing at a character the user cannot edit and cannot even reliably find
//! in the source. So [`Pane::takes_input`] is false there and true in edit
//! mode, which is the question about *this instant* that `crate::pane` is
//! emphatic about, and [`Pane::cursor`] answers `None`.
//!
//! `Esc` is declined in both forms, and `q` as well in the rendering, so
//! `crate::app`'s existing rule hands focus back to the agent. There is no
//! third state — no filter box, no confirmation — so [`Pane::exit_hint`] is the
//! default `esc→agent` in every state this pane can be in, including the ones
//! it passes through, which is what that method's documentation asks of it.
//! [`Pane::action_hint`] keeps the way between the two forms beside it while
//! the pad has focus. That condition matters: with focus on the agent, `Alt+T`
//! belongs to the agent and advertising it over the pad would be a lie.
//!
//! ## Undo, redo, and saving now
//!
//! `Ctrl+Z` and `Ctrl+Y` undo and redo in the edit form — `Ctrl+Shift+Z` too,
//! where the terminal reports the Shift — and `Ctrl+S` saves now in either. A
//! Ctrl chord pressed into a focused right pane is that pane's to answer;
//! `crate::keys::ctrl_chord` states the rule once. The pad answers these three
//! and declines the rest, which therefore do nothing at all — the
//! `(in the pad, editing)` row in `F1, ?` says so out loud about
//! `Ctrl+D`/`Ctrl+U`, the two a reader who has learned the read-only panes is
//! most likely to try.
//!
//! Undo and redo are the editor's keys, with everything else it types: what
//! one undo takes back is `crate::editor`'s answer — a word and what followed
//! it, a run of backspaces or of deletes, or a whole paste, with the caret put
//! back where it was. To the autosave an undo is an edit like any other, with
//! one difference the editor's saved point makes possible: an undo that lands
//! back on what was last written leaves nothing owed, so nothing is written.
//!
//! `Ctrl+S` is the autosave without the two seconds. It is in both forms
//! because saving is not typing, and somebody who turned the pad over to read
//! what they wrote should not have to turn it back to keep it. It also ends the
//! undo step being typed, saved or not: the key says "this is a point I might
//! come back to". Undo is in the edit form only: the rendering is read-only,
//! and a change made there would happen to a text with no caret on screen to
//! say where.
//!
//! ## Drawing
//!
//! The edit form is the editor's layout, drawn as it comes through the editor's
//! `View`: one wrap table that the rows, the cursor and a click are all read
//! from, mended a line at a time as the text changes and coloured only as far
//! as the screen. `crate::editor`'s module doc has the invariants that table
//! keeps — a click lands on the row it was made on, a click against a table a
//! keystroke has invalidated is declined — and what an edit costs. What this
//! file adds is what stands between the pane's rect and that table: the notices
//! drawn above the text, which a click steps over by the number the same frame
//! drew them with, and the column `scroll::bar_width` keeps back. The source is
//! coloured as markdown; the guard that draws a line plain rather than let its
//! colours slide off its words is the editor's `faithful`.
//!
//! The rendering is a cache of its own here, because rendered rows and source
//! rows share nothing — see [`For`].
//!
//! ## When the file is read, and when it is written
//!
//! The pad is read on the first frame it is drawn, and on the first key if one
//! somehow arrives before a frame. `new` touches no disk, which is the rule the
//! shell and the ask panes already follow and which `ViewerPane::render`
//! explains: being drawn is the only signal a pane gets that it is on screen,
//! so it is the only honest place for work a session that never presses the key
//! should not pay for. The keystroke door is the same read, taken at whichever
//! comes first, and it exists so that a load can never land on top of something
//! already typed.
//!
//! Writing is a dirty flag and a two-second debounce: [`Pane::tick`] saves when
//! the text has changed and nothing has been typed for [`QUIET`], and
//! [`PadPane::flush`] saves at once, which is what the shell calls on quit and
//! when the pad leaves the screen. `Ctrl+S` is the same save asked for by the
//! user rather than the shell. A save that works tells the editor so, and from
//! then on "has the text changed" is the editor's `is_modified` — so an undo
//! back to what was written owes the file nothing.
//!
//! **The write is synchronous, on the tick thread, and that needs saying rather
//! than assuming**, because `crate::pane::Pane::tick` says in as many words
//! that it must not block. What makes it acceptable is the size: at most
//! [`MAX_BYTES`] — 64 KiB — to a path in the user's own profile, at
//! most once every two seconds, through a `write` and a `rename`. The
//! alternative is a worker thread, a channel, a shutdown path and a way for a
//! save that failed on the other side to become a notice on this one, which is
//! four moving parts to spare a millisecond that only exists at all when
//! somebody has just stopped typing. The residual is real and is this: a
//! machine that loses power between the last keystroke and the next save loses
//! up to two seconds of typing. A pad written on every keystroke would cost a
//! file rename per character instead, which is the trade this direction is the
//! cheap side of.
//!
//! A save that fails is on screen rather than swallowed — `store::save_at`'s `Err`
//! is already a sentence for the person who typed the text — and a load that
//! came back truncated stops saving for the whole session, for the reason
//! `store::Loaded::truncated` gives: writing a truncated pad back over the file
//! it was read from deletes the rest of somebody's notes, and looks from the
//! outside exactly like an ordinary autosave.

mod store;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};

use crate::editor::{Editor, Look, Outcome, Policy, Tabs, View};
use crate::pane::{Handled, Pane};
use crate::panes::viewer::source::Grammar;
use crate::panes::viewer::{markdown, source, theme};
use crate::scroll;
use crate::text::{block, err};

/// The most the pad will hold. An insert or a paste that would take it past
/// this is refused rather than trimmed to fit.
///
/// The same number as `source::HIGHLIGHT_MAX_BYTES`, and deliberately the same
/// rather than coincidentally: past that size the highlighter gives up and
/// returns plain text, so a pad allowed to grow beyond it would go grey one
/// keystroke after it was fine, at a size nobody chose and with nothing on
/// screen saying why. What can be typed and what can be drawn in colour are
/// one decision, and the assertion under this is the line that keeps them one:
/// a drift between the two is a compile error rather than a colour that
/// quietly stops happening.
///
/// It was the buffer's own constant while the buffer was the pad's. It is the
/// pad's policy now, handed to the shared editor in [`PAD`]; the files view
/// hands it the reader's 512 KiB instead and has to say when a file is too big
/// to colour, which by this line the pad never does.
const MAX_BYTES: usize = 64 * 1024;

const _: () = assert!(MAX_BYTES == source::HIGHLIGHT_MAX_BYTES);

/// What the pad tells the editor about its text: the cap above, that a tab is
/// two spaces — typed, pasted or loaded — and that its file is LF with no byte
/// order mark, so the cap counted on disk is the cap counted in memory.
///
/// Two rather than four because the pad holds markdown, where two spaces is a
/// nesting level and four is a code block. Spaces rather than a literal tab
/// because a note has no use for one and the pad has never held one: a tab
/// pasted out of a code block becomes the same two spaces the key types. That
/// used to be argued from the caret — a tab drawn four cells wide beside a
/// column counted in characters — and the editor now measures in cells and
/// draws a tab at its stop, so it is a choice about markdown rather than a
/// limit of the layout.
const PAD: Policy = Policy {
    max_bytes: MAX_BYTES,
    tabs: Tabs::Spaces(2),
    line_ending_bytes: 1,
    bom_bytes: 0,
};

/// How long the pad must be left alone before it is written.
///
/// Long enough that a sentence is one save rather than forty, short enough that
/// what is lost to a crash is a phrase and not a paragraph. It is also the
/// number in the residual the module doc discloses, so the two have to be the
/// same number and this is it.
const QUIET: Duration = Duration::from_secs(2);

/// What an empty pad says about itself.
///
/// The ask pane's opening screen makes the general argument — an empty box is
/// indistinguishable from a broken one, and this pane is empty every time
/// somebody opens it for the first time — and there is a second one here. This
/// is the only place `Alt+T` is discoverable from inside the pane: the border
/// has room for the way out and nothing else, and a rendering nobody knows
/// about is a feature that does not exist.
const OPENING: &str = "This is the scratch pad. Write here while the agent is \
                       working; what you type is kept for this workspace and is \
                       waiting the next time you open it here.\n\
                       \n\
                       alt+t shows it rendered as markdown, and brings you \
                       back. In the rendering, t on its own does the same, \
                       because nothing there is being typed into.\n\
                       \n\
                       esc hands focus back to the agent.";

/// Which of the two forms of the same text is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    /// The markdown source, with a caret in it.
    Edit,
    /// What that source renders to, read-only.
    Rendered,
}

impl Form {
    fn turned(self) -> Self {
        match self {
            Form::Edit => Form::Rendered,
            Form::Rendered => Form::Edit,
        }
    }
}

/// What a turn from the source to the rendering owes the next frame: the same
/// fraction of the document on screen.
///
/// `ViewerPane::toggle_raw`'s answer to the same problem: the two layouts share
/// no rows, so the nearest honest thing to "where I was" is how far down I was.
/// It depends on how the rendering wrapped, and nothing outside `render` knows
/// that — the pane is not told its width until it is drawn — so the key that
/// asked records what it wanted and the frame delivers it. The other direction,
/// back to the source, brings the caret into view instead, which is the
/// editor's `View::follow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fraction {
    was: usize,
    before: usize,
}

/// What a cached rendering was built from.
///
/// All three, because a rendering built for any other value of any of them is
/// a rendering of something else: the width decides where lines break, `rev`
/// says whether the text is the text that was rendered, and the theme decides
/// the colours. The edit form is not cached here — the editor keeps its own
/// layout and mends it per line — and the rendering is not mended at all: a
/// markdown rendering reflows whole paragraphs and tables, so an edit's reach
/// in it is not a line, and the form it is drawn in is one nobody is typing
/// into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct For {
    width: usize,
    rev: u64,
    mode: theme::Mode,
}

/// The rows the last rendering produced. No wrap table beside them: rendered
/// rows correspond to nothing in the source, so there is no caret to place in
/// them and no click to answer.
struct Rendered {
    key: For,
    rows: Vec<Line<'static>>,
}

/// A per-workspace markdown scratch pad.
pub struct PadPane {
    /// The file this pad is written to, worked out once when the pane is
    /// built, or `None` when this machine will not say where the user's
    /// profile is.
    ///
    /// **A path rather than the workspace root it came from, derived once
    /// rather than at each of the three places that want it.** That is not a
    /// saving and this is not a cache: a path derived twice is a path that can
    /// be derived differently, and `crate::paths` is a whole module written
    /// around the one sentence that a rule for writing a directory down may not
    /// disagree with itself. What it closes here is a pad read from one file
    /// and written back to another.
    ///
    /// It is never inside the workspace, which `store`'s module doc argues at
    /// length: a pad under the repository is an untracked file in the git pane
    /// next door and a `*.md` the reader would follow the moment it was typed
    /// into.
    ///
    /// Working it out in [`PadPane::new`] does not break the promise the module
    /// doc makes about a session that never presses the key. `store::path_for`
    /// reads two environment variables and does string arithmetic on them —
    /// `Path::is_absolute` asks this platform's parser and not its filesystem —
    /// so there is no disk anywhere in it, and the first frame is still the
    /// first time this pane opens a file.
    ///
    /// A `None` is therefore in hand at construction rather than at the first
    /// failed save, which is where it used to surface: the shape before this
    /// one discovered that there was nowhere to write only once somebody had
    /// filled the pad with sentences that were already going nowhere. Saying so
    /// on the opening screen is a larger change than the one that put this
    /// field here and is not made yet — but the fact is now in hand for
    /// whoever makes it.
    path: Option<PathBuf>,
    mode: theme::Mode,
    form: Form,
    /// The text, the caret, its undo, and the edit form's layout.
    text: Editor,
    /// Where the pad is scrolled to, in either form, and whether the next
    /// frame owes the caret a place on screen.
    view: View,
    /// The rect the last frame was given, so `cursor` and a click are answered
    /// against the layout that is actually on screen.
    drawn: Rect,
    /// How many rows of that rect the notices took. Written by `render` and
    /// read by a click, which is the same one-calculation rule the wrap is
    /// under: what was drawn and what a click is measured against are one
    /// number or they are a bug.
    noticed: u16,
    /// Whether the file has been consulted yet.
    read: bool,
    /// The file was longer than the buffer's cap, so this session must never
    /// save. See `store::Loaded::truncated`.
    truncated: bool,
    /// The file is there and abeam could not read it, so this session must
    /// never save either. See `store::Loaded::unreadable`, which carries the
    /// probe: an empty pad written over notes abeam never saw is the worst
    /// thing this pane can do, and it is one autosave away whenever a scanner
    /// or a sync client has the file open for a moment.
    unreadable: bool,
    /// What the file looked like when this pane last read or wrote it.
    ///
    /// Handed to every save so that a second abeam window on the same workspace
    /// is noticed rather than silently overwritten — see `store::Stamp`, which
    /// is also where the rule lives that a stamp nobody could take never stops
    /// a save.
    stamp: store::Stamp,
    /// When the text last changed, and by being `Some` at all, that it differs
    /// from what is on disk. One field rather than a flag and a clock, because
    /// two of them are two things to forget in different places.
    changed: Option<Instant>,
    /// The last save failure, as the sentence `store::save` wrote.
    failed: Option<String>,
    /// Something typed or pasted was turned away for want of room. Kept beside
    /// `Buffer::is_full` rather than derived from it, because that method's own
    /// documentation says the two questions differ: a paste can be refused with
    /// room still spare, when what is left is smaller than what arrived.
    refused: bool,
    /// Bumped on every change to the text, so the cached rendering can tell
    /// whether it was built from this text or the one before it.
    ///
    /// The pane's own count, as it always was, rather than the one the editor
    /// keeps for its layout. The rendering is this pane's cache, and keying it
    /// by the editor's number would make it right only for as long as the
    /// editor never starts counting again — which it does whenever it is
    /// replaced, as `ensure_read` and several tests here replace it.
    rev: u64,
    fraction: Option<Fraction>,
    rendered: Option<Rendered>,
    /// Where the cursor goes, worked out by the frame that drew the row it sits
    /// on. `cursor` takes `&self` and cannot lay anything out, and a second
    /// calculation there is the drift this whole design exists to prevent.
    caret: Option<(u16, u16)>,
}

impl PadPane {
    /// A pad for this workspace, which has not been read yet.
    ///
    /// No disk at all, deliberately: a session that never presses the key never
    /// pays for the read, and the first frame is where it happens instead. See
    /// the module doc. Working out *which* file it would read costs nothing
    /// against that promise and happens here — see [`path`](Self::path) for
    /// both halves of why.
    pub fn new(root: PathBuf, theme: crate::config::Theme) -> Self {
        Self {
            path: store::path_for(&root),
            mode: mode_of(theme),
            form: Form::Edit,
            text: Editor::new(PAD),
            view: View::default(),
            drawn: Rect::ZERO,
            noticed: 0,
            read: false,
            truncated: false,
            unreadable: false,
            stamp: store::Stamp::default(),
            changed: None,
            failed: None,
            refused: false,
            rev: 0,
            fraction: None,
            rendered: None,
            caret: None,
        }
    }

    /// Point this pad at a file, for a test that must not write into the
    /// profile of whoever is running the suite.
    ///
    /// [`new`](Self::new) derives the path from the workspace root and lands in
    /// the user's own profile, which is right in a session and wrong in a test:
    /// a unit test able to write there is a test that has escaped its fixture.
    /// The shell's four flush paths — `set_right_view` when the pad leaves the
    /// screen, `set_focus` when the keys go back to the agent, `sync_workspaces`
    /// when git stops naming a worktree, and `flush_pads` on the way out — can
    /// only be exercised through a real pane, so this is how `crate::app`'s
    /// tests give one a file inside a `TempDir` they own.
    #[cfg(test)]
    pub(crate) fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    pub fn set_theme(&mut self, theme: crate::config::Theme) {
        self.mode = mode_of(theme);
    }

    /// Write the pad now, if there is anything to write.
    ///
    /// The shell calls this at the four moments the debounce cannot cover: the
    /// view changing, the keys going back to the agent, git forgetting a
    /// worktree whose workspace is about to be dropped, and the way out. Every
    /// one of them is the same shape — the pad is about to stop being looked
    /// at, or about to stop existing, and the next thing the user does is
    /// somewhere else entirely.
    pub fn flush(&mut self) {
        if self.changed.is_some() {
            self.store();
        }
    }

    /// Take the pad off disk, once.
    ///
    /// Called from `render` and from the two paths that can change the text.
    /// Being drawn is the ordinary signal and the one `ViewerPane` uses; the
    /// other two are here so that a keystroke arriving before the first frame
    /// cannot be overwritten by a file read afterwards, which would be a pane
    /// that quietly ate the first thing somebody typed into it.
    fn ensure_read(&mut self) {
        if self.read {
            return;
        }
        self.read = true;
        let loaded = self.path.as_deref().map(store::load_at).unwrap_or_default();
        self.unreadable = loaded.unreadable;
        self.stamp = loaded.stamp;
        self.text = Editor::from_text(PAD, &loaded.text);
        // Either cap, because there are two of them and they are set
        // independently: `store` stops the read at `MAX_BYTES` and the buffer
        // refuses to hold more than that. They agree today, and the day
        // one of them moves is the day this pane would otherwise start saving a
        // pad it had only seen the front of.
        self.truncated = loaded.truncated || self.text.truncated();
        self.rev += 1;
        // The end, which is where the next note goes, so the pad opens showing
        // the end of what is already there. `from_text` leaves the caret at the
        // start, because where it goes is a question about what the text is
        // for, and this is the pad's answer.
        self.text.set_caret(usize::MAX, usize::MAX);
        self.view.follow();
    }

    /// Write the pad, and say whether anything on screen changed by it.
    ///
    /// Almost always nothing does — a save is invisible, which is the whole
    /// point of an autosave — so this returns true only when the notice it
    /// draws has appeared or gone. `crate::pane::Pane::tick` is explicit about
    /// what a needless true costs: a re-render of the agent's whole screen.
    fn store(&mut self) -> bool {
        // The three states in which this pane must not write, all of them
        // settled before this is called and all of them already on screen:
        // `notices` is standing over the text saying so. Returning here rather
        // than attempting and failing is the difference between refusing to
        // touch a file and being lucky about it.
        if self.truncated || self.unreadable {
            return false;
        }
        let Some(path) = self.path.clone() else {
            return false;
        };

        let now = match store::save_at(&path, &self.text.text(), self.stamp) {
            Ok(stamp) => {
                self.stamp = stamp;
                self.text.mark_saved();
                // Cleared here and nowhere else, because this field is both the
                // dirty flag and the retry: `tick` and `flush` each ask it
                // whether anything is owed, so clearing it up front turned one
                // unlucky moment — a network blip on a redirected profile, a
                // scanner holding the file — into a paragraph that was never
                // written and never would be. Left set, a failure costs one
                // attempt per quiet interval until it works.
                self.changed = None;
                None
            }
            Err(why) => Some(why),
        };
        let differs = now != self.failed;
        self.failed = now;
        differs
    }

    /// What the editor did with a key, a paste or a click, answered for the
    /// pad.
    ///
    /// An edit is a change the file is owed — unless it landed back on the
    /// text last written, which the editor's saved point says in one
    /// comparison, and then nothing is owed and a failure notice from before is
    /// no longer true. Either way a refusal notice standing from before is
    /// taken down, since the text it was about is not the text any more, and
    /// the caret is brought into view. A refusal puts its notice up. A move
    /// brings the caret into view and changes nothing else.
    ///
    /// What the shell is told is [`Outcome::handled`], whose variants carry the
    /// two arguments this used to make for itself: a refused letter is still
    /// claimed, because a declined `q` would move the writer into the agent's
    /// prompt at the moment the pad is full; and a key that did nothing is
    /// declined, because a frame for it re-renders the agent's whole screen.
    fn after(&mut self, outcome: Outcome) -> Handled {
        match outcome {
            Outcome::Edited => {
                self.rev += 1;
                if self.text.is_modified() {
                    self.changed = Some(Instant::now());
                } else {
                    self.changed = None;
                    self.failed = None;
                }
                self.refused = false;
                self.view.follow();
            }
            Outcome::Refused => self.refused = true,
            Outcome::Moved => self.view.follow(),
            Outcome::Still => {}
        }
        outcome.handled()
    }

    /// Turn the pad over.
    fn turn(&mut self) -> Handled {
        match self.form {
            Form::Edit => {
                self.fraction = Some(Fraction {
                    was: self.view.scroll.offset,
                    before: self.view.scroll.max(),
                });
            }
            // Coming back to the source, the caret is where the reader was
            // before they looked at the rendering, and it is about to be drawn
            // again — so it is both the honest anchor and one that has to be on
            // screen anyway. A fraction still owed from a turn the other way
            // that no frame has delivered is forgotten, because it was about a
            // page that is no longer the one on screen.
            Form::Rendered => {
                self.fraction = None;
                self.view.follow();
            }
        }
        self.form = self.form.turned();
        Handled::Yes
    }

    /// Edit mode, where every printable key is text.
    ///
    /// `q`, `j`, `g` and `t` included, which is the trade any type-into-a-pane
    /// makes and the reason `takes_input` answers true here. Nothing typed here
    /// goes near `crate::scroll::Scroll::key` unless the editor has handed it
    /// back, and that is the single most important line in the file: that
    /// vocabulary claims `j`, `k`, `g`, `G`, `b` and space, every one of which
    /// is a letter somebody is in the middle of typing, and a pad that routed
    /// keys through it would swallow a word and scroll instead.
    fn edit_key(&mut self, key: KeyEvent) -> Handled {
        // Before the editor, though it would hand the chord back anyway: this
        // is the key the whole pane hangs off and it should be the first thing
        // read here.
        if matches!(key.code, KeyCode::Char('t' | 'T')) && crate::keys::alt_chord(&key) {
            return self.turn();
        }
        if let Some(outcome) = self.text.key(&key) {
            return self.after(outcome);
        }
        match key.code {
            KeyCode::Char('s' | 'S') if crate::keys::ctrl_chord(&key) => self.save_now(),
            // The two keys in the scroll vocabulary that are not also letters,
            // so they can go on meaning what they mean everywhere else without
            // taking a character away from anybody. They page the view and
            // leave the caret, which is what `View::glance` is.
            KeyCode::PageUp | KeyCode::PageDown => self.view.glance(key),
            // Declined rather than claimed, which is what puts the user back at
            // the agent through `crate::app`'s rule. There is nothing here for
            // `Esc` to close first — no filter box, no draft that is not
            // already the document — so it means the one thing it means.
            _ => Handled::No,
        }
    }

    /// The rendering, which is a read-only view like any other.
    fn rendered_key(&mut self, key: KeyEvent) -> Handled {
        if let Some(handled) = self.view.scroll.key(key) {
            return handled;
        }
        match key.code {
            // Saving is not typing, so it is here too; the module doc says why
            // undo is not.
            KeyCode::Char('s' | 'S') if crate::keys::ctrl_chord(&key) => self.save_now(),
            // Bare `t` and `Alt+T` both, and the module doc says why the two
            // forms differ about this. A bare Ctrl chord is excluded, and not
            // because it belongs to something hosted — the pad hosts nothing,
            // which is the reason it may take `Ctrl+S` above. `Ctrl+T` is
            // simply neither the letter nor the chord this pane turns over on,
            // and a toggle reachable by a third spelling is one more thing
            // `F1, ?` would have to say. Ctrl *with* Alt is AltGr rather than a
            // chord, which is why this is two questions and not `!ctrl`.
            KeyCode::Char('t' | 'T')
                if crate::keys::is_text(&key) || crate::keys::alt_chord(&key) =>
            {
                self.turn()
            }
            // `Esc` and `q` fall through so the shell hands focus back, which
            // is what they do in every other read-only view.
            _ => Handled::No,
        }
    }

    /// `Ctrl+S`: the save the debounce would make, made now, and nothing when
    /// nothing is owed.
    ///
    /// Through [`store`](Self::store), so it refuses exactly where the
    /// autosave does — a truncated or unreadable pad, or one with nowhere to
    /// go, each of which already has its notice standing — and a failure is the
    /// same sentence on screen. It answers as `tick` does: a frame only when the
    /// notice appeared or went, because a save that worked changes nothing
    /// anybody can see.
    ///
    /// The undo step being typed ends here whether or not there is anything to
    /// write, for the reason the module doc gives.
    fn save_now(&mut self) -> Handled {
        self.text.end_step();
        if self.changed.is_none() {
            return Handled::No;
        }
        self.store().into()
    }

    /// Bring the rows up to date for this width: the editor's own mended
    /// layout in the edit form, and a rendering built from nothing unless the
    /// one in hand was built from exactly this text at exactly this width.
    fn ensure_layout(&mut self, width: usize) {
        match self.form {
            Form::Edit => {
                let look = Look {
                    mode: self.mode,
                    grammar: Grammar::for_code("markdown"),
                };
                self.text.lay_out(width, look);
            }
            Form::Rendered => {
                let key = For {
                    width,
                    rev: self.rev,
                    mode: self.mode,
                };
                if self.rendered.as_ref().is_some_and(|r| r.key == key) {
                    return;
                }
                self.rendered = Some(Rendered {
                    key,
                    rows: markdown::render(&self.text.text(), width, self.mode),
                });
            }
        }
    }

    /// What the pane has to say about itself before the text, in the order a
    /// reader needs it.
    ///
    /// Above the text rather than below it, which is `QueuePane::render`'s
    /// argument one pane along: a pane that cannot do the thing it is for has
    /// to say so where somebody will read it, and the bottom of a document
    /// somebody has scrolled is not that place.
    fn notices(&self, width: usize) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        // First, because it is the only one of these that is true before
        // anybody has typed anything, and telling somebody afterwards that
        // their words were never being kept is telling them too late.
        if self.path.is_none() {
            out.extend(block(&store::nowhere(), width, err()));
        }
        if self.truncated {
            out.extend(block(&self.too_long(), width, err()));
        }
        if self.unreadable {
            out.extend(block(&self.unread(), width, err()));
        }
        if let Some(why) = &self.failed {
            out.extend(block(why, width, err()));
        }
        // One or the other, never both: they are two readings of the same
        // refusal, and the second is only reachable when the first is not true.
        if self.text.is_full() {
            out.extend(block(&full(), width, err()));
        } else if self.refused {
            out.extend(block(&would_not_fit(), width, err()));
        }
        out
    }

    /// The notice a truncated load leaves standing for the whole session.
    ///
    /// It names the file, because the remedy is outside abeam: the user has to
    /// go and shorten or move that file, and a message that would not say which
    /// one leaves them looking for it.
    fn too_long(&self) -> String {
        format!(
            "abeam could not fit {} in the pad: this is the first {} KiB of \
             what is in it and there is more. Nothing will be saved this \
             session, because writing what is here would delete the rest of \
             that file.",
            self.names(),
            MAX_BYTES / 1024
        )
    }

    /// The notice a load that could not open the file leaves standing.
    ///
    /// It names another program, because that is what this nearly always is —
    /// a scanner, a sync client, an editor with the file locked — and a message
    /// that said only "could not read" would leave somebody looking at their
    /// own permissions.
    fn unread(&self) -> String {
        format!(
            "{} is there and abeam could not read it, so this pad has opened \
             empty. Nothing will be saved this session, because writing what is \
             here would replace a file abeam has never seen — check whether \
             another program has it open.",
            self.names()
        )
    }

    /// The pad's file for a notice that has to name it, or a phrase for the
    /// pane that has no file at all.
    fn names(&self) -> String {
        self.path
            .as_ref()
            .map_or_else(|| "the pad file".to_string(), |p| p.display().to_string())
    }
}

/// The palette this pane draws in. The same two-line map the viewer and the ask
/// pane each keep, and kept here rather than shared because `crate::config` and
/// `viewer::theme` are deliberately separate types: one is what a user wrote in
/// a file, the other is what a renderer switches on.
fn mode_of(theme: crate::config::Theme) -> theme::Mode {
    match theme {
        crate::config::Theme::Dark => theme::Mode::Dark,
        crate::config::Theme::Light => theme::Mode::Light,
    }
}

/// Said when the pad will take nothing more at all.
///
/// It says nothing about whether what is already here is being saved, and that
/// omission is deliberate: a load cut back to the cap can arrive full, so this
/// notice and the truncated one can be on screen together, and a line here
/// promising a save would then be contradicting the line above it.
fn full() -> String {
    format!(
        "The pad is full at {} KiB and will take nothing more. Move some of \
         what is here somewhere else to make room.",
        MAX_BYTES / 1024
    )
}

/// Said when something typed or pasted was turned away with room still spare.
///
/// The other half of `Buffer::is_full`'s caveat: that method reports the pad's
/// state and not the last refusal, and the two part company whenever what
/// arrived was bigger than what was left. So this has to cover a single
/// keystroke as well as a paste — a pad three bytes short of the cap turns away
/// the next `é` — and it says "what will not fit" rather than naming the
/// clipboard, which is what it used to do and was wrong about whenever a
/// character rather than a paste had been refused.
///
/// Whole rather than trimmed, because half a pasted paragraph is worse than
/// none of it: the reader has to notice the cut, and the place it happened is
/// off the bottom of a pane they had already stopped looking at.
fn would_not_fit() -> String {
    format!(
        "That would not fit. The pad holds {} KiB, and what will not fit is \
         turned away whole rather than trimmed to the room left, so nothing \
         was added.",
        MAX_BYTES / 1024
    )
}

impl Pane for PadPane {
    /// The file reader's convention, and the same word it uses: which of the
    /// two forms of one document is on screen. The shell draws `exit_hint`
    /// ahead of this and owns the border, so these are the words only.
    fn title(&self) -> String {
        match self.form {
            Form::Edit => "pad".to_string(),
            Form::Rendered => "pad · rendered".to_string(),
        }
    }

    /// The route to the other form, shown by the shell only while this pane has
    /// the keys. In the edit form the chord is necessary because bare `t` is
    /// text; in the rendering it is free and is the same toggle the file reader
    /// already teaches.
    fn action_hint(&self) -> Option<&'static str> {
        Some(match self.form {
            Form::Edit => "alt+t→rendered",
            Form::Rendered => "t→editing",
        })
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        self.drawn = inner;
        self.caret = None;
        self.noticed = 0;
        if inner.width == 0 || inner.height == 0 {
            // Nothing is measured on the way out. `Scroll::measure` clamps the
            // offset to what it has just been told will fit, so measuring a
            // viewport of zero here scrolls the pad to the top: drag a terminal
            // down to two rows and back and a reader half way through a long
            // pad is at the beginning of it, with no key having been pressed.
            // The last real numbers are the ones to keep, which is what
            // `crate::scroll::Scroll` says about the gap between frames
            // generally.
            return;
        }
        // Being drawn is the signal that this pane is on screen, and the only
        // one it gets. See the module doc, and `ViewerPane::render`.
        self.ensure_read();

        // The page, before anything is written on it. The same fill the file
        // reader paints and for its reason: the syntax colours below are
        // chosen against this background, and F3 is worth having only if the
        // page turns with them.
        f.render_widget(Block::new().style(self.mode.theme().base()), inner);

        let mut area = inner;
        let notices = self.notices(area.width as usize);
        // At least one row is kept for the text. A pane one row tall is not a
        // pad anybody can use, and a notice that displaced the whole document
        // to say so would be the pane arguing with itself.
        let rows = u16::try_from(notices.len())
            .unwrap_or(u16::MAX)
            .min(area.height.saturating_sub(1));
        if rows > 0 {
            f.render_widget(
                Paragraph::new(notices),
                Rect {
                    height: rows,
                    ..area
                },
            );
            self.noticed = rows;
            area.y += rows;
            area.height -= rows;
        }

        // The scrollbar takes a column from the text rather than sitting on top
        // of it, and it is reserved whether or not it is drawn — see
        // `scroll::bar_width`, which explains why deciding per frame would
        // re-wrap the whole document the moment it crossed the pane height.
        let width = area.width.saturating_sub(scroll::bar_width(area.width)) as usize;
        if width == 0 {
            // Unreachable while `bar_width` will not claim the only column
            // there is — it wants twenty-four before it takes one — so this
            // answers a question that cannot currently be asked, and is kept
            // because that threshold is not this file's to promise. Not
            // measured, for the reason above.
            return;
        }
        self.ensure_layout(width);
        let text = Rect {
            width: width as u16,
            ..area
        };
        let height = area.height as usize;
        let visible: Vec<Line<'static>> = match self.form {
            // Measured, brought into view if a key asked, and coloured as far as
            // the screen: the editor's `View`.
            Form::Edit => self
                .view
                .frame(&mut self.text, height)
                .into_iter()
                .map(|row| row.text)
                .collect(),
            Form::Rendered => {
                self.view.hide_cursor();
                let rows = self.rendered.as_ref().map_or(&[][..], |r| &r.rows[..]);
                self.view.scroll.measure(rows.len(), height);
                if let Some(Fraction { was, before }) = self.fraction.take() {
                    // A form that fitted the pane whole has no fraction to
                    // keep, and the top is the only place it can have been.
                    let to = (was * self.view.scroll.max()).checked_div(before).unwrap_or(0);
                    self.view.scroll.to(to);
                }
                rows.iter()
                    .skip(self.view.scroll.offset)
                    .take(height)
                    .cloned()
                    .collect()
            }
        };
        f.render_widget(Paragraph::new(visible), text);
        self.view.scroll.render_bar(f, area);

        if self.text.is_empty() {
            // Under the first row in the edit form, because that row is the one
            // empty line the caret is sitting on and a hint drawn over it would
            // put the cursor inside the pane's own words. In the rendering
            // there is no caret and nothing else on the page at all, so it
            // starts at the top.
            let top = u16::from(self.form == Form::Edit);
            if area.height > top {
                let hint = block(OPENING, width, self.mode.theme().dim());
                f.render_widget(
                    Paragraph::new(hint),
                    Rect {
                        y: area.y + top,
                        height: area.height - top,
                        ..text
                    },
                );
            }
        }

        // In pane coordinates: the view's row, below the notices.
        if let Some((col, row)) = self.view.cursor() {
            self.caret = Some((col as u16, (area.y - inner.y) + row as u16));
        }
    }

    /// Saves a pad that has been left alone for [`QUIET`], and nothing else.
    ///
    /// False on almost every call, which is what it has to be: a save changes
    /// nothing on screen, and a true here re-renders the agent's whole screen
    /// to redraw a page that is identical. The one true is a save that failed
    /// and left a notice that was not there before.
    fn tick(&mut self) -> bool {
        let Some(since) = self.changed else {
            return false;
        };
        if since.elapsed() < QUIET {
            return false;
        }
        self.store()
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        self.ensure_read();
        Ok(match self.form {
            Form::Edit => self.edit_key(key),
            Form::Rendered => self.rendered_key(key),
        })
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        // The wheel first, and in both forms: it is the one gesture that means
        // the same thing on a page you are writing and a page you are reading.
        if let Some(handled) = self.view.wheel(ev) {
            return Ok(handled);
        }
        if self.form != Form::Edit || !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
            return Ok(Handled::No);
        }
        // `ev.row` is pane-relative already; the notices are what stand between
        // the pane and the text, and this is the number the same frame drew
        // them with.
        let Some(row) = ev.row.checked_sub(self.noticed) else {
            return Ok(Handled::No);
        };
        // From there the editor's to answer, through the view that drew the
        // rows — including the two clicks it declines rather than answers
        // wrongly: one against a table the last keystroke has invalidated, and
        // one on the column `scroll::bar_width` kept back. `Editor::click`
        // carries both arguments. A declined click becomes a selection, which
        // is what `crate::app` does with everything a pane turns down, and the
        // frame the keystroke has already asked for makes the next one right.
        match self
            .view
            .click(&mut self.text, row as usize, ev.column as usize)
        {
            Some(true) => Ok(self.after(Outcome::Moved)),
            Some(false) | None => Ok(Handled::No),
        }
    }

    /// The glance path — `Alt+J`, `Alt+K`, `Alt+PgDn`, `Alt+PgUp` — in both
    /// forms.
    ///
    /// Overridden because the default declines for a pane that takes input, and
    /// in the edit form this is the **only** way the pad moves without
    /// dragging the caret along with it: `Down` there is a caret key, as it has
    /// to be, so without this a reader glancing at a pad they are not focused
    /// on would have no way to see the top of it. `crate::pane::Pane::scroll_key`
    /// says what getting this wrong costs elsewhere — "a glance at the command
    /// view silently walking its history" — and the pad's version of that
    /// mistake would be worse: a glance binding that reached `edit_key` would
    /// type into a document.
    fn scroll_key(&mut self, key: KeyEvent) -> Result<Handled> {
        Ok(self.view.glance(key))
    }

    /// True in the edit form and false in the rendering, which is the question
    /// about this instant that `crate::pane::Pane::takes_input` asks.
    fn takes_input(&self) -> bool {
        self.form == Form::Edit
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.caret
    }

    /// A paste is the fastest way to get a real note into a pad — an error, a
    /// stack trace, a paragraph out of the agent's transcript — and it goes in
    /// whole, newlines included, because the buffer already knows what to do
    /// with them.
    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        self.ensure_read();
        if self.form != Form::Edit {
            return Ok(Handled::No);
        }
        // An empty clipboard comes back `Still` rather than `Refused` — see
        // `Editor::paste` — so it puts no notice up saying the pad was too
        // small to hold it.
        let outcome = self.text.paste(text);
        Ok(self.after(outcome))
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// A pad for a workspace nothing else on this machine has ever seen, and
    /// the file it would write to — which is inside the fixture, and that is
    /// the point of this function rather than a convenience of it.
    ///
    /// The path is set here rather than left to [`PadPane::new`] to derive, and
    /// the reason is not that the profile is slow or awkward to reach: it is
    /// that a unit test able to write into the profile of whoever is running
    /// the suite is a test that has escaped its fixture, and the escape should
    /// be impossible rather than merely unused. Most of the tests below never
    /// reach a save at all, but that is a fact about how they happen to be
    /// written today, and the next one added here has no way of knowing it was
    /// relying on it. Everything any pane built by this function writes lands
    /// in the `TempDir`, which goes when the test ends however it ends.
    ///
    /// One helper and not two, so that this module has a single way to build a
    /// pane: a door that cannot be left open is easier to keep shut than two
    /// that have to agree with each other. The tests that never write take the
    /// path and drop it.
    fn pad(dir: &TempDir) -> (PadPane, PathBuf) {
        let path = dir.path().join("scratch.md");
        let mut pad = PadPane::new(dir.path().to_path_buf(), crate::config::Theme::Dark);
        pad.set_path(path.clone());
        (pad, path)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn wheel_down() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Type at the pane, one key at a time, the way somebody at a keyboard
    /// does — rather than reaching past it into the buffer.
    fn type_in(p: &mut PadPane, text: &str) {
        for ch in text.chars() {
            p.handle_key(key(KeyCode::Char(ch))).expect("a keystroke");
        }
    }

    /// Draw one frame into `area` of a terminal with room for it, for the
    /// degenerate rects a real drag passes through on its way down.
    fn draw(p: &mut PadPane, area: Rect) {
        let mut term = Terminal::new(TestBackend::new(40, 12)).expect("a test terminal");
        term.draw(|f| p.render(f, area)).expect("draw the pad");
    }

    /// Draw one frame and flatten it, so a test can ask what is on screen
    /// rather than what the code meant to put there.
    fn screen(p: &mut PadPane, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("a test terminal");
        term.draw(|f| p.render(f, Rect::new(0, 0, w, h)))
            .expect("draw the pad");
        term.backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    // --- what the two forms do with a keyboard ----------------------------

    #[test]
    fn q_and_j_and_g_are_letters_in_the_edit_form_and_keys_in_the_rendering() {
        let dir = TempDir::new("pad-letters");
        let (mut p, _) = pad(&dir);

        // Every one of these is a scroll key in the other four right-hand
        // views, and every one of them is a character here.
        for ch in "qjgGbt ".chars() {
            assert_eq!(
                p.handle_key(key(KeyCode::Char(ch))).unwrap(),
                Handled::Yes,
                "{ch:?} was not claimed, so the shell would act on it"
            );
        }
        assert_eq!(p.text.text(), "qjgGbt ");

        // The same keys in the rendering move the page instead.
        p.handle_paste(&"a line\n".repeat(40)).unwrap();
        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        screen(&mut p, 30, 6);
        p.view.scroll.to(0);

        assert_eq!(p.handle_key(key(KeyCode::Char('j'))).unwrap(), Handled::Yes);
        assert_eq!(p.view.scroll.offset, 1);
        assert_eq!(p.handle_key(key(KeyCode::Char('G'))).unwrap(), Handled::Yes);
        assert_eq!(p.view.scroll.offset, p.view.scroll.max());
        assert_eq!(p.handle_key(key(KeyCode::Char('g'))).unwrap(), Handled::Yes);
        assert_eq!(p.view.scroll.offset, 0);
        // ...and `q` is the shell's again, which is how you leave a read-only
        // view.
        assert_eq!(p.handle_key(key(KeyCode::Char('q'))).unwrap(), Handled::No);
    }

    #[test]
    fn esc_is_declined_in_both_forms_so_the_shell_hands_focus_back() {
        let dir = TempDir::new("pad-esc");
        let (mut p, _) = pad(&dir);
        type_in(&mut p, "a note");

        assert_eq!(p.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
        assert_eq!(p.text.text(), "a note", "esc must not clear the pad");
        assert_eq!(p.exit_hint(), "esc→agent");
        assert_eq!(p.action_hint(), Some("alt+t→rendered"));

        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        assert_eq!(p.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
        assert_eq!(p.exit_hint(), "esc→agent");
        assert_eq!(p.action_hint(), Some("t→editing"));
    }

    #[test]
    fn alt_t_turns_the_pad_over_both_ways_and_bare_t_only_turns_it_back() {
        let dir = TempDir::new("pad-turn");
        let (mut p, _) = pad(&dir);
        assert_eq!(p.form, Form::Edit);

        // In the edit form the bare key is a letter, and pressing it proves
        // both halves at once.
        assert_eq!(p.handle_key(key(KeyCode::Char('t'))).unwrap(), Handled::Yes);
        assert_eq!(p.text.text(), "t");
        assert_eq!(p.form, Form::Edit);

        assert_eq!(p.handle_key(alt(KeyCode::Char('t'))).unwrap(), Handled::Yes);
        assert_eq!(p.form, Form::Rendered);
        // In the rendering the bare key is the toggle the file reader taught.
        assert_eq!(p.handle_key(key(KeyCode::Char('t'))).unwrap(), Handled::Yes);
        assert_eq!(p.form, Form::Edit);
        assert_eq!(p.text.text(), "t", "the toggle typed nothing");

        // And the chord comes back the other way too, including when the
        // terminal reports the shift that made the letter upper case.
        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        assert_eq!(p.form, Form::Rendered);
        p.handle_key(KeyEvent::new(
            KeyCode::Char('T'),
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert_eq!(p.form, Form::Edit);
    }

    #[test]
    fn the_chord_turns_the_pad_over_from_either_alt_key() {
        // Windows reports AltGr as Ctrl+Alt, so a right-hand `Alt+T` arrives
        // here carrying CONTROL. This pane used to ask `alt && !ctrl` and so
        // turned over from one half of the keyboard only — while every global
        // binding worked from both, because `crate::keys::global` never looked
        // at CONTROL. The bug was the disagreement, and this is its test; the
        // shared answer is `crate::keys::alt_chord`.
        let dir = TempDir::new("pad-altgr");
        let (mut p, _) = pad(&dir);
        let altgr = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT | KeyModifiers::CONTROL);

        assert_eq!(p.handle_key(altgr('t')).unwrap(), Handled::Yes);
        assert_eq!(p.form, Form::Rendered);
        assert_eq!(p.text.text(), "", "the chord typed nothing");

        // And back, which is the arm in the *other* form — it had the same
        // fault written a different way, as a bare `!ctrl`.
        assert_eq!(p.handle_key(altgr('t')).unwrap(), Handled::Yes);
        assert_eq!(p.form, Form::Edit);

        // A plain Ctrl chord is still not the toggle: the pad's Ctrl chords
        // are undo, redo and save, and it is only Ctrl *with* Alt that means
        // AltGr.
        let ctrl_t = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(p.handle_key(ctrl_t).unwrap(), Handled::No);
        assert_eq!(p.form, Form::Edit);
        assert_eq!(p.text.text(), "", "and Ctrl+T is not a letter either");
    }

    #[test]
    fn a_character_behind_altgr_is_typed_rather_than_dropped() {
        // `€` is AltGr+4 on a UK layout and AltGr+E on a German one, and
        // crossterm reports it as `Char('€')` with ALT and CONTROL both set —
        // the character, not the key under it. The text arm used to demand
        // `!ctrl && !alt` and dropped every one of them silently.
        let dir = TempDir::new("pad-altgr-text");
        let (mut p, _) = pad(&dir);
        for c in ['€', '@', '\\', '~'] {
            let ev = KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT | KeyModifiers::CONTROL);
            assert_eq!(p.handle_key(ev).unwrap(), Handled::Yes, "AltGr {c}");
        }
        assert_eq!(p.text.text(), "€@\\~");

        // The chords either side of it are still chords, so this bought the
        // characters back without spending `Ctrl+A` or `Alt+B` on them.
        for mods in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            assert_eq!(
                p.handle_key(KeyEvent::new(KeyCode::Char('a'), mods))
                    .unwrap(),
                Handled::No,
                "{mods:?}"
            );
        }
        assert_eq!(p.text.text(), "€@\\~");
    }

    #[test]
    fn what_the_pane_takes_and_where_it_points_follow_the_form_it_is_in() {
        let dir = TempDir::new("pad-form");
        let (mut p, _) = pad(&dir);
        type_in(&mut p, "# a heading");
        screen(&mut p, 30, 6);

        assert!(p.takes_input());
        assert!(p.cursor().is_some());
        assert_eq!(p.title(), "pad");

        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        screen(&mut p, 30, 6);
        assert!(!p.takes_input(), "nothing can be typed into a rendering");
        assert_eq!(p.cursor(), None, "and there is no cell to point at");
        assert_eq!(p.title(), "pad · rendered");
    }

    // --- the one calculation ----------------------------------------------

    #[test]
    fn the_cursor_sits_on_the_cell_the_character_it_is_in_front_of_is_drawn_in() {
        let dir = TempDir::new("pad-cells");
        let (mut p, _) = pad(&dir);
        // Two ideographs and eight letters: ten characters and twelve cells, in
        // a ten-cell pane. It wraps after `f`, and every column below is a
        // different number from the char index that produced it.
        p.handle_paste("設計abcdefgh").unwrap();
        assert_eq!(scroll::bar_width(10), 0, "no bar to reserve a column for");
        screen(&mut p, 10, 4);

        // The caret arrives at the end of the paste, which is the second row.
        assert_eq!(p.text.caret(), (0, 10));
        assert_eq!(p.cursor(), Some((2, 1)));

        // ...and in front of `c`, which is character four and cell six.
        p.handle_key(key(KeyCode::Home)).unwrap();
        for _ in 0..4 {
            p.handle_key(key(KeyCode::Right)).unwrap();
        }
        screen(&mut p, 10, 4);
        assert_eq!(p.text.caret(), (0, 4));
        assert_eq!(
            p.cursor(),
            Some((6, 0)),
            "the caret was measured in characters rather than cells"
        );

        // The assertion that makes the two agree rather than merely both
        // exist: the cell the cursor names is the one holding the character it
        // is in front of.
        let drawn = screen(&mut p, 10, 4);
        let (col, row) = p.cursor().expect("a caret in the edit form");
        let at: char = drawn
            .chars()
            .nth(row as usize * 10 + col as usize)
            .expect("a cell on screen");
        assert_eq!(at, 'c');
    }

    #[test]
    fn a_click_puts_the_caret_where_it_was_clicked_including_past_a_short_row() {
        let dir = TempDir::new("pad-click");
        let (mut p, _) = pad(&dir);
        p.handle_paste("hello\nhi\n").unwrap();
        screen(&mut p, 20, 6);

        assert_eq!(p.handle_mouse(&click(3, 0)).unwrap(), Handled::Yes);
        assert_eq!(p.text.caret(), (0, 3));

        // The commonest click there is: the empty space to the right of a short
        // line, on a row every other line in the document is wider than.
        p.handle_mouse(&click(15, 1)).unwrap();
        assert_eq!(p.text.caret(), (1, 2));

        // And below the last row, which means the last row.
        p.handle_mouse(&click(0, 5)).unwrap();
        assert_eq!(p.text.caret(), (2, 0));

        // A click in the rendering points at nothing, because rendered rows
        // and source rows do not correspond.
        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        screen(&mut p, 20, 6);
        assert_eq!(p.handle_mouse(&click(1, 0)).unwrap(), Handled::No);
    }

    #[test]
    fn a_click_on_a_wrapped_row_puts_the_caret_on_the_row_that_was_clicked() {
        // The invariant the module doc names, and the one a pointer makes
        // visible. `abc日def` at four columns wraps to `abc` / `日de` / `f`, so
        // cell three of the first row is past its content — and answering with
        // the index the second row starts at would draw the caret a line below
        // the cell the pointer was over.
        let dir = TempDir::new("pad-wrapped-click");
        let (mut p, _) = pad(&dir);
        p.handle_paste("abc日def").unwrap();
        screen(&mut p, 4, 6);

        p.handle_mouse(&click(3, 0)).unwrap();
        assert_eq!(
            p.text.caret(),
            (0, 2),
            "the last caret position that belongs to the row clicked"
        );
        screen(&mut p, 4, 6);
        assert_eq!(p.cursor().map(|(_, row)| row), Some(0), "and drawn on it");

        // Every cell of every row, which is the reviewer's sweep in miniature:
        // a click inside a row's rectangle never produces a caret drawn outside
        // it.
        for row in 0..3u16 {
            for col in 0..4u16 {
                p.handle_mouse(&click(col, row)).unwrap();
                screen(&mut p, 4, 6);
                assert_eq!(
                    p.cursor().map(|(_, at)| at),
                    Some(row),
                    "a click at row {row} cell {col} was drawn on another row"
                );
            }
        }
    }

    #[test]
    fn a_click_against_a_layout_the_last_key_invalidated_is_declined() {
        // `App::run` drains every queued input event before it draws, so these
        // two arrive with no frame between them. Answered against the previous
        // frame's wrap table, the click lands on `bravo` — a line the pointer
        // was never over, and no panic to mark it, because `set_caret` clamps
        // whatever it is handed.
        let dir = TempDir::new("pad-stale-click");
        let (mut p, _) = pad(&dir);
        p.handle_paste("alpha\nbravo\ncharlie\n").unwrap();
        screen(&mut p, 20, 8);
        p.text.set_caret(0, 0);

        p.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(
            p.handle_mouse(&click(0, 2)).unwrap(),
            Handled::No,
            "the click was answered against a table the Enter had invalidated"
        );
        assert_eq!(p.text.caret(), (1, 0), "and moved the caret anyway");

        // The frame the keystroke already asked for makes the next one right.
        screen(&mut p, 20, 8);
        p.handle_mouse(&click(0, 2)).unwrap();
        assert_eq!(p.text.caret(), (2, 0));
    }

    #[test]
    fn a_click_on_the_column_the_scrollbar_reserved_is_not_a_place_in_the_text() {
        let dir = TempDir::new("pad-bar-click");
        let (mut p, _) = pad(&dir);
        p.handle_paste("hello\nhi\n").unwrap();
        screen(&mut p, 30, 6);
        assert_eq!(scroll::bar_width(30), 1, "wide enough to keep a column");
        p.text.set_caret(0, 0);

        assert_eq!(
            p.handle_mouse(&click(29, 0)).unwrap(),
            Handled::No,
            "the bar's column answered as though it were text"
        );
        assert_eq!(p.text.caret(), (0, 0));

        // The last column of the *text* is still text, which is the half this
        // must not break: a click in the empty space to the right of a short
        // line is the commonest press there is.
        assert_eq!(p.handle_mouse(&click(28, 0)).unwrap(), Handled::Yes);
        assert_eq!(p.text.caret(), (0, 5));
    }

    #[test]
    fn a_click_lands_on_the_right_row_under_a_notice() {
        // The other half of the one-calculation rule: what was drawn and what a
        // click is measured against are the same number. A notice that pushed
        // the text down without the click knowing would put the caret a row
        // above wherever the pointer was.
        let dir = TempDir::new("pad-click-notice");
        let (mut p, _) = pad(&dir);
        p.handle_paste("hello\nhi\n").unwrap();
        p.read = true;
        p.failed = Some("abeam could not save the scratch pad.".to_string());
        screen(&mut p, 46, 8);
        assert!(p.noticed > 0, "the failure was not drawn");

        p.handle_mouse(&click(1, p.noticed + 1)).unwrap();
        assert_eq!(p.text.caret(), (1, 1));
    }

    // --- glancing, and never typing ---------------------------------------

    #[test]
    fn a_glance_scrolls_the_pad_in_both_forms_and_types_nothing_into_it() {
        let dir = TempDir::new("pad-glance");
        let (mut p, _) = pad(&dir);
        p.handle_paste(&"a line\n".repeat(40)).unwrap();
        screen(&mut p, 30, 6);
        assert!(p.view.scroll.offset > 0, "a pad opens where the writing stopped");

        let before = p.text.text();
        p.view.scroll.to(0);
        assert_eq!(p.scroll_key(key(KeyCode::Down)).unwrap(), Handled::Yes);
        assert_eq!(p.view.scroll.offset, 1);
        assert_eq!(p.scroll_key(key(KeyCode::PageDown)).unwrap(), Handled::Yes);
        assert!(p.view.scroll.offset > 1);
        assert_eq!(p.scroll_key(key(KeyCode::Up)).unwrap(), Handled::Yes);
        assert_eq!(
            p.text.text(),
            before,
            "a glance binding typed into the document"
        );

        // The same in the rendering, where the default would have served but
        // the override has to keep working.
        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        screen(&mut p, 30, 6);
        p.view.scroll.to(0);
        assert_eq!(p.scroll_key(key(KeyCode::Down)).unwrap(), Handled::Yes);
        assert_eq!(p.view.scroll.offset, 1);
    }

    #[test]
    fn a_frame_with_no_room_in_it_does_not_forget_where_the_pad_was_left() {
        // A terminal dragged down to nothing and back. The early return used to
        // measure a viewport of zero on the way out, and `Scroll::measure`
        // clamps the offset to what it has just been told will fit — so a
        // reader half way through a long pad came back to the top of it with no
        // key having been pressed.
        let dir = TempDir::new("pad-squeezed");
        let (mut p, _) = pad(&dir);
        p.handle_paste(&"a line\n".repeat(40)).unwrap();
        screen(&mut p, 30, 8);
        p.view.scroll.to(12);
        let was = p.view.scroll.offset;
        assert!(was > 0, "somewhere to come back to");

        draw(&mut p, Rect::new(0, 0, 30, 0));
        draw(&mut p, Rect::new(0, 0, 0, 8));
        draw(&mut p, Rect::new(0, 0, 0, 0));
        assert_eq!(p.view.scroll.offset, was);

        screen(&mut p, 30, 8);
        assert_eq!(p.view.scroll.offset, was, "the pad came back somewhere else");
    }

    #[test]
    fn the_wheel_moves_the_page_in_both_forms() {
        let dir = TempDir::new("pad-wheel");
        let (mut p, _) = pad(&dir);
        p.handle_paste(&"a line\n".repeat(40)).unwrap();
        screen(&mut p, 30, 6);
        p.view.scroll.to(0);
        p.handle_mouse(&wheel_down()).unwrap();
        assert_eq!(p.view.scroll.offset, 3);

        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        screen(&mut p, 30, 6);
        p.view.scroll.to(0);
        p.handle_mouse(&wheel_down()).unwrap();
        assert_eq!(p.view.scroll.offset, 3);
    }

    // --- pasting ----------------------------------------------------------

    #[test]
    fn a_paste_goes_into_the_pad_and_is_declined_by_the_rendering() {
        let dir = TempDir::new("pad-paste");
        let (mut p, _) = pad(&dir);
        assert_eq!(
            p.handle_paste("the retry budget\r\nand the timeout")
                .unwrap(),
            Handled::Yes
        );
        assert_eq!(p.text.text(), "the retry budget\nand the timeout");

        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        assert_eq!(p.handle_paste("and one more thing").unwrap(), Handled::No);
        assert_eq!(p.text.text(), "the retry budget\nand the timeout");
    }

    // --- what the pane says about its own state ---------------------------

    #[test]
    fn an_empty_pad_says_what_it_is_and_a_pad_with_writing_in_it_does_not() {
        let dir = TempDir::new("pad-opening");
        let (mut p, _) = pad(&dir);
        let drawn = screen(&mut p, 46, 14);
        assert!(drawn.contains("scratch"), "{drawn}");
        assert!(
            drawn.contains("alt+t"),
            "the only place the toggle is discoverable: {drawn}"
        );

        type_in(&mut p, "a");
        let drawn = screen(&mut p, 46, 14);
        assert!(
            !drawn.contains("scratch"),
            "the hint outstayed the empty pad: {drawn}"
        );
        assert!(drawn.starts_with('a'), "{drawn}");
    }

    #[test]
    fn a_pad_that_will_take_no_more_says_so_rather_than_leaving_a_dead_key() {
        let dir = TempDir::new("pad-full");
        let (mut p, _) = pad(&dir);
        p.read = true;
        p.text = Editor::from_text(PAD, &"x".repeat(MAX_BYTES));
        p.rev += 1;

        assert_eq!(p.handle_key(key(KeyCode::Char('y'))).unwrap(), Handled::Yes);
        assert!(p.refused);
        let drawn = screen(&mut p, 46, 10);
        assert!(drawn.contains("full"), "{}", &drawn[..46 * 3]);
    }

    #[test]
    fn a_full_pad_keeps_the_letter_q_instead_of_handing_the_writer_to_the_agent() {
        // `crate::app` reads a bare `q` the right pane declined as "the user is
        // done with this pane" and moves focus to the agent. A pad that turned
        // `q` down because it had no room for it would therefore throw the
        // writer into the agent's prompt at the exact moment it was trying to
        // say it was full — and every letter after that would be typed at a
        // conversation rather than into a note.
        let dir = TempDir::new("pad-full-q");
        let (mut p, _) = pad(&dir);
        p.read = true;
        p.text = Editor::from_text(PAD, &"x".repeat(MAX_BYTES));
        p.rev += 1;

        for ch in "qjgt ".chars() {
            assert_eq!(
                p.handle_key(key(KeyCode::Char(ch))).unwrap(),
                Handled::Yes,
                "{ch:?} stopped being the pad's the moment the pad was full"
            );
        }
        // `Esc` still is not the pad's, full or empty, which is what keeps the
        // way out working.
        assert_eq!(p.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
    }

    #[test]
    fn a_refusal_asks_for_the_frame_that_draws_the_notice_explaining_it() {
        // `App::handle_event` paints only for an event something came of, so a
        // refusal reporting `No` would set the flag and never show the sentence
        // — which is the silence `MAX_BYTES` is built to prevent, back
        // by another door. Nothing else rescues it: `tick` asks for no frame
        // unless a save is due, and a pad at the cap has nothing left to save.
        let dir = TempDir::new("pad-refusal-frame");
        let (mut p, _) = pad(&dir);
        p.read = true;
        p.text = Editor::from_text(PAD, &"x".repeat(MAX_BYTES));
        p.rev += 1;
        // A frame with no notice on it yet, so that the assertion below is
        // about this keystroke rather than about what was already drawn.
        p.refused = false;
        let before = screen(&mut p, 46, 10);
        assert!(before.contains("full"), "the standing notice");

        // One byte of room, and a character that needs two. This is the case
        // `Buffer::is_full`'s caveat is about — refused, with the pad not full
        // — and it is a keystroke rather than a paste, which is the half the
        // notice used to be wrong about.
        p.text = Editor::from_text(PAD, &"x".repeat(MAX_BYTES - 1));
        p.rev += 1;
        assert!(!p.text.is_full(), "room to spare");
        let quiet = screen(&mut p, 46, 10);
        assert!(!quiet.contains("full"), "a pad with room says nothing");

        assert_eq!(
            p.handle_key(key(KeyCode::Char('é'))).unwrap(),
            Handled::Yes,
            "the frame that would have drawn the notice was never asked for"
        );
        assert!(!p.tick(), "and tick will not ask for one either");
        let drawn = screen(&mut p, 46, 10);
        assert!(drawn.contains("fit"), "{}", &drawn[..46 * 3]);
    }

    #[test]
    fn a_paste_of_nothing_is_not_a_paste_that_would_not_fit() {
        // `Buffer::insert_str` answers false for an empty paste and for one too
        // large, and only the second is worth a notice.
        let dir = TempDir::new("pad-empty-paste");
        let (mut p, _) = pad(&dir);
        assert_eq!(p.handle_paste("").unwrap(), Handled::No);
        assert!(!p.refused, "an empty clipboard was reported as too large");
    }

    #[test]
    fn a_paste_turned_away_with_room_to_spare_is_still_a_paste_that_said_nothing() {
        // `Buffer::is_full` reports the pad's state rather than the last
        // refusal, and its own documentation says so: a paste bigger than the
        // room left is refused whole by a pad that is not full.
        let dir = TempDir::new("pad-refused");
        let (mut p, _) = pad(&dir);
        p.read = true;
        p.text = Editor::from_text(PAD, &"x".repeat(MAX_BYTES - 4));
        p.rev += 1;
        assert!(!p.text.is_full());

        assert_eq!(
            p.handle_paste("a paste much longer than four bytes")
                .unwrap(),
            Handled::Yes,
            "claimed, because the refusal is a sentence that has to be drawn"
        );
        let drawn = screen(&mut p, 46, 10);
        assert!(drawn.contains("fit"), "{}", &drawn[..46 * 3]);
    }

    #[test]
    fn a_save_that_failed_is_on_screen_rather_than_swallowed() {
        let dir = TempDir::new("pad-failed");
        let (mut p, _) = pad(&dir);
        p.read = true;
        p.failed = Some("abeam could not save the scratch pad: nowhere.".to_string());
        let drawn = screen(&mut p, 46, 10);
        assert!(drawn.contains("nowhere"), "{}", &drawn[..46 * 3]);
    }

    // --- undo, redo, and saving now ---------------------------------------

    #[test]
    fn ctrl_z_takes_a_sentence_back_a_word_at_a_time_and_ctrl_y_puts_it_back() {
        let dir = TempDir::new("pad-undo");
        let (mut p, _) = pad(&dir);
        screen(&mut p, 40, 8);
        type_in(&mut p, "the retry budget");

        assert_eq!(p.handle_key(ctrl('z')).unwrap(), Handled::Yes);
        assert_eq!(p.text.text(), "the retry ");
        assert_eq!(p.handle_key(ctrl('z')).unwrap(), Handled::Yes);
        assert_eq!(p.text.text(), "the ");
        assert_eq!(p.handle_key(ctrl('y')).unwrap(), Handled::Yes);
        assert_eq!(p.text.text(), "the retry ");

        // The caret came back with the text, and the next frame draws it
        // there: after the word that was put back, not where it was before.
        screen(&mut p, 40, 8);
        assert_eq!(p.cursor(), Some((10, 0)));
    }

    #[test]
    fn an_undo_with_nothing_to_take_back_types_nothing_and_asks_for_no_frame() {
        let dir = TempDir::new("pad-undo-nothing");
        let (mut p, _) = pad(&dir);
        screen(&mut p, 40, 8);
        assert_eq!(p.handle_key(ctrl('z')).unwrap(), Handled::No);
        assert_eq!(p.handle_key(ctrl('y')).unwrap(), Handled::No);
        assert_eq!(p.text.text(), "", "a chord is not a letter");

        // A capital with no Shift reported is Caps Lock, and still undo; with
        // the Shift reported it is `Ctrl+Shift+Z`, which is redo.
        type_in(&mut p, "x");
        let caps = KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::CONTROL);
        assert_eq!(p.handle_key(caps).unwrap(), Handled::Yes);
        assert_eq!(p.text.text(), "");
        let shifted = KeyEvent::new(
            KeyCode::Char('Z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(p.handle_key(shifted).unwrap(), Handled::Yes);
        assert_eq!(p.text.text(), "x");
    }

    #[test]
    fn altgr_z_is_a_character_and_not_an_undo() {
        // Ctrl+Alt is how Windows spells AltGr, and on a layout where AltGr+Z
        // types something it arrives as that character. Read as an undo, it
        // would take text back while the user was typing more of it.
        let dir = TempDir::new("pad-altgr-z");
        let (mut p, _) = pad(&dir);
        type_in(&mut p, "ab");
        let altgr = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT | KeyModifiers::CONTROL);
        assert_eq!(p.handle_key(altgr).unwrap(), Handled::Yes);
        assert_eq!(p.text.text(), "abz");
    }

    #[test]
    fn an_undo_is_owed_to_the_file_like_any_other_edit() {
        let dir = TempDir::new("pad-undo-saved");
        let (mut p, path) = pad(&dir);
        screen(&mut p, 40, 8);
        type_in(&mut p, "keep this drop that");
        p.flush();
        assert!(p.changed.is_none());

        p.handle_key(ctrl('z')).unwrap();
        assert_eq!(p.text.text(), "keep this drop ");
        assert!(p.changed.is_some(), "the undo changed the text and is unsaved");
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(!p.tick());
        assert_eq!(
            std::fs::read_to_string(&path).expect("the pad"),
            "keep this drop "
        );
    }

    #[test]
    fn an_undo_back_to_what_was_saved_leaves_nothing_owed() {
        let dir = TempDir::new("pad-undo-to-saved");
        let (mut p, path) = pad(&dir);
        screen(&mut p, 40, 8);
        type_in(&mut p, "abc");
        p.handle_key(ctrl('s')).unwrap();
        assert!(p.changed.is_none());

        // The step after a save is a step of its own — this used to undo all
        // six letters — and undoing it lands on what is on disk.
        type_in(&mut p, "def");
        assert!(p.changed.is_some());
        p.handle_key(ctrl('z')).unwrap();
        assert_eq!(p.text.text(), "abc");
        assert!(p.changed.is_none(), "the file already has this text");

        // So the debounce writes nothing: the file is left as it is.
        std::fs::write(&path, "abc").expect("the same bytes, newer");
        let before = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        assert!(!p.tick());
        p.flush();
        assert_eq!(std::fs::metadata(&path).and_then(|m| m.modified()).ok(), before);
    }

    #[test]
    fn ctrl_s_on_a_pad_that_must_not_be_saved_writes_nothing() {
        // The truncated and the unreadable pad each refuse every save for the
        // session, and `Ctrl+S` is a save like the others: it must not be the
        // one door that writes the front of somebody's notes over the whole.
        let dir = TempDir::new("pad-ctrl-s-truncated");
        let (mut p, path) = pad(&dir);
        p.read = true;
        p.truncated = true;
        p.text = Editor::from_text(PAD, "the first sixty-four kilobytes of it\n");
        p.rev += 1;
        type_in(&mut p, "more");
        p.handle_key(ctrl('s')).unwrap();
        assert!(!path.exists(), "a truncated pad was written by Ctrl+S");

        let dir = TempDir::new("pad-ctrl-s-unreadable");
        let (mut p, path) = pad(&dir);
        std::fs::create_dir_all(&path).expect("a directory in the file's place");
        screen(&mut p, 46, 12);
        assert!(p.unreadable);
        type_in(&mut p, "more");
        p.handle_key(ctrl('s')).unwrap();
        assert!(p.failed.is_none(), "the pane tried to write: {:?}", p.failed);
        assert!(path.is_dir());
    }

    #[test]
    fn an_undo_takes_down_the_notice_about_a_refusal_it_has_made_moot() {
        let dir = TempDir::new("pad-undo-refused");
        let (mut p, _) = pad(&dir);
        p.read = true;
        p.text = Editor::from_text(PAD, &"x".repeat(MAX_BYTES - 4));
        p.rev += 1;
        type_in(&mut p, "ab");
        p.handle_paste("a paste much longer than the two bytes left").unwrap();
        assert!(p.refused);

        assert_eq!(p.handle_key(ctrl('z')).unwrap(), Handled::Yes);
        assert!(!p.refused, "the notice outlived the text it was about");
    }

    #[test]
    fn ctrl_s_saves_now_rather_than_two_seconds_from_now() {
        let dir = TempDir::new("pad-ctrl-s");
        let (mut p, path) = pad(&dir);
        screen(&mut p, 40, 8);
        type_in(&mut p, "a thought worth keeping");
        assert!(!path.exists(), "the debounce has not run");

        assert_eq!(
            p.handle_key(ctrl('s')).unwrap(),
            Handled::No,
            "a save that worked changes nothing on screen, so asks for no frame"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("the pad Ctrl+S wrote"),
            "a thought worth keeping"
        );
        assert!(p.changed.is_none(), "and nothing is owed any more");
        assert_eq!(p.text.text(), "a thought worth keeping", "and nothing was typed");

        // With nothing owed it writes nothing, as `flush` does not.
        std::fs::remove_file(&path).expect("clear the file");
        assert_eq!(p.handle_key(ctrl('s')).unwrap(), Handled::No);
        assert!(!path.exists());
    }

    #[test]
    fn ctrl_s_saves_from_the_rendering_too_and_undo_does_not_reach_it() {
        let dir = TempDir::new("pad-ctrl-s-rendered");
        let (mut p, path) = pad(&dir);
        screen(&mut p, 40, 8);
        type_in(&mut p, "read it back");
        p.handle_key(alt(KeyCode::Char('t'))).unwrap();
        assert_eq!(p.form, Form::Rendered);

        p.handle_key(ctrl('s')).unwrap();
        assert!(path.exists(), "saving is not typing, so the rendering takes it");
        assert_eq!(p.handle_key(ctrl('z')).unwrap(), Handled::No);
        assert_eq!(p.text.text(), "read it back", "the rendering is read-only");
    }

    #[test]
    fn a_ctrl_s_that_fails_says_so_on_screen() {
        // The retry test's obstruction: a file where the pad's directory would
        // have to be.
        let dir = TempDir::new("pad-ctrl-s-failed");
        let (mut p, _) = pad(&dir);
        screen(&mut p, 40, 8);
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").expect("a file in the way");
        p.set_path(blocked.join("scratch.md"));
        type_in(&mut p, "worth keeping");

        assert_eq!(
            p.handle_key(ctrl('s')).unwrap(),
            Handled::Yes,
            "the failure is a notice that was not there before"
        );
        assert!(p.failed.is_some());
        assert!(p.changed.is_some(), "and the save is still owed");
    }

    #[test]
    fn the_overlay_names_the_pads_three_ctrl_keys() {
        let row = |key: &str| {
            crate::keys::HELP
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, what)| *what)
                .unwrap_or_else(|| panic!("{key} is not in the F1 overlay"))
        };
        assert!(row("Ctrl+Z / Ctrl+Y (pad)").contains("undo / redo"));
        assert!(row("Ctrl+S (pad)").contains("save now"));
    }

    // --- the file -----------------------------------------------------------

    #[test]
    fn the_pad_is_not_read_until_the_first_frame_and_is_read_then() {
        let dir = TempDir::new("pad-deferred");
        let (mut p, _) = pad(&dir);
        assert!(
            !p.read,
            "building a pane went to the disk; a session that never presses \
             the key must not pay for it"
        );
        screen(&mut p, 30, 6);
        assert!(p.read);
    }

    #[test]
    fn what_was_typed_is_on_disk_and_is_there_again_the_next_time() {
        let dir = TempDir::new("pad-roundtrip");
        let (mut p, path) = pad(&dir);
        screen(&mut p, 40, 8);
        p.handle_paste("ask about the retry budget\n").unwrap();
        assert!(!p.tick(), "a pad is not written two seconds early");
        assert!(!path.exists());

        // ...and now the two seconds have passed.
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(!p.tick(), "a save that worked changes nothing on screen");
        assert_eq!(p.failed, None, "{:?}", p.failed);
        assert!(p.changed.is_none(), "the pad is still marked unsaved");
        assert!(path.exists());

        let (mut back, _) = pad(&dir);
        assert!(back.text.is_empty(), "read before the first frame");
        screen(&mut back, 40, 8);
        assert_eq!(back.text.text(), "ask about the retry budget\n");
    }

    #[test]
    fn flushing_writes_a_pad_that_the_debounce_has_not_reached_yet() {
        let dir = TempDir::new("pad-flush");
        let (mut p, path) = pad(&dir);
        screen(&mut p, 40, 8);
        type_in(&mut p, "quit before the debounce");
        p.flush();
        assert_eq!(p.failed, None, "{:?}", p.failed);
        assert!(path.exists(), "the pad was lost on the way out");

        // ...and a pad with nothing new in it writes nothing at all, which is
        // what stops a view switch rewriting a file every time.
        let _ = std::fs::remove_file(&path);
        p.flush();
        assert!(!path.exists());
    }

    #[test]
    fn a_pad_that_arrived_truncated_says_so_and_is_never_written() {
        let dir = TempDir::new("pad-truncated");
        let (mut p, path) = pad(&dir);
        // The state a load leaves behind when the file was longer than the
        // buffer's cap. Planted rather than made: `store`'s own tests prove
        // that a file over the cap comes back flagged, and writing 64 KiB here
        // to prove it again would say nothing about the pane.
        p.read = true;
        p.truncated = true;
        p.text = Editor::from_text(PAD, "the first sixty-four kilobytes of it\n");
        p.rev += 1;

        let drawn = screen(&mut p, 46, 12);
        assert!(drawn.contains("fit"), "{}", &drawn[..46 * 4]);
        assert!(drawn.contains("delete"), "{}", &drawn[..46 * 4]);

        type_in(&mut p, "and a note of my own");
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(!p.tick());
        p.flush();
        assert!(
            !path.exists(),
            "a truncated pad was written over {}",
            path.display()
        );
    }

    #[test]
    fn a_pad_that_could_not_be_read_says_so_and_is_never_written() {
        // The failure this pane came closest to shipping. A directory where the
        // file should be is the portable way to have a path that is there and
        // will not open; the one that made it critical is a file a scanner has
        // held open for a moment, and `store` has that test because only
        // Windows can express it.
        let dir = TempDir::new("pad-unreadable");
        let (mut p, path) = pad(&dir);
        std::fs::create_dir_all(&path).expect("a directory in the file's place");

        let drawn = screen(&mut p, 46, 12);
        assert!(drawn.contains("replace"), "{}", &drawn[..46 * 5]);

        type_in(&mut p, "and a note of my own");
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(
            !p.tick(),
            "a pane that must not save has nothing new to draw"
        );
        p.flush();

        // No failure message, which is the assertion: a pane that had tried and
        // been stopped by the filesystem would have one. This one did not try.
        assert!(
            p.failed.is_none(),
            "the pane wrote, and was only saved by luck: {:?}",
            p.failed
        );
        assert!(path.is_dir(), "...and the thing in the way is still there");
    }

    #[test]
    fn a_pad_with_nowhere_to_go_says_so_before_anybody_has_typed() {
        // A machine that will not say where the profile is. The old shape found
        // this out on the first debounce, two seconds after somebody had filled
        // the pad with sentences that were already going nowhere.
        let dir = TempDir::new("pad-nowhere");
        let (mut p, _) = pad(&dir);
        p.path = None;

        let drawn = screen(&mut p, 46, 10);
        assert!(drawn.contains("nowhere"), "{}", &drawn[..46 * 5]);
        assert!(
            p.changed.is_none() && p.failed.is_none(),
            "the notice is the opening screen and not the wreck of a save"
        );
    }

    #[test]
    fn a_save_that_failed_is_tried_again_rather_than_given_up_on() {
        let dir = TempDir::new("pad-retry");
        let (mut p, _) = pad(&dir);
        screen(&mut p, 40, 8);

        // A file where the pad's directory would have to be, so that creating
        // it fails: an ordinary, temporary, unlucky sort of failure, which is
        // the kind this is about. Pointed there after the first frame, so the
        // read that has already happened is the one the fixture arranged.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").expect("a file in the way");
        let path = blocked.join("scratch.md");
        p.set_path(path.clone());

        type_in(&mut p, "worth keeping");
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(
            p.tick(),
            "the failure is a notice that was not there before"
        );
        assert!(p.failed.is_some());
        assert!(
            p.changed.is_some(),
            "a save that failed is still owed, and nothing else will ever ask"
        );

        // The obstruction goes, and the next quiet interval writes the pad.
        // That is the whole of it: the text was never unrecoverable, only
        // unwritten, and clearing the dirty flag up front threw it away.
        std::fs::remove_file(&blocked).expect("clear the way");
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(p.tick(), "the notice goes when the save works");
        assert_eq!(p.failed, None, "{:?}", p.failed);
        assert!(path.exists(), "the second attempt wrote the pad");
        assert!(p.changed.is_none(), "and nothing is owed any more");
    }

    #[test]
    fn a_second_window_writing_the_pad_is_noticed_rather_than_overwritten() {
        // Two terminals in one repository is ordinary, and two abeams on one
        // workspace share one file. Without the stamp they overwrite each other
        // for the rest of the session with nothing on either screen.
        let dir = TempDir::new("pad-second-window");
        let (mut p, path) = pad(&dir);
        screen(&mut p, 46, 10);
        type_in(&mut p, "mine");
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(!p.tick(), "the first save works and says nothing");

        let theirs = "what the other window had in it\n";
        std::fs::write(&path, theirs).expect("the other abeam's save");

        type_in(&mut p, " and more");
        p.changed = Instant::now().checked_sub(QUIET * 2);
        assert!(p.tick(), "a refusal is a notice that was not there before");
        let why = p.failed.clone().expect("a sentence about the other window");
        assert!(why.contains("changed on disk"), "{why}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the pad is still there"),
            theirs,
            "the other window's notes were overwritten"
        );
    }
}
