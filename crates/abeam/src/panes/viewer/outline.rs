//! The shape of the document on screen, as a list you can jump into.
//!
//! Two things live here, and they are two halves of one idea. [`Entry`] is a
//! place in the **laid-out rows** the pane is already scrolling — a markdown
//! heading, or a definition in a source file — and [`View`] is the list of
//! those places that `o` puts on screen.
//!
//! There is a second `outline` module in this pane and it is unrelated:
//! `mermaid::flow::outline` is the indented text a *flowchart* falls back to
//! when the pane is too narrow to draw one. The two never meet — nothing here
//! knows about diagrams and nothing there knows about rows — but the names
//! collide in a grep, so this is the paragraph that says which is which.
//!
//! ## `Entry::row` is a row of `lines`, and it goes stale exactly as a hit does
//!
//! An entry's `row` indexes [`ViewerPane::lines`](super::ViewerPane), which is
//! the same contract [`search::Hit::row`](super::search::Hit::row) has and it
//! carries the same hazard: **it is invalid the moment `lines` is rebuilt**. A
//! width change re-wraps every paragraph, `t` swaps the document for a form
//! that shares no rows with it at all, and `F3` re-styles the lot; an entry
//! left over from the previous layout points at whatever happens to be at that
//! index now, so `Enter` lands somewhere the reader did not choose and the
//! breadcrumb names a section they are not in — both of them silently.
//!
//! The answer is the one the search already uses, and it is not a rule anybody
//! has to remember: the outline is rebuilt in `ViewerPane::ensure_layout`,
//! which is the single place `lines` is rebuilt, out of the same call that
//! produces them. Nothing else may build one. That is why [`symbols`] below
//! reports *source lines* rather than rows — it knows nothing about layout, and
//! the mapping from a source line to the row it landed on is `source_lines`'s,
//! made in the same pass that emits the row.
//!
//! ## Two ways in, because there are two kinds of document
//!
//! A rendered markdown file gets its entries from the renderer itself —
//! `markdown::render_outlined` records a heading at the moment it emits one, so
//! the row is the row, not a guess made afterwards by looking for bold text. A
//! source file gets them from [`symbols`], a line scanner in the same
//! discipline as [`docs`](super::docs): one forward pass, no regex, no
//! backtracking.
//!
//! Everything else — a `.txt`, a `.json`, and **markdown shown as its source**
//! — has no outline at all, and `o` declines rather than opening an empty list.
//! Markdown-as-source is the one of those worth arguing rather than stating.
//! Its headings are plainly there, as `#` lines a scanner would find in twenty
//! lines of code, and it is deliberately not done: `pulldown_cmark` is what
//! decides what a heading is in this pane, and a line scanner is not. Where the
//! two disagree — a `#` inside a fence, a setext heading under a row of `=`, a
//! `#` in an indented code block — one document would list two different tables
//! of its own contents depending on which key was last pressed, with nothing on
//! screen to say which was right. `t` is one keystroke back to the form that
//! has an outline, and that is a cheaper answer than a second authority on what
//! a heading is.
//!
//! ## What a wrong entry costs, which is why the scanner may be cheap
//!
//! [`docs`](super::docs) buys its exactness because a false positive there
//! hands a run of somebody's *code* to the markdown renderer, where a `---`
//! becomes a rule and a `# ...` becomes a heading — lines that are not merely
//! misdrawn but absent. Nothing here can do that. A false symbol adds one wrong
//! row to a jump list; the reader sees a name that does not look like anything
//! they were after, and the worst it can do is land them in the wrong part of a
//! document they still have all of. So the scanner guards the cheap cases — a
//! line comment, a Python triple-quoted block — and does not attempt to know
//! what is inside a Rust string literal.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::list::Cursor;
use super::source::expand_tabs;
use super::theme;
use crate::pane::Handled;
use crate::scroll;
use crate::text::{block, clip, clip_line};

/// The joiner between the parts of a breadcrumb. A single-cell `›` rather than
/// `>` or `/`, because both of those are things a heading can contain and the
/// separator has to stay readable as a separator in `Paths › a/b > c`.
const CRUMB_JOIN: &str = " › ";

/// How many cells of title a breadcrumb may propose to take before it starts
/// dropping its outer levels.
///
/// A cap and not a fit: [`Pane::title`](crate::pane::Pane::title) is handed no
/// width and cannot be, because the shell decides how much of the border is
/// left after the exit hint, the pending mark and the workspace label. So this
/// is not "what will fit", it is "how much a convenience may ask for" — and the
/// number is chosen against the 46 columns this pane is routinely given, where
/// a whole chain would otherwise be longer than everything else in the title
/// put together.
const CRUMB_MAX: usize = 30;

