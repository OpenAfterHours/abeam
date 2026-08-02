//! Walking to a file, rather than waiting to be pointed at one.
//!
//! The rest of the viewer is passive: it renders whatever the watcher, the git
//! view or the startup walk hands it. This is the one part of the pane a
//! person drives, and driving it has to be *cheap* and *stable*.
//!
//! Cheap, because `render` runs on every keystroke Claude receives. Reading a
//! directory is the only work here that touches the disk, and it happens when
//! the reader moves or asks — never from `render`, and never for more than
//! [`MAX_ENTRIES`] rows. Both bounds matter: this runs on the thread that
//! pumps Claude's pty, so a `read_dir` per frame would put a syscall behind
//! every character typed at the agent next door, and an uncapped one would put
//! a fifty-thousand-entry sort there.
//!
//! Stable, because nothing here may move on its own. A file arriving from the
//! watcher waits while the list is up — see `ViewerPane::render` — so the row
//! under the cursor is always the row that was under it a moment ago.
//!
//! ## Two ways to reach a file, because one is not enough
//!
//! Walking the tree answers "what is in here". It does not answer "where is
//! that file called keymap", and in a repository of any size that is the
//! question actually being asked. `/` opens a find over an index of every file
//! under the root — built by the same worker walk that finds the markdown, so
//! it costs no extra disk — and matches the query as a *subsequence* of the
//! root-relative path. Subsequence rather than substring because that is what
//! lets `capv` reach `crates/abeam/src/panes/viewer.rs`, and typing initials is
//! how anyone who has used a fuzzy finder expects to get there.
//!
//! The ranking is the part that makes it usable rather than merely correct. A
//! query that fits inside the file name beats the same letters spread across
//! directory names, the closest run of them beats a scattered one, and a
//! shorter path breaks the tie. See [`Rank`], whose field order *is* the
//! comparison, and [`subseq`], which is where "closest" is made to mean
//! something specific enough to sort on.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::DEFAULT_VIEWPORT;
use super::theme;
use crate::pane::Handled;
use crate::scroll::{self, Scroll};
use crate::text::{block, clip_line};
use crate::watch::in_noise;

/// How many entries one directory listing will hold.
///
/// `files.rs` caps its walk for the same reason, and this one needs it more:
/// that walk is on a worker thread, while this runs on the thread that pumps
/// Claude's pty and draws its frames. `Enter` on a vendored directory with
/// fifty thousand entries in it would otherwise materialise fifty thousand
/// rows and sort them, between one of Claude's keystrokes and the next.
///
/// Two thousand is far more than anyone scrolls — the find exists precisely so
/// that nobody has to — and is a fraction of a millisecond to sort. Past it the
/// title says the listing was cut short, rather than quietly misreporting what
/// is in the directory.
pub const MAX_ENTRIES: usize = 2_000;

/// How long `r` waits before it will re-read a directory again.
///
/// The console emits a key event per auto-repeat tick, about thirty a second,
/// and the shell drains the whole batch before drawing — so a held `r` is
/// thirty gitignore-aware directory walks a second, on the UI thread. The
/// whole-repository walk is protected by being asynchronous and one-at-a-time
/// (`ViewerPane::rescan`); this one is synchronous and so needs a clock
/// instead. A quarter of a second is well below the repeat rate and well above
/// the fastest anyone presses a key twice on purpose.
const RELOAD_COOLDOWN: Duration = Duration::from_millis(250);

/// What came of a key, in the vocabulary the pane needs rather than the one
/// the shell does.
///
/// `Ignored` maps to `Handled::No`, and that mapping is load-bearing: a key
/// that changed nothing must not cost a frame, and a frame here re-renders
/// Claude's whole screen. It is also how `Esc` with no find open reaches the
/// shell as "back to Claude" while `Esc` inside a find does not.
pub enum Outcome {
    Ignored,
    Moved,
    /// `r`. The listing has been re-read; the find index is the pane's, and
    /// only a walk can refresh that. `changed` is whether the re-read produced
    /// anything different, because the walk is worth starting either way and
    /// the frame is not.
    Refreshed { changed: bool },
    /// Open this file in the document view.
    Open(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The way back up. Always the first row, and never anywhere else.
    Parent,
    Dir,
    File,
}

#[derive(PartialEq, Eq)]
struct Entry {
    kind: Kind,
    /// What the row says: `..`, `src/`, `main.rs`. A directory wears its
    /// slash, because in a one-column list the alternative is colour alone and
    /// colour is the first thing a terminal takes away.
    label: String,
    path: PathBuf,
}

/// An open find. Absent rather than empty-stringed, because "no find" and "a
/// find whose query is empty" are different states: the second one lists every
/// file under the root and swallows `j` as a character.
struct Find {
    query: String,
    /// Indices into [`Browser::index`], best first.
    hits: Vec<usize>,
    sel: usize,
}

pub struct Browser {
    root: PathBuf,
    /// The directory listed. Always the root or something under it — there is
    /// no `..` out of the top, because abeam was started *here*.
    dir: PathBuf,
    entries: Vec<Entry>,
    /// The directory held more than [`MAX_ENTRIES`], and `entries` is a prefix
    /// of it. Said in the title, because a list that silently stops is a list
    /// that lies about a file it did not reach.
    cut: bool,
    /// Whether a listing has been built at all. The first one is deferred to
    /// the first [`Browser::align_to`] rather than done in `new`, because `new`
    /// runs inside `App::new` — before the first frame — and a repository on a
    /// network share would spend that time with nothing at all on screen.
    listed: bool,
    /// The reader's palette, mirrored from the pane so the list matches the
    /// document it opens into. Pushed in by `ViewerPane::toggle_theme` rather
    /// than read from a global: the list is drawn on its own frames, and two
    /// halves of one pane disagreeing about the page colour is the one thing
    /// this must not do.
    theme: theme::Mode,
    sel: usize,
    scroll: Scroll,
    /// The selection moved, or the list changed, since the last frame. See
    /// [`Browser::reveal`].
    follow: bool,
    find: Option<Find>,
    /// Every file under the root, root-relative with `/` separators. Handed
    /// over by the worker walk.
    index: Vec<String>,
    /// Whether that walk has answered. An empty index means two different
    /// things before and after it does, and only one of them is "no match".
    indexed: bool,
    /// The document the listing was last lined up with. See [`Browser::align_to`].
    aligned: Option<PathBuf>,
    /// When `r` last re-read the directory. `None` until it has.
    read_at: Option<Instant>,
}

impl Browser {
    pub fn new(root: PathBuf) -> Self {
        let mut scroll = Scroll::default();
        // Seeded for the same reason `ViewerPane` seeds its own: keys can be
        // drained in the same batch as the `Alt+E` that opened the list, before
        // any frame has said how tall the pane is, and a page measured against
        // a viewport of zero is a page of one row.
        scroll.measure(0, DEFAULT_VIEWPORT);
        Self {
            dir: root.clone(),
            root,
            entries: Vec::new(),
            cut: false,
            listed: false,
            theme: theme::Mode::default(),
            sel: 0,
            scroll,
            follow: false,
            find: None,
            index: Vec::new(),
            indexed: false,
            aligned: None,
            read_at: None,
        }
    }

    /// Mirror the pane's palette. Only `ViewerPane::toggle_theme` calls it.
    pub fn set_theme(&mut self, mode: theme::Mode) {
        self.theme = mode;
    }

    /// Line the listing up with the document on screen, if that is a different
    /// document from the last time it was looked at. Also where the *first*
    /// listing gets built, which is why the pane calls this before showing the
    /// list rather than only when the document changed.
    ///
    /// The condition is the rest of it. Opening the list *beside the file you
    /// are reading* is what makes it useful — the neighbouring files are
    /// almost always the ones wanted. But re-opening it after a detour into a
    /// document and finding it reset to somewhere else is what makes a file
    /// list feel like a dialog instead of a place, so a document that has not
    /// changed moves nothing.
    pub fn align_to(&mut self, doc: Option<&Path>) {
        if self.listed && self.aligned.as_deref() == doc {
            return;
        }
        self.listed = true;
        self.aligned = doc.map(Path::to_path_buf);
        let dir = doc
            .and_then(Path::parent)
            // A path from outside the root — the git view resolves against the
            // worktree top level, which can be above where abeam was started —
            // has no place in the tree this list walks.
            .filter(|d| d.starts_with(&self.root))
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        self.open_dir(&dir, doc);
    }

