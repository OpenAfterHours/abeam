//! Finding a phrase in every file under the root, rather than in the one
//! document on screen.
//!
//! Three questions, three answers, and this is the third. [`super::search`]
//! reaches a *place* in the document the reader already has open.
//! [`super::browse`]'s find reaches a *file* by its name. This one answers
//! "which files say this", and it is the only one of the three that has to read
//! the disk to answer, which is what the rest of this module is arranged
//! around.
//!
//! ## It does not walk the tree
//!
//! `files::spawn_scan` walked it already, gitignore-aware, and handed back
//! every non-ignored file under the root. That list is an `Arc<[String]>` the
//! browser holds, this module holds and each query carries, so the repository
//! is walked once per `r` rather than once per query, and twenty thousand
//! strings are never copied. `files.rs` puts the argument as "one walk answers
//! two questions", and gives the reason: a second gitignore walk of the
//! repository doubles the disk cost of startup to re-derive what the first one
//! already had in its hand. That does not get weaker for a third question.
//!
//! What comes with the list is two caps this module did not choose and cannot
//! see from the inside — the walk stops after `files::MAX_ENTRIES` entries and
//! keeps at most `files::MAX_FILES` of the files it saw — and a third arrives
//! per file, because [`load::load`] reads at most `load::MAX_BYTES`, so a match
//! in the second megabyte of a generated report is not there to be found. They
//! are why a count is written `137+` rather than `137`: see [`Grep::title`].
//!
//! ## Enter, not every keystroke
//!
//! The find over file *names* runs on each key, and should: it is a pass over
//! twenty thousand strings already in memory and costs a fraction of the
//! keystroke that asked for it. This is thousands of file reads. Per keystroke
//! it would start a sweep of the whole repository for `n`, `ne`, `nee` and
//! `need` before the reader had finished typing `needle`, and throw four of
//! them away — four times the disk, and on a cold repository the answer to the
//! finished query arrives *after* the answers to the abandoned ones would have.
//! So the box waits for `Enter`. An empty list says that in words, because a
//! box that visibly does nothing while you type is otherwise indistinguishable
//! from one that is broken.
//!
//! ## The worker
//!
//! `git.rs` is the model, down to the shape of the channels: a long-lived
//! thread, an [`Ask`] stamped with a generation, and a pane that only ever
//! `try_recv`s. Two things are this module's own.
//!
//! Answers come back in [batches](Batch) rather than as one result, so the list
//! fills as the sweep goes. This repository is 57 files and 1.9 MB and sweeps
//! in 44 ms in a release build; `files::MAX_FILES` allows three hundred and
//! fifty times that many files, which is where the wait stops being something
//! only a benchmark notices. A list that appeared all at once at the end would
//! have the reader believing the key had done nothing for the whole time it was
//! working.
//!
//! And a superseded query is *abandoned*, not merely ignored. `git.rs` can
//! afford to let a stale refresh finish, because it is one `git status` and
//! there is nothing behind it. Here a second query would queue behind the first
//! sweep's remaining nineteen thousand files. So the generation lives in an
//! `AtomicU64` the pane writes and the worker re-reads between files: a new
//! query changes it, and the sweep in flight stops at its next file rather than
//! at its last one.
//!
//! ## What a row is
//!
//! The path, the line number and the line the match is on, with the match
//! marked in the same colours [`super::search`] uses — one idea of what a hit
//! looks like, whichever search found it. `browse::hit_spans` argues that in a
//! narrow pane the *name* is the half worth keeping, because it is what the
//! reader is comparing between rows; here the match is the other half of that,
//! so the layout gives each of them a column and lets the directory go. See
//! [`Grep::line`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::list::Cursor;
use super::{DEFAULT_VIEWPORT, load, search, source, theme};
use crate::pane::Handled;
use crate::scroll;
use crate::text::{block, clip_line, elide_left, plural};

/// How many matches one file contributes before the sweep moves on.
///
/// The argument is what the *next* keystroke does. `Enter` on a result opens
/// that file with the document search already seeded, and that search is the
/// tool for "where in this file": an exact count in the title, `n` and `N`, and
/// every match highlighted. So the question this list answers is "is this file
/// one of the ones I mean", and past a point extra rows stop answering it and
/// start being a worse version of the search the reader is one key away from.
///
/// The number that argument reaches is [`DEFAULT_VIEWPORT`] — the pane's own
/// idea of a screenful. A file that contributes more than one screenful has
/// stopped being a row in a list and become the list. An earlier draft picked a
/// larger number and defended it with the ratio below, which argues *downwards*
/// and so could never have chosen it; this is the bound the argument actually
/// makes.
///
/// Measured, on this repository — about sixty files the walk keeps, 2 MB of
/// text. `self` matches 2,414 times across 46 files and the busiest single file
/// holds **345** of them: uncapped, that one file is a seventh of the answer and
/// the first 345 rows of it, and four files like it exhaust [`MAX_HITS`] before
/// the sweep reaches the fifth. Capped, the same sweep is 521 rows and every one
/// of the 46 files is in it.
///
/// The cost is that the 21st match in a file is reached with `Enter` and `n`
/// rather than by scrolling, and [`Grep::title`] counts the files that were cut
/// so the cost is never silent. 18 of the 46 hit it on that query.
const MAX_PER_FILE: usize = DEFAULT_VIEWPORT;

/// How many matches the whole sweep reports before it stops.
///
/// Not a time budget. The sweep is on a worker thread, and the whole of this
/// repository is 47 ms in a release build and 570 ms in a debug one — the
/// second being the number to remember before concluding that a `cargo run` of
/// abeam has hung — and none of it is on the thread that draws. The UI thread's
/// share is a `try_recv` per frame.
///
/// **The bound a reader can hold is `MAX_HITS / MAX_PER_FILE` — 75 files.**
/// That is the promise this cap makes and the one worth stating outright: no
/// query is answered with fewer than 75 files' worth of results, whatever any
/// one of them contains. Everything below is why the number is where it is.
///
/// Memory. Each hit carries a path and up to [`PREVIEW`] characters of line —
/// about 310 bytes of ASCII, so ~470 KB at this cap. Characters, not bytes: a
/// repository written in CJK or emoji is up to four bytes each, and the
/// ceiling is nearer **1.3 MB**. That is more than one document at
/// `load::MAX_BYTES` and the same order as the laid-out form of one, which
/// this pane already caches in `lines` — so it is the largest thing the viewer
/// holds and not larger than the largest thing it already held.
///
/// The reader. On this repository `self` matches 2,414 times and `e` matches
/// 163,705; with [`MAX_PER_FILE`] applied those become 521 and 1,170. This cap
/// sits above both. It is deliberately *not* justified as "a query that reaches
/// it has failed to be a query" — that is true of a 57-file repository and
/// false of the monorepo `files::MAX_FILES` is sized for, where a common
/// identifier legitimately lives in nine hundred files. There it is a real
/// truncation of a real answer, which is exactly why reaching it puts a `+` in
/// the title rather than stopping quietly.
const MAX_HITS: usize = 1_500;

/// How much of a matching line the worker carries back.
///
/// A bound on memory rather than on layout, and measured in **characters**, so
/// see [`MAX_HITS`] for what that costs in bytes at the worst. At that cap this
/// is the only part of a result that is not a handful of words, and one
/// minified bundle can put a two-megabyte line behind every hit in it. It is
/// wider than any row can
/// draw: [`Grep::line`] gives the preview three fifths of the pane and the
/// viewer is half of the terminal, so a 240-column terminal — wider than a 4K
/// screen at a legible size — asks for about 70. The margin between the two is
/// what lets `line` window the same text again against a pane that has been
/// dragged, without having to go back to a file the sweep has finished with.
const PREVIEW: usize = 192;

/// How much of the line before the match a preview keeps.
///
/// A match at column 400 has to be *on* the row or the row is not a result, so
/// the preview is a window rather than a prefix. This much of what comes before
/// it comes too, because `= foo(` before a match is what tells the reader which
/// of the file's uses this is, and a window that started exactly at the match
/// would have thrown that away.
const LEAD: usize = 16;

/// One match, in the units the file has rather than the units the pane has.
///
/// A *line* and not a row: nothing has been laid out, and the same file opened
/// in the document view can wrap that line across three rows or — if it is
/// markdown — reflow it into a paragraph that no longer contains it at all.
/// See [`Hit::ordinal`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Root-relative, `/`-separated: the path as `files::rel` spells it, which
    /// is the spelling the file list and the breadcrumb already use.
    pub path: String,
    /// Counting from one, as an editor and everyone talking about code does.
    pub line: usize,
    /// Characters into [`Hit::text`], not into the line: the text is already a
    /// window onto a line that may be far wider than any pane.
    pub start: usize,
    pub len: usize,
    /// The window of the line this match is in. See [`PREVIEW`].
    pub text: String,
    /// Which match of *this file's* this is, counting from zero.
    ///
    /// The whole of what `Enter` can honestly hand to the document search. A
    /// line number would be exact for a source file and meaningless for
    /// rendered markdown, where the rows on screen are a reflow of the text
    /// this was found in; an ordinal is approximate for both and wrong in the
    /// same visible way, which the pane can then say out loud.
    pub ordinal: usize,
}