/// Cells of indent per level in the list. Two, the same step the markdown
/// renderer gives a nested list, so an outline of a document looks like the
/// document's own nesting rather than like a second convention.
const STEP: usize = 2;

/// How deep the drawn indent may go, in levels.
///
/// The level itself is never clamped — it is what the breadcrumb chains on, and
/// a clamped one would make two different depths compare equal. This is only
/// about the row: past four steps the indent is eating the name, and the name
/// is the whole of what the row is for.
const MAX_STEPS: usize = 4;

/// One place in the laid-out document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Which laid-out row of [`ViewerPane::lines`](super::ViewerPane). Invalid
    /// the moment `lines` is rebuilt — see the module doc, and
    /// [`search::Hit::row`](super::search::Hit::row), which says the same thing
    /// about the same vector.
    pub row: usize,
    /// Heading level for markdown, nesting depth for a source file. One-based,
    /// so a top-level item is 1 and the `fn` inside an `impl` block is 2.
    pub level: u8,
    /// What the row says: the heading's text, or the definition's label.
    pub text: String,
}

// --- the index -----------------------------------------------------------

/// The definitions in a source file, as `(0-based source line, level, label)`.
///
/// **Source lines, not rows.** This has never seen a layout and must not: the
/// mapping from a line of the file to the row it was drawn on is made by
/// `super::source_lines`, in the pass that emits the row, and is the only
/// version of it that can be right for the `lines` it was built beside.
///
/// The language is chosen by extension, and the rule is
/// [`docs::regions`](super::docs::regions)'s rather than a second one: `.rs` is
/// Rust, `.py` and `.pyi` are Python, everything else has no outline. The test
/// is written out again here rather than shared, and that is the one thing in
/// this module a reviewer should push on. `docs::language` is private to a
/// module this change does not own, and publishing it to import a two-line
/// extension test would be a change to the settled half of the feature for the
/// benefit of the new half. What it costs is that the two can drift: a language
/// added to `docs` and not to this list opens as a document whose prose is
/// rendered and whose definitions are not listed, which is a pane that half
/// knows what it is looking at. If a third language is ever added, both lists
/// have to move.
///
/// Ordered by line and never repeated, because both callers depend on it: the
/// row mapping walks it with a cursor that only goes forwards, and
/// [`ancestors`] binary-searches what is built from it.
pub fn symbols(text: &str, path: &Path) -> Vec<(usize, u8, String)> {
    match language(path) {
        Some(Lang::Rust) => rust(text),
        Some(Lang::Python) => python(text),
        None => Vec::new(),
    }
}

enum Lang {
    Rust,
    Python,
}

fn language(path: &Path) -> Option<Lang> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("rs") {
        return Some(Lang::Rust);
    }
    if ext.eq_ignore_ascii_case("py") || ext.eq_ignore_ascii_case("pyi") {
        return Some(Lang::Python);
    }
    None
}

/// Nesting depth, worked out from indentation and from nothing else.
///
/// A stack of the indents that are still open rather than `indent / 4`, and the
/// difference is the whole reason this is a type. Four columns is Rust's
/// convention and not Python's — two-space Python is ordinary — so under a
/// divisor every method of such a file would come out at the same level as its
/// class. A stack asks the only question actually being asked, which is whether
/// this item is further in than the one still open, and it is right for tabs,
/// for three spaces and for a file that mixes them.
#[derive(Default)]
struct Depth {
    open: Vec<usize>,
}

impl Depth {
    fn level(&mut self, indent: usize) -> u8 {
        while self.open.last().is_some_and(|&at| indent <= at) {
            self.open.pop();
        }
        self.open.push(indent);
        // Saturating rather than wrapping, and the clamp is unreachable in any
        // real file: it takes 255 strictly increasing indents to reach it.
        u8::try_from(self.open.len()).unwrap_or(u8::MAX)
    }
}

/// Columns of leading whitespace, and the rest of the line after it.
///
/// Tabs are expanded first, for `source::expand_tabs`'s reason one module
/// along: a `\t` is not a thing a terminal cell holds, and the pane has already
/// expanded the row this line is about, so measuring the indent any other way
/// would leave this scanner and the gutter beside it with two different ideas
/// of what column something is in.
fn indented(raw: &str) -> (usize, String) {
    let line = expand_tabs(raw);
    let indent = line.len() - line.trim_start().len();
    let rest = line[indent..].trim_end().to_string();
    (indent, rest)
}