    /// The worker walk answered. Replaces the find index, keeping the reader on
    /// the row they had chosen if it survived the new walk.
    pub fn set_index(&mut self, files: Vec<String>) {
        let keep = self.hit_path();
        self.index = files;
        self.indexed = true;
        self.refilter(keep);
    }

    /// Is a query being typed? The pane answers `Pane::takes_input` with this,
    /// which decides whether a paste has anywhere to go and what the border
    /// promises as the way out.
    pub fn finding(&self) -> bool {
        self.find.is_some()
    }

    /// Close any find without opening anything. Leaving the list entirely is
    /// one of the ways a query ends: coming back to a stale one is never what
    /// the next `Alt+E` means.
    pub fn cancel_find(&mut self) {
        if self.find.take().is_some() {
            self.reveal();
        }
    }

    pub fn title(&self) -> String {
        match &self.find {
            // The query first, prefixed with the key that opened it, because a
            // title is clipped from the right and the count is the part that
            // can be spared. `/` costs one column and says which mode this is.
            Some(find) => format!(
                "/{} · {} {}",
                find.query,
                find.hits.len(),
                plural(find.hits.len(), "match", "matches")
            ),
            None => {
                // The `..` row is not something *in* this directory, so it is
                // not counted as one. A listing that claims three items and
                // shows two files reads as a bug in the filter.
                let n = self
                    .entries
                    .iter()
                    .filter(|e| e.kind != Kind::Parent)
                    .count();
                let more = if self.cut { "+" } else { "" };
                format!("{} · {n}{more} {}", self.here(), plural(n, "item", "items"))
            }
        }
    }

    pub fn render(&mut self, f: &mut Frame, inner: Rect) {
        let rows = self.rows();
        let height = inner.height as usize;
        // A pane that just got shorter can have left the selection below the
        // fold, and `Scroll::measure` only clamps the offset — it never looks
        // at what is selected. Treated as a move for exactly that reason.
        let resized = height != self.scroll.viewport();
        self.scroll.measure(rows, height);
        if std::mem::take(&mut self.follow) || resized {
            self.scroll_into_view();
        }
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // The scrollbar takes a column from the text rather than sitting on
        // top of it: an elided name is worse than a narrower one.
        let text_w = inner.width - scroll::bar_width(inner.width);
        let lines: Vec<Line> = if rows == 0 {
            block(self.nothing(), text_w as usize, self.theme.theme().dim())
        } else {
            (self.scroll.offset..rows)
                .take(inner.height as usize)
                .map(|i| self.line(i, text_w as usize))
                .collect()
        };

        f.render_widget(
            Paragraph::new(lines),
            Rect {
                width: text_w,
                ..inner
            },
        );
        self.scroll.render_bar(f, inner);
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        match self.find {
            Some(_) => self.find_key(key),
            None => self.list_key(key),
        }
    }

    /// The glance keys — `Alt+J`/`Alt+K`/`Alt+PgDn`/`Alt+PgUp` — arriving as
    /// the bare key the list would have seen had it been focused.
    ///
    /// They move the *view* and nothing else. Reading the pane from the other
    /// side of the window is a read, and silently re-choosing the file `Enter`
    /// would open is not something a read may do. It is the same rule the wheel
    /// follows two methods down, and the reason `Pane::scroll_key` exists at
    /// all.
    pub fn scroll_view(&mut self, key: KeyEvent) -> Handled {
        self.scroll.key(key).unwrap_or(Handled::No)
    }