/// What a sweep could not report, so that the count never claims to be the
/// whole answer when it is not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Report {
    /// [`MAX_HITS`] stopped the sweep before the last file.
    full: bool,
    /// Files with more matches than [`MAX_PER_FILE`] would take.
    cut: usize,
    /// Files that [`load`] read only the first `load::MAX_BYTES` of **and that
    /// put a hit in the list**.
    ///
    /// The second half is the whole of it. Counted on truncation alone — which
    /// is where this started — one generated file over half a megabyte anywhere
    /// under the root puts a `+` on the count of every query for ever, and a
    /// mark that is always on says nothing at all. Tied to a file that actually
    /// contributed, it means what a reader would take it to mean: *this list has
    /// a row from a file you are not seeing all of*.
    ///
    /// It undercounts, and knowingly. A truncated file whose head matched
    /// nothing may match in the tail nobody read, and that is invisible from
    /// here — the alternative is reading the tail, which is the thing
    /// `load::MAX_BYTES` exists to refuse.
    clipped: usize,
}

impl Report {
    /// Is the count below what is really there?
    fn short(&self) -> bool {
        self.full || self.cut > 0 || self.clipped > 0
    }
}

/// A query, and which one it is.
///
/// The file list travels with it rather than being read from something the
/// worker set up once, for the reason `git.rs` gives about its root: the pane
/// can be re-rooted and can finish another walk while a sweep is running, and a
/// worker holding a list from before either would be reporting paths that no
/// longer mean what it thinks.
struct Ask {
    generation: u64,
    root: PathBuf,
    files: Arc<[String]>,
    needle: String,
}

/// Some of the answer to one [`Ask`].
struct Batch {
    generation: u64,
    hits: Vec<Hit>,
    /// The sweep is over, and this is what it could not say. `None` on every
    /// batch but the last.
    done: Option<Report>,
}

/// What came of a key, in the vocabulary the pane needs.
///
/// `Ignored` maps to `Handled::No`, and that mapping is load-bearing for the
/// same reason `browse::Outcome`'s is: a key that changed nothing must not cost
/// a frame, and a frame here re-renders the agent's whole screen.
pub enum Outcome {
    Ignored,
    Moved,
    /// `Esc` with nothing left to close. The pane puts the reader back wherever
    /// `f` was pressed.
    Leave,
    /// Open this file with the document search already looking for `query`, at
    /// that file's `ordinal`th match. See [`Hit::ordinal`].
    Open {
        path: PathBuf,
        query: String,
        ordinal: usize,
    },
}

impl From<Handled> for Outcome {
    fn from(handled: Handled) -> Self {
        if handled.is_yes() {
            Outcome::Moved
        } else {
            Outcome::Ignored
        }
    }
}

/// Being dropped is a cancellation like any other, and has to say so.
///
/// `ViewerPane::set_root` replaces this wholesale, which is how the worker
/// thread is asked to stop: its request channel goes with it, so its next
/// `recv` fails. Its *current* sweep is the problem. Dropping the receiving end
/// only stops that at the next `send`, and a query matching nothing does not
/// send until the last file — so a workspace switch would otherwise read every
/// file of the tree that has just been left, one whole sweep after nobody could
/// hear the answer.
///
/// One store fixes it, because cancellation is already a shared word the sweep
/// re-reads between files. Which of the two the worker notices first — this
/// store or the channel closing a moment later — is not ordered and does not
/// need to be: both mean stop, and it does.
impl Drop for Grep {
    fn drop(&mut self) {
        self.alive
            .store(self.generation.wrapping_add(1), Ordering::Relaxed);
    }
}

pub struct Grep {
    root: PathBuf,
    theme: theme::Mode,
    /// The file list, shared with the browser rather than copied. See
    /// `Browser::index`.
    files: Arc<[String]>,
    /// Whether the walk has answered. An empty list means two different things
    /// before and after it does, and only one of them is "nothing matches" —
    /// the same distinction `browse::nothing` draws, and for the same reason.
    indexed: bool,
    /// The walk that produced `files` stopped short of the tree. See
    /// [`Grep::set_index`].
    partial: bool,

    /// The box is open, so every printable key is a letter of the query.
    typing: bool,
    /// What is in the box.
    query: String,
    /// The query the hits below are about, which is not `query` while somebody
    /// is typing the next one over the top of the last one's results.
    ran: Option<String>,
    hits: Vec<Hit>,
    cursor: Cursor,
    report: Report,
    /// A sweep is out. Display only: what makes an empty list "still reading"
    /// rather than "nothing matches".
    running: bool,

    generation: u64,
    /// The generation the worker is still allowed to be working on.
    ///
    /// Written by the pane before it sends, read by the worker between files.
    /// That is the whole of cancellation, and it is a shared word rather than a
    /// second channel because the worker is inside a loop over twenty thousand
    /// files and the only question it needs to ask there is "is this still
    /// wanted".
    ///
    /// ## Why `Relaxed` is enough, at all four sites
    ///
    /// **Nothing is published through this word.** It carries no pointer and
    /// guards no data: it is a number, compared for equality, and both possible
    /// answers are safe on their own. Read stale, the worker does one more
    /// file's work and then notices; read fresh, it stops a file sooner. There
    /// is no state whose visibility depends on which it saw.
    ///
    /// Everything the worker actually *reads* — the query, the root, the file
    /// list — arrives inside an [`Ask`] through the `mpsc` channel, and every
    /// hit goes back inside a [`Batch`] through the other one. Those channels
    /// carry the happens-before edges, as they must, and they would carry them
    /// whatever this ordering said.
    ///
    /// **So the reason to state it is the change that would break it.** Hang a
    /// pointer, a length, or a "the list has been replaced" flag off this word
    /// and `Relaxed` stops being sufficient — and nothing will catch it, because
    /// x86 gives acquire-release for free and every test here runs on one.
    /// Anything sent to this thread goes through the channel; this word answers
    /// one question and owns nothing.
    alive: Arc<AtomicU64>,
    tx: Sender<Ask>,
    rx: Receiver<Batch>,
    /// Cleared the first time the channel reports the worker gone, so a dead
    /// worker is reported once rather than spun on. Same shape as `git.rs`.
    worker: bool,
}

impl Grep {
    pub fn new(root: PathBuf) -> Self {
        let alive = Arc::new(AtomicU64::new(0));
        let (tx, rx) = spawn_worker(Arc::clone(&alive));
        Self {
            root,
            theme: theme::Mode::default(),
            files: Arc::from(Vec::new()),
            indexed: false,
            partial: false,
            typing: false,
            query: String::new(),
            ran: None,
            hits: Vec::new(),
            // Seeded with a height for the reason `Browser::new` seeds its own:
            // keys can be drained in the same batch as the `f` that opened
            // this, before any frame has said how tall the pane is, and a page
            // measured against a viewport of zero is a page of one row.
            cursor: Cursor::new(DEFAULT_VIEWPORT),
            report: Report::default(),
            running: false,
            generation: 0,
            alive,
            tx,
            rx,
            worker: true,
        }
    }

    /// Mirror the pane's palette, exactly as the browser does — the two halves
    /// of one pane are drawn on separate frames and must not disagree about
    /// what colour the page is.
    pub fn set_theme(&mut self, mode: theme::Mode) {
        self.theme = mode;
    }

    /// The worker walk answered.
    ///
    /// A query asked before it did was a query over nothing, so it is run
    /// again here rather than left showing a "nothing matches" that was never
    /// about the reader's query. Only on the first list: `r` replaces the index
    /// too, and re-sweeping the repository because somebody refreshed a
    /// directory listing is disk nobody asked for.
    /// `cut` is `files::Scan::cut` — the walk stopped short of the tree, so no
    /// count over this list can be a definite one. It has to be handed over
    /// rather than inferred: `files::MAX_ENTRIES` bounds *entries visited* and
    /// `in_noise` filters afterwards, so a list far below `files::MAX_FILES` can
    /// still be all a truncated walk produced.
    pub fn set_index(&mut self, files: Arc<[String]>, cut: bool) {
        let first = !self.indexed;
        self.files = files;
        self.partial = cut;
        self.indexed = true;
        if first && self.ran.is_some() {
            self.submit();
        }
    }

    /// Is a query being typed? `Pane::takes_input` is answered from this, which
    /// decides whether `q` is a letter and where a paste goes.
    pub fn typing(&self) -> bool {
        self.typing
    }

