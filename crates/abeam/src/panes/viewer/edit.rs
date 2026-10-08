//! Typing into the file on screen, and putting it back on disk.
//!
//! `e` turns the source view into an editor: the same rows, the same gutter, a
//! caret in them. Everything a keystroke does to the text is
//! `crate::editor`'s, which the pad has been typing into since before this
//! file existed, and everything a save does to the disk is `crate::disk`'s,
//! which this file is the first caller of. What is left here is the pane
//! around the two: which state the page is in, what the border says about it,
//! and every way the rest of abeam could replace a page that is holding text
//! no disk has.
//!
//! ## Three states, and the one that is new
//!
//! **Reading the disk** is the viewer as it always was, and [`Edit`] does not
//! exist. **Typing** is `e`: the caret is in the text and every printable key
//! is a letter, `q` and `j` included. **Reading unsaved text** is the state the
//! others do not have, and it is the one this design turns on: `Esc` from
//! typing goes back to the reading view and *keeps the text*, shown rendered or
//! as source exactly as `t` says, with `● ` in front of the file's name in the
//! title — and, on the border while the pane has the keys, `● unsaved` leading
//! the hint that says what can be done with it (see the border section below).
//! `Esc` never throws anything away, because it is the key every other mode in
//! this pane has taught as "never mind", and a "never mind" that cost somebody
//! a paragraph would be the worst key in the program. Throwing it away is `x`
//! twice, saving it is `Ctrl+S` — from either state, because saving is not
//! typing — and `e` goes back to typing.
//!
//! The page in that third state is an ordinary [`Doc`] whose body is the
//! editor's text, put there by [`ViewerPane::put_on_page`]. That is what makes
//! the rest of the reader work on it unchanged — `t`, `/`, `o`, the scroll
//! keys, the rendering of a `.py`'s docstrings — rather than each needing to be
//! told that a second source of text exists.
//!
//! An edit with nothing unsaved in it is not kept once nobody is typing: `Esc`
//! from a clean editor closes it, and so does anything that replaces the page.
//! Closing it loses a caret. Keeping it would make every guard below fire on a
//! page that has nothing to lose.
//!
//! ## A file that is not there yet
//!
//! `a` in the file list names one, and `super::name` decides what the name may
//! be. A name that is already a file opens it, as `e` would. Any other opens
//! an empty editor whose baseline is [`Baseline::Absent`] ([`Edit::create`]),
//! and that one fact carries the rest: the first save *creates*, making the
//! directories the name passes through, and is refused like any other if a
//! file has appeared there since — so an agent that wrote the same name first
//! is not written over by one press. Until then nothing of it is on disk.
//! `Esc` with nothing typed goes back to the list and leaves no file and no
//! directory; with something typed it is unsaved text like any other —
//! guarded, copied out, offered back by naming the file again — and `x x`
//! throws it away and puts back the page the name was given over. A first save
//! tells the list and starts the walk again, so the list, the find and `Tab`
//! know the file; the watcher tells the git view.
//!
//! ## What may not replace the page
//!
//! While there is unsaved text the page is the only copy of it outside a
//! recovery file, so anything that would put something else there is refused,
//! with a sentence in the border saying why: `Enter` in the git view, `Tab`
//! and `Shift+Tab`, `r`, `Enter` on a repository result, `F1, B`, and a switch
//! to another worktree. Most of them arrive through `ViewerPane::show`, which
//! is where the guard is, once — see its doc. The file list and the worktree
//! switch do not, and ask [`ViewerPane::may_replace`] themselves. A document
//! the watcher queues waits behind `◆`, as it does behind the file list.
//!
//! ## Saving
//!
//! `Ctrl+S` encodes the text in the file's own [`Format`] and hands it to
//! `disk::save` with the [`Baseline`] — the bytes the editor was opened on, or
//! the bytes the last save wrote. `disk::save` compares the disk with that
//! immediately before it replaces anything, and refuses if they differ. That
//! refusal is the conflict state, `◆ changed on disk`, and it records what the
//! disk held when it refused ([`Conflict::seen`]).
//!
//! **Writing over a change takes a second press, and the second press is still
//! a comparison.** It is a save that expects the bytes the refusal saw, not a
//! save that expects nothing: if the agent has written again since — a version
//! nobody has seen — that save is refused in turn, and the refusal is raised
//! again about the new one. And it counts only once the refusal has been drawn,
//! which is `crate::app`'s `close_drawn` rule arriving in a pane: the shell
//! drains every queued key before it draws, so two `Ctrl+S` in one batch would
//! otherwise refuse and overwrite with the refusal never on screen. A press
//! that arrives before the frame is the press that asks for it.
//!
//! A conflict found any other way — the watcher reporting the file, or a
//! recovery copy typed against an older version — is not a refusal yet, and
//! the first `Ctrl+S` in one writes nothing: it *is* the save that is refused,
//! against whatever the disk holds at that moment. So `ctrl+s again
//! overwrites` is on the border only when there has been a first press, and a
//! habit — `Ctrl+S` on coming back to the pane — can never be the one keystroke
//! that writes over somebody's work. For a recovery copy this is the only thing
//! between the copy and the disk, because there the baseline *is* the disk and
//! `disk::save` would see nothing to refuse.
//!
//! **A clean editor never writes.** `Ctrl+S` with nothing unsaved is nothing,
//! conflict or not: there is no text of the user's in it to put anywhere, and a
//! save from it could only put an old version back over a newer one. `x x` is
//! held to the drawn rule as well.
//!
//! ## The watcher
//!
//! `App::route` hands this pane every path that changed, not only markdown,
//! and the file being edited is compared by both of its spellings —
//! [`Edit::requested`] and [`Edit::resolved`] — under `crate::paths`' rule, so
//! a link and its target and a case difference on Windows are all one file.
//! The disk is then read and compared with the baseline:
//!
//! - **the same** is this pane's own save coming back, and says nothing — no
//!   `◆`, no document queued, no border marked unread. [`Echo`] carries the
//!   same answer for a save whose editor has already closed;
//! - **different, with unsaved text** is the conflict state, at once, rather
//!   than at the next `Ctrl+S` — the save-time comparison is the backstop;
//! - **different, with nothing unsaved** is read again, into a *fresh* editor
//!   with the caret where it was. Fresh, and not the old one with the new text
//!   put in as a step, because a step can be undone: one `Ctrl+Z` would take the
//!   agent's change back out and `Ctrl+S` would then write that away with no
//!   conflict to say so. The reload is a floor the history cannot go below.
//!   It is the one change that raises no `◆`, deliberately — with nothing
//!   unsaved there is nothing to be in conflict with, and following the agent
//!   is what the reader has always done. A file that can no longer be edited at
//!   all — deleted, renamed, grown past the cap, its line endings mixed —
//!   closes the editor instead, and the page shows the disk as it now is, with
//!   the reason above it.
//!
//! ## Recovery copies
//!
//! Unsaved text is copied into the profile — `crate::disk::drafts`, never the
//! repository — on the pad's quiet interval, from the pane's tick. The copy is
//! kept under the path the file resolved to when it was opened, and that stays
//! its key for the whole life of the edit: a save that follows a link
//! repointed since must not leave the copy behind under a name nothing will
//! look for. The copy records the fingerprint of the version the text was
//! typed against. `e` on that file opens the disk's text and puts the copy
//! over it as one step of undo, so the text reads as unsaved and one `Ctrl+Z`
//! is the disk; if the disk is no longer the version the copy was typed
//! against, it opens in the conflict state. Opening the file to read it says
//! that a copy is waiting, and an `e` the strict read refuses says where.
//!
//! **A copy is deleted by a save and by `x x`, and by nothing else.** Not by an
//! undo back to the disk and not by leaving: either can happen to text that
//! came out of a crash, and the cost of keeping a copy too long is that the
//! next `e` offers it again, where the cost of deleting one too soon is the
//! work it held. The one exception is a copy that turns out, at `e`, to be
//! exactly what is on disk, which holds nothing to lose. A copy that is there
//! and will not read is left alone for the session and a notice above the
//! page says so — `Drafts::read`'s rule. Quitting keeps the copy, written up
//! to the last keystroke, and the warning before the quit says so: a confirmed
//! quit takes the text off the screen and not out of the profile. It says so
//! only of a copy that was written — the first `F1, Q` writes it before asking
//! — and a copy the profile refused is named as text the quit will lose.
//!
//! ## What focus does not do
//!
//! Nothing is saved when the keys go back to the agent. The pad saves on
//! focus loss because its file is abeam's own; this file is the user's, and a
//! save is a decision they make with a key, not one `F4` makes for them. The
//! text, the caret and the editor stay exactly where they were.
//!
//! The border's `ctrl+s` is drawn only while the pane has focus, through
//! `Pane::action_hint`, because with the agent focused the chord is the
//! agent's — in Claude it stashes the prompt. The notices above the text name
//! no key that could reach the agent, for the same reason: they are drawn
//! whether or not the pane has the keys.
//!
//! The way out leads, as on every pane, because a border is clipped from the
//! right and the way out is the one instruction nobody can do without:
//! `esc→done · ctrl+s save · ● README.md · editing` while typing, and
//! `esc→agent · ● unsaved · ctrl+s save · e edit · x x discard` while reading
//! unsaved text, which puts the state second for the same reason. A conflict
//! puts `◆` at the front of its instruction. At forty columns the way out and
//! the mark survive in every state, and `crate::app`'s tests measure it.
//!
//! ## What a frame costs
//!
//! The editor's own: [`editor::View::frame`](crate::editor::View::frame) lays
//! out only what changed and colours only the rows asked for, and this file
//! asks for the pane's height and numbers those rows with the reader's
//! [`Gutter`]. No frame reads the whole text — `Editor::text` is called on a
//! save, on `Esc`, and when a recovery copy is written, which is to say on a
//! key or on a two-second quiet, never per frame.
//!
//! **A tab is drawn by the editor's rule and not the reader's**, which the
//! editor's module doc owns: the editor fills to the next four-cell stop from
//! the start of its row, the reader's source view expands counting characters,
//! and a line with a wide character before a tab can come out differently in
//! the two. Accepted for now, and named here because `e` is where somebody
//! will see it.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::source::{self, Grammar};
use super::{Doc, Gutter, Mode, State, ViewerPane};
use crate::disk::drafts::{self, Drafts};
use crate::disk::{self, Baseline, Fingerprint, Format, Opened, Options, Refusal, SaveError, Saved};
use super::load::MAX_BYTES;
use super::name::{self, Named};
use crate::editor::{Editor, Look, Outcome, Policy, TabKey, View};
use crate::pane::Handled;
use crate::panes::pad::QUIET;
use crate::paths;
use crate::scroll::{self, Scroll};
use crate::text::block;

/// Where recovery copies go: the profile's `drafts/` — and, in a build of the
/// tests, a directory under the system temp instead.
///
/// The test half is the pad's `set_path` argument made impossible to forget.
/// `crate::app`'s tests build a whole `App`, and with it this pane, and a test
/// that typed into a file and let a tick pass would otherwise write somebody's
/// test text into the profile of whoever runs the suite. The pane's own tests
/// point it somewhere they own; this is what every other test gets.
pub(super) fn drafts_here() -> Option<Drafts> {
    if cfg!(test) {
        Some(Drafts::at(std::env::temp_dir().join("abeam-test-drafts")))
    } else {
        Drafts::here()
    }
}

/// A file opened for editing, and everything its page owes it until the text
/// is on disk or thrown away.
pub(super) struct Edit {
    /// The file as the reader named it — `Doc::path` — which is what every
    /// sentence about it says and the spelling a save is asked to write to.
    requested: PathBuf,
    /// What that resolves to, links followed: the file a save replaces, and
    /// the second spelling the watcher may report a change by.
    resolved: PathBuf,
    editor: Editor,
    view: View,
    format: Format,
    /// What the disk held when this was opened or last saved.
    baseline: Baseline,
    /// Chosen once, from the file's name and first line, the way the reader
    /// chooses it.
    grammar: Option<Grammar>,
    /// Whether the caret is in the text, or the reader is showing it.
    typing: bool,
    conflict: Option<Conflict>,
    /// `x` was pressed once over the unsaved text. See
    /// [`ViewerPane::discard_key`].
    question: Option<Question>,
    /// The question, set aside by [`ViewerPane::keystroke`] for the key that
    /// might answer it.
    held: Option<Question>,
    /// The last thing a save, a recovery or a reload had to say.
    said: Option<Said>,
    /// Something typed or pasted was turned away for want of room. The pad's
    /// `refused`, for the pad's reason: `Editor::is_full` is a different
    /// question.
    refused: bool,
    copy: Copy,
    /// The reader's offset when `Esc` last left this text on the page, so that
    /// an `e` straight back finds the caret where it was rather than moved to
    /// the top of a page nobody scrolled. See [`ViewerPane::resume_edit`].
    left_at: Option<usize>,
    /// The line the last frame drew at the top, for `Esc` to put at the top of
    /// the reading view.
    top: usize,
    /// How many rows the notices took and how wide the gutter was in the last
    /// frame, so a click is measured against what was drawn — the pad's
    /// `noticed`, and its one-calculation rule.
    noticed: u16,
    gutter: u16,
    /// Where the last frame drew the caret, in pane coordinates.
    caret: Option<(u16, u16)>,
    /// The file stopped being one abeam can edit while nothing here was
    /// unsaved — deleted, renamed, grown past the cap — in the strict read's
    /// words. See [`ViewerPane::recheck`] for why the editor stays open over
    /// it rather than closing.
    gone: Option<String>,
    /// For a file `a` is creating: the document that was on the page when the
    /// name was given, which a discard puts back. `None` for a file that was
    /// opened rather than named.
    came_from: Option<PathBuf>,
}

/// The disk is not what the text was typed against.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Conflict {
    why: Why,
    /// A frame has put the sentence on screen, so the next `Ctrl+S` is made in
    /// front of it. See the module doc.
    drawn: bool,
    /// What the disk held when a save was refused: what the second `Ctrl+S` is
    /// allowed to write over, and nothing newer. See the module doc's
    /// *Saving*.
    ///
    /// `None` for the two conflicts that are not refusals yet, and for a
    /// refusal whose disk could not be held whole — gone unreadable for the
    /// moment, or grown past what abeam edits — which nothing is written over
    /// until a later press finds it readable again.
    seen: Option<Baseline>,
}

impl Conflict {
    /// A save refused against what is at `path` now.
    fn refused(path: &Path) -> Self {
        Conflict {
            why: Why::Refused,
            drawn: false,
            seen: disk_now(path),
        }
    }

    /// A change found some other way, which the next `Ctrl+S` turns into a
    /// refusal.
    fn found(why: Why) -> Self {
        Conflict {
            why,
            drawn: false,
            seen: None,
        }
    }
}

/// What the disk holds at `path` now, whole, as a baseline a save can expect —
/// or `None` when that cannot be had: the file would not open, or it is larger
/// than abeam edits, and a file abeam could not have opened is not one it
/// writes over.
fn disk_now(path: &Path) -> Option<Baseline> {
    use std::io::Read;

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Some(Baseline::Absent),
        Err(_) => return None,
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_BYTES).then(|| Baseline::Present(bytes.into()))
}

/// Which of three routes found the disk changed. Only the first is one a
/// second `Ctrl+S` can act on, which is why the other two become it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// A `Ctrl+S` was refused because the disk had changed.
    Refused,
    /// The watcher reported the file and the disk was not the baseline.
    Changed,
    /// A recovery copy was typed against a version that is no longer on disk.
    ///
    /// Kept apart from the other two for one more reason than wording: this is
    /// the conflict a disk that matches the baseline does *not* resolve,
    /// because here the baseline is the disk and the text came from somewhere
    /// older.
    Recovered,
}

/// `x x`'s first press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Question {
    /// A frame has drawn the question, so a second `x` is an answer to it.
    drawn: bool,
}

/// What the page says about the last thing that happened to the file.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Said {
    /// A save that did not happen, in `SaveError`'s words.
    Failed(String),
    /// A save that happened worse than asked, in `Saved::notice`'s words.
    Noted(String),
    /// The text came out of a recovery copy written this long ago.
    Recovered(String),
    /// The file changed on disk while nothing here was unsaved, and was read
    /// again.
    Reloaded,
}

/// The recovery copy's bookkeeping. See the module doc.
#[derive(Debug, Default)]
struct Copy {
    /// The text has changed since the copy was last brought up to date, and
    /// when. The pad's `changed`: one field for the dirty flag and the clock.
    owed: Option<Instant>,
    /// The path the copy is kept under: what the file resolved to when it was
    /// opened, for the edit's whole life. See the module doc.
    key: PathBuf,
    /// This session must not write a copy of this file, and the sentence that
    /// says why: a copy that is there and would not read, or no profile.
    blocked: Option<String>,
    /// The last write failed, in `Drafts`' words.
    failed: Option<String>,
    /// The profile holds a copy of this file that this edit answers for —
    /// offered back at `e`, or written since — and no save or discard has
    /// deleted it. Not a [`Copy::blocked`] one, which nothing here touches.
    ///
    /// What it guards is the text in that copy when the editor no longer
    /// holds it: a copy offered back and undone to look at the disk leaves the
    /// editor clean, with the recovered text only in the redo and the profile.
    /// A clean editor is reloaded when the file changes on disk, and the reload
    /// is a fresh editor with no redo — so the next keystroke's copy would have
    /// replaced the only one left. While this is set the change is a conflict
    /// instead, as it is for unsaved text; see [`ViewerPane::recheck`].
    in_profile: bool,
    /// The fingerprint of the version the text was typed against, which is
    /// what the copy records.
    ///
    /// The baseline's, almost always. The one exception is a recovery that
    /// started in conflict: that text was typed against the version the old
    /// copy names, not the one on disk now, and writing the disk's fingerprint
    /// into the next copy would let a later session open it *without* the
    /// conflict and save it over a change nobody has seen.
    against: Option<Fingerprint>,
}