/// The first word of `s`, and what follows it with the gap eaten.
///
/// Split at the first character an identifier cannot contain rather than at
/// whitespace, because `impl<T> Foo` has no space after the keyword and a
/// whitespace split would read the whole of `impl<T>` as a word that is not
/// `impl`.
fn word(s: &str) -> (&str, &str) {
    let end = s
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(s.len());
    (&s[..end], s[end..].trim_start())
}

/// The label a row shows, cut out of the line it was found on.
///
/// The line from the keyword onwards, stopped at the first character that
/// begins something the reader does not need in a jump list — an argument list,
/// a body, an initialiser. Not parsed: `fn parse(text: &str) -> Vec<Hit>` is
/// cut to `fn parse` by finding a `(`, which is the same amount of work as
/// looking at the line and is right for every shape of it.
///
/// What is deliberately dropped is everything *before* the keyword: `pub`,
/// `pub(crate)`, `async`, `unsafe`, `default`, `extern "C"`. They are true of
/// the item and they are the same handful of words over and over, so in a
/// column forty cells wide at best they push the name — the one part that
/// differs between rows — off the right-hand side. The source is one `Enter`
/// away and says all of it.
fn label(item: &str, cuts: &[char]) -> String {
    let end = item.find(cuts).unwrap_or(item.len());
    item[..end].trim_end().to_string()
}

/// Rust definitions.
///
/// The keyword has to be the first thing on the line bar a visibility or a
/// modifier, which is what item position means and is also most of the guard
/// against finding one inside a string: a `fn` in the middle of a sentence is
/// not at the start of a line, and a `///` line is skipped outright.
///
/// **A raw string whose lines start at item position is not guarded**, and this
/// repository's own fixtures are the obvious example — `r#"fn main() {"#` in a
/// test adds a row named `fn main`. Tracking Rust string state properly means
/// knowing about `r#`, escapes and char literals, which is a lexer; the module
/// doc argues why a wrong row does not earn one.
fn rust(text: &str) -> Vec<(usize, u8, String)> {
    let mut out = Vec::new();
    let mut depth = Depth::default();
    for (n, raw) in text.split('\n').enumerate() {
        let (indent, rest) = indented(raw);
        let Some(item) = rust_item(&rest) else {
            continue;
        };
        // `impl` keeps its generics, because `impl<T> From<T> for Wrap<T>` is
        // most of what distinguishes one `impl` block from the next in a file
        // that has six of them. Everywhere else `<` is a cut, so `struct
        // Page<'a>` reads as `struct Page`.
        let cuts: &[char] = if item.starts_with("impl") {
            &['(', '{', ';']
        } else {
            &['(', '{', '=', ';', '<']
        };
        let text = label(item, cuts);
        if text.is_empty() {
            continue;
        }
        out.push((n, depth.level(indent), text));
    }
    out
}

/// The line from its item keyword onwards, or `None` if it does not start one.
fn rust_item(rest: &str) -> Option<&str> {
    // The cheapest guard there is, and the only one a line comment needs. `*`
    // catches the continuation rows of a `/** */` block, which are the lines of
    // a block comment that could otherwise look like anything.
    if rest.starts_with("//") || rest.starts_with("/*") || rest.starts_with('*') {
        return None;
    }
    let mut rest = rest;
    loop {
        if let Some(after) = strip_pub(rest) {
            rest = after;
            continue;
        }
        let (kw, tail) = word(rest);
        match kw {
            "fn" | "struct" | "enum" | "trait" | "mod" | "type" | "union" => {
                return named(rest, tail, false);
            }
            // The one keyword that has to allow more than a name after it: an
            // `impl` may carry a generic parameter list before the type it is
            // about, which is why this is not folded into the arm above.
            "impl" => return named(rest, tail, true),
            // Both are modifiers *and* items, and which one decides whether
            // this line is `const MAX: usize = 4` or `const fn max()`. Wrong
            // either way is a label rather than a lost line, but a `const fn`
            // labelled `const` would be a row that names no function in a file
            // full of them.
            "const" | "static" => {
                if matches!(word(tail).0, "fn" | "unsafe" | "extern" | "async") {
                    rest = tail;
                    continue;
                }
                return named(rest, tail, false);
            }
            "unsafe" | "async" | "default" | "extern" => rest = tail,
            _ => return None,
        }
    }
}

/// A keyword is only an item if something nameable follows it, which is what
/// keeps `type` — an ordinary English word — from turning every line of prose
/// that opens with it into a row.
fn named<'a>(item: &'a str, tail: &str, generic: bool) -> Option<&'a str> {
    let ok = tail.starts_with(|c: char| {
        c.is_alphabetic() || c == '_' || (generic && (c == '<' || c == '\''))
    });
    ok.then_some(item)
}