    /// Has the sweep finished?
    ///
    /// Nothing on screen asks — an empty list says "reading the files" from
    /// `running` and a full one needs no such note, since the rows are their own
    /// progress bar. The pane's own suite asks, because a test that drove `f`
    /// and then assumed a schedule would be a test that fails on a slow disk
    /// and passes everywhere it was run.
    #[cfg(test)]
    pub fn settled(&self) -> bool {
        !self.running
    }

    /// How many matches are in the list. Tests only, and for the same reason.
    #[cfg(test)]
    pub fn found(&self) -> usize {
        self.hits.len()
    }

    /// A list whose results arrive when a test says so rather than when a
    /// worker finishes, and a way to deliver one.
    ///
    /// `git.rs`'s `over` is the model and the argument is the same: handed both
    /// ends, a test can answer by hand, which is stricter than racing a real
    /// sweep to a chosen moment rather than weaker. What needs it is the
    /// question of *which view is on screen when a batch lands* — a sweep left
    /// running behind a document must not cost the agent frames — and that
    /// cannot be asked at all if the answer arrives whenever the disk feels like
    /// it.
    ///
    /// The delivery goes through the real channel, so `ViewerPane::tick` sees a
    /// batch exactly as it would in life. The generation is read from `alive`
    /// rather than captured, because the pane bumps it on every query and a
    /// batch stamped with a stale one is one the list is right to throw away.
    #[cfg(test)]
    pub fn detached(root: PathBuf) -> (Self, impl FnMut(Vec<Hit>)) {
        let mut grep = Self::new(root);
        let (tx, rx) = mpsc::channel::<Batch>();
        grep.rx = rx;
        let alive = Arc::clone(&grep.alive);
        let post = move |hits| {
            let generation = alive.load(Ordering::Relaxed);
            let _ = tx.send(Batch {
                generation,
                hits,
                done: None,
            });
        };
        (grep, post)
    }

    /// Is there anything behind the box to come back to?
    ///
    /// The border asks, because `Esc` out of the box means two different things
    /// depending on the answer and may only promise the one it will do.
    pub fn has_results(&self) -> bool {
        self.ran.is_some()
    }

    /// Open the box over whatever is already there.
    ///
    /// **Always the box, never the list behind it**, and that is the decision
    /// worth writing down rather than the emptiness. The tempting alternative is
    /// for `f` to show the previous results when there are some and open the box
    /// when there are not — which is one key instead of two for the commonest
    /// way back. It is refused for `ViewerPane::toggle_browse`'s reason: a key
    /// whose effect depends on whether a sweep happens to be sitting behind it
    /// is a key that has to be *looked at* before it is pressed, and the whole
    /// value of a one-letter binding is that it does not. `Esc` on the empty box
    /// is the second key, and it is the same "undo the keystroke that opened
    /// this" rule both other boxes in this pane already follow.
    ///
    /// Empty rather than pre-filled with the last query, for a smaller reason:
    /// the common case is a different phrase, and a box that came back full
    /// would start it with a held Backspace.
    pub fn open(&mut self) {
        self.typing = true;
        self.query.clear();
    }

    /// Shut the box on the way out of this view entirely.
    ///
    /// `Alt+E` peels the results off and then means what it always meant, and a
    /// box left open behind it would keep `Pane::takes_input` and `exit_hint`
    /// answering for a query nobody can see — the same thing
    /// `ViewerPane::toggle_browse` already does to both of the other two boxes,
    /// and for the same reason.
    pub fn close_box(&mut self) {
        self.close();
    }

    pub fn title(&self) -> String {
        // Which of the three searches this is, said in words rather than by a
        // punctuation mark. `/` prefixes the query in all three; the file list
        // has a directory in front of it and the document has a file name, and
        // this one would otherwise be the only title in the pane that opened
        // with a bare `/` and left the reader to work out which box they were
        // in.
        let head = format!("all files · /{}", self.query);
        // The list under an open box is the *last* query's answer, so a count
        // here would be about a phrase that is no longer the one in the title.
        // Naming the key instead is honest and is also the one thing worth
        // saying to somebody looking at a box that has visibly done nothing.
        if self.typing && self.ran.as_deref() != Some(self.query.as_str()) {
            return format!("{head} · enter to search");
        }
        if self.ran.is_none() {
            return head;
        }
        let n = self.hits.len();
        // A count is a claim, and neither of these two states can support one.
        // Before the walk lands there is no list to sweep, so `0 matches` is a
        // definite answer about a question nothing has looked at; while a sweep
        // is out the count is true only of the files read so far. The body says
        // both of these in words already, and a title that contradicted the
        // screen under it would be the more believed of the two.
        if !self.indexed {
            return format!("{head} · waiting for the walk");
        }
        if self.running {
            return format!("{head} · {n} so far");
        }
        // `+` is `browse`'s convention for a list that is a prefix of the truth
        // — `2+ items` — and it stands for all four ways this list can be one:
        // the total cap, a file cut at [`MAX_PER_FILE`], a file `load` read only
        // the head of, and a walk that stopped short of the tree.
        let short = self.report.short() || self.partial;
        let more = if short { "+" } else { "" };
        // One of the four is named as well as marked, because it is the only one
        // with an answer of its own. For the other three the remedy is a longer
        // phrase or nothing at all — a truncated file's tail is unreachable
        // whatever anyone types — while the matches missing *here* are in files
        // this list did reach, and `Enter` on any row of one of them shows every
        // one of them. Named last, where a clipped title spares it first, for
        // the reason `browse::title` puts the count after the query: of the two
        // the `+` is the part that must survive, because it is the part that
        // says the list is not the whole answer.
        let cut = match self.report.cut {
            0 => String::new(),
            k => format!(" · {k} {} cut", plural(k, "file", "files")),
        };
        // `1+` is "one, and there are more", so it is plural whatever `n` is.
        let word = if short {
            "matches"
        } else {
            plural(n, "match", "matches")
        };
        format!("{head} · {n}{more} {word}{cut}")
    }

