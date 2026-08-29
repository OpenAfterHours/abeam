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
//! collide in a grep, so this is the paragraph that says which is which. The
//! other module now carries the same paragraph pointing back at this one, which
//! is the whole of what makes either of them a signpost.
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
//! ### What that does not promise, which is a resize nobody has drawn yet
//!
//! It promises that an entry's row is a row of the `lines` it was built beside,
//! and that much holds by construction. It does **not** promise that the reader
//! ends up where the entry pointed, and the gap is one frame wide.
//!
//! A frame is the only thing that tells this pane how wide it is, and the shell
//! drains every queued event before it draws — so a terminal resize followed in
//! the same batch by `o` and `Enter` is answered against the *previous* frame's
//! layout, correctly, and then the re-wrap that the resize causes moves the
//! offset the jump has just set. Three keys inside one frame, which is key
//! repeat or a paste rather than anybody typing.
//!
//! It is not a property of any one mode. `Mode::Browse` and `Mode::Results`
//! skip `ensure_layout` altogether, which makes it look like theirs, but the
//! document reaches the same window by the shorter route of resize-then-`o` —
//! so hoisting the layout into those two branches would not close it. What is
//! actually happening is that a re-wrap re-interprets *every* offset, and the
//! reader who was simply scrolled somewhere is moved by exactly the same
//! amount. That is why it is left alone rather than fixed here: re-aiming the
//! outline's `Enter` across a re-wrap would make it the one position in this
//! pane that survives one, and the reader's own would still not.
//! `a_jump_aimed_before_the_relayout_is_re_wrapped_like_any_other_offset` pins
//! it from both routes, so that "an entry is never stale" is not read as "the
//! jump always lands".
//!
//! ## Two ways in, and three rooms behind one of them
//!
//! A rendered markdown file gets its entries from the renderer itself —
//! `markdown::render_outlined` records a heading at the moment it emits one, so
//! the row is the row, not a guess made afterwards by looking for bold text.
//! Everything else gets them from [`symbols`], which reports source *lines* and
//! leaves the row to `source_lines`.
//!
//! [`symbols`] is one door with three rooms behind it, and only two of them are
//! line scanners. `.rs` and `.py` are [`rust`] and [`python`], in the same
//! discipline as [`docs`]: one forward pass, no regex, no backtracking. A `.md`
//! shown as its source is [`headings`], which is not a scanner at all — it is
//! `pulldown_cmark`, and the next section is why.
//!
//! **Markdown has one in both of its forms**, and that is worth arguing rather
//! than stating, because half of it was refused once. What was refused is a
//! `#`-line scanner, and refusing that was right: a scanner and
//! `pulldown_cmark` disagree about a `#` inside a fence, about a setext heading
//! written under a row of `=`, and about a `#` in an indented code block, so one
//! document would list two different tables of its own contents depending on
//! which key was last pressed, with nothing on screen to say which was right.
//! What the refusal did not consider is the answer taken here: parse the source
//! **with the same parser**, over the same
//! [`markdown::options`], and read the headings out of
//! `into_offset_iter`, which reports the byte range each event came from. There
//! is still exactly one authority on what a heading is. [`headings`] asks it
//! where the headings are in the *source*; `render_outlined` asks it where they
//! landed in the *rows*; and
//! `a_markdown_outline_is_the_renderers_own_answer_read_off_the_source` holds
//! the two against each other on the three cases the scanner would have got
//! wrong.
//!
//! They agree on which lines are headings and at what level, which is the whole
//! of what the refusal was protecting. They can differ in what a row *says*, and
//! [`headings`] is where that is argued: the rendered form's label carries the
//! decoration the renderer drew — an image glyph, an elided destination — and
//! that decoration is width-dependent, so it could not be matched even if it
//! were wanted.
//!
//! Everything else — a `.txt`, a `.json`, a `.toml` — has no outline at all,
//! and `o` declines rather than opening an empty list.
//!
//! ## What a wrong entry costs, which is why the scanner may be cheap
//!
//! [`docs`] buys its exactness because a false positive there
//! hands a run of somebody's *code* to the markdown renderer, where a `---`
//! becomes a rule and a `# ...` becomes a heading — lines that are not merely
//! misdrawn but absent. Nothing here can do that. A false symbol adds one wrong
//! row to a jump list; the reader sees a name that does not look like anything
//! they were after, and the worst it can do is land them in the wrong part of a
//! document they still have all of. So the scanner guards the cheap cases — a
//! line comment, a Python string in any of its four spellings — and does not
//! attempt to know what is inside a Rust string literal.
//!
//! **One thing here is not cheap to get wrong, and it is the exception that
//! shapes [`python`].** Python's triple-quoted blocks span lines, so the state
//! carries, and a delimiter counted where there was none does not add a row —
//! it opens a string that never closes and drops every definition below it, with
//! nothing on screen to say rows are missing and a breadcrumb naming a stale
//! section for the whole swallowed span. That is why the quoted strings are
//! taken out of a line before its triples are looked for, and why that is the
//! one place in this module worth more than a line scanner's usual care.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::list::Cursor;
use super::source::expand_tabs;
use super::theme;
use super::{docs, markdown};
use crate::pane::Handled;
use crate::scroll;
use crate::text::{block, clip, clip_line};