/// `pub`, `pub(crate)`, `pub(super)`, `pub(in a::b)`, and what follows it.
///
/// Written out rather than matched as a word because the restricted forms carry
/// a parenthesised path that may contain a space, so there is no split of the
/// line into words that puts `pub(in crate::panes)` on one side of it.
fn strip_pub(rest: &str) -> Option<&str> {
    let after = rest.strip_prefix("pub")?;
    let after = match after.strip_prefix('(') {
        Some(inner) => &inner[inner.find(')')? + 1..],
        None => after,
    };
    // Or `public_api` would be read as a `pub` in front of nothing at all.
    (after.is_empty() || after.starts_with(char::is_whitespace)).then(|| after.trim_start())
}

/// Python definitions.
///
/// `class`, `def` and `async def`, at whatever indent they are written at —
/// which is also the level, because in Python the indent *is* the nesting and
/// there is nothing to guess.
///
/// Triple-quoted blocks are tracked, unlike Rust's strings, and the reason is
/// that they are cheap to track and expensive not to: a docstring holding an
/// example is the normal way Python documentation is written, and an indented
/// `def` inside one would put a row in the list for a function that does not
/// exist. Counting delimiters on a line is not a lexer — a `"""` inside a `#`
/// comment or inside a single-quoted string miscounts, and then a run of the
/// file is skipped rather than a wrong row added, which is the direction to be
/// wrong in.
fn python(text: &str) -> Vec<(usize, u8, String)> {
    let mut out = Vec::new();
    let mut depth = Depth::default();
    let mut open: Option<&str> = None;
    for (n, raw) in text.split('\n').enumerate() {
        // Whether the line *started* inside a string, which is the question,
        // rather than whether it ends inside one: the code in front of a
        // docstring's opening quotes is still code.
        let inside = open.is_some();
        match open {
            Some(delim) => {
                if raw.matches(delim).count() % 2 == 1 {
                    open = None;
                }
            }
            None => {
                if raw.matches("\"\"\"").count() % 2 == 1 {
                    open = Some("\"\"\"");
                } else if raw.matches("'''").count() % 2 == 1 {
                    open = Some("'''");
                }
            }
        }
        if inside {
            continue;
        }
        let (indent, rest) = indented(raw);
        if rest.starts_with('#') {
            continue;
        }
        // The `async` is dropped from the label along with Rust's visibility
        // words and for the same reason: it is true of the definition and it is
        // not what tells one row from the next.
        let item = rest
            .strip_prefix("async ")
            .map_or(&rest[..], str::trim_start);
        let (kw, tail) = word(item);
        if !matches!(kw, "def" | "class") || named(item, tail, false).is_none() {
            continue;
        }
        // `:` is in the cut set here and not in Rust's, because it is what ends
        // a `class A:` with no bases — where in Rust it would cut `const MAX:
        // usize` down to a name with no type and `impl<T: Debug>` down to
        // nothing worth reading.
        let text = label(item, &['(', ':', '=']);
        if text.is_empty() {
            continue;
        }
        out.push((n, depth.level(indent), text));
    }
    out
}

// --- the breadcrumb ------------------------------------------------------

/// The chain of entries enclosing `row`, outermost first.
///
/// The innermost entry at or above the row, then each entry above *that* at a
/// lower level, which is what "encloses" means in a list that carries depth and
/// no parent pointers.
///
/// The first step is a binary search, and that is not an optimisation to be
/// traded away: [`Pane::title`](crate::pane::Pane::title) runs on **every
/// frame**, and a linear scan of a 512 KB source file's thousand-odd
/// definitions would be on the thread that pumps the agent's pty. The walk back
/// for the ancestors is linear in the entries passed on the way and is left
/// that way deliberately: bounding it means a parent index on every [`Entry`],
/// which is one more field that has to be rebuilt correctly with the rows every
/// single time, to save a scan whose worst case — one heading with a thousand
/// same-level siblings under it — is a thousand integer comparisons.
pub fn ancestors(entries: &[Entry], row: usize) -> Vec<&str> {
    let at = entries.partition_point(|e| e.row <= row);
    let Some(mut i) = at.checked_sub(1) else {
        return Vec::new();
    };
    let mut chain = vec![entries[i].text.as_str()];
    let mut level = entries[i].level;
    while level > 1 && i > 0 {
        i -= 1;
        if entries[i].level < level {
            level = entries[i].level;
            chain.push(entries[i].text.as_str());
        }
    }
    chain.reverse();
    chain
}