/// The last save this pane made, remembered after its editor has closed.
///
/// A save from the reading view closes the editor at once, and the watcher's
/// report of that save arrives a debounce later — when there is no edit left
/// to recognise it. Without this, the pane's own write would come back as
/// news: queued as a document, the `◆` on the border of whatever view is up,
/// and the page reloaded from disk.
pub(super) struct Echo {
    requested: PathBuf,
    resolved: PathBuf,
    wrote: Baseline,
}

impl Echo {
    /// What a save that happened wrote, by both of the names the watcher may
    /// report it under.
    fn of(saved: &Saved) -> Self {
        Echo {
            requested: saved.requested.clone(),
            resolved: saved.path.clone(),
            wrote: saved.baseline.clone(),
        }
    }

    fn names(&self, path: &Path) -> bool {
        paths::same_dir(path, &self.requested) || paths::same_dir(path, &self.resolved)
    }
}

/// Where `e` puts the caret, read off the reading view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// A source layout, with this line at the top of the page.
    Line(usize),
    /// A rendered one, scrolled `at` of `of` of the way: its rows correspond to
    /// no line, so how far down the reader was is the nearest honest answer.
    Fraction { at: usize, of: usize },
}

/// What an [`Edit`] is started on, from either door: a file `disk::open` read,
/// or one `a` named that is not there yet.
struct File {
    requested: PathBuf,
    /// The resolved path, which is also the recovery copy's key for the edit's
    /// life.
    resolved: PathBuf,
    text: String,
    format: Format,
    policy: Policy,
    baseline: Baseline,
    /// The fingerprint of what the text is typed against: the file's, or
    /// `None` for a file that is not there.
    against: Option<Fingerprint>,
}

/// Where the recovery copy of `path` is kept: the path resolved, links
/// followed — or, for a path that is not there (deleted, renamed, or not
/// created yet), the deepest part of it that is, resolved, with the rest joined
/// on. One spelling for one file however it was reached, which is what makes a
/// copy findable by a later session that reached it another way.
pub(super) fn key_for(path: &Path) -> Option<PathBuf> {
    if let Ok(resolved) = paths::resolve(path) {
        return Some(resolved);
    }
    let name = path.file_name()?;
    Some(key_for(path.parent()?)?.join(name))
}

/// Which kind of thing a notice is, for the two whose being drawn is what lets
/// a key act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Says {
    Question,
    Conflict,
    Other,
}

impl Edit {
    /// An editor on `opened`, with any recovery copy of it put over the disk's
    /// text, and the caret at `place`.
    fn open(opened: Opened, drafts: Option<&Drafts>, place: Place) -> Self {
        let file = File {
            policy: opened.policy(),
            against: Some(opened.fingerprint()),
            requested: opened.requested,
            resolved: opened.path,
            text: opened.text,
            format: opened.format,
            baseline: opened.baseline,
        };
        let mut edit = Edit::start(file, drafts);
        edit.place_caret(place);
        edit
    }

    /// An empty editor on `requested`, a file that is not there yet: `a`'s.
    ///
    /// The baseline is [`Baseline::Absent`], which is the whole of what makes
    /// it a new file to everything downstream: the first save creates rather
    /// than replaces, and is refused if a file has appeared there since; the
    /// directories its name passes through are made by that save and not
    /// before, so `Esc` on an untouched one leaves nothing behind. `key` is
    /// where its recovery copy goes — see [`key_for`] — and a copy already
    /// there, from a session that crashed while naming the same file, is
    /// offered back exactly as one for an existing file is. A new file's `Tab`
    /// types two spaces: there is no indentation of its own to follow.
    fn create(
        requested: PathBuf,
        key: PathBuf,
        format: Format,
        drafts: Option<&Drafts>,
        came_from: Option<PathBuf>,
    ) -> Self {
        let file = File {
            policy: format.policy(TabKey::Spaces(2)),
            against: None,
            requested,
            resolved: key,
            text: String::new(),
            format,
            baseline: Baseline::Absent,
        };
        let mut edit = Edit::start(file, drafts);
        edit.came_from = came_from;
        edit.place_caret(Place::Line(0));
        edit
    }

    /// The editor both of the above build, with any recovery copy of the file
    /// put over its text.
    fn start(file: File, drafts: Option<&Drafts>) -> Self {
        let mut editor = Editor::from_text(file.policy, &file.text);
        let grammar = Grammar::for_file(&file.requested, &file.text);
        let mut copy = Copy {
            against: file.against,
            key: file.resolved.clone(),
            ..Copy::default()
        };
        let mut said = None;
        let mut conflict = None;
        match drafts {
            // Said before anybody types, which is the pad's `nowhere` rule:
            // telling somebody afterwards that their words were never being
            // kept is telling them too late.
            None => copy.blocked = Some(drafts::nowhere()),
            Some(drafts) => match drafts.read(&copy.key) {
                Ok(None) => {}
                // Over the disk's text as one step, so the text reads as
                // unsaved and one undo is exactly what is on disk.
                Ok(Some(recovery)) => match editor.replace_all(&recovery.text) {
                    Outcome::Edited => {
                        said = Some(Said::Recovered(ago(recovery.saved_at)));
                        copy.in_profile = true;
                        if recovery.base != file.against {
                            conflict = Some(Conflict::found(Why::Recovered));
                            copy.against = recovery.base;
                        }
                    }
                    // The copy is what is on disk: somebody saved the same text
                    // another way, so nothing is unsaved and the copy is stale.
                    // A delete that fails leaves it to be found equal again.
                    Outcome::Still | Outcome::Moved => {
                        let _ = drafts.delete(&copy.key);
                    }
                    Outcome::Refused => {
                        copy.blocked = Some(too_big_to_recover(&file.requested));
                    }
                },
                // There and unreadable: somebody's unsaved text, which a copy
                // written now would replace. `Drafts::read` has the argument.
                Err(why) => copy.blocked = Some(why),
            },
        }
        Edit {
            requested: file.requested,
            resolved: file.resolved,
            editor,
            view: View::default(),
            format: file.format,
            baseline: file.baseline,
            grammar,
            typing: true,
            conflict,
            question: None,
            held: None,
            said,
            refused: false,
            copy,
            left_at: None,
            top: 0,
            noticed: 0,
            gutter: 0,
            caret: None,
            gone: None,
            came_from: None,
        }
    }

    /// A file `a` named and nothing has saved yet: nothing of it is on disk,
    /// and closing it should leave the reader where the name was given.
    fn unborn(&self) -> bool {
        matches!(self.baseline, Baseline::Absent)
    }

    /// Put the caret at the start of the line `place` names, and that line at
    /// the top of the next frame.
    fn place_caret(&mut self, place: Place) {
        match place {
            Place::Line(line) => self.editor.set_caret(line, 0),
            Place::Fraction { at, of } => self.editor.caret_to_fraction(at, of),
        };
        let line = self.editor.caret().0;
        self.view.show_line_at_top(line);
        self.top = line;
    }

    /// Whether `path` is this file, by either of its spellings.
    fn names(&self, path: &Path) -> bool {
        paths::same_dir(path, &self.requested) || paths::same_dir(path, &self.resolved)
    }

    /// What the editor did with a key, a paste or a click, answered for the
    /// files view — the pad's `after`, with a recovery copy where the pad has
    /// an autosave.
    ///
    /// An edit that leaves the text unsaved owes the copy a write; one that
    /// lands back on the text last saved owes it nothing, and in particular not
    /// a delete — see the module doc. A refusal puts its notice up. A move
    /// brings the caret into view. What the shell is told is
    /// [`Outcome::handled`], for the reasons that type gives.
    fn after(&mut self, outcome: Outcome) -> Handled {
        match outcome {
            Outcome::Edited => {
                self.refused = false;
                // What these said is about the text as it arrived, and it is
                // not that text any more.
                if matches!(self.said, Some(Said::Recovered(_) | Said::Reloaded)) {
                    self.said = None;
                }
                if self.editor.is_modified() {
                    self.copy.owed = Some(Instant::now());
                }
                self.view.follow();
            }
            Outcome::Refused => self.refused = true,
            Outcome::Moved => self.view.follow(),
            Outcome::Still => {}
        }
        outcome.handled()
    }

    /// Bring the recovery copy up to date, once the text has been left alone
    /// for [`QUIET`]. True when what the page says about the copy changed.
    ///
    /// On the tick thread, synchronously, for the pad's reason and within the
    /// pad's bound: one file of at most the editor's cap, into the user's own
    /// profile, at most once per quiet interval.
    fn tick(&mut self, drafts: Option<&Drafts>, root: &Path) -> bool {
        match self.copy.owed {
            Some(since) if since.elapsed() >= QUIET => self.keep_copy(drafts, root),
            _ => false,
        }
    }

    /// Write the copy, if the text is unsaved. Only ever a write: a copy is
    /// deleted by a save or a discard and by nothing else. A failure is tried
    /// again after the next quiet interval, as the pad's is, and said once.
    fn keep_copy(&mut self, drafts: Option<&Drafts>, root: &Path) -> bool {
        self.copy.owed = None;
        if !self.editor.is_modified() {
            return false;
        }
        let Some(drafts) = drafts.filter(|_| self.copy.blocked.is_none()) else {
            return false;
        };
        let text = self.editor.text();
        let failed = drafts
            .write(root, &self.copy.key, &text, self.copy.against)
            .err();
        match failed {
            Some(_) => self.copy.owed = Some(Instant::now()),
            None => self.copy.in_profile = true,
        }
        let differs = failed != self.copy.failed;
        self.copy.failed = failed;
        differs
    }

    /// The text is on disk, or has been thrown away: the copy goes, whether
    /// this session wrote it or a crashed one did. Not a copy this session was
    /// told not to touch — a save does not make an unreadable copy readable.
    fn forget_copy(&mut self, drafts: Option<&Drafts>) -> Result<(), String> {
        self.copy.owed = None;
        if self.copy.blocked.is_some() {
            return Ok(());
        }
        let forgot = match drafts {
            Some(drafts) => drafts.delete(&self.copy.key),
            None => Ok(()),
        };
        if forgot.is_ok() {
            self.copy.in_profile = false;
        }
        forgot
    }

    /// The file changed on disk with nothing unsaved here: read it again, or
    /// say why it can no longer be.
    ///
    /// **Into a fresh editor**, with the caret put back where it was and the
    /// view left where it is. Not into this one as a step of undo — which is
    /// what `Editor::replace_all` offers, and was what this did — because the
    /// step can be taken back: one `Ctrl+Z` would remove the agent's change
    /// from the text and `Ctrl+S` would write that removal with no conflict to
    /// stop it, the baseline being the agent's version by then. The history
    /// before a reload is history of a file that is no longer on disk.
    fn reload(&mut self, root: &Path) -> Result<(), Refusal> {
        let opened = disk::open(&self.requested, root)?;
        let (line, col) = self.editor.caret();
        let against = opened.fingerprint();
        self.editor = Editor::from_text(opened.policy(), &opened.text);
        self.editor.set_caret(line, col);
        self.format = opened.format;
        self.baseline = opened.baseline;
        self.resolved = opened.path;
        self.copy.against = Some(against);
        self.conflict = None;
        self.gone = None;
        self.said = Some(Said::Reloaded);
        Ok(())
    }

    /// How far down the text the view is, in the reader's words.
    fn position(&self) -> String {
        percent(&self.view.scroll)
    }
}

impl ViewerPane {
    /// Is a file being typed into, right now, with the document up?
    pub(super) fn editing(&self) -> bool {
        matches!(self.mode, Mode::Doc) && self.edit.as_ref().is_some_and(|edit| edit.typing)
    }

    /// The file holding text that is on no disk, as the reader named it, or
    /// `None` when there is none.
    ///
    /// The shell's question at quit and at a worktree switch, and the one
    /// every guard in this pane asks. "Unsaved" is the editor's
    /// `is_modified` — a letter typed and rubbed out still counts, as it does
    /// in any editor that keeps a saved point.
    pub fn unsaved(&self) -> Option<&Path> {
        self.edit
            .as_ref()
            .filter(|edit| edit.editor.is_modified())
            .map(|edit| edit.requested.as_path())
    }

    /// [`ViewerPane::unsaved`], as the border names it: relative to the root.
    pub fn unsaved_name(&self) -> Option<String> {
        self.unsaved().map(|path| self.label(path))
    }

    /// May the page be replaced? `false` while it holds unsaved text, with the
    /// sentence in [`ViewerPane::refusal`] for the border.
    ///
    /// Public because two of the things that would replace it are the shell's
    /// rather than this pane's — a switch to another worktree moves the git
    /// pane and the shells as well, and has to be refused before any of them
    /// move. Everything inside this pane comes through `show`, which asks this.
    ///
    /// The sentence names the route rather than the keys, because the border
    /// it is drawn on may be the git view's: `F1, E` is the hub's and cannot
    /// reach the agent, and from there the files view's own border names what
    /// saves and what discards.
    pub fn may_replace(&mut self) -> bool {
        let Some(name) = self.unsaved_name() else {
            return true;
        };
        self.refusal = Some(format!("{name} has unsaved text: F1, E to save or discard it"));
        false
    }

    /// What the quit warning says about unsaved text: the file, and whether a
    /// copy of it will survive the quit.
    ///
    /// **Kept only when a copy is on disk and holds the text as it is now**:
    /// written since the last change by a write that worked, or offered back at
    /// `e` and not changed since. Being *allowed* to write one is not enough,
    /// and this used to say "kept" on that alone. A profile inside the
    /// workspace — abeam started in the home directory — refuses every copy,
    /// and the warning then promised the very text the second press threw
    /// away. `F1, Q` writes the copy before it asks (`crate::app`), so by the
    /// time this is drawn the answer is the answer of a write that was tried.
    pub fn quit_note(&self) -> Option<String> {
        let edit = self.edit.as_ref().filter(|edit| edit.editor.is_modified())?;
        let name = self.label(&edit.requested);
        let copy = &edit.copy;
        let kept = self.drafts.is_some()
            && copy.blocked.is_none()
            && copy.failed.is_none()
            && copy.owed.is_none();
        Some(if kept {
            format!("unsaved {name} kept as a recovery copy")
        } else {
            format!("unsaved {name} will be lost")
        })
    }

    /// Is this a path the pane wants the watcher's news of — the file being
    /// edited, or the one it last saved? Two name comparisons and nothing on
    /// disk, so the shell can ask it of every path in a batch before the
    /// dearer question of whose workspace the path is in.
    pub fn minds(&self, path: &Path) -> bool {
        self.edit.as_ref().is_some_and(|edit| edit.names(path))
            || self.echo.as_ref().is_some_and(|echo| echo.names(path))
    }

    /// Whether [`ViewerPane::minds`] can be true of anything: the shell skips
    /// the whole question when it cannot.
    pub fn minds_disk(&self) -> bool {
        self.edit.is_some() || self.echo.is_some()
    }

    /// The sentence the last refused replacement left, for the shell to draw
    /// at the front of the border, whichever view is up.
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// The shell's first word about every keystroke, paste and click, wherever
    /// it is going.
    ///
    /// Two things here are answers to the key *before* this one, and a key
    /// that never reaches this pane must still end them. The refusal and the
    /// note go. The `x x` question is set aside for this one key: if the key is
    /// this pane's, `handle_key` finds it there and may answer it; if it goes
    /// anywhere else — `F4`, a letter typed at the agent, a hub command — the
    /// next call here finds it unanswered and it is gone. That is "any other
    /// key cancels" for a question whose other keys mostly do not arrive here.
    pub fn keystroke(&mut self) {
        self.refusal = None;
        self.note = None;
        if let Some(edit) = self.edit.as_mut() {
            edit.held = edit.question.take();
        }
    }

    /// The `x x` question, taken by the key that is about to be handled: it is
    /// either answered by this key or withdrawn by it.
    pub(super) fn take_question(&mut self) -> Option<Question> {
        let edit = self.edit.as_mut()?;
        let held = edit.held.take();
        let asked = edit.question.take();
        held.or(asked)
    }

