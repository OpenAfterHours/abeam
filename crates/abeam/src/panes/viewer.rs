//! The file / markdown view.
//!
//! This is the pane that replaces opening an editor to read what an agent just
//! wrote. It is read-only, and deliberately: everything it does not do — no
//! editing, no shelling out, no text input — is what lets an unbound keystroke
//! here be harmless.
//!
//! ## How it stays out of the way
//!
//! The shell's watcher notices markdown changing under the repo root and hands
//! it here through [`ViewerPane::follow`]; the pane takes it up only while it is
//! the pane on screen. `render` is the one place that knows the viewer is
//! visible, so that is where a pending file is taken up. If the git view is
//! showing, the file waits and [`ViewerPane::has_pending`] tells the shell to
//! mark the border. Nothing here touches focus. A background agent that yanks
//! the pane out from under someone mid-read is delightful twice and infuriating
//! thereafter.
//!
//! The file list is the second half of that rule. Being drawn is no longer
//! enough on its own: while the list is up the pane is on screen and still
//! refuses the file, because someone walking a directory tree is *using* the
//! pane and a document appearing over it would be the same yank by another
//! route. Nothing switches the view under you, and `render` is where that is
//! enforced for both halves.
//!
//! The search box is the third, and it is held back for a sharper reason than
//! either. Someone typing a query is using the pane as surely as someone
//! walking a tree — but they are also typing *about this document*, and taking
//! a file up rebuilds every row, which invalidates every hit. The keystroke
//! they are half way through would land in a query over a file they never
//! asked for, and the count in the title would change under their fingers from
//! `3/7` to something about a document they cannot see. So a file waits while
//! the box is open, and is released by the key that closes it — the same shape
//! as the list, where `Alt+E` both leaves and releases.
//!
//! It waits for the box and not for the search. A closed search with its hits
//! still marked is somebody *reading*, which is the state the whole pane exists
//! to follow. What happens to that search when the file lands is one line drawn
//! once, in `show`, and it is the same line the reader's scroll place is drawn
//! on: the same document rewritten keeps the search and re-finds it, because an
//! agent rewriting what somebody is halfway through searching is the common
//! case; a *different* document ends it, because the hits are that document's
//! rows and the count was never true of any other.
//!
//! The results of a repository search are the fourth, and they are held back
//! for both of the reasons above at once. Somebody comparing forty matches is
//! using the pane as surely as somebody walking a tree; and the row they are
//! about to press `Enter` on would be replaced, mid-reach, by a document they
//! did not ask for.
//!
//! ## Three things to look at, one pane
//!
//! [`Mode`] is the document, the list, or the results of a search over every
//! file under the root. `Alt+E` — [`ViewerPane::toggle_browse`] — moves between
//! the first two in both directions, and `Enter` on a file moves one way,
//! because a list that stayed up after you chose something would need a second
//! key to show you what you chose. They are the same reading position from
//! either end: the list is how a file is reached, the document is what the list
//! is for.
//!
//! The results are a layer over whichever of those two raised them, which is
//! why they are the mode that carries something back — [`Back`]. `f` opens
//! them, `Esc` closes them onto the view they were opened from, and `Alt+E`
//! peels them off and then means exactly what it has always meant. Nothing else
//! changes the mode; in particular the watcher cannot, which is the rule above.
//!
//! They have one property the other two do not, and it is the genuinely new
//! risk in this pane: **the rows of the results list arrive from another
//! thread, between frames.** The document and the file list only ever change
//! because something on this thread changed them. A list that grows underneath
//! a reader is the same hazard as a file arriving underneath one, one level
//! down — and the answer is the same shape. `grep::absorb` appends and touches
//! the cursor with nothing at all, so the row under the selection stays the row
//! that was under it; [`list::Cursor`] carries the argument for why reacting to
//! each batch would pin the view and take the wheel away. That one line is what
//! holds it, which is why it is pointed at from here.
//!
//! The list itself lives in [`browse`] and the search in [`grep`], because a
//! selectable, filterable directory tree is a pane's worth of code on its own,
//! a worker thread that reads the repository is another, and this file is long
//! enough. What being *in* a list means — which row is chosen, and keeping it on
//! screen — is [`list`], one level down again: the directory listing, the find
//! over file names and the results of a search over their contents are three
//! lists in that one pane, and the bookkeeping they share is the part that goes
//! subtly wrong when it is written three times.
//!
//! ## Where the work happens
//!
//! Everything slow is on a worker thread, cached, or capped:
//!
//! - the gitignore-aware walk that builds the recency list and the find index
//!   runs on its own thread and reports through a channel `tick` polls,
//! - the sweep that answers `f` — reading every file the walk found, looking for
//!   a phrase — is the largest piece of work this pane can start and none of it
//!   is here. It runs on a long-lived thread of [`grep`]'s, reports in batches
//!   through a second channel the same `tick` drains, and is abandoned mid-file
//!   when the query is superseded. What bounds it is four caps, three of which
//!   it inherits: `files::MAX_ENTRIES` and `files::MAX_FILES` decide how much
//!   repository it can see, `load::MAX_BYTES` how much of each file, and
//!   `grep::MAX_HITS` how much it may report. It is also the reason `f` waits
//!   for `Enter` where `/` does not,
//! - the watcher is the shell's, on `notify`'s thread behind a debouncer,
//! - layout — parse, highlight, wrap — happens once per `(file, width)` pair
//!   and is cached, because `render` runs on every keystroke the agent sees,
//! - the file list's own directory read is the one gitignore walk that happens
//!   *on this thread*. It is cached between keys, never done from `render`, and
//!   bounded by `browse::MAX_ENTRIES`: one directory is a bounded amount of
//!   work in a way a repository is not, and that is the whole reason it is
//!   allowed here at all.
//!
//! What is left on the frame path is reading the file, laying it out, and
//! syntect's first-use initialisation. Layout is the expensive one and it
//! recurs: a new width means a new layout, so a window drag pays it per frame.
//! Both caps that bound it — `load::MAX_BYTES` and `source::HIGHLIGHT_MAX_BYTES`
//! — are set from measured time rather than from a round number, and are
//! documented where they are declared.
//!
//! ## Scrolling
//!
//! The document is pre-wrapped to the pane's exact width and scrolled by
//! physical row. That is why `crate::text::wrap` exists rather than
//! `Paragraph::wrap`: with
//! reflow at draw time the scroll offset and the widget's line count are
//! measured in different units, and every jump-to-end lands somewhere else.
//!
//! It is also what makes [`search`] possible in one implementation rather than
//! three. Those pre-wrapped rows are what `/` searches, so a hit is `(row,
//! character, length)` — already in the unit the pane scrolls by — and rendered
//! markdown, raw markdown and a highlighted source file are all just rows. What
//! that costs, and why the alternative does not exist, is [`search`]'s argument.
//!
//! And it is what [`grep`] has to be reconciled with. A search over files on
//! disk knows a *line*; this pane scrolls by *row*, and for rendered markdown
//! the two are not relatable at all. So `Enter` on a result carries an ordinal
//! rather than a position — the third match in that file is looked for again as
//! the third match on the page — and when the page turns out to have fewer, the
//! reader is told rather than parked at the top. See [`ViewerPane::show_at`]
//! and [`ViewerPane::missed`].

mod browse;
mod files;
mod grep;
mod list;
mod load;
mod markdown;
mod mermaid;
mod search;
mod source;
mod theme;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::pane::{Handled, Pane};
use crate::scroll::{self, Scroll};
use crate::text::{self, wrap};
use browse::Browser;
use files::Scan;
use grep::Grep;
use load::{LoadError, Loaded};
use search::{Margin, Search};

/// Assumed page size before the first frame has told us the real one.
const DEFAULT_VIEWPORT: usize = 20;

/// Line numbers cost four or five columns. Worth it in a normal right pane,
/// not worth it in a squeezed one.
const LINE_NUMBER_MIN_WIDTH: usize = 30;

/// Which of the three things the pane is showing. See the module doc.
#[derive(Clone, Copy)]
enum Mode {
    Doc,
    Browse,
    /// The search over every file under the root. Raised by `f` over either of
    /// the other two, and it remembers which — `Esc` out of it has to put the
    /// reader back where they pressed the key, and dropping somebody who was
    /// walking a directory into a document instead is the same yank the whole
    /// pane is built to avoid.
    Results {
        back: Back,
    },
}

/// Which of the two settled modes the results were raised over.
///
/// A second enum rather than a boxed `Mode`, because there are exactly two
/// answers and they are the two modes that are *not* the results: a `Mode`
/// inside a `Mode` would make `Results { back: Results { .. } }` representable,
/// and the thing it would mean is a pane that has to be `Esc`d out of twice
/// through a state nothing on screen ever showed.
#[derive(Clone, Copy)]
enum Back {
    Doc,
    Browse,
}

impl Back {
    fn mode(self) -> Mode {
        match self {
            Back::Doc => Mode::Doc,
            Back::Browse => Mode::Browse,
        }
    }
}

enum Body {
    Markdown(String),
    Source(String),
}

struct Doc {
    path: PathBuf,
    body: Body,
    truncated: bool,
    bytes: u64,
}

enum State {
    Empty,
    Doc(Doc),
    /// A path that could not be read. Kept rather than discarded so `r` has
    /// something to retry and the title can say which file went wrong.
    Failed {
        path: PathBuf,
        why: LoadError,
    },
}

pub struct ViewerPane {
    root: PathBuf,
    state: State,
    mode: Mode,
    browse: Browser,
    /// The search over every file under the root, and the list of what it
    /// found. Beside [`ViewerPane::browse`] rather than inside it, because it
    /// is a third view of the pane rather than a second view of the file list:
    /// its rows are places in files and not files, and `Enter` on one of them
    /// does something no row of the file list can ask for.
    grep: Grep,

    /// Markdown shown as the source it was rendered from. A property of the
    /// *pane* and not of the document, deliberately: `t` is a decision about
    /// how to read, and a reader who asked for source does not want the next
    /// save, the next `Tab` or the watcher to quietly put the rendering back.
    raw: bool,

    /// Light or dark, for the same reason and with the same scope as `raw`:
    /// a decision about how to read, held by the pane, surviving the next
    /// document. F3 flips it.
    theme: theme::Mode,

    /// The document laid out for `laid_out` columns. Rebuilt when either the
    /// document or the width changes, and never otherwise.
    lines: Vec<Line<'static>>,
    laid_out: usize,
    /// The line-number gutter those rows were drawn with, if any. Beside
    /// `laid_out` because it is a property of the same layout and is only ever
    /// right for the same one. See [`search::Margin`].
    margin: Margin,
    dirty: bool,

    /// A search over the rows above. `None` until `/`, and again once `Esc`
    /// has cleared the hits — "no search" and "a search whose query is empty"
    /// are different states, exactly as they are in the file list, and only one
    /// of them swallows `q` as a letter.
    ///
    /// Held beside `lines` and rebuilt with them: every hit in it is an index
    /// into that vector, so the two are only ever right together. `ensure_layout`
    /// is the single place that rebuilds one, which is why it is also the single
    /// place that refinds the other.
    search: Option<Search>,
    /// A phrase this pane is looking for in this document and cannot show, with
    /// the ordinal that was asked for.
    ///
    /// Not a [`Search`], and that is the point of it. A shut search with nothing
    /// marked is the one state this pane refuses to hold — see
    /// [`ViewerPane::settle_search`] — because `Esc` would then have a stage
    /// with nothing on screen to explain it. This is a line of title and not a
    /// stage: `Esc` does not see it, and it is dropped by the next document, by
    /// `/`, and by the `t` it names. `t` is the one key that *acts* on it, in
    /// [`ViewerPane::toggle_raw`], and [`ViewerPane::revive_search`] is what
    /// turns it back into a search when the document starts containing the
    /// phrase again.
    ///
    /// Two things produce it and the reader cannot tell them apart, which is
    /// right, because the sentence is the same. A query they typed that matches
    /// nothing. And `Enter` on a repository result, which opens a file and asks
    /// for that file's *n*th match — an ordinal counted over rows the grep never
    /// saw. Rendering markdown drops `**`, the URL behind a link and the
    /// backticks around code; and in *any* body a line wider than the pane is
    /// hard-broken, so a match straddling the break is one `f` reports and this
    /// search cannot find. Either way the reader would otherwise land at the top
    /// of a file they chose for a phrase, with the phrase nowhere on screen and
    /// nothing saying why.
    missed: Option<(String, usize)>,

    scroll: Scroll,

    /// A file the watcher noticed, waiting for the pane to be on screen.
    pending: Option<PathBuf>,
    /// The pane changed its own state inside [`Pane::render`], so the title the
    /// shell drew for this frame describes something other than the body under
    /// it: a document behind, when a pending file was taken up, or a scroll
    /// position behind, when a search jumped to a hit off screen.
    ///
    /// The shell renders the border — and asks this pane for its title — before
    /// it renders the pane, so the frame on which a pending document is first
    /// shown carries the *previous* state's title: `files`, over a page with a
    /// document on it. Every later frame is right, which is why this went years
    /// without being noticed. What makes it a bug rather than a cosmetic skew is
    /// that there is no promise of a later frame. `crate::app` draws when
    /// something asks it to, and a pane that changed its own state during a
    /// render has asked nobody.
    ///
    /// It surfaced on Linux, and only once `crate::watch` stopped reporting
    /// reads as writes: the events that used to arrive from abeam opening this
    /// very document kept the loop drawing, so the title corrected itself on the
    /// noise the pane itself generated. Take the noise away and a `sh` sitting
    /// idle at a prompt produces nothing else to draw for, and the wrong title
    /// stays on screen for the rest of the session.
    ///
    /// Answered on the next [`Pane::tick`], which is what "this pane wants to be
    /// redrawn" already means everywhere else — rather than by moving the take
    /// into `tick`, because being drawn is the only signal this pane gets that
    /// it is the view on screen, and that is the whole reason the take is where
    /// it is.
    owed: bool,
    /// Markdown under the root, newest first. `Tab` walks it.
    recent: Vec<PathBuf>,
    recent_ix: usize,

    scan: Option<Receiver<Scan>>,
    /// Whether the shell's watcher started. Display only — the pane says so on
    /// an empty screen rather than quietly never updating.
    watching: bool,
}

impl ViewerPane {
    pub fn new(root: PathBuf) -> Self {
        // The walk starts before the first frame so there is something to show.
        // The watcher is the shell's; it calls `set_watching` once it knows.
        let scan = Some(files::spawn_scan(root.clone()));
        let mut scroll = Scroll::default();
        scroll.measure(0, DEFAULT_VIEWPORT);
        Self {
            browse: Browser::new(root.clone()),
            grep: Grep::new(root.clone()),
            root,
            state: State::Empty,
            mode: Mode::Doc,
            raw: false,
            theme: theme::Mode::default(),
            lines: Vec::new(),
            laid_out: 0,
            margin: Margin::default(),
            dirty: true,
            search: None,
            missed: None,
            scroll,
            pending: None,
            owed: false,
            recent: Vec::new(),
            recent_ix: 0,
            scan,
            watching: false,
        }
    }

    /// Flip the reader between its light and dark palettes.
    ///
    /// The laid-out document holds baked styles, so this has to invalidate it —
    /// a palette that only took effect on the next file would look like a key
    /// that did nothing. Relaying it out is the same work a width change
    /// already costs, and it happens once per press rather than per frame.
    ///
    /// Scoped to this pane on purpose: the git and diagnostics views draw in
    /// named ANSI colours that already follow the terminal's own palette, and
    /// the two pty views are drawing whatever their child sent. This is the one
    /// pane whose colours are abeam's to choose.
    pub fn toggle_theme(&mut self) {
        self.theme = self.theme.flipped();
        self.dirty = true;
        self.browse.set_theme(self.theme);
        self.grep.set_theme(self.theme);
    }

    /// Start on a chosen palette, before anything has been drawn.
    ///
    /// The setter this pane deliberately did not have while there was nowhere
    /// to remember an answer: `F3` flips, and flipping needs no starting point
    /// beyond the default. `crate::config`'s `theme` key is that somewhere, so
    /// the pane is now told once — from `App::new`, before the first frame —
    /// and flipped for the rest of the session.
    ///
    /// It takes `crate::config`'s two-valued type rather than this module's
    /// [`theme::Mode`], which is the shorter of two changes: `Mode` carries the
    /// palettes as well as the choice and is private to the viewer, and
    /// publishing it to let a config file name a colour scheme would be
    /// exporting the colours to import a word. The mapping is these four lines
    /// and it is the whole of what the two types have to agree about.
    pub fn set_theme(&mut self, theme: crate::config::Theme) {
        let mode = match theme {
            crate::config::Theme::Dark => theme::Mode::Dark,
            crate::config::Theme::Light => theme::Mode::Light,
        };
        if self.theme != mode {
            self.theme = mode;
            // Same as `toggle_theme`: the laid-out document holds baked styles,
            // so a palette that only took effect on the next file would be a
            // setting that did nothing.
            self.dirty = true;
            self.browse.set_theme(mode);
            self.grep.set_theme(mode);
        }
    }

    /// Point the reader at another worktree.
    ///
    /// The [`Browser`] is rebuilt wholesale rather than given a `set_root` of
    /// its own, and that is the shorter of two changes as well as the safer
    /// one. `dir`, `entries`, `index`, `indexed`, `aligned`, `find` and
    /// `listing` are every one of them relative to the root — a setter would
    /// have to reset all seven, and the one it forgot would be silent.
    /// `aligned` is the sharpest of them: [`Browser::align_to`]
    /// short-circuits when the document has not changed, so a stale value would
    /// send the first `Alt+E` in the new workspace back into a directory of the
    /// old one.
    ///
    /// The palette is carried over by hand, because it is the one thing here
    /// that is a decision about *reading* rather than a fact about the root —
    /// the same reason `raw` and `theme` outlive a document.
    ///
    /// [`State::Empty`] is deliberate and reuses machinery that already exists
    /// rather than adding any: `tick` opens the newest markdown when the state
    /// is `Empty` and a scan lands, so a workspace switch behaves exactly like
    /// startup and the reader opens on the newest document *of the worktree it
    /// has moved to*.
    pub fn set_root(&mut self, root: PathBuf) {
        self.browse = Browser::new(root.clone());
        self.browse.set_theme(self.theme);
        // Rebuilt wholesale for the same reason and with one more of its own:
        // every row it holds is a path under the root that is going away, the
        // query it ran was about that tree, and the sweep that produced them
        // may still be reading it. Replacing the whole thing drops the request
        // channel, which is what ends that thread — where a `set_root` would
        // have to remember to bump the generation *and* clear the rows *and*
        // re-point the worker, and the one it forgot would be silent.
        self.grep = Grep::new(root.clone());
        self.grep.set_theme(self.theme);
        // A list of places in a tree that is no longer on screen is not a list
        // to leave somebody looking at. The mode is otherwise left alone —
        // somebody who was walking a directory is put in the new root's
        // directory, which is what `Browser::new` has just arranged.
        if let Mode::Results { back } = self.mode {
            self.mode = back.mode();
        }
        self.missed = None;
        self.root = root.clone();
        self.state = State::Empty;
        self.pending = None;
        // Otherwise `Tab` walks straight out of the workspace, into documents
        // of the tree that is no longer on screen.
        self.recent.clear();
        self.recent_ix = 0;
        self.scroll.to(0);
        // Rows of a document in a tree that is no longer on screen.
        self.search = None;
        self.dirty = true;
        // **Replaced, not re-requested.** `rescan` guards on `scan.is_none()`,
        // so calling it here would leave a walk of the *old* root in flight and
        // let its answer land in the new `Browser` as that workspace's index and
        // recency list. Dropping the receiver makes the old answer unreachable
        // rather than merely unwanted: the worker's `send` fails and nothing has
        // to remember to ignore it.
        self.scan = Some(files::spawn_scan(root));
    }