/// The chain as one string, elided from its **outer** end.
///
/// `None` when there is nothing above the reader, which is a document with no
/// outline and also the top of one that has.
///
/// Outer end, because the innermost heading is the one the reader is under and
/// the one they cannot otherwise name. Dropping from the inner end would leave
/// `The panes › …`, which answers a question nobody has: which file they are in
/// is the title's own first word.
pub fn crumb(entries: &[Entry], row: usize) -> Option<String> {
    let chain = ancestors(entries, row);
    let (last, rest) = chain.split_last()?;
    // The innermost first and on its own terms: if it alone is over budget it
    // is clipped rather than dropped, because there is nothing left to drop to
    // and a breadcrumb with nothing in it is worse than a shortened one.
    let mut parts = vec![clip(last, CRUMB_MAX)];
    let mut used = parts[0].width();
    let mut all = true;
    for part in rest.iter().rev() {
        let want = used + CRUMB_JOIN.width() + part.width();
        if want > CRUMB_MAX {
            all = false;
            break;
        }
        used = want;
        parts.push((*part).to_string());
    }
    if !all {
        parts.push("…".into());
    }
    parts.reverse();
    Some(parts.join(CRUMB_JOIN))
}

// --- the view ------------------------------------------------------------

/// What came of a key, in the vocabulary the pane needs. `Ignored` maps to
/// `Handled::No` for [`grep::Outcome`](super::grep::Outcome)'s reason: a key
/// that changed nothing must not cost a frame, and a frame here re-renders the
/// agent's whole screen.
pub enum Outcome {
    Ignored,
    Moved,
    /// `Esc`, `q` or a second `o`. The reader goes back to the document exactly
    /// where they left it.
    Leave,
    /// `Enter`. Put the document at this row and show it.
    Jump(usize),
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

/// The list `o` puts on screen.
///
/// It holds a [`Cursor`] and nothing else, which is the point of it: the
/// directory listing, the find over file names and the results of a repository
/// search are already three lists behind that one type, and a fourth written by
/// hand is how `End` comes to land on the first row of the last page in one of
/// them and on the last row in the other three. See [`super::list`].
///
/// It does **not** hold the rows. They live on the pane, beside the `lines`
/// they index and rebuilt with them, and are handed in per call — the same
/// arrangement `Cursor` itself insists on, and here it is load-bearing rather
/// than tidy: a copy kept in this struct would be exactly the stale row the
/// module doc opens by ruling out.
///
/// It does not hold the palette either. `F3`, `set_theme` and `set_root` each
/// change that, and a copy here would be a fourth place that has to remember;
/// the pane has the answer and passes it to the one method that draws.
pub struct View {
    cursor: Cursor,
}

impl View {
    pub fn new(viewport: usize) -> Self {
        View {
            cursor: Cursor::new(viewport),
        }
    }

    /// Open on the section the reader is already in.
    ///
    /// `offset` is where the document is scrolled to, so the row that opens
    /// selected is the one the breadcrumb has been naming — a list that opened
    /// at the top would make `o` a key that loses your place in order to show
    /// you where it is.
    ///
    /// `viewport` is the pane's real height rather than a guess, which it can
    /// be because the document was drawn a frame ago and this list is exactly
    /// as tall. `Cursor::new`'s own guess exists for the case where nothing has
    /// been drawn at all; leaning on it here would make the first `PageDown`
    /// after `o` — drained from the same batch of keys, before any frame — move
    /// by the wrong number of rows.
    pub fn open(&mut self, entries: &[Entry], offset: usize, viewport: usize) {
        self.cursor = Cursor::new(viewport.max(1));
        self.cursor.sel = entries
            .partition_point(|e| e.row <= offset)
            .saturating_sub(1);
        self.cursor.reveal();
    }

    /// `n/m`, in the register the other two lists use for the same fact.
    ///
    /// Which row is chosen is deliberately not exposed any other way. It is
    /// only ever acted on from inside this type — `Enter` reads it, and the
    /// pane is handed the *row* rather than the index — so a getter would be a
    /// second way to ask a question with one answer, and the tests read the
    /// title, which is what the reader reads.
    pub fn title(&self, entries: &[Entry], name: &str) -> String {
        format!(
            "{name} · outline · {}/{}",
            (self.cursor.sel + 1).min(entries.len().max(1)),
            entries.len()
        )
    }

