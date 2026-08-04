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
//! ## Two things to look at, one pane
//!
//! [`Mode`] is the document or the list. `Alt+E` — [`ViewerPane::toggle_browse`]
//! — moves between them in both directions, and `Enter` on a file moves one
//! way, because a list that stayed up after you chose something would need a
//! second key to show you what you chose. Nothing else changes the mode; in
//! particular the watcher cannot, which is the rule above. They are the same
//! reading position from either end: the list is how a file is reached, the
//! document is what the list is for. The list itself lives in [`browse`],
//! because a selectable, filterable directory tree is a pane's worth of code on
//! its own and this file is long enough. What being *in* a list means — which
//! row is chosen, and keeping it on screen — is [`list`], one level down again:
//! the directory listing and the find over the repository are two lists in that
//! one pane, and the bookkeeping they share is the part that goes subtly wrong
//! when it is written twice.
//!
//! ## Where the work happens
//!
//! Everything slow is on a worker thread, cached, or capped:
//!
//! - the gitignore-aware walk that builds the recency list and the find index
//!   runs on its own thread and reports through a channel `tick` polls,
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

mod browse;
mod files;
mod list;
mod load;
mod markdown;
mod source;
mod theme;

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
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
use load::{LoadError, Loaded};

/// Assumed page size before the first frame has told us the real one.
const DEFAULT_VIEWPORT: usize = 20;

/// Line numbers cost four or five columns. Worth it in a normal right pane,
/// not worth it in a squeezed one.
const LINE_NUMBER_MIN_WIDTH: usize = 30;

/// Which of the two things the pane is showing. See the module doc.
enum Mode {
    Doc,
    Browse,
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
    dirty: bool,

    scroll: Scroll,

    /// A file the watcher noticed, waiting for the pane to be on screen.
    pending: Option<PathBuf>,
    /// A document was taken up inside [`Pane::render`], so the title the shell
    /// drew for this frame is a document behind the body under it.
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
            root,
            state: State::Empty,
            mode: Mode::Doc,
            raw: false,
            theme: theme::Mode::default(),
            lines: Vec::new(),
            laid_out: 0,
            dirty: true,
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
        self.root = root.clone();
        self.state = State::Empty;
        self.pending = None;
        // Otherwise `Tab` walks straight out of the workspace, into documents
        // of the tree that is no longer on screen.
        self.recent.clear();
        self.recent_ix = 0;
        self.scroll.to(0);
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
        self.mode = match self.mode {
            Mode::Doc => {
                // Open the list beside the document on screen. Whether that
                // means moving is [`Browser::align_to`]'s judgement, not ours:
                // it is the difference between a list that is a place and a
                // list that resets itself every time it is looked at.
                let doc = self.path().map(Path::to_path_buf);
                self.browse.align_to(doc.as_deref());
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
        };
    }