    /// A path the watcher reported. `Some` when it is this pane's to answer —
    /// the file being edited, or the file it last saved — with whether a frame
    /// is owed; `None` for anything else, which the shell then routes as it
    /// always has.
    ///
    /// `Some` is also the shell's sign *not* to queue the path as a document,
    /// and for the file being edited that is the whole point: the reader's
    /// reload of it would put the disk's text over the page.
    pub fn disk_changed(&mut self, path: &Path) -> Option<bool> {
        if self.edit.as_ref().is_some_and(|edit| edit.names(path)) {
            return Some(self.recheck());
        }
        let echo = self.echo.as_ref()?;
        if !echo.names(path) {
            return None;
        }
        match echo.wrote.is_on_disk(&echo.resolved) {
            // This pane's own write, coming back.
            Ok(true) => Some(false),
            // Somebody else's, or no telling: the reader's ordinary news, and
            // the echo has nothing left to recognise.
            Ok(false) | Err(_) => {
                self.echo = None;
                None
            }
        }
    }

    /// A batch the watcher could not keep the paths of: the file being edited
    /// may be among them, so it is looked at. Whether a frame is owed.
    pub fn disk_may_have_changed(&mut self) -> bool {
        self.edit.is_some() && self.recheck()
    }

    /// Compare the file being edited with what it was opened on or last saved
    /// as, and say what that means. See the module doc's *The watcher*.
    fn recheck(&mut self) -> bool {
        let Some(edit) = self.edit.as_mut() else {
            return false;
        };
        // Nothing of the user's is in this edit: no unsaved text, and no copy
        // in the profile that the editor has stopped holding — text a reload
        // would lose as surely, see `Copy::in_profile`. Only then is a change
        // on disk read in rather than raised as a conflict.
        let nothing_of_theirs = !edit.editor.is_modified() && !edit.copy.in_profile;
        match edit.baseline.is_on_disk(&edit.resolved) {
            // The disk is what this pane last read or wrote: its own save
            // coming back, or a change undone. A conflict about the disk
            // differing from the baseline is no longer true; one a recovery
            // raised is about an older version than the baseline, and stays.
            Ok(true) => {
                if edit.conflict.as_ref().is_some_and(|c| c.why != Why::Recovered) {
                    edit.conflict = None;
                    return true;
                }
                false
            }
            Ok(false) if nothing_of_theirs => match edit.reload(&self.root) {
                Ok(()) => true,
                // Already said, and still so: nothing new to draw.
                Err(_) if edit.gone.is_some() => false,
                // Gone, or no longer a file abeam can edit, with nothing of the
                // user's in the editor.
                //
                // **The editor stays open over it, and blocked**, rather than
                // closing. It used to close, and the next key somebody was
                // already pressing then landed in the reader instead: `Tab`
                // opened another file, `?` the ask, and `q` handed the keys to
                // the agent, so the rest of a sentence typed into a file went
                // into the agent's composer and its `Enter` sent it. A pane must
                // not change what the keys mean under somebody typing — the rule
                // the watcher has kept since the first page it ever held back.
                // So it stays the editor, keeps the keys, says why at the top,
                // and saves nothing until a save has been refused against what
                // is there now (`seen: None`); `Esc` is the way out, and shows
                // the disk as it is.
                Err(refusal) => {
                    edit.gone = Some(refusal.message());
                    edit.conflict = Some(Conflict {
                        why: Why::Refused,
                        drawn: false,
                        seen: None,
                    });
                    true
                }
            },
            // A refusal about exactly what is on disk now is still true: the
            // watcher has reported the change the refusal was made against —
            // typically a debounce after it — and replacing it with a fresh
            // conflict would make the overwrite the notice promises take a
            // third press.
            Ok(false)
                if edit.conflict.as_ref().is_some_and(|c| {
                    let seen = c.seen.as_ref();
                    c.why == Why::Refused
                        && seen.is_some_and(|seen| seen.is_on_disk(&edit.resolved).unwrap_or(false))
                }) =>
            {
                false
            }
            Ok(false) => {
                // Not drawn yet even if a conflict was already up: what the
                // reader saw described a change, and this is another one —
                // and a refusal's `seen` is now of a version that is not on
                // disk, so the next press must refuse again rather than act.
                let why = match edit.conflict.as_ref().map(|c| c.why) {
                    Some(Why::Recovered) => Why::Recovered,
                    _ => Why::Changed,
                };
                edit.conflict = Some(Conflict::found(why));
                true
            }
            // Could not tell — held open by somebody, most likely. Not an
            // answer either way; the save's own comparison is the backstop.
            Err(_) => false,
        }
    }

    /// Write the recovery copy now if one is owed: the way out, where the tick
    /// that would have written it is not going to happen.
    pub fn flush_draft(&mut self) {
        if let Some(edit) = self.edit.as_mut()
            && edit.copy.owed.is_some()
        {
            edit.keep_copy(self.drafts.as_ref(), &self.root);
        }
    }

    /// Keep recovery copies in `drafts`, for a test that must look at them.
    #[cfg(test)]
    pub(crate) fn set_drafts(&mut self, drafts: Option<Drafts>) {
        self.drafts = drafts;
    }

    /// The pane's tick, for the edit: the recovery copy's debounce.
    pub(super) fn tick_edit(&mut self) -> bool {
        match self.edit.as_mut() {
            Some(edit) => edit.tick(self.drafts.as_ref(), &self.root),
            None => false,
        }
    }

    // --- in and out -----------------------------------------------------

    /// `e`: into the file on screen, or back into its unsaved text.
    ///
    /// A file `disk::open` will not give back exactly is refused here, with its
    /// sentence above the page, and the reader goes on showing it as it always
    /// has. Declined on the empty screen, where there is nothing to edit; a
    /// file the reader could not read is asked anyway, because the strict read
    /// has its own and better answer about why.
    pub(super) fn begin_edit(&mut self) -> Handled {
        if self.edit.is_some() {
            return self.resume_edit();
        }
        let path = match &self.state {
            State::Doc(doc) => doc.path.clone(),
            State::Failed { path, .. } => path.clone(),
            State::Empty => return Handled::No,
        };
        match disk::open(&path, &self.root) {
            Ok(opened) => {
                let place = self.place();
                self.edit = Some(Edit::open(opened, self.drafts.as_ref(), place));
                // From here the edit's own baseline is the newer answer about
                // this file's echoes, and the copy it offered has been opened.
                self.echo = None;
                self.offered = None;
            }
            Err(refusal) => {
                // Refused, and a copy of unsaved text from an earlier session
                // may still be waiting for this file. It cannot be opened here,
                // so it is at least said where it is.
                let mut said = read_only(&self.label(&path), &refusal);
                if let Some(copy) = self.copy_of(&path) {
                    said.push_str(&format!(
                        " Unsaved text for it from an earlier session is in {}.",
                        copy.display()
                    ));
                }
                self.note = Some(said);
            }
        }
        Handled::Yes
    }

    /// `Enter` in the file list's name box: the name checked, and then an empty
    /// editor on a file that is not there yet, or the file it already names
    /// opened to edit — or the reason back on the box, which stays open. See
    /// `super::name`.
    pub(super) fn name_file(&mut self, dir: PathBuf, name: String) -> Handled {
        match name::check(&self.root, &dir, &name) {
            Err(why) => self.browse.refuse_name(why),
            Ok(Named::Existing(path)) => {
                self.browse.close_name();
                if self.show(path) {
                    self.mode = Mode::Doc;
                    // Laid out now, so that `e` places the caret against this
                    // file's rows and not the last one's.
                    if self.laid_out > 0 {
                        self.ensure_layout(self.laid_out);
                    }
                    self.begin_edit();
                }
            }
            Ok(Named::New { path, .. }) => {
                self.browse.close_name();
                self.create(path);
            }
        }
        Handled::Yes
    }

    /// An empty editor on `path`, which is not there yet, with the line ending
    /// its neighbours use (`super::name::line_ending`) and no mark.
    ///
    /// The page behind it is left as it was — the editor is drawn over it, and
    /// `Esc` with nothing typed goes back to the list without touching it — and
    /// remembered, so that throwing the new file's text away can put it back.
    fn create(&mut self, path: PathBuf) {
        // The list cannot be open over unsaved text, so this is a backstop; a
        // clean editor is let go as `show` lets one go.
        if !self.may_replace() {
            return;
        }
        self.drop_clean_edit();
        let key = key_for(&path).unwrap_or_else(|| path.clone());
        let format = Format {
            eol: name::line_ending(&path, &self.root),
            bom: false,
        };
        let came_from = self.path().map(Path::to_path_buf);
        self.edit = Some(Edit::create(path, key, format, self.drafts.as_ref(), came_from));
        self.echo = None;
        self.offered = None;
        self.mode = Mode::Doc;
    }

    /// Where the recovery copy of the file at `path` is, if there is one: by
    /// the key a copy is written under, which is the path resolved.
    pub(super) fn copy_of(&self, path: &Path) -> Option<PathBuf> {
        let drafts = self.drafts.as_ref()?;
        let copy = drafts.path_of(&key_for(path)?);
        copy.is_file().then_some(copy)
    }

    /// The dim line a page opened over a waiting recovery copy carries, so that
    /// the copy is found by somebody reading the file and not only by somebody
    /// who already knows to press `e`. `None` for a page with no copy.
    ///
    /// The time is the copy file's own, read off the directory entry rather
    /// than out of the copy, because this is asked every time a page is shown
    /// and the copy is asked for its text only by `e`.
    pub(super) fn offer_copy(&self, path: &Path) -> Option<String> {
        let copy = self.copy_of(path)?;
        let when = std::fs::metadata(&copy).and_then(|meta| meta.modified()).ok();
        Some(format!(
            "Unsaved text for {} from {} is kept in a recovery copy · e to resume it.",
            self.label(path),
            ago(when)
        ))
    }

    /// `e` over unsaved text.
    ///
    /// Straight back to the caret when the page has not moved since `Esc` left
    /// it, because `Esc` then `e` is somebody changing their mind and should
    /// cost nothing. Scrolled since, and the caret goes where the reader is now
    /// looking — the same rule as a fresh `e` — because that is the line they
    /// went looking for.
    fn resume_edit(&mut self) -> Handled {
        let place = self.place();
        let offset = self.scroll.offset;
        let Some(edit) = self.edit.as_mut() else {
            return Handled::No;
        };
        edit.typing = true;
        if edit.left_at == Some(offset) {
            edit.view.follow();
        } else {
            edit.place_caret(place);
        }
        Handled::Yes
    }

    /// Where `e` should put the caret: the line at the top of a source layout,
    /// or the fraction of the way down a rendered one.
    fn place(&self) -> Place {
        match self.top_line() {
            Some(line) => Place::Line(line),
            None => Place::Fraction {
                at: self.scroll.offset,
                of: self.scroll.max(),
            },
        }
    }

    /// The source line at the top of the reading view, for a source layout.
    ///
    /// The last line starting at or above the offset — and then the first line
    /// starting on that same row, which is the head of a rendered doc block
    /// rather than its tail when the offset is inside one. See
    /// `Source::line_rows`.
    fn top_line(&self) -> Option<usize> {
        let rows = &self.line_rows;
        if rows.is_empty() {
            return None;
        }
        let at = rows
            .partition_point(|&row| row <= self.scroll.offset)
            .saturating_sub(1);
        let row = rows[at];
        Some(rows.partition_point(|&r| r < row))
    }

    /// `Esc` while typing: back to the page, keeping every word.
    ///
    /// With unsaved text the page now shows it, and the edit stays to hold it.
    /// With none, the edit closes. Either way the line that was at the top of
    /// the editor goes at the top of the page, or the same fraction of the way
    /// down a rendered one — `toggle_raw`'s answer to two layouts that share no
    /// rows.
    fn stop_typing(&mut self) -> Handled {
        let Some(edit) = self.edit.as_mut() else {
            return Handled::No;
        };
        // A file `a` named, with nothing typed into it: it leaves nothing at
        // all behind — no file, no directory, no page for a file that is not
        // there — and the reader goes back to the list the name was given in.
        if edit.unborn() && !edit.editor.is_modified() {
            self.edit = None;
            self.mode = Mode::Browse;
            return Handled::Yes;
        }
        edit.typing = false;
        edit.caret = None;
        let top = edit.top;
        let fraction = (edit.view.scroll.offset, edit.view.scroll.max());
        if edit.editor.is_modified() {
            let (path, text) = (edit.requested.clone(), edit.editor.text());
            self.put_on_page(&path, text);
        } else {
            self.close_edit();
        }
        self.aim(top, fraction);
        let offset = self.scroll.offset;
        if let Some(edit) = self.edit.as_mut() {
            edit.left_at = Some(offset);
        }
        Handled::Yes
    }

    /// Close an edit with nothing unsaved in it, leaving the page showing what
    /// is on disk — the editor's text, which is what it last read or saved,
    /// unless the disk has changed since, and then the disk's.
    ///
    /// Refuses to close one that *has* unsaved text. Every caller has asked
    /// first; this is the line that makes forgetting to ask a page left open
    /// rather than a paragraph lost.
    fn close_edit(&mut self) {
        let Some(edit) = self.drop_clean_edit() else {
            return;
        };
        // Never saved: nothing of it is on disk, and nothing goes on the page.
        if edit.unborn() {
            return;
        }
        if edit.conflict.is_some() {
            self.show(edit.requested);
        } else {
            self.put_on_page(&edit.requested, edit.editor.text());
        }
    }

    /// Let go of an edit with nothing unsaved in it, and hand it back; keep
    /// one that has, and hand back nothing — which is what makes a caller that
    /// forgot to ask [`ViewerPane::may_replace`] a page left open rather than a
    /// paragraph lost.
    ///
    /// Its recovery copy, if it has one, stays. The text in it may have come
    /// out of a crash and been undone here only to be looked at, and the next
    /// `e` offering it again is the cheap side of that trade: see the module
    /// doc.
    pub(super) fn drop_clean_edit(&mut self) -> Option<Edit> {
        let edit = self.edit.take()?;
        if edit.editor.is_modified() {
            self.edit = Some(edit);
            return None;
        }
        Some(edit)
    }

    /// Leave the page for the file list, if it may be left: refused over
    /// unsaved text, and a clean edit closed on the way.
    pub(super) fn leave_page(&mut self) -> bool {
        if !self.may_replace() {
            return false;
        }
        self.close_edit();
        true
    }

    /// Make `text` the page's, as the document at `path`, unless it already
    /// is. See the module doc for why the reading view is an ordinary [`Doc`].
    fn put_on_page(&mut self, path: &Path, text: String) {
        if let State::Doc(doc) = &self.state
            && doc.path == path
            && doc.text() == text
        {
            return;
        }
        let bytes = text.len() as u64;
        self.state = State::Doc(Doc::of(path.to_path_buf(), text, false, bytes));
        self.dirty = true;
    }

    /// Put `line` at the top of the reading view, or `at` of `of` of the way
    /// down a page that has no lines. Laid out now rather than at the next
    /// frame, for `toggle_raw`'s reason: the place has to be found in the rows
    /// the page is about to have.
    fn aim(&mut self, line: usize, (at, of): (usize, usize)) {
        if self.laid_out == 0 {
            return;
        }
        self.ensure_layout(self.laid_out);
        self.scroll.measure(self.lines.len(), self.scroll.viewport());
        let to = match self.line_rows.get(line) {
            Some(&row) => row,
            // A rendered page.
            None if self.line_rows.is_empty() => {
                at.saturating_mul(self.scroll.max()).checked_div(of).unwrap_or(0)
            }
            // The editor's last line, past the end of what the reader numbers:
            // the end.
            None => usize::MAX,
        };
        self.scroll.to(to);
    }

    // --- keys -----------------------------------------------------------

