//! Finding a string in the document that is on screen.
//!
//! Not to be confused with the find in [`super::browse`], which reaches a
//! *file* anywhere under the root. This reaches a *place* in the one document
//! the reader already has open, and the two want opposite things from a
//! matcher: a path is matched as a subsequence, because typing `capv` to reach
//! `crates/abeam/src/panes/viewer.rs` is how anyone who has used a fuzzy finder
//! expects to get there. Prose is not. A subsequence over a paragraph matches
//! nearly every paragraph, so the answer is every row and the reader has been
//! handed noise dressed as results. Plain substring here, and smart case:
//! case-insensitive until the query contains a capital, which is the one
//! convention that needs no key to turn it on and no explanation when it fires.
//!
//! No regular expressions, and the honest reason is the box rather than the
//! manifest: `fancy-regex` is already linked into this binary behind syntect's
//! `default-fancy`, so "it would be a dependency" is an argument abeam cannot
//! make here — `Cargo.toml`'s `libc` note draws exactly that distinction.
//!
//! The box is the reason. This runs on every keystroke, so half of every
//! pattern ever typed is a broken one — `[a-`, `(foo` — and a matcher that can
//! fail needs somewhere to say it failed. A one-line title already carrying the
//! file name, the form, the query and the count is not that somewhere, and a
//! search that silently found nothing while a bracket was open would be
//! indistinguishable from one that worked. Backtracking has no useful bound
//! over half a megabyte either, on the thread that pumps the agent's pty.
//!
//! ## What is searched, and why it is the rows
//!
//! [`ViewerPane::lines`](super::ViewerPane) is the document already wrapped to
//! the pane's exact width and styled. This searches *that*, and reports hits as
//! `(row, character, length)` — the row being the unit the pane already scrolls
//! by, so a hit needs no translation to be scrolled to or drawn on.
//!
//! The alternative — searching the source text and mapping offsets back — works
//! for a `.rs` file and cannot work at all for rendered markdown. Rendering
//! reflows prose, drops fence markers and turns a table into a grid, so there
//! is no offset that means the same thing on both sides; the same argument
//! `ViewerPane::toggle_raw` already makes about the scroll position. Searching
//! the rows is the one approach that is identical for rendered markdown, raw
//! markdown and a highlighted source file without the markdown renderer having
//! to grow a second output. It is also the honest semantic for a pane whose
//! whole job is to show you a document: you find what is on screen.
//!
//! Two consequences, neither hidden:
//!
//! - **A match that straddles a wrap is missed.** `parse` split as `par` /
//!   `se` across two rows is two rows, and nothing here joins them. The cost is
//!   a hit the reader can see and the search cannot, which is the failure mode
//!   worth having: it is visible and it is one `n` away from the next real hit,
//!   where a phantom hit at a row-join would be neither.
//! - **In rendered markdown you cannot find what was rendered away** — `**`,
//!   the URL behind a link, the backticks around code. They are not on screen.
//!   `t` shows the source, and the title says which form is up, which is why
//!   `· rendered` is the last thing the title gives up while a search is open.
//!
//! And one place the rule is bent, because following it there produced a worse
//! one. A source file's line numbers are on screen, so `/42` would find every
//! line whose *number* contains a 42. They are skipped — see [`Margin`] — on
//! the grounds that the gutter is the **pane's** margin and not the document's
//! text. What settles it is that leaving them in would not have been a rule
//! anybody could see: `source_lines` draws no gutter below
//! `LINE_NUMBER_MIN_WIDTH` columns and rendered markdown has none at all, so
//! `/42` would already have found different things in the two forms of one file
//! and at 29 columns versus 31, with nothing on screen explaining either.
//! "The pane's own margin is not part of the document" is visible from the
//! screen; "sometimes the margin is searchable" is not.
//!
//! ## Characters, not cells and not bytes
//!
//! `crate::text` measures in terminal cells throughout, because everything it
//! does is deciding what fits. A hit is not that: it is a position, and the row
//! it is in is walked by character. `crate::text::restyle` takes the same unit
//! for the same reason, and carries the argument for why the pair must not be
//! cells.
//!
//! The comparison is per character rather than over a lowercased copy of the
//! row, and that is the one place this departs from `browse::rank` next door.
//! `rank` lowercases the whole path and takes its offsets from *that* string,
//! precisely because `char::to_lowercase` is not length-preserving. Here the
//! offsets have to index the row as drawn, so the row cannot be rewritten
//! before it is searched, and the folding moves into the comparison instead.