    pub fn render(&mut self, f: &mut Frame, inner: Rect, entries: &[Entry], mode: theme::Mode) {
        let rows = entries.len();
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
            // Unreachable while `o` declines an empty outline, and drawn anyway
            // for the reason every other list in this pane draws something: a
            // blank pane is indistinguishable from a broken one, and a guard
            // one function away is not a guard this one can see.
            block(
                "Nothing to list in this document.",
                text_w as usize,
                mode.theme().dim(),
            )
        } else {
            (offset..rows)
                .take(inner.height as usize)
                .map(|i| self.line(&entries[i], i == self.cursor.sel, text_w as usize, mode))
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

    /// One entry: indented by its level, and coloured by it.
    ///
    /// Both, and the pairing is `theme::Theme::heading`'s rule rather than a
    /// choice made here: hue is what an eye skimming for a level is fastest at,
    /// and hue alone is never a signal, so the indent carries the same fact for
    /// a reader who receives no colour. The bold is taken off, which is the one
    /// place this departs from the document: a heading is bold there because it
    /// is one row among many that are not, and a list where every row is a
    /// heading has nothing left for the bold to contrast with.
    fn line(&self, entry: &Entry, selected: bool, w: usize, mode: theme::Mode) -> Line<'static> {
        let t = mode.theme();
        let steps = usize::from(entry.level).saturating_sub(1).min(MAX_STEPS);
        let mut spans = vec![
            Span::raw(" ".repeat(steps * STEP + 1)),
            Span::styled(
                entry.text.clone(),
                t.heading(entry.level).remove_modifier(Modifier::BOLD),
            ),
        ];
        // Every row is clipped here and nowhere else. A pane that overflows its
        // rect corrupts the frame rather than merely looking wrong.
        spans = clip_line(Line::from(spans), w).spans;
        if !selected {
            return Line::from(spans);
        }
        // Padded to the full width, or the highlight would stop at the end of
        // the text instead of marking the row. `browse::line`'s rule.
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
        Line::from(spans).style(t.selection())
    }

    /// **This list claims no search key, and the omission is the decision.**
    ///
    /// `/` in this pane means "find a phrase in the rows on screen", and its
    /// hits are indexed into `lines` — the *document's* rows, which are not
    /// what is on screen here. Wiring it up would mean one of two things: a
    /// search whose highlighting is painted onto a document nobody can see, or
    /// a fourth matcher over a different vector of rows, which is precisely the
    /// fourth copy [`super::list`] exists to prevent.
    ///
    /// `f` is declined for a narrower reason. It would have to come back to
    /// somewhere, and `super::Back` says in as many words that there are
    /// exactly two answers because they are the two settled modes — a third
    /// would make the results reachable from a view that is itself a layer over
    /// the document, which is the `Results { back: Results }` shape that enum
    /// exists to make unrepresentable. `Esc` and then `f` is two keys, and it
    /// is the same two keys the file list already needs to reach a document.
    ///
    /// What is left is the list vocabulary, `Enter`, and the three ways out.
    pub fn key(&mut self, entries: &[Entry], key: KeyEvent) -> Outcome {
        if let Some(handled) = self.cursor.key(entries.len(), key) {
            return handled.into();
        }
        match key.code {
            // `Ctrl` plus a letter is the agent's everywhere in this program,
            // and `Cursor::key` hands it back rather than declining it so that
            // this arm is where it gets decided.
            KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => Outcome::Ignored,
            KeyCode::Enter => match entries.get(self.cursor.sel) {
                Some(entry) => Outcome::Jump(entry.row),
                None => Outcome::Ignored,
            },
            // All three mean "never mind", and never mind does not cost you
            // your place: `o` because undoing the keystroke that opened this is
            // the rule every box in this pane follows, `Esc` and `q` because
            // this view claims them rather than letting them fall through to
            // the shell as "give focus back to the agent" — the reader is one
            // key from the document, not one key from leaving the pane.
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('o') => Outcome::Leave,
            _ => Outcome::Ignored,
        }
    }

    pub fn mouse(&mut self, entries: &[Entry], ev: &MouseEvent) -> Outcome {
        self.cursor
            .mouse(entries.len(), ev)
            .unwrap_or(Handled::No)
            .into()
    }