    /// Every key while typing.
    ///
    /// The editor's first — every printable key, `Enter`, `Tab`, the arrows,
    /// `Ctrl+Z`/`Ctrl+Y` — and what it hands back is this pane's: `Ctrl+S`
    /// saves, `Esc` goes back to reading, and `PgUp`/`PgDn` page the view and
    /// leave the caret, the pad's answer for the two scroll keys that are not
    /// letters. Everything else is declined and does nothing, `Ctrl` chords
    /// included; `Esc` and `q` cannot be among them, so the shell's way out
    /// never fires from here.
    pub(super) fn edit_key(&mut self, key: KeyEvent) -> Handled {
        let Some(edit) = self.edit.as_mut() else {
            return Handled::No;
        };
        if let Some(outcome) = edit.editor.key(&key) {
            return edit.after(outcome);
        }
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('s' | 'S') if crate::keys::ctrl_chord(&key) => self.save(),
            KeyCode::Esc if plain => self.stop_typing(),
            KeyCode::PageUp | KeyCode::PageDown => edit.view.glance(key),
            _ => Handled::No,
        }
    }

    /// A glance binding while typing: the page moves and the caret does not.
    pub(super) fn edit_glance(&mut self, key: KeyEvent) -> Handled {
        match self.edit.as_mut() {
            Some(edit) => edit.view.glance(key),
            None => Handled::No,
        }
    }

    /// A paste while typing: into the text, whole, as one step of undo.
    pub(super) fn edit_paste(&mut self, text: &str) -> Handled {
        match self.edit.as_mut() {
            Some(edit) => {
                let outcome = edit.editor.paste(text);
                edit.after(outcome)
            }
            None => Handled::No,
        }
    }

    /// The pointer while typing: the wheel scrolls, and a click puts the
    /// caret where it landed — on a line number, at the start of that line.
    ///
    /// Measured against the last frame's notices and gutter, the numbers the
    /// same frame drew them with. The two clicks the editor declines — against
    /// a stale layout, and on the column kept back for the bar — come back
    /// declined, and the shell makes a selection of them, as it does in the
    /// pad.
    pub(super) fn edit_mouse(&mut self, ev: &MouseEvent) -> Handled {
        let Some(edit) = self.edit.as_mut() else {
            return Handled::No;
        };
        if let Some(handled) = edit.view.wheel(ev) {
            return handled;
        }
        if !matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) {
            return Handled::No;
        }
        let Some(row) = ev.row.checked_sub(edit.noticed) else {
            return Handled::No;
        };
        let col = usize::from(ev.column).saturating_sub(usize::from(edit.gutter));
        match edit.view.click(&mut edit.editor, usize::from(row), col) {
            Some(true) => edit.after(Outcome::Moved),
            Some(false) | None => Handled::No,
        }
    }

    /// `Ctrl+S`, while typing or while reading unsaved text.
    ///
    /// A save with nothing unsaved is nothing, conflict or not, and is
    /// declined. In a conflict, the press is one of three things, and none of
    /// them is a save that expects nothing (see the module doc's *Saving*):
    ///
    /// - a conflict that is not a refusal yet becomes one, against what is on
    ///   disk now, and nothing is written;
    /// - a refusal not yet drawn waits for the frame this press asks for;
    /// - a drawn refusal saves expecting exactly what it saw, so that anything
    ///   written since is refused in its turn.
    ///
    /// The undo step being typed ends either way, for the pad's reason.
    pub(super) fn save(&mut self) -> Handled {
        let Some(name) = self.edit.as_ref().map(|edit| self.label(&edit.requested)) else {
            return Handled::No;
        };
        let Some(edit) = self.edit.as_mut() else {
            return Handled::No;
        };
        edit.editor.end_step();
        if !edit.editor.is_modified() {
            return Handled::No;
        }
        let expect = match &edit.conflict {
            None => Some(edit.baseline.clone()),
            Some(Conflict {
                why: Why::Refused,
                drawn: false,
                ..
            }) => return Handled::Yes,
            // `None` when the refusal could not hold the disk: refused again,
            // and asked again what is there.
            Some(Conflict {
                why: Why::Refused,
                seen,
                ..
            }) => seen.clone(),
            Some(_) => None,
        };
        let Some(expect) = expect else {
            edit.conflict = Some(Conflict::refused(&edit.resolved));
            edit.gone = None;
            return Handled::Yes;
        };
        let bytes = edit.format.encode(&edit.editor.text());
        // A file `a` named makes the directories its name passes through, at
        // its first save and only then; a file that was opened never does — a
        // directory that vanished under it is a change, not something to put
        // back quietly.
        let born = edit.unborn();
        let options = Options {
            force: false,
            create_dirs: born,
        };
        edit.said = None;
        match disk::save(&edit.requested, &self.root, &bytes, &expect, options) {
            Ok(saved) => {
                // What was written, for the watcher's report of it — see
                // [`Echo`]. While the edit lives its baseline answers the same
                // question; this outlives it.
                self.echo = Some(Echo::of(&saved));
                edit.editor.mark_saved();
                edit.baseline = saved.baseline;
                edit.resolved = saved.path;
                edit.conflict = None;
                edit.copy.against = edit.baseline.fingerprint();
                let noted = saved
                    .notice
                    .map(|notice| notice.message(Path::new(&name)));
                let forgot = edit.forget_copy(self.drafts.as_ref()).err();
                if edit.typing {
                    edit.said = noted.map(Said::Noted);
                    edit.copy.failed = forgot;
                } else {
                    // Saved from the reading view: nothing is unsaved and
                    // nobody is typing, and the page already shows what was
                    // written, so the edit closes. What the save had to say
                    // goes above the page instead of going with it.
                    self.close_edit();
                    let said: Vec<String> = noted.into_iter().chain(forgot).collect();
                    if !said.is_empty() {
                        self.note = Some(said.join(" "));
                    }
                }
                // A file that did not exist does now: the list shows it at
                // once, and the walk that feeds the find, `Tab` and `f` is
                // started again so they know it too. The watcher tells the git
                // view.
                if born {
                    self.browse.relist();
                    self.rescan();
                }
            }
            Err(SaveError::Changed) => {
                edit.conflict = Some(Conflict::refused(&edit.resolved));
                edit.gone = None;
            }
            Err(why) => edit.said = Some(Said::Failed(why.message(Path::new(&name)))),
        }
        Handled::Yes
    }

    /// `x` over unsaved text: the question, or — when a frame has drawn the
    /// question and nothing else was pressed since — the answer.
    ///
    /// `asked` is what the key found standing when it arrived; see
    /// [`ViewerPane::keystroke`] for how a key that went elsewhere withdraws
    /// it. An `x` arriving before the question was drawn is the question
    /// again rather than the answer, which is `crate::app`'s `close_drawn` rule
    /// and for its reason: two `x`es in one input batch must not throw away a
    /// paragraph the question was never on screen for.
    pub(super) fn discard_key(&mut self, asked: Option<Question>) -> Handled {
        match asked {
            Some(Question { drawn: true }) => self.discard(),
            Some(Question { drawn: false }) | None => {
                if let Some(edit) = self.edit.as_mut() {
                    edit.question = Some(Question { drawn: false });
                }
            }
        }
        Handled::Yes
    }

    /// Throw the unsaved text away: the recovery copy goes, and the page shows
    /// the file as it is on disk, at the place the reader was.
    fn discard(&mut self) {
        let Some(mut edit) = self.edit.take() else {
            return;
        };
        if let Err(why) = edit.forget_copy(self.drafts.as_ref()) {
            self.note = Some(why);
        }
        // Never on disk: there is no file to show as it is, so the page goes
        // back to what it was when the name was given, and the reader to the
        // list it was given in.
        if edit.unborn() {
            match edit.came_from {
                Some(path) => {
                    self.show(path);
                }
                None => {
                    self.state = State::Empty;
                    self.dirty = true;
                }
            }
            self.mode = Mode::Browse;
            return;
        }
        self.show(edit.requested);
    }

    // --- drawing --------------------------------------------------------

    /// What the page has to say above the text, in the order a reader needs
    /// it: the question that is waiting for a key, the conflict that changes
    /// what `Ctrl+S` does, then what went wrong, then what is merely so.
    fn notices(&self, width: usize) -> Vec<(Says, Vec<Line<'static>>)> {
        let t = self.theme.theme();
        let warn = Style::new().fg(t.warn).add_modifier(Modifier::BOLD);
        let bad = Style::new().fg(t.danger).add_modifier(Modifier::BOLD);
        let dim = t.dim();
        let mut out = Vec::new();
        // First, because a second `x` acts only once every row of it is drawn.
        if let Some(edit) = &self.edit
            && !edit.typing
            && edit.question.is_some()
        {
            let name = self.label(&edit.requested);
            out.push((Says::Question, block(&question(&name), width, warn)));
        }
        if let Some(note) = &self.note {
            out.push((Says::Other, block(note, width, bad)));
        }
        let Some(edit) = &self.edit else {
            if let Some(offer) = &self.offered {
                out.push((Says::Other, block(offer, width, dim)));
            }
            return out;
        };
        let name = self.label(&edit.requested);
        if let Some(conflict) = &edit.conflict {
            let said = match &edit.gone {
                Some(why) => gone(&name, why),
                None => changed(conflict, &name),
            };
            out.push((Says::Conflict, block(&said, width, bad)));
        }
        match &edit.said {
            Some(Said::Failed(why) | Said::Noted(why)) => {
                out.push((Says::Other, block(why, width, bad)));
            }
            Some(Said::Recovered(when)) => {
                out.push((Says::Other, block(&recovered(when), width, dim)));
            }
            Some(Said::Reloaded) => out.push((Says::Other, block(&reloaded(&name), width, dim))),
            None => {}
        }
        if let Some(why) = edit.copy.blocked.as_ref().or(edit.copy.failed.as_ref()) {
            out.push((Says::Other, block(why, width, bad)));
        }
        // Only while typing, because they are about what typing can do.
        if edit.typing {
            if edit.grammar.is_some() && edit.editor.past_highlight_cap() {
                out.push((Says::Other, block(&plain(&name), width, dim)));
            }
            // One or the other, never both, for the pad's reason.
            if edit.editor.is_full() {
                out.push((Says::Other, block(&full(&name), width, bad)));
            } else if edit.refused {
                out.push((Says::Other, block(&would_not_fit(), width, bad)));
            }
        }
        out
    }

    /// Draw the notices across the top of `inner` and hand back what is left
    /// for the page, always at least one row of it.
    ///
    /// **This is where a question and a conflict become answerable.** Each is
    /// marked drawn only when every row of its sentence fitted, so a pane too
    /// short to say it is a pane in which the second key still asks rather
    /// than acts.
    pub(super) fn draw_notices(&mut self, f: &mut Frame, inner: Rect) -> Rect {
        let notices = self.notices(usize::from(inner.width));
        let room = usize::from(inner.height.saturating_sub(1));
        let mut lines = Vec::new();
        let (mut question, mut conflict) = (false, false);
        for (says, rows) in notices {
            let fits = lines.len() + rows.len() <= room;
            match says {
                Says::Question => question = fits,
                Says::Conflict => conflict = fits,
                Says::Other => {}
            }
            lines.extend(rows);
        }
        lines.truncate(room);
        let rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        if rows > 0 {
            f.render_widget(
                Paragraph::new(lines),
                Rect {
                    height: rows,
                    ..inner
                },
            );
        }
        if let Some(edit) = self.edit.as_mut() {
            if question && let Some(asked) = edit.question.as_mut() {
                asked.drawn = true;
            }
            if conflict && let Some(standing) = edit.conflict.as_mut() {
                standing.drawn = true;
            }
            edit.noticed = rows;
        }
        Rect {
            y: inner.y + rows,
            height: inner.height - rows,
            ..inner
        }
    }

    /// One frame of the file being typed into: the notices, then the editor's
    /// rows from the offset — only the rows that fit — with the reader's gutter
    /// down the left and the bar down the right.
    pub(super) fn render_edit(&mut self, f: &mut Frame, inner: Rect) {
        let area = self.draw_notices(f, inner);
        let mode = self.theme;
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        edit.caret = None;
        // The scrollbar's column is kept back whether or not the bar is drawn,
        // for the reader's reason: deciding per frame would re-wrap the whole
        // text the moment it crossed the pane's height.
        let width = usize::from(area.width.saturating_sub(scroll::bar_width(area.width)));
        let gutter = Gutter::new(edit.editor.line_count(), width, mode);
        let text = width.saturating_sub(gutter.width());
        if text == 0 || area.height == 0 {
            // Not measured, for the pad's reason: a viewport of nothing would
            // scroll the text to the top on the way through a drag.
            return;
        }
        edit.gutter = u16::try_from(gutter.width()).unwrap_or(u16::MAX);
        edit.editor.lay_out(
            text,
            Look {
                mode,
                grammar: edit.grammar,
            },
        );
        let rows = edit.view.frame(&mut edit.editor, usize::from(area.height));
        if let Some(first) = rows.first() {
            edit.top = first.line;
        }
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .map(|row| {
                let prefix = if row.first {
                    gutter.numbered(row.line + 1)
                } else {
                    gutter.blank()
                };
                let mut line = row.text;
                line.spans.splice(0..0, prefix);
                line
            })
            .collect();
        f.render_widget(
            Paragraph::new(lines),
            Rect {
                width: u16::try_from(width).unwrap_or(u16::MAX),
                ..area
            },
        );
        edit.view.scroll.render_bar(f, area);
        if let Some((col, row)) = edit.view.cursor() {
            let col = u16::try_from(gutter.width() + col).unwrap_or(u16::MAX);
            let row = (area.y - inner.y) + u16::try_from(row).unwrap_or(u16::MAX);
            edit.caret = Some((col, row));
        }
    }

    /// The caret, while typing.
    pub(super) fn edit_cursor(&self) -> Option<(u16, u16)> {
        if !self.editing() {
            return None;
        }
        self.edit.as_ref().and_then(|edit| edit.caret)
    }

    // --- the border -----------------------------------------------------

    /// The whole title while typing: `●` and the file, `editing`, the conflict
    /// if there is one, and the position — the reader's ranking, what is given
    /// up last first. The dot is in front of the name rather than a word after
    /// it, because a word after it is the first thing a narrow border loses.
    pub(super) fn edit_title(&self) -> Option<String> {
        if !self.editing() {
            return None;
        }
        let edit = self.edit.as_ref()?;
        let mark = if self.pending.is_some() { "◆ " } else { "" };
        let new = if edit.unborn() { " · new" } else { "" };
        Some(format!(
            "{mark}{}{} · editing{new}{} · {}",
            self.dot(),
            self.label(&edit.requested),
            self.diamond(),
            edit.position()
        ))
    }

    /// `● ` in front of the name of a file with unsaved text, in either state.
    pub(super) fn dot(&self) -> &'static str {
        if self.unsaved().is_some() { "● " } else { "" }
    }

    /// ` · ◆ changed on disk`, while the disk is not what the text was typed
    /// against.
    pub(super) fn diamond(&self) -> &'static str {
        match &self.edit {
            Some(edit) if edit.conflict.is_some() => " · ◆ changed on disk",
            _ => "",
        }
    }

    /// `esc→done` while typing — the press keeps the text — and
    /// `esc→new file` in the one state where it releases a waiting document.
    pub(super) fn edit_exit_hint(&self) -> Option<&'static str> {
        if !self.editing() {
            return None;
        }
        let edit = self.edit.as_ref()?;
        Some(match (edit.editor.is_modified(), edit.unborn()) {
            // Nothing typed into a file that is not there: back to the list.
            (false, true) => "esc→list",
            (false, false) if self.pending.is_some() => "esc→new file",
            _ => "esc→done",
        })
    }

    /// The keys that act on the file, for the shell to draw while the pane has
    /// focus and only then. See `Pane::action_hint` on the viewer for why the
    /// condition is the point.
    ///
    /// While typing: what `Ctrl+S` will do, behind `◆` in a conflict — and
    /// `again overwrites` only once a save has been refused, which is the only
    /// state in which a press does. While reading unsaved text: the state
    /// first, then the three keys there are; or, once `x` has been pressed, the
    /// one that finishes it. The shell draws all of it after the way out.
    pub(super) fn edit_action_hint(&self) -> Option<&'static str> {
        if !matches!(self.mode, Mode::Doc) {
            return None;
        }
        let edit = self.edit.as_ref()?;
        let why = edit.conflict.as_ref().map(|conflict| conflict.why);
        // `again overwrites` only for a refusal that saw something it may
        // write over; one that could not hold the disk refuses again instead.
        let overwrites = edit.conflict.as_ref().is_some_and(|c| c.seen.is_some());
        if edit.typing {
            return Some(match why {
                _ if edit.gone.is_some() && !edit.editor.is_modified() => "◆ no longer editable",
                Some(Why::Refused) if overwrites => "◆ ctrl+s again overwrites",
                Some(_) => "◆ ctrl+s save",
                None => "ctrl+s save",
            });
        }
        // A search box over the unsaved page: every letter is the query's.
        if self.typing() {
            return None;
        }
        Some(match (edit.question, why) {
            (Some(_), _) => "x again discards",
            (None, Some(Why::Refused)) if overwrites => {
                "● unsaved · ◆ ctrl+s again overwrites · e edit · x x discard"
            }
            (None, Some(_)) => "● unsaved · ◆ ctrl+s save · e edit · x x discard",
            (None, None) => "● unsaved · ctrl+s save · e edit · x x discard",
        })
    }
}

// ---------------------------------------------------------------------------
// what the page says
// ---------------------------------------------------------------------------
//
// No key that could reach the agent is named in any of these, because they are
// drawn whether or not the pane has the keys: `ctrl+s` with the agent focused is
// the agent's. The keys are on the border, which shows them only to a focused
// pane. The one exception is `x`, in the question, which is only ever on screen
// in the instant after `x` was pressed here.

/// Why `e` left the file read-only, in the strict read's words.
fn read_only(name: &str, refusal: &Refusal) -> String {
    format!("{name} is read-only here. {}", refusal.message())
}

/// The question `x` asks.
fn question(name: &str) -> String {
    format!(
        "Press x again to throw away what you typed into {name} and show the file as it is on \
         disk. Any other key keeps it."
    )
}

/// The conflict, in the words that fit how it was found and what a second
/// press would do.
fn changed(conflict: &Conflict, name: &str) -> String {
    match (conflict.why, &conflict.seen) {
        (Why::Refused, Some(Baseline::Absent)) => format!(
            "Not saved: {name} has been deleted since it was read. Saving again puts it back with \
             this text."
        ),
        (Why::Refused, Some(Baseline::Present(_))) => {
            let refused = SaveError::Changed.message(Path::new(name));
            format!("{refused} Saving again writes over that version, and over no newer one.")
        }
        (Why::Refused, None) => format!(
            "Not saved: {name} has changed on disk since it was read, and it is now larger than \
             abeam edits or will not open, so abeam will not write over it."
        ),
        (Why::Changed, _) => format!(
            "{name} has changed on disk since it was opened here. Saving will be refused once, \
             and only a second save writes over that change."
        ),
        (Why::Recovered, _) => format!(
            "{name} has changed on disk since this text was typed. Saving will be refused once, \
             and only a second save writes over that change."
        ),
    }
}