use ratatui::text::Line;

use crate::pane::Handled;

/// One match, in the units the pane already scrolls and draws in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Which laid-out row. Invalid the moment `lines` is rebuilt, which is why
    /// [`Search::find`] is wired to the rebuild as well as to the keystroke.
    pub row: usize,
    /// Characters into that row's text, spans ignored.
    pub start: usize,
    pub len: usize,
}

/// The pane's own left margin on the rows it has just laid out: how wide it is,
/// and how many rows from the top carry it. [`matches`] starts each of those
/// rows past it, while still reporting offsets from the start of the row so
/// that `crate::text::restyle` needs to know nothing about any of this.
///
/// Both halves are needed and neither is a guess. `source_lines` draws a
/// line-number gutter only at or above `LINE_NUMBER_MIN_WIDTH` columns, and
/// only on the rows of the file itself — the truncation notice underneath it
/// has none, and skipping four columns of `— stopped at …` would be skipping
/// the document. Rendered markdown, the empty screen and the unreadable notice
/// have no margin at all and say so with a zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margin {
    pub width: usize,
    pub rows: usize,
}

/// A search over the document, and the box it is typed into.
///
/// One type for two states rather than two, because they are the same search:
/// `Enter` closes the box and keeps everything else, so a reader who has
/// accepted a query still has hits to step through and a count in the title. The
/// pane distinguishes them with [`Search::typing`], and that predicate is what
/// `Pane::takes_input` and the three-stage `Esc` are both answered from — one
/// fact rather than two that can disagree.
pub struct Search {
    query: String,
    /// The box is open, so every printable key is a letter of the query.
    typing: bool,
    hits: Vec<Hit>,
    /// Which hit `n` and `N` move from. Meaningless while `hits` is empty, and
    /// clamped into range by every method that can shorten the list.
    at: usize,
    /// The row a re-filter aims from: where the reader was when they opened the
    /// box, and wherever they have moved the view to since.
    ///
    /// Not the current scroll offset, which is the obvious choice and wrong.
    /// Each keystroke scrolls the view to its own answer, so aiming from the
    /// offset would aim the *next* keystroke from that answer — and since the
    /// jump leaves the hit part-way down the page, the hit above it becomes the
    /// first one at or after the top. The query would walk backwards through
    /// the document, half a page per letter typed.
    anchor: usize,
    /// The hit under `at` has moved, or the rows beneath it were rebuilt. Read
    /// and cleared by the pane on the next frame, which is the only place that
    /// knows how tall the pane is and so the only place that can decide whether
    /// the hit needs scrolling to. The same shape as `list::Cursor`'s `follow`,
    /// and for the same reason.
    follow: bool,
}

impl Search {
    /// Open the box. `anchor` is the row the reader is on, which is where the
    /// first hit is looked for — searching from the top of a document somebody
    /// is forty pages into is a search for a different question.
    pub fn open(anchor: usize) -> Self {
        Self {
            query: String::new(),
            typing: true,
            hits: Vec::new(),
            at: 0,
            anchor,
            follow: false,
        }
    }

    pub fn typing(&self) -> bool {
        self.typing
    }

    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// The hit `n` and `N` move from, and the one drawn in the second colour.
    pub fn current(&self) -> Option<Hit> {
        self.hits.get(self.at).copied()
    }

    /// Close the box and keep everything else. `Enter`, and also `Esc` — the
    /// two differ in what they say on the border, not in what they do here.
    pub fn accept(&mut self) {
        self.typing = false;
    }