    /// Is a find box open? The border and the paste route both ask, and both
    /// are asking about this instant rather than about the pane's type.
    fn finding(&self) -> bool {
        matches!(self.mode, Mode::Browse) && self.browse.finding()
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

    // --- layout ----------------------------------------------------------

    fn ensure_layout(&mut self, width: usize) {
        if !self.dirty && width == self.laid_out {
            return;
        }
        self.lines = self.build(width);
        self.laid_out = width;
        self.dirty = false;
    }

    fn build(&self, width: usize) -> Vec<Line<'static>> {
        if width == 0 {
            return Vec::new();
        }
        let t = self.theme.theme();
        match &self.state {
            State::Empty => empty_hint(width, self.watching, t),
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
                lines
            }
            State::Doc(doc) => {
                // Raw markdown goes through the same path as any other source
                // file, gutter and all: syntect has a Markdown grammar, and a
                // reader looking at the source of a document wants to see the
                // line numbers they are about to talk about.
                let mut lines = match &doc.body {
                    Body::Markdown(text) if !self.raw => markdown::render(text, width, self.theme),
                    Body::Markdown(text) | Body::Source(text) => {
                        source_lines(text, &doc.path, width, self.theme)
                    }
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
                lines
            }
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
        // In the document view it deliberately says nothing about a pending
        // file: by the time this pane renders its own title it has already
        // taken one up, so a mark here could never be seen.
        match &self.state {
            State::Empty => "files".to_string(),
            State::Failed { path, .. } => format!("{} · unreadable", self.label(path)),
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
                format!(
                    "{}{trunc}{form} · {}",
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

        // Being drawn *is* the signal that this pane is the one on screen, and
        // it is the only such signal a pane gets. Auto-follow happens here for
        // exactly that reason.
        if let Some(path) = self.pending.take() {
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

        let start = self.scroll.offset;
        let end = (start + inner.height as usize).min(self.lines.len());
        let visible = self.lines[start.min(end)..end].to_vec();
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

        // The walk answers once, then the receiver is dropped.
        if let Some(found) = self.scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.scan = None;
            self.browse.set_index(found.files);
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
        // The list owns every key while it is up, including the scroll ones:
        // there they move a selection rather than an offset, and a pane cannot
        // hand the same key to two vocabularies and hope.
        if matches!(self.mode, Mode::Browse) {
            let out = self.browse.key(key);
            return Ok(self.absorb(out));
        }

        // Deliberately the same vocabulary as Claude's own transcript view, and
        // as the other two panes, so the app has one way to scroll rather than
        // three that drift.
        if let Some(handled) = self.scroll.key(key) {
            return Ok(handled);
        }

        let handled = match key.code {
            KeyCode::Tab => self.step(true),
            KeyCode::BackTab => self.step(false),
            KeyCode::Char('t') => self.toggle_raw(),

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
        if matches!(self.mode, Mode::Browse) {
            let out = self.browse.mouse(ev);
            return Ok(self.absorb(out));
        }
        Ok(self.scroll.mouse(ev).unwrap_or(Handled::No))
    }

    /// `Alt+J`/`Alt+K`/`Alt+PgDn`/`Alt+PgUp`, arriving as the bare key.
    ///
    /// In the document view that is the same key this pane would have handled
    /// anyway. In the list it is not: there `Down` moves the *selection*, and a
    /// glance is a read — the whole point of the binding is that it costs no
    /// focus round trip, so it must not quietly re-aim the `Enter` the reader
    /// presses when they get here. It moves the view alone.
    fn scroll_key(&mut self, key: KeyEvent) -> Result<Handled> {
        if matches!(self.mode, Mode::Browse) {
            return Ok(self.browse.scroll_view(key));
        }
        self.handle_key(key)
    }

    /// Only while a query is being typed. Everything else this pane does is a
    /// read; in the find box `j` is a letter, and both things the shell asks
    /// this for — where a paste goes, and whether leaving hands focus back to
    /// the agent — turn on exactly that.
    fn takes_input(&self) -> bool {
        self.finding()
    }

    /// `Esc` closes the find and leaves you in the list, which is one press
    /// short of the agent. A border promising `esc→agent` there would be naming
    /// a key that does something else.
    fn exit_hint(&self) -> &'static str {
        if self.finding() {
            " · esc→list"
        } else {
            " · esc→agent"
        }
    }

    /// A pasted path goes into the find box and nowhere else. It is the one
    /// place in a read-only pane with somewhere to put text, and pasting a path
    /// out of the agent's transcript is a likely way to reach a given file.
    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        if !matches!(self.mode, Mode::Browse) {
            return Ok(Handled::No);
        }
        let out = self.browse.paste(text);
        Ok(self.absorb(out))
    }
}

/// Source files get a line-number gutter, because "look at line 42" is how
/// anyone talks about code. A wrapped continuation gets a blank number, which
/// is the only thing distinguishing it from the next line.
fn source_lines(text: &str, path: &Path, width: usize, mode: theme::Mode) -> Vec<Line<'static>> {
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
    out
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
        pane.browse.set_index(vec!["docs/design.md".into()]);
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
        pane.browse.set_index(vec!["src/main.rs".into()]);
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