/// The file stopped being one abeam can edit under an editor with nothing
/// unsaved in it. No key named, for this section's reason.
fn gone(name: &str, why: &str) -> String {
    format!(
        "{name} changed on disk into a file abeam cannot edit. {why} Nothing here will be \
         written over it; leaving the editor shows it as it now is."
    )
}

fn recovered(when: &str) -> String {
    format!(
        "Recovered unsaved text from {when}. One undo goes back to the file as it is on disk."
    )
}

fn reloaded(name: &str) -> String {
    format!("{name} changed on disk while nothing here was unsaved, so it has been read again.")
}

fn plain(name: &str) -> String {
    format!(
        "Drawn without colour: {name} is larger than the {} KiB abeam highlights.",
        source::HIGHLIGHT_MAX_BYTES / 1024
    )
}

fn full(name: &str) -> String {
    format!(
        "{name} is at the {} KiB abeam will edit and will take nothing more.",
        crate::panes::viewer::load::MAX_BYTES / 1024
    )
}

/// The pad's `would_not_fit`, for a file.
fn would_not_fit() -> String {
    format!(
        "That would not fit. abeam edits files up to {} KiB, and what will not fit is turned \
         away whole rather than trimmed to the room left, so nothing was added.",
        crate::panes::viewer::load::MAX_BYTES / 1024
    )
}

fn too_big_to_recover(path: &Path) -> String {
    format!(
        "abeam found a recovery copy of {} that is larger than it will edit. It has been left \
         where it is.",
        path.display()
    )
}

/// How long ago `at` was, in the words a person uses: near enough to read at a
/// glance, which is all a notice about a recovered copy needs.
fn ago(at: Option<SystemTime>) -> String {
    let Some(at) = at else {
        return "an earlier session".to_string();
    };
    // A copy from the future is a clock that moved; it was written a moment
    // ago as far as anybody can tell.
    let secs = SystemTime::now()
        .duration_since(at)
        .map_or(0, |since| since.as_secs());
    match secs {
        0..90 => "a moment ago".to_string(),
        90..5_400 => format!("{} minutes ago", (secs + 30) / 60),
        5_400..129_600 => format!("{} hours ago", (secs + 1_800) / 3_600),
        _ => format!("{} days ago", (secs + 43_200) / 86_400),
    }
}