    pub fn render(&mut self, f: &mut Frame, inner: Rect) {
        let rows = self.hits.len();
        // The one moment the row count and the pane's height are both known,
        // which is why the cursor is told here and nowhere else.
        self.cursor.measure(rows, inner.height as usize);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // The scrollbar takes a column from the text rather than sitting on top
        // of it, as in every other list in this pane.
        let text_w = inner.width - scroll::bar_width(inner.width);
        let offset = self.cursor.scroll.offset;
        let lines: Vec<Line> = if rows == 0 {
            block(self.nothing(), text_w as usize, self.theme.theme().dim())
        } else {
            (offset..rows)
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
        self.cursor.scroll.render_bar(f, inner);
    }

    /// Take whatever the worker has said since the last frame. Never blocks:
    /// `Pane::tick` runs on the thread that pumps the agent's pty.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(batch) => changed |= self.absorb(batch),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // We hold the request sender, so the worker cannot have
                    // finished normally: it either never started or it panicked.
                    // Reported once — as a sweep that is no longer running —
                    // rather than spun on.
                    if self.worker {
                        self.worker = false;
                        self.running = false;
                        changed = true;
                    }
                    break;
                }
            }
        }
        changed
    }

    fn absorb(&mut self, batch: Batch) -> bool {
        // An answer to a query the reader has already replaced. Dropped rather
        // than appended: it is about a different phrase, and the count in the
        // title would be the sum of two questions.
        if batch.generation != self.generation {
            return false;
        }
        let grew = !batch.hits.is_empty();
        self.hits.extend(batch.hits);
        if let Some(report) = batch.done {
            self.report = report;
            self.running = false;
        }
        // Nothing is done to the cursor. A list that merely grew, with the
        // selection still on the row it was on, has nothing to bring back on
        // screen — and revealing once per arriving batch would pin the view to
        // the selection and take the wheel away. See [`super::list`].
        grew || !self.running
    }

    // --- keys -------------------------------------------------------------

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if self.typing {
            self.box_key(key)
        } else {
            self.list_key(key)
        }
    }

    /// While the box is open every printable key is text — `q`, `j` and `f`
    /// included. The same trade `browse::find_key` makes and the same table,
    /// because a reader who has learned one filter box in this pane has learned
    /// all of them: the arrows and `Ctrl+N`/`Ctrl+P` step the results behind
    /// the box, and the paging keys page them.
    fn box_key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.cursor.page() as isize;
        let half = self.cursor.half() as isize;

        match key.code {
            // Never mind. Only an `Esc` with nothing behind it leaves the
            // results entirely — being thrown out of the pane by the key that
            // means "never mind" is the single most annoying thing a box like
            // this can do.
            KeyCode::Esc => self.close(),
            // The one key that costs disk. See the module doc.
            KeyCode::Enter => self.run(),

            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            KeyCode::Char('d') if ctrl => self.step(half),
            KeyCode::Char('u') if ctrl => self.step(-half),
            // Ctrl+letter is the agent's everywhere else in the program, so the
            // rest must not fall into the plain-letter arm below.
            KeyCode::Char(_) if ctrl => Outcome::Ignored,
            KeyCode::Char(c) => {
                self.query.push(c);
                Outcome::Moved
            }
            KeyCode::Backspace => {
                if self.query.pop().is_none() {
                    // Backspacing past the start of the query leaves the box.
                    // Those keystrokes came from opening it, so undoing the
                    // last should undo the first — the rule both other boxes in
                    // this pane follow.
                    self.close()
                } else {
                    Outcome::Moved
                }
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

    /// With the box shut this is an ordinary list, so the whole of the moving
    /// vocabulary is [`Cursor::key`]'s and the F1 table stays one table. What
    /// is left here is what a *result* means by a key.
    fn list_key(&mut self, key: KeyEvent) -> Outcome {
        let rows = self.hits.len();
        if let Some(handled) = self.cursor.key(rows, key) {
            return handled.into();
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char(_) if ctrl => Outcome::Ignored,
            KeyCode::Enter => self.open_hit(),
            // Both, and neither is a spare. `/` opens the box in the file list
            // and in the document, so it opens this one; `f` is the key that
            // got the reader here, and a key that worked a moment ago and does
            // nothing now is worse than one that never worked.
            KeyCode::Char('/') | KeyCode::Char('f') => {
                self.open();
                Outcome::Moved
            }
            KeyCode::Esc => Outcome::Leave,
            _ => Outcome::Ignored,
        }
    }

    /// The glance keys, arriving as the bare key. They move the view and
    /// nothing else: reading the pane from the other side of the window must
    /// not re-aim the `Enter` that follows it. `browse::scroll_view`'s rule.
    pub fn scroll_view(&mut self, key: KeyEvent) -> Handled {
        self.cursor.scroll.key(key).unwrap_or(Handled::No)
    }

    pub fn mouse(&mut self, ev: &MouseEvent) -> Outcome {
        let rows = self.hits.len();
        self.cursor.mouse(rows, ev).unwrap_or(Handled::No).into()
    }

    /// A pasted phrase, which only the box has anywhere to put. Pasting an
    /// identifier out of the agent's transcript is one of the likelier ways a
    /// repository search gets started.
    pub fn paste(&mut self, text: &str) -> Outcome {
        if !self.typing {
            return Outcome::Ignored;
        }
        // One line, no control characters: the reading both other boxes in this
        // pane take of a multi-line paste, for the same reason.
        let line: String = text
            .chars()
            .take_while(|c| *c != '\n' && *c != '\r')
            .filter(|c| !c.is_control())
            .collect();
        if line.is_empty() {
            return Outcome::Ignored;
        }
        self.query.push_str(&line);
        Outcome::Moved
    }

    /// Shut the box, keeping whatever it was opened over.
    ///
    /// The half-typed query goes with it. What is on screen behind the box is
    /// the *last run* query's results, so leaving the new text in the title
    /// would caption one query's results with another's.
    fn close(&mut self) -> Outcome {
        self.typing = false;
        self.query = self.ran.clone().unwrap_or_default();
        if self.ran.is_none() {
            Outcome::Leave
        } else {
            Outcome::Moved
        }
    }

    /// `Enter` in the box.
    fn run(&mut self) -> Outcome {
        if self.query.is_empty() {
            // Not a search for everything. An empty box is one that has been
            // opened and not yet used, and reading every file in the repository
            // to report that they all match nothing is the most expensive way
            // there is to say nothing.
            return Outcome::Ignored;
        }
        self.typing = false;
        self.ran = Some(self.query.clone());
        self.hits.clear();
        self.report = Report::default();
        self.cursor.sel = 0;
        self.cursor.scroll.to(0);
        self.submit();
        Outcome::Moved
    }

    /// Send [`Grep::ran`] to the worker, abandoning whatever was in flight.
    fn submit(&mut self) {
        let Some(needle) = self.ran.clone() else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        // Before the send, so that by the time the worker could pick this up
        // every earlier sweep is already stale by construction — the same
        // ordering `GitPane::set_root` keeps, and here it is also what stops
        // the sweep already running.
        self.alive.store(self.generation, Ordering::Relaxed);
        self.running = true;
        let ask = Ask {
            generation: self.generation,
            root: self.root.clone(),
            files: Arc::clone(&self.files),
            needle,
        };
        if self.tx.send(ask).is_err() {
            self.worker = false;
            self.running = false;
        }
    }

    fn open_hit(&mut self) -> Outcome {
        let Some(hit) = self.hits.get(self.cursor.sel) else {
            // A click below the last row, or `Enter` on an empty list. Declined
            // rather than acted on, which is `browse::enter`'s answer to the
            // same question.
            return Outcome::Ignored;
        };
        Outcome::Open {
            // `join` on a `/`-separated relative path is right on both
            // platforms; Windows accepts either separator in a path it is
            // given.
            path: self.root.join(&hit.path),
            query: self.ran.clone().unwrap_or_default(),
            ordinal: hit.ordinal,
        }
    }

    fn step(&mut self, delta: isize) -> Outcome {
        let rows = self.hits.len();
        self.cursor.step(rows, delta).into()
    }

    fn select(&mut self, to: usize) -> Outcome {
        let rows = self.hits.len();
        self.cursor.select(rows, to).into()
    }

    // --- drawing ----------------------------------------------------------

    /// What an empty list says. Never nothing: a blank pane is indistinguishable
    /// from a broken one, and all four of these states are a keystroke away.
    fn nothing(&self) -> &'static str {
        // First, because it is the only one of these that is not a fact about
        // the repository. Without it a thread that never spawned, or one that
        // panicked, falls through to the arm below and the reader is told "no
        // file under this directory contains that" — a definite claim about the
        // tree, when the truth is that nothing looked at it. `git.rs` surfaces
        // its own dead worker for the same reason; a wrong answer delivered
        // confidently is worse than an error.
        if !self.worker {
            return "The search worker stopped. Nothing was read, so this is not an answer about \
                    what is under this directory.";
        }
        if !self.indexed {
            // Blaming the query for an index that does not exist yet sends the
            // reader off to fix a query that was never wrong. `browse::nothing`
            // drew this line first.
            return "Still walking the repository. The search will run as soon as the walk is done.";
        }
        if self.running {
            return "Reading the files. Matches appear as they are found.";
        }
        match self.ran {
            // The one screen that has to explain the design, because the design
            // is a key that appears to do nothing: a box that filtered as you
            // typed would read every file in the repository per keystroke.
            None => {
                "Type a phrase and press Enter. Every file under this directory is read, so this \
                 one waits for the whole phrase rather than running on each key."
            }
            Some(_) => {
                "No file under this directory contains that. Backspace to widen it, Esc to go back."
            }
        }
    }

    /// One result: where it is, then what it says.
    ///
    /// ```text
    ///  …es/viewer.rs:1042  let mark = if self.pen
    ///  grep.rs:88          for entry in walk.take
    /// ```
    ///
    /// Two columns rather than a sentence, and padded to a fixed boundary so
    /// they line up. `browse::hit_spans` argues that in a narrow pane the name
    /// is the half worth keeping because it is what the reader compares between
    /// rows; here the match is the other half of exactly that comparison, and a
    /// ragged left edge on the second column would make every row start with
    /// hunting for where it begins.
    ///
    /// The locator is elided from the *left* — `crate::text::elide_left`, which
    /// exists for this — so the directory is what goes and the file name and
    /// the line number are what stay. Losing `crates/abeam/src/panes/` costs a
    /// reader nothing they cannot get back by pressing `Enter`; losing
    /// `:1042` costs them the thing they came for.
    fn line(&self, i: usize, w: usize) -> Line<'static> {
        let hit = &self.hits[i];
        let t = self.theme.theme();
        let lw = locator_width(w);

        let loc = elide_left(&format!("{}:{}", hit.path, hit.line), lw);
        let pad = lw.saturating_sub(loc.width());
        let mut spans = vec![
            Span::raw(" "),
            Span::raw(loc),
            Span::raw(" ".repeat(pad + 1)),
        ];

        // Windowed again, and against the pane this time: the worker sized the
        // preview to fit in memory, which is a different question from fitting
        // on a row.
        let text: Vec<char> = hit.text.chars().collect();
        let (preview, at) = window(&text, hit.start, w.saturating_sub(lw + 2));
        let chars: Vec<char> = preview.chars().collect();
        let end = (at + hit.len).min(chars.len());
        let cut = |from: usize, to: usize| chars[from.min(to)..to].iter().collect::<String>();
        spans.push(Span::raw(cut(0, at.min(chars.len()))));
        // The same two colours the document search paints with, so a match
        // looks like a match wherever the reader met it.
        spans.push(Span::styled(cut(at, end), t.hit(i == self.cursor.sel)));
        spans.push(Span::raw(cut(end, chars.len())));

        // Every row is clipped here and nowhere else. A pane that overflows its
        // rect corrupts the frame rather than merely looking wrong.
        let mut spans = clip_line(Line::from(spans), w).spans;
        if i != self.cursor.sel {
            return Line::from(spans);
        }
        // Padded to the full width, or the highlight would stop at the end of
        // the text instead of marking the row. `browse::line`'s rule.
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        Line::from(spans).style(t.selection())
    }
}

