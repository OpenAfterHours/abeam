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
//! ## Where the work happens
//!
//! Everything slow is either on a worker thread or off the frame path:
//!
//! - the gitignore-aware walk for the file list runs on its own thread and
//!   reports through a channel `tick` polls,
//! - the watcher is the shell's, on `notify`'s thread behind a debouncer,
//! - layout — parse, highlight, wrap — happens once per `(file, width)` pair
//!   and is cached, because `render` runs on every keystroke Claude receives.
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

mod files;
mod load;
mod markdown;
mod source;

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::pane::{Handled, Pane};
use crate::scroll::{self, Scroll};
use crate::text::{self, dim, wrap};
use load::{LoadError, Loaded};

/// Assumed page size before the first frame has told us the real one.
const DEFAULT_VIEWPORT: usize = 20;

/// Line numbers cost four or five columns. Worth it in a normal right pane,
/// not worth it in a squeezed one.
const LINE_NUMBER_MIN_WIDTH: usize = 30;

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
    Failed { path: PathBuf, why: LoadError },
}

pub struct ViewerPane {
    root: PathBuf,
    state: State,

    /// The document laid out for `laid_out` columns. Rebuilt when either the
    /// document or the width changes, and never otherwise.
    lines: Vec<Line<'static>>,
    laid_out: usize,
    dirty: bool,

    scroll: Scroll,

    /// A file the watcher noticed, waiting for the pane to be on screen.
    pending: Option<PathBuf>,
    /// Markdown under the root, newest first. `Tab` walks it.
    recent: Vec<PathBuf>,
    recent_ix: usize,