    pub fn mouse(&mut self, ev: &MouseEvent) -> Outcome {
        if let Some(handled) = self.scroll.mouse(ev) {
            return if handled.is_yes() {
                Outcome::Moved
            } else {
                Outcome::Ignored
            };
        }
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let row = self.scroll.offset + ev.row as usize;
                // Below the last row there is nothing to choose, the same
                // answer `git.rs` gives for a click on a row that is not a
                // file. Snapping the selection to the end of the list instead
                // would mean a click on the empty half of a short pane
                // silently re-aiming `Enter`.
                if row >= self.rows() {
                    return Outcome::Ignored;
                }
                self.select(row)
            }
            _ => Outcome::Ignored,
        }
    }

    /// A pasted path, which only a find has anywhere to put. Pasting something
    /// copied out of Claude's transcript is one of the likelier ways a
    /// particular file gets looked up.
    pub fn paste(&mut self, text: &str) -> Outcome {
        if self.find.is_none() {
            return Outcome::Ignored;
        }
        // One line, and no control characters: a multi-line paste into a
        // one-line box is a mistake, and its first line is the most useful
        // reading of it.
        let line: String = text
            .chars()
            .take_while(|c| *c != '\n' && *c != '\r')
            .filter(|c| !c.is_control())
            .collect();
        if line.is_empty() {
            return Outcome::Ignored;
        }
        if let Some(find) = self.find.as_mut() {
            find.query.push_str(&line);
        }
        self.refilter(None);
        Outcome::Moved
    }

    // --- keys -------------------------------------------------------------

    /// The scroll vocabulary of `crate::scroll`, spelled out again rather than
    /// delegated to it, because here it moves a *selection* and not an offset.
    /// `Scroll::key` measures `G` against the last screenful, which is the
    /// right answer for a document and the wrong one for a list — `End` has to
    /// land on the last entry, not on the first entry of the last page.
    fn list_key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.page() as isize;
        let half = self.half() as isize;

        match key.code {
            KeyCode::Char('d') if ctrl => self.step(half),
            KeyCode::Char('u') if ctrl => self.step(-half),
            // Ctrl+letter is Claude's everywhere else in the program, so the
            // rest must not fall into the plain-letter arms below.
            KeyCode::Char(_) if ctrl => Outcome::Ignored,

            KeyCode::Char('j') | KeyCode::Down => self.step(1),
            KeyCode::Char('k') | KeyCode::Up => self.step(-1),
            KeyCode::Char(' ') | KeyCode::PageDown => self.step(page),
            KeyCode::Char('b') | KeyCode::PageUp => self.step(-page),
            KeyCode::Char('g') | KeyCode::Home => self.select(0),
            KeyCode::Char('G') | KeyCode::End => self.select(usize::MAX),
            KeyCode::Tab => self.wrap(1),
            KeyCode::BackTab => self.wrap(-1),

            KeyCode::Enter => self.enter(),
            // `-` as well as Backspace: it is one key rather than a reach, and
            // it is what every file manager that ever ran in a terminal used.
            KeyCode::Backspace | KeyCode::Char('-') => self.up(),
            KeyCode::Char('/') => {
                self.find = Some(Find {
                    query: String::new(),
                    hits: Vec::new(),
                    sel: 0,
                });
                self.refilter(None);
                Outcome::Moved
            }
            KeyCode::Char('r') => self.refresh(),

            // Esc and q are not ours. The shell reads an unhandled one as
            // "give focus back to Claude", which is the way out of here.
            _ => Outcome::Ignored,
        }
    }

    /// While a find is open every printable key is text — `j`, `q` and `r`
    /// included. That is the trade a type-to-filter makes, and it is why the
    /// selection moves on the arrows and on `Ctrl+N`/`Ctrl+P` here: those are
    /// the two shapes a reader already has in their fingers for a filter box,
    /// and neither of them can be a letter of a filename.
    fn find_key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.page() as isize;
        let half = self.half() as isize;

        match key.code {
            // Cancels the find and stays in the list. Only an Esc with nothing
            // open falls through to the shell as "back to Claude" — being
            // thrown out of the pane by the key that means "never mind" is the
            // single most annoying thing a filter box can do.
            KeyCode::Esc => {
                self.find = None;
                self.reveal();
                Outcome::Moved
            }
            KeyCode::Enter => self.open_hit(),

            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            // The half-page keys keep working in here, because the F1 overlay
            // is one table for the whole program and an open query is not a
            // reason for a documented key to go quietly dead.
            KeyCode::Char('d') if ctrl => self.step(half),
            KeyCode::Char('u') if ctrl => self.step(-half),
            KeyCode::Char(_) if ctrl => Outcome::Ignored,
            KeyCode::Char(c) => {
                if let Some(find) = self.find.as_mut() {
                    find.query.push(c);
                }
                self.refilter(None);
                Outcome::Moved
            }
            KeyCode::Backspace => {
                let Some(find) = self.find.as_mut() else {
                    return Outcome::Ignored;
                };
                if find.query.pop().is_none() {
                    // Backspacing past the start of the query leaves the find.
                    // Those keystrokes came from opening it, so undoing the
                    // last one should undo the first.
                    self.find = None;
                    self.reveal();
                } else {
                    self.refilter(None);
                }
                Outcome::Moved
            }

            KeyCode::Down | KeyCode::Tab => self.step(1),
            KeyCode::Up | KeyCode::BackTab => self.step(-1),
            KeyCode::PageDown => self.step(page),
            KeyCode::PageUp => self.step(-page),
            KeyCode::Home => self.select(0),
            KeyCode::End => self.select(usize::MAX),

            _ => Outcome::Ignored,
        }
    }

    fn enter(&mut self) -> Outcome {
        let Some(entry) = self.entries.get(self.sel) else {
            return Outcome::Ignored;
        };
        let (kind, path) = (entry.kind, entry.path.clone());
        match kind {
            Kind::Parent => self.up(),
            Kind::Dir => {
                self.open_dir(&path, None);
                Outcome::Moved
            }
            Kind::File => {
                // Remembered so that coming back here from the document does
                // not count as a new document and reset the listing.
                self.aligned = Some(path.clone());
                Outcome::Open(path)
            }
        }
    }

    fn up(&mut self) -> Outcome {
        if self.dir == self.root {
            return Outcome::Ignored;
        }
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else {
            return Outcome::Ignored;
        };
        // Landing on the directory just left is what makes climbing back up
        // feel like returning to a place rather than starting over — and it is
        // what lets someone look into three sibling directories in a row
        // without re-finding their position between each one.
        let leaving = self.dir.clone();
        self.open_dir(&parent, Some(&leaving));
        Outcome::Moved
    }

    fn open_hit(&mut self) -> Outcome {
        let Some(rel) = self.hit_path() else {
            return Outcome::Ignored;
        };
        let path = self.root.join(&rel);
        self.aligned = Some(path.clone());
        // The find is spent: it answered its question, and the next `Alt+E`
        // never means "the same query again". Leaving the listing on the
        // opened file means Esc out of the document lands somewhere related to
        // where the reader just was.
        self.find = None;
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        self.open_dir(&dir, Some(&path));
        Outcome::Open(path)
    }

    // --- the listing ------------------------------------------------------

    fn open_dir(&mut self, dir: &Path, select: Option<&Path>) {
        self.dir = dir.to_path_buf();
        (self.entries, self.cut) = list(&self.root, &self.dir, MAX_ENTRIES);
        self.sel = select
            .and_then(|want| self.entries.iter().position(|e| e.path == want))
            .unwrap_or(0);
        self.scroll.to(0);
        self.reveal();
    }

    /// `r`, with the guard a synchronous walk on the UI thread has to have.
    /// See [`RELOAD_COOLDOWN`].
    fn refresh(&mut self) -> Outcome {
        if self.read_at.is_some_and(|at| at.elapsed() < RELOAD_COOLDOWN) {
            return Outcome::Ignored;
        }
        self.read_at = Some(Instant::now());
        Outcome::Refreshed {
            changed: self.reload(),
        }
    }

    /// Re-read the directory, keeping the reader where they were: on the same
    /// path if it is still there, and otherwise on the same *row*, which is
    /// what `git.rs`'s `refind` does for the same reason. The file that
    /// vanished is very often the one that was selected, and jumping to the top
    /// of the list is where a refresh loses someone's place most visibly.
    ///
    /// Returns whether anything on screen actually changed, so a refresh that
    /// found the directory exactly as it was costs no frame.
    fn reload(&mut self) -> bool {
        let (entries, cut) = list(&self.root, &self.dir, MAX_ENTRIES);
        if entries == self.entries && cut == self.cut {
            return false;
        }
        let keep = self.entries.get(self.sel).map(|e| e.path.clone());
        self.entries = entries;
        self.cut = cut;
        self.sel = keep
            .and_then(|want| self.entries.iter().position(|e| e.path == want))
            .unwrap_or(self.sel)
            .min(self.entries.len().saturating_sub(1));
        self.reveal();
        true
    }

    /// Recompute the matches. `keep` is the path the reader had selected, and
    /// it is `None` on every keystroke of a query — where resetting to the
    /// best match is the entire point — and `Some` only when a background walk
    /// replaced the index under them, where it very much is not.
    fn refilter(&mut self, keep: Option<String>) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()) else {
            return;
        };
        let hits = search(&self.index, &query);
        let sel = keep
            .and_then(|keep| hits.iter().position(|&i| self.index[i] == keep))
            .unwrap_or(0);
        // Back to the top only when the selection went there too. A walk
        // landing under an open find must not re-park the *view* any more than
        // it may re-park the choice.
        if sel == 0 {
            self.scroll.to(0);
        }
        if let Some(find) = self.find.as_mut() {
            find.hits = hits;
            find.sel = sel;
        }
        self.reveal();
    }

    // --- selection --------------------------------------------------------

    fn rows(&self) -> usize {
        match &self.find {
            Some(find) => find.hits.len(),
            None => self.entries.len(),
        }
    }

    fn cursor(&self) -> usize {
        match &self.find {
            Some(find) => find.sel,
            None => self.sel,
        }
    }

    fn step(&mut self, delta: isize) -> Outcome {
        let n = self.rows();
        if n == 0 {
            return Outcome::Ignored;
        }
        let to = (self.cursor() as isize + delta).clamp(0, n as isize - 1);
        self.select(to as usize)
    }

    /// Tab wraps where `j` stops. With one screenful of files, Tab from the
    /// last entry back to the first is what a reader means by it; `j` at the
    /// bottom is someone who has arrived at the bottom.
    fn wrap(&mut self, delta: isize) -> Outcome {
        let n = self.rows() as isize;
        if n == 0 {
            return Outcome::Ignored;
        }
        self.select((((self.cursor() as isize + delta) % n + n) % n) as usize)
    }

    fn select(&mut self, to: usize) -> Outcome {
        let n = self.rows();
        if n == 0 {
            return Outcome::Ignored;
        }
        let to = to.min(n - 1);
        if to == self.cursor() {
            // Nothing moved, so nothing was acted on. Reporting otherwise
            // spends a frame — Claude's whole screen included — on a key that
            // could not do anything.
            return Outcome::Ignored;
        }
        match self.find.as_mut() {
            Some(find) => find.sel = to,
            None => self.sel = to,
        }
        self.reveal();
        Outcome::Moved
    }

    /// The selection has moved, or the list under it has changed.
    ///
    /// Scrolled into view twice, and the second time is the one that has to be
    /// there. Here, on the numbers the last frame left behind, so a burst of
    /// keys drained before the next frame pages from roughly the right place.
    /// And again in `render`, because `Scroll` is told the row count by a frame
    /// and by nothing else: climbing out of a one-file directory into a
    /// four-hundred-entry one leaves it believing the list is one row long, so
    /// it clamps the offset to zero and the selected row is off screen with
    /// nothing left to bring it back.
    ///
    /// Deliberately *not* done on every frame. The wheel is allowed to move the
    /// view away from the selection, and a frame that dragged it back would
    /// make scrolling by wheel impossible.
    fn reveal(&mut self) {
        self.follow = true;
        self.scroll_into_view();
    }

    /// Bring the selected row into view without recentring — a list that jumps
    /// under you is harder to read than one that scrolls by a line.
    fn scroll_into_view(&mut self) {
        let row = self.cursor();
        let page = self.scroll.viewport().max(1);
        if row < self.scroll.offset {
            self.scroll.to(row);
        } else if row >= self.scroll.offset + page {
            self.scroll.to(row + 1 - page);
        }
    }

    fn page(&self) -> usize {
        // One row of overlap, the same as everywhere else that pages.
        self.scroll.viewport().saturating_sub(1).max(1)
    }

    fn half(&self) -> usize {
        (self.scroll.viewport() / 2).max(1)
    }

    /// The selected match, as the index spells it.
    fn hit_path(&self) -> Option<String> {
        let find = self.find.as_ref()?;
        self.index.get(*find.hits.get(find.sel)?).cloned()
    }

    // --- drawing ----------------------------------------------------------

    fn line(&self, i: usize, w: usize) -> Line<'static> {
        let spans = match &self.find {
            Some(find) => hit_spans(&self.index[find.hits[i]], self.theme.theme()),
            None => entry_spans(&self.entries[i], self.theme.theme()),
        };

        // Every row is clipped here and nowhere else. A pane that overflows its
        // rect corrupts the frame rather than merely looking wrong.
        let mut spans = clip_line(Line::from(spans), w).spans;
        if i != self.cursor() {
            return Line::from(spans);
        }
        // Padded to the full width, or the highlight would stop at the end of
        // the text instead of marking the row. Both halves of the pair come
        // from the palette — this row repaints the background under it, so a
        // foreground inherited from the page would be a pairing nobody chose
        // and nobody checked.
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        Line::from(spans).style(self.theme.theme().selection())
    }

    /// What an empty list says. Never nothing: a blank pane is indistinguishable
    /// from a broken one, and all three of these states are a keystroke away.
    fn nothing(&self) -> &'static str {
        match &self.find {
            // The walk has not answered yet. "No file matches" here blames the
            // query for an index that does not exist, and the fix — wait a
            // moment — is not one a reader could guess from that.
            Some(_) if !self.indexed => {
                "Still walking the repository. The find will fill in a moment."
            }
            Some(_) => "No file matches. Backspace to widen it, Esc to go back to the list.",
            None => "Nothing here that is not ignored. Backspace to go up, r to look again.",
        }
    }

    /// Where the list is, as the reader thinks of it: relative to the root,
    /// wearing a trailing slash so a directory listing cannot be mistaken for
    /// the name of a file.
    fn here(&self) -> String {
        let rel = self
            .dir
            .strip_prefix(&self.root)
            .unwrap_or(&self.dir)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            "./".to_string()
        } else {
            format!("{rel}/")
        }
    }
}