/// The joiner between the parts of a breadcrumb. A single-cell `›` rather than
/// `>` or `/`, because both of those are things a heading can contain and the
/// separator has to stay readable as a separator in `Paths › a/b > c`.
const CRUMB_JOIN: &str = " › ";

/// What says the *outer* end of a chain was dropped. One cell, and deliberately
/// the same glyph [`clip`] uses for a name cut short, because both are the same
/// promise to the reader — there is more of this than you can see. Which of the
/// two a given `…` is is said by where it sits: this one is only ever the first
/// thing in the crumb.
const CRUMB_MARK: &str = "…";

/// How many cells of title a breadcrumb may take, marker included.
///
/// A cap and not a fit: [`Pane::title`](crate::pane::Pane::title) is handed no
/// width and cannot be, because the shell decides how much of the border is
/// left after the exit hint, the pending mark and the workspace label. So this
/// is not "what will fit", it is "how much a convenience may ask for" — and
/// what it buys is that the eliding happens **here**, where the parts are known
/// to be levels of a document, instead of in a border cutting a string wherever
/// it runs out.
///
/// Thirty was too much for that to buy anything, and the reason recorded for
/// thirty — that it was chosen against the 46 columns this pane is routinely
/// given — was not true of any measurement. Measured on this repository's
/// `docs/design.md`, scrolled into `Asking Copilot instead`: the title in front
/// of the crumb is 34 cells, and a focused border puts `esc→agent · ` in front
/// of *that* for another 12. At that width a thirty-cell cap is never the
/// binding constraint — the border is — and what reached the screen was
/// `… › Asking …`: one mark from here meaning "levels were dropped", one from
/// the shell meaning "the title did not fit", side by side with nothing to tell
/// them apart.
///
/// Fourteen is small enough that what this function returns is usually the
/// whole of what the reader is shown, so both marks in it are this module's own
/// and [`CRUMB_MARK`] can say which is which by where it sits. It is not a
/// promise that nothing is clipped: `Asking Copilot instead` is 22 cells and
/// comes back as `… › Asking Co…` at any cap this small, and a narrow focused
/// border can still take the whole crumb away. That second one is the
/// ordering's business rather than this constant's — the crumb is the first
/// thing the title gives up, deliberately, and `ViewerPane::title` argues why.
///
/// It is a cap on the finished string and not on the chain before the marker is
/// added, which is the distinction the first version of this got wrong: the
/// whole string is built and *that* is measured, so a cap of thirty can no
/// longer return thirty-four cells.
const CRUMB_MAX: usize = 14;

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

/// What a document names its own parts, as `(0-based source line, level,
/// label)`: the definitions of a source file, and the headings of a markdown
/// one shown as its source.
///
/// **Source lines, not rows.** This has never seen a layout and must not: the
/// mapping from a line of the file to the row it was drawn on is made by
/// `super::source_lines`, in the pass that emits the row, and is the only
/// version of it that can be right for the `lines` it was built beside.
///
/// The language is chosen by extension, and by **the same two functions the
/// rest of the pane chooses it with** rather than by a third list written out
/// here: [`docs::language`] says which source language a
/// path is, and `watch::is_markdown` says whether it is a document — which is
/// the predicate `show` already asks to decide whether the body is
/// [`Body::Markdown`](super::Body).
///
/// This used to be a copy of `docs::language` and a copy of its `enum Lang`,
/// defended on the grounds that publishing them would change the settled half
/// of the feature for the benefit of the new half. The same diff reached into
/// `markdown` for exactly that benefit — splitting `render`, adding a field to
/// the renderer and changing `finish`'s return type — so the defence did not
/// survive being read next to the change it was written in. `mod docs;` is
/// private, so `pub(super)` widens nothing outside `panes::viewer`, and `docs`
/// already exports `regions` and `Region` to that same `super`. What the copy
/// cost was one-directional drift with no test that could fail: a language
/// added to `docs` alone would open as a document whose prose is rendered and
/// whose definitions are silently unlisted.
///
/// Ordered by line and never repeated, because both callers depend on it: the
/// row mapping walks it with a cursor that only goes forwards, and
/// [`ancestors`] binary-searches what is built from it.
pub fn symbols(text: &str, path: &Path) -> Vec<(usize, u8, String)> {
    // Asked first, and it has to be: a `.md` is not a source language, so
    // `docs::language` says `None` about one and would send a document whose
    // headings are plainly listable down the "no outline" arm.
    if crate::watch::is_markdown(path) {
        return headings(text);
    }
    match docs::language(path) {
        Some(docs::Lang::Rust) => rust(text),
        Some(docs::Lang::Python) => python(text),
        None => Vec::new(),
    }
}