    /// Told once at startup, so the empty screen can admit it when there is no
    /// watcher rather than looking like a pane that simply never notices.
    pub fn set_watching(&mut self, on: bool) {
        if self.watching != on {
            self.watching = on;
            self.dirty = true;
        }
    }

    /// A file the watcher saw change. Queued rather than shown: taking it up is
    /// `render`'s job, because being drawn is the only way this pane learns it
    /// is the one on screen.
    pub fn follow(&mut self, path: PathBuf) {
        // Newest first, and never duplicated: the list is a recency order, not
        // a history.
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path.clone());
        self.recent_ix = 0;
        self.pending = Some(path);
    }

    /// Something is waiting to be shown. The shell asks so it can mark the
    /// border of a pane that is not the one on screen.
    ///
    /// That is not the only way a file comes to be waiting any more — the list
    /// holds one back while it is up, and the shell will not mark a view it is
    /// already showing — so `title` carries the mark in that one case. Both
    /// marks read from this.
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Point the pane at a file. The integration seam: the shell can call this
    /// with whatever an agent just touched.
    ///
    /// Cannot fail. A file that is missing, binary, locked or enormous becomes
    /// something the pane says rather than something the caller handles — the
    /// caller has nowhere to put an error, and the reader is better served by
    /// being told which file and why.
    pub fn show(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        // Whatever was queued is superseded. `show` means "this file is on
        // screen now", so by definition nothing is waiting to be — and the
        // queue is checked again by the very next `render`, which would
        // otherwise replace this file with the one the watcher happened to
        // mention first. Choosing a file in the list and being shown a
        // different one is the exact failure the pending queue exists to
        // prevent, arriving through the door the list opened. The same holds
        // for `Enter` in the git view and for `Tab`, where the shell can queue
        // and show within one pass of its loop.
        self.pending = None;
        // Re-showing the file already on screen is a reload, and a reload must
        // not throw away the reader's place. An agent rewriting a document
        // someone is halfway through is the common case, not the rare one.
        if self.path() != Some(path.as_path()) {
            self.scroll.to(0);
            // A search belongs to the document it was run over — the hits are
            // its rows and the count in the title is about it — so a *different*
            // document ends it. Carrying the highlighting across would mark
            // words in a file nobody has searched, under a count that was never
            // true of it.
            //
            // A reload is the opposite case and deliberately keeps it: the
            // agent rewriting the document somebody is searching is exactly
            // when the search should still be there afterwards. That is the
            // same line this branch already draws for the reader's place, which
            // is why it is drawn here and not in a second condition.
            self.search = None;
            // The notice is about a phrase and a document together, and one of
            // the two has just been replaced.
            self.missed = None;
            if let Some(ix) = self.recent.iter().position(|p| *p == path) {
                self.recent_ix = ix;
            }
        }

        self.state = match load::load(&path) {
            Ok(Loaded {
                text,
                truncated,
                bytes,
            }) => {
                let body = if crate::watch::is_markdown(&path) {
                    Body::Markdown(text)
                } else {
                    Body::Source(text)
                };
                State::Doc(Doc {
                    path,
                    body,
                    truncated,
                    bytes,
                })
            }
            Err(why) => State::Failed { path, why },
        };
        self.dirty = true;
    }

    /// Open a file at a match a repository search found in it: `Enter` on a
    /// result, and the only caller.
    ///
    /// **The one route allowed past `show`'s rule that a different document
    /// ends the search.** That rule is right for every other caller — the hits
    /// are the old document's rows and the count in the title was never true of
    /// the new one — and it is right *here* too, which is why the search is
    /// replaced rather than carried across. What survives is the reader's
    /// question, and it survives as a new search over the new document's rows,
    /// which is the only form of it that can be true. Written as "clear, then
    /// seed" rather than as a condition inside `show`, so that the exception is
    /// a line somebody can see at the call site instead of a branch every other
    /// caller has to be read past.
    ///
    /// An *ordinal* and not a line number, because the grep read the file's
    /// logical lines and this pane is about to lay out physical rows. Rendered
    /// markdown makes the gap obvious; a `.rs` hides it until a line is wider
    /// than the pane, and then the two disagree there too. See
    /// [`grep::Hit::ordinal`], [`ViewerPane::missed`] for what the reader is
    /// told when they do, and [`search`]'s module doc for the change that would
    /// retire the disagreement.
    ///
    /// A file that cannot be read is not seeded at all, and that is not a
    /// shortcut. `build` turns an unreadable path into rows of the pane's *own
    /// voice* — "no such file — it may have been renamed or deleted", "Tab for
    /// the next markdown file" — and a seed resolved against those would open on
    /// `a.txt · unreadable · /file · 1/2`, having found the reader's phrase
    /// twice in the apology for not having their file. The reader's own `/` may
    /// search that screen, deliberately and since Phase 1, because they can see
    /// what they are searching; a seed is the pane searching on their behalf for
    /// something it was told is in the *file*, and there is no file.
    fn show_at(&mut self, path: PathBuf, query: String, ordinal: usize) {
        self.show(path);
        if self.reader_has_a_document() {
            self.missed = None;
            self.search = Some(Search::seeded(query, ordinal));
        } else {
            self.missed = Some((query, ordinal));
        }
        // The list has answered its question. Staying in it after `Enter` would
        // mean pressing another key to see what was chosen — `browse`'s rule,
        // one list along.
        self.mode = Mode::Doc;
        // Laid out here rather than at the next frame, so that the title the
        // shell draws *for this frame* already knows whether the phrase is on
        // the page. Not an extra layout: it is the one the next frame was going
        // to do, moved earlier by a few microseconds, exactly as `toggle_raw`
        // moves it. Skipped before the first frame, where there is no width to
        // lay out for and `laid_out` would produce an empty document that every
        // phrase is missing from.
        if self.laid_out > 0 {
            self.ensure_layout(self.laid_out);
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.state {
            State::Empty => None,
            State::Doc(d) => Some(&d.path),
            State::Failed { path, .. } => Some(path),
        }
    }

    /// Re-read the file on screen. Bound to `r`, and the answer to a document
    /// that was mid-write when the watcher fired.
    pub fn reload(&mut self) {
        if let Some(path) = self.path().map(Path::to_path_buf) {
            self.show(path);
        }
    }

    /// Swap between the document and the file list. The seam a second `Alt+E`
    /// drives from the shell.
    pub fn toggle_browse(&mut self) {
        // The results are peeled off first, and then `Alt+E` does what it has
        // always done. It has meant "between the document and the file list"
        // since before there was a third thing to be in, and a key that meant
        // one of three things depending on which view happened to be up is a
        // key nobody can press without looking. So `f` from the document and
        // `Alt+E` leaves the document showing the list; `f` from the list and
        // `Alt+E` leaves the list showing the document.
        if let Mode::Results { back } = self.mode {
            self.mode = back.mode();
            self.grep.close_box();
        }
        self.mode = match self.mode {
            Mode::Doc => {
                // Open the list beside the document on screen. Whether that
                // means moving is [`Browser::align_to`]'s judgement, not ours:
                // it is the difference between a list that is a place and a
                // list that resets itself every time it is looked at.
                let doc = self.path().map(Path::to_path_buf);
                self.browse.align_to(doc.as_deref());
                // The box does not survive leaving the document; the hits do.
                // A box that stayed open over a list would keep `takes_input`
                // and `exit_hint` answering for a query nobody can see, and
                // coming back would resume typing into it. The hits are just
                // marks on a document that is still there, and marking it again
                // on the way back is what a reader who pressed `Alt+E` to look
                // something up expects to find.
                self.close_search();
                Mode::Browse
            }
            Mode::Browse => {
                // A query does not survive leaving the list. Coming back to a
                // stale one is never what the next `Alt+E` means, and it is
                // also what keeps `takes_input` and `exit_hint` answerable from
                // one fact instead of two that can disagree.
                self.browse.cancel_find();
                Mode::Doc
            }
            // Peeled off above, so this is the state the two lines up there
            // exist to make unreachable rather than a case with an answer.
            Mode::Results { back } => back.mode(),
        };
    }

    /// Leave the results, putting the reader back where `f` was pressed. `Esc`
    /// with nothing left to close, and the only way out that is not `Alt+E` or
    /// `Enter` on a row.
    fn leave_results(&mut self) {
        if let Mode::Results { back } = self.mode {
            self.mode = back.mode();
        }
    }

    /// `f`, from the document and from the file list.
    ///
    /// Pane-local, and exempt from the invariant at the top of `crate::keys`
    /// for the reason stated there: it is only ever delivered to a focused pane
    /// with the viewer showing, so no agent is listening for it. It is free in
    /// both places it is bound — neither `crate::scroll`'s table, nor
    /// `list::Cursor`'s, nor `browse`'s own arms, nor the document's claim it —
    /// and in both of the boxes those two views can raise it is a letter,
    /// because the box is asked first.
    fn open_results(&mut self, back: Back) -> Handled {
        self.mode = Mode::Results { back };
        self.grep.open();
        Handled::Yes
    }

    /// Is a find box open? The border and the paste route both ask, and both
    /// are asking about this instant rather than about the pane's type.
    fn finding(&self) -> bool {
        matches!(self.mode, Mode::Browse) && self.browse.finding()
    }

    /// Is the *document's* search box open? The other half of the same
    /// question, and the one that decides whether `q` is a letter.
    ///
    /// Mode-guarded like [`ViewerPane::finding`], because a search can outlive
    /// an `Alt+E` — its hits do — and a pane that reported the document's state
    /// while the list was on screen would promise `Esc` did something the list
    /// has never heard of.
    fn typing(&self) -> bool {
        matches!(self.mode, Mode::Doc) && self.search.as_ref().is_some_and(Search::typing)
    }

    /// Is the *repository* search's box open? The third of the same question,
    /// mode-guarded like the other two and for the same reason: this box's
    /// results outlive `Enter` on one of them, so the pane must not report a
    /// box as open while a document is what is on screen.
    fn grepping(&self) -> bool {
        matches!(self.mode, Mode::Results { .. }) && self.grep.typing()
    }

    /// Rendered markdown, or the source it was rendered from.
    ///
    /// Only markdown has two forms. On anything else this is a key the pane has
    /// no answer for and says so, rather than reporting that it acted and
    /// leaving a reader pressing something that visibly does nothing.
    fn toggle_raw(&mut self) -> Handled {
        if !matches!(&self.state, State::Doc(d) if crate::watch::is_markdown(&d.path)) {
            return Handled::No;
        }

        // The two layouts share no rows at all — rendering drops fence markers,
        // reflows prose and turns a table into a grid — so there is no offset
        // that means the same thing on both sides. The same *fraction* of the
        // document is the closest honest answer, and it is the one that keeps
        // `t` useful for what it is actually pressed for: checking the source
        // of the passage currently on screen. Left to `measure`'s clamp
        // instead, a reader near the end of a rendered document would be
        // dropped somewhere arbitrary in the middle of its source, which reads
        // as the toggle having lost their place rather than approximated it.
        let was = self.scroll.offset;
        let before = self.scroll.max();

        // `t` is the key [`ViewerPane::missed`] names as the answer, and this is
        // where it delivers on it: the search goes back, aimed at the same
        // ordinal, so the reader arrives at the match rather than at a form they
        // now have to search by hand for a phrase the pane already knows.
        //
        // In one direction only. The claim that makes this worth doing is that
        // the source form provably contains the phrase — the grep matched the
        // file's source text — and that claim runs rendered → source. Going the
        // other way is towards the one form that may have rendered the phrase
        // away entirely, so re-seeding there would be promising a match on the
        // strength of an argument for the opposite move. The notice survives the
        // trip instead, and is still true: `settle_search` will restate it, and
        // the remedy it names changes with the form under it.
        if !self.raw
            && let Some((query, ordinal)) = self.missed.take()
        {
            self.search = Some(Search::seeded(query, ordinal));
        }

        self.raw = !self.raw;
        self.dirty = true;
        // Laid out here rather than at the next frame, because the fraction has
        // to be applied against the row count the new form actually produces.
        // This is not an extra layout: it is the one the next frame was going
        // to do, moved earlier by a few microseconds.
        self.ensure_layout(self.laid_out);
        self.scroll
            .measure(self.lines.len(), self.scroll.viewport());
        let to = was
            .saturating_mul(self.scroll.max())
            .checked_div(before)
            // A form that fitted on screen has no fraction to carry over, and
            // was at the top of itself by definition.
            .unwrap_or(0);
        self.scroll.to(to);
        Handled::Yes
    }

    /// Start a walk of the repository, unless one is already running.
    ///
    /// At most one at a time. Key auto-repeat on `r` would otherwise start a
    /// gitignore walk per repeat tick, thirty a second, and throw away every
    /// answer but the last.
    fn rescan(&mut self) {
        if self.scan.is_none() {
            self.scan = Some(files::spawn_scan(self.root.clone()));
        }
    }

    /// Fold what the list did back into the pane.
    fn absorb(&mut self, out: browse::Outcome) -> Handled {
        match out {
            browse::Outcome::Ignored => Handled::No,
            browse::Outcome::Moved => Handled::Yes,
            // The list has re-read its own directory; the find index is the
            // pane's, and only a walk can refresh it. The walk is worth
            // starting whether or not the directory turned out to have changed
            // — but a frame is not, and a frame here re-renders the agent.
            browse::Outcome::Refreshed { changed } => {
                self.rescan();
                changed.into()
            }
            browse::Outcome::Open(path) => {
                self.show(path);
                // The list has answered its question. Staying in it after Enter
                // would mean pressing Alt+E to see the file just chosen.
                self.mode = Mode::Doc;
                Handled::Yes
            }
        }
    }

    /// Fold what the results list did back into the pane.
    fn absorb_result(&mut self, out: grep::Outcome) -> Handled {
        match out {
            grep::Outcome::Ignored => Handled::No,
            grep::Outcome::Moved => Handled::Yes,
            grep::Outcome::Leave => {
                self.leave_results();
                Handled::Yes
            }
            grep::Outcome::Open {
                path,
                query,
                ordinal,
            } => {
                self.show_at(path, query, ordinal);
                Handled::Yes
            }
        }
    }

    // --- layout ----------------------------------------------------------

    /// Lay the document out for `width`, if it is not already.
    ///
    /// The one place `lines` is rebuilt, and therefore the one place the hits
    /// can be refound. Nothing else rebuilds them: `t`, `F3`, `r`, `set_theme`,
    /// `set_root`, a reload, a file taken up by the watcher and a window drag
    /// all say so by setting `dirty` or by arriving with a new width, and then
    /// wait for this. Counting those routes and wiring each one would be a list
    /// that grows; this is the funnel they already go through, and a hit left
    /// over from the previous layout is a row index into a document that no
    /// longer exists — the highlight lands on the wrong words and `n` scrolls to
    /// the wrong place, both of them silently.
    ///
    /// It costs no extra layout, which is the point of putting it here rather
    /// than laying out again after the fact — re-wrapping a 512 KB document is
    /// ~210 ms and this pane draws on every keystroke the agent receives.
    fn ensure_layout(&mut self, width: usize) {
        if !self.dirty && width == self.laid_out {
            return;
        }
        (self.lines, self.margin) = self.build(width);
        self.laid_out = width;
        self.dirty = false;
        if let Some(search) = self.search.as_mut() {
            search.find(&self.lines, self.margin);
        }
        // A rebuild can leave an accepted search with nothing marked — a resize
        // that hard-breaks the word, or the agent deleting it — and this is
        // where that becomes the notice in [`ViewerPane::missed`].
        self.settle_search();
        self.revive_search();
    }

    /// The other half of [`ViewerPane::settle_search`]: a notice that has
    /// stopped being true becomes a search again.
    ///
    /// Without it the notice is written once and never re-examined, because the
    /// thing that would re-examine it — a `Search` and its `find` — is exactly
    /// what `settle_search` took away. So widening the pane until the wrap that
    /// split the phrase is gone, or `F3`, or `r`, or the agent putting the word
    /// back, would leave `· no match` in the title over a document with the
    /// match plainly on it. The pair is a cycle, and it is what makes the notice
    /// self-maintaining rather than a snapshot of one layout.
    ///
    /// It costs one `matches` pass per rebuild while a notice is up — the same
    /// pass a live search already pays for, and rebuilds are width changes and
    /// reloads rather than frames.
    ///
    /// **The reveal is deliberately disarmed.** [`Search::seeded`] arms it,
    /// because pressing `Enter` on a result is a reader asking to be taken
    /// somewhere; arriving here is a *rebuild*, which is the one thing
    /// `Search::find` refuses to move anybody for. Dragging a window would
    /// otherwise scroll a reader who is fifty rows further on back to a match
    /// they had finished with.
    fn revive_search(&mut self) {
        if self.search.is_some() || !self.reader_has_a_document() {
            return;
        }
        let Some((query, at)) = self.missed.clone() else {
            return;
        };
        let mut search = Search::seeded(query, at);
        search.find(&self.lines, self.margin);
        if search.hits().is_empty() {
            return;
        }
        search.take_follow();
        self.search = Some(search);
        self.missed = None;
    }

    /// The rows, and how much of them is the pane's own margin rather than the
    /// document. See [`search::Margin`].
    fn build(&self, width: usize) -> (Vec<Line<'static>>, Margin) {
        if width == 0 {
            return (Vec::new(), Margin::default());
        }
        let t = self.theme.theme();
        let plain = |lines| (lines, Margin::default());
        match &self.state {
            State::Empty => plain(empty_hint(width, self.watching, t)),
            State::Failed { path, why } => {
                let mut lines = vec![
                    Line::from(Span::styled(
                        self.label(path),
                        Style::default().fg(t.warn).add_modifier(Modifier::BOLD),
                    )),
                    Line::default(),
                ];
                lines.extend(text::block(&why.message(), width, t.dim()));
                lines.push(Line::default());
                // Naming Alt+G is a small layering leak — the globals are the
                // shell's — and it earns it. This screen is where someone
                // arrives without having asked to, and Tab walks the markdown
                // list, which is not where they came from.
                lines.extend(text::block(
                    "r to retry · Tab for the next markdown file · Alt+G for git",
                    width,
                    t.dim(),
                ));
                plain(lines)
            }
            State::Doc(doc) => {
                // Raw markdown goes through the same path as any other source
                // file, gutter and all: syntect has a Markdown grammar, and a
                // reader looking at the source of a document wants to see the
                // line numbers they are about to talk about.
                let (mut lines, gutter) = match &doc.body {
                    Body::Markdown(text) if !self.raw => {
                        (markdown::render(text, width, self.theme), 0)
                    }
                    Body::Markdown(text) | Body::Source(text) => {
                        source_lines(text, &doc.path, width, self.theme)
                    }
                };
                // Measured before the notice below is appended, because the
                // notice has no gutter and its first four columns are document.
                let margin = Margin {
                    width: gutter,
                    rows: lines.len(),
                };
                if doc.truncated {
                    lines.push(Line::default());
                    lines.extend(text::block(
                        &format!(
                            "— stopped at {} of {} —",
                            load::human(load::MAX_BYTES),
                            load::human(doc.bytes)
                        ),
                        width,
                        t.dim(),
                    ));
                }
                (lines, margin)
            }
        }
    }

    // --- searching the document -------------------------------------------

    /// Every key while the search box is open, and every printable one is a
    /// letter — `q`, `j` and `n` included.
    ///
    /// That is not a detail of this table, it is the whole of why the table
    /// exists. `App::handle_key` reads a `q` or an `Esc` the pane declined as
    /// "the user is done with this pane" and hands focus back to the agent, so
    /// a query with a `q` in it would have thrown the reader out of the pane
    /// mid-word. The list's find made the same promise before this one did;
    /// this is the second box in the same pane and the two agree on purpose.
    ///
    /// What is left over is spelled out rather than handed to `Scroll::key`,
    /// for the reason `browse::find_key` gives: a box that takes typing cannot
    /// also read `j` as "down". The keys that survive are the ones a query
    /// cannot contain — the arrows and Tab step between hits, `Ctrl+N`/`Ctrl+P`
    /// are the shape a reader already has in their fingers for a filter box,
    /// and the paging and jump keys still move the document, which is all that
    /// is left of the F1 promise once `j`, `g` and `space` are letters.
    fn search_key(&mut self, key: KeyEvent) -> Handled {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // Closes the box and keeps the hits. Only an Esc with nothing left
            // to close falls through to the shell — being thrown out of the
            // pane by the key that means "never mind" is the single most
            // annoying thing a box like this can do.
            KeyCode::Esc | KeyCode::Enter => {
                self.close_search();
                Handled::Yes
            }

            KeyCode::Char('n') if ctrl => self.step_hit(true),
            KeyCode::Char('p') if ctrl => self.step_hit(false),
            // The paging keys keep working in here, because the F1 overlay is
            // one table for the whole program and an open query is not a reason
            // for a documented key to go quietly dead.
            KeyCode::Char('d') if ctrl => self.scroll_searching(key),
            KeyCode::Char('u') if ctrl => self.scroll_searching(key),
            // Ctrl+letter is the agent's everywhere else in the program, so the
            // rest must not fall into the plain-letter arm below.
            KeyCode::Char(_) if ctrl => Handled::No,

            KeyCode::Char(c) => {
                if let Some(search) = self.search.as_mut() {
                    search.push(c);
                    search.find(&self.lines, self.margin);
                    // Every keystroke re-aims from where the reader was, which
                    // is what "the view jumps to the first hit at or after the
                    // current position" has to mean once the previous keystroke
                    // has already moved the view.
                    search.aim();
                }
                Handled::Yes
            }
            KeyCode::Backspace => {
                let Some(search) = self.search.as_mut() else {
                    return Handled::No;
                };
                if search.pop() {
                    search.find(&self.lines, self.margin);
                    search.aim();
                } else {
                    // Backspacing past the start of the query leaves the box
                    // *and* takes the search with it: those keystrokes came
                    // from opening it, and there is nothing to keep. The same
                    // rule `browse.rs` follows, so the two boxes cannot drift.
                    self.search = None;
                }
                Handled::Yes
            }

            KeyCode::Down | KeyCode::Tab => self.step_hit(true),
            KeyCode::Up | KeyCode::BackTab => self.step_hit(false),
            // The keys the F1 overlay calls "scroll a page" and "jump to top /
            // bottom" go on meaning that, because `g`, `G`, `space` and `b` are
            // letters in here and these are all that is left of the promise.
            // The list's find sends `Home`/`End` to its *selection* instead,
            // and the two differ because what is under the box differs: there
            // it is a list of files with ends, here it is a document.
            KeyCode::PageDown | KeyCode::PageUp | KeyCode::Home | KeyCode::End => {
                self.scroll_searching(key)
            }

            _ => Handled::No,
        }
    }

    /// `/`. Opens the box on the row the reader is looking at, which is where
    /// the first hit is looked for.
    fn open_search(&mut self) -> Handled {
        self.search = Some(Search::open(self.scroll.offset));
        // A question of the reader's own supersedes the one the pane was
        // carrying for them, and two `/`-prefixed phrases in one title would be
        // two answers to one question.
        self.missed = None;
        Handled::Yes
    }

    /// Close the box, keeping the hits.
    fn close_search(&mut self) {
        if let Some(search) = self.search.as_mut() {
            search.accept();
        }
        self.settle_search();
    }

    /// A closed search with nothing marked is not a state this pane has. It
    /// becomes a [notice](ViewerPane::missed) instead.
    ///
    /// The *search* going is what makes `Esc` three states rather than four.
    /// Shut with no hits, the reader sees no highlighting at all, so an `Esc`
    /// that stopped there would be a keypress eaten by a stage nothing on screen
    /// mentions, and `exit_hint` could not describe it truthfully either.
    ///
    /// What was wrong with simply dropping it is the other half. The reader had
    /// a query and a `· no match` in the title; a resize that hard-breaks the
    /// matched word took the search away and the sentence explaining it with the
    /// same keystroke, so the pane went from answering a question to silence
    /// without anyone asking it to. The notice keeps the sentence and keeps
    /// none of the machinery: no hits to be stale, no `Esc` stage, no `q` in a
    /// box. [`ViewerPane::revive_search`] is what turns it back.
    ///
    /// Called from every route that can produce the state, which is the part
    /// that needs saying because `Esc` is only the first of them. `Esc` and
    /// `Enter` close the box; `Alt+E` closes it on the way to the list;
    /// `ensure_layout` can take the last hit away from a search closed minutes
    /// ago; and `Enter` on a repository result seeds one whose phrase this
    /// document may not contain at all. Guarding only where the state was first
    /// noticed would have left the invariant true of that route and false of the
    /// others.
    /// Is there a *document* under the rows, rather than the pane talking about
    /// itself?
    ///
    /// The empty hint and the unreadable notice are rows like any other, and the
    /// reader's own `/` searches them — deliberately, and since Phase 1, because
    /// they can see what they are searching and a box with nothing in the title
    /// to show for it is worst on exactly those two screens. A phrase the *pane*
    /// is carrying on their behalf is a different thing: it was found in a file,
    /// and the answer to "is it here" has to be about that file rather than
    /// about the apology for not having it. Both places that carry one ask this,
    /// so the seed and its revival cannot drift into disagreeing.
    fn reader_has_a_document(&self) -> bool {
        matches!(self.state, State::Doc(_))
    }

    fn settle_search(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        if search.typing() || !search.hits().is_empty() {
            return;
        }
        // The ordinal travels with the phrase, and it is not decoration: a seed
        // that this form of the document could not honour is about to be offered
        // `t`, and `t` has to ask the other form for the *same* match rather
        // than for the first one.
        self.missed = Some((search.query().to_string(), search.at()));
        self.search = None;
    }

    fn step_hit(&mut self, forward: bool) -> Handled {
        match self.search.as_mut() {
            Some(search) => search.step(forward),
            None => Handled::No,
        }
    }

    /// Scroll the document from inside the box, and take the anchor with it.
    ///
    /// Without the second half, a reader who paged down and then typed another
    /// letter would be thrown back to wherever they were when they pressed `/`.
    /// The anchor means "where the reader put the view", and this is one of the
    /// two ways they put it somewhere — the other is `scroll_key`.
    fn scroll_searching(&mut self, key: KeyEvent) -> Handled {
        let handled = self.scroll.key(key).unwrap_or(Handled::No);
        if handled.is_yes()
            && let Some(search) = self.search.as_mut()
        {
            search.set_anchor(self.scroll.offset);
        }
        handled
    }

    /// Bring the current hit on screen, if it has moved since the last frame.
    ///
    /// Called from `render` and nowhere else, because how tall the pane is can
    /// only be learned from a frame — the same reason `list::Cursor` splits its
    /// reveal in two. A hit already on screen moves nothing at all, so `n`
    /// between two visible matches does not scroll the page out from under the
    /// paragraph being read.
    ///
    /// A hit that is *not* on screen is centred rather than scrolled minimally
    /// to. `list::Cursor` does the opposite for a selection, deliberately, and
    /// the two differ because the moves differ: a selection moves a row at a
    /// time and a search jumps. Landing a jump on the last row of the pane puts
    /// the matched sentence at the bottom edge with nothing after it, and the
    /// reason to jump to a match is to read on from it — and to see enough
    /// either side to tell whether it is the match that was wanted.
    ///
    /// Reports whether it moved, because a move here is a move made *inside* a
    /// render and the title above it was drawn before it happened. See `owed`.
    fn reveal_hit(&mut self) -> bool {
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        if !search.take_follow() {
            return false;
        }
        let Some(hit) = search.current() else {
            return false;
        };
        let page = self.scroll.viewport().max(1);
        if hit.row >= self.scroll.offset && hit.row < self.scroll.offset + page {
            return false;
        }
        self.scroll.to(hit.row.saturating_sub(page / 2)).is_yes()
    }

    /// Paint the hits onto rows that have already been cloned out of `lines`.
    ///
    /// `rows` is `render`'s copy of the visible slice, so the highlighting never
    /// reaches the cache: a keystroke that changes which hit is current repaints
    /// a screenful of spans and leaves `dirty` alone, where marking the layout
    /// stale would re-wrap the whole document — ~210 ms at the cap — for a
    /// change of colour.
    /// The hits are in reading order, so the ones on screen are a contiguous
    /// run and the search for its start is a binary one. That matters at the
    /// count a one-character query reaches — `e` over a 512 KB document is
    /// 39,710 hits — where walking the vector to paint the dozen that are
    /// visible would put the whole of it on the frame path.
    fn highlight(&self, rows: &mut [Line<'static>], from: usize) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let t = self.theme.theme();
        let now = search.current();
        let hits = search.hits();
        for hit in &hits[hits.partition_point(|h| h.row < from)..] {
            let Some(line) = rows.get_mut(hit.row - from) else {
                break;
            };
            text::restyle(line, hit.start, hit.len, t.hit(Some(*hit) == now));
        }
    }

    // --- the file list ----------------------------------------------------

    fn step(&mut self, forward: bool) -> Handled {
        if self.recent.is_empty() {
            // Nothing to step to, so nothing was acted on. Reporting `Yes` here
            // spent a whole frame — the agent's screen included — on a key that
            // did nothing.
            return Handled::No;
        }
        let n = self.recent.len();
        // Only advance from the current file if it is actually in the list;
        // otherwise `Tab` after a `show()` from outside should start at the top.
        self.recent_ix = if self.path().is_some() {
            if forward {
                (self.recent_ix + 1) % n
            } else {
                (self.recent_ix + n - 1) % n
            }
        } else {
            0
        };
        let next = self.recent[self.recent_ix].clone();
        self.show(next);
        Handled::Yes
    }

    /// Path as the user thinks of it: relative to the repo root.
    fn label(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    fn position(&self) -> String {
        let max = self.scroll.max();
        if max == 0 {
            return "all".into();
        }
        if self.scroll.offset >= max {
            return "end".into();
        }
        format!("{}%", self.scroll.offset * 100 / max)
    }
}

impl Pane for ViewerPane {
    fn title(&self) -> String {
        // Same rule as the list one branch down, and the same `◆`: this pane is
        // on screen and still holding a file back, so the shell has no border
        // to mark and the pane says it itself. What releases the file here is
        // `Esc` or `Alt+E` — whichever gets the reader out of the results —
        // rather than one named key, which is why the mark is bare.
        if matches!(self.mode, Mode::Results { .. }) {
            let mark = if self.pending.is_some() { "◆ " } else { "" };
            return format!("{mark}{}", self.grep.title());
        }
        if matches!(self.mode, Mode::Browse) {
            // The one place this pane marks a pending file itself, and it has
            // to be: the shell draws `◆ Alt+E` only on the border of the view
            // that is *not* showing, and the list is the one state where this
            // pane is showing and still holding a file back. There is no
            // double mark to worry about — the shell renders the title of
            // whichever view is up, so this string is never drawn while git is.
            // `Alt+E` is exactly the key that releases the file, which is what
            // makes the shell's wording the right wording here too.
            let mark = if self.pending.is_some() { "◆ " } else { "" };
            return format!("{mark}{}", self.browse.title());
        }
        // The document view marks a pending file in exactly one state, and for
        // the same reason the list does: normally a file waiting here could
        // never be seen, because by the time this pane renders its own title it
        // has already taken one up. While the search box is open it does not,
        // so this is where the reader is told — and, as in the list, the key
        // that closes the box is the key that releases the file.
        let mark = if self.typing() && self.pending.is_some() {
            "◆ "
        } else {
            ""
        };
        // `/` works on all three screens, so the query has to be visible on all
        // three. The empty hint and the unreadable notice are real rows — they
        // are found in and highlighted like any other — and a box the reader is
        // typing into with nothing on screen to show for it is worse there than
        // in a document, because those are the two screens somebody arrives at
        // without having asked to.
        //
        // What to do about a phrase that is not on the page — and there is an
        // answer for *every* body, which there was not when this only knew about
        // markdown. `f` reports matches over a file's logical lines and this
        // pane searches the physical rows they were wrapped into, so a line
        // wider than the pane hides a match from `/` in a plain `.rs` exactly as
        // rendering hides one in markdown. A notice naming no way out at all was
        // the worse half of that: the reader is told the phrase is not here and
        // given nothing to press.
        //
        // The remedy goes on the miss rather than beside it, and each body gets
        // the one that is true of it. Rendered markdown names `t`, because the
        // commonest reason to find nothing in a rendering is that the thing was
        // rendered away and `t` is where it went. Everything else names the
        // width, because a wrap is the only way a phrase the sweep found can be
        // missing from the rows it was wrapped into.
        let miss = match &self.state {
            State::Doc(doc) if matches!(doc.body, Body::Markdown(_)) && !self.raw => {
                " · t for source"
            }
            State::Doc(_) => " · widen if a wrap split it",
            _ => "",
        };
        // Two things can be looking for a phrase in this document and only one
        // of them is a `Search`. The other is a phrase `Enter` on a repository
        // result sent the pane to find and the document could not show — see
        // [`ViewerPane::missed`] — and it borrows this slot rather than growing
        // one of its own, because it is the same sentence about the same
        // question and the reader has no reason to care which of the two
        // mechanisms is producing it.
        let find = match (&self.search, &self.missed) {
            (Some(search), _) => format!(" · {}", search.label(miss)),
            (None, Some((query, _))) => format!(" · {}", search::no_match(query, miss)),
            (None, None) => String::new(),
        };
        match &self.state {
            State::Empty => format!("{mark}files{find}"),
            State::Failed { path, .. } => {
                format!("{mark}{} · unreadable{find}", self.label(path))
            }
            State::Doc(doc) => {
                let trunc = if doc.truncated { " · truncated" } else { "" };
                // Which of the two forms is on screen, but only for the file
                // that has two: saying "rendered" about a `.rs` would be
                // advertising a toggle that is not there.
                let form = match &doc.body {
                    Body::Markdown(_) if self.raw => " · source",
                    Body::Markdown(_) => " · rendered",
                    Body::Source(_) => "",
                };
                // Read left to right, this is what the reader gives up last
                // first: the file name, then which form of it, then the query,
                // then where they are in it. A title is clipped from the right,
                // and `· rendered` has to outlast the query rather than trail
                // it — it is the answer to "why can I not find `**`", and at
                // the 46 columns this pane is routinely given, sitting second
                // to last meant it was clipped off exactly when a long query
                // was missing, which is the one moment it exists for.
                format!(
                    "{mark}{}{form}{find}{trunc} · {}",
                    self.label(&doc.path),
                    self.position()
                )
            }
        }
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // The page, before anything is written on it.
        //
        // This is the one pane in abeam that paints a background rather than
        // letting the terminal's show through, and it is what makes F3 worth
        // having: a reader in a bright room gets a bright page without also
        // having to reconfigure their terminal. ratatui styles are patches, so
        // one fill here is enough — every span drawn on top names a foreground
        // and inherits this background, and the rows with no text on them at
        // all keep both. The fill covers the whole rect including the scrollbar
        // column, which is why it is not folded into the `Paragraph` below.
        f.render_widget(Block::new().style(self.theme.theme().base()), inner);

        if matches!(self.mode, Mode::Browse) {
            // A pending file stays pending. Being drawn is the signal that this
            // pane is on screen, but in the list it is on screen *and in use*:
            // replacing a directory someone is walking with a document an agent
            // just wrote is the same yank the pending queue exists to prevent,
            // arriving through the one door it was never closed on.
            self.browse.render(f, inner);
            return;
        }

        // The third door, closed the same way. Somebody reading a list of
        // matches is using the pane exactly as somebody walking a directory is,
        // and here the yank would be sharper than either: the reader is part
        // way through checking which of forty results is the one, and the row
        // they were about to press `Enter` on would be replaced by a document
        // they never asked for.
        if matches!(self.mode, Mode::Results { .. }) {
            self.grep.render(f, inner);
            return;
        }

        // Being drawn *is* the signal that this pane is the one on screen, and
        // it is the only such signal a pane gets. Auto-follow happens here for
        // exactly that reason — and, as in the list, being on screen is not
        // enough while the pane is being *used*. A file arriving under an open
        // search box would rebuild every row and take every hit with it, mid
        // query. See the module doc.
        if !self.typing()
            && let Some(path) = self.pending.take()
        {
            self.show(path);
            // The border above this rect was drawn from the state that existed
            // a moment ago, which is no longer the state under it. See `owed`.
            self.owed = true;
        }

        // The column is reserved whether or not the bar is drawn: deciding per
        // frame would re-wrap the whole document every time a scrollbar
        // appeared, and the text would jump sideways as you scrolled.
        let text_width = inner.width - scroll::bar_width(inner.width);

        self.ensure_layout(text_width as usize);
        self.scroll.measure(self.lines.len(), inner.height as usize);
        // After `measure`, which is the moment the pane's height is known, and
        // before the slice below is taken from the offset it may move. A jump
        // to a hit off screen changes the position this pane's own title
        // reports, and that title was drawn before this call — the same skew,
        // and the same answer, as a document taken up above. See `owed`.
        self.owed |= self.reveal_hit();

        let start = self.scroll.offset;
        let end = (start + inner.height as usize).min(self.lines.len());
        let mut visible = self.lines[start.min(end)..end].to_vec();
        // Onto the copy. `visible` is already a clone of the cached rows, so
        // the highlighting is spliced into something that dies with this frame
        // and `dirty` is never touched.
        self.highlight(&mut visible, start);
        f.render_widget(
            Paragraph::new(visible),
            Rect {
                width: text_width,
                ..inner
            },
        );
        self.scroll.render_bar(f, inner);
    }

    fn tick(&mut self) -> bool {
        // A document taken up during the last render, whose title never made it
        // onto the screen the body did. Claimed first and unconditionally, so it
        // is owed exactly one frame however the rest of this goes.
        let mut changed = std::mem::take(&mut self.owed);

        // Whatever the worker has found since the last frame. A `try_recv`
        // drain and nothing else: this runs on the thread that pumps the agent's
        // pty.
        //
        // Drained whatever the mode, so the channel never backs up and the list
        // is complete when the reader comes back to it — but a frame is only
        // owed when those rows are the thing on screen. `Enter` on a result
        // leaves a sweep of twenty thousand files still running behind a
        // document, and every batch of it would otherwise re-render the agent's
        // whole screen for rows nobody can see, which is the cost `grep::Outcome`
        // exists to keep honest.
        //
        // The sweep is not cancelled to achieve that, and the difference
        // matters: cancelling would make the list permanently partial, so the
        // `f` that brings the reader back would show an answer that stopped
        // wherever they happened to press `Enter` — and the title has no honest
        // way to say so, since nothing was capped. The harm is the frames; this
        // is the frames.
        changed |= self.grep.tick() && matches!(self.mode, Mode::Results { .. });

        // The walk answers once, then the receiver is dropped.
        if let Some(found) = self.scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.scan = None;
            // One list, two readers and a worker. At `files::MAX_FILES` this is
            // twenty thousand strings, and the `Arc` is what keeps it from
            // being twenty thousand strings three times over — and what lets
            // the grep's worker keep sweeping the list it started on while a
            // later walk replaces it here.
            let files: Arc<[String]> = found.files.into();
            self.browse.set_index(Arc::clone(&files));
            // The grep is told whether the walk saw the whole tree as well as
            // what it found, because neither of the walk's own caps can be seen
            // from the list: `files::MAX_ENTRIES` counts entries visited rather
            // than files kept, so a truncated walk can hand over a short list
            // that looks complete. A count over it would otherwise be a definite
            // answer about a repository nothing finished reading.
            self.grep.set_index(files, found.cut);
            self.recent = found.recent;
            self.recent_ix = 0;
            // Nothing has been asked for yet, so open the newest thing there
            // is. A pane that starts empty when the repo is full of documents
            // reads as broken.
            if matches!(self.state, State::Empty)
                && let Some(newest) = self.recent.first()
            {
                self.pending = Some(newest.clone());
            }
            changed = true;
        }

        changed
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        // The results own every key while they are up, for the reason the list
        // does one branch down — and `f` is claimed here rather than delegated
        // because it is the key that opened this view and the key that reopens
        // its box, which is a fact about the pane's modes rather than about the
        // grep.
        if matches!(self.mode, Mode::Results { .. }) {
            let out = self.grep.key(key);
            return Ok(self.absorb_result(out));
        }

        // The list owns every key while it is up, including the scroll ones:
        // there they move a selection rather than an offset, and a pane cannot
        // hand the same key to two vocabularies and hope.
        if matches!(self.mode, Mode::Browse) {
            // Before the list is offered the key, and only while no box of its
            // own is open — in there `f` is a letter of a filename, and the
            // list is asked first about every other printable key for exactly
            // that reason.
            if !self.browse.finding() && bare(key, 'f') {
                return Ok(self.open_results(Back::Browse));
            }
            let out = self.browse.key(key);
            return Ok(self.absorb(out));
        }

        // The search box owns every key while it is open, and it has to be
        // asked before the scroll vocabulary: in there `j` is a letter and `q`
        // is a letter, and a pane cannot hand the same key to two vocabularies
        // and hope. This is the document's half of the rule the list already
        // follows two branches up.
        if self.typing() {
            return Ok(self.search_key(key));
        }

        // Deliberately the same vocabulary as Claude's own transcript view, and
        // as the other two panes, so the app has one way to scroll rather than
        // three that drift.
        if let Some(handled) = self.scroll.key(key) {
            // Where the reader has put the view is where the next query looks
            // from — see `Search::anchor`.
            if handled.is_yes()
                && let Some(search) = self.search.as_mut()
            {
                search.set_anchor(self.scroll.offset);
            }
            return Ok(handled);
        }

        let handled = match key.code {
            // `Ctrl` plus a letter is the agent's everywhere in this program,
            // and this is the arm that keeps it so. `crate::scroll::key` hands
            // it *back* rather than declining it — deliberately, so that a
            // pane's own table is where the decision gets made — and every
            // plain-letter arm below would otherwise take it: `Ctrl+R` reloaded
            // the document and started a gitignore walk of the repository, for
            // a chord aimed at the agent. `browse.rs` and `list.rs` have said
            // this since they were written; the document view had the same hole
            // and no arm to close it.
            KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::No,

            KeyCode::Tab => self.step(true),
            KeyCode::BackTab => self.step(false),
            KeyCode::Char('t') => self.toggle_raw(),

            // Pane-local, and exempt from the invariant at the top of
            // `crate::keys` for the reason `w` is in the git view: they are
            // only ever delivered to a focused pane with the document showing,
            // so no agent is listening for them. `/` mirrors the file list's
            // `/`; `n` and `N` are only outside the box, where inside it they
            // are letters.
            KeyCode::Char('/') => self.open_search(),
            // The third search, and the only one that reads the disk. `/` is
            // this document, `f` is every file there is; the two sit next to
            // each other on the same screen because the question a reader has
            // is often the second one after the first has come up empty.
            _ if bare(key, 'f') => self.open_results(Back::Doc),
            KeyCode::Char('n') => self.step_hit(true),
            KeyCode::Char('N') => self.step_hit(false),
            // The middle of the three states `Esc` passes through: the box is
            // shut and the hits are still marked, and this is what clears them.
            // Only an `Esc` with neither falls through to the shell as "give
            // focus back to the agent".
            KeyCode::Esc if self.search.is_some() => {
                self.search = None;
                Handled::Yes
            }

            KeyCode::Char('r') | KeyCode::Enter => {
                self.reload();
                // The only way a file created since startup joins the list if
                // the watcher could not start.
                self.rescan();
                Handled::Yes
            }

            // Esc and q are not ours. The shell reads an unhandled one as
            // "give focus back to the agent", which is the way out of here.
            _ => Handled::No,
        };
        Ok(handled)
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        if matches!(self.mode, Mode::Results { .. }) {
            let out = self.grep.mouse(ev);
            return Ok(self.absorb_result(out));
        }
        if matches!(self.mode, Mode::Browse) {
            let out = self.browse.mouse(ev);
            return Ok(self.absorb(out));
        }
        let handled = self.scroll.mouse(ev).unwrap_or(Handled::No);
        // The wheel is the third way the reader puts the view somewhere, and
        // the next letter of a query looks from wherever that is.
        if handled.is_yes()
            && let Some(search) = self.search.as_mut()
        {
            search.set_anchor(self.scroll.offset);
        }
        Ok(handled)
    }

    /// `Alt+J`/`Alt+K`/`Alt+PgDn`/`Alt+PgUp`, arriving as the bare key.
    ///
    /// In the document view that is the same key this pane would have handled
    /// anyway. In the list it is not: there `Down` moves the *selection*, and a
    /// glance is a read — the whole point of the binding is that it costs no
    /// focus round trip, so it must not quietly re-aim the `Enter` the reader
    /// presses when they get here. It moves the view alone.
    fn scroll_key(&mut self, key: KeyEvent) -> Result<Handled> {
        // The results are a list, so the same rule again: `Down` in there moves
        // the row `Enter` would open, and a glance from the other side of the
        // window must not re-aim it.
        if matches!(self.mode, Mode::Results { .. }) {
            return Ok(self.grep.scroll_view(key));
        }
        if matches!(self.mode, Mode::Browse) {
            return Ok(self.browse.scroll_view(key));
        }
        // With the box open, `handle_key` would type the glance into the query
        // — `Alt+J` arrives as a bare `Down`, and `Down` in there steps between
        // hits. A glance is a read from the other side of the window and must
        // move the view and nothing else, which is the whole reason this method
        // exists; the list has said so since it grew a find box of its own.
        if self.typing() {
            return Ok(self.scroll_searching(key));
        }
        self.handle_key(key)
    }

    /// Only while a query is being typed — into any of the three boxes.
    /// Everything else this pane does is a read; in a box `j` is a letter, and
    /// both things the shell asks this for — where a paste goes, and whether
    /// leaving hands focus back to the agent — turn on exactly that.
    fn takes_input(&self) -> bool {
        self.finding() || self.typing() || self.grepping()
    }

    /// Seven answers, and the border has to be true in every one of them
    /// because it is the only place the way out is written down. It names what
    /// *this* press does, not where the sequence ends, which is what makes a
    /// three-press `Esc` describable one press at a time.
    ///
    /// The results, which are a layer over one of the other two views, are worth
    /// three of the seven. `esc→results` closes their box onto the matches
    /// behind it. With nothing behind it — the box has never been run — the same
    /// press leaves the whole view, and where it leaves *to* is the half a
    /// single answer could not have covered: `esc→list` when `f` was pressed in
    /// the file list, `esc→page` when it was pressed in the document. Dropping
    /// somebody who was walking a directory into a document instead is the yank
    /// this pane exists to avoid, and the border must not promise it.
    ///
    /// `esc→page` and not `esc→document`, because `/` calls the document view
    /// "the page" and the border has one vocabulary or it has none.
    ///
    /// The other four are older. `Esc` in the list's find closes it and leaves
    /// you in the list, one press short of the agent. In the document's box it
    /// closes the box and keeps the hits — unless there are none to keep, where
    /// the same press ends the search outright and saying `esc→hits` would be
    /// promising something that is not there. With a file waiting it does
    /// something else again: closing the box releases the file on the very next
    /// frame, and the document under the hits is about to be a different one, so
    /// it names the file rather than the hits. The `◆` this pane has already put
    /// in the title is what makes that read.
    ///
    /// A [notice](ViewerPane::missed) is deliberately not one of the seven. It
    /// is a sentence in the title with no state behind it, so `Esc` passes
    /// straight through it to the shell and the answer is `esc→agent` — which is
    /// exactly what happens.
    fn exit_hint(&self) -> &'static str {
        if let Mode::Results { back } = self.mode {
            return match (self.grep.typing(), self.grep.has_results(), back) {
                (true, true, _) => " · esc→results",
                (_, _, Back::Browse) => " · esc→list",
                (_, _, Back::Doc) => " · esc→page",
            };
        }
        if matches!(self.mode, Mode::Browse) {
            return if self.browse.finding() {
                " · esc→list"
            } else {
                " · esc→agent"
            };
        }
        match &self.search {
            Some(s) if s.typing() && self.pending.is_some() => " · esc→new file",
            Some(s) if s.typing() && s.hits().is_empty() => " · esc→clear",
            // "the hits", not "the document": the reader is already in the
            // document, and what the press buys them is that the marks survive
            // it.
            Some(s) if s.typing() => " · esc→hits",
            Some(_) => " · esc→clear",
            None => " · esc→agent",
        }
    }

    /// Pasted text goes into whichever box is open, and nowhere else.
    ///
    /// Both are somewhere a read-only pane can put text, which is what
    /// `takes_input` has just promised the shell. A path pasted out of the
    /// agent's transcript is a likely way to reach a file; a phrase pasted out
    /// of it is a likely way to find where the agent got that phrase from.
    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        if matches!(self.mode, Mode::Results { .. }) {
            let out = self.grep.paste(text);
            return Ok(self.absorb_result(out));
        }
        if matches!(self.mode, Mode::Browse) {
            let out = self.browse.paste(text);
            return Ok(self.absorb(out));
        }
        if !self.typing() {
            return Ok(Handled::No);
        }
        // One line, and no control characters: the same reading of a multi-line
        // paste the list's find takes, for the same reason — a one-line box has
        // nowhere to put the rest, and the first line is the useful part.
        let line: String = text
            .chars()
            .take_while(|c| *c != '\n' && *c != '\r')
            .filter(|c| !c.is_control())
            .collect();
        if line.is_empty() {
            return Ok(Handled::No);
        }
        if let Some(search) = self.search.as_mut() {
            for c in line.chars() {
                search.push(c);
            }
            search.find(&self.lines, self.margin);
            search.aim();
        }
        Ok(Handled::Yes)
    }
}