/// How much of a row the locator may take before the preview starts.
///
/// Two fifths. The split is what makes the two columns comparable between rows,
/// and it is weighted towards the preview because the locator is one short
/// token that elides gracefully — `elide_left` always leaves the name and the
/// line — while the preview is prose that stops being a preview once there is
/// no room for anything either side of the match. At the 46 columns this pane
/// is routinely given — 45 once `scroll::bar_width` has taken its column — that
/// is 18 for `viewer.rs:1042` and 25 for the line.
///
/// The floor is where a name stops surviving at all: below six columns
/// `elide_left` is returning an ellipsis and two letters, and the row is better
/// spent entirely on what the file says.
fn locator_width(w: usize) -> usize {
    (w * 2 / 5).max(6)
}

/// A window of `hay` with the match at `at` inside it, and where the match
/// landed in it.
///
/// The match has to be *on* the row or the row is not a result, so this is a
/// window and not a prefix: a minified bundle puts its only match at column
/// 40,000, and a preview that started at the beginning of that line would show
/// the reader forty characters of somebody's compressed variable names.
///
/// Both cuts are marked with `…`, which is `crate::text`'s rule — truncation is
/// always marked — and the left one has to be, because without it a window
/// starting inside a line is indistinguishable from a line that starts there.
fn window(hay: &[char], at: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    if hay.len() <= width {
        return (hay.iter().collect(), at);
    }
    // Never more than half the window, however wide [`LEAD`] is: the match has
    // to be *inside* the window, and a fixed lead wider than the window itself
    // would put the window entirely in front of it. That is not hypothetical —
    // a 46-column pane leaves 25 for the preview and the row `line` draws when
    // the pane is dragged narrower leaves fewer.
    let lead = LEAD.min(width / 2);
    // As late as the match allows and as early as the end of the line allows;
    // the second `min` is what stops a match near the end from leaving the
    // window hanging off it with the last few characters unused.
    let start = at.saturating_sub(lead).min(hay.len() - width);
    let end = (start + width).min(hay.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&hay[start..end]);
    if end < hay.len() {
        out.push('…');
    }
    (out, at - start + usize::from(start > 0))
}

// ---------------------------------------------------------------------------
// the worker
// ---------------------------------------------------------------------------

fn spawn_worker(alive: Arc<AtomicU64>) -> (Sender<Ask>, Receiver<Batch>) {
    let (req_tx, req_rx) = mpsc::channel::<Ask>();
    let (res_tx, res_rx) = mpsc::channel::<Batch>();

    // A failed spawn drops `res_tx` with the closure, and the pane's
    // disconnected-channel path reports it. Nothing here can panic the UI
    // thread, and there is deliberately no join: the thread ends when the pane
    // drops its sender, which is also what a workspace switch does to it.
    let _ = std::thread::Builder::new()
        .name("abeam-grep".into())
        .spawn(move || {
            while let Ok(ask) = req_rx.recv() {
                // Superseded before it was ever picked up: two queries can be
                // queued while this thread is inside one sweep.
                if alive.load(Ordering::Relaxed) != ask.generation {
                    continue;
                }
                if !sweep(&ask, &alive, &mut |batch| res_tx.send(batch).is_ok()) {
                    break;
                }
            }
        });

    (req_tx, res_rx)
}