/// The headings of a markdown file, as `(0-based source line, level, text)`.
///
/// **The renderer's own parser, asked a different question.** `Parser` with
/// [`markdown::options`] is what decides what a
/// heading is in this pane; `into_offset_iter` is the same walk with each
/// event's byte range attached, so the answer here and the answer
/// `markdown::render_outlined` records cannot disagree about *what* is a
/// heading. They can only differ in what they index — a line of the file here, a
/// row of the layout there — which is the one thing a document shown in two
/// forms needs them to differ in. See the module doc for what was refused
/// instead.
///
/// The text is joined from the heading's own events rather than sliced out of
/// the source, so `## A **bold** word` is listed as `A bold word` and
/// `` ## The `o` key `` as `The o key`: the words, without the punctuation that
/// was only ever markup.
///
/// **The words, and not the decoration the rendering adds on top of them**,
/// which is where this and `render_outlined` differ and the difference is
/// deliberate. A heading holding an image or a link comes back from the
/// renderer carrying `▨` and an elided destination, because that is what the
/// renderer *drew* — and what it drew depends on the pane's width, since
/// `elide_url` is given a share of it. Neither is a name the document gave the
/// section; both are the pane talking about it. So each form's outline names
/// what that form has on screen, and what the two agree on is the thing a
/// `#`-line scanner would have got wrong: which lines are headings, and at what
/// level. Measured over 4001 markdown files off this machine's disk: 48,270
/// headings, the same count and the same level sequence as the renderer in
/// every one of the 4001, and a label that differs in 829 of them — badges and
/// links in headings, and nothing else.
///
/// An empty heading is left out, for `render_outlined`'s reason: a blank row in
/// a jump list is a target the reader cannot choose between.
fn headings(source: &str) -> Vec<(usize, u8, String)> {
    let mut out: Vec<(usize, u8, String)> = Vec::new();
    let mut open: Option<(usize, u8, String)> = None;
    // Where the running line count is true up to. Heading starts arrive in
    // document order, so the newlines in front of each one are counted once
    // between it and the one before rather than from the top of the file every
    // time — which over a 512 KiB document with a heading every few lines is the
    // difference between one pass and hundreds.
    let mut at = 0usize;
    let mut line = 0usize;
    for (event, range) in Parser::new_ext(source, markdown::options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                // `max` rather than a bare slice: a range that started behind
                // the cursor would be `start > end` and a panic on the key that
                // opens a file. There is no such range — `into_offset_iter`
                // walks the source forwards and headings cannot nest — so this
                // is the cheapest way to say "and if there ever is, the heading
                // takes the line of the one before it".
                let start = range.start.max(at);
                line += source[at..start].matches('\n').count();
                at = start;
                open = Some((line, level as u8, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((line, level, text)) = open.take() {
                    let text = text.trim();
                    if !text.is_empty() {
                        out.push((line, level, text.to_string()));
                    }
                }
            }
            // Everything the renderer would have put in the heading's text.
            // `Code` is in the list because `## The `o` key` reads as `The o
            // key` on the page and has to read as that here too.
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                if let Some((.., text)) = open.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((.., text)) = open.as_mut() {
                    text.push(' ');
                }
            }
            _ => {}
        }
    }
    out
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
        // `unwrap_or` and not a cast, which is the whole of the difference: `as
        // u8` wraps, so the 256th open indent would be level 0 and the outline
        // would say that the most deeply nested item in the file is its
        // outermost. This says `u8::MAX` instead, and the fallback is
        // unreachable in any real file — 256 strictly increasing indents, one
        // per level, with no line closing any of them.
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

/// One line of Python with its strings and its comment taken out, and whether
/// it ends inside a triple-quoted block.
///
/// `open` is the block that was already running when the line started, or
/// `None`. What comes back is the line's *code* — enough to find a `def` on and
/// to measure the indent of, since neither a string nor a `#` can be in front of
/// the indent — and the block still open at the end of it.
///
/// One forward pass over the bytes, in [`docs`]'s discipline: no backtracking,
/// no regex, and no attempt to be a lexer. Two things it does not know, both
/// left deliberately: a `\` at the end of a line joins that line to the next,
/// which this reads as two lines; and a triple-quoted block written inside
/// another one is read as the first one ending where the second one starts.
/// The second is the whole of the residue in the sweep [`python`] records, and
/// it is one file of 7362.
///
/// The output is only ever read for its leading whitespace and its first word,
/// so the removed strings leave nothing behind rather than a placeholder — a
/// `def` cannot be spelled across the gap a string used to fill.
fn code_of(line: &str, open: Option<&'static str>) -> (String, Option<&'static str>) {
    let b = line.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    // A line that starts inside a block contributes no code at all until the
    // block ends, and the caller skips it whole either way — but the *state*
    // has to come out right, because a closing `"""` can be followed on the
    // same line by the opening of another one.
    if let Some(delim) = open {
        match triple_end(b, 0, delim.as_bytes()[0]) {
            Some(end) => i = end,
            None => return (out, Some(delim)),
        }
    }
    // Where the run of code not yet copied begins. Copied out in slices rather
    // than byte by byte, which is also what keeps this right on a line with
    // non-ASCII in it: every index below lands on a quote, a `#` or the end of
    // the line, and all three are ASCII.
    let mut from = i;
    while i < b.len() {
        let quote = match b[i] {
            // Everything after it is prose. The parity count read a `"""` in a
            // comment as a delimiter too — it counted `raw` and only then asked
            // whether the line was a comment — which is the second of the two
            // ways it opened a string that was never there.
            b'#' => break,
            c @ (b'"' | b'\'') => c,
            _ => {
                i += 1;
                continue;
            }
        };
        out.push_str(&line[from..i]);
        if i + 2 < b.len() && b[i + 1] == quote && b[i + 2] == quote {
            match triple_end(b, i + 3, quote) {
                Some(end) => i = end,
                None => {
                    let delim = if quote == b'"' { "\"\"\"" } else { "'''" };
                    return (out, Some(delim));
                }
            }
        } else {
            i = quoted_end(b, i + 1, quote);
        }
        from = i;
    }
    out.push_str(&line[from..i]);
    (out, None)
}