/// Source files get a line-number gutter, because "look at line 42" is how
/// anyone talks about code. A wrapped continuation gets a blank number, which
/// is the only thing distinguishing it from the next line.
///
/// Reports how wide that gutter came out, because it is the pane's own margin
/// and the search steps over it — `/42` is a question about the code and not
/// about which line it is on. Zero in a pane too narrow to have drawn one, and
/// then there is nothing to step over rather than four columns to step over
/// anyway. See [`search::Margin`].
fn source_lines(
    text: &str,
    path: &Path,
    width: usize,
    mode: theme::Mode,
) -> (Vec<Line<'static>>, usize) {
    let rows = source::highlight_file(text, path, mode);
    let numbers = width >= LINE_NUMBER_MIN_WIDTH;
    let digits = if numbers {
        rows.len().to_string().len().max(3)
    } else {
        0
    };

    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        let (first, cont) = if numbers {
            (
                vec![Span::styled(
                    format!("{:>digits$} ", i + 1),
                    mode.theme().dim(),
                )],
                vec![Span::raw(format!("{:>digits$} ", ""))],
            )
        } else {
            (Vec::new(), Vec::new())
        };
        out.extend(wrap::hard_wrap(row, width, &first, &cont));
    }
    // The gutter is the number plus the space after it, and every row wears it
    // — a wrapped continuation gets a blank one of exactly the same width,
    // which is what makes one number describe the whole layout.
    let gutter = if numbers { digits + 1 } else { 0 };
    (out, gutter)
}