/// Read every file in `ask`, reporting matches in batches.
///
/// Returns whether the pane is still listening, so the worker can stop rather
/// than sweep a repository nobody will hear the answer to.
///
/// `load::load` per file, and nothing here re-implements any of what it does:
/// it sniffs a NUL in the first block and refuses the file, caps at
/// `load::MAX_BYTES`, normalises CRLF so a line is a line on both platforms,
/// and decodes lossily so a latin-1 README is still searchable. A grep with its
/// own idea of any of those would find matches the document view then could not
/// show — which is the one failure this feature cannot have, since `Enter` on a
/// result is a promise that the document view will show it.
///
/// A file `load` refuses is skipped in silence, and that is the right silence:
/// the reasons are a binary file, a file that has been deleted since the walk,
/// and one the process may not read. None of the three is news about the
/// reader's query, and a list of them would be a list of everything in `target`
/// that got past the gitignore.
fn sweep(ask: &Ask, alive: &AtomicU64, send: &mut impl FnMut(Batch) -> bool) -> bool {
    let needle: Vec<char> = ask.needle.chars().collect();
    let fold = search::folded(&needle);
    let mut hits: Vec<Hit> = Vec::new();
    let mut report = Report::default();
    let mut total = 0usize;
    // One buffer for the whole sweep. A `Vec<char>` per line of every file in
    // the repository is an allocation per line, and the lines are the inner
    // loop.
    let mut hay: Vec<char> = Vec::new();

    for rel in ask.files.iter() {
        // Re-read between files, so a query typed while this one is running
        // abandons it at the next file rather than at the last one. See the
        // module doc.
        if alive.load(Ordering::Relaxed) != ask.generation {
            return true;
        }
        if total >= MAX_HITS {
            report.full = true;
            break;
        }
        let Ok(loaded) = load::load(&ask.root.join(rel)) else {
            continue;
        };

        let mut here = 0usize;
        let mut cut = false;
        for (n, line) in loaded.text.lines().enumerate() {
            if cut {
                break;
            }
            if total >= MAX_HITS {
                // Said here as well as at the top of the file loop, because the
                // budget can run out inside the sweep's *last* file and then
                // there is no next file to notice on its behalf: the loop would
                // end normally, the report would come back clean, and a title
                // that had stopped thirty lines short would print a confident
                // count with no `+` on it.
                report.full = true;
                break;
            }
            hay.clear();
            // Tabs first, and it buys two things at once. A `\t` in a `Span` is
            // a row whose drawn width and measured width disagree — see
            // `source::expand_tabs`. And the document view expands them too, so
            // matching the expanded line is what keeps a hit found here findable
            // by `/` after `Enter`: a query with a run of spaces in it would
            // otherwise match one and not the other.
            //
            // The branch is the inner loop of the whole sweep, which is why the
            // common case does not go through `expand_tabs` at all: it returns
            // `line.to_string()` when there is nothing to expand, an allocation
            // per line of every file in the repository to copy a line unchanged.
            if line.contains('\t') {
                hay.extend(source::expand_tabs(line).chars());
            } else {
                hay.extend(line.chars());
            }
            for start in search::starts(&hay, &needle, 0, fold) {
                if here >= MAX_PER_FILE {
                    // One match past the cap is the whole of what has to be
                    // known: that the file has more. Counting how many more
                    // would mean reading the rest of a file whose extra matches
                    // are not going to be shown either way — and a file with
                    // *exactly* the cap in it has to come out uncut, or the
                    // title would report a file that was reported in full.
                    cut = true;
                    break;
                }
                if total >= MAX_HITS {
                    report.full = true;
                    break;
                }
                let (text, off) = window(&hay, start, PREVIEW);
                hits.push(Hit {
                    path: rel.clone(),
                    line: n + 1,
                    start: off,
                    len: needle.len(),
                    text,
                    ordinal: here,
                });
                here += 1;
                total += 1;
            }
        }
        // Counted once per file however many lines were left unread in it: the
        // title says how many *files* are short, not how many matches are.
        if cut {
            report.cut += 1;
        }
        // And only for a file the reader can actually see a row from. See
        // [`Report::clipped`]: on truncation alone this is a mark that is on for
        // every query the moment one generated file exists under the root.
        if loaded.truncated && here > 0 {
            report.clipped += 1;
        }

        // Flushed per file that found something, rather than per screenful of
        // hits. The empty list says "matches appear as they are found" and a
        // batch that waited for twenty of them makes that untrue in exactly the
        // case the reader is most in doubt: three hits scattered across twenty
        // thousand files would have arrived in one message at the very end,
        // after a wait with nothing on screen — which is the failure batching
        // exists to prevent. The message count is bounded by [`MAX_HITS`],
        // since a message is only ever sent when at least one hit is in it.
        if !hits.is_empty()
            && !send(Batch {
                generation: ask.generation,
                hits: std::mem::take(&mut hits),
                done: None,
            })
        {
            return false;
        }
    }

    send(Batch {
        generation: ask.generation,
        hits,
        done: Some(report),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // The real walk, because what this search can see is defined by what it
    // hands over. Imported here rather than beside the rest: nothing outside a
    // test needs it, now that whether the walk was truncated arrives as a flag
    // rather than being re-derived from its caps.
    use super::super::files;
    use crate::testutil::TempDir;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::Path;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A real tree with a real `.git` in it, so `ignore` recognises a
    /// repository and the gitignore assertions below cannot pass for the wrong
    /// reason. `browse.rs`'s fixture, for the same reason it has one.
    fn tree(tag: &str, files: &[(&str, &[u8])]) -> TempDir {
        let dir = TempDir::new(tag);
        std::fs::create_dir_all(dir.path().join(".git")).expect("create .git");
        for (rel, body) in files {
            let path = dir.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixture directory");
            }
            std::fs::write(&path, body).expect("write fixture");
        }
        dir
    }

    /// Sweep a real tree through the real walk, synchronously.
    ///
    /// The walk is the subject as much as the sweep is: what this search can
    /// see is defined by what `files::scan` hands it, so a hand-written list
    /// here would be asserting against a fixture rather than against the thing.
    fn sweep_tree(dir: &TempDir, needle: &str) -> (Vec<Hit>, Report) {
        let ask = Ask {
            generation: 1,
            root: dir.path().to_path_buf(),
            files: files::scan(dir.path()).files.into(),
            needle: needle.to_string(),
        };
        run(&ask)
    }

    fn run(ask: &Ask) -> (Vec<Hit>, Report) {
        let alive = AtomicU64::new(ask.generation);
        let mut hits = Vec::new();
        let mut report = Report::default();
        sweep(ask, &alive, &mut |batch| {
            hits.extend(batch.hits);
            if let Some(done) = batch.done {
                report = done;
            }
            true
        });
        (hits, report)
    }

    fn places(hits: &[Hit]) -> Vec<(String, usize)> {
        hits.iter().map(|h| (h.path.clone(), h.line)).collect()
    }

    // --- the sweep --------------------------------------------------------

    #[test]
    fn a_phrase_is_found_under_a_directory_nothing_ever_pointed_the_pane_at() {
        // The whole point: a file three directories down, which is neither
        // markdown nor anywhere near the directory being listed.
        let dir = tree(
            "grep-nested",
            &[
                ("plan.md", b"nothing here\n"),
                (
                    "src/panes/deep/keymap.rs",
                    b"fn one() {}\nlet needle = 1;\n",
                ),
            ],
        );
        let (hits, report) = sweep_tree(&dir, "needle");
        assert_eq!(places(&hits), [("src/panes/deep/keymap.rs".to_string(), 2)]);
        assert_eq!(hits[0].text, "let needle = 1;");
        assert_eq!(hits[0].start, 4, "characters into the line it is on");
        assert_eq!(hits[0].ordinal, 0);
        assert_eq!(report, Report::default(), "nothing was cut short");
    }

    #[test]
    fn what_the_walk_will_not_offer_the_sweep_never_reads() {
        // Inherited rather than re-derived, which is the reason this reads the
        // real index instead of a list of its own: a second gitignore walk
        // would double the disk cost of a search to answer a question the first
        // one already answered.
        let dir = tree(
            "grep-ignored",
            &[
                (".gitignore", b"*.key\n"),
                ("secret.key", b"the needle is in here\n"),
                ("kept.md", b"the needle is in here too\n"),
                ("target/debug/build.log", b"needle\n"),
                ("node_modules/pkg/index.js", b"needle\n"),
            ],
        );
        let (hits, _) = sweep_tree(&dir, "needle");
        assert_eq!(
            places(&hits),
            [("kept.md".to_string(), 1)],
            "the gitignore, or `in_noise`, was not inherited"
        );
    }

    #[test]
    fn a_binary_file_is_skipped_rather_than_rendered_into_the_list() {
        // `load` sniffs the NUL, exactly as it does for the document view, and
        // a grep with its own idea of binary would put a row of replacement
        // glyphs in a list a person reads.
        let dir = tree(
            "grep-binary",
            &[
                ("a.png", b"\x89PNG\x00needle\xff\xfe"),
                ("b.txt", b"needle\n"),
            ],
        );
        let (hits, _) = sweep_tree(&dir, "needle");
        assert_eq!(places(&hits), [("b.txt".to_string(), 1)]);
    }

    #[test]
    fn the_matcher_is_the_document_searchs_matcher() {
        // Smart case is the visible half of it, and the half a reader would
        // notice disagreeing: a phrase `f` finds and `/` then cannot find in
        // the same file is two ideas of what a match is, neither of them
        // written anywhere they can see.
        let dir = tree("grep-case", &[("a.txt", b"Plan for the plan\nPLAN\n")]);
        let (lower, _) = sweep_tree(&dir, "plan");
        assert_eq!(lower.len(), 3, "case-insensitive until told otherwise");
        let (upper, _) = sweep_tree(&dir, "Plan");
        assert_eq!(upper.len(), 1, "one capital is the reader meaning it");
        assert_eq!(upper[0].start, 0);

        // ...and matches do not overlap here either, or the ordinal `Enter`
        // hands over would count something the document search never will.
        let dir = tree("grep-overlap", &[("a.txt", b"aaaa\n")]);
        let (hits, _) = sweep_tree(&dir, "aa");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[1].ordinal, 1);
    }

    #[test]
    fn each_files_matches_are_numbered_from_that_files_own_start() {
        // What `Enter` hands to the document search. Numbering them from the
        // start of the *sweep* would open the second file's first match as its
        // fourth.
        let dir = tree(
            "grep-ordinal",
            &[("a.txt", b"x\nx\nx\n"), ("b.txt", b"x\n")],
        );
        let (hits, _) = sweep_tree(&dir, "x");
        let by_file: Vec<(String, usize)> =
            hits.iter().map(|h| (h.path.clone(), h.ordinal)).collect();
        assert_eq!(
            by_file,
            [
                ("a.txt".to_string(), 0),
                ("a.txt".to_string(), 1),
                ("a.txt".to_string(), 2),
                ("b.txt".to_string(), 0),
            ]
        );
    }

    #[test]
    fn a_file_with_more_matches_than_the_cap_is_cut_and_the_cut_is_counted() {
        let body: String = (0..MAX_PER_FILE + 8).map(|_| "needle\n").collect();
        let dir = tree(
            "grep-perfile",
            &[("big.txt", body.as_bytes()), ("small.txt", b"needle\n")],
        );
        let (hits, report) = sweep_tree(&dir, "needle");
        assert_eq!(hits.len(), MAX_PER_FILE + 1, "the other file is still read");
        assert_eq!(report.cut, 1);
        assert!(report.short(), "the count has to admit it is short");
        // ...and the file that fitted is not counted as cut.
        assert_eq!(hits.iter().filter(|h| h.path == "small.txt").count(), 1);
    }

    #[test]
    fn the_total_cap_reached_inside_the_last_file_is_still_admitted() {
        // The cap was only ever noticed by the *next* file, at the top of the
        // file loop — so a sweep that ran out of budget part way through its
        // last file ended normally, reported a clean sweep, and printed a
        // confident count with no `+` on it over lines it had never read. The
        // cap's own doc says that must not happen.
        let per = 10;
        let mut files: Vec<(String, Vec<u8>)> = (0..MAX_HITS / per - 1)
            .map(|i| (format!("f{i:04}.txt"), "needle\n".repeat(per).into_bytes()))
            .collect();
        // Sorts last, and holds far more than the budget left for it.
        files.push(("zlast.txt".to_string(), "needle\n".repeat(40).into_bytes()));
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let dir = tree("grep-total-last", &refs);

        let (hits, report) = sweep_tree(&dir, "needle");
        assert_eq!(hits.len(), MAX_HITS);
        assert!(
            report.full,
            "the sweep stopped thirty lines into its last file and said nothing"
        );
        assert!(report.short());
    }

    #[test]
    fn only_a_truncated_file_that_is_in_the_list_marks_the_count() {
        // Counted on truncation alone, one generated file over half a megabyte
        // anywhere under the root put a `+` on every query for ever — and a mark
        // that is always on says nothing at all.
        // One phrase at the top of an over-long file, and a different one in a
        // small file beside it.
        let mut big = b"haystack\n".to_vec();
        while (big.len() as u64) < load::MAX_BYTES + 8192 {
            big.extend_from_slice(b"padding padding padding\n");
        }
        let dir = tree(
            "grep-clipped",
            &[("huge.txt", big.as_slice()), ("a.txt", b"needle\n")],
        );

        let (hits, report) = sweep_tree(&dir, "needle");
        assert_eq!(hits.len(), 1, "only the small file matches");
        assert_eq!(
            report.clipped, 0,
            "a truncated file nobody is looking at marked the count"
        );
        assert!(!report.short(), "and so the count is a definite one");

        // ...and a truncated file the reader *can* see a row from does mark it,
        // because the rest of that file is a place this list cannot reach.
        let (hits, report) = sweep_tree(&dir, "haystack");
        assert_eq!(hits.len(), 1);
        assert_eq!(report.clipped, 1);
        assert!(report.short());
    }

    #[test]
    fn the_sweep_stops_at_the_total_and_says_that_it_did() {
        // One file per row rather than one enormous file, so this is the total
        // doing the stopping and not the per-file cap.
        let files: Vec<(String, Vec<u8>)> = (0..MAX_HITS / MAX_PER_FILE + 4)
            .map(|i| {
                (
                    format!("f{i:04}.txt"),
                    (0..MAX_PER_FILE)
                        .map(|_| "needle\n")
                        .collect::<String>()
                        .into_bytes(),
                )
            })
            .collect();
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let dir = tree("grep-total", &refs);

        let (hits, report) = sweep_tree(&dir, "needle");
        assert!(report.full, "the sweep ran out of files before the cap");
        assert!(hits.len() <= MAX_HITS, "{} hits", hits.len());
        assert!(hits.len() > MAX_HITS - MAX_PER_FILE);
        assert!(report.short());
    }

    #[test]
    fn a_query_the_reader_has_replaced_is_abandoned_rather_than_finished() {
        // The difference between this and letting a stale sweep run to the end:
        // the next query is queued behind however many thousand files are left
        // in this one. `git.rs` can afford to let a stale `git status` finish;
        // this cannot.
        let files: Vec<(String, Vec<u8>)> = (0..50)
            .map(|i| (format!("f{i:03}.txt"), b"needle\n".to_vec()))
            .collect();
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let dir = tree("grep-cancel", &refs);

        let ask = Ask {
            generation: 7,
            root: dir.path().to_path_buf(),
            files: files::scan(dir.path()).files.into(),
            needle: "needle".to_string(),
        };
        // As if the reader had typed another query while this one was running.
        let alive = AtomicU64::new(8);
        let mut batches = 0;
        sweep(&ask, &alive, &mut |_| {
            batches += 1;
            true
        });
        assert_eq!(batches, 0, "a superseded sweep reported anyway");

        // ...and one nobody replaced does finish.
        let (hits, _) = run(&ask);
        assert_eq!(hits.len(), 50);
    }

    #[test]
    fn a_sweep_nobody_is_listening_to_stops() {
        let files: Vec<(String, Vec<u8>)> = (0..40)
            .map(|i| (format!("f{i:03}.txt"), b"needle needle\n".to_vec()))
            .collect();
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let dir = tree("grep-deaf", &refs);
        let ask = Ask {
            generation: 1,
            root: dir.path().to_path_buf(),
            files: files::scan(dir.path()).files.into(),
            needle: "needle".to_string(),
        };
        let alive = AtomicU64::new(1);
        assert!(
            !sweep(&ask, &alive, &mut |_| false),
            "the sweep carried on talking to a dropped receiver"
        );
    }

    #[test]
    fn a_tab_reaches_the_row_as_the_spaces_the_document_view_would_have_drawn() {
        // A `\t` written into a terminal cell is not a character the cell can
        // hold, and `unicode_width` measures it as nothing — so a preview of a
        // Makefile line would be a row whose drawn width and measured width
        // disagree, in a pane that clips every row to its exact rect.
        let dir = tree("grep-tabs", &[("Makefile", b"build:\n\tcargo needle\n")]);
        let (hits, _) = sweep_tree(&dir, "needle");
        assert_eq!(hits.len(), 1);
        assert!(!hits[0].text.contains('\t'), "{:?}", hits[0].text);
        assert_eq!(hits[0].text, "    cargo needle");
        assert_eq!(
            hits[0]
                .text
                .chars()
                .skip(hits[0].start)
                .take(6)
                .collect::<String>(),
            "needle",
            "expanding the tabs moved the match and not the offset"
        );
    }

    #[test]
    fn a_match_far_along_a_line_is_still_on_the_row() {
        // A minified bundle puts its only match at column 40,000. A preview
        // that was a prefix of the line would show the reader none of it.
        let mut body = "x".repeat(400);
        body.push_str("needle");
        body.push('\n');
        let dir = tree("grep-window", &[("min.js", body.as_bytes())]);
        let (hits, _) = sweep_tree(&dir, "needle");
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        let text: Vec<char> = hit.text.chars().collect();
        assert!(text.len() <= PREVIEW + 2, "{} characters", text.len());
        assert_eq!(
            text[hit.start..hit.start + hit.len]
                .iter()
                .collect::<String>(),
            "needle",
            "the offset does not point at the match"
        );
        assert!(hit.text.starts_with('…'), "the cut is not marked");
    }

    #[test]
    fn a_window_keeps_some_of_what_comes_before_the_match() {
        let hay: Vec<char> = "0123456789abcdefghijklmnopqrstuvwxyz".chars().collect();
        // Whole line, untouched.
        assert_eq!(
            window(&hay, 4, 40),
            ("0123456789abcdefghijklmnopqrstuvwxyz".to_string(), 4)
        );

        // A match in the middle: LEAD characters of lead, and both cuts marked.
        let (text, at) = window(&hay, 20, 12);
        assert!(text.starts_with('…') && text.ends_with('…'), "{text}");
        assert_eq!(text.chars().nth(at), Some('k'), "{text} at {at}");

        // A match at the very end does not leave the window hanging off it.
        let (text, at) = window(&hay, 35, 10);
        assert_eq!(text.chars().nth(at), Some('z'), "{text} at {at}");
        assert!(!text.ends_with('…'), "{text}");
    }

    // --- the list ---------------------------------------------------------

    /// A grep with hits in it, as a sweep would have left them, and no worker
    /// racing the assertions.
    fn listed(root: &Path, query: &str, hits: Vec<Hit>) -> Grep {
        let mut g = Grep::new(root.to_path_buf());
        g.set_index(Arc::from(Vec::new()), false);
        g.ran = Some(query.to_string());
        g.query = query.to_string();
        g.hits = hits;
        g.running = false;
        g
    }

    fn hit(path: &str, line: usize, text: &str, start: usize, len: usize, ordinal: usize) -> Hit {
        Hit {
            path: path.to_string(),
            line,
            start,
            len,
            text: text.to_string(),
            ordinal,
        }
    }

    fn drawn(g: &mut Grep, w: u16, h: u16) -> Vec<String> {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("a test terminal");
        term.draw(|f| g.render(f, Rect::new(0, 0, w, h)))
            .expect("draw the results");
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_row_keeps_the_file_name_the_line_and_the_match() {
        // At the 46 columns this pane is routinely given, with a path far too
        // long for it. The directory is what goes; the name, the line and the
        // matched word are what the reader is comparing between rows.
        let dir = TempDir::new("grep-row");
        let mut g = listed(
            dir.path(),
            "needle",
            vec![hit(
                "crates/abeam/src/panes/viewer.rs",
                1042,
                "let needle = 1;",
                4,
                6,
                0,
            )],
        );
        let rows = drawn(&mut g, 46, 4);
        assert!(rows[0].contains("viewer.rs:1042"), "{:?}", rows[0]);
        assert!(
            rows[0].starts_with(" …"),
            "the path was not elided: {:?}",
            rows[0]
        );
        assert!(rows[0].contains("needle"), "{:?}", rows[0]);
    }

    #[test]
    fn an_empty_list_says_which_of_the_four_reasons_it_is_empty() {
        let dir = TempDir::new("grep-empty");
        let mut g = Grep::new(dir.path().to_path_buf());
        // The walk has not answered. Blaming the query for an index that does
        // not exist sends the reader off to fix a query that was never wrong.
        g.open();
        assert!(g.nothing().contains("Still walking"), "{}", g.nothing());

        g.set_index(Arc::from(Vec::new()), false);
        assert!(
            g.nothing().contains("press Enter"),
            "an open box with nothing run has to explain why it is idle: {}",
            g.nothing()
        );

        g.running = true;
        g.ran = Some("x".into());
        assert!(g.nothing().contains("Reading the files"), "{}", g.nothing());

        g.running = false;
        assert!(g.nothing().contains("No file"), "{}", g.nothing());
    }

    #[test]
    fn the_title_admits_every_cap_that_stopped_it() {
        let dir = TempDir::new("grep-title");
        let mut g = listed(
            dir.path(),
            "needle",
            vec![hit("a.txt", 1, "needle", 0, 6, 0)],
        );
        assert_eq!(g.title(), "all files · /needle · 1 match");

        g.report = Report {
            full: true,
            ..Report::default()
        };
        assert_eq!(
            g.title(),
            "all files · /needle · 1+ matches",
            "a list that stopped at the cap claimed to be the whole answer"
        );

        g.report = Report {
            cut: 3,
            ..Report::default()
        };
        assert_eq!(
            g.title(),
            "all files · /needle · 1+ matches · 3 files cut",
            "the one cap whose answer is `Enter` rather than a longer phrase"
        );

        // A file `load` would only read the head of is the same admission.
        g.report = Report {
            clipped: 1,
            ..Report::default()
        };
        assert_eq!(g.title(), "all files · /needle · 1+ matches");

        // ...and so is a walk that stopped short of the tree. Handed over rather
        // than inferred from the list's length: `files::MAX_ENTRIES` counts
        // entries visited, so a truncated walk can produce a short list that
        // looks complete from here.
        g.report = Report::default();
        g.partial = true;
        assert_eq!(g.title(), "all files · /needle · 1+ matches");

        // While the box is open the count would be about the query that is no
        // longer in the title, so it is not offered at all.
        g.report = Report::default();
        g.partial = false;
        g.open();
        assert_eq!(g.title(), "all files · / · enter to search");
    }

    #[test]
    fn a_count_is_not_claimed_before_there_is_anything_to_count() {
        // The body already says "still walking" and "reading the files"; a
        // title saying `0 matches` beside either is the more believed of the
        // two, and it is a definite claim about a repository nothing has
        // finished looking at.
        let dir = TempDir::new("grep-title-early");
        let mut g = Grep::new(dir.path().to_path_buf());
        g.open();
        g.query = "needle".into();
        g.run();
        assert_eq!(
            g.title(),
            "all files · /needle · waiting for the walk",
            "a count over an index that does not exist yet"
        );

        g.set_index(Arc::from(vec!["a.txt".to_string()]), false);
        assert_eq!(
            g.title(),
            "all files · /needle · 0 so far",
            "a count over a sweep that has not finished"
        );

        g.absorb(Batch {
            generation: g.generation,
            hits: vec![hit("a.txt", 1, "needle", 0, 6, 0)],
            done: Some(Report::default()),
        });
        assert_eq!(g.title(), "all files · /needle · 1 match");
    }

    #[test]
    fn a_worker_that_is_gone_is_not_an_answer_about_the_repository() {
        // "No file under this directory contains that" is a definite claim, and
        // a thread that never spawned has not read a single byte of the tree.
        let dir = TempDir::new("grep-title-dead");
        let mut g = listed(dir.path(), "needle", Vec::new());
        assert!(g.nothing().contains("No file"), "{}", g.nothing());
        g.worker = false;
        assert!(
            g.nothing().contains("worker stopped"),
            "a dead worker answered for the repository: {}",
            g.nothing()
        );
    }

    #[test]
    fn an_answer_to_a_query_the_reader_has_replaced_is_dropped() {
        // The failure this guards is not one wrong frame. The stale batches of
        // a whole-repository sweep would be *appended* to the new query's, so
        // the list would be two questions' answers under one count.
        let dir = TempDir::new("grep-stale");
        let mut g = Grep::new(dir.path().to_path_buf());
        g.set_index(Arc::from(vec!["a.txt".to_string()]), false);
        g.query = "one".into();
        g.run();
        let first = g.generation;
        g.open();
        g.query = "two".into();
        g.run();
        assert_ne!(g.generation, first);

        assert!(
            !g.absorb(Batch {
                generation: first,
                hits: vec![hit("a.txt", 1, "one", 0, 3, 0)],
                done: Some(Report::default()),
            }),
            "a superseded answer cost the agent a frame"
        );
        assert!(g.hits.is_empty(), "and landed in the new query's list");
        assert!(g.running, "and settled a query it was never about");

        // ...and the answer that does belong to the query in the box is taken.
        assert!(g.absorb(Batch {
            generation: g.generation,
            hits: vec![hit("a.txt", 2, "two", 0, 3, 0)],
            done: Some(Report::default()),
        }));
        assert_eq!(g.hits.len(), 1);
        assert!(!g.running);
    }

    #[test]
    fn every_printable_key_is_a_letter_while_the_box_is_open() {
        let dir = TempDir::new("grep-keys");
        let mut g = Grep::new(dir.path().to_path_buf());
        g.set_index(Arc::from(vec!["a.txt".to_string()]), false);
        g.open();
        for c in ['q', 'j', 'f', '/', 'r', 'G'] {
            assert!(
                matches!(g.key(key(KeyCode::Char(c))), Outcome::Moved),
                "{c} was not claimed by the box"
            );
        }
        assert_eq!(g.query, "qjf/rG");
        assert!(g.typing());

        // Enter runs it and shuts the box, and then the same letters move a
        // selection again.
        g.key(key(KeyCode::Enter));
        assert!(!g.typing());
        assert_eq!(g.ran.as_deref(), Some("qjf/rG"));
        // A query nobody typed is not a search.
        g.open();
        assert!(matches!(g.key(key(KeyCode::Enter)), Outcome::Ignored));
        assert!(g.typing(), "and the box is still open to type into");
    }

    #[test]
    fn esc_leaves_the_box_before_it_leaves_the_results() {
        let dir = TempDir::new("grep-esc");
        let mut g = Grep::new(dir.path().to_path_buf());
        g.set_index(Arc::from(Vec::new()), false);
        // Nothing behind the box yet: one press is the way out.
        g.open();
        assert!(matches!(g.key(key(KeyCode::Esc)), Outcome::Leave));

        // With results behind it, the same press shows them.
        g.ran = Some("needle".into());
        g.hits = vec![hit("a.txt", 1, "needle", 0, 6, 0)];
        g.open();
        g.query = "half typed".into();
        assert!(matches!(g.key(key(KeyCode::Esc)), Outcome::Moved));
        assert!(!g.typing());
        assert_eq!(
            g.query, "needle",
            "the abandoned text was left captioning the last query's results"
        );
        assert!(matches!(g.key(key(KeyCode::Esc)), Outcome::Leave));

        // Backspacing past the start of the query is the same door.
        g.open();
        assert!(matches!(g.key(key(KeyCode::Backspace)), Outcome::Moved));
        assert!(!g.typing());
    }

    #[test]
    fn enter_on_a_result_asks_for_the_file_and_the_place_in_it() {
        let dir = TempDir::new("grep-open");
        let mut g = listed(
            dir.path(),
            "needle",
            vec![
                hit("a.txt", 1, "needle", 0, 6, 0),
                hit("src/b.rs", 9, "needle", 0, 6, 2),
            ],
        );
        g.cursor.select(2, 1);
        match g.key(key(KeyCode::Enter)) {
            Outcome::Open {
                path,
                query,
                ordinal,
            } => {
                assert!(path.ends_with("b.rs"), "{path:?}");
                assert_eq!(query, "needle");
                assert_eq!(ordinal, 2, "the third match *of that file*");
            }
            _ => panic!("Enter on a result must ask for it to be opened"),
        }

        // Enter on an empty list is declined rather than acted on: a frame here
        // re-renders the agent's whole screen.
        let mut none = listed(dir.path(), "needle", Vec::new());
        assert!(matches!(none.key(key(KeyCode::Enter)), Outcome::Ignored));
    }

    #[test]
    fn a_query_asked_before_the_walk_answered_is_run_when_it_does() {
        // Otherwise the reader is left looking at "no file contains that",
        // which was never about their query — the same lie `browse::nothing`
        // refuses to tell.
        let dir = tree("grep-early", &[("a.txt", b"needle\n")]);
        let mut g = Grep::new(dir.path().to_path_buf());
        g.open();
        g.query = "needle".into();
        g.run();
        assert!(g.hits.is_empty());

        g.set_index(files::scan(dir.path()).files.into(), false);
        // The worker is real here, so wait for it rather than assuming a
        // schedule.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while g.hits.is_empty() && std::time::Instant::now() < deadline {
            g.tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(places(&g.hits), [("a.txt".to_string(), 1)]);
    }

    #[test]
    fn dropping_the_view_stops_the_sweep_it_left_running() {
        // `ViewerPane::set_root` replaces this whole struct, and the sweep in
        // flight is over a tree nobody can see any more. Dropping the receiver
        // alone would only stop it at its next `send` — and a query matching
        // nothing does not send until the last file.
        let dir = TempDir::new("grep-drop");
        let mut g = Grep::new(dir.path().to_path_buf());
        g.set_index(Arc::from(vec!["a.txt".to_string()]), false);
        g.query = "x".into();
        g.run();
        let alive = Arc::clone(&g.alive);
        let was = g.generation;
        assert_eq!(alive.load(Ordering::Relaxed), was);

        drop(g);
        assert_ne!(
            alive.load(Ordering::Relaxed),
            was,
            "the sweep was left reading a worktree nobody is looking at"
        );
    }

    #[test]
    fn a_worker_that_never_started_is_reported_once_rather_than_spun_on() {
        let dir = TempDir::new("grep-dead");
        let mut g = Grep::new(dir.path().to_path_buf());
        g.set_index(Arc::from(vec!["a.txt".to_string()]), false);
        g.query = "x".into();
        g.run();
        // As if the thread had gone: the pane still holds the request sender,
        // so this is the only way the channel can end.
        let (tx, rx) = mpsc::channel::<Batch>();
        drop(tx);
        g.rx = rx;
        assert!(g.tick(), "the pane was never told the sweep stopped");
        assert!(!g.running);
        assert!(!g.tick(), "and it is told once, not every tick");
    }
}