/// Just past the `qqq` that closes a triple-quoted block, or `None` if the line
/// ends inside it.
fn triple_end(b: &[u8], mut i: usize, quote: u8) -> Option<usize> {
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == quote && i + 2 < b.len() && b[i + 1] == quote && b[i + 2] == quote {
            return Some(i + 3);
        }
        i += 1;
    }
    None
}

/// Just past the quote that closes an ordinary `'...'` or `"..."`.
///
/// The end of the line when there is no closing quote. A single-quoted literal
/// cannot span a line on its own, so an unterminated one is a file that will
/// not compile rather than a state worth carrying to the next line — and eating
/// the rest of the line is the safe direction to be wrong in, because it can
/// only lose a row and never invent one.
fn quoted_end(b: &[u8], mut i: usize, quote: u8) -> usize {
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    b.len()
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
/// exist.
///
/// **How they are tracked is [`code_of`], and it is not a parity count.** It
/// was one, and the magnitude of what that got wrong is worth recording,
/// because the direction was right and "a run of the file is skipped" was far
/// too kind a description of it. Counting `"""`s on a line miscounts as soon as
/// one is written inside an ordinary quoted string, which is not exotic —
/// `{"'''"}`, `line.split('"""')[0]`, `for u in (t + '"""', t + "'''")` — and
/// the miscount does not lose a row. It opens a string that is never closed,
/// and loses **every definition below it**.
///
/// Swept against `ast` over CPython 3.12's `Lib` and its site-packages: 7362
/// files, 171,479 definitions. The parity count missed 1633 of them across 50
/// files, with `tomlkit/items.py` losing 290 of its 308 and `ast.py` 116 of its
/// 168. Taking the quoted strings out of the line first brings that to 108
/// misses in **one** file — `Lib/test/test_tokenize.py`, which is CPython's own
/// tokenizer test suite and is triple-quoted strings nested inside
/// triple-quoted strings for a thousand lines. A line scanner cannot read that
/// file, and the module doc says why it is not going to become the thing that
/// could. False positives are 2 either way.
fn python(text: &str) -> Vec<(usize, u8, String)> {
    let mut out = Vec::new();
    let mut depth = Depth::default();
    let mut open: Option<&'static str> = None;
    for (n, raw) in text.split('\n').enumerate() {
        // Whether the line *started* inside a string, which is the question,
        // rather than whether it ends inside one: the code in front of a
        // docstring's opening quotes is still code.
        let inside = open.is_some();
        let (code, still) = code_of(raw, open);
        open = still;
        if inside {
            continue;
        }
        // `code` and not `raw`, which is the second half of what `code_of`
        // buys: a `def` written inside a string on an otherwise ordinary line
        // is not a definition, and neither is one inside a `#` comment.
        let (indent, rest) = indented(&code);
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
/// Both halves run on **every frame**, because
/// [`Pane::title`](crate::pane::Pane::title) does — so what follows is about a
/// few hundred integer comparisons on the thread that pumps the agent's pty,
/// and both are cheap enough. It is worth being exact about *which* is which,
/// because a first draft of this comment had the emphasis backwards.
///
/// The binary search is the smaller half. It finds the innermost entry, and
/// over this repository's own `viewer.rs` — 223 definitions when this was
/// measured, which is three hundred-odd at
/// [`load::MAX_BYTES`](super::load::MAX_BYTES)'s density — that is eight
/// comparisons.
///
/// **The walk back is the larger half**, and it is linear in the entries passed
/// on the way rather than in the depth found. 187 of those 223 are at one
/// level — five in six — so a title drawn near the end of the file walks most
/// of the list to find the one item above them at a lower one. That is still a
/// couple of
/// hundred integer comparisons over a `Vec` that is already in cache, which is
/// why it is left alone: bounding it means a parent index on every [`Entry`],
/// one more field that has to be rebuilt correctly with the rows every single
/// time, and the thing it would buy is not measurable next to the layout that
/// produced those rows.
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
    // Levels are dropped one at a time and the **finished string** is measured
    // each time, marker and joiners included. A running total is what the first
    // version kept, and it is why a cap of thirty could return thirty-four
    // cells: the loop broke *before* the `…` and its joiner were pushed, so the
    // four cells they cost were never in the budget they overran.
    //
    // One string built per level of the document, and six is as deep as
    // markdown goes — against a `Vec` and a running width that had to be right
    // about the same arithmetic in two places.
    for dropped in 0..chain.len() {
        let shown = chain[dropped..].join(CRUMB_JOIN);
        let text = if dropped == 0 {
            shown
        } else {
            format!("{CRUMB_MARK}{CRUMB_JOIN}{shown}")
        };
        if text.width() <= CRUMB_MAX {
            return Some(text);
        }
    }
    // Nothing left to drop to. The innermost is clipped rather than dropped,
    // because a breadcrumb with nothing in it is worse than a shortened one —
    // the reader would be told nothing rather than told most of it — and it
    // keeps the marker in front of it if there were levels above it to lose.
    let head = if rest.is_empty() {
        String::new()
    } else {
        format!("{CRUMB_MARK}{CRUMB_JOIN}")
    };
    let room = CRUMB_MAX.saturating_sub(head.width());
    Some(format!("{head}{}", clip(last, room)))
}

// --- the view ------------------------------------------------------------

/// What came of a key, in the vocabulary the pane needs. `Ignored` maps to
/// `Handled::No` for [`grep::Outcome`](super::grep::Outcome)'s reason: a key
/// that changed nothing must not cost a frame, and a frame here re-renders the
/// agent's whole screen.
pub enum Outcome {
    Ignored,
    Moved,
    /// `Esc` or a second `o`. The reader goes back to the document exactly
    /// where they left it.
    Leave,
    /// `Enter`. Put the document at this row and show it, unless it is already
    /// on screen — see [`View::key`].
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
/// It does not hold the palette either. `F3` and `set_theme` change it, and
/// `set_root` rebuilds `browse` and `grep` from scratch and so has to *re-apply*
/// it to both — three places that already have to remember, and a copy here
/// would be a fourth. The pane has the answer and passes it to the one method
/// that draws.
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
    /// With one place where there is no such row: above the first heading
    /// [`crumb`] names nothing, and this opens on the first entry instead —
    /// `saturating_sub` on a `partition_point` of zero. That is the right
    /// answer and not a fallback, because the first entry is the section the
    /// reader is about to scroll into, but it is the one case where "the entry
    /// the breadcrumb has been naming" is a sentence about nothing.
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
            // Unreachable, and the proof takes three steps rather than one, so
            // it is written down: `o` declines an empty outline; while this
            // view is up every key belongs to it, so `t`, `r` and a file the
            // watcher offers cannot change the document underneath it; and the
            // two calls that *can* replace the document from outside — `show`
            // and `set_root` — both leave this mode as they do it. A rebuild at
            // a new width is the one thing that does happen here, and it cannot
            // empty the list: `build` returns nothing only for a width of zero,
            // and a zero-width pane has already been returned from above.
            //
            // Drawn anyway, for the reason every other list in this pane draws
            // something: a blank pane is indistinguishable from a broken one,
            // and none of the three steps above is a guard *this* function can
            // see.
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

    /// One entry: indented by its level, pipped past the depth the indent can
    /// carry, and coloured by it.
    ///
    /// All three, because the first two together are what makes the level
    /// readable without hue and neither does it alone. `theme::Theme::heading`
    /// gives H4, H5 and H6 one colour on purpose, and [`MAX_STEPS`] stops the
    /// indent at four — so with the indent alone an H5 and an H6 are the same
    /// nine spaces in the same yellow, which is two levels rendered as one row
    /// and is exactly the regression `markdown::HEADING_PIPS` was added to fix
    /// one module along. The pip is that same fix, drawn from that same
    /// function: `▸ ▹ ▫` differ in *shape*, so the three levels the indent has
    /// run out of room for are still three. It also answers [`STEP`]'s own
    /// stated goal — an outline of a document looks like the document's own
    /// nesting — because these are the marks the document itself wears.
    ///
    /// The bold is taken off, which is the one place this departs from the
    /// document: a heading is bold there because it is one row among many that
    /// are not, and a list where every row is a heading has nothing left for
    /// the bold to contrast with.
    fn line(&self, entry: &Entry, selected: bool, w: usize, mode: theme::Mode) -> Line<'static> {
        let t = mode.theme();
        let steps = usize::from(entry.level).saturating_sub(1).min(MAX_STEPS);
        let mut spans = vec![Span::raw(" ".repeat(steps * STEP + 1))];
        if let Some(pip) = markdown::heading_pip(usize::from(entry.level)) {
            spans.push(Span::styled(pip, t.dim()));
        }
        spans.push(Span::styled(
            // Expanded here rather than trusted to be absent, because this is
            // the row and the clip below cannot save a row it mismeasures:
            // `text::clip` asks `UnicodeWidthChar::width`, which calls a `\t`
            // nothing at all, while `clip_spans` counts it as one — so `# a\tb`
            // in a heading is a row that measures under the pane and draws over
            // it. The breadcrumb takes the same text and is not treated the same
            // way, deliberately: a title is not a pre-wrapped row, and the
            // shell's border truncates it rather than letting it run into the
            // next cell.
            //
            // The stops are the name's own rather than the row's, which is off
            // by the indent in front of it. That is a cosmetic difference and
            // the alternative is worse: measuring from the row would break the
            // same heading in two places depending on how deep it is nested.
            expand_tabs(&entry.text),
            t.heading(entry.level).remove_modifier(Modifier::BOLD),
        ));
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
    /// Half of `/`'s answer is mechanical: `/` in this pane means "find a
    /// phrase in the rows on screen", and its hits are indexed into `lines` —
    /// the *document's* rows, which are not what is on screen here. So it would
    /// have to be a filter box over these rows instead, in the shape the file
    /// list and the results already have.
    ///
    /// This used to say a fourth matcher is "precisely the fourth copy
    /// [`super::list`] exists to prevent", and that is not what that module
    /// says. It exists to stop a fourth [`Cursor`] being written by hand; where
    /// it discusses the duplicated *box* code it declines to unify it and says
    /// why, and `docs/design.md` argues at length that the three matchers are
    /// deliberately different from each other. None of that is an argument
    /// against a fourth.
    ///
    /// The real reason is the one `ViewerPane::exit_hint` half-writes.
    /// This is the only layer in the pane with a single unconditional way out:
    /// `Esc` here means "back to the page", in every state this view can be in,
    /// which is what lets the border say so without a condition. A filter box
    /// would put a stage in front of that — `Esc` to close the box, `Esc` again
    /// to leave — inside a layer whose whole promise is that backing out of it
    /// costs nothing and needs no looking. The pane has three `Esc` ladders
    /// already; a fourth, in a view opened by one keystroke over the document,
    /// is where they stop being learnable.
    ///
    /// **It is not free, and the cost is worth writing down rather than
    /// arguing away.** Over this repository's own `viewer.rs` the outline is
    /// 223 entries when this was measured, 187 of them at one level; at the 46
    /// columns this pane is routinely given a level-two name has 42 cells
    /// before it is cut; and six consecutive rows of it share the prefix
    /// `fn a_doc`. `docs/status.md` records that as a known gap.
    ///
    /// `f` is declined for a narrower reason, and the reason is
    /// [`super::Mode::Outline`]'s rather than `super::Back`'s — `Back` says
    /// only that there are exactly two answers, and it is `Mode::Outline`'s own
    /// doc that argues why this view is not a third. `f` would have to come
    /// back to somewhere, and a third answer would make the results reachable
    /// from a view that is itself a layer over the document, which is the
    /// `Results { back: Results }` shape `Back` exists to make unrepresentable.
    /// `Esc` and then `f` is two keys, and it is the same two keys the file list
    /// already needs to reach a document.
    ///
    /// What is left is the list vocabulary, `Enter`, and the two ways out.
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
            // Both mean "never mind", and never mind does not cost you your
            // place: `o` because undoing the keystroke that opened a thing is
            // the rule every box in this pane follows, `Esc` because this is a
            // layer over the document and `Esc` is what closes a layer here —
            // the same staging the search's `Esc` already has.
            //
            // **`q` is deliberately not in this arm**, and it is the one key
            // here worth arguing rather than stating. It falls to
            // `Outcome::Ignored`, the pane hands the shell a `Handled::No`, and
            // the shell reads that as "give focus back to the agent" — which is
            // what `q` means in the document, in the file list and in the
            // results. The git pane's worktree list is this view's shape
            // exactly: a list layer over a settled view, opened by a bare
            // letter, closed by `Esc` or by that letter again — and
            // `GitPane::worktree_key` says in as many words that `q` there
            // "means what it has always meant, which is back to the agent".
            // Claiming it here would make the outline a fifth entry in the
            // exhaustive list `crate::keys`'s `Esc or q` row carries, and the
            // only thing that makes that list worth having is that it is short.
            // Three of its four entries are places where `q` is a letter
            // somebody is typing — a shell, a find box, the ask's composer —
            // and the fourth, the worktree list, does not claim `q` at all. An
            // outline that claimed it would be the first entry on that list
            // with nothing being typed behind it.
            KeyCode::Esc | KeyCode::Char('o') => Outcome::Leave,
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
    fn a_language_this_does_not_read_has_no_outline_and_markdown_has_one_either_way() {
        let rust = "fn main() {}\n";
        assert!(found(rust, "notes.txt").is_empty());
        assert!(found(rust, "Makefile").is_empty());
        // The recorded decision, and it has changed: a `.md` has an outline in
        // both of its forms, because the source is parsed by the renderer's own
        // parser rather than scanned for `#` lines. The module doc says what was
        // refused and what was not.
        assert_eq!(
            found("# Heading\n\nbody\n", "design.md"),
            [(1, "Heading".into())]
        );
        // And the extension list is `docs::language`'s rather than a second
        // copy, so a `.pyi` is Python here exactly as it is there. This is the
        // assertion the copy could not have: it fails the moment the two lists
        // are two lists again and only one of them learns a language.
        assert_eq!(found("class A:\n", "stub.pyi"), [(1, "class A".into())]);
    }

    #[test]
    fn a_markdown_outline_is_the_renderers_own_answer_read_off_the_source() {
        // The whole of why this is not a `#`-line scanner. Four of the six
        // `#`-looking lines below are not headings and one heading has no `#`
        // at all, which is the disagreement a scanner would have had with the
        // rendering of the same file.
        //
        // The assertion is deliberately *not* against a list written out by
        // hand for the second half of it: it is against `render_outlined`, the
        // renderer's answer for the same document, so a change that made the
        // two forms of one file list different tables of its contents would
        // fail here rather than be noticed by a reader.
        let md = concat!(
            "# Title\n",
            "\n",
            "```\n",
            "# not a heading\n",
            "```\n",
            "\n",
            "    # nor is this\n",
            "\n",
            "Setext\n",
            "======\n",
            "\n",
            "<!-- # nor this -->\n",
            "\n",
            "## Real `two`\n",
        );
        let listed = symbols(md, Path::new("design.md"));
        assert_eq!(
            listed,
            [
                (0, 1, "Title".into()),
                (8, 1, "Setext".into()),
                (13, 2, "Real two".into()),
            ]
        );
        // Source *lines*, and the renderer's are *rows* — the two numbers are
        // different and have to be, which is why only the names and the levels
        // are held against each other.
        let (_, drawn) = markdown::render_outlined(md, 40, theme::Mode::Dark);
        let theirs: Vec<(u8, &str)> = drawn.iter().map(|e| (e.level, e.text.as_str())).collect();
        let ours: Vec<(u8, &str)> = listed.iter().map(|(_, l, t)| (*l, t.as_str())).collect();
        assert_eq!(ours, theirs);

        // Where the two are *allowed* to differ, pinned so that the rule is a
        // rule rather than a thing nobody noticed. Of 4001 real markdown files
        // swept off this machine's disk, 829 have a badge or a link in a
        // heading, and there the renderer's label carries what it drew — the
        // image glyph, the destination elided against a width this function is
        // not given. The words are the same words and the level is the same
        // level in all 4001, which is the half a `#`-line scanner would have got
        // wrong.
        let linked = "# [Foo](https://example.com/a/b) ![bar](x.png)\n";
        assert_eq!(
            symbols(linked, Path::new("r.md")),
            [(0, 1, "Foo bar".into())]
        );
        let (_, decorated) = markdown::render_outlined(linked, 40, theme::Mode::Dark);
        assert_eq!(decorated.len(), 1);
        assert_eq!(decorated[0].level, 1);
        assert!(decorated[0].text.contains("Foo"), "{:?}", decorated[0].text);
        assert!(decorated[0].text.contains('▨'), "{:?}", decorated[0].text);
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
        // A chain inside the cap is joined whole and marked with nothing.
        assert_eq!(crumb(&entries, 45).unwrap(), "Design › Keys");
    }

    #[test]
    fn a_long_breadcrumb_is_elided_from_its_outer_end() {
        let entries = vec![
            entry(0, 1, "Design"),
            entry(10, 2, "The panes"),
            entry(20, 3, "ask"),
        ];
        let shown = crumb(&entries, 25).unwrap();
        // The innermost survives whole, because it is the one the reader cannot
        // otherwise name; the outer end is what goes, marked.
        assert_eq!(shown, "… › ask");
        assert!(!shown.contains("Design"), "{shown}");
        // And a chain that fits is not marked at all.
        assert!(!crumb(&[entry(0, 1, "Short")], 5).unwrap().contains('…'));
    }

    #[test]
    fn a_breadcrumb_never_costs_more_cells_than_its_own_cap() {
        // The cap is on the **finished string**, marker and joiners included.
        // The first version kept a running total and broke out of its loop
        // *before* pushing the `…` and the joiner carrying it, so a cap of
        // thirty produced thirty-four cells and the constant did not mean what
        // its own doc said. Swept over every depth markdown has and every name
        // length either side of the cap.
        for depth in 1..8usize {
            for len in 1..40usize {
                let entries: Vec<Entry> = (0..depth)
                    .map(|i| entry(i * 10, i as u8 + 1, &"n".repeat(len)))
                    .collect();
                let shown = crumb(&entries, depth * 10).expect("a crumb");
                assert!(
                    shown.width() <= CRUMB_MAX,
                    "{shown:?} is {} cells for depth {depth}, names of {len}",
                    shown.width()
                );
            }
        }
        // And the marker appears only when a level was actually dropped —
        // otherwise the leading `…` would be a claim about nothing.
        let pair = [entry(0, 1, "a"), entry(1, 2, "b")];
        assert_eq!(crumb(&pair, 2).unwrap(), "a › b");
    }

    #[test]
    fn no_row_of_the_list_is_ever_wider_than_the_pane_it_was_drawn_for() {
        // The invariant every view in this pane owes: a row that overflows its
        // rect corrupts the frame rather than merely looking wrong. Swept over
        // every width down to nothing, because a pane is dragged through all of
        // them, and over both the plain and the selected row — the selected one
        // is padded out to the full width *after* it is clipped, which is the
        // step that could put a cell back past the edge.
        //
        // **There is a tab in two of the fixtures and it is the point of them.**
        // `text::clip` asks `UnicodeWidthChar::width`, which calls a `\t`
        // nothing at all, while `clip_spans` counts the span it is in as one
        // cell per character — so an unexpanded tab is a row that measures under
        // the pane and draws over it. `# a\tb\tc` at width five came out seven
        // cells wide. A heading is the only entry that can carry one: a source
        // symbol has been through `indented`, which expands them.
        let view = View::new(10);
        let deep = entry(0, 9, "a definition\twith a name far too long for this");
        let tabbed = entry(0, 1, "a\tb\tc");
        for e in [&deep, &tabbed, &entry(0, 1, "short")] {
            for w in 0..40 {
                for selected in [false, true] {
                    let line = view.line(e, selected, w, theme::Mode::Dark);
                    let drawn: usize = line.spans.iter().map(|s| s.content.width()).sum();
                    assert!(drawn <= w, "{drawn} cells drawn in {w} columns for {e:?}");
                    assert!(
                        !line.spans.iter().any(|s| s.content.contains('\t')),
                        "a tab reached the row"
                    );
                }
            }
        }
    }

    #[test]
    fn the_deepest_levels_are_told_apart_by_shape_and_not_only_by_hue() {
        // `Theme::heading` hands H4, H5 and H6 one colour on purpose, and
        // `MAX_STEPS` stops the indent at four steps — so with the indent alone
        // an H5 and an H6 were the same nine spaces in the same yellow. Two
        // levels rendered as one row is exactly the regression
        // `markdown::HEADING_PIPS` was added to fix in the document, and the
        // outline of that document owes the same thing.
        let view = View::new(10);
        let drawn = |level: u8| -> String {
            view.line(&entry(0, level, "Name"), false, 46, theme::Mode::Dark)
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        };
        let rows: Vec<String> = (1..=6).map(drawn).collect();
        for (i, a) in rows.iter().enumerate() {
            for (j, b) in rows.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "levels {} and {} draw the same row", i + 1, j + 1);
            }
        }
        // And the shapes are the document's own rather than a second
        // convention, which is what `STEP` says the indent is for.
        for level in 4..=6usize {
            let pip = markdown::heading_pip(level).expect("the document's own mark");
            assert!(rows[level - 1].contains(pip), "{:?}", rows[level - 1]);
        }
    }

    #[test]
    fn a_triple_quote_written_inside_an_ordinary_string_does_not_swallow_the_file() {
        // Not a corner. The three lines below are `tomlkit/items.py`, `ast.py`
        // and `pdb.py` respectively, and a parity count over the delimiters read
        // every one of them as *opening* a docstring — which then never closed,
        // so every definition under it was dropped with nothing on screen to say
        // rows were missing. `tomlkit/items.py` lost 290 of its 308.
        let src = concat!(
            "ESCAPES = {\"'''\"}\n",
            "def one():\n",
            "    pass\n",
            "for u in (t + '\"\"\"', t + \"'''\"):\n",
            "    pass\n",
            "def two():\n",
            "    head = line.split('\"\"\"')[0]\n",
            "def three():\n",
            "    pass\n",
        );
        assert_eq!(
            found(src, "a.py"),
            [
                (1, "def one".into()),
                (1, "def two".into()),
                (1, "def three".into()),
            ]
        );
        // And the thing the counting was there for still works: a real
        // docstring is still tracked, and a `#` comment holding a `"""` no
        // longer opens one.
        let doc = concat!(
            "class Store:\n",
            "    # a \"\"\" written in a comment\n",
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
            found(doc, "s.py"),
            [(1, "class Store".into()), (2, "def real".into())]
        );
        // An escaped quote does not end a block either, which is the one case
        // `find` alone would have got wrong.
        let escaped = concat!(
            "x = \"\"\"a \\\"\"\" still open\n",
            "def hidden():\n",
            "    pass\n",
            "\"\"\"\n",
            "def shown():\n",
            "    pass\n",
        );
        assert_eq!(found(escaped, "e.py"), [(1, "def shown".into())]);
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