/// How far down a scroll is, as the reader's title says it. Shared with the
/// reader so the two titles cannot say it differently.
pub(super) fn percent(scroll: &Scroll) -> String {
    let max = scroll.max();
    if max == 0 {
        return "all".into();
    }
    if scroll.offset >= max {
        return "end".into();
    }
    format!("{}%", scroll.offset * 100 / max)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::Pane;
    use crate::testutil::TempDir;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Duration;

    /// A repository and a profile side by side in one fixture, and neither
    /// inside the other — the arrangement `Drafts::write` insists on, and the
    /// one every ordinary machine has.
    struct Fx {
        dir: TempDir,
    }

    impl Fx {
        fn new(tag: &str) -> Self {
            let dir = TempDir::new(tag);
            std::fs::create_dir_all(dir.path().join("repo")).expect("a repository");
            Fx { dir }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("repo")
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.root().join(name);
            std::fs::write(&path, bytes).expect("write a file into the repository");
            path
        }

        fn drafts(&self) -> Drafts {
            Drafts::at(self.dir.path().join("profile").join("drafts"))
        }

        /// A pane on the repository showing `path`, with no walk racing the
        /// test, and its recovery copies kept in this fixture's profile rather
        /// than in the profile of whoever runs the suite.
        fn pane(&self, path: &Path) -> ViewerPane {
            let mut pane = ViewerPane::new(self.root());
            pane.scan = None;
            pane.set_drafts(Some(self.drafts()));
            assert!(pane.show(path), "nothing unsaved, so nothing refused");
            pane
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// One key, the way the shell delivers it: [`ViewerPane::keystroke`] first,
    /// because the shell calls it for every key wherever it goes.
    fn press(pane: &mut ViewerPane, code: KeyCode) -> Handled {
        pane.keystroke();
        pane.handle_key(key(code)).expect("a key")
    }

    fn chord(pane: &mut ViewerPane, c: char) -> Handled {
        pane.keystroke();
        pane.handle_key(ctrl(c)).expect("a chord")
    }

    fn type_in(pane: &mut ViewerPane, text: &str) {
        for c in text.chars() {
            assert_eq!(press(pane, KeyCode::Char(c)), Handled::Yes, "{c:?} was not typed");
        }
    }

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// One frame, as the rows a reader would see.
    fn draw(pane: &mut ViewerPane, w: u16, h: u16) -> Vec<String> {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("a test terminal");
        term.draw(|f| pane.render(f, Rect::new(0, 0, w, h)))
            .expect("draw the pane");
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .filter_map(|x| buf.cell((x, y)).map(ratatui::buffer::Cell::symbol))
                    .collect()
            })
            .collect()
    }

    /// The same frame as one run of words, so that a sentence the pane wrapped
    /// is still found whole wherever the wrap fell.
    fn page(pane: &mut ViewerPane, w: u16, h: u16) -> String {
        draw(pane, w, h)
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn edit(pane: &ViewerPane) -> &Edit {
        pane.edit.as_ref().expect("an edit")
    }

    fn text(pane: &ViewerPane) -> String {
        edit(pane).editor.text()
    }

    /// `line 1` to `line n`, one a line, with a final newline.
    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    /// The tick's quiet interval, already over.
    fn quiet_is_over(pane: &mut ViewerPane) {
        if let Some(edit) = pane.edit.as_mut() {
            edit.copy.owed = Instant::now().checked_sub(QUIET * 2);
        }
    }

    // --- e ----------------------------------------------------------------

    #[test]
    fn e_from_the_source_view_puts_the_caret_on_the_top_line_and_that_line_at_the_top() {
        let fx = Fx::new("edit-e-source");
        let path = fx.write("notes.txt", numbered(100).as_bytes());
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        assert!(!pane.line_rows.is_empty(), "a source layout numbers its lines");
        pane.scroll.to(30);

        assert_eq!(press(&mut pane, KeyCode::Char('e')), Handled::Yes);
        assert!(pane.editing());
        assert!(pane.takes_input(), "every letter is text now");
        assert_eq!(edit(&pane).editor.caret(), (30, 0));
        let rows = draw(&mut pane, 40, 10);
        assert!(rows[0].starts_with(" 31 line 31"), "{rows:#?}");
        assert_eq!(pane.cursor(), Some((4, 0)), "after the gutter, on the top row");
    }

    #[test]
    fn e_from_a_rendered_page_puts_the_caret_the_same_fraction_of_the_way_down() {
        let fx = Fx::new("edit-e-rendered");
        let doc: String = (1..=60).map(|i| format!("Paragraph {i}.\n\n")).collect();
        let path = fx.write("notes.md", doc.as_bytes());
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        assert!(pane.line_rows.is_empty(), "a rendered page has no line to name");
        let half = pane.scroll.max() / 2;
        pane.scroll.to(half);
        let (at, of) = (pane.scroll.offset, pane.scroll.max());

        press(&mut pane, KeyCode::Char('e'));
        let line = at * edit(&pane).editor.line_count() / of;
        assert_eq!(edit(&pane).editor.caret(), (line, 0));
        draw(&mut pane, 40, 10);
        assert_eq!(edit(&pane).top, line, "and that line is the top one drawn");
        assert_eq!(pane.cursor(), Some((4, 0)));
    }

    #[test]
    fn e_from_inside_a_rendered_doc_comment_starts_at_the_head_of_it() {
        // A doc block is reflowed, so none of its lines has a row of its own;
        // each is given the row the block starts on, and the top line of a
        // page scrolled into one is the block's first.
        let fx = Fx::new("edit-e-doc");
        let path = fx.write("lib.rs", b"/// One.\n/// Two.\n/// Three.\nfn a() {}\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        let rows = &pane.line_rows;
        assert_eq!(rows[0], rows[1]);
        assert_eq!(rows[1], rows[2]);
        assert!(rows[3] > rows[2], "{rows:?}");
        assert_eq!(pane.top_line(), Some(0));
    }

    #[test]
    fn a_file_the_strict_read_refuses_stays_read_only_with_the_reason_above_it() {
        let fx = Fx::new("edit-refused");
        let path = fx.write("mixed.txt", b"one\r\ntwo\nthree\r\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 10);

        assert_eq!(press(&mut pane, KeyCode::Char('e')), Handled::Yes, "a frame for the reason");
        assert!(pane.edit.is_none());
        assert!(!pane.takes_input());
        let shown = page(&mut pane, 60, 10);
        assert!(shown.contains("mixed.txt is read-only here."), "{shown}");
        assert!(shown.contains("Mixed line endings"), "{shown}");
        assert!(shown.contains("one"), "and the file is still on the page: {shown}");

        // The next key takes it down.
        press(&mut pane, KeyCode::Char('j'));
        assert!(!page(&mut pane, 60, 10).contains("Mixed line endings"));

        // And there is nothing to edit on the empty screen at all.
        let mut empty = ViewerPane::new(fx.root());
        empty.scan = None;
        assert_eq!(empty.handle_key(key(KeyCode::Char('e'))).unwrap(), Handled::No);
    }

    // --- typing, and Esc ----------------------------------------------------

    #[test]
    fn every_letter_is_text_while_editing_and_the_border_says_how_out() {
        let fx = Fx::new("edit-letters");
        let path = fx.write("plan.md", b"\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));

        // Every one of these is a key somewhere else in this pane, or the way
        // out of it, and every one of them is a letter here.
        type_in(&mut pane, "qjgGfotnrex/?");
        assert_eq!(text(&pane), "qjgGfotnrex/?\n");
        assert!(pane.editing(), "nothing typed left the editor");
        assert_eq!(pane.exit_hint(), "esc→done");
        assert_eq!(pane.action_hint(), Some("ctrl+s save"));
        assert!(pane.title().starts_with("● plan.md · editing"), "{}", pane.title());
        // A chord that is not the editor's or the pane's does nothing at all,
        // and is declined rather than typed.
        assert_eq!(chord(&mut pane, 'd'), Handled::No);
        assert_eq!(text(&pane), "qjgGfotnrex/?\n");
    }

    #[test]
    fn esc_goes_back_to_reading_and_keeps_every_word_in_both_forms() {
        let fx = Fx::new("edit-esc");
        let disk = b"# Plan\n\nDo the thing.\n";
        let path = fx.write("plan.md", disk);
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        for code in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
            press(&mut pane, code);
        }
        type_in(&mut pane, " Now.");

        assert_eq!(press(&mut pane, KeyCode::Esc), Handled::Yes, "claimed: focus stays here");
        assert!(!pane.editing());
        assert!(!pane.takes_input());
        assert_eq!(pane.unsaved(), Some(path.as_path()));
        let rendered = page(&mut pane, 40, 10);
        assert!(rendered.contains("Do the thing. Now."), "{rendered}");
        assert!(!rendered.contains("# Plan"), "rendered, as the reader had it");
        assert!(pane.title().starts_with("● plan.md · rendered"), "{}", pane.title());
        assert_eq!(std::fs::read(&path).unwrap(), disk, "and nothing reached the disk");

        // `t` shows the source, which is the unsaved text as typed.
        press(&mut pane, KeyCode::Char('t'));
        let source = page(&mut pane, 40, 10);
        assert!(source.contains("# Plan"), "{source}");
        assert!(source.contains("Do the thing. Now."), "{source}");

        // `e` goes back to it, with the caret where it was left.
        press(&mut pane, KeyCode::Char('e'));
        assert!(pane.editing());
        type_in(&mut pane, "!");
        assert_eq!(text(&pane), "# Plan\n\nDo the thing. Now.!\n");
    }

    #[test]
    fn esc_from_an_editor_with_nothing_unsaved_closes_it() {
        let fx = Fx::new("edit-esc-clean");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        // A letter typed and rubbed out is still unsaved, as in any editor
        // with a saved point — but an undo back to it is not.
        type_in(&mut pane, "x");
        chord(&mut pane, 'z');
        assert!(pane.unsaved().is_none());
        press(&mut pane, KeyCode::Esc);
        assert!(pane.edit.is_none(), "a caret is all there was to lose");
        assert!(pane.action_hint().is_none());
    }

    #[test]
    fn esc_puts_the_editor_s_top_line_at_the_top_of_the_page() {
        let fx = Fx::new("edit-esc-aim");
        let path = fx.write("notes.txt", numbered(100).as_bytes());
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        draw(&mut pane, 40, 10);
        for _ in 0..4 {
            pane.scroll_key(key(KeyCode::PageDown)).unwrap();
        }
        draw(&mut pane, 40, 10);
        let top = edit(&pane).top;
        assert!(top > 0, "the glances moved the editor");

        press(&mut pane, KeyCode::Esc);
        assert_eq!(pane.top_line(), Some(top));
    }

    #[test]
    fn ctrl_z_belongs_to_the_editor_and_not_to_the_reading_view() {
        let fx = Fx::new("edit-undo-reading");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "ab");
        press(&mut pane, KeyCode::Esc);
        assert_eq!(chord(&mut pane, 'z'), Handled::No, "nothing to type into, so no undo");
        assert_eq!(text(&pane), "ab# Plan\n");
    }

    // --- the border ---------------------------------------------------------

    #[test]
    fn ctrl_s_is_named_only_on_the_focused_border_and_never_in_the_title() {
        // The shell draws `action_hint` only while the pane has the keys. With
        // the agent focused `Ctrl+S` is the agent's — Claude's stash — so the
        // title, which is drawn either way, must never name it.
        let fx = Fx::new("edit-hints");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        assert_eq!(pane.action_hint(), Some("ctrl+s save"));
        assert!(!pane.title().contains("ctrl"), "{}", pane.title());

        press(&mut pane, KeyCode::Esc);
        assert_eq!(
            pane.action_hint(),
            Some("● unsaved · ctrl+s save · e edit · x x discard")
        );
        assert_eq!(pane.exit_hint(), "esc→agent");
        assert!(!pane.title().contains("ctrl"), "{}", pane.title());
        assert!(!page(&mut pane, 40, 10).contains("ctrl"), "nor the page");

        chord(&mut pane, 's');
        assert!(pane.action_hint().is_none(), "nothing unsaved, nothing to offer");
        assert!(!pane.title().contains("unsaved"));
    }

    // --- saving -------------------------------------------------------------

    #[test]
    fn ctrl_s_writes_the_file_back_with_its_own_line_endings_and_mark() {
        let fx = Fx::new("edit-save-crlf");
        let path = fx.write("notes.md", b"\xEF\xBB\xBF# Notes\r\n\r\nfirst\r\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        for code in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
            press(&mut pane, code);
        }
        type_in(&mut pane, " and more");
        press(&mut pane, KeyCode::Enter);
        type_in(&mut pane, "second");

        assert_eq!(chord(&mut pane, 's'), Handled::Yes);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"\xEF\xBB\xBF# Notes\r\n\r\nfirst and more\r\nsecond\r\n"
        );
        assert!(pane.unsaved().is_none());
        assert!(pane.editing(), "a save stays in the editor");
        assert!(!pane.title().contains("unsaved"), "{}", pane.title());
        assert_eq!(chord(&mut pane, 's'), Handled::No, "nothing unsaved is nothing to save");
        let names: Vec<_> = std::fs::read_dir(fx.root())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["notes.md"], "and nothing left beside it");
    }

    #[test]
    fn ctrl_s_from_the_reading_view_saves_and_leaves_the_saved_text_on_the_page() {
        let fx = Fx::new("edit-save-reading");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "Saved: ");
        press(&mut pane, KeyCode::Esc);

        assert_eq!(chord(&mut pane, 's'), Handled::Yes);
        assert_eq!(std::fs::read(&path).unwrap(), b"Saved: # Plan\n");
        assert!(pane.edit.is_none(), "nothing unsaved and nobody typing");
        assert!(pane.echo.is_some(), "what was written is remembered for its echo");
        let shown = page(&mut pane, 40, 10);
        assert!(shown.contains("Saved: # Plan"), "{shown}");
        assert!(!pane.title().contains("unsaved"));
    }

    #[test]
    fn a_save_from_the_reading_view_still_says_what_it_could_not_tidy_up() {
        // The edit closes on a save from the reading view, and what the save
        // had to say must not close with it. A recovery copy that will not be
        // deleted — a directory where the file was — is the one such sentence
        // a test can make on every platform.
        let fx = Fx::new("edit-save-note");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        quiet_is_over(&mut pane);
        pane.tick();
        let copy = fx.drafts().path_of(&edit(&pane).resolved);
        std::fs::remove_file(&copy).unwrap();
        std::fs::create_dir(&copy).unwrap();
        press(&mut pane, KeyCode::Esc);

        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"x# Plan\n");
        assert!(pane.edit.is_none());
        let shown = page(&mut pane, 60, 12);
        assert!(shown.contains("could not remove the recovery copy"), "{shown}");
    }

    #[test]
    fn a_save_the_disk_changed_under_overwrites_only_once_the_conflict_was_drawn() {
        let fx = Fx::new("edit-conflict");
        let path = fx.write("plan.md", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        press(&mut pane, KeyCode::End);
        type_in(&mut pane, " two");
        std::fs::write(&path, b"the agent's\n").unwrap();

        assert_eq!(chord(&mut pane, 's'), Handled::Yes);
        assert_eq!(std::fs::read(&path).unwrap(), b"the agent's\n", "refused, not written");
        assert!(pane.title().contains("◆ changed on disk"), "{}", pane.title());
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s again overwrites"));
        // A second press arriving in the same batch, before any frame, is the
        // press that asks for the frame — not the overwrite.
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"the agent's\n");

        let shown = page(&mut pane, 50, 12);
        assert!(shown.contains("has changed on disk since it was read"), "{shown}");
        assert!(!shown.contains("ctrl"), "the page names no chord: {shown}");
        assert_eq!(chord(&mut pane, 's'), Handled::Yes);
        assert_eq!(std::fs::read(&path).unwrap(), b"one two\n");
        assert!(!pane.title().contains("◆"), "{}", pane.title());
        assert!(pane.unsaved().is_none());
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false, reason = "putting the file back to be deleted")]
    fn a_save_that_is_refused_says_why_and_keeps_every_word() {
        let fx = Fx::new("edit-save-readonly");
        let path = fx.write("locked.md", b"# Locked\n");
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms.clone()).unwrap();
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");

        assert_eq!(chord(&mut pane, 's'), Handled::Yes, "a frame for the sentence");
        let shown = page(&mut pane, 50, 12);
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).unwrap();
        assert!(shown.contains("read-only"), "{shown}");
        assert_eq!(std::fs::read(&path).unwrap(), b"# Locked\n");
        assert_eq!(text(&pane), "x# Locked\n", "and the text is still here");
        assert!(pane.unsaved().is_some());
    }

    // --- x x ------------------------------------------------------------

    #[test]
    fn x_twice_throws_the_text_away_but_only_once_the_question_was_on_screen() {
        let fx = Fx::new("edit-discard");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "Gone: ");
        quiet_is_over(&mut pane);
        pane.tick();
        let resolved = edit(&pane).resolved.clone();
        let copy = fx.drafts().path_of(&resolved);
        assert!(copy.is_file(), "a recovery copy to see go");
        press(&mut pane, KeyCode::Esc);

        assert_eq!(press(&mut pane, KeyCode::Char('x')), Handled::Yes);
        assert_eq!(pane.action_hint(), Some("x again discards"));
        // Two `x`es in one batch: the second arrives before any frame drew the
        // question, and is the question again.
        press(&mut pane, KeyCode::Char('x'));
        assert!(pane.unsaved().is_some(), "thrown away unseen");

        let shown = page(&mut pane, 50, 12);
        assert!(shown.contains("Press x again"), "{shown}");
        press(&mut pane, KeyCode::Char('x'));
        assert!(pane.edit.is_none());
        assert!(!copy.exists(), "the recovery copy went with it");
        assert_eq!(std::fs::read(&path).unwrap(), b"# Plan\n");
        let shown = page(&mut pane, 50, 12);
        assert!(!shown.contains("Gone"), "the page is the disk's: {shown}");
    }

    #[test]
    fn any_other_key_between_the_two_presses_keeps_the_text() {
        let fx = Fx::new("edit-discard-cancel");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "kept ");
        press(&mut pane, KeyCode::Esc);

        // A key this pane handles.
        press(&mut pane, KeyCode::Char('x'));
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('j'));
        press(&mut pane, KeyCode::Char('x'));
        assert!(pane.unsaved().is_some(), "j withdrew the question");

        // And keys that never reach it — `F4`, and a letter at the agent —
        // which the shell reports through `keystroke` alone.
        draw(&mut pane, 50, 12);
        pane.keystroke();
        pane.keystroke();
        press(&mut pane, KeyCode::Char('x'));
        assert!(pane.unsaved().is_some(), "a key elsewhere withdrew the question");
        assert!(!page(&mut pane, 50, 12).is_empty());
        press(&mut pane, KeyCode::Char('x'));
        assert!(pane.edit.is_none(), "and asked again, it is answered");
    }

    // --- the watcher --------------------------------------------------------

    #[test]
    fn the_watcher_reporting_the_pane_s_own_save_is_quiet_before_and_after_it_closes() {
        let fx = Fx::new("edit-echo");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        chord(&mut pane, 's');

        assert_eq!(pane.disk_changed(&path), Some(false), "its own save, while it is open");
        assert!(edit(&pane).conflict.is_none());
        press(&mut pane, KeyCode::Esc);
        assert!(pane.edit.is_none());
        assert_eq!(pane.disk_changed(&path), Some(false), "and once it has closed");
        assert!(!pane.has_pending());

        // Somebody else's write is the reader's ordinary news again.
        std::fs::write(&path, b"# Theirs\n").unwrap();
        assert_eq!(pane.disk_changed(&path), None);
        assert!(pane.echo.is_none());
    }

    #[test]
    fn a_change_on_disk_under_unsaved_text_is_a_conflict_at_once_whatever_the_file() {
        let fx = Fx::new("edit-watch-conflict");
        let path = fx.write("main.rs", b"fn main() {}\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        press(&mut pane, KeyCode::End);
        type_in(&mut pane, " // mine");
        std::fs::write(&path, b"fn main() { theirs() }\n").unwrap();

        // Spelled as the watcher may spell it: on Windows, in another case.
        #[cfg(windows)]
        let reported = PathBuf::from(path.to_string_lossy().to_uppercase());
        #[cfg(unix)]
        let reported = path.clone();
        assert_eq!(pane.disk_changed(&reported), Some(true));
        assert!(pane.title().contains("◆ changed on disk"), "{}", pane.title());
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s save"), "no press has been refused yet");
        assert_eq!(text(&pane), "fn main() {} // mine\n", "the text is untouched");
        assert_eq!(pane.disk_changed(&fx.root().join("other.rs")), None);

        // The disk going back to what was opened takes the conflict with it.
        std::fs::write(&path, b"fn main() {}\n").unwrap();
        assert_eq!(pane.disk_changed(&path), Some(true));
        assert!(edit(&pane).conflict.is_none());
    }

    #[test]
    fn a_change_on_disk_with_nothing_unsaved_is_read_again_and_cannot_be_undone() {
        let fx = Fx::new("edit-watch-reload");
        let path = fx.write("notes.txt", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        chord(&mut pane, 'z');
        press(&mut pane, KeyCode::End);
        std::fs::write(&path, b"one\ntwo\n").unwrap();

        assert_eq!(pane.disk_changed(&path), Some(true));
        assert_eq!(text(&pane), "one\ntwo\n");
        assert_eq!(edit(&pane).editor.caret(), (0, 3), "the caret stayed where it was");
        assert!(pane.unsaved().is_none(), "what is on disk is not unsaved");
        assert!(edit(&pane).conflict.is_none());
        assert!(page(&mut pane, 50, 10).contains("read again"));
        // The reload is a floor: nothing undoes the agent's change out of the
        // text, so there is no revert for a `Ctrl+S` to write — and the redo
        // of what was typed before it is gone with the file it was typed into.
        assert_eq!(chord(&mut pane, 'z'), Handled::No);
        assert_eq!(chord(&mut pane, 'y'), Handled::No);
        assert_eq!(text(&pane), "one\ntwo\n");
        assert_eq!(chord(&mut pane, 's'), Handled::No);
        assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\n");
    }

    #[test]
    fn a_file_gone_from_under_a_clean_editor_blocks_it_and_it_keeps_the_keys() {
        // Deleted while somebody is about to type: the editor stays, keeps the
        // keys, says why, and writes nothing — the next letters must not land
        // in the reader, where `Tab` opens another file and `q` hands the
        // sentence to the agent.
        let fx = Fx::new("edit-watch-gone");
        let path = fx.write("notes.txt", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 10);
        press(&mut pane, KeyCode::Char('e'));
        std::fs::remove_file(&path).unwrap();

        assert_eq!(pane.disk_changed(&path), Some(true));
        assert!(pane.editing(), "the editor stayed open");
        assert!(pane.takes_input());
        assert_eq!(pane.action_hint(), Some("◆ no longer editable"));
        let shown = page(&mut pane, 60, 10);
        assert!(shown.contains("changed on disk into a file abeam cannot edit"), "{shown}");
        assert!(shown.contains("No such file"), "and why: {shown}");
        assert_eq!(chord(&mut pane, 's'), Handled::No, "nothing to save, so nothing saved");
        assert_eq!(pane.disk_changed(&path), Some(false), "the same news again draws nothing");

        // The keys are still the editor's.
        for code in [KeyCode::Char('q'), KeyCode::Tab, KeyCode::Char('?')] {
            assert_eq!(press(&mut pane, code), Handled::Yes, "{code:?}");
        }
        assert!(pane.editing());
        assert!(text(&pane).starts_with("q"), "{:?}", text(&pane));
        assert!(!path.exists(), "and nothing put it back");
        for _ in 0..3 {
            chord(&mut pane, 'z');
        }

        // `Esc` is the way out, and the page is the disk's.
        press(&mut pane, KeyCode::Esc);
        assert!(pane.edit.is_none());
        assert!(page(&mut pane, 60, 10).contains("no such file"));

        // Grown into something the strict read refuses: the same block.
        let path = fx.write("mixed.txt", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 10);
        press(&mut pane, KeyCode::Char('e'));
        std::fs::write(&path, b"one\r\ntwo\n").unwrap();
        assert_eq!(pane.disk_changed(&path), Some(true));
        assert!(pane.editing());
        assert!(page(&mut pane, 60, 10).contains("Mixed line endings"));
        assert_eq!(std::fs::read(&path).unwrap(), b"one\r\ntwo\n");
    }

    #[test]
    fn the_watcher_s_report_of_a_refused_change_does_not_start_the_refusal_over() {
        // The refusal is made against v2; the watcher then reports v2, a
        // debounce later. The notice promised that one more press writes over
        // that version, and it must still be one.
        let fx = Fx::new("edit-refusal-echo");
        let path = fx.write("plan.md", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        std::fs::write(&path, b"agent v2\n").unwrap();
        chord(&mut pane, 's');
        draw(&mut pane, 50, 12);

        assert_eq!(pane.disk_changed(&path), Some(false), "nothing new to say");
        assert!(edit(&pane).conflict.as_ref().is_some_and(|c| c.why == Why::Refused && c.drawn));
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"xone\n");

        // A newer change than the refusal saw is news, and starts it over.
        type_in(&mut pane, "y");
        std::fs::write(&path, b"agent v3\n").unwrap();
        chord(&mut pane, 's');
        draw(&mut pane, 50, 12);
        std::fs::write(&path, b"agent v4\n").unwrap();
        assert_eq!(pane.disk_changed(&path), Some(true));
        assert!(edit(&pane).conflict.as_ref().is_some_and(|c| c.why == Why::Changed));
    }

    #[test]
    fn a_copy_of_a_file_that_is_gone_is_still_found() {
        // Deleted or renamed: the copy is under the path the file had, and
        // resolving that path fails, so it is found through the directory.
        let fx = Fx::new("edit-copy-gone");
        let path = fx.write("plan.md", b"# Plan\n");
        let key = crate::paths::resolve(&path).unwrap();
        fx.drafts().write(&fx.root(), &key, "mine\n", Some(Fingerprint::of(b"# Plan\n"))).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(key_for(&path), Some(key.clone()));

        let mut pane = fx.pane(&path);
        draw(&mut pane, 200, 12);
        press(&mut pane, KeyCode::Char('e'));
        let shown = page(&mut pane, 200, 12);
        let copy = fx.drafts().path_of(&key).display().to_string();
        assert!(shown.contains(&copy), "{shown}");
    }

    /// A copy offered back, undone to look at the disk — the editor is clean,
    /// and the recovered text is in the redo and the profile and nowhere else —
    /// and then the agent writes the file. That used to reload into a fresh
    /// editor, which took the redo with it, and the next keystroke's copy
    /// replaced the old one: the text was gone from both places.
    #[test]
    fn a_recovered_copy_undone_to_look_at_the_disk_survives_the_disk_changing() {
        let fx = Fx::new("edit-recovered-undo");
        let path = fx.write("plan.md", b"# Plan\n");
        let key = crate::paths::resolve(&path).unwrap();
        fx.drafts().write(&fx.root(), &key, "mine\n", Some(Fingerprint::of(b"# Plan\n"))).unwrap();
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        assert_eq!(text(&pane), "mine\n");
        chord(&mut pane, 'z');
        assert_eq!(text(&pane), "# Plan\n", "one undo is the disk");
        assert!(pane.unsaved().is_none());

        std::fs::write(&path, b"# Plan, the agent's\n").unwrap();
        assert_eq!(pane.disk_changed(&path), Some(true));
        assert!(
            edit(&pane).conflict.as_ref().is_some_and(|c| c.why == Why::Changed),
            "a conflict, not a reload"
        );
        assert_eq!(fx.drafts().read(&key).unwrap().expect("the copy").text, "mine\n");
        chord(&mut pane, 'y');
        assert_eq!(text(&pane), "mine\n", "the redo still holds what was recovered");

        // Undone again and left: the copy is still there for the next `e`,
        // which offers it in conflict with the file as it now is.
        chord(&mut pane, 'z');
        press(&mut pane, KeyCode::Esc);
        assert!(pane.edit.is_none());
        assert_eq!(fx.drafts().read(&key).unwrap().expect("the copy").text, "mine\n");
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        assert_eq!(text(&pane), "mine\n");
        assert!(edit(&pane).conflict.as_ref().is_some_and(|c| c.why == Why::Recovered));

        // With no copy in the profile, a clean editor is still read in again.
        let fx = Fx::new("edit-clean-reload");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        std::fs::write(&path, b"# Plan, the agent's\n").unwrap();
        assert_eq!(pane.disk_changed(&path), Some(true));
        assert!(edit(&pane).conflict.is_none());
        assert_eq!(text(&pane), "# Plan, the agent's\n");
    }

    #[test]
    fn a_clean_editor_never_writes_even_in_a_conflict() {
        let fx = Fx::new("edit-clean-conflict");
        let path = fx.write("plan.md", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        std::fs::write(&path, b"theirs\n").unwrap();
        pane.disk_changed(&path);
        chord(&mut pane, 's');
        draw(&mut pane, 50, 12);
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s again overwrites"));
        // Undone back to the version it was opened on: nothing of the user's is
        // left in it, and a save would put the old file back over the new one.
        chord(&mut pane, 'z');
        assert!(pane.unsaved().is_none());
        assert_eq!(chord(&mut pane, 's'), Handled::No);
        assert_eq!(std::fs::read(&path).unwrap(), b"theirs\n");
    }

    #[test]
    fn writing_over_a_change_expects_the_version_the_refusal_saw_and_no_newer_one() {
        let fx = Fx::new("edit-conflict-v3");
        let path = fx.write("plan.md", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        press(&mut pane, KeyCode::End);
        type_in(&mut pane, " two");
        std::fs::write(&path, b"agent v2\n").unwrap();
        chord(&mut pane, 's');
        draw(&mut pane, 50, 12);
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s again overwrites"));

        // The agent writes again after the reader saw the refusal about v2.
        std::fs::write(&path, b"agent v3\n").unwrap();
        assert_eq!(chord(&mut pane, 's'), Handled::Yes);
        assert_eq!(std::fs::read(&path).unwrap(), b"agent v3\n", "v3 was written over unseen");
        assert!(
            edit(&pane).conflict.as_ref().is_some_and(|c| c.why == Why::Refused && !c.drawn),
            "refused again, about the new version, and not yet seen"
        );
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"agent v3\n", "and not before it is drawn");

        draw(&mut pane, 50, 12);
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"one two\n");
        assert!(edit(&pane).conflict.is_none());
    }

    #[test]
    fn one_ctrl_s_never_writes_over_a_change_the_watcher_found() {
        // `Ctrl+S` out of habit on coming back to the pane, in front of a `◆`
        // that has been on screen for a minute: still only the first press.
        let fx = Fx::new("edit-conflict-habit");
        let path = fx.write("plan.md", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 50, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        std::fs::write(&path, b"theirs\n").unwrap();
        pane.disk_changed(&path);
        draw(&mut pane, 50, 12);

        assert_eq!(chord(&mut pane, 's'), Handled::Yes);
        assert_eq!(std::fs::read(&path).unwrap(), b"theirs\n");
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s again overwrites"));
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"theirs\n", "the refusal was never drawn");
        draw(&mut pane, 50, 12);
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"xone\n");
    }

    #[test]
    fn a_deleted_file_is_put_back_by_a_second_press_and_only_by_one() {
        let fx = Fx::new("edit-conflict-deleted");
        let path = fx.write("plan.md", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "kept ");
        std::fs::remove_file(&path).unwrap();

        chord(&mut pane, 's');
        assert!(!path.exists());
        let shown = page(&mut pane, 60, 12);
        assert!(shown.contains("has been deleted since it was read"), "{shown}");
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"kept one\n");
    }

    #[test]
    fn a_batch_that_lost_its_paths_still_looks_at_the_file_being_edited() {
        let fx = Fx::new("edit-watch-overflow");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        assert!(!pane.disk_may_have_changed(), "no edit, nothing to look at");
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        assert!(!pane.disk_may_have_changed(), "unchanged");
        std::fs::write(&path, b"# Theirs\n").unwrap();
        assert!(pane.disk_may_have_changed());
        assert!(edit(&pane).conflict.is_some());
    }

    // --- what may not replace the page -----------------------------------

    #[test]
    fn unsaved_text_refuses_everything_that_would_put_another_page_over_it() {
        let fx = Fx::new("edit-guards");
        let a = fx.write("a.md", b"# a\n");
        let b = fx.write("b.md", b"# b\n");
        let mut pane = fx.pane(&a);
        pane.recent = vec![a.clone(), b.clone()];
        draw(&mut pane, 50, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "mine ");
        press(&mut pane, KeyCode::Esc);
        let refused = |pane: &mut ViewerPane, what: &str| {
            let said = pane.refusal().map(str::to_string);
            assert!(
                said.as_deref().is_some_and(|s| s.contains("a.md has unsaved text")),
                "{what} was not refused out loud: {said:?}"
            );
            assert_eq!(pane.path(), Some(a.as_path()), "{what} replaced the page");
            pane.keystroke();
        };

        assert!(!pane.show(&b));
        refused(&mut pane, "show");
        let at = pane.recent_ix;
        assert_eq!(press(&mut pane, KeyCode::Tab), Handled::Yes);
        assert_eq!(pane.recent_ix, at, "the walk did not move either");
        refused(&mut pane, "Tab");
        press(&mut pane, KeyCode::BackTab);
        refused(&mut pane, "Shift+Tab");
        press(&mut pane, KeyCode::Char('r'));
        refused(&mut pane, "r");
        pane.open_browse();
        assert!(matches!(pane.mode, Mode::Doc), "the list did not open");
        refused(&mut pane, "F1, B");
        assert!(!pane.set_root(fx.dir.path().to_path_buf()));
        assert_eq!(pane.root, fx.root());
        refused(&mut pane, "a worktree switch");

        // The watcher's document waits behind the mark.
        pane.follow(b.clone());
        draw(&mut pane, 50, 10);
        assert_eq!(pane.path(), Some(a.as_path()));
        assert!(pane.title().starts_with("◆ "), "{}", pane.title());

        // And through all of it the text was kept.
        assert!(page(&mut pane, 50, 10).contains("mine"));
        assert!(pane.unsaved().is_some());
        // The refusal is gone at the next key, however it arrives.
        pane.show(&b);
        assert!(pane.refusal().is_some());
        pane.keystroke();
        assert!(pane.refusal().is_none());
    }

    #[test]
    fn an_edit_with_nothing_unsaved_is_closed_rather_than_guarded() {
        let fx = Fx::new("edit-guards-clean");
        let a = fx.write("a.md", b"# a\n");
        let b = fx.write("b.md", b"# b\n");
        let mut pane = fx.pane(&a);
        draw(&mut pane, 50, 10);
        press(&mut pane, KeyCode::Char('e'));
        assert!(pane.show(&b));
        assert!(pane.edit.is_none());
        assert_eq!(pane.path(), Some(b.as_path()));

        press(&mut pane, KeyCode::Char('e'));
        pane.open_browse();
        assert!(matches!(pane.mode, Mode::Browse));
        assert!(pane.edit.is_none());
    }

    #[test]
    fn a_document_the_watcher_queues_waits_for_the_editor_and_esc_releases_it() {
        let fx = Fx::new("edit-pending");
        let a = fx.write("a.md", b"# a\n");
        let b = fx.write("b.md", b"# b\n");
        let mut pane = fx.pane(&a);
        draw(&mut pane, 50, 10);
        press(&mut pane, KeyCode::Char('e'));
        pane.follow(b.clone());
        draw(&mut pane, 50, 10);
        assert_eq!(pane.path(), Some(a.as_path()), "nothing moves under somebody typing");
        assert!(pane.title().starts_with("◆ a.md · editing"), "{}", pane.title());
        assert_eq!(pane.exit_hint(), "esc→new file", "nothing unsaved: Esc releases it");
        type_in(&mut pane, "x");
        assert_eq!(pane.exit_hint(), "esc→done", "unsaved: Esc keeps the text instead");
        chord(&mut pane, 'z');

        press(&mut pane, KeyCode::Esc);
        draw(&mut pane, 50, 10);
        assert_eq!(pane.path(), Some(b.as_path()));
    }

    // --- the view -----------------------------------------------------------

    #[test]
    fn a_glance_scrolls_the_file_being_typed_into_and_never_moves_the_caret() {
        let fx = Fx::new("edit-glance");
        let path = fx.write("notes.txt", numbered(100).as_bytes());
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        draw(&mut pane, 40, 10);
        let caret = edit(&pane).editor.caret();

        // `F1, J` and `F1, PgDn`, arriving as the bare keys.
        assert_eq!(pane.scroll_key(key(KeyCode::Down)).unwrap(), Handled::Yes);
        assert_eq!(pane.scroll_key(key(KeyCode::PageDown)).unwrap(), Handled::Yes);
        // `PgDn` with the keys, which pages and leaves the caret too.
        assert_eq!(press(&mut pane, KeyCode::PageDown), Handled::Yes);
        // The wheel.
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(pane.handle_mouse(&wheel).unwrap(), Handled::Yes);
        draw(&mut pane, 40, 10);
        assert_eq!(edit(&pane).editor.caret(), caret);
        assert_eq!(text(&pane), numbered(100), "and nothing was typed");
        assert!(edit(&pane).view.scroll.offset > 10);
        assert_eq!(pane.cursor(), None, "the caret is off screen, not dragged on");
    }

    #[test]
    fn a_paste_goes_in_whole_and_a_click_puts_the_caret_where_it_landed() {
        let fx = Fx::new("edit-paste-click");
        let path = fx.write("notes.txt", b"alpha\nbravo\ncharlie\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        draw(&mut pane, 40, 10);

        assert_eq!(pane.handle_paste("one\ntwo ").unwrap(), Handled::Yes);
        assert_eq!(text(&pane), "one\ntwo alpha\nbravo\ncharlie\n");
        draw(&mut pane, 40, 10);
        // Row three is `charlie`; four columns of gutter come first.
        assert_eq!(pane.handle_mouse(&click(4 + 2, 3)).unwrap(), Handled::Yes);
        assert_eq!(edit(&pane).editor.caret(), (3, 2));
        // A click on a line number is the start of that line.
        assert_eq!(pane.handle_mouse(&click(1, 2)).unwrap(), Handled::Yes);
        assert_eq!(edit(&pane).editor.caret(), (2, 0));
        // And reading, a paste goes nowhere.
        press(&mut pane, KeyCode::Esc);
        assert_eq!(pane.handle_paste("more").unwrap(), Handled::No);
    }

    #[test]
    fn the_outline_over_an_unsaved_page_still_says_it_is_unsaved() {
        let fx = Fx::new("edit-outline");
        let path = fx.write("plan.md", b"# Plan\n\n## Steps\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        press(&mut pane, KeyCode::Esc);
        draw(&mut pane, 40, 10);
        assert_eq!(press(&mut pane, KeyCode::Char('o')), Handled::Yes);
        assert!(matches!(pane.mode, Mode::Outline));
        assert!(pane.title().starts_with("● plan.md"), "{}", pane.title());
    }

    #[test]
    fn the_editor_wears_the_reader_s_gutter_and_the_cursor_sits_after_it() {
        let fx = Fx::new("edit-gutter");
        let path = fx.write("main.rs", b"fn main() {\n    let a_long_name_that_wraps = 1;\n}\n");
        let mut pane = fx.pane(&path);
        let reading = draw(&mut pane, 32, 6);
        press(&mut pane, KeyCode::Char('e'));
        let rows = draw(&mut pane, 32, 6);
        assert!(rows[0].starts_with("  1 fn main() {"), "{rows:#?}");
        assert_eq!(&rows[0][..4], &reading[0][..4], "the reader's gutter, to the column");
        assert!(rows[1].starts_with("  2     let"), "{rows:#?}");
        assert!(rows[2].starts_with("    "), "a wrapped row has a blank gutter: {rows:#?}");
        assert!(rows[3].starts_with("  3 }"), "{rows:#?}");
        assert_eq!(pane.cursor(), Some((4, 0)));
        // Narrow, there is no gutter, as in the reader.
        let rows = draw(&mut pane, 20, 6);
        assert!(rows[0].starts_with("fn main() {"), "{rows:#?}");
        assert_eq!(pane.cursor(), Some((0, 0)));
    }

    #[test]
    fn drawing_the_editor_at_hostile_sizes_does_not_panic() {
        let fx = Fx::new("edit-sizes");
        let path = fx.write("plan.md", b"# Plan\n\nSome words here.\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        std::fs::write(&path, b"changed\n").unwrap();
        pane.disk_changed(&path);
        for (w, h) in [(1, 1), (2, 2), (5, 1), (40, 1), (40, 2), (3, 30), (80, 40)] {
            draw(&mut pane, w, h);
        }
    }

    #[test]
    fn a_conflict_too_tall_for_the_pane_is_not_answered_by_a_second_ctrl_s() {
        // Drawn means every row of the sentence was on screen. A pane two rows
        // tall keeps one for the text, so the sentence does not fit and the
        // overwrite must not be armed by a frame that could not show it.
        let fx = Fx::new("edit-conflict-short");
        let path = fx.write("plan.md", b"one\n");
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        std::fs::write(&path, b"theirs\n").unwrap();
        chord(&mut pane, 's');
        draw(&mut pane, 40, 2);
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"theirs\n");
        draw(&mut pane, 40, 10);
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"xone\n");
    }

    #[test]
    fn a_file_too_large_to_colour_is_drawn_plain_and_says_why() {
        let fx = Fx::new("edit-plain");
        let big: String = "A line of prose that is long enough to add up.\n".repeat(1_600);
        assert!(big.len() > source::HIGHLIGHT_MAX_BYTES);
        let path = fx.write("big.md", big.as_bytes());
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        assert!(page(&mut pane, 60, 12).contains("Drawn without colour"));

        let small = fx.write("small.md", b"# Small\n");
        assert!(pane.show(&small));
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        assert!(!page(&mut pane, 60, 12).contains("without colour"));
    }

    // --- recovery copies ----------------------------------------------------

    #[test]
    fn unsaved_text_is_copied_out_after_the_quiet_and_the_copy_goes_with_a_save() {
        let fx = Fx::new("edit-copy");
        let disk = b"# Plan\n";
        let path = fx.write("plan.md", disk);
        let mut pane = fx.pane(&path);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        let resolved = edit(&pane).resolved.clone();
        let drafts = fx.drafts();

        assert!(!pane.tick());
        assert_eq!(drafts.read(&resolved), Ok(None), "not before the quiet");
        quiet_is_over(&mut pane);
        assert!(!pane.tick(), "a copy that worked changes nothing on screen");
        let copy = drafts.read(&resolved).unwrap().expect("a copy");
        assert_eq!(copy.text, "x# Plan\n");
        assert_eq!(copy.base, Some(Fingerprint::of(disk)), "typed against the disk's version");
        assert!(!crate::paths::under(&fx.root(), &drafts.path_of(&resolved)), "never in the repo");

        // The way out writes what the debounce has not reached yet.
        type_in(&mut pane, "y");
        pane.flush_draft();
        assert_eq!(drafts.read(&resolved).unwrap().expect("a copy").text, "xy# Plan\n");

        chord(&mut pane, 's');
        assert_eq!(drafts.read(&resolved), Ok(None), "saved, and the copy went");
    }

    #[test]
    fn a_copy_outlives_an_undo_and_a_leaving_and_is_offered_again() {
        // Text recovered from a crash, undone back to the disk to compare, and
        // left: only a save or `x x` deletes a copy, so the next `e` offers the
        // text again rather than the profile quietly losing it.
        let fx = Fx::new("edit-copy-undone");
        let path = fx.write("plan.md", b"# Plan\n");
        let resolved = crate::paths::resolve(&path).unwrap();
        fx.drafts()
            .write(&fx.root(), &resolved, "crash # Plan\n", Some(Fingerprint::of(b"# Plan\n")))
            .unwrap();
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        assert_eq!(text(&pane), "crash # Plan\n");
        chord(&mut pane, 'z');
        assert!(pane.unsaved().is_none());
        quiet_is_over(&mut pane);
        pane.tick();
        press(&mut pane, KeyCode::Esc);
        assert!(pane.edit.is_none());

        assert!(fx.drafts().read(&resolved).unwrap().is_some(), "the copy was deleted");
        press(&mut pane, KeyCode::Char('e'));
        assert_eq!(text(&pane), "crash # Plan\n", "and is offered again");
        press(&mut pane, KeyCode::Esc);
        chord(&mut pane, 's');
        assert_eq!(fx.drafts().read(&resolved), Ok(None), "a save is what deletes it");
    }

    #[test]
    fn a_page_over_a_waiting_copy_says_so_and_a_refused_e_says_where_it_is() {
        let fx = Fx::new("edit-copy-offer");
        let path = fx.write("plan.md", b"# Plan\n");
        let resolved = crate::paths::resolve(&path).unwrap();
        fx.drafts()
            .write(&fx.root(), &resolved, "mine\n", Some(Fingerprint::of(b"# Plan\n")))
            .unwrap();
        let mut pane = fx.pane(&path);
        let shown = page(&mut pane, 80, 12);
        assert!(shown.contains("is kept in a recovery copy · e to resume it"), "{shown}");
        press(&mut pane, KeyCode::Char('e'));
        assert_eq!(text(&pane), "mine\n");
        press(&mut pane, KeyCode::Esc);
        assert!(!page(&mut pane, 80, 12).contains("e to resume it"), "the copy is open now");

        // A file the strict read refuses, with a copy waiting: where it is.
        let other = fx.write("mixed.md", b"one\n");
        let key = crate::paths::resolve(&other).unwrap();
        fx.drafts().write(&fx.root(), &key, "lost?\n", None).unwrap();
        std::fs::write(&other, b"one\r\ntwo\n").unwrap();
        let mut pane = fx.pane(&other);
        draw(&mut pane, 80, 12);
        press(&mut pane, KeyCode::Char('e'));
        let shown = page(&mut pane, 200, 12);
        assert!(shown.contains("Unsaved text for it from an earlier session is in"), "{shown}");
        let copy = fx.drafts().path_of(&key).display().to_string();
        assert!(shown.contains(&copy), "{shown}");
    }

    #[test]
    fn the_quit_warning_says_whether_the_text_will_survive_it() {
        let fx = Fx::new("edit-quit-note");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        assert_eq!(pane.quit_note(), None);
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        // Nothing written yet, so nothing promised: `F1, Q` writes the copy
        // before it asks, which is the flush here.
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md will be lost"));
        pane.flush_draft();
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md kept as a recovery copy"));
        pane.set_drafts(None);
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md will be lost"));
    }

    /// A profile inside the workspace — abeam started in the home directory —
    /// refuses every copy. The warning used to promise one anyway, on the
    /// strength of being allowed to write it, and the second `F1, Q` then lost
    /// the text it had called safe.
    #[test]
    fn the_quit_warning_promises_a_copy_only_once_a_write_of_it_has_worked() {
        let fx = Fx::new("edit-quit-inside");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        let inside = fx.root().join(".abeam").join("drafts");
        pane.set_drafts(Some(Drafts::at(inside)));
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        pane.flush_draft();
        assert!(edit(&pane).copy.failed.is_some(), "the profile inside refused the copy");
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md will be lost"));
        assert!(!fx.root().join(".abeam").exists(), "and nothing was made in the repository");

        // The same text once a profile takes it: written, and only then kept.
        pane.set_drafts(Some(fx.drafts()));
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md will be lost"));
        pane.flush_draft();
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md kept as a recovery copy"));
        let key = crate::paths::resolve(&path).unwrap();
        assert_eq!(fx.drafts().read(&key).unwrap().expect("a copy").text, "x# Plan\n");
        // A change since is not in the copy until it is written again.
        type_in(&mut pane, "y");
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md will be lost"));
        pane.flush_draft();
        assert_eq!(pane.quit_note().as_deref(), Some("unsaved plan.md kept as a recovery copy"));
    }

    #[test]
    fn the_watcher_is_asked_about_only_while_there_is_something_to_recognise() {
        let fx = Fx::new("edit-minds");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        assert!(!pane.minds_disk(), "no edit and no save: the shell skips the question");
        draw(&mut pane, 40, 10);
        press(&mut pane, KeyCode::Char('e'));
        assert!(pane.minds_disk());
        assert!(pane.minds(&path));
        assert!(!pane.minds(&fx.root().join("other.md")));
    }

    #[test]
    fn a_recovery_copy_comes_back_as_unsaved_text_one_undo_from_the_disk() {
        let fx = Fx::new("edit-recover");
        let disk = b"# Plan\n\nold\n";
        let path = fx.write("plan.md", disk);
        let resolved = crate::paths::resolve(&path).unwrap();
        let draft = "# Plan\n\nnew, from last time\n";
        fx.drafts()
            .write(&fx.root(), &resolved, draft, Some(Fingerprint::of(disk)))
            .unwrap();
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));

        assert_eq!(text(&pane), "# Plan\n\nnew, from last time\n");
        assert!(pane.unsaved().is_some());
        assert!(edit(&pane).conflict.is_none(), "typed against what is on disk");
        assert!(page(&mut pane, 60, 12).contains("Recovered unsaved text from a moment ago"));
        chord(&mut pane, 'z');
        assert_eq!(text(&pane), "# Plan\n\nold\n", "one undo is the disk");
        assert!(pane.unsaved().is_none());
    }

    #[test]
    fn a_copy_typed_against_another_version_opens_in_the_conflict_state() {
        let fx = Fx::new("edit-recover-conflict");
        let path = fx.write("plan.md", b"theirs\n");
        let resolved = crate::paths::resolve(&path).unwrap();
        fx.drafts()
            .write(&fx.root(), &resolved, "mine\n", Some(Fingerprint::of(b"original\n")))
            .unwrap();
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));

        assert!(pane.title().contains("◆ changed on disk"), "{}", pane.title());
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s save"));
        let shown = page(&mut pane, 60, 12);
        assert!(shown.contains("changed on disk since this text was typed"), "{shown}");
        // The baseline *is* the disk, so `disk::save` would see nothing to
        // refuse: the conflict is the only thing in the way. Drawn and all, the
        // first press is the refusal and writes nothing.
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"theirs\n");
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s again overwrites"));
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"theirs\n", "nor before it is drawn");
        draw(&mut pane, 60, 12);
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&path).unwrap(), b"mine\n");
    }

    #[test]
    fn a_copy_that_will_not_read_is_left_alone_for_the_session_and_said_so() {
        let fx = Fx::new("edit-recover-damaged");
        let path = fx.write("plan.md", b"# Plan\n");
        let resolved = crate::paths::resolve(&path).unwrap();
        let drafts = fx.drafts();
        std::fs::create_dir_all(drafts.dir()).unwrap();
        let damaged = drafts.path_of(&resolved);
        std::fs::write(&damaged, b"not a recovery copy\n").unwrap();

        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        type_in(&mut pane, "x");
        quiet_is_over(&mut pane);
        pane.tick();
        assert_eq!(std::fs::read(&damaged).unwrap(), b"not a recovery copy\n");
        assert!(page(&mut pane, 60, 12).contains("left where it is"));
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&damaged).unwrap(), b"not a recovery copy\n", "nor by a save");
    }

    #[test]
    fn having_nowhere_to_keep_copies_says_so_before_anybody_types() {
        let fx = Fx::new("edit-nowhere");
        let path = fx.write("plan.md", b"# Plan\n");
        let mut pane = fx.pane(&path);
        pane.set_drafts(None);
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('e'));
        assert!(page(&mut pane, 60, 12).contains("nowhere in your profile"));
    }

    // --- a new file -----------------------------------------------------------

    /// The file list open over `pane`, its name box open, and `name` typed.
    fn naming(pane: &mut ViewerPane, name: &str) {
        pane.open_browse();
        draw(pane, 60, 12);
        assert_eq!(press(pane, KeyCode::Char('a')), Handled::Yes);
        for c in name.chars() {
            press(pane, KeyCode::Char(c));
        }
    }

    #[test]
    fn a_in_the_list_opens_a_name_box_whose_every_key_is_a_letter() {
        let fx = Fx::new("create-box");
        let path = fx.write("README.md", b"# hi\n");
        let mut pane = fx.pane(&path);
        naming(&mut pane, "");
        assert!(pane.browse.naming());
        assert!(pane.takes_input());
        assert_eq!(pane.exit_hint(), "esc→cancel");
        assert!(pane.title().starts_with("new file: ▌ · in ./"), "{}", pane.title());

        // `f`, `q`, `j`, `a` and `r` are keys in the list; here they are letters.
        for c in "fqjar/x.md".chars() {
            assert_eq!(press(&mut pane, KeyCode::Char(c)), Handled::Yes, "{c}");
        }
        assert!(matches!(pane.mode, Mode::Browse), "f opened no search");
        assert_eq!(pane.title(), "new file: fqjar/x.md▌ · in ./ · creates fqjar/");
        assert!(pane.handle_paste("y").unwrap().is_yes());
        assert!(pane.title().contains("x.mdy▌"));

        // `Esc` throws the name away and stays in the list; so does
        // `Backspace` past the start.
        assert_eq!(press(&mut pane, KeyCode::Esc), Handled::Yes);
        assert!(!pane.browse.naming());
        assert!(matches!(pane.mode, Mode::Browse));
        press(&mut pane, KeyCode::Char('a'));
        press(&mut pane, KeyCode::Backspace);
        assert!(!pane.browse.naming());
        assert!(!fx.root().join("fqjar").exists(), "and nothing was made");

        // A name that is a file already says what `Enter` will do.
        press(&mut pane, KeyCode::Char('a'));
        for c in "README.md".chars() {
            press(&mut pane, KeyCode::Char(c));
        }
        assert!(pane.title().ends_with("· there already: opens it"), "{}", pane.title());
    }

    #[test]
    fn a_refused_name_keeps_the_box_open_and_says_why_until_the_name_changes() {
        let fx = Fx::new("create-refused");
        let path = fx.write("README.md", b"# hi\n");
        let mut pane = fx.pane(&path);
        naming(&mut pane, "con.md");
        assert_eq!(press(&mut pane, KeyCode::Enter), Handled::Yes);
        assert!(pane.browse.naming(), "the box is still open");
        assert!(pane.edit.is_none());
        let shown = page(&mut pane, 60, 12);
        assert!(shown.contains("con.md is a device on Windows"), "{shown}");
        assert!(shown.contains("README.md"), "the listing is still under it: {shown}");
        press(&mut pane, KeyCode::Backspace);
        assert!(!page(&mut pane, 60, 12).contains("is a device"), "mended, and gone");
        assert!(std::fs::read_dir(fx.root()).unwrap().count() == 1, "nothing was made");
    }

    #[test]
    fn a_new_name_opens_an_empty_editor_and_esc_with_nothing_typed_leaves_nothing() {
        let fx = Fx::new("create-untouched");
        let path = fx.write("README.md", b"# hi\n");
        let mut pane = fx.pane(&path);
        naming(&mut pane, "notes/today.md");
        assert!(pane.title().ends_with("· creates notes/"), "{}", pane.title());
        press(&mut pane, KeyCode::Enter);

        assert!(pane.editing());
        assert!(edit(&pane).unborn());
        assert_eq!(text(&pane), "");
        // Named the way the reader names every file: relative, with this
        // platform's separator.
        let name = pane.label(&fx.root().join("notes").join("today.md"));
        assert!(pane.title().starts_with(&format!("{name} · editing · new")), "{}", pane.title());
        assert_eq!(pane.exit_hint(), "esc→list");
        draw(&mut pane, 60, 12);
        assert_eq!(pane.cursor(), Some((4, 0)));

        assert_eq!(press(&mut pane, KeyCode::Esc), Handled::Yes);
        assert!(pane.edit.is_none());
        assert!(matches!(pane.mode, Mode::Browse), "back to the list the name was given in");
        assert!(!fx.root().join("notes").exists(), "no directory");
        assert_eq!(pane.path(), Some(path.as_path()), "and the page is as it was");
    }

    #[test]
    fn the_first_save_makes_the_directories_and_the_file_in_its_neighbours_ending() {
        let fx = Fx::new("create-save");
        let path = fx.write("README.md", b"# hi\r\n");
        let mut pane = fx.pane(&path);
        naming(&mut pane, "notes/today.md");
        press(&mut pane, KeyCode::Enter);
        type_in(&mut pane, "hello");
        press(&mut pane, KeyCode::Enter);
        assert_eq!(pane.exit_hint(), "esc→done", "something typed: Esc keeps it");

        assert_eq!(chord(&mut pane, 's'), Handled::Yes);
        let made = fx.root().join("notes").join("today.md");
        assert_eq!(std::fs::read(&made).unwrap(), b"hello\r\n", "the root README's ending");
        assert!(!edit(&pane).unborn(), "a file now, saved like any other");
        assert!(!pane.title().contains("· new"), "{}", pane.title());
        assert!(pane.scan.is_some(), "the walk that feeds the find and Tab was started");
        assert!(pane.browse.title().contains("2 items"), "{}", pane.browse.title());
        let names: Vec<_> = std::fs::read_dir(fx.root().join("notes"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["today.md"], "and nothing beside it");

        // The page is the file now.
        press(&mut pane, KeyCode::Esc);
        assert!(pane.edit.is_none());
        assert_eq!(pane.path(), Some(made.as_path()));
        assert!(page(&mut pane, 60, 12).contains("hello"));
    }

    #[test]
    fn a_name_that_is_already_a_file_opens_it_to_edit() {
        let fx = Fx::new("create-existing");
        let path = fx.write("README.md", b"# hi\n");
        let other = fx.write("other.md", b"# other\n\nline\n");
        let mut pane = fx.pane(&other);
        // In any case, on every platform: `super::name`'s rule, because on
        // Windows and macOS any case is that file.
        naming(&mut pane, "readme.MD");
        press(&mut pane, KeyCode::Enter);

        assert!(pane.editing());
        assert!(!edit(&pane).unborn());
        assert_eq!(text(&pane), "# hi\n");
        assert_eq!(pane.label(&edit(&pane).requested), "README.md", "the disk's spelling");
        assert_eq!(edit(&pane).editor.caret(), (0, 0));
        assert_eq!(pane.path().map(Path::to_path_buf), crate::paths::resolve(&path).ok());
    }

    /// A file that is there, reached through a link that leads out of the
    /// workspace: refused in the box, and the page never shows a line of it —
    /// the strict read `e` makes would refuse it too, but only after `show`
    /// had put it on screen.
    #[test]
    fn a_name_that_leads_to_a_file_outside_the_workspace_never_reaches_the_page() {
        let fx = Fx::new("create-outside");
        let path = fx.write("README.md", b"# hi\n");
        let elsewhere = fx.dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("secret.md"), b"# not the work\n").unwrap();
        // A junction on Windows, which needs no privilege; a symlink on Unix.
        let link = fx.root().join("out");
        #[cfg(windows)]
        let made = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&elsewhere)
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|out| out.status.success());
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&elsewhere, &link).is_ok();
        if !made {
            eprintln!("skipped: this machine would not make a directory link");
            return;
        }

        let mut pane = fx.pane(&path);
        naming(&mut pane, "out/secret.md");
        assert!(!pane.title().contains("opens it"), "{}", pane.title());
        assert_eq!(press(&mut pane, KeyCode::Enter), Handled::Yes);
        assert!(pane.browse.naming(), "the box is still open");
        assert!(pane.edit.is_none());
        assert_eq!(pane.path(), Some(path.as_path()), "the page is still the one it was");
        let shown = page(&mut pane, 60, 12);
        assert!(shown.contains("outside this workspace"), "{shown}");
        assert!(!shown.contains("not the work"), "the outside file was drawn: {shown}");
    }

    #[test]
    fn throwing_a_new_file_s_text_away_goes_back_to_the_list_and_the_page_it_left() {
        let fx = Fx::new("create-discard");
        let path = fx.write("README.md", b"# hi\n");
        let mut pane = fx.pane(&path);
        naming(&mut pane, "draft.md");
        press(&mut pane, KeyCode::Enter);
        type_in(&mut pane, "never kept");
        press(&mut pane, KeyCode::Esc);
        assert!(pane.title().starts_with("● draft.md"), "{}", pane.title());
        assert!(page(&mut pane, 60, 12).contains("never kept"));

        press(&mut pane, KeyCode::Char('x'));
        draw(&mut pane, 60, 12);
        press(&mut pane, KeyCode::Char('x'));
        assert!(pane.edit.is_none());
        assert!(matches!(pane.mode, Mode::Browse));
        assert_eq!(pane.path(), Some(path.as_path()), "the page it was named over");
        assert!(!fx.root().join("draft.md").exists());
    }

    #[test]
    fn a_file_that_appears_before_the_first_save_is_not_written_over_by_one_press() {
        let fx = Fx::new("create-appeared");
        let path = fx.write("README.md", b"# hi\n");
        let mut pane = fx.pane(&path);
        naming(&mut pane, "new.md");
        press(&mut pane, KeyCode::Enter);
        type_in(&mut pane, "mine");
        let made = fx.root().join("new.md");
        std::fs::write(&made, b"theirs\n").unwrap();

        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&made).unwrap(), b"theirs\n", "refused, not replaced");
        assert_eq!(pane.action_hint(), Some("◆ ctrl+s again overwrites"));
        draw(&mut pane, 60, 12);
        chord(&mut pane, 's');
        assert_eq!(std::fs::read(&made).unwrap(), b"mine");
    }

    #[test]
    fn a_new_file_s_unsaved_text_is_guarded_and_copied_out_like_any_other() {
        let fx = Fx::new("create-guarded");
        let path = fx.write("README.md", b"# hi\n");
        let mut pane = fx.pane(&path);
        naming(&mut pane, "notes/new.md");
        press(&mut pane, KeyCode::Enter);
        type_in(&mut pane, "unsaved");
        let target = fx.root().join("notes").join("new.md");
        assert!(!pane.show(&path), "nothing replaces it");
        pane.keystroke();
        let name = pane.label(&target);
        // As `F1, Q` asks it: the copy written first.
        pane.flush_draft();
        assert_eq!(pane.quit_note(), Some(format!("unsaved {name} kept as a recovery copy")));

        // Kept under a key that does not depend on the file being there: the
        // directory that is, resolved, and the rest of the name.
        quiet_is_over(&mut pane);
        pane.tick();
        let key = key_for(&target).unwrap();
        assert_eq!(fx.drafts().read(&key).unwrap().expect("a copy").text, "unsaved");
        assert_eq!(fx.drafts().read(&key).unwrap().expect("a copy").base, None);
        assert!(!fx.root().join("notes").exists(), "still nothing in the repository");

        // A later session naming the same file is offered it back.
        let mut pane = fx.pane(&path);
        naming(&mut pane, "notes/new.md");
        press(&mut pane, KeyCode::Enter);
        assert_eq!(text(&pane), "unsaved");
        assert!(edit(&pane).conflict.is_none(), "typed against no file, and there is none");
    }

    #[test]
    fn the_overlay_names_the_key_that_makes_a_file() {
        let (_, said) = crate::keys::HELP
            .iter()
            .find(|(key, _)| *key == "a (file list)")
            .expect("`a` is in the F1 overlay");
        assert!(said.contains("new file"), "{said}");
    }

    #[test]
    fn a_long_ago_is_said_in_the_words_a_person_uses() {
        let now = SystemTime::now();
        let ago_by = |secs: u64| ago(now.checked_sub(Duration::from_secs(secs)));
        assert_eq!(ago_by(5), "a moment ago");
        assert_eq!(ago_by(10 * 60), "10 minutes ago");
        assert_eq!(ago_by(3 * 3_600), "3 hours ago");
        assert_eq!(ago_by(3 * 86_400), "3 days ago");
        assert_eq!(ago(None), "an earlier session");
        assert_eq!(ago(now.checked_add(Duration::from_secs(60))), "a moment ago");
    }

    // --- what a frame costs ---------------------------------------------

    /// The spec's target, measured at the pane: a key typed into the middle of
    /// a README-sized markdown file, with a frame drawn after it, is one
    /// frame's work. The bound is a debug build's and generous; the number is
    /// printed for anyone measuring a release one.
    #[test]
    fn a_keystroke_in_a_long_file_costs_the_screen_and_not_the_file() {
        let fx = Fx::new("edit-cost");
        let para = "The retry budget is wrong, and the *reason* is in `load.rs`; \
                    see [the design](docs/design.md) for why.\n\n";
        let doc = para.repeat(46 * 1024 / para.len() + 1);
        assert!(doc.len() >= 46 * 1024);
        let path = fx.write("README.md", doc.as_bytes());
        let mut pane = fx.pane(&path);
        draw(&mut pane, 60, 20);
        press(&mut pane, KeyCode::Char('e'));
        let middle = edit(&pane).editor.line_count() / 2;
        if let Some(edit) = pane.edit.as_mut() {
            edit.editor.set_caret(middle, 10);
            edit.view.follow();
        }
        draw(&mut pane, 60, 20);

        let keys = "typing in the middle";
        let started = Instant::now();
        for c in keys.chars() {
            press(&mut pane, KeyCode::Char(c));
            draw(&mut pane, 60, 20);
        }
        let per_key = started.elapsed() / keys.len() as u32;
        eprintln!("a key and a frame, 46 KB of markdown: {per_key:?}");
        assert!(per_key < Duration::from_millis(250), "{per_key:?} per key");
    }
}