    /// The reader moved the view themselves, so the next keystroke should look
    /// for its hit from here rather than from wherever they started.
    pub fn set_anchor(&mut self, row: usize) {
        self.anchor = row;
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
    }

    /// Take a character back, reporting whether there was one. `false` is
    /// Backspace at an empty query, which closes the box — those keystrokes
    /// came from opening it, so undoing the last should undo the first. The
    /// same rule `browse.rs` follows, so the two boxes cannot drift.
    pub fn pop(&mut self) -> bool {
        self.query.pop().is_some()
    }

    /// Recompute the hits against the rows as they are now laid out.
    ///
    /// Called on every keystroke *and* on every rebuild of those rows — a width
    /// change, `t`, `F3`, a reload — because a hit is a row index and a rebuild
    /// makes every one of them a guess about a document that no longer exists.
    ///
    /// It re-finds and nothing else, and pointedly does not ask for the reader
    /// to be taken anywhere. A rebuild is not something they did: an agent
    /// saving the file, `r`, `F3` and a window drag all arrive here, and a
    /// `find` that armed the reveal would drag somebody who had read fifty rows
    /// on back to a match they had finished with — on a pane they need not even
    /// be focused on, which is the invariant `super`'s module doc opens with.
    /// `toggle_raw` is the sharpest of them: it calls this itself, in order to
    /// carry the reader's place across as a fraction, and would have had its
    /// own answer thrown away by the next frame. Arming the reveal belongs to
    /// [`Search::aim`] and [`Search::step`], which is to say to the reader —
    /// and every route driven by one ends in one of those two.
    ///
    /// What it does keep is which match the reader was on, by *ordinal*: the
    /// third match is still the third match. That is exact when only the
    /// styling changed, and an approximation when the wrapping did — re-wrapping
    /// moves boundaries, so a narrower pane can destroy a match that used to
    /// fit on one row and leave the reader one match early. Which is the same
    /// cost the module doc opens with, arriving from the other side, and still
    /// the closest thing to a position that survives the rows being rebuilt.
    pub fn find(&mut self, lines: &[Line<'_>], margin: Margin) {
        self.hits = matches(lines, &self.query, margin);
        self.at = self.at.min(self.hits.len().saturating_sub(1));
    }

    /// Point at the first hit at or after the anchor, wrapping to the first hit
    /// in the document when there is none below it.
    ///
    /// One of the two places the reveal is armed. Every route the reader drives
    /// — a letter, a Backspace, a paste — ends here, which is what lets
    /// [`Search::find`] stay silent on every route they did not.
    pub fn aim(&mut self) {
        self.at = self
            .hits
            .iter()
            .position(|h| h.row >= self.anchor)
            .unwrap_or(0);
        self.follow = true;
    }

    /// `n` and `N`. Wrapping, because the alternative is a key that dies at the
    /// last match and a reader who cannot tell that from a key that broke.
    pub fn step(&mut self, forward: bool) -> Handled {
        let n = self.hits.len();
        if n == 0 {
            return Handled::No;
        }
        self.at = if forward {
            (self.at + 1) % n
        } else {
            (self.at + n - 1) % n
        };
        self.follow = true;
        Handled::Yes
    }

    /// Whether the hit needs bringing on screen, asked once and answered once.
    pub fn take_follow(&mut self) -> bool {
        std::mem::take(&mut self.follow)
    }

    /// What the title says about this search.
    ///
    /// The query first behind the `/` that opened it, then where the reader is
    /// among the hits — the same order and the same argument as the file list's
    /// title: a title is clipped from the right, and of the two the count is
    /// the part that can be spared.
    ///
    /// An empty query is `/` alone, which is the box saying it is open and
    /// nothing more; a count of zero would be an answer to a question nobody
    /// has asked yet. A query that matches nothing says so in words. `0/0` is
    /// the same fact and reads as a bug in the counter.
    ///
    /// `miss` is appended to that miss and nowhere else, so the caller can put
    /// the answer where the question is being asked: in rendered markdown the
    /// commonest reason for finding nothing is that the thing being looked for
    /// was rendered away, and the caller is the only one that knows the body is
    /// markdown. It costs the same columns as the `· rendered` sitting further
    /// left, and it only ever appears when the columns are doing nothing else.
    pub fn label(&self, miss: &str) -> String {
        if self.query.is_empty() {
            return "/".to_string();
        }
        match self.hits.len() {
            0 => format!("/{} · no match{miss}", self.query),
            n => format!("/{} · {}/{n}", self.query, self.at + 1),
        }
    }
}

/// Every match of `query` in `lines`, in reading order.
///
/// Non-overlapping, scanning forward: `aa` in `aaaa` is two hits and not three.
/// Overlapping matches would put two highlights on the same cells and make the
/// count something a reader could not check by eye.
///
/// The cost is the document's characters times the query's, once per keystroke,
/// and it is not nothing. Measured in a release build over half a megabyte of
/// prose — the cap `load::MAX_BYTES` sets — wrapped to 78 columns: a
/// one-character `e` takes **8 ms** and finds 45,874 hits, `quick` 8 ms, a
/// 25-character phrase 4 ms. Short queries cost the most, which is the opposite
/// of the intuition: the inner loop exits sooner but is entered at nearly every
/// character of the document. The adversarial shape — half a megabyte of `a`
/// queried with `aaa…ab`, where that loop runs to its last character and then
/// fails — is 13 ms.
///
/// A debug build is **ten times** every one of those, 187 ms at the worst,
/// which is the number to remember before concluding that a `cargo run` of
/// abeam feels broken.
///
/// That is spent on the thread that pumps the agent's pty, so what it is
/// measured against matters: re-laying the same document out is ~210 ms, and a
/// keystroke in the box does this *instead* of that rather than as well as it —
/// which is the whole reason the search is wired to the rows and does no
/// parsing of its own. Drawing the hits is separate again and does not scale
/// with the count at all: `ViewerPane::highlight` binary-searches this vector
/// rather than walking all 45,874 of them to paint the dozen on screen.
fn matches(lines: &[Line<'_>], query: &str, margin: Margin) -> Vec<Hit> {
    let needle: Vec<char> = query.chars().collect();
    if needle.is_empty() {
        // Not "every position in the document". An empty query is a box that
        // has been opened and not yet used, and the reader has asked for
        // nothing.
        return Vec::new();
    }
    // Smart case. One capital anywhere in the query is the reader saying they
    // meant that capital: `Plan` finds the heading and not the fifty mentions
    // of `plan`, and nothing had to be turned on to get it.
    let fold = !needle.iter().any(|c| c.is_uppercase());

    let mut out = Vec::new();
    let mut hay: Vec<char> = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        hay.clear();
        hay.extend(line.spans.iter().flat_map(|s| s.content.chars()));
        // Past the pane's own margin, on the rows that have one. The offsets
        // pushed below are still counted from the start of the row, because
        // that is what draws the highlight.
        let from = if row < margin.rows { margin.width } else { 0 };
        if hay.len() < from + needle.len() {
            continue;
        }
        let mut at = from;
        while at + needle.len() <= hay.len() {
            if (0..needle.len()).all(|i| same(hay[at + i], needle[i], fold)) {
                out.push(Hit {
                    row,
                    start: at,
                    len: needle.len(),
                });
                at += needle.len();
            } else {
                at += 1;
            }
        }
    }
    out
}

fn same(a: char, b: char, fold: bool) -> bool {
    a == b || (fold && a.to_lowercase().eq(b.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn rows(text: &[&str]) -> Vec<Line<'static>> {
        text.iter()
            .map(|s| Line::from(vec![Span::raw((*s).to_string())]))
            .collect()
    }

    /// Where the reader is, spelled out: row, character, length.
    fn place(s: &Search) -> Option<(usize, usize, usize)> {
        s.current().map(|h| (h.row, h.start, h.len))
    }

    fn found(lines: &[Line<'static>], query: &str) -> Vec<(usize, usize)> {
        matches(lines, query, Margin::default())
            .into_iter()
            .map(|h| (h.row, h.start))
            .collect()
    }

    #[test]
    fn a_substring_is_found_wherever_it_sits_in_a_row() {
        let lines = rows(&["parse the thing", "nothing here", "reparse it"]);
        assert_eq!(found(&lines, "parse"), [(0, 0), (2, 2)]);
        assert_eq!(found(&lines, "zzz"), []);
        // An empty query is a box nobody has typed into, not a match on every
        // position in the document.
        assert_eq!(found(&lines, ""), []);
    }

    #[test]
    fn a_query_in_lower_case_ignores_case_and_one_capital_stops_it() {
        let lines = rows(&["Plan for the plan", "PLAN"]);
        assert_eq!(found(&lines, "plan"), [(0, 0), (0, 13), (1, 0)]);
        assert_eq!(found(&lines, "Plan"), [(0, 0)], "the capital was meant");
        assert_eq!(found(&lines, "PLAN"), [(1, 0)]);
    }

    #[test]
    fn matches_do_not_overlap_so_the_count_is_one_a_reader_can_check() {
        let lines = rows(&["aaaa"]);
        assert_eq!(found(&lines, "aa"), [(0, 0), (0, 2)]);
    }

    #[test]
    fn a_hit_is_counted_in_characters_across_the_spans_of_its_row() {
        // What a row of rendered markdown is: several spans, one sentence. The
        // offset has to be into the sentence, or `text::restyle` paints the
        // wrong word.
        let line = Line::from(vec![
            Span::raw("do ".to_string()),
            Span::raw("the ".to_string()),
            Span::raw("thing".to_string()),
        ]);
        assert_eq!(found(&[line], "the th"), [(0, 3)]);

        // ...and characters, not bytes: two ideographs before the match are two
        // characters and four cells, and the byte count is six.
        let line = Line::from(vec![Span::raw("設計parse".to_string())]);
        assert_eq!(found(&[line], "parse"), [(0, 2)]);
    }

    #[test]
    fn a_match_split_across_a_wrap_is_missed_and_that_is_the_documented_cost() {
        // The one thing searching the laid-out rows gives up. Pinned rather
        // than left to be rediscovered as a bug: a reader can see the hit the
        // search cannot, and the module doc owes them that in writing.
        let lines = rows(&["the par", "se step"]);
        assert_eq!(found(&lines, "parse"), []);
    }

    #[test]
    fn the_panes_own_margin_is_not_part_of_the_document() {
        // `/42` must not find line 42's number. The rows below are what
        // `source_lines` draws: a four-column gutter on the file's own rows and
        // none on the truncation notice under them.
        let lines = rows(&[" 42 let x = 42;", "  5 let y = 5;", "— stopped at 42 —"]);
        let margin = Margin { width: 4, rows: 2 };
        let skipped: Vec<(usize, usize)> = matches(&lines, "42", margin)
            .into_iter()
            .map(|h| (h.row, h.start))
            .collect();
        assert_eq!(
            skipped,
            [(0, 12), (2, 13)],
            "the gutter was searched, or the notice was not"
        );

        // No gutter is not a narrower gutter: below `LINE_NUMBER_MIN_WIDTH` and
        // in rendered markdown there is no margin, and then there is nothing to
        // skip rather than four columns to skip anyway.
        assert_eq!(found(&lines, "42"), [(0, 1), (0, 12), (2, 13)]);
    }

    /// A search over a document of five rows, with a hit on rows 1 and 3, as a
    /// reader would have left it: found, then aimed.
    fn search(anchor: usize, query: &str) -> (Search, Vec<Line<'static>>) {
        let lines = rows(&["one", "hit here", "three", "hit again", "five"]);
        let mut s = Search::open(anchor);
        for c in query.chars() {
            s.push(c);
        }
        s.find(&lines, Margin::default());
        s.aim();
        (s, lines)
    }

    #[test]
    fn the_first_hit_is_the_one_at_or_after_where_the_reader_was() {
        let (s, _) = search(0, "hit");
        assert_eq!(s.current().map(|h| h.row), Some(1));

        let (s, _) = search(2, "hit");
        assert_eq!(s.current().map(|h| h.row), Some(3), "not back at the top");

        // ...and past the last hit it wraps, rather than reporting nothing in a
        // document that plainly contains the word.
        let (s, _) = search(4, "hit");
        assert_eq!(s.current().map(|h| h.row), Some(1));
    }

    #[test]
    fn n_and_shift_n_wrap_in_both_directions() {
        let (mut s, _) = search(0, "hit");
        assert_eq!(s.current().map(|h| h.row), Some(1));
        assert_eq!(s.step(true), Handled::Yes);
        assert_eq!(s.current().map(|h| h.row), Some(3));
        s.step(true);
        assert_eq!(s.current().map(|h| h.row), Some(1), "round the end");
        s.step(false);
        assert_eq!(s.current().map(|h| h.row), Some(3), "and back round it");

        // A key that cannot move must not claim it did: a frame here re-renders
        // the agent's whole screen.
        let (mut none, _) = search(0, "zzz");
        assert_eq!(none.step(true), Handled::No);
        assert_eq!(none.step(false), Handled::No);
    }

    #[test]
    fn re_finding_after_a_rewrap_keeps_the_reader_on_the_same_match() {
        let (mut s, _) = search(0, "hit");
        s.step(true);
        assert_eq!(s.current().map(|h| h.row), Some(3), "the second match");

        // The same document, wrapped narrower: every row has moved and the
        // second match is still the second match.
        let narrow = rows(&["one", "hit", "here", "three", "hit", "again", "five"]);
        s.find(&narrow, Margin::default());
        assert_eq!(place(&s), Some((4, 0, 3)));

        // ...and a rebuild that leaves fewer matches than the reader had
        // stepped to clamps rather than pointing past the end.
        s.find(&rows(&["hit"]), Margin::default());
        assert_eq!(place(&s), Some((0, 0, 3)));
    }

    #[test]
    fn the_title_says_which_query_and_where_in_it_and_admits_a_miss() {
        let mut fresh = Search::open(0);
        assert_eq!(fresh.label(""), "/", "an open box, nothing asked yet");
        fresh.find(&rows(&["anything"]), Margin::default());
        assert_eq!(fresh.label(""), "/");

        let (mut s, _) = search(0, "hit");
        assert_eq!(s.label(""), "/hit · 1/2");
        s.step(true);
        assert_eq!(s.label(""), "/hit · 2/2");

        let (miss, _) = search(0, "zzz");
        assert_eq!(miss.label(""), "/zzz · no match");
        // The caller's answer, on the miss and nowhere else.
        assert_eq!(
            miss.label(" · t for source"),
            "/zzz · no match · t for source"
        );
        assert_eq!(s.label(" · t for source"), "/hit · 2/2");
    }

    #[test]
    fn only_what_the_reader_did_asks_for_the_view_to_move() {
        // The pane brings the hit on screen only when this says to. A flag that
        // never cleared would drag the view back on every frame and take the
        // wheel away, which is the mistake `list::Cursor` documents at length.
        let (mut s, lines) = search(0, "hit");
        assert!(s.take_follow(), "the reader typed a query");
        assert!(!s.take_follow());
        s.step(true);
        assert!(s.take_follow(), "the reader pressed n");
        assert!(!s.take_follow());

        // And a *rebuild* is not something the reader did. An agent saving the
        // file, `r`, `F3` and a window drag all land in `find`, and a `find`
        // that armed this would take somebody who had read on to the end of the
        // document back to a match they had finished with — on a pane they need
        // not even be focused on.
        s.find(&lines, Margin::default());
        assert!(!s.take_follow(), "a rebuild moved the reader");
        s.find(&rows(&["hit", "hit", "hit"]), Margin::default());
        assert!(!s.take_follow(), "a re-wrap moved the reader");
    }

    #[test]
    fn backspacing_past_the_start_of_the_query_reports_that_it_could_not() {
        let mut s = Search::open(0);
        s.push('a');
        assert!(s.pop());
        assert!(!s.pop(), "the box has nothing left to take back");
    }
}