    /// The glance keys, arriving as the bare key. They move the view and
    /// nothing else: reading the pane from the other side of the window must
    /// not re-aim the `Enter` that follows it. `browse::scroll_view`'s rule.
    pub fn scroll_view(&mut self, key: KeyEvent) -> Handled {
        self.cursor.scroll.key(key).unwrap_or(Handled::No)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(level, label)` for every symbol found, which is what the assertions
    /// below are about — the line numbers are pinned separately and only where
    /// they are the point.
    fn found(text: &str, name: &str) -> Vec<(u8, String)> {
        symbols(text, Path::new(name))
            .into_iter()
            .map(|(_, level, label)| (level, label))
            .collect()
    }

    fn entry(row: usize, level: u8, text: &str) -> Entry {
        Entry {
            row,
            level,
            text: text.into(),
        }
    }

    #[test]
    fn a_rust_file_lists_its_items_and_makes_an_impl_blocks_functions_its_children() {
        let src = concat!(
            "const MAX: usize = 4;\n",
            "\n",
            "/// fn documented\n",
            "pub struct Entry {\n",
            "    row: usize,\n",
            "}\n",
            "\n",
            "impl Entry {\n",
            "    pub(crate) fn row(&self) -> usize {\n",
            "        fn helper() {}\n",
            "        self.row\n",
            "    }\n",
            "}\n",
            "\n",
            "pub mod tests {\n",
            "    async fn go() {}\n",
            "}\n",
        );
        assert_eq!(
            found(src, "a.rs"),
            [
                // `= 4;` is not part of the name, and neither is the `pub`.
                (1, "const MAX: usize".into()),
                (1, "struct Entry".into()),
                // The whole point of the level: everything inside the `impl`
                // block hangs off it, and the `fn` inside *that* off the `fn`.
                (1, "impl Entry".into()),
                (2, "fn row".into()),
                (3, "fn helper".into()),
                (1, "mod tests".into()),
                (2, "fn go".into()),
            ]
        );
        // The `///` line holds a perfectly good `fn` at item position and is
        // not one. A line comment is the one string case that is guarded.
        assert!(!found(src, "a.rs").iter().any(|(_, l)| l == "fn documented"));
    }

    #[test]
    fn a_python_file_takes_its_levels_from_the_indentation_it_is_written_at() {
        // Two-space indent, which is the fixture rather than an aside: a
        // `indent / 4` rule would put every method of this class at the same
        // level as the class, and that is the mistake `Depth` exists to avoid.
        let src = concat!(
            "import os\n",
            "\n",
            "class Store:\n",
            "  def get(self, key):\n",
            "    return key\n",
            "\n",
            "  async def put(self, key):\n",
            "    def inner():\n",
            "      pass\n",
            "    return inner\n",
            "\n",
            "def free():\n",
            "  pass\n",
        );
        assert_eq!(
            found(src, "s.py"),
            [
                (1, "class Store".into()),
                (2, "def get".into()),
                // `async` goes the way Rust's `pub` does: true of it, and not
                // what tells one row from the next.
                (2, "def put".into()),
                (3, "def inner".into()),
                (1, "def free".into()),
            ]
        );
    }

    #[test]
    fn a_definition_written_inside_a_python_docstring_is_not_a_definition() {
        // A docstring holding an example is how Python documentation is
        // normally written, so this is the common case rather than a corner.
        let src = concat!(
            "class Store:\n",
            "    \"\"\"Holds things.\n",
            "\n",
            "    def not_a_method(self):\n",
            "        ...\n",
            "    \"\"\"\n",
            "\n",
            "    def real(self):\n",
            "        pass\n",
        );
        assert_eq!(
            found(src, "s.py"),
            [(1, "class Store".into()), (2, "def real".into())]
        );
        // And the block is left again, rather than swallowing the rest of the
        // file: `def real` above is the proof, and this is the line that says
        // so if the assertion above is ever loosened.
        assert_eq!(symbols(src, Path::new("s.py"))[1].0, 7);
    }

    #[test]
    fn a_language_this_does_not_read_has_no_outline_and_neither_does_markdown() {
        let rust = "fn main() {}\n";
        assert!(found(rust, "notes.txt").is_empty());
        // The recorded decision, pinned so that changing it has to change a
        // test: markdown's headings come from the renderer, and a `.md` shown
        // as its source is the one form of a document with no outline at all.
        // See the module doc for why a second scanner was refused.
        assert!(found("# Heading\n\nbody\n", "design.md").is_empty());
    }

    #[test]
    fn a_keyword_with_nothing_nameable_after_it_is_not_an_item() {
        // `type` is an ordinary English word, and a line of prose that opens
        // with it — inside a raw string, say, where nothing guards it — must
        // not become a row just for starting with the right five letters.
        assert!(found("type\n", "a.rs").is_empty());
        assert!(found("impl\n", "a.rs").is_empty());
        assert!(found("fn 9lives() {}\n", "a.rs").is_empty());
        // And a word that merely starts with `pub` is not a visibility.
        assert!(found("public_holiday(x);\n", "a.rs").is_empty());
    }