/// One directory, gitignore-aware, and whether it stopped at `max`.
///
/// `max_depth(1)` is the whole trick: it makes `ignore` yield the directory
/// itself and its immediate children, which buys the same gitignore semantics
/// the startup walk has without a second implementation of them. Doing this by
/// hand would mean re-deriving `.gitignore`, `.git/info/exclude` and the global
/// ignore file, and getting one of them wrong means a listing full of build
/// artefacts.
///
/// `max` is a parameter rather than [`MAX_ENTRIES`] read directly, so that the
/// cap can be tested without materialising two thousand files to test it with.
fn list(root: &Path, dir: &Path, max: usize) -> (Vec<Entry>, bool) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut cut = false;

    let walk = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .parents(true)
        .build();

    for entry in walk.flatten() {
        let path = entry.path();
        // Depth 0 is `dir` itself.
        if path == dir || in_noise(root, path) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let file_type = entry.file_type();
        let kind = if file_type.is_some_and(|t| t.is_dir()) {
            Kind::Dir
        } else if file_type.is_some_and(|t| t.is_file()) {
            Kind::File
        } else {
            // A symlink the walk was not told to follow, a device node, a
            // socket. Nothing the viewer could open.
            continue;
        };
        if dirs.len() + files.len() >= max {
            cut = true;
            break;
        }
        let entry = Entry {
            kind,
            label: if kind == Kind::Dir {
                format!("{name}/")
            } else {
                name.to_string()
            },
            path: path.to_path_buf(),
        };
        if kind == Kind::Dir {
            dirs.push(entry);
        } else {
            files.push(entry);
        }
    }

    dirs.sort_unstable_by(|a, b| ci_cmp(&a.label, &b.label));
    files.sort_unstable_by(|a, b| ci_cmp(&a.label, &b.label));

    let mut out = Vec::with_capacity(dirs.len() + files.len() + 1);
    // The way back out goes first, and directories before files, because a
    // list you navigate is read top-down: the rows that lead somewhere else
    // belong above the rows that end the journey.
    if dir != root
        && let Some(parent) = dir.parent()
    {
        out.push(Entry {
            kind: Kind::Parent,
            label: "..".to_string(),
            path: parent.to_path_buf(),
        });
    }
    out.extend(dirs);
    out.extend(files);
    (out, cut)
}

fn entry_spans(e: &Entry, t: &theme::Theme) -> Vec<Span<'static>> {
    let style = match e.kind {
        Kind::Parent => t.dim(),
        Kind::Dir => Style::default().fg(t.accent),
        // No colour of its own: it inherits the page's foreground, which is the
        // palette's most legible pairing with the background under it.
        Kind::File => Style::default(),
    };
    // A leading space, matching the git view's rows: a name hard against the
    // border reads as if it has been cut off.
    vec![Span::raw(" "), Span::styled(e.label.clone(), style)]
}

/// A find result: the name first, its directory dim behind it.
///
/// Not the path in reading order, which is what the git pane shows. In a
/// forty-column pane a list of full paths clips to a column of directory
/// prefixes, and the name — the thing being searched for and the thing being
/// compared between rows — is the half that would go. Putting it first means
/// it is the half that survives.
fn hit_spans(rel: &str, t: &theme::Theme) -> Vec<Span<'static>> {
    match rel.rsplit_once('/') {
        Some((dir, name)) => vec![
            Span::raw(" "),
            Span::raw(name.to_string()),
            Span::styled(format!("  {dir}/"), t.dim()),
        ],
        None => vec![Span::raw(" "), Span::raw(rel.to_string())],
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { one } else { many }.to_string()
}

/// Case-insensitive ordering, without allocating.
///
/// `README.md` and `readme.md` belong next to each other in a list a person
/// reads, and byte order puts every capital letter above every lowercase one —
/// which in a source tree means the two halves of an alphabet, twice. Compared
/// lazily rather than through `to_lowercase`, because the find index is sorted
/// with this too and that is twenty thousand paths.
pub fn ci_cmp(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

// ---------------------------------------------------------------------------
// the find
// ---------------------------------------------------------------------------

/// How good a match is, smallest first. The field order *is* the comparison —
/// that is what `derive(Ord)` means here, and it is why the fields are in this
/// order rather than a tidier one.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    /// 0 when the query fits inside the file name, 1 when it had to spread
    /// across the directories to match at all. Searching for `viewer` should
    /// offer `panes/viewer.rs` before `viewer/load.rs`, and this is the field
    /// that does it.
    scope: u8,
    /// Characters between the ends of the *closest* run of the query in the
    /// path — the window [`subseq`] reports, which is the narrowest one there
    /// is. A run beats the same letters scattered through a long name.
    span: usize,
    /// How far in that run starts, so a prefix beats the identical run buried
    /// in the middle of something longer.
    start: usize,
    /// The path's own length, last of all: everything else being equal, the
    /// shallower file is the one that was meant.
    len: usize,
}

/// Indices into `index` that match `query`, best first.
fn search(index: &[String], query: &str) -> Vec<usize> {
    if query.is_empty() {
        // Not a ranking question. An empty query is "show me everything",
        // which is the state `/` opens in, and the walk already sorted it.
        return (0..index.len()).collect();
    }
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut hits: Vec<(Rank, usize)> = index
        .iter()
        .enumerate()
        .filter_map(|(i, path)| rank(path, &needle).map(|r| (r, i)))
        .collect();
    // The path itself is the final tiebreak, so the order is total: two files
    // that score identically must not swap places between one keystroke and
    // the next.
    hits.sort_unstable_by(|(ra, ia), (rb, ib)| ra.cmp(rb).then_with(|| index[*ia].cmp(&index[*ib])));
    hits.into_iter().map(|(_, i)| i).collect()
}

fn rank(path: &str, needle: &[char]) -> Option<Rank> {
    let hay: Vec<char> = path.chars().flat_map(char::to_lowercase).collect();
    // Computed on the lowercased form, because `to_lowercase` is not always
    // length-preserving and an offset into the original would then be wrong.
    let name_at = hay.iter().rposition(|&c| c == '/').map_or(0, |i| i + 1);

    if let Some((first, last)) = subseq(&hay[name_at..], needle) {
        return Some(Rank {
            scope: 0,
            span: last - first,
            start: first,
            len: hay.len(),
        });
    }
    let (first, last) = subseq(&hay, needle)?;
    Some(Rank {
        scope: 1,
        span: last - first,
        start: first,
        len: hay.len(),
    })
}