    scan: Option<Receiver<Vec<PathBuf>>>,
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
            root,
            state: State::Empty,
            lines: Vec::new(),
            laid_out: 0,
            dirty: true,
            scroll,
            pending: None,
            recent: Vec::new(),
            recent_ix: 0,
            scan,
            watching: false,
        }
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
    /// border of a pane that is not currently drawing — which is the only
    /// situation in which anything is ever waiting.
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
        // Re-showing the file already on screen is a reload, and a reload must
        // not throw away the reader's place. Claude rewriting a document
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
        match &self.state {
            State::Empty => empty_hint(width, self.watching),
            State::Failed { path, why } => {
                let mut lines = vec![
                    Line::from(Span::styled(
                        self.label(path),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::default(),
                ];
                lines.extend(text::block(&why.message(), width, dim()));
                lines.push(Line::default());
                // Naming Alt+G is a small layering leak — the globals are the
                // shell's — and it earns it. This screen is where someone
                // arrives without having asked to, and Tab walks the markdown
                // list, which is not where they came from.
                lines.extend(text::block(
                    "r to retry · Tab for the next markdown file · Alt+G for git",
                    width,
                    dim(),
                ));
                lines
            }
            State::Doc(doc) => {
                let mut lines = match &doc.body {
                    Body::Markdown(text) => markdown::render(text, width),
                    Body::Source(text) => source_lines(text, &doc.path, width),
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
                        dim(),
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
            // spent a whole frame — Claude's screen included — on a key that
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
        // Deliberately says nothing about a pending file. By the time this pane
        // renders its own title it has already taken one up, so a mark here
        // could never be seen; the shell asks `has_pending` and marks the
        // border of the view that is *not* showing.
        match &self.state {
            State::Empty => "files".to_string(),
            State::Failed { path, .. } => format!("{} · unreadable", self.label(path)),
            State::Doc(doc) => {
                let trunc = if doc.truncated { " · truncated" } else { "" };
                format!(
                    "{}{trunc} · {}",
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

        // Being drawn *is* the signal that this pane is the one on screen, and
        // it is the only such signal a pane gets. Auto-follow happens here for
        // exactly that reason.
        if let Some(path) = self.pending.take() {
            self.show(path);
        }

        // The column is reserved whether or not the bar is drawn: deciding per
        // frame would re-wrap the whole document every time a scrollbar
        // appeared, and the text would jump sideways as you scrolled.
        let text_width = inner.width - scroll::bar_width(inner.width);

        self.ensure_layout(text_width as usize);
        self.scroll
            .measure(self.lines.len(), inner.height as usize);

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
        let mut changed = false;

        // The walk answers once, then the receiver is dropped.
        if let Some(found) = self.scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.scan = None;
            self.recent = found;
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
        // Deliberately the same vocabulary as Claude's own transcript view, and
        // as the other two panes, so the app has one way to scroll rather than
        // three that drift.
        if let Some(handled) = self.scroll.key(key) {
            return Ok(handled);
        }

        let handled = match key.code {
            KeyCode::Tab => self.step(true),
            KeyCode::BackTab => self.step(false),

            KeyCode::Char('r') | KeyCode::Enter => {
                self.reload();
                // The only way a file created since startup joins the list if
                // the watcher could not start — but at most one walk at a time.
                // Key auto-repeat on `r` would otherwise start one gitignore
                // walk of the repository per repeat tick, thirty a second, and
                // throw away every answer but the last.
                if self.scan.is_none() {
                    self.scan = Some(files::spawn_scan(self.root.clone()));
                }
                Handled::Yes
            }

            // Esc and q are not ours. The shell reads an unhandled one as
            // "give focus back to Claude", which is the way out of here.
            _ => Handled::No,
        };
        Ok(handled)
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        Ok(self.scroll.mouse(ev).unwrap_or(Handled::No))
    }
}

/// Source files get a line-number gutter, because "look at line 42" is how
/// anyone talks about code. A wrapped continuation gets a blank number, which
/// is the only thing distinguishing it from the next line.
fn source_lines(text: &str, path: &Path, width: usize) -> Vec<Line<'static>> {
    let rows = source::highlight_file(text, path);
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
                vec![Span::styled(format!("{:>digits$} ", i + 1), dim())],
                vec![Span::raw(format!("{:>digits$} ", ""))],
            )
        } else {
            (Vec::new(), Vec::new())
        };
        out.extend(wrap::hard_wrap(row, width, &first, &cont));
    }
    out
}

fn empty_hint(width: usize, watching: bool) -> Vec<Line<'static>> {
    let hint = |s: &str| text::block(s, width, dim());
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
    lines.extend(hint("Tab  next file"));
    lines.extend(hint("r    look again"));
    lines.extend(hint("j k  scroll"));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crossterm::event::{KeyEventKind, KeyModifiers, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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

        assert_eq!(pane.handle_key(key(KeyCode::Char('k'))).unwrap(), Handled::No);
        assert_eq!(pane.scroll.offset, 0, "already at the top");

        pane.handle_key(key(KeyCode::Char('j'))).unwrap();
        assert_eq!(pane.scroll.offset, 1);
        pane.handle_key(key(KeyCode::Char(' '))).unwrap();
        assert_eq!(pane.scroll.offset, 1 + 9, "a page keeps one line of overlap");
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
            term.draw(|f| pane.render(f, Rect::new(0, 0, w, h))).unwrap();
        }
        // ...and after all that the pane is still usable.
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10))).unwrap();
        assert!(!pane.lines.is_empty());
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
        term.draw(|f| pane.render(f, Rect::new(0, 0, 40, 10))).unwrap();
        assert_eq!(pane.path(), Some(path.as_path()));
        assert!(pane.pending.is_none());
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
        let (tx, rx) = std::sync::mpsc::channel::<Vec<PathBuf>>();
        pane.scan = Some(rx);
        for _ in 0..5 {
            pane.handle_key(key(KeyCode::Char('r'))).unwrap();
        }

        // Still the same receiver — if `r` had replaced it, this answer would
        // go nowhere and `tick` would never see it.
        tx.send(vec![dir.write("late.md", b"# late\n")]).unwrap();
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
        // yet. Reporting `Yes` here spent a frame — Claude's whole screen
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