    #[test]
    fn an_impl_keeps_its_generics_where_everything_else_gives_them_up() {
        // The one asymmetry in the cut sets, and the reason for it: `impl<T>
        // From<T> for Wrap<T>` is mostly generics, and a file with six `impl`
        // blocks in it would otherwise list `impl` six times.
        assert_eq!(
            found("impl<T> From<T> for Wrap<T> {\n", "a.rs"),
            [(1, "impl<T> From<T> for Wrap<T>".into())]
        );
        assert_eq!(
            found("pub struct Page<'a> {\n", "a.rs"),
            [(1, "struct Page".into())]
        );
    }

    #[test]
    fn a_const_fn_is_labelled_as_the_function_it_is() {
        assert_eq!(found("const fn max() {}\n", "a.rs"), [(1, "fn max".into())]);
        assert_eq!(
            found("static NAME: &str = \"\";\n", "a.rs"),
            [(1, "static NAME: &str".into())]
        );
        assert_eq!(found("mod files;\n", "a.rs"), [(1, "mod files".into())]);
        // And the edge that comes with the cut set, pinned rather than left to
        // be discovered: a `;` inside a type is a cut like any other, so an
        // array's length goes with it. A label that stops early is the cheapest
        // thing this module can get wrong — it still names the item, and
        // `Enter` still lands on the line it is on.
        assert_eq!(
            found("static N: [u8; 2] = [];\n", "a.rs"),
            [(1, "static N: [u8".into())]
        );
    }

    #[test]
    fn the_breadcrumb_names_the_innermost_section_and_the_ones_enclosing_it() {
        let entries = vec![
            entry(0, 1, "Design"),
            entry(10, 2, "The panes"),
            entry(20, 3, "ask"),
            entry(30, 3, "git"),
            entry(40, 2, "Keys"),
        ];
        // Inside `ask`: itself, plus every level below it on the way out. `git`
        // is a sibling and is not in the chain, which is what "at a lower
        // level" buys over "everything above".
        assert_eq!(ancestors(&entries, 25), ["Design", "The panes", "ask"]);
        // Exactly on a heading's own row counts as being in it.
        assert_eq!(ancestors(&entries, 30), ["Design", "The panes", "git"]);
        assert_eq!(ancestors(&entries, 45), ["Design", "Keys"]);
        // Above the first heading there is nothing to name, and a document with
        // no outline at all is the same answer by the same route.
        assert!(ancestors(&entries, 0).len() == 1);
        assert!(crumb(&[], 0).is_none());
        assert_eq!(crumb(&entries, 25).unwrap(), "Design › The panes › ask");
    }

    #[test]
    fn a_long_breadcrumb_is_elided_from_its_outer_end() {
        let entries = vec![
            entry(0, 1, "A section with a very long name indeed"),
            entry(10, 2, "And another underneath it"),
            entry(20, 3, "the one you are in"),
        ];
        let shown = crumb(&entries, 25).unwrap();
        // The innermost survives whole, because it is the one the reader cannot
        // otherwise name; the outer end is what goes, marked.
        assert!(shown.ends_with("the one you are in"), "{shown}");
        assert!(shown.starts_with('…'), "{shown}");
        assert!(!shown.contains("A section"), "{shown}");
        // And a chain that fits is not marked at all.
        assert!(!crumb(&[entry(0, 1, "Short")], 5).unwrap().contains('…'));
    }

    #[test]
    fn no_row_of_the_list_is_ever_wider_than_the_pane_it_was_drawn_for() {
        // The invariant every view in this pane owes: a row that overflows its
        // rect corrupts the frame rather than merely looking wrong. Swept over
        // every width down to nothing, because a pane is dragged through all of
        // them, and over both the plain and the selected row — the selected one
        // is padded out to the full width *after* it is clipped, which is the
        // step that could put a cell back past the edge.
        let view = View::new(10);
        let deep = entry(0, 9, "a definition with a name far too long for this");
        for e in [&deep, &entry(0, 1, "short")] {
            for w in 0..40 {
                for selected in [false, true] {
                    let line = view.line(e, selected, w, theme::Mode::Dark);
                    let drawn: usize = line.spans.iter().map(|s| s.content.width()).sum();
                    assert!(drawn <= w, "{drawn} cells drawn in {w} columns");
                }
            }
        }
    }

    #[test]
    fn an_innermost_heading_too_long_to_fit_is_clipped_rather_than_dropped() {
        // There is nothing left to drop to, and an empty breadcrumb is worse
        // than a shortened one — the reader would be told nothing rather than
        // told most of it.
        let long = "a".repeat(200);
        let shown = crumb(&[entry(0, 1, &long)], 5).unwrap();
        assert!(shown.width() <= CRUMB_MAX, "{shown}");
        assert!(shown.ends_with('…'), "{shown}");
    }
}