/// The *closest* run of `needle` through `hay` as a subsequence, or `None`.
///
/// A single forward-then-backward pass is not enough, and the difference shows
/// up in ordinary use. Forward finds the earliest possible end and backward
/// tightens the start against it — but the narrowest window need not end at the
/// earliest end. Searching `rs`, that pass measures `browse.rs` from its
/// leading `r`, four characters wide, while `viewer.rs` happens to have the two
/// letters adjacent and scores one. Both files end in `.rs`; ranking either
/// above the other on that is nonsense a reader can see.
///
/// So the pass is repeated from just past each window's start, keeping the
/// narrowest — the standard minimum-window-subsequence sweep. Worst case it is
/// the query length times the path length; a path is forty characters and a
/// query is five, so the whole index costs a fraction of the keystroke that
/// asked for it.
fn subseq(hay: &[char], needle: &[char]) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return Some((0, 0));
    }
    let mut best: Option<(usize, usize)> = None;
    let mut from = 0;

    while from < hay.len() {
        // Forward to the earliest end reachable from `from`.
        let mut want = 0;
        let mut end = None;
        for (i, c) in hay.iter().enumerate().skip(from) {
            if *c == needle[want] {
                want += 1;
                if want == needle.len() {
                    end = Some(i);
                    break;
                }
            }
        }
        // Nothing left to match, and no later start could do better either.
        let Some(end) = end else { break };

        // Backward from it to the latest start, which is the narrowest window
        // ending there.
        let mut want = needle.len();
        let mut start = end;
        for (i, c) in hay[from..=end].iter().enumerate().rev() {
            if *c == needle[want - 1] {
                want -= 1;
                start = from + i;
                if want == 0 {
                    break;
                }
            }
        }

        if best.is_none_or(|(bs, be)| end - start < be - bs) {
            best = Some((start, end));
        }
        from = start + 1;
    }
    best
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// A real tree. `ignore` reads the filesystem and there is no honest way to
    /// fake a directory to it, which is the same reason `crate::testutil`
    /// exists at all.
    ///
    /// A `.git` directory comes with it: `ignore` only applies `.gitignore`
    /// rules inside something it recognises as a repository, so without one
    /// every gitignore assertion below would pass for the wrong reason.
    fn tree(tag: &str, paths: &[&str]) -> TempDir {
        let dir = TempDir::new(tag);
        std::fs::create_dir_all(dir.path().join(".git")).expect("create .git");
        for rel in paths {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixture directory");
            }
            std::fs::write(&path, b"x\n").expect("write fixture");
        }
        dir
    }

    /// A browser showing the root, with a find index the worker walk would have
    /// produced for the same tree, and a viewport as if a frame had been drawn.
    fn browser(dir: &TempDir, index: &[&str]) -> Browser {
        let mut b = Browser::new(dir.path().to_path_buf());
        b.set_index(index.iter().map(|s| (*s).to_string()).collect());
        b.align_to(None);
        viewport(&mut b, 10);
        b
    }

    /// Stand in for a frame of a particular height, for the tests whose subject
    /// is a key rather than a drawing.
    fn viewport(b: &mut Browser, height: usize) {
        let rows = b.rows();
        b.scroll.measure(rows, height);
    }

    /// Draw for real. The only way to exercise what `render` does with the row
    /// count and the pane height together, which is where the two of them are
    /// known at the same time and nowhere else.
    fn draw(b: &mut Browser, width: u16, height: u16) {
        let mut term = Terminal::new(TestBackend::new(width.max(1), height.max(1)))
            .expect("a test terminal");
        term.draw(|f| b.render(f, Rect::new(0, 0, width, height)))
            .expect("draw the list");
    }

    fn labels(b: &Browser) -> Vec<String> {
        b.entries.iter().map(|e| e.label.clone()).collect()
    }

    fn hits(b: &Browser) -> Vec<String> {
        let find = b.find.as_ref().expect("a find is open");
        find.hits.iter().map(|&i| b.index[i].clone()).collect()
    }

    fn selected(b: &Browser) -> String {
        match &b.find {
            Some(find) => b.index[find.hits[find.sel]].clone(),
            None => b.entries[b.sel].label.clone(),
        }
    }

    fn moved(out: Outcome) -> bool {
        matches!(out, Outcome::Moved)
    }

    fn ignored(out: Outcome) -> bool {
        matches!(out, Outcome::Ignored)
    }

    // --- the listing ------------------------------------------------------

    #[test]
    fn the_way_out_comes_first_then_directories_then_files() {
        let dir = tree(
            "browse-order",
            &["zebra.md", "Alpha.md", "src/main.rs", "Docs/one.md"],
        );
        let mut b = browser(&dir, &[]);
        assert_eq!(labels(&b), ["Docs/", "src/", "Alpha.md", "zebra.md"]);

        // ...and a parent entry appears exactly where there is a parent to go
        // to, which is everywhere but the root.
        b.select(1);
        assert!(moved(b.enter()));
        assert_eq!(labels(&b), ["..", "main.rs"]);
    }

    #[test]
    fn the_first_listing_waits_until_the_list_is_actually_wanted() {
        // `Browser::new` runs inside `App::new`, before the first frame. A
        // gitignore-aware read of the root there is a stall with nothing on
        // screen at all — on a network share, a long one.
        let dir = tree("browse-lazy", &["a.md"]);
        let mut b = Browser::new(dir.path().to_path_buf());
        assert!(b.entries.is_empty(), "nothing was read at construction");

        b.align_to(None);
        assert_eq!(labels(&b), ["a.md"]);
    }

    #[test]
    fn build_output_and_ignored_files_are_not_offered() {
        let dir = tree(
            "browse-ignored",
            &[
                ".gitignore",
                "secret.key",
                "kept.md",
                "target/debug/thing",
                "node_modules/pkg/index.js",
            ],
        );
        std::fs::write(dir.path().join(".gitignore"), b"*.key\n").expect("write ignore");

        let b = browser(&dir, &[]);
        // `target` and `node_modules` are `crate::watch`'s noise list; the key
        // file is `.gitignore`'s doing; `.gitignore` itself and `.git` are
        // hidden. A file list nobody has to scroll past build output.
        assert_eq!(labels(&b), ["kept.md"]);
    }

    #[test]
    fn a_directory_too_big_to_list_is_cut_short_and_says_so() {
        // The cap is what keeps `Enter` on a vendored directory from sorting
        // fifty thousand rows on the thread that pumps Claude's pty.
        let dir = tree("browse-cap", &["a.md", "b.md", "c.md", "d.md"]);
        let (entries, cut) = list(dir.path(), dir.path(), 2);
        assert_eq!(entries.len(), 2);
        assert!(cut);

        let (entries, cut) = list(dir.path(), dir.path(), 400);
        assert_eq!(entries.len(), 4);
        assert!(!cut, "a directory that fits is not reported as cut");

        // ...and the reader is told, rather than left believing two files is
        // all there is.
        let mut b = browser(&dir, &[]);
        (b.entries, b.cut) = list(dir.path(), dir.path(), 2);
        assert_eq!(b.title(), "./ · 2+ items");
    }

    #[test]
    fn climbing_back_up_lands_on_the_directory_just_left() {
        // The thing that makes walking a tree feel like moving around rather
        // than restarting: three sibling directories can be looked into one
        // after another without re-finding the position between each.
        let dir = tree(
            "browse-updown",
            &["a/one.md", "b/two.md", "c/three.md", "d/four.md"],
        );
        let mut b = browser(&dir, &[]);
        assert_eq!(labels(&b), ["a/", "b/", "c/", "d/"]);

        b.select(2);
        assert!(moved(b.enter()));
        assert_eq!(labels(&b), ["..", "three.md"]);

        assert!(moved(b.key(key(KeyCode::Backspace))));
        assert_eq!(b.entries[b.sel].label, "c/", "landed back on where it was");

        // `-` is the same key by another name, and Enter on `..` is a third.
        b.select(3);
        b.enter();
        assert!(moved(b.key(key(KeyCode::Char('-')))));
        assert_eq!(b.entries[b.sel].label, "d/");
        b.enter();
        b.select(0);
        assert!(moved(b.enter()));
        assert_eq!(b.entries[b.sel].label, "d/");
    }

    #[test]
    fn climbing_out_of_a_small_directory_into_a_large_one_shows_the_selection() {
        // `Scroll` learns the row count from a frame and from nothing else, so
        // after this navigation it still believed the list was two rows long,
        // clamped the offset to zero, and drew four hundred entries with the
        // selected one nowhere on screen — and nothing would have brought it
        // back until the next key. Every other test here walks between
        // four-entry directories, which is why this was invisible.
        let paths: Vec<String> = (0..400).map(|i| format!("d{i:03}/one.md")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let dir = tree("browse-bigjump", &refs);

        let mut b = browser(&dir, &[]);
        draw(&mut b, 40, 20);
        assert!(moved(b.key(key(KeyCode::End))));
        draw(&mut b, 40, 20);
        assert_eq!(selected(&b), "d399/");

        b.enter(); // into a directory with one file in it
        draw(&mut b, 40, 20);
        assert_eq!(b.rows(), 2);

        assert!(moved(b.key(key(KeyCode::Char('-')))));
        draw(&mut b, 40, 20);
        assert_eq!(selected(&b), "d399/", "back on the directory just left...");
        assert!(
            b.sel >= b.scroll.offset && b.sel < b.scroll.offset + 20,
            "...and on screen: row {} in a window of 20 starting at {}",
            b.sel,
            b.scroll.offset
        );
    }

    #[test]
    fn a_pane_that_shrank_still_shows_what_is_selected() {
        // `Scroll::measure` clamps the offset and never looks at the selection,
        // so a drag that halves the window can leave it below the fold.
        let paths: Vec<String> = (0..40).map(|i| format!("f{i:02}.md")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let dir = tree("browse-shrink", &refs);

        let mut b = browser(&dir, &[]);
        draw(&mut b, 40, 20);
        b.key(key(KeyCode::End));
        draw(&mut b, 40, 20);
        assert!(b.sel >= b.scroll.offset && b.sel < b.scroll.offset + 20);

        draw(&mut b, 40, 6);
        assert!(
            b.sel >= b.scroll.offset && b.sel < b.scroll.offset + 6,
            "row {} is outside the window of 6 at {}",
            b.sel,
            b.scroll.offset
        );
    }

    #[test]
    fn the_root_has_nowhere_further_up_to_go() {
        // This is where abeam was started. A list that walked out of the
        // repository would be offering files no other part of the program can
        // talk about.
        let dir = tree("browse-top", &["a.md"]);
        let mut b = browser(&dir, &[]);
        assert!(ignored(b.key(key(KeyCode::Backspace))));
        assert!(ignored(b.key(key(KeyCode::Char('-')))));
        assert_eq!(b.title(), "./ · 1 item");
    }

    #[test]
    fn enter_on_a_file_asks_for_it_rather_than_opening_it_here() {
        let dir = tree("browse-open", &["docs/design.md"]);
        let mut b = browser(&dir, &[]);
        b.enter();
        // Past the `..` row, which is the one thing always in the same place.
        b.select(1);
        let out = b.key(key(KeyCode::Enter));
        match out {
            Outcome::Open(path) => assert!(path.ends_with("design.md"), "{path:?}"),
            _ => panic!("Enter on a file must ask for it to be opened"),
        }
    }

    #[test]
    fn a_key_that_moves_nothing_says_it_did_nothing() {
        // A frame here re-renders Claude's whole screen. `j` at the bottom of a
        // list is not worth one.
        let dir = tree("browse-ends", &["a.md", "b.md"]);
        let mut b = browser(&dir, &[]);
        assert!(ignored(b.key(key(KeyCode::Char('k')))));
        assert!(moved(b.key(key(KeyCode::Char('j')))));
        assert!(ignored(b.key(key(KeyCode::Char('j')))));
        // ...but Tab wraps, so it is never a dead key.
        assert!(moved(b.key(key(KeyCode::Tab))));
        assert_eq!(b.sel, 0);
        assert!(moved(b.key(key(KeyCode::BackTab))));
        assert_eq!(b.sel, 1);
    }

    /// The list's half of `scroll.rs`'s equivalent test. The F1 overlay is one
    /// table and it has to be true in the pane that holds a selection too — and
    /// this vocabulary is hand-rolled here rather than delegated, which is
    /// exactly the situation that lets a documented key go quietly dead.
    #[test]
    fn every_key_the_help_overlay_advertises_moves_the_selection() {
        let paths: Vec<String> = (0..100).map(|i| format!("f{i:03}.md")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let dir = tree("browse-vocab", &refs);
        let mut b = browser(&dir, &[]);
        viewport(&mut b, 10);
        assert_eq!(b.rows(), 100);

        b.key(key(KeyCode::Char('j')));
        assert_eq!(b.sel, 1);
        b.key(key(KeyCode::Down));
        assert_eq!(b.sel, 2);
        b.key(key(KeyCode::Char(' ')));
        assert_eq!(b.sel, 2 + 9, "a page keeps one row of overlap");
        b.key(key(KeyCode::Char('b')));
        assert_eq!(b.sel, 2);
        b.key(key(KeyCode::PageDown));
        assert_eq!(b.sel, 11);
        b.key(key(KeyCode::PageUp));
        assert_eq!(b.sel, 2);
        b.key(ctrl('d'));
        assert_eq!(b.sel, 7);
        b.key(ctrl('u'));
        assert_eq!(b.sel, 2);
        b.key(key(KeyCode::Char('k')));
        assert_eq!(b.sel, 1);

        b.key(key(KeyCode::Char('G')));
        assert_eq!(b.sel, 99, "G reaches the last entry, not the last page");
        b.key(key(KeyCode::Char('g')));
        assert_eq!(b.sel, 0);
        b.key(key(KeyCode::End));
        assert_eq!(b.sel, 99);
        b.key(key(KeyCode::Home));
        assert_eq!(b.sel, 0);
    }

    #[test]
    fn the_selection_is_kept_on_screen_as_it_moves() {
        let dir = tree(
            "browse-reveal",
            &["a.md", "b.md", "c.md", "d.md", "e.md", "f.md"],
        );
        let mut b = browser(&dir, &[]);
        viewport(&mut b, 3);

        b.key(key(KeyCode::End));
        assert!(b.sel >= b.scroll.offset && b.sel < b.scroll.offset + 3);
        b.key(key(KeyCode::Char('g')));
        assert_eq!(b.scroll.offset, 0);
    }

    #[test]
    fn the_glance_keys_move_the_view_and_never_the_selection() {
        // `Alt+J` is a read from the other side of the window. A read that
        // silently re-aimed `Enter` at a different file would be worse than a
        // dead key, and the wheel follows the same rule.
        let dir = tree(
            "browse-glance",
            &["a.md", "b.md", "c.md", "d.md", "e.md", "f.md"],
        );
        let mut b = browser(&dir, &[]);
        // A frame first, so the list is in the state a glance actually finds
        // it in: opening it asked for the selection to be brought into view,
        // and that request belongs to the frame after it.
        draw(&mut b, 40, 3);

        assert_eq!(b.scroll_view(key(KeyCode::Down)), Handled::Yes);
        assert_eq!(b.scroll.offset, 1);
        assert_eq!(b.sel, 0, "the selection did not follow the view");
        assert_eq!(b.scroll_view(key(KeyCode::PageDown)), Handled::Yes);
        assert_eq!(b.sel, 0);
        assert_eq!(b.scroll_view(key(KeyCode::Up)), Handled::Yes);
        assert_eq!(b.sel, 0);

        // ...and a frame drawn afterwards leaves the view where the glance put
        // it, or scrolling without focus would be impossible.
        let was = b.scroll.offset;
        assert!(was > 0);
        draw(&mut b, 40, 3);
        assert_eq!(b.scroll.offset, was);
    }

    #[test]
    fn a_click_past_the_end_of_the_list_selects_nothing() {
        let dir = tree("browse-click", &["a.md", "b.md"]);
        let mut b = browser(&dir, &[]);
        let click = |row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row,
            modifiers: KeyModifiers::NONE,
        };
        assert!(moved(b.mouse(&click(1))));
        assert_eq!(b.sel, 1);
        // The empty half of a short pane. Snapping to the last row here would
        // silently re-aim Enter.
        assert!(ignored(b.mouse(&click(7))));
        assert_eq!(b.sel, 1);
    }

    #[test]
    fn a_listing_read_once_is_not_read_again_per_frame() {
        // `render` runs on every keystroke Claude receives, so the listing is
        // cached. Drawn for real, because a `render` that called `list()` every
        // frame would satisfy any assertion that never drew one.
        let dir = tree("browse-cache", &["a.md"]);
        let mut b = browser(&dir, &[]);
        std::fs::write(dir.path().join("b.md"), b"x\n").expect("write");

        for _ in 0..5 {
            draw(&mut b, 40, 10);
        }
        assert_eq!(labels(&b), ["a.md"], "five frames re-read nothing");

        assert!(matches!(
            b.key(key(KeyCode::Char('r'))),
            Outcome::Refreshed { changed: true }
        ));
        assert_eq!(labels(&b), ["a.md", "b.md"]);
    }

    #[test]
    fn holding_r_down_does_not_re_read_the_directory_per_repeat_tick() {
        // The same failure the whole-repository walk is guarded against, and
        // the same key. That guard is "at most one in flight"; this walk is
        // synchronous, so it needs a clock instead.
        let dir = tree("browse-repeat", &["a.md"]);
        let mut b = browser(&dir, &[]);
        assert!(matches!(
            b.key(key(KeyCode::Char('r'))),
            Outcome::Refreshed { .. }
        ));

        std::fs::write(dir.path().join("b.md"), b"x\n").expect("write");
        for _ in 0..30 {
            assert!(
                ignored(b.key(key(KeyCode::Char('r')))),
                "a held r must not walk the directory again, or cost a frame"
            );
        }
        assert_eq!(labels(&b), ["a.md"], "and it did not touch the disk");
    }

    #[test]
    fn a_refresh_that_found_nothing_new_costs_no_frame() {
        let dir = tree("browse-nochange", &["a.md"]);
        let mut b = browser(&dir, &[]);
        assert!(matches!(
            b.key(key(KeyCode::Char('r'))),
            Outcome::Refreshed { changed: false },
        ));
    }

    #[test]
    fn a_refresh_after_the_selected_file_was_deleted_stays_on_the_same_row() {
        // By path first and by row otherwise, which is what `git.rs` does for
        // the same reason: the file that vanished is very often the one that
        // was selected, and jumping to the top is where a refresh loses
        // someone's place most visibly.
        let dir = tree("browse-deleted", &["a.md", "b.md", "c.md", "d.md"]);
        let mut b = browser(&dir, &[]);
        b.select(2);
        assert_eq!(selected(&b), "c.md");

        std::fs::remove_file(dir.path().join("c.md")).expect("delete the selected file");
        b.read_at = None; // the cooldown is not this test's subject
        assert!(matches!(
            b.key(key(KeyCode::Char('r'))),
            Outcome::Refreshed { changed: true }
        ));
        assert_eq!(labels(&b), ["a.md", "b.md", "d.md"]);
        assert_eq!(selected(&b), "d.md", "the same row, not the top of the list");
    }

    #[test]
    fn a_directory_that_disappears_under_the_reader_still_has_a_way_out() {
        let dir = tree("browse-gone", &["docs/one.md", "docs/two.md"]);
        let mut b = browser(&dir, &[]);
        b.enter();
        assert_eq!(labels(&b), ["..", "one.md", "two.md"]);
        b.select(2);

        std::fs::remove_dir_all(dir.path().join("docs")).expect("delete the directory");
        b.read_at = None;
        b.key(key(KeyCode::Char('r')));
        // Nothing left to list, but the way back is not something the disk has
        // an opinion about — and a pane with almost no rows must not panic when
        // it is drawn or when Enter arrives.
        assert_eq!(labels(&b), [".."]);
        draw(&mut b, 40, 10);
        assert!(moved(b.key(key(KeyCode::Enter))));
        assert_eq!(b.here(), "./");
    }

    // --- the find ---------------------------------------------------------

    #[test]
    fn a_find_matches_a_subsequence_of_the_whole_path() {
        // The point of the index: `notes/keymap.md` is nowhere near the
        // directory the list is sitting in, and typing four letters reaches it.
        let dir = tree("find-subseq", &["a.md"]);
        let mut b = browser(
            &dir,
            &["notes/keymap.md", "src/main.rs", "crates/abeam/src/app.rs"],
        );

        b.key(key(KeyCode::Char('/')));
        assert_eq!(hits(&b).len(), 3, "an empty query is everything");

        for c in "kymp".chars() {
            b.key(key(KeyCode::Char(c)));
        }
        assert_eq!(hits(&b), ["notes/keymap.md"]);
        assert!(b.title().starts_with("/kymp"), "{}", b.title());
        assert!(b.title().contains("1 match"), "{}", b.title());

        // Case folds both ways, and a query that matches nothing says so
        // rather than silently keeping the last result.
        b.key(key(KeyCode::Backspace));
        b.key(key(KeyCode::Char('P')));
        assert_eq!(hits(&b), ["notes/keymap.md"]);
        b.key(key(KeyCode::Char('z')));
        assert!(hits(&b).is_empty());
        assert!(b.title().contains("0 matches"), "{}", b.title());
    }

    #[test]
    fn a_hit_in_the_file_name_outranks_one_spread_across_directories() {
        let dir = tree("find-rank", &["a.md"]);
        let mut b = browser(
            &dir,
            &[
                "viewer/load.rs",
                "very/interesting/elsewhere/we/read.rs",
                "panes/viewer.rs",
            ],
        );
        b.key(key(KeyCode::Char('/')));
        for c in "viewer".chars() {
            b.key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            hits(&b),
            [
                // In the name.
                "panes/viewer.rs",
                // In a directory name — still one run, so it beats...
                "viewer/load.rs",
                // ...a match stretched across four path components.
                "very/interesting/elsewhere/we/read.rs",
            ]
        );
    }

    #[test]
    fn between_two_hits_in_the_name_the_tighter_and_shorter_one_wins() {
        let dir = tree("find-tight", &["a.md"]);
        let mut b = browser(&dir, &["app.rs", "a-much-longer-p-r-s.rs", "apple/pear.rs"]);
        b.key(key(KeyCode::Char('/')));
        for c in "apr".chars() {
            b.key(key(KeyCode::Char(c)));
        }
        assert_eq!(
            hits(&b),
            ["app.rs", "a-much-longer-p-r-s.rs", "apple/pear.rs"]
        );
    }

    #[test]
    fn an_extension_query_does_not_rank_files_that_all_share_it() {
        // Every path here ends `.rs`, so the closest run of `rs` is the same in
        // all of them and the ranking has to fall through to the tiebreaks. A
        // single forward-then-backward pass measured `browse.rs` from its first
        // `r`, four characters away, and put `viewer.rs` above it on a number
        // that described neither match.
        let dir = tree("find-ext", &["a.md"]);
        let mut b = browser(&dir, &["browse.rs", "viewer.rs", "app.rs"]);
        b.key(key(KeyCode::Char('/')));
        for c in "rs".chars() {
            b.key(key(KeyCode::Char(c)));
        }
        // Shortest first, then alphabetical — never one of the longer ones.
        assert_eq!(hits(&b), ["app.rs", "browse.rs", "viewer.rs"]);
    }

    #[test]
    fn esc_cancels_the_find_and_stays_in_the_list() {
        // Only an Esc with nothing open falls through to the shell as "back to
        // Claude". Being thrown out of the pane by the key that means "never
        // mind" is the worst thing a filter box can do.
        let dir = tree("find-esc", &["a.md", "b.md"]);
        let mut b = browser(&dir, &["a.md", "b.md"]);
        b.select(1);

        b.key(key(KeyCode::Char('/')));
        b.key(key(KeyCode::Char('a')));
        assert!(b.finding());

        assert!(moved(b.key(key(KeyCode::Esc))));
        assert!(!b.finding(), "the find is gone");
        assert_eq!(b.sel, 1, "and the list is exactly where it was left");

        assert!(
            ignored(b.key(key(KeyCode::Esc))),
            "the second Esc is the shell's"
        );
    }

    #[test]
    fn q_is_a_letter_in_a_find_and_the_way_out_of_the_list() {
        let dir = tree("find-q", &["a.md"]);
        let mut b = browser(&dir, &["quick.md", "slow.md"]);
        // In the list it is nobody's: the shell reads an unhandled one as
        // "give focus back to Claude".
        assert!(ignored(b.key(key(KeyCode::Char('q')))));

        b.key(key(KeyCode::Char('/')));
        assert!(moved(b.key(key(KeyCode::Char('q')))));
        assert_eq!(hits(&b), ["quick.md"]);
    }

    #[test]
    fn backspacing_past_the_start_of_a_query_leaves_the_find() {
        let dir = tree("find-back", &["a.md"]);
        let mut b = browser(&dir, &["a.md"]);
        b.key(key(KeyCode::Char('/')));
        b.key(key(KeyCode::Char('a')));
        b.key(key(KeyCode::Backspace));
        assert!(b.finding(), "the query is empty, the find is not");
        b.key(key(KeyCode::Backspace));
        assert!(!b.finding());
    }

    #[test]
    fn while_a_find_is_open_letters_are_text_and_the_arrows_are_the_selection() {
        let dir = tree("find-keys", &["a.md"]);
        let mut b = browser(&dir, &["jack.md", "jill.md", "june.md"]);
        b.key(key(KeyCode::Char('/')));

        // `j` would be "down" in the list and is a letter here.
        b.key(key(KeyCode::Char('j')));
        assert_eq!(hits(&b).len(), 3);
        assert_eq!(b.cursor(), 0);

        b.key(key(KeyCode::Down));
        assert_eq!(b.cursor(), 1);
        b.key(ctrl('n'));
        assert_eq!(b.cursor(), 2);
        b.key(ctrl('p'));
        assert_eq!(b.cursor(), 1);
        b.key(key(KeyCode::Up));
        assert_eq!(b.cursor(), 0);
        // ...and the half-page keys the overlay promises still work in here.
        b.key(ctrl('d'));
        assert_eq!(b.cursor(), 2);
        b.key(ctrl('u'));
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn a_pasted_path_extends_the_query_and_only_inside_a_find() {
        let dir = tree("find-paste", &["a.md"]);
        let mut b = browser(&dir, &["docs/design.md", "a.md"]);
        assert!(ignored(b.paste("design")), "the list has nowhere to put it");

        b.key(key(KeyCode::Char('/')));
        assert!(moved(b.paste("docs/design")));
        assert_eq!(hits(&b), ["docs/design.md"]);

        // A path copied out of a transcript arrives with its newline attached,
        // and sometimes with the next line as well.
        b.key(key(KeyCode::Esc));
        b.key(key(KeyCode::Char('/')));
        assert!(moved(b.paste("design.md\nsomething else\n")));
        assert_eq!(hits(&b), ["docs/design.md"]);
    }

    #[test]
    fn opening_a_match_leaves_the_list_sitting_on_the_file_it_opened() {
        let dir = tree("find-open", &["docs/design.md", "a.md"]);
        let mut b = browser(&dir, &["a.md", "docs/design.md"]);
        b.key(key(KeyCode::Char('/')));
        for c in "design".chars() {
            b.key(key(KeyCode::Char(c)));
        }
        let out = b.key(key(KeyCode::Enter));
        match out {
            Outcome::Open(path) => assert!(path.ends_with("design.md"), "{path:?}"),
            _ => panic!("Enter on a match must open it"),
        }
        // The find is spent, and Esc out of the document now lands somewhere
        // related to where the reader just was rather than back at the root.
        assert!(!b.finding());
        assert_eq!(b.here(), "docs/");
        assert_eq!(b.entries[b.sel].label, "design.md");
    }

    #[test]
    fn a_walk_landing_under_an_open_find_moves_neither_the_selection_nor_the_view() {
        // The startup walk answers once and `r` starts another, and either can
        // land while someone is halfway through choosing a file. Re-parking the
        // view is as disruptive as re-parking the choice.
        let names: Vec<String> = (0..40).map(|i| format!("f{i:02}.md")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let dir = tree("find-reindex", &["a.md"]);
        let mut b = browser(&dir, &refs);

        b.key(key(KeyCode::Char('/')));
        draw(&mut b, 40, 10);
        b.key(key(KeyCode::End));
        draw(&mut b, 40, 10);
        let chosen = selected(&b);
        assert_eq!(chosen, "f39.md");
        assert!(b.scroll.offset > 0, "the view had to scroll to show it");
        // Where the chosen row sits *on screen*, which is what a reader would
        // notice moving. The row index itself is allowed to shift, because a
        // walk that found a new file sorting above this one moves everything
        // below it down by one.
        let row_on_screen = b.cursor() - b.scroll.offset;

        let mut grown = names.clone();
        grown.push("brand-new.md".to_string());
        grown.sort();
        b.set_index(grown);
        draw(&mut b, 40, 10);

        assert_eq!(selected(&b), chosen, "still on the file they had chosen");
        assert_eq!(
            b.cursor() - b.scroll.offset,
            row_on_screen,
            "and it did not move on screen either"
        );
    }

    #[test]
    fn a_find_opened_before_the_walk_answers_says_so_rather_than_no_match() {
        // Blaming the query for an index that does not exist yet sends the
        // reader off to fix a query that was never wrong.
        let dir = tree("find-early", &["a.md"]);
        let mut b = Browser::new(dir.path().to_path_buf());
        b.align_to(None);
        b.key(key(KeyCode::Char('/')));
        assert!(b.nothing().contains("Still walking"), "{}", b.nothing());

        b.set_index(Vec::new());
        assert!(b.nothing().contains("No file matches"), "{}", b.nothing());
    }

    // --- ranking, directly ------------------------------------------------

    #[test]
    fn a_subsequence_is_measured_by_the_closest_run_of_it_there_is() {
        let closest = |hay: &str, needle: &str| {
            subseq(
                &hay.chars().collect::<Vec<_>>(),
                &needle.chars().collect::<Vec<_>>(),
            )
        };
        // A greedy forward pass alone finds `a`@0 and the only `b`, and calls a
        // match spanning the whole string tight.
        assert_eq!(closest("a-a-ab", "ab"), Some((4, 5)));
        // Forward-then-backward is not enough either: it finds the narrowest
        // window ending at the *earliest* end, which here is four characters
        // wide when there is a pair sitting adjacent further along.
        assert_eq!(closest("axxxbab", "ab"), Some((5, 6)));
        assert_eq!(closest("browse.rs", "rs"), Some((7, 8)));

        assert_eq!(closest("a-a-ab", "z"), None);
        assert_eq!(closest("a-a-ab", ""), Some((0, 0)));
        assert_eq!(closest("", "a"), None);
    }

    // --- drawing ----------------------------------------------------------

    #[test]
    fn every_row_fits_the_width_it_was_given() {
        // A row that overflows its rect corrupts the frame rather than merely
        // looking wrong, so this holds at widths no one would choose too.
        let dir = tree(
            "browse-widths",
            &["a-really-quite-long-file-name-indeed.md", "src/main.rs"],
        );
        let mut b = browser(&dir, &["crates/abeam/src/panes/viewer/browse.rs"]);
        for find in [false, true] {
            if find {
                b.key(key(KeyCode::Char('/')));
            }
            for w in [1usize, 2, 8, 22, 46, 120] {
                for i in 0..b.rows() {
                    let line = b.line(i, w);
                    let used: usize = line.spans.iter().map(|s| s.content.width()).sum();
                    assert!(used <= w, "row {i} is {used} cells wide at {w}");
                }
            }
        }
    }

    #[test]
    fn drawing_at_hostile_sizes_does_not_panic() {
        let dir = tree("browse-sizes", &["a.md", "docs/b.md"]);
        let mut b = browser(&dir, &["a.md", "docs/b.md"]);
        for (w, h) in [(0, 0), (1, 1), (2, 20), (25, 1), (60, 20)] {
            draw(&mut b, w, h);
        }
        b.key(key(KeyCode::Char('/')));
        for (w, h) in [(0, 0), (1, 1), (60, 20)] {
            draw(&mut b, w, h);
        }
        assert!(b.finding());
    }

    #[test]
    fn an_empty_list_says_something_rather_than_nothing() {
        // A blank pane is indistinguishable from a broken one, and both of
        // these states are one keystroke away.
        let dir = tree("browse-empty", &[]);
        let mut b = browser(&dir, &[]);
        assert_eq!(b.rows(), 0);
        assert!(b.nothing().contains("Nothing here"));
        draw(&mut b, 40, 10);

        b.key(key(KeyCode::Char('/')));
        b.key(key(KeyCode::Char('z')));
        assert!(b.nothing().contains("No file matches"));
        draw(&mut b, 40, 10);
    }

    #[test]
    fn the_title_says_where_it_is_before_it_says_how_much_is_in_it() {
        // Titles are clipped from the right in a 46-column pane, so the path is
        // the half that has to survive.
        let dir = tree("browse-title", &["docs/one.md", "docs/two.md"]);
        let mut b = browser(&dir, &[]);
        b.enter();
        assert_eq!(b.title(), "docs/ · 2 items");
    }
}