/// A letter with nothing held down with it.
///
/// `Ctrl` plus a letter is the agent's everywhere in this program, and both
/// `crate::scroll` and `list::Cursor` hand it back rather than declining it —
/// so that the pane's own arms are where that gets decided rather than where it
/// gets forgotten. `Alt+F` is Claude's `nextWord`, which `crate::keys` names as
/// the collision that nearly shipped; it is not this pane's either.
fn bare(key: KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && key.modifiers.is_empty()
}

fn empty_hint(width: usize, watching: bool, t: &theme::Theme) -> Vec<Line<'static>> {
    let hint = |s: &str| text::block(s, width, t.dim());
    let mut lines = hint(
        "Nothing open yet. This pane follows the markdown written under this \
         directory, and renders whatever it is pointed at.",
    );
    lines.push(Line::default());
    if !watching {
        lines.extend(hint(
            "The file watcher could not start here, so changes will not be \
             noticed on their own. Press r to look again.",
        ));
        lines.push(Line::default());
    }
    // Alt+E is named here for the same reason the unreadable screen names
    // Alt+G: this is a screen someone arrives at with nothing to press, and
    // the file list is the one thing that gets them off it under their own
    // steam. Tab only walks markdown, and there may be none.
    lines.extend(hint("Alt+E  the file list"));
    lines.extend(hint("Tab    next file"));
    lines.extend(hint("r      look again"));
    lines.extend(hint("j k    scroll"));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crossterm::event::{KeyEventKind, KeyModifiers, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    /// A pane with no scan racing the assertions. Every test here is about what
    /// `show` and the keys do, and the startup walk dropping a file in halfway
    /// through would make them flap. (There is no watcher to silence: the shell
    /// owns that one and never starts it in a test.)
    fn quiet(root: &Path) -> ViewerPane {
        let mut pane = ViewerPane::new(root.to_path_buf());
        pane.scan = None;
        pane
    }

    /// Lay out as if a frame of this size had been drawn.
    fn laid(pane: &mut ViewerPane, width: usize, height: usize) -> Vec<String> {
        pane.ensure_layout(width);
        pane.scroll.measure(pane.lines.len(), height);
        text(&pane.lines)
    }

    #[test]
    fn a_markdown_file_arrives_styled_not_as_its_source() {
        let dir = TempDir::new("view-md");
        let path = dir.write("plan.md", b"# Plan\n\nDo **the thing**.\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);

        assert_eq!(laid(&mut pane, 40, 10), ["# Plan", "", "Do the thing."]);
        assert!(pane.title().contains("plan.md"));
    }

    #[test]
    fn a_source_file_is_highlighted_and_numbered() {
        let dir = TempDir::new("view-rs");
        let path = dir.write("main.rs", b"fn main() {\n    println!(\"hi\");\n}\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);

        let lines = laid(&mut pane, 40, 10);
        assert_eq!(lines[0], "  1 fn main() {");
        assert_eq!(lines[2], "  3 }");
        // Not markdown: the braces and the string survive verbatim, and there
        // is colour past the gutter.
        assert!(lines[1].contains("println!(\"hi\");"));
        assert!(
            pane.lines[1]
                .spans
                .iter()
                .skip(1)
                .any(|s| s.style.fg.is_some())
        );
    }

    #[test]
    fn a_narrow_pane_drops_the_line_numbers_rather_than_the_code() {
        let dir = TempDir::new("view-narrow");
        let path = dir.write("a.rs", b"let x = 1;\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);
        assert_eq!(laid(&mut pane, 20, 10)[0], "let x = 1;");
    }

    // --- the four things that must never panic ---------------------------

    #[test]
    fn a_file_that_is_not_there_becomes_a_notice() {
        let dir = TempDir::new("view-missing");
        let mut pane = quiet(dir.path());
        pane.show(dir.path().join("ghost.md"));

        let lines = laid(&mut pane, 40, 10);
        assert_eq!(lines[0], "ghost.md");
        assert!(lines.iter().any(|l| l.contains("no such file")));
        assert!(pane.title().contains("unreadable"));
        // Nobody asks to be here, so the screen has to name a way out that
        // leads back to where they were — Tab only walks the markdown list.
        let text: String = lines.concat();
        assert!(text.contains("Alt+G"), "{text}");
    }

    #[test]
    fn a_binary_file_is_described_rather_than_drawn() {
        let dir = TempDir::new("view-bin");
        let path = dir.write("a.png", &[0x89, b'P', b'N', b'G', 0x00, 0xff, 0xfe]);
        let mut pane = quiet(dir.path());
        pane.show(&path);

        let lines = laid(&mut pane, 40, 10);
        assert!(lines.iter().any(|l| l.contains("binary file")));
        // Emphatically not the bytes.
        assert!(!lines.iter().any(|l| l.contains('\u{fffd}')));
    }

    #[test]
    fn a_directory_is_a_notice_not_an_access_denied() {
        let dir = TempDir::new("view-dir");
        let mut pane = quiet(dir.path());
        pane.show(dir.path());
        assert!(
            laid(&mut pane, 40, 10)
                .iter()
                .any(|l| l.contains("not a regular file"))
        );
    }

    #[test]
    fn an_enormous_file_is_capped_and_the_cap_is_visible() {
        let dir = TempDir::new("view-big");
        let mut body = Vec::new();
        while (body.len() as u64) < load::MAX_BYTES + 8192 {
            body.extend_from_slice(b"a line of a very long document\n");
        }
        let path = dir.write("huge.txt", &body);

        let mut pane = quiet(dir.path());
        pane.show(&path);
        let lines = laid(&mut pane, 40, 10);
        assert!(lines.last().unwrap().contains("stopped at"));
        assert!(pane.title().contains("truncated"));
    }

    // --- scrolling --------------------------------------------------------

    /// A hundred physical rows at width 40. A list, not a paragraph: reflowed
    /// prose would collapse to twenty rows and the paging arithmetic below
    /// would be testing the clamp instead of the paging.
    fn scrollable(dir: &TempDir) -> ViewerPane {
        let body: String = (1..=100).map(|i| format!("- line {i}\n")).collect();
        let path = dir.write("long.md", body.as_bytes());
        let mut pane = quiet(dir.path());
        pane.show(&path);
        laid(&mut pane, 40, 10);
        pane
    }

    /// The vocabulary itself is `crate::scroll`'s, and tested there. What has
    /// to be true *here* is that the pane hands its keys to it, measured
    /// against the rows this document actually laid out.
    #[test]
    fn the_scroll_keys_reach_the_shared_vocabulary() {
        let dir = TempDir::new("scroll-keys");
        let mut pane = scrollable(&dir);
        let total = pane.lines.len();

        assert_eq!(
            pane.handle_key(key(KeyCode::Char('k'))).unwrap(),
            Handled::No
        );
        assert_eq!(pane.scroll.offset, 0, "already at the top");

        pane.handle_key(key(KeyCode::Char('j'))).unwrap();
        assert_eq!(pane.scroll.offset, 1);
        pane.handle_key(key(KeyCode::Char(' '))).unwrap();
        assert_eq!(
            pane.scroll.offset,
            1 + 9,
            "a page keeps one line of overlap"
        );
        pane.handle_key(ctrl('d')).unwrap();
        assert_eq!(pane.scroll.offset, 15);

        pane.handle_key(key(KeyCode::Char('G'))).unwrap();
        assert_eq!(pane.scroll.offset, total - 10);
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('j'))).unwrap(),
            Handled::No,
            "a key that changes nothing must not report that it acted"
        );
    }

    #[test]
    fn the_wheel_scrolls_without_the_pane_being_focused() {
        let dir = TempDir::new("scroll-wheel");
        let mut pane = scrollable(&dir);
        let ev = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        pane.handle_mouse(&ev).unwrap();
        assert_eq!(pane.scroll.offset, 3);
    }

    #[test]
    fn esc_and_q_are_left_for_the_shell_to_read_as_go_back() {
        let dir = TempDir::new("scroll-esc");
        let mut pane = scrollable(&dir);
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('q'))).unwrap(),
            Handled::No
        );
    }

    #[test]
    fn a_rewrite_of_the_open_file_keeps_the_readers_place() {
        let dir = TempDir::new("scroll-keep");
        let mut pane = scrollable(&dir);
        pane.handle_key(key(KeyCode::Char(' '))).unwrap();
        let was = pane.scroll.offset;
        assert!(was > 0);

        // Same path: the agent rewrote what someone is halfway through.
        let path = pane.path().unwrap().to_path_buf();
        pane.show(&path);
        assert_eq!(pane.scroll.offset, was);

        // Different path: a new document starts at the top.
        let other = dir.write("other.md", b"# other\n");
        pane.show(&other);
        assert_eq!(pane.scroll.offset, 0);
    }

    #[test]
    fn re_laying_out_at_a_new_width_rewraps_and_keeps_the_offset_in_range() {
        let dir = TempDir::new("scroll-resize");
        let mut pane = scrollable(&dir);
        pane.handle_key(key(KeyCode::Char('G'))).unwrap();
        let wide = pane.lines.len();

        // Narrower: more physical rows, so the offset stays valid.
        laid(&mut pane, 12, 10);
        assert!(pane.lines.len() >= wide);
        assert!(pane.scroll.offset <= pane.scroll.max());
    }

    // --- the frame path ---------------------------------------------------

    #[test]
    fn drawing_at_hostile_sizes_does_not_panic() {
        let dir = TempDir::new("view-sizes");
        let mut pane = scrollable(&dir);
        let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();

        for (w, h) in [(0, 0), (1, 1), (2, 20), (25, 1), (60, 20)] {
            term.draw(|f| pane.render(f, Rect::new(0, 0, w, h)))
                .unwrap();
        }
        // ...and after all that the pane is still usable.
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();
        assert!(!pane.lines.is_empty());
    }

    /// The whole point of F3, and the one behaviour no other test would catch:
    /// this pane paints a page rather than letting the terminal's background
    /// show through, so a reader gets a bright page in a bright room without
    /// also reconfiguring their terminal.
    #[test]
    fn the_reader_paints_its_own_page_and_f3_repaints_it() {
        let dir = TempDir::new("view-theme-page");
        let path = dir.write("doc.md", b"# heading\n\nshort body\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);

        // Every cell, including the blank rows below the text and the column
        // the scrollbar reserves — a page with holes in it is not a page.
        let page = |pane: &mut ViewerPane| -> Vec<Color> {
            let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
            term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
                .unwrap();
            term.backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.bg)
                .collect()
        };

        for cell in page(&mut pane) {
            assert_eq!(
                cell,
                theme::DARK.bg,
                "the terminal's background shows through"
            );
        }

        pane.toggle_theme();
        for cell in page(&mut pane) {
            assert_eq!(cell, theme::LIGHT.bg, "F3 did not repaint the page");
        }
    }

    /// A laid-out document holds baked styles. Without the invalidation in
    /// `toggle_theme` the new palette would only reach the *next* file, which
    /// from the reader's side is a key that did nothing.
    #[test]
    fn f3_restyles_the_document_already_on_screen() {
        let dir = TempDir::new("view-theme-relayout");
        let path = dir.write("doc.md", b"# heading\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);

        let heading = |pane: &mut ViewerPane| {
            pane.ensure_layout(40);
            pane.lines[0]
                .spans
                .iter()
                .find(|s| s.content.contains("heading"))
                .and_then(|s| s.style.fg)
                .expect("the heading is coloured")
        };

        let before = heading(&mut pane);
        assert_eq!(before, theme::DARK.heading(1).fg.unwrap());

        pane.toggle_theme();
        assert_eq!(heading(&mut pane), theme::LIGHT.heading(1).fg.unwrap());
    }

    /// The list and the document are two halves of one pane. They are drawn on
    /// separate frames, so nothing but this wire keeps them from disagreeing
    /// about what colour the page is.
    #[test]
    fn f3_reaches_the_file_list_as_well_as_the_document() {
        let dir = TempDir::new("view-theme-list");
        dir.write("a.md", b"# a\n");
        let mut pane = quiet(dir.path());
        pane.toggle_browse();
        pane.toggle_theme();

        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();
        // Two legitimate backgrounds here, not one: the selected row repaints
        // its own. Both have to come from the palette that was switched to, and
        // neither may be the terminal's.
        for cell in term.backend().buffer().content() {
            assert!(
                cell.bg == theme::LIGHT.bg || cell.bg == theme::LIGHT.sel_bg,
                "the list kept the old page: {:?}",
                cell.bg
            );
        }
    }

    #[test]
    fn the_pane_takes_up_a_pending_file_only_when_it_is_the_one_on_screen() {
        let dir = TempDir::new("view-pending");
        let path = dir.write("fresh.md", b"# fresh\n");
        let mut pane = quiet(dir.path());
        pane.pending = Some(path.clone());

        // Not drawn: the file waits and the title carries the mark.
        assert!(pane.path().is_none());
        assert!(pane.title().starts_with("files"));

        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();
        assert_eq!(pane.path(), Some(path.as_path()));
        assert!(pane.pending.is_none());
    }

    #[test]
    fn taking_a_document_up_mid_frame_asks_for_the_frame_that_shows_its_title() {
        // The shell asks this pane for its title and draws the border *before*
        // it renders the pane, so the frame that first shows a pending document
        // carries the title of the pane that had none: `files`, over a page with
        // a document on it. Every later frame is right — and nothing promises a
        // later frame, because a pane that changed its own state inside a render
        // has asked nobody to draw again.
        //
        // It reached CI as a twenty-second timeout waiting for a title that was
        // never going to arrive, on the one platform where nothing else was
        // producing frames: a `sh` idle at a prompt, once `crate::watch` stopped
        // reporting abeam's own reads as writes. Until then the pane was rescued
        // by the events it generated by opening the document.
        let dir = TempDir::new("view-owed");
        let path = dir.write("fresh.md", b"# fresh\n");
        let mut pane = quiet(dir.path());
        pane.pending = Some(path.clone());

        // Nothing is owed before the render that changes anything.
        assert!(!pane.tick(), "a quiet pane asks for nothing");

        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();

        // The title the shell just drew and the body under it disagree, which is
        // the state this frame has to be followed out of.
        assert!(
            pane.title().contains("fresh.md"),
            "the pane is showing the document now: {}",
            pane.title()
        );
        assert!(
            pane.tick(),
            "the frame that shows this title was never asked for"
        );
        // Exactly one, so an idle pane does not redraw for ever.
        assert!(!pane.tick(), "one frame is owed, not a stream of them");
    }

    #[test]
    fn holding_r_down_does_not_start_a_walk_per_repeat_tick() {
        // The console emits a key event per auto-repeat tick, ~30 a second, and
        // the shell drains the whole batch before drawing. Starting a fresh
        // gitignore walk of the repository for each one — and dropping every
        // answer but the last — is a lot of disk for one held key.
        let dir = TempDir::new("view-rescan");
        let mut pane = quiet(dir.path());

        // Stand in for a walk still running: a channel nothing has answered.
        let (tx, rx) = std::sync::mpsc::channel::<Scan>();
        pane.scan = Some(rx);
        for _ in 0..5 {
            pane.handle_key(key(KeyCode::Char('r'))).unwrap();
        }

        // Still the same receiver — if `r` had replaced it, this answer would
        // go nowhere and `tick` would never see it.
        tx.send(Scan {
            recent: vec![dir.write("late.md", b"# late\n")],
            files: vec!["late.md".into()],
            cut: false,
        })
        .unwrap();
        assert!(pane.tick(), "the in-flight walk still reports");
        assert_eq!(pane.recent.len(), 1);
    }

    #[test]
    fn tab_walks_the_file_list_and_wraps_round() {
        let dir = TempDir::new("view-tab");
        let a = dir.write("a.md", b"# a\n");
        let b = dir.write("b.md", b"# b\n");
        let mut pane = quiet(dir.path());
        pane.recent = vec![a.clone(), b.clone()];
        pane.show(&a);

        pane.handle_key(key(KeyCode::Tab)).unwrap();
        assert_eq!(pane.path(), Some(b.as_path()));
        pane.handle_key(key(KeyCode::Tab)).unwrap();
        assert_eq!(pane.path(), Some(a.as_path()));
        pane.handle_key(key(KeyCode::BackTab)).unwrap();
        assert_eq!(pane.path(), Some(b.as_path()));
    }

    #[test]
    fn tab_with_nothing_to_walk_says_it_did_nothing() {
        // A repository with no markdown in it, or a scan that has not answered
        // yet. Reporting `Yes` here spent a frame — the agent's whole screen
        // redrawn — on a key that could not move.
        let dir = TempDir::new("view-tab-empty");
        let mut pane = quiet(dir.path());
        assert!(pane.recent.is_empty());
        assert_eq!(pane.handle_key(key(KeyCode::Tab)).unwrap(), Handled::No);
        assert_eq!(pane.handle_key(key(KeyCode::BackTab)).unwrap(), Handled::No);
    }

    #[test]
    fn an_empty_pane_says_what_it_is_for() {
        let dir = TempDir::new("view-empty");
        let mut pane = quiet(dir.path());
        let lines = laid(&mut pane, 40, 10);
        assert!(lines.iter().any(|l| l.contains("markdown")));
        // No watcher in a quiet pane, and it admits that rather than pretending.
        assert!(lines.iter().any(|l| l.contains("watcher could not start")));
        assert_eq!(pane.title(), "files");
    }

    // --- the file list ----------------------------------------------------

    #[test]
    fn the_second_alt_e_opens_the_list_beside_the_file_on_screen() {
        let dir = TempDir::new("view-browse");
        std::fs::create_dir_all(dir.path().join("docs")).expect("create docs");
        let design = dir.path().join("docs").join("design.md");
        std::fs::write(&design, b"# design\n").expect("write");

        let mut pane = quiet(dir.path());
        pane.show(&design);
        pane.toggle_browse();
        // Where the reader already was, not the root: the neighbouring files
        // are almost always the ones wanted.
        assert!(pane.title().starts_with("docs/"), "{}", pane.title());

        // ...and back again, onto the same document.
        pane.toggle_browse();
        assert_eq!(pane.path(), Some(design.as_path()));
    }

    #[test]
    fn enter_on_a_file_in_the_list_opens_it_in_the_document_view() {
        let dir = TempDir::new("view-browse-open");
        dir.write("plan.md", b"# Plan\n\nDo **the thing**.\n");
        let mut pane = quiet(dir.path());
        pane.toggle_browse();

        assert_eq!(
            pane.handle_key(key(KeyCode::Enter)).unwrap(),
            Handled::Yes,
            "opening a file is something coming of the key"
        );
        assert!(matches!(pane.mode, Mode::Doc), "the list has done its job");
        assert_eq!(laid(&mut pane, 40, 10), ["# Plan", "", "Do the thing."]);
    }

    #[test]
    fn a_file_arriving_while_the_list_is_up_waits_rather_than_taking_over() {
        // The rule the whole design rests on: nothing switches the view under
        // you. Being drawn is normally the signal to take a pending file up,
        // and in the list that signal has to mean the opposite.
        let dir = TempDir::new("view-browse-pending");
        let fresh = dir.write("fresh.md", b"# fresh\n");
        let mut pane = quiet(dir.path());
        pane.toggle_browse();
        pane.follow(fresh.clone());

        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();

        assert!(pane.path().is_none(), "the document view is untouched");
        assert!(
            pane.has_pending(),
            "and the shell can still mark the border"
        );
        // No border to mark while the list is the thing showing, so the pane
        // says it itself.
        assert!(pane.title().starts_with("◆ "), "{}", pane.title());

        // Leaving the list is what releases it, exactly as the mark says.
        pane.toggle_browse();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();
        assert_eq!(pane.path(), Some(fresh.as_path()));
    }

    #[test]
    fn a_file_chosen_in_the_list_is_not_replaced_by_one_the_watcher_queued() {
        // The agent writes something while a directory is being walked, so a
        // file is pending; the reader picks a different one and presses Enter.
        // The very next frame used to swap in the queued file, so what arrived
        // on screen was never the file that was chosen. The same shape reaches
        // `Enter` in the git view and `Tab`, because the shell can queue and
        // show within one pass of its loop.
        let dir = TempDir::new("view-browse-supersede");
        let chosen = dir.write("chosen.md", b"# chosen\n");
        let queued = dir.write("queued.md", b"# queued\n");

        let mut pane = quiet(dir.path());
        pane.toggle_browse();
        pane.follow(queued);
        assert!(pane.has_pending());

        // `chosen.md` sorts first, so the selection is already on it.
        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(pane.path(), Some(chosen.as_path()));
        assert!(!pane.has_pending(), "the queue was superseded, not stacked");

        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();
        assert_eq!(
            pane.path(),
            Some(chosen.as_path()),
            "and the frame after it still shows what was chosen"
        );
    }

    #[test]
    fn esc_in_the_list_is_left_for_the_shell_but_esc_in_a_find_is_not() {
        let dir = TempDir::new("view-browse-esc");
        dir.write("a.md", b"# a\n");
        let mut pane = quiet(dir.path());
        pane.toggle_browse();

        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
        pane.handle_key(key(KeyCode::Char('/'))).unwrap();
        assert_eq!(
            pane.handle_key(key(KeyCode::Esc)).unwrap(),
            Handled::Yes,
            "the find swallows it, and the reader stays in the list"
        );
        assert!(matches!(pane.mode, Mode::Browse));
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
    }

    #[test]
    fn the_border_promises_the_key_that_esc_actually_presses() {
        // Three states, and the border has to be true in all of them — it is
        // the only place the way out is written down.
        let dir = TempDir::new("view-browse-hint");
        dir.write("a.md", b"# a\n");
        let mut pane = quiet(dir.path());
        assert!(!pane.takes_input());
        assert_eq!(pane.exit_hint(), " · esc→agent");

        pane.toggle_browse();
        assert!(!pane.takes_input(), "the list is still only read");
        assert_eq!(pane.exit_hint(), " · esc→agent");

        pane.handle_key(key(KeyCode::Char('/'))).unwrap();
        assert!(pane.takes_input(), "a query is typing");
        assert_eq!(
            pane.exit_hint(),
            " · esc→list",
            "Esc closes the find; the agent is one press further than that"
        );

        // Leaving the list ends the query, so the two answers cannot drift out
        // of step with what Esc would do.
        pane.toggle_browse();
        assert!(!pane.takes_input());
        assert_eq!(pane.exit_hint(), " · esc→agent");
    }

    #[test]
    fn a_glance_at_the_list_scrolls_it_without_re_choosing_the_file() {
        // `Alt+J` arrives as a bare `Down`. In the document that is the same
        // key the pane would have handled; in the list `Down` moves the
        // selection, and a read from the other side of the window must not
        // re-aim the Enter that follows it.
        let dir = TempDir::new("view-glance");
        for i in 0..20 {
            dir.write(&format!("f{i:02}.md"), b"# x\n");
        }
        let mut pane = quiet(dir.path());
        pane.toggle_browse();
        let mut term = Terminal::new(TestBackend::new(40, 6)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 6)))
            .unwrap();

        let before = pane.title();
        assert_eq!(
            pane.scroll_key(key(KeyCode::Down)).unwrap(),
            Handled::Yes,
            "the view moved"
        );
        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert_eq!(
            pane.path().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("f00.md")),
            "Enter opened the row that was selected before the glance"
        );
        assert!(before.starts_with("./"), "{before}");

        // ...and in the document view a glance is an ordinary scroll.
        let long: String = (1..=100).map(|i| format!("- line {i}\n")).collect();
        pane.show(dir.write("long.md", long.as_bytes()));
        laid(&mut pane, 40, 10);
        assert_eq!(pane.scroll_key(key(KeyCode::Down)).unwrap(), Handled::Yes);
        assert_eq!(pane.scroll.offset, 1);
    }

    #[test]
    fn a_pasted_path_reaches_the_find_and_nothing_else() {
        let dir = TempDir::new("view-paste");
        std::fs::create_dir_all(dir.path().join("docs")).expect("create docs");
        std::fs::write(dir.path().join("docs").join("design.md"), b"# design\n").expect("write");

        let mut pane = quiet(dir.path());
        pane.browse
            .set_index(vec!["docs/design.md".to_string()].into());
        // Nothing in the document view can take text.
        assert_eq!(pane.handle_paste("docs/design.md").unwrap(), Handled::No);

        pane.toggle_browse();
        assert_eq!(pane.handle_paste("nope").unwrap(), Handled::No);
        pane.handle_key(key(KeyCode::Char('/'))).unwrap();
        assert_eq!(pane.handle_paste("docs/design").unwrap(), Handled::Yes);
        assert!(pane.title().contains("1 match"), "{}", pane.title());
    }

    #[test]
    fn a_find_reaches_a_file_nothing_ever_pointed_the_pane_at() {
        // The whole point of the index: a `.rs` three directories away, which
        // is neither markdown nor anywhere near the directory being listed.
        let dir = TempDir::new("view-browse-find");
        std::fs::create_dir_all(dir.path().join("src")).expect("create src");
        std::fs::write(dir.path().join("src").join("main.rs"), b"fn main() {}\n").expect("write");

        let mut pane = quiet(dir.path());
        pane.browse
            .set_index(vec!["src/main.rs".to_string()].into());
        pane.toggle_browse();

        for code in "/main".chars().map(KeyCode::Char) {
            pane.handle_key(key(code)).unwrap();
        }
        assert!(pane.title().contains("1 match"), "{}", pane.title());
        pane.handle_key(key(KeyCode::Enter)).unwrap();

        assert!(matches!(pane.mode, Mode::Doc));
        assert_eq!(laid(&mut pane, 40, 10)[0], "  1 fn main() {}");
    }

    // --- raw and rendered -------------------------------------------------

    #[test]
    fn t_swaps_rendered_markdown_for_its_source_and_back() {
        let dir = TempDir::new("view-raw");
        let path = dir.write("plan.md", b"# Plan\n\nDo **the thing**.\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);
        laid(&mut pane, 40, 10);

        assert_eq!(
            pane.handle_key(key(KeyCode::Char('t'))).unwrap(),
            Handled::Yes
        );
        let lines = laid(&mut pane, 40, 10);
        // The source, gutter and all: the markers the rendering consumed are
        // back, and they are numbered.
        assert_eq!(lines[0], "  1 # Plan");
        assert_eq!(lines[2], "  3 Do **the thing**.");
        assert!(pane.title().contains("· source"), "{}", pane.title());

        pane.handle_key(key(KeyCode::Char('t'))).unwrap();
        assert_eq!(laid(&mut pane, 40, 10), ["# Plan", "", "Do the thing."]);
        assert!(pane.title().contains("· rendered"), "{}", pane.title());
    }

    #[test]
    fn a_source_file_has_no_second_form_to_toggle_to() {
        // `Handled::No` rather than a no-op that claims to have acted: a frame
        // here re-renders the agent's whole screen.
        let dir = TempDir::new("view-raw-rs");
        let path = dir.write("main.rs", b"fn main() {}\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('t'))).unwrap(),
            Handled::No
        );
        assert!(!pane.raw, "and the pane's idea of raw is unchanged");

        // Nor is there anything to toggle on a screen with no document on it.
        let mut empty = quiet(dir.path());
        assert_eq!(
            empty.handle_key(key(KeyCode::Char('t'))).unwrap(),
            Handled::No
        );
    }

    #[test]
    fn the_toggle_lands_the_reader_at_the_same_point_in_the_other_form() {
        // Deliberate, and not `measure`'s clamp: the two forms share no rows,
        // so the fraction is the closest thing to a position that survives.
        let dir = TempDir::new("view-raw-place");
        let body: String = (1..=100).map(|i| format!("- line {i}\n")).collect();
        let path = dir.write("long.md", body.as_bytes());
        let mut pane = quiet(dir.path());
        pane.show(&path);
        laid(&mut pane, 40, 10);

        pane.handle_key(key(KeyCode::Char('G'))).unwrap();
        assert_eq!(pane.scroll.offset, pane.scroll.max());
        pane.handle_key(key(KeyCode::Char('t'))).unwrap();
        assert_eq!(
            pane.scroll.offset,
            pane.scroll.max(),
            "the end of a document is still the end of it"
        );

        pane.handle_key(key(KeyCode::Char('g'))).unwrap();
        pane.handle_key(key(KeyCode::Char('t'))).unwrap();
        assert_eq!(pane.scroll.offset, 0);
    }

    // --- searching the document -------------------------------------------

    /// `/` and a query, as a reader types it.
    fn query(pane: &mut ViewerPane, q: &str) {
        pane.handle_key(key(KeyCode::Char('/'))).unwrap();
        for c in q.chars() {
            pane.handle_key(key(KeyCode::Char(c))).unwrap();
        }
    }

    fn hits(pane: &ViewerPane) -> Vec<(usize, usize)> {
        let s = pane.search.as_ref().expect("a search");
        s.hits().iter().map(|h| (h.row, h.start)).collect()
    }

    /// Which hit of the document the reader is on, counting from zero.
    fn nth_hit(pane: &ViewerPane) -> Option<usize> {
        let s = pane.search.as_ref()?;
        let now = s.current()?;
        s.hits().iter().position(|h| *h == now)
    }

    fn draw(pane: &mut ViewerPane, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("a test terminal");
        term.draw(|f| pane.render(f, Rect::new(0, 0, w, h)))
            .expect("draw the document");
        term
    }

    #[test]
    fn a_search_finds_what_is_on_the_page_in_each_of_the_three_things_a_body_can_be() {
        // The whole design decision in one test: rendered markdown, the source
        // it was rendered from, and a highlighted source file are all just rows
        // by the time this runs, so one matcher serves all three — and the
        // markers the rendering ate are findable in exactly the form that still
        // has them.
        let dir = TempDir::new("view-search-forms");
        let path = dir.write("plan.md", b"# Plan\n\nDo **the thing**.\n");
        let mut pane = quiet(dir.path());
        pane.show(&path);
        laid(&mut pane, 40, 10);

        query(&mut pane, "thing");
        assert_eq!(hits(&pane), [(2, 7)], "row 2 of ['# Plan', '', 'Do the…']");
        assert!(pane.title().contains("/thing · 1/1"), "{}", pane.title());

        // The documented cost of searching the page: what was rendered away is
        // not there to be found — and the title puts the answer on the miss,
        // where the question is being asked.
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        query(&mut pane, "**");
        assert_eq!(hits(&pane), []);
        assert!(
            pane.title().contains("/** · no match · t for source"),
            "{}",
            pane.title()
        );
        assert!(pane.title().contains("· rendered"), "{}", pane.title());

        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('t'))).unwrap(),
            Handled::Yes
        );
        laid(&mut pane, 40, 10);
        query(&mut pane, "**");
        assert_eq!(
            hits(&pane),
            [(2, 7), (2, 18)],
            "row '  3 Do **the thing**.'"
        );

        // ...and a source file, gutter and all.
        let rs = dir.write("main.rs", b"fn main() {\n    println!(\"hi\");\n}\n");
        pane.show(&rs);
        laid(&mut pane, 40, 10);
        query(&mut pane, "println");
        assert_eq!(hits(&pane), [(1, 8)]);
    }

    /// Sixty rows with a needle every twentieth, wrapped to fit at width 40 and
    /// to wrap at width 20 — so a narrower pane genuinely moves every hit.
    fn needles(dir: &TempDir) -> ViewerPane {
        let body: String = (1..=60)
            .map(|i| {
                if i % 20 == 0 {
                    format!("- line {i} has the needle in it\n")
                } else {
                    format!("- line {i}\n")
                }
            })
            .collect();
        let path = dir.write("long.md", body.as_bytes());
        let mut pane = quiet(dir.path());
        pane.show(&path);
        pane
    }

    #[test]
    fn a_width_change_re_finds_the_hits_and_leaves_the_reader_on_the_same_one() {
        // A hit is a row index, and a re-wrap makes every one of them a guess
        // about a document that no longer exists. `ensure_layout` is the funnel
        // every route that rebuilds the rows already goes through, which is why
        // the refind hangs off it rather than off a list of them.
        let dir = TempDir::new("view-search-rewrap");
        let mut pane = needles(&dir);
        draw(&mut pane, 40, 10);

        query(&mut pane, "needle");
        assert_eq!(hits(&pane).len(), 3);
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        pane.handle_key(key(KeyCode::Char('n'))).unwrap();
        assert_eq!(nth_hit(&pane), Some(1));
        draw(&mut pane, 40, 10);
        let wide = pane.search.as_ref().unwrap().current().unwrap().row;
        let was = pane.scroll.offset;

        draw(&mut pane, 20, 10);
        assert_eq!(hits(&pane).len(), 3, "the same three, re-found");
        assert_eq!(nth_hit(&pane), Some(1), "still the reader's hit");
        let narrow = pane.search.as_ref().unwrap().current().unwrap().row;
        assert_ne!(narrow, wide, "and every row moved, which is the point");
        // Re-found, *not* re-aimed. Dragging a window narrower is not the
        // reader asking to be taken anywhere, and the rows they were looking at
        // are still the rows they were looking at.
        assert_eq!(pane.scroll.offset, was, "the resize moved the reader");
    }

    #[test]
    fn a_rebuild_nobody_asked_for_leaves_the_reader_exactly_where_they_were() {
        // The bug this is here to keep out: `find` armed the reveal, so an
        // agent saving the file scrolled somebody who had read on to the end of
        // the document back to a match they had finished with — on a pane they
        // need not even have been focused on. Four routes, one shape.
        let dir = TempDir::new("view-search-quiet-rebuild");
        let mut pane = needles(&dir);
        let path = pane.path().unwrap().to_path_buf();
        draw(&mut pane, 40, 10);
        query(&mut pane, "needle");
        // The frame the query itself asked for, which is where the reader's own
        // jump to the first match happens, and the one it owes for having moved
        // the view under a title already drawn. Everything below is about the
        // frames *nobody* asked for.
        draw(&mut pane, 40, 10);
        assert!(pane.tick(), "the reader's own jump owes a frame");
        pane.handle_key(key(KeyCode::Enter)).unwrap();

        pane.handle_key(key(KeyCode::Char('G'))).unwrap();
        let end = pane.scroll.offset;
        assert!(end > 0);

        // The agent saves the file.
        pane.show(&path);
        draw(&mut pane, 40, 10);
        assert_eq!(pane.scroll.offset, end, "a save moved the reader");
        assert!(!pane.tick(), "and asked for a frame on top of it");

        // `r`.
        pane.reload();
        draw(&mut pane, 40, 10);
        assert_eq!(pane.scroll.offset, end, "r moved the reader");

        // F3, which rebuilds every row to restyle it.
        pane.toggle_theme();
        draw(&mut pane, 40, 10);
        assert_eq!(pane.scroll.offset, end, "F3 moved the reader");

        // `t` is the sharpest of them: it lays the document out *itself*, to
        // carry the reader's place across as a fraction, and an armed reveal
        // would have thrown that answer away on the next frame.
        pane.handle_key(key(KeyCode::Char('t'))).unwrap();
        let fraction = pane.scroll.offset;
        assert_eq!(fraction, pane.scroll.max(), "the end is still the end");
        draw(&mut pane, 40, 10);
        assert_eq!(pane.scroll.offset, fraction, "the frame undid `t`'s answer");
    }

    #[test]
    fn n_and_shift_n_walk_the_hits_and_wrap_round() {
        let dir = TempDir::new("view-search-step");
        let mut pane = needles(&dir);
        draw(&mut pane, 40, 10);
        query(&mut pane, "needle");
        pane.handle_key(key(KeyCode::Enter)).unwrap();

        assert_eq!(nth_hit(&pane), Some(0));
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('n'))).unwrap(),
            Handled::Yes
        );
        assert_eq!(nth_hit(&pane), Some(1));
        pane.handle_key(key(KeyCode::Char('n'))).unwrap();
        pane.handle_key(key(KeyCode::Char('n'))).unwrap();
        assert_eq!(nth_hit(&pane), Some(0), "past the last is the first");
        pane.handle_key(key(KeyCode::Char('N'))).unwrap();
        assert_eq!(nth_hit(&pane), Some(2), "and back off the top");

        // With nothing searched for they are not keys at all, and must not cost
        // a frame — the agent's whole screen — for saying so.
        let mut fresh = needles(&dir);
        laid(&mut fresh, 40, 10);
        assert_eq!(
            fresh.handle_key(key(KeyCode::Char('n'))).unwrap(),
            Handled::No
        );
        assert_eq!(
            fresh.handle_key(key(KeyCode::Char('N'))).unwrap(),
            Handled::No
        );
    }

    #[test]
    fn a_hit_below_the_fold_is_scrolled_to_and_one_already_on_screen_is_not() {
        let dir = TempDir::new("view-search-reveal");
        let mut pane = needles(&dir);
        draw(&mut pane, 40, 10);
        assert_eq!(pane.scroll.offset, 0);

        query(&mut pane, "needle");
        draw(&mut pane, 40, 10);
        let row = pane.search.as_ref().unwrap().current().unwrap().row;
        assert!(row > 10, "the first needle is off the first screen");
        assert!(
            row >= pane.scroll.offset && row < pane.scroll.offset + 10,
            "row {row} was never scrolled to; offset {}",
            pane.scroll.offset
        );
        // Centred rather than dragged to the bottom edge: a match is where the
        // reader starts reading, so there has to be something after it.
        assert!(pane.scroll.offset < row, "nothing above the match");
        // The jump happened inside the render, so the percentage in the title
        // the shell had already drawn is the one from before it. One frame is
        // owed for the same reason a document taken up mid-render owes one, and
        // exactly one, so an idle pane does not redraw for ever.
        assert!(
            pane.tick(),
            "the frame that shows this position was skipped"
        );
        assert!(!pane.tick());

        // A hit already on screen moves nothing, or `n` between two visible
        // matches would scroll the paragraph out from under the reader.
        let was = pane.scroll.offset;
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        let next = pane.search.as_ref().unwrap();
        assert!(next.hits().len() > 1);
        draw(&mut pane, 40, 10);
        assert_eq!(pane.scroll.offset, was);
    }

    #[test]
    fn a_query_that_matches_nothing_says_so_rather_than_going_quiet() {
        let dir = TempDir::new("view-search-miss");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);

        query(&mut pane, "haystack");
        assert_eq!(hits(&pane), []);
        assert!(
            pane.title().contains("/haystack · no match"),
            "{}",
            pane.title()
        );

        // An open box with nothing typed into it is not a miss — nobody has
        // asked anything yet — and it does not claim a count either.
        pane.handle_key(key(KeyCode::Backspace)).unwrap();
        assert!(pane.title().contains("· /haystac ·"), "{}", pane.title());
    }

    #[test]
    fn every_printable_key_is_a_letter_while_the_box_is_open() {
        // The one that keeps the reader in the pane. `App::handle_key` reads a
        // `q` this pane declined as "give focus back to the agent", so a query
        // with a `q` in it would have thrown them out mid-word — and `j` would
        // have scrolled the document instead of being typed.
        let dir = TempDir::new("view-search-letters");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);

        query(&mut pane, "");
        for c in ['q', 'j', 'n', 'N', 'r', 't', 'g', 'G', '/'] {
            assert_eq!(
                pane.handle_key(key(KeyCode::Char(c))).unwrap(),
                Handled::Yes,
                "{c} was not claimed by the box"
            );
        }
        assert!(pane.title().contains("/qjnNrtgG/"), "{}", pane.title());
        assert_eq!(pane.scroll.offset, 0, "and none of them scrolled anything");

        // Ctrl+letter is still the agent's, exactly as it is in the list's find.
        assert_eq!(pane.handle_key(ctrl('c')).unwrap(), Handled::No);
        assert!(pane.title().contains("/qjnNrtgG/"), "{}", pane.title());
    }

    #[test]
    fn a_glance_scrolls_the_document_instead_of_typing_into_the_box() {
        // `Alt+J` arrives as a bare `Down`, which inside the box steps between
        // hits. A read from the other side of the window moves the view and
        // nothing else — the rule `Pane::scroll_key` exists for.
        let dir = TempDir::new("view-search-glance");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);
        query(&mut pane, "line");

        assert_eq!(pane.scroll_key(key(KeyCode::Down)).unwrap(), Handled::Yes);
        assert_eq!(pane.scroll.offset, 1);
        assert!(
            pane.title().contains("/line · 1/60 ·"),
            "the query took the keystroke: {}",
            pane.title()
        );

        // ...and where the reader has put the view is where the next letter
        // looks from, or typing one more character would throw them back to
        // wherever they pressed `/`.
        pane.handle_key(key(KeyCode::Char(' '))).unwrap();
        assert!(pane.title().contains("/line "), "{}", pane.title());
        let row = pane.search.as_ref().unwrap().current().unwrap().row;
        assert!(row >= 1, "aimed from the top again");

        // The paging half of the F1 promise survives an open query too: `g`,
        // `G`, `space` and `b` are letters in here, so these four keys are all
        // that is left of it.
        for code in [
            KeyCode::PageDown,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::Home,
        ] {
            assert_eq!(
                pane.handle_key(key(code)).unwrap(),
                Handled::Yes,
                "{code:?}"
            );
        }
        assert_eq!(pane.scroll.offset, 0, "Home came back to the top");
        // The query is still `line ` — a space, typed above — and none of the
        // four keys landed in it.
        assert!(
            pane.title().contains("· /line  · 2/60 ·"),
            "{}",
            pane.title()
        );
    }

    #[test]
    fn esc_closes_the_box_then_clears_the_hits_then_belongs_to_the_shell() {
        let dir = TempDir::new("view-search-esc");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);
        assert!(!pane.takes_input());
        assert_eq!(pane.exit_hint(), " · esc→agent");

        query(&mut pane, "needle");
        assert!(pane.takes_input(), "a query is typing");
        assert_eq!(pane.exit_hint(), " · esc→hits");

        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::Yes);
        assert!(!pane.takes_input(), "the box is shut");
        assert_eq!(pane.exit_hint(), " · esc→clear");
        assert_eq!(hits(&pane).len(), 3, "and the hits are still there");
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('n'))).unwrap(),
            Handled::Yes,
            "n still steps them, which is what keeping them is for"
        );

        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::Yes);
        assert!(pane.search.is_none());
        assert_eq!(pane.exit_hint(), " · esc→agent");
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
    }

    #[test]
    fn the_box_is_visible_on_every_screen_it_can_be_opened_on() {
        // `/` works wherever the pane has rows, and both of these have real
        // ones — they are searched and highlighted like any other. A box the
        // reader is typing into with nothing in the title to show for it is
        // worse on these two than in a document, because they are the screens
        // somebody arrives at without having asked to.
        let dir = TempDir::new("view-search-states");
        let mut empty = quiet(dir.path());
        laid(&mut empty, 40, 10);
        query(&mut empty, "watcher");
        assert!(empty.takes_input());
        assert!(
            empty.title().starts_with("files · /watcher · 1/"),
            "{}",
            empty.title()
        );

        let mut missing = quiet(dir.path());
        missing.show(dir.path().join("ghost.md"));
        laid(&mut missing, 40, 10);
        query(&mut missing, "retry");
        assert!(
            missing.title().contains("unreadable · /retry · 1/1"),
            "{}",
            missing.title()
        );
        // ...and the `t for source` answer belongs to rendered markdown only.
        // Offering it on a screen with no source behind it names a key that
        // does nothing there.
        query(&mut missing, "zzz");
        assert!(
            missing.title().ends_with("/zzz · no match"),
            "{}",
            missing.title()
        );
    }

    #[test]
    fn the_panes_own_margin_is_not_searched_and_says_so_by_where_it_stops() {
        // `/42` is a question about the code, not about which line it is on.
        // The rule a reader can see is "the gutter is the pane's margin"; the
        // rule they cannot is "the margin is searchable in some layouts", which
        // is what leaving it in would have meant — there is no gutter in
        // rendered markdown and none below `LINE_NUMBER_MIN_WIDTH` columns.
        let dir = TempDir::new("view-search-gutter");
        let body: String = (1..=50).map(|i| format!("let x{i} = 1;\n")).collect();
        let path = dir.write("a.rs", body.as_bytes());
        let mut pane = quiet(dir.path());
        pane.show(&path);

        laid(&mut pane, 40, 10);
        assert_eq!(pane.margin.width, 4, "three digits and the space after");
        assert_eq!(
            pane.margin.rows,
            pane.lines.len(),
            "nothing here is below the gutter"
        );
        query(&mut pane, "42");
        assert_eq!(
            hits(&pane),
            [(41, 9)],
            "row 42's number was matched, or `x42` was not"
        );

        // Narrow enough that there is no gutter: then there is nothing to skip,
        // rather than four columns skipped anyway.
        laid(&mut pane, 20, 10);
        assert_eq!(pane.margin.width, 0);
        assert_eq!(hits(&pane), [(41, 5)]);
    }

    #[test]
    fn a_box_with_nothing_in_it_to_keep_leaves_no_stage_behind() {
        // Three states, not four. A shut search with no hits shows the reader
        // nothing, so an `Esc` that stopped there would be a keypress eaten by
        // a stage they cannot see — and `exit_hint` could not describe it
        // truthfully either.
        let dir = TempDir::new("view-search-empty");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);

        query(&mut pane, "haystack");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(pane.search.is_none());
        assert_eq!(pane.exit_hint(), " · esc→agent");
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);

        // Backspacing past the start of a query is the other way out, and it
        // takes the whole search with it: those keystrokes came from opening
        // the box, so undoing the last undoes the first. `browse.rs` says the
        // same about the list's find.
        query(&mut pane, "ne");
        for _ in 0..3 {
            pane.handle_key(key(KeyCode::Backspace)).unwrap();
        }
        assert!(pane.search.is_none());
    }

    #[test]
    fn nothing_leaves_a_shut_search_with_nothing_marked_by_it() {
        // Four routes produce that state and only one of them is `Esc`, which
        // is the whole reason the guard is its own method rather than a line
        // inside the one that was written first. In every one of them the
        // reader sees no highlighting at all, so an `Esc` that stopped there
        // would be a keypress eaten by a stage nothing on screen mentions.
        let dir = TempDir::new("view-search-settle");

        // Alt+E out of a box that found nothing.
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);
        query(&mut pane, "haystack");
        pane.toggle_browse();
        pane.toggle_browse();
        assert!(pane.search.is_none(), "Alt+E left a shut, empty search");
        assert_eq!(pane.exit_hint(), " · esc→agent");

        // A resize that breaks the matched word across two rows. `line 20` is
        // one word to nobody, and at twelve columns it is two rows.
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);
        query(&mut pane, "the needle in");
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert_eq!(hits(&pane).len(), 3);
        laid(&mut pane, 12, 10);
        assert!(pane.search.is_none(), "a drag left a shut, empty search");

        // The agent rewrites the file without the word in it. The reload keeps
        // the search — that is the point of keeping it — and then there is
        // nothing for it to be about.
        let mut pane = needles(&dir);
        let path = pane.path().unwrap().to_path_buf();
        laid(&mut pane, 40, 10);
        query(&mut pane, "needle");
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        std::fs::write(&path, b"- nothing to find here\n").expect("rewrite");
        pane.reload();
        laid(&mut pane, 40, 10);
        assert!(pane.search.is_none(), "a rewrite left a shut, empty search");
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
    }

    #[test]
    fn the_border_never_promises_what_this_particular_esc_will_not_do() {
        // Two states where `esc→hits` was a lie. It names what *this* press
        // does, which is what makes a three-press Esc describable one press at
        // a time.
        let dir = TempDir::new("view-search-hint");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);

        // Nothing found: this Esc ends the search rather than keeping anything.
        query(&mut pane, "haystack");
        assert_eq!(pane.exit_hint(), " · esc→clear");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        query(&mut pane, "needle");
        assert_eq!(pane.exit_hint(), " · esc→hits");

        // A file waiting: this Esc closes the box, and the very next frame
        // takes the file up and ends the search with the document it was about.
        // The `◆` already in the title is what makes the hint read.
        pane.follow(dir.write("fresh.md", b"# fresh\n"));
        assert_eq!(pane.exit_hint(), " · esc→new file");
        assert!(pane.title().starts_with("◆ "), "{}", pane.title());
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        draw(&mut pane, 40, 10);
        assert!(pane.path().unwrap().ends_with("fresh.md"));
        assert!(pane.search.is_none(), "the hits outlived their document");
    }

    #[test]
    fn enter_keeps_the_query_the_hits_and_the_marks_on_the_page() {
        let dir = TempDir::new("view-search-accept");
        let mut pane = needles(&dir);
        draw(&mut pane, 40, 10);
        query(&mut pane, "needle");

        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert!(
            !pane.takes_input(),
            "Enter is not a second Esc for the hits"
        );
        assert!(pane.title().contains("/needle · 1/3"), "{}", pane.title());

        let term = draw(&mut pane, 40, 10);
        assert!(
            term.backend()
                .buffer()
                .content()
                .iter()
                .any(|c| c.bg == theme::DARK.hit_now_bg),
            "the accepted query stopped being drawn"
        );
    }

    #[test]
    fn the_marks_are_drawn_onto_the_frame_and_never_into_the_cached_layout() {
        // A keystroke that changes which hit is current repaints a screenful of
        // spans. Marking the layout stale to do it would re-wrap the whole
        // document — ~210 ms at `load::MAX_BYTES` — for a change of colour.
        let dir = TempDir::new("view-search-cache");
        let mut pane = needles(&dir);
        draw(&mut pane, 40, 10);
        query(&mut pane, "needle");
        let term = draw(&mut pane, 40, 10);

        let cells = term.backend().buffer().content();
        assert!(
            cells.iter().any(
                |c| c.bg == theme::DARK.hit_now_bg && c.modifier.contains(Modifier::UNDERLINED)
            ),
            "the current hit is not marked, or not marked twice over"
        );
        assert!(
            !pane.dirty,
            "a keystroke inside the box invalidated the layout"
        );
        assert!(
            !pane
                .lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .any(|s| s.style.bg == Some(theme::DARK.hit_bg)
                    || s.style.bg == Some(theme::DARK.hit_now_bg)),
            "the highlight was spliced into the cache rather than the frame"
        );
    }

    #[test]
    fn a_file_arriving_while_the_box_is_open_waits_rather_than_taking_over() {
        // The same rule as the file list, for a sharper reason: taking the file
        // up rebuilds every row, so the hits under a half-typed query would
        // become hits in a document the reader never asked for.
        let dir = TempDir::new("view-search-pending");
        let mut pane = needles(&dir);
        let here = pane.path().unwrap().to_path_buf();
        draw(&mut pane, 40, 10);
        query(&mut pane, "needle");

        let fresh = dir.write("fresh.md", b"# fresh\n");
        pane.follow(fresh.clone());
        draw(&mut pane, 40, 10);
        assert_eq!(
            pane.path(),
            Some(here.as_path()),
            "the document is untouched"
        );
        assert!(pane.has_pending());
        // No border to mark while this pane is the one showing, so it says it
        // itself — exactly as the list does.
        assert!(pane.title().starts_with("◆ "), "{}", pane.title());

        // The key that closes the box is the key that releases the file.
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        draw(&mut pane, 40, 10);
        assert_eq!(pane.path(), Some(fresh.as_path()));
    }

    #[test]
    fn a_search_the_reader_is_no_longer_typing_does_not_hold_a_file_back() {
        // The line is the box, not the search. Somebody reading a document with
        // its hits still marked is the state this whole pane exists to follow,
        // and the hits are re-found against whatever arrives.
        let dir = TempDir::new("view-search-pending-shut");
        let mut pane = needles(&dir);
        draw(&mut pane, 40, 10);
        query(&mut pane, "needle");
        pane.handle_key(key(KeyCode::Enter)).unwrap();

        let fresh = dir.write("fresh.md", b"# fresh\n");
        pane.follow(fresh.clone());
        draw(&mut pane, 40, 10);
        assert_eq!(pane.path(), Some(fresh.as_path()));
    }

    #[test]
    fn a_search_belongs_to_its_document_and_survives_that_document_being_rewritten() {
        let dir = TempDir::new("view-search-reload");
        let mut pane = needles(&dir);
        let path = pane.path().unwrap().to_path_buf();
        draw(&mut pane, 40, 10);
        query(&mut pane, "needle");
        pane.handle_key(key(KeyCode::Enter)).unwrap();

        // The agent rewrites what somebody is halfway through searching, which
        // is the common case rather than the rare one.
        pane.show(&path);
        draw(&mut pane, 40, 10);
        assert_eq!(hits(&pane).len(), 3, "the search went with the reload");

        // A different document is the opposite case: the hits are its rows and
        // the count was never about this file.
        pane.show(dir.write("other.md", b"# other\n"));
        assert!(pane.search.is_none());
        assert_eq!(pane.exit_hint(), " · esc→agent");
    }

    #[test]
    fn the_box_does_not_survive_the_file_list_but_the_marks_do() {
        let dir = TempDir::new("view-search-browse");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);
        query(&mut pane, "needle");

        pane.toggle_browse();
        assert!(!pane.takes_input(), "a box nobody can see was still open");
        assert_eq!(
            pane.exit_hint(),
            " · esc→agent",
            "the list's answer, not the document's"
        );

        pane.toggle_browse();
        assert_eq!(hits(&pane).len(), 3);
        assert!(pane.title().contains("/needle"), "{}", pane.title());
    }

    #[test]
    fn a_pasted_phrase_reaches_the_document_search_and_nothing_else() {
        let dir = TempDir::new("view-search-paste");
        let mut pane = needles(&dir);
        laid(&mut pane, 40, 10);
        // Nothing in the document view can take text until a box is open.
        assert_eq!(pane.handle_paste("needle").unwrap(), Handled::No);

        query(&mut pane, "");
        assert_eq!(pane.handle_paste("needle\nand more").unwrap(), Handled::Yes);
        assert!(pane.title().contains("/needle · 1/3"), "{}", pane.title());
    }

    // --- searching every file ---------------------------------------------

    /// A pane whose startup walk has landed, so the repository search has an
    /// index to sweep.
    ///
    /// The walk is the real one, and deliberately: what `f` can find is defined
    /// by what `files::scan` hands over — the gitignore, the noise list, the
    /// twenty-thousand-file cap — and an index placed by hand here would be
    /// asserting against a fixture rather than against the thing.
    fn scanned(root: &Path) -> ViewerPane {
        let mut pane = ViewerPane::new(root.to_path_buf());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while pane.scan.is_some() && std::time::Instant::now() < deadline {
            pane.tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(pane.scan.is_none(), "the walk never answered");
        // Otherwise the next frame takes up whichever document the walk found
        // newest, which is a race with every assertion below.
        pane.pending = None;
        pane
    }

    /// `f`, a phrase, `Enter`, and then wait for the sweep rather than assume a
    /// schedule.
    fn find_all(pane: &mut ViewerPane, q: &str) {
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('f'))).unwrap(),
            Handled::Yes
        );
        for c in q.chars() {
            pane.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !pane.grep.settled() && std::time::Instant::now() < deadline {
            pane.tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        pane.tick();
        assert!(pane.grep.settled(), "the sweep never finished");
    }

    #[test]
    fn f_opens_the_repository_search_from_the_document_and_from_the_list() {
        // Both, because both are places somebody is when the question occurs to
        // them — and `Esc` has to put them back in the one they left, rather
        // than dropping a reader who was walking a directory into a document.
        let dir = TempDir::new("view-f");
        dir.write("plan.md", b"# plan\n");
        let mut pane = scanned(dir.path());

        assert_eq!(
            pane.handle_key(key(KeyCode::Char('f'))).unwrap(),
            Handled::Yes
        );
        assert!(matches!(pane.mode, Mode::Results { back: Back::Doc }));
        assert!(pane.takes_input(), "the box is open");
        assert!(
            pane.title().starts_with("all files · /"),
            "{}",
            pane.title()
        );
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(matches!(pane.mode, Mode::Doc));

        pane.toggle_browse();
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('f'))).unwrap(),
            Handled::Yes
        );
        assert!(matches!(pane.mode, Mode::Results { back: Back::Browse }));
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(
            matches!(pane.mode, Mode::Browse),
            "back into the list, not into a document nobody asked for"
        );
    }

    #[test]
    fn f_is_a_letter_inside_every_box_it_could_have_been_a_key_in() {
        // The claim `f` rests on is that it is free in both views it is bound
        // in. It is not free in the boxes those views can raise — nothing is —
        // and each of the three asks its box first for exactly that reason.
        let dir = TempDir::new("view-f-letter");
        dir.write("a.md", b"# a\n");

        let mut pane = quiet(dir.path());
        pane.show(dir.path().join("a.md"));
        laid(&mut pane, 40, 10);
        query(&mut pane, "f");
        assert!(
            matches!(pane.mode, Mode::Doc),
            "f opened a view from inside the document's own box"
        );
        assert!(pane.title().contains("/f"), "{}", pane.title());

        let mut pane = quiet(dir.path());
        pane.toggle_browse();
        pane.handle_key(key(KeyCode::Char('/'))).unwrap();
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        assert!(matches!(pane.mode, Mode::Browse));
        assert!(pane.title().contains("/f"), "{}", pane.title());

        // ...including its own.
        let mut pane = quiet(dir.path());
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        assert!(pane.title().contains("/f"), "{}", pane.title());

        // And Ctrl+F is the agent's, here as everywhere: `crate::scroll` hands
        // Ctrl+letter back rather than declining it, so without a guard it
        // would have fallen straight into the arm below.
        let mut pane = quiet(dir.path());
        assert_eq!(pane.handle_key(ctrl('f')).unwrap(), Handled::No);
        assert!(matches!(pane.mode, Mode::Doc));
    }

    #[test]
    fn enter_on_a_result_opens_the_file_and_lands_on_the_match() {
        let dir = TempDir::new("view-result-open");
        std::fs::create_dir_all(dir.path().join("src")).expect("create src");
        let body: String = (1..=60)
            .map(|i| {
                if i == 40 {
                    "let needle = 1;\n".to_string()
                } else {
                    format!("let x{i} = 1;\n")
                }
            })
            .collect();
        std::fs::write(dir.path().join("src").join("main.rs"), body.as_bytes()).expect("write");

        let mut pane = scanned(dir.path());
        draw(&mut pane, 46, 10);
        find_all(&mut pane, "needle");
        assert!(pane.title().contains("· 1 match"), "{}", pane.title());

        assert_eq!(pane.handle_key(key(KeyCode::Enter)).unwrap(), Handled::Yes);
        assert!(
            matches!(pane.mode, Mode::Doc),
            "the list answered its question"
        );
        assert!(
            pane.path().unwrap().ends_with("main.rs"),
            "{:?}",
            pane.path()
        );
        // Not into a box. `Search::open` would have left one, and `q` in a box
        // is a letter — a reader who pressed Enter on a result asked for a
        // document.
        assert!(!pane.takes_input(), "Enter dropped the reader into a box");

        let hit = pane.search.as_ref().expect("the query came with it");
        let row = hit.current().expect("and it found the match").row;
        assert_eq!(row, 39, "line 40 of a source file is the fortieth row");
        assert!(pane.title().contains("/needle · 1/1"), "{}", pane.title());

        draw(&mut pane, 46, 10);
        assert!(
            row >= pane.scroll.offset && row < pane.scroll.offset + 10,
            "row {row} was never scrolled to; offset {}",
            pane.scroll.offset
        );
    }

    #[test]
    fn the_nth_match_of_a_file_is_the_one_that_opens() {
        // What the ordinal is for. A file with four matches has one row per
        // match in the list, and pressing Enter on the third has to land on the
        // third — the pane cannot use the line number, because the rows it
        // scrolls by are not the lines the sweep counted.
        let dir = TempDir::new("view-result-nth");
        dir.write(
            "notes.txt",
            b"needle one\nplain\nneedle two\nneedle three\n",
        );
        let mut pane = scanned(dir.path());
        draw(&mut pane, 46, 10);
        find_all(&mut pane, "needle");
        assert!(pane.title().contains("· 3 matches"), "{}", pane.title());

        pane.handle_key(key(KeyCode::Char('j'))).unwrap();
        pane.handle_key(key(KeyCode::Char('j'))).unwrap();
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(pane.title().contains("/needle · 3/3"), "{}", pane.title());
        assert_eq!(
            pane.search.as_ref().unwrap().current().unwrap().row,
            3,
            "the third match is on the fourth line"
        );
    }

    #[test]
    fn a_match_the_rendering_dropped_is_reported_rather_than_silently_missed() {
        // The documented imprecision, arriving. The sweep read the file's
        // source and found four `**`; the document view renders that markdown,
        // which eats every one of them. Landing the reader at the top of a file
        // they chose for a phrase, with no phrase on screen and nothing saying
        // why, is the failure this has to not be.
        let dir = TempDir::new("view-result-miss");
        dir.write("plan.md", b"# Plan\n\nDo **the thing** and **another**.\n");
        let mut pane = scanned(dir.path());
        draw(&mut pane, 46, 10);
        find_all(&mut pane, "**");
        assert!(pane.title().contains("· 4 matches"), "{}", pane.title());

        // The second of that file's four.
        pane.handle_key(key(KeyCode::Down)).unwrap();
        pane.handle_key(key(KeyCode::Enter)).unwrap();

        assert!(
            pane.search.is_none(),
            "a shut search with nothing marked is a stage Esc cannot describe"
        );
        assert!(
            pane.title().contains("/** · no match · t for source"),
            "{}",
            pane.title()
        );
        assert_eq!(
            pane.exit_hint(),
            " · esc→agent",
            "the notice grew an Esc stage of its own"
        );

        // `t` is the key the title named and this is it delivering: the form
        // that does contain the phrase, at the ordinal that was asked for.
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('t'))).unwrap(),
            Handled::Yes
        );
        assert!(pane.title().contains("/** · 2/4"), "{}", pane.title());
        assert_eq!(pane.exit_hint(), " · esc→clear", "and it is a search again");
    }

    #[test]
    fn a_file_arriving_while_the_results_are_up_waits_rather_than_taking_over() {
        // The third door onto the same rule. Here the yank is sharper than in
        // either of the other two: the row the reader is reaching for `Enter`
        // on would be replaced by a document they never asked about.
        let dir = TempDir::new("view-result-pending");
        dir.write("a.md", b"# a\n");
        let mut pane = quiet(dir.path());
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        let fresh = dir.write("fresh.md", b"# fresh\n");
        pane.follow(fresh.clone());

        draw(&mut pane, 40, 10);
        assert!(pane.path().is_none(), "the document view is untouched");
        assert!(
            pane.has_pending(),
            "and the shell can still mark the border"
        );
        // No border to mark while this pane is the one showing, so it says it
        // itself — exactly as the list and the document's box do.
        assert!(pane.title().starts_with("◆ "), "{}", pane.title());

        pane.handle_key(key(KeyCode::Esc)).unwrap();
        draw(&mut pane, 40, 10);
        assert_eq!(pane.path(), Some(fresh.as_path()));
    }

    #[test]
    fn the_border_promises_the_key_that_esc_presses_in_all_three_views() {
        let dir = TempDir::new("view-result-hint");
        dir.write("a.md", b"# a\n");
        let mut pane = quiet(dir.path());
        assert_eq!(pane.exit_hint(), " · esc→agent");

        // Raised over the document, with nothing behind the box: one press home.
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        assert!(pane.takes_input());
        assert_eq!(pane.exit_hint(), " · esc→page");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(matches!(pane.mode, Mode::Doc));
        assert_eq!(pane.exit_hint(), " · esc→agent");

        // With a query run, the box has something behind it and the press that
        // shuts it is not the press that leaves.
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        for c in "needle".chars() {
            pane.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(!pane.takes_input(), "Enter ran it and shut the box");
        assert_eq!(pane.exit_hint(), " · esc→page");
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        assert_eq!(pane.exit_hint(), " · esc→results");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(pane.exit_hint(), " · esc→page");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(matches!(pane.mode, Mode::Doc));

        // Raised over the list, the same two presses name the list.
        pane.toggle_browse();
        assert_eq!(pane.exit_hint(), " · esc→agent");
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        assert_eq!(pane.exit_hint(), " · esc→results");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert_eq!(pane.exit_hint(), " · esc→list");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(matches!(pane.mode, Mode::Browse));
        assert_eq!(pane.exit_hint(), " · esc→agent");
    }

    #[test]
    fn alt_e_peels_the_results_off_and_then_means_what_it_always_meant() {
        // Three views and one key that has always been about two of them. A key
        // that meant one of three things depending on what happened to be up is
        // a key nobody can press without looking first.
        let dir = TempDir::new("view-result-alte");
        dir.write("a.md", b"# a\n");
        let mut pane = quiet(dir.path());

        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        pane.toggle_browse();
        assert!(matches!(pane.mode, Mode::Browse));
        assert!(!pane.takes_input(), "a box nobody can see was still open");

        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        pane.toggle_browse();
        assert!(matches!(pane.mode, Mode::Doc));
        assert!(!pane.takes_input());
    }

    /// A file whose one match sits across the hard break a 46-column pane puts
    /// in it: 38 columns of padding, then the phrase.
    ///
    /// At 46 the pane keeps 45 for text and spends 4 of those on the line-number
    /// gutter, so a row holds 41 characters of the file and `needle` starts at
    /// 38 — `nee` on one row, `dle` on the next. The sweep reads the line and
    /// finds it; the document search reads the rows and cannot.
    fn split_across_a_wrap(dir: &TempDir) -> std::path::PathBuf {
        let body = format!("{}needle\n", "x".repeat(38));
        dir.write("wide.txt", body.as_bytes())
    }

    #[test]
    fn a_match_a_wrap_split_is_a_miss_with_a_way_out_in_any_body() {
        // The sharpest form of the documented imprecision, and the one that had
        // no notice at all: `f` reports a match over the file's logical lines,
        // the document searches the physical rows those were wrapped into, and
        // in a plain `.txt` there is no `t` to offer. The reader was told the
        // phrase is not here and given nothing to press.
        let dir = TempDir::new("view-wrap-miss");
        split_across_a_wrap(&dir);
        let mut pane = scanned(dir.path());
        draw(&mut pane, 46, 10);
        find_all(&mut pane, "needle");
        assert!(pane.title().contains("· 1 match"), "{}", pane.title());

        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(
            pane.search.is_none(),
            "the row the sweep found is not a row this search can reach"
        );
        assert!(
            pane.title()
                .contains("/needle · no match · widen if a wrap split it"),
            "the notice names no way out: {}",
            pane.title()
        );
        // `t` is not that way out here and must not be advertised as one: there
        // is no second form of a `.txt` to switch to, and the key declines.
        assert_eq!(
            pane.handle_key(key(KeyCode::Char('t'))).unwrap(),
            Handled::No
        );
    }

    #[test]
    fn a_notice_stops_being_shown_the_moment_it_stops_being_true() {
        // Written once and never re-examined, the notice outlived its own
        // subject: widening the pane until the wrap is gone puts the match
        // plainly on screen with `· no match` still in the title. The thing that
        // would have re-checked it is a `Search`, which is exactly what
        // `settle_search` took away, so `revive_search` is the other half of
        // that pair.
        let dir = TempDir::new("view-wrap-revive");
        split_across_a_wrap(&dir);
        let mut pane = scanned(dir.path());
        draw(&mut pane, 46, 10);
        find_all(&mut pane, "needle");
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(pane.title().contains("· no match"), "{}", pane.title());
        let was = pane.scroll.offset;

        // Wide enough that the line no longer breaks.
        draw(&mut pane, 90, 10);
        assert!(
            pane.title().contains("/needle · 1/1"),
            "the notice outlived the wrap that caused it: {}",
            pane.title()
        );
        assert!(pane.missed.is_none());
        assert_eq!(
            pane.scroll.offset, was,
            "a drag is not the reader asking to be taken anywhere"
        );

        // ...and back again: the pair is a cycle, not a one-way door.
        draw(&mut pane, 46, 10);
        assert!(
            pane.title().contains("/needle · no match"),
            "{}",
            pane.title()
        );
    }

    #[test]
    fn a_query_the_reader_typed_keeps_its_sentence_when_a_rebuild_empties_it() {
        // The case Phase 1 left silent, and the reason the notice is not only
        // for seeds. `Esc` on a miss used to drop the search *and* the sentence
        // explaining it in one keystroke, so the pane went from answering a
        // question to saying nothing without anybody asking it to.
        let dir = TempDir::new("view-typed-miss");
        let mut pane = quiet(dir.path());
        pane.show(dir.write("a.md", b"# a\n\nnothing here\n"));
        laid(&mut pane, 40, 10);

        query(&mut pane, "haystack");
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        assert!(pane.search.is_none(), "the state Esc must not stop in");
        assert!(
            pane.title().contains("/haystack · no match"),
            "the sentence went with the search: {}",
            pane.title()
        );
        // And it is still not an `Esc` stage: the next press is the shell's.
        assert_eq!(pane.exit_hint(), " · esc→agent");
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
        // A new question replaces it rather than stacking beside it.
        query(&mut pane, "a");
        assert!(!pane.title().contains("haystack"), "{}", pane.title());
    }

    #[test]
    fn a_result_whose_file_has_gone_does_not_search_the_apology_for_it() {
        // `build` turns an unreadable path into rows of the pane's own voice,
        // and a seed resolved against those found the reader's phrase twice in
        // "no such file — it may have been renamed or deleted" and "Tab for the
        // next markdown file", then reported `1/2` as if it had opened their
        // file at their match.
        let dir = TempDir::new("view-result-gone");
        let path = dir.write("a.txt", b"the file is here\n");
        let mut pane = scanned(dir.path());
        draw(&mut pane, 46, 10);
        find_all(&mut pane, "file");
        assert!(pane.title().contains("· 1 match"), "{}", pane.title());

        std::fs::remove_file(&path).expect("remove");
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        draw(&mut pane, 46, 10);

        assert!(pane.title().contains("unreadable"), "{}", pane.title());
        assert!(
            pane.search.is_none(),
            "the pane's own voice was searched on the reader's behalf"
        );
        assert!(
            pane.title().contains("/file · no match"),
            "and the phrase they came for is still named: {}",
            pane.title()
        );
    }

    #[test]
    fn landing_on_a_different_match_from_the_one_that_was_chosen_says_so() {
        // Three matches in the file, two of them reachable on the page. Asking
        // for the third clamps to the second — a real match of the right phrase,
        // and *not the row the reader pressed Enter on*. `2/2` alone reads as a
        // complete answer to a question they did not ask.
        let dir = TempDir::new("view-result-clamp");
        let body = format!("needle\nneedle\n{}needle\n", "x".repeat(38));
        dir.write("three.txt", body.as_bytes());
        let mut pane = scanned(dir.path());
        draw(&mut pane, 46, 10);
        find_all(&mut pane, "needle");
        assert!(pane.title().contains("· 3 matches"), "{}", pane.title());

        pane.handle_key(key(KeyCode::Char('G'))).unwrap();
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(
            pane.title().contains("/needle · 2/2 · not the 3rd"),
            "the reader was quietly put on a match they did not choose: {}",
            pane.title()
        );

        // The first two rows are exact, and say nothing extra.
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        pane.handle_key(key(KeyCode::Esc)).unwrap();
        pane.handle_key(key(KeyCode::Char('g'))).unwrap();
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(pane.title().contains("/needle · 1/2"), "{}", pane.title());
        assert!(!pane.title().contains("not the"), "{}", pane.title());
    }

    #[test]
    fn a_sweep_running_behind_a_document_does_not_cost_the_agent_frames() {
        // `Enter` on a result leaves the sweep running, which is right — the
        // reader can come back to a complete list. What is not right is a frame
        // per batch: `tick` reporting news re-renders the agent's whole screen
        // for rows nobody can see.
        let dir = TempDir::new("view-result-frames");
        dir.write("a.txt", b"needle\n");
        let mut pane = quiet(dir.path());
        let (grep, mut post) = grep::Grep::detached(dir.path().to_path_buf());
        pane.grep = grep;
        pane.grep
            .set_index(Arc::from(vec!["a.txt".to_string()]), false);

        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        for c in "needle".chars() {
            pane.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        pane.handle_key(key(KeyCode::Enter)).unwrap();

        // While the rows are the thing on screen, a batch is news.
        post(vec![grep::Hit {
            path: "a.txt".into(),
            line: 1,
            start: 0,
            len: 6,
            text: "needle".into(),
            ordinal: 0,
        }]);
        assert!(pane.tick(), "the list grew and nothing asked for a frame");
        assert_eq!(pane.grep.found(), 1);

        // Behind a document it is not, and the list still fills.
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(matches!(pane.mode, Mode::Doc));
        post(vec![grep::Hit {
            path: "a.txt".into(),
            line: 2,
            start: 0,
            len: 6,
            text: "needle".into(),
            ordinal: 1,
        }]);
        assert!(
            !pane.tick(),
            "a batch nobody can see re-rendered the agent's screen"
        );
        assert_eq!(
            pane.grep.found(),
            2,
            "the sweep was cancelled, so coming back shows a list that stopped \
             wherever Enter happened to be pressed"
        );
    }

    #[test]
    fn a_walk_that_stopped_short_reaches_the_count_that_is_drawn_over_it() {
        // The flag has to travel: neither of the walk's caps can be seen from
        // the list it produces, so a count over a truncated walk would be a
        // definite answer about a repository nothing finished reading.
        let dir = TempDir::new("view-scan-cut");
        let mut pane = quiet(dir.path());
        let (tx, rx) = std::sync::mpsc::channel::<Scan>();
        pane.scan = Some(rx);
        tx.send(Scan {
            recent: Vec::new(),
            files: vec!["a.txt".into()],
            cut: true,
        })
        .expect("the pane is listening");
        assert!(pane.tick());

        pane.handle_key(key(KeyCode::Char('f'))).unwrap();
        for c in "zzz".chars() {
            pane.handle_key(key(KeyCode::Char(c))).unwrap();
        }
        pane.handle_key(key(KeyCode::Enter)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !pane.grep.settled() && std::time::Instant::now() < deadline {
            pane.tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        pane.tick();
        assert!(
            pane.title().contains("0+ matches"),
            "a truncated walk produced a definite count: {}",
            pane.title()
        );
    }

    #[test]
    fn the_help_table_names_the_key_that_opens_the_repository_search() {
        // The F1 overlay is one table for the whole program and nothing catches
        // a key that was added and never written down. This is that catch, for
        // the one key this feature binds.
        let (_, what) = crate::keys::HELP
            .iter()
            .find(|(k, _)| *k == "f")
            .expect("f is bound but not in the F1 overlay");
        assert!(what.contains("every file"), "{what}");
    }

    #[test]
    fn what_the_walk_has_not_answered_yet_is_not_the_querys_fault() {
        // Blaming the query for an index that does not exist sends the reader
        // off to fix a query that was never wrong. `browse` drew this line
        // first; a search that reads files has more of a wait to explain.
        let dir = TempDir::new("view-result-early");
        dir.write("a.md", b"# needle\n");
        let mut pane = quiet(dir.path());
        pane.handle_key(key(KeyCode::Char('f'))).unwrap();

        let mut term = Terminal::new(TestBackend::new(46, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 46, 10)))
            .unwrap();
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("Still walking"), "{screen}");
    }

    // --- being pointed at another worktree ---------------------------------

    #[test]
    fn moving_to_another_worktree_starts_the_reader_over_inside_it() {
        let here = TempDir::new("view-root-here");
        let there = TempDir::new("view-root-there");
        let mine = here.write("mine.md", b"# mine\n");
        there.write("theirs.md", b"# theirs\n");

        let mut pane = quiet(here.path());
        pane.show(&mine);
        pane.recent = vec![mine.clone()];
        pane.handle_key(key(KeyCode::Char('j'))).unwrap();
        assert_eq!(pane.path(), Some(mine.as_path()));

        pane.set_root(there.path().to_path_buf());

        assert!(
            matches!(pane.state, State::Empty),
            "the old worktree's document was still on screen under the new \
             workspace's name"
        );
        assert!(
            pane.recent.is_empty(),
            "Tab would have walked straight out of the workspace"
        );
        assert!(pane.pending.is_none());
        assert_eq!(pane.scroll.offset, 0);
        assert!(pane.scan.is_some(), "nothing is walking the new root");

        // `State::Empty` is not a blank screen for its own sake: `tick` opens
        // the newest markdown when the state is `Empty` and a scan lands, so a
        // switch behaves exactly like startup — in the worktree it moved to.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while pane.pending.is_none() && std::time::Instant::now() < deadline {
            pane.tick();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            pane.pending.as_deref().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("theirs.md")),
            "the reader opened on a document of the worktree it left"
        );
    }

    #[test]
    fn the_file_list_in_a_new_worktree_never_opens_in_the_old_one() {
        // `Browser::align_to` short-circuits when the document has not changed,
        // so a browser that kept its `aligned` would answer the first `Alt+E`
        // after a switch with a directory of the tree that is no longer on
        // screen — and every path in it would be one the new workspace has
        // never heard of.
        let here = TempDir::new("view-root-list-here");
        std::fs::create_dir_all(here.path().join("docs")).expect("create docs");
        let design = here.path().join("docs").join("design.md");
        std::fs::write(&design, b"# design\n").expect("write");

        let there = TempDir::new("view-root-list-there");
        there.write("only-there.md", b"# only there\n");

        let mut pane = quiet(here.path());
        pane.show(&design);
        pane.toggle_browse();
        assert!(pane.title().starts_with("docs/"), "{}", pane.title());
        pane.toggle_browse();

        pane.set_root(there.path().to_path_buf());
        // The walk is not what this test is about, and an answer landing
        // halfway through would make it flap.
        pane.scan = None;
        pane.toggle_browse();
        assert!(
            pane.title().starts_with("./"),
            "the list opened in a worktree that is no longer on screen: {}",
            pane.title()
        );

        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("only-there.md"), "{text}");
    }

    #[test]
    fn the_readers_palette_survives_a_move_to_another_worktree() {
        // `raw` and `theme` are decisions about how to *read* rather than facts
        // about a root, which is why they outlive a document — and a `Browser`
        // rebuilt with its own default would put half the pane back to dark on
        // every switch.
        let here = TempDir::new("view-root-theme-here");
        let there = TempDir::new("view-root-theme-there");
        there.write("a.md", b"# a\n");

        let mut pane = quiet(here.path());
        pane.toggle_theme();
        pane.set_root(there.path().to_path_buf());
        pane.scan = None;
        pane.toggle_browse();

        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10)))
            .unwrap();
        for cell in term.backend().buffer().content() {
            assert!(
                cell.bg == theme::LIGHT.bg || cell.bg == theme::LIGHT.sel_bg,
                "the new workspace's list kept the old palette: {:?}",
                cell.bg
            );
        }
    }

    #[test]
    fn a_walk_of_the_old_root_cannot_land_in_the_new_one() {
        // `rescan` guards on `scan.is_none()`, so re-requesting rather than
        // *replacing* would leave the old root's walk in flight and let its
        // answer arrive as the new workspace's index and recency list. Dropping
        // the receiver makes that answer unreachable rather than merely
        // unwanted.
        let here = TempDir::new("view-root-scan-here");
        let there = TempDir::new("view-root-scan-there");

        let mut pane = quiet(here.path());
        let (tx, rx) = std::sync::mpsc::channel::<Scan>();
        pane.scan = Some(rx);

        pane.set_root(there.path().to_path_buf());
        // Whatever the walk of the old root eventually answers goes nowhere.
        assert!(
            tx.send(Scan {
                recent: vec![here.write("stale.md", b"# stale\n")],
                files: vec!["stale.md".into()],
                cut: false,
            })
            .is_err(),
            "the old walk still had somewhere to deliver its answer"
        );
    }

    #[test]
    fn release_events_never_reach_a_pane_but_are_harmless_if_they_do() {
        // The shell filters these (conpty-findings constraint 3). Pinning the
        // pane's behaviour anyway, because a pane that scrolled twice per
        // keystroke would be blamed on the pane.
        let dir = TempDir::new("view-release");
        let mut pane = scrollable(&dir);
        let mut ev = key(KeyCode::Char('j'));
        ev.kind = KeyEventKind::Release;
        pane.handle_key(ev).unwrap();
        assert_eq!(
            pane.scroll.offset, 1,
            "the pane does not inspect kind, the shell does"
        );
    }
}
