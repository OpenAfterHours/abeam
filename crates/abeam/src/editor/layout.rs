//! The rows an edited document is drawn as, kept between frames and mended
//! rather than rebuilt when the text changes — and coloured only as far down
//! as anybody has looked.
//!
//! The pad used to lay itself out from nothing on every edit: join the lines,
//! highlight the whole text, wrap every line. At syntect's 370 KB/s that is
//! 125 ms a keystroke on a 46 KB README, which is a pane that visibly lags the
//! typing in it — and a files view that edits up to 512 KiB could not use it at
//! all. So the layout here keeps, for every logical line, what it was built
//! from and what it built: the line's text, its spans, the highlighter's state
//! after it, where it wraps, and its rows. Wrapping and colouring are then two
//! different jobs done on two different schedules.
//!
//! ## Wrapping is eager, and only what changed
//!
//! Every line is wrapped, always, because the row count is what the scroll
//! offset, the caret's row and a click are all measured in, and a layout that
//! did not know how many rows the document was could not say where anything
//! is. It is also cheap — a walk of each line's characters, with no regex in
//! it — so a 512 KiB file is wrapped whole when it opens.
//!
//! After an edit the new lines are compared with the ones the layout was built
//! from, from the top and from the bottom, and only what is between the two
//! runs of equal lines is wrapped again. Nothing tells the layout what an edit
//! did, deliberately: a second account of every change, kept by every edit and
//! by undo, is a second copy of the truth that the first edit to forget it
//! would make into a layout of some other document. Comparing is a walk of
//! string comparisons that stops at the first byte that differs.
//!
//! ## Colouring is lazy, and stops where nothing can differ
//!
//! Colour costs what wrapping does not: syntect is the 370 KB/s, and a line
//! cannot be coloured without the state the line above it left the grammar in.
//! So colour is done on demand, from the top, only as far down as the rows
//! somebody has asked to draw — plus [`LOOKAHEAD`] lines, so that a scroll of a
//! row or two is not a highlight call of its own — and the layout keeps a
//! watermark: every line above `valid` is coloured, and correctly, for the text
//! as it is. A line below it may be uncoloured, or coloured from a text that
//! has since changed, and is never drawn until colouring has reached it.
//!
//! An edit pulls the watermark up to the first line it changed, and colouring
//! starts again from there. Below the edit there is usually a long run of lines
//! whose text did not change and whose colours were right a moment ago, and
//! the layout keeps that run as a *chain*: lines whose cached states follow one
//! from another, and the state that went into the first of them. When colouring
//! arrives at the chain with the state it was entered with last time, every
//! line in it is right as it stands — each line's colours are a function of the
//! state entering it and its own text, and both are as they were — so the
//! watermark jumps to the end of the chain. `source::Resume` says why that
//! comparison is sound in syntect 5.3 and the one way it is conservative. When
//! the state is not the same, the first line of the chain is recoloured and
//! the chain starts one line further down, entered with the state that line
//! used to hand on. That is the whole of the incremental rule.
//!
//! What laziness adds is a bound on the cases where the rule does not save
//! anything. Typing inside a paragraph recolours one line, either way. Opening
//! a fence recolours every line below it, because every one of them really has
//! changed — and so does typing in a fence's info string, because syntect keeps
//! the opening line's text in the state for the closing fence to be matched
//! against, so `python` and `pytho` are different states for every line of the
//! block. Done eagerly, that was 200 ms a keystroke in a long code block. Done
//! lazily it is the lines on screen, and the rest are coloured if and when the
//! reader scrolls to them.
//!
//! A different width re-wraps every line and colours none of them again. A
//! different palette or grammar starts again from nothing, which is what `F3`
//! costs.
//!
//! ## Past the highlight cap
//!
//! Whether a document is coloured at all is the editor's decision, made once
//! when the text arrives and handed in here — see `Editor::past_highlight_cap`
//! — and never revisited by an edit. Above `source::HIGHLIGHT_MAX_BYTES` the
//! answer is no, which is the reader's rule and for the reader's reason: the
//! cap is a time budget, measured, for highlighting a document from the top.

use ratatui::text::{Line, Span};

use super::wrap::{breaks, cells, char_at, into_rows, plain_rows};
use crate::panes::viewer::source::{self, Grammar, HIGHLIGHT_MAX_BYTES, Resume};
use crate::panes::viewer::theme;

/// How many lines below the last row asked for are coloured with it, so that
/// scrolling a row at a time is not a highlight call per row. Small, because it
/// is paid on every edit whose colours reach the bottom of the screen — typing
/// in a fence's info string recolours the screen and this — and a scroll that
/// outruns it costs a line a row, which is nothing.
const LOOKAHEAD: usize = 8;

/// What the colours are made of: the palette, and the grammar if there is one.
/// A layout built for any other look is a layout of other colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub mode: theme::Mode,
    pub grammar: Option<Grammar>,
}

/// Whether a document of `bytes` is too large to colour. One predicate, so the
/// editor's decision and anything that reports it cannot disagree about where
/// the line is.
pub fn past_cap(bytes: usize) -> bool {
    bytes > HIGHLIGHT_MAX_BYTES
}

/// One row of the laid-out document, with what a gutter needs to number it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The logical line this row is part of, counted from zero.
    pub line: usize,
    /// Whether it is that line's first row, which is the row a line number
    /// goes on; the rows a line wrapped onto get a blank gutter instead.
    pub first: bool,
    pub text: Line<'static>,
}

/// One logical line as it was laid out.
struct Laid {
    /// The line this entry was built from, which is what the next edit is
    /// compared against.
    text: String,
    /// The colours, or empty while the line has not been coloured — in which
    /// case it is drawn plain, and it is never drawn before colouring has
    /// reached it unless the document is not coloured at all.
    spans: Vec<Span<'static>>,
    /// The highlighter's state after this line, when it was last coloured.
    /// Right for the lines above the watermark and along the chain, and
    /// meaningless anywhere else.
    after: Option<Resume>,
    /// The char index each of this line's rows begins at. Never empty, and
    /// `starts[0]` is always 0.
    starts: Vec<usize>,
    rows: Vec<Line<'static>>,
    /// The document row this line's first row is, so a click on a row can
    /// find the line it belongs to without re-walking the wrap.
    first: usize,
}

impl Laid {
    fn new(text: &str, width: usize) -> Self {
        let starts = breaks(text, width);
        Self {
            rows: plain_rows(text, &starts),
            text: text.to_string(),
            spans: Vec::new(),
            after: None,
            starts,
            first: 0,
        }
    }

    fn cut(&mut self) {
        self.rows = if self.spans.is_empty() {
            plain_rows(&self.text, &self.starts)
        } else {
            into_rows(&self.spans, &self.starts)
        };
    }
}

/// A run of lines below an edit whose colours were right before it: each
/// line's cached state follows from the one above, starting from `entering`.
/// See the module doc.
struct Chain {
    from: usize,
    to: usize,
    entering: Option<Resume>,
}

/// How much the last update and the frames after it did, for the tests that
/// pin "only the lines that changed" and "only the lines on screen".
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Touched {
    pub highlighted: usize,
    pub wrapped: usize,
}

/// Which of the three ways `patch` found a chain below an edit, counted over
/// the layout's life, for the test that must show each of them happened.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChainHits {
    /// The edit was inside the coloured part.
    pub inside: usize,
    /// It was below the coloured part and above a chain already waiting.
    pub kept: usize,
    /// It was inside a waiting chain, which was cut to the part below it.
    pub cut: usize,
}

/// A whole document's worth of [`Laid`], and what it was laid out for.
struct Built {
    width: usize,
    look: Look,
    /// Whether colour was given up — no grammar, or the editor said no.
    plain: bool,
    /// The editor revision this describes. See `Editor::click`.
    rev: u64,
    lines: Vec<Laid>,
    rows: usize,
    hl: Option<source::Lines>,
    /// Every line above this is coloured, and correctly.
    valid: usize,
    chain: Option<Chain>,
    #[cfg(test)]
    touched: Touched,
    #[cfg(test)]
    chain_hits: ChainHits,
}

#[derive(Default)]
pub struct Layout {
    built: Option<Built>,
}

impl Layout {
    /// Bring the layout up to date with `lines`: wrap what changed, and mark
    /// what can no longer be trusted to be coloured. See the module doc.
    pub fn update(&mut self, lines: &[String], rev: u64, width: usize, look: Look, colour: bool) {
        let plain = !colour || look.grammar.is_none();
        let reusable = self
            .built
            .as_ref()
            .is_some_and(|b| b.look == look && b.plain == plain);
        if !reusable {
            self.built = Some(Built::fresh(lines, width, look, plain, rev));
            return;
        }
        if let Some(b) = self.built.as_mut() {
            #[cfg(test)]
            {
                b.touched = Touched::default();
            }
            if b.rev != rev {
                b.patch(lines);
                b.rev = rev;
            }
            if b.width != width {
                b.width = width;
                b.rewrap();
            }
        }
    }

    /// The revision the layout in hand was built from, if there is one.
    pub fn rev(&self) -> Option<u64> {
        self.built.as_ref().map(|b| b.rev)
    }

    /// The width it was built for.
    pub fn width(&self) -> Option<usize> {
        self.built.as_ref().map(|b| b.width)
    }

    /// How many rows the whole document is.
    pub fn row_count(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.rows)
    }

    /// The document row line `line` begins on.
    pub fn first_row_of(&self, line: usize) -> Option<usize> {
        self.built.as_ref()?.lines.get(line).map(|l| l.first)
    }

    /// The rows from `from`, at most `n` of them, coloured — which is where the
    /// colouring happens, and why this takes `&mut`.
    pub fn rows(&mut self, from: usize, n: usize) -> Vec<Row> {
        let Some(b) = self.built.as_mut() else {
            return Vec::new();
        };
        if n == 0 || from >= b.rows {
            return Vec::new();
        }
        let last = b.line_at((from + n - 1).min(b.rows - 1));
        b.colour_through(last + LOOKAHEAD);

        let mut out = Vec::with_capacity(n);
        for (i, laid) in b.lines.iter().enumerate().skip(b.line_at(from)) {
            for (r, row) in laid.rows.iter().enumerate() {
                if out.len() == n {
                    return out;
                }
                if laid.first + r >= from {
                    out.push(Row {
                        line: i,
                        first: r == 0,
                        text: row.clone(),
                    });
                }
            }
        }
        out
    }

    /// Where the caret is in the laid-out document, as `(row, column in
    /// cells)`. See `Editor::caret_cell`, which carries the argument.
    pub fn caret_cell(&self, caret: (usize, usize)) -> Option<(usize, usize)> {
        let b = self.built.as_ref()?;
        let (row, col) = caret;
        let laid = b.lines.get(row)?;
        let r = laid
            .starts
            .partition_point(|&at| at <= col)
            .saturating_sub(1);
        Some((laid.first + r, cells(&laid.text, laid.starts[r], col)))
    }

    /// The caret position a click at document row `row`, cell `col`, names:
    /// the wrap table read backwards. See `Editor::click`.
    pub fn hit(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        let b = self.built.as_ref()?;
        if b.rows == 0 {
            return None;
        }
        let i = b.line_at(row.min(b.rows - 1));
        let laid = b.lines.get(i)?;
        let r = row.min(b.rows - 1) - laid.first;
        let c = char_at(&laid.text, laid.starts[r], laid.starts.get(r + 1).copied(), col);
        Some((i, c))
    }

    /// What the last update, and every frame drawn since, did.
    #[cfg(test)]
    pub fn touched(&self) -> Touched {
        self.built.as_ref().map(|b| b.touched).unwrap_or_default()
    }

    /// Whether this layout draws without colour.
    #[cfg(test)]
    pub fn plain(&self) -> bool {
        self.built.as_ref().is_some_and(|b| b.plain)
    }

    /// How far down colouring has reached.
    #[cfg(test)]
    pub fn valid(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.valid)
    }

    /// Which ways `patch` has kept a chain, since the layout was last built.
    #[cfg(test)]
    pub fn chain_hits(&self) -> ChainHits {
        self.built.as_ref().map(|b| b.chain_hits).unwrap_or_default()
    }
}

impl Built {
    /// Wrap every line from the top, colouring none of them yet.
    fn fresh(lines: &[String], width: usize, look: Look, plain: bool, rev: u64) -> Self {
        let hl = match look.grammar {
            Some(grammar) if !plain => Some(source::Lines::new(grammar, look.mode)),
            _ => None,
        };
        let mut b = Built {
            width,
            look,
            plain,
            rev,
            lines: lines.iter().map(|line| Laid::new(line, width)).collect(),
            rows: 0,
            hl,
            valid: 0,
            chain: None,
            #[cfg(test)]
            touched: Touched {
                highlighted: 0,
                wrapped: lines.len(),
            },
            #[cfg(test)]
            chain_hits: ChainHits::default(),
        };
        b.number(0);
        b
    }

    /// The state colouring must enter line `k` with, when everything above it
    /// is coloured.
    fn entering(&self, k: usize) -> Option<Resume> {
        match k {
            0 => self.hl.as_ref().map(source::Lines::start),
            _ => self.lines[k - 1].after.clone(),
        }
    }

    /// Mend the layout for a text that differs from the one it was built from.
    fn patch(&mut self, lines: &[String]) {
        let old_len = self.lines.len();
        let common = old_len.min(lines.len());
        let mut top = 0;
        while top < common && self.lines[top].text == lines[top] {
            top += 1;
        }
        let mut bottom = 0;
        while bottom < common - top
            && self.lines[old_len - 1 - bottom].text == lines[lines.len() - 1 - bottom]
        {
            bottom += 1;
        }
        let old_end = old_len - bottom;
        let new_end = lines.len() - bottom;
        if top == old_end && top == new_end {
            // The same text under a new revision: an edit and its undo, or a
            // character typed and rubbed out between two frames.
            return;
        }

        // The chain below the edit, in the old numbering, worked out before
        // the splice moves anything. Below a change inside the coloured part
        // it is the rest of the coloured part; below a change further down it
        // is whatever was left of the chain there already.
        let chain = if self.hl.is_none() {
            None
        } else if old_end < self.valid {
            #[cfg(test)]
            {
                self.chain_hits.inside += 1;
            }
            Some(Chain {
                from: old_end,
                to: self.valid,
                entering: self.entering(old_end),
            })
        } else {
            match self.chain.take() {
                Some(c) if old_end <= c.from => {
                    #[cfg(test)]
                    {
                        self.chain_hits.kept += 1;
                    }
                    Some(c)
                }
                Some(c) if old_end < c.to => {
                    #[cfg(test)]
                    {
                        self.chain_hits.cut += 1;
                    }
                    Some(Chain {
                        from: old_end,
                        entering: self.lines[old_end - 1].after.clone(),
                        to: c.to,
                    })
                }
                _ => None,
            }
        };
        self.valid = self.valid.min(top);
        self.chain = chain
            .map(|c| Chain {
                from: c.from - old_end + new_end,
                to: c.to - old_end + new_end,
                entering: c.entering,
            })
            .filter(|c| c.from < c.to);

        let width = self.width;
        let fresh: Vec<Laid> = lines[top..new_end]
            .iter()
            .map(|line| Laid::new(line, width))
            .collect();
        #[cfg(test)]
        {
            self.touched.wrapped += fresh.len();
        }
        self.lines.splice(top..old_end, fresh);
        self.number(top);
    }

    /// Re-wrap every line for a new width, keeping every colour.
    fn rewrap(&mut self) {
        for laid in &mut self.lines {
            laid.starts = breaks(&laid.text, self.width);
            laid.cut();
        }
        #[cfg(test)]
        {
            self.touched.wrapped += self.lines.len();
        }
        self.number(0);
    }

    /// Colour every line down to `line`, from the watermark. See the module
    /// doc for the chain.
    fn colour_through(&mut self, line: usize) {
        let Some(hl) = self.hl.as_ref() else {
            return;
        };
        let end = (line + 1).min(self.lines.len());
        let mut k = self.valid;
        if k >= end {
            return;
        }
        let mut state = match k {
            0 => Some(hl.start()),
            _ => self.lines[k - 1].after.clone(),
        };
        while k < end {
            if let Some(c) = self.chain.as_mut()
                && c.from == k
            {
                if c.entering == state {
                    // Everything along the chain is right as it stands.
                    k = c.to;
                    self.valid = k;
                    self.chain = None;
                    if k >= end {
                        return;
                    }
                    state = self.lines[k - 1].after.clone();
                    continue;
                }
                // Not yet: this line is recoloured, and the chain now starts
                // below it, entered with what this line used to hand on.
                c.entering = self.lines[k].after.clone();
                c.from = k + 1;
                if c.from >= c.to {
                    self.chain = None;
                }
            }
            let laid = &mut self.lines[k];
            laid.spans = colour(hl, &mut state, &laid.text);
            laid.after = state.clone();
            laid.cut();
            #[cfg(test)]
            {
                self.touched.highlighted += 1;
            }
            k += 1;
            self.valid = k;
        }
    }

    /// The logical line document row `row` belongs to.
    fn line_at(&self, row: usize) -> usize {
        self.lines.partition_point(|l| l.first <= row).saturating_sub(1)
    }

    /// Renumber the first row of every line from `from` down, and the total.
    fn number(&mut self, from: usize) {
        let mut at = match from {
            0 => 0,
            _ => self.lines[from - 1].first + self.lines[from - 1].rows.len(),
        };
        for laid in &mut self.lines[from..] {
            laid.first = at;
            at += laid.rows.len();
        }
        self.rows = at;
    }
}

/// One line's colours from `state`, advancing it — plain when the
/// highlighter's answer does not add up to the line, for the reason on
/// [`faithful`].
fn colour(hl: &source::Lines, state: &mut Option<Resume>, line: &str) -> Vec<Span<'static>> {
    match state.as_mut() {
        Some(at) => faithful(Some(hl.line(at, line)), line),
        None => faithful(None, line),
    }
}

/// How many characters a run of spans holds.
fn spans_chars(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// The highlighter's spans for one source line, or a plain one when they do not
/// add up to the line they came from.
///
/// A function rather than three lines inside the layout, so that the guard can
/// be exercised at all: what it defends against is a highlighter that
/// miscounts, and there is no way to ask syntect for one. Eleven awkward
/// markdown lines through the real thing all came back exact, so today this is
/// belt to a brace that holds — but the brace is a foreign grammar engine one
/// version bump away, and the cost of it ever being wrong is every colour on
/// the row sliding off the word it belongs to, silently, in a pane where the
/// user is typing. A row that has lost its colours says nothing untrue.
///
/// The spans are cut at the *same* indices [`breaks`] gave the plain text, so
/// a line that passes this cannot drift from the caret either. That is also
/// why a tab reaches the highlighter as a tab: a span that held four spaces
/// where the line holds one `\t` would fail this check on every indented line
/// of a tabbed file.
///
/// It takes the spans by value and hands them back, because nearly every line
/// passes and a copy of every line's colours is what an open used to spend
/// its time on.
pub fn faithful(spans: Option<Vec<Span<'static>>>, line: &str) -> Vec<Span<'static>> {
    match spans {
        Some(spans) if spans_chars(&spans) == line.chars().count() => spans,
        _ => vec![Span::raw(line.to_string())],
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::err;

    fn md() -> Look {
        Look {
            mode: theme::Mode::Dark,
            grammar: Grammar::for_code("markdown"),
        }
    }

    fn split(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_string).collect()
    }

    /// Every row of a layout, which colours all of it.
    fn all(l: &mut Layout) -> Vec<Row> {
        let n = l.row_count();
        l.rows(0, n)
    }

    fn built(lines: &[String], width: usize, look: Look) -> Layout {
        let mut l = Layout::default();
        l.update(lines, 1, width, look, true);
        l
    }

    /// A markdown document with everything whose state outlives a line.
    const DOC: &str = "# Notes\n\nThe retry *budget* is wrong.\n\n```rust\nfn main() {\n\
                       \tlet x = 1;\n}\n```\n\n- one\n- two\n\n<div>\nraw html\n</div>\n\n\
                       > a quote\n> continued\n\nThe end, with 日本語 and an é.";

    #[test]
    fn typing_inside_a_paragraph_recolours_and_rewraps_only_that_line() {
        let mut lines = split(DOC);
        let mut l = built(&lines, 40, md());
        all(&mut l);
        lines[2].insert(4, 'X');
        l.update(&lines, 2, 40, md(), true);
        all(&mut l);
        assert_eq!(l.touched(), Touched { highlighted: 1, wrapped: 1 });
    }

    #[test]
    fn opening_a_fence_recolours_every_line_below_it_and_an_edit_inside_one_does_not() {
        let mut lines = split("intro\n\np1\np2\np3\np4\np5");
        let mut l = built(&lines, 40, md());
        all(&mut l);

        // Every line below an opened fence is inside it now, so every one of
        // them really has changed and the walk goes to the end of the document.
        lines[2] = "```".to_string();
        l.update(&lines, 2, 40, md(), true);
        assert_eq!(all(&mut l), all(&mut built(&lines, 40, md())));
        assert_eq!(
            l.touched(),
            Touched { highlighted: 5, wrapped: 1 },
            "the edited line and the four below it, and only the edited one re-wrapped"
        );

        // Inside the fence, a word typed leaves the grammar exactly where it
        // was, and the walk stops at once.
        lines[4].push_str(" more");
        l.update(&lines, 3, 40, md(), true);
        assert_eq!(all(&mut l), all(&mut built(&lines, 40, md())));
        assert_eq!(l.touched(), Touched { highlighted: 1, wrapped: 1 });

        // And taking the fence away colours them all back.
        lines[2] = "p1".to_string();
        l.update(&lines, 4, 40, md(), true);
        assert_eq!(all(&mut l), all(&mut built(&lines, 40, md())));
    }

    #[test]
    fn only_the_rows_asked_for_are_coloured_and_the_rest_when_they_are() {
        let lines: Vec<String> = (0..160).map(|i| format!("line {i}")).collect();
        let mut l = built(&lines, 40, md());
        assert_eq!(
            l.touched(),
            Touched { highlighted: 0, wrapped: 160 },
            "opening colours nothing"
        );

        let shown = l.rows(0, 20);
        assert_eq!(shown.len(), 20);
        assert_eq!(l.valid(), 20 + LOOKAHEAD, "the rows asked for, and the lookahead");

        // Asking further down colours down to there, from where it had got to.
        l.rows(100, 20);
        assert_eq!(l.valid(), 120 + LOOKAHEAD);
        assert_eq!(l.touched().highlighted, 120 + LOOKAHEAD);
        assert_eq!(all(&mut l), all(&mut built(&lines, 40, md())));
    }

    #[test]
    fn typing_in_a_fences_info_string_recolours_the_screen_and_not_the_block() {
        // syntect keeps the opening line's text in the state for the closing
        // fence to be matched against, so every keystroke in `python` changes
        // the state of every line in the block. Eagerly that was the whole
        // block a key; lazily it is what is on screen.
        let mut text = String::from("```py\n");
        for i in 0..160 {
            text.push_str(&format!("    v{i} = f({i})\n"));
        }
        text.push_str("```\n\nafter the block");
        let mut lines = split(&text);
        let mut l = built(&lines, 60, md());
        l.rows(0, 30);
        for (rev, c) in (2u64..).zip("thon".chars()) {
            lines[0].push(c);
            l.update(&lines, rev, 60, md(), true);
            l.rows(0, 30);
            let t = l.touched();
            assert!(
                t.highlighted <= 30 + LOOKAHEAD + 1 && t.wrapped == 1,
                "typing {c:?} into the info string did {t:?}"
            );
        }
        assert_eq!(all(&mut l), all(&mut built(&lines, 60, md())));
    }

    #[test]
    fn a_new_width_rewraps_every_line_and_recolours_none() {
        let lines = split(DOC);
        let mut l = built(&lines, 40, md());
        all(&mut l);
        let n = lines.len();
        l.update(&lines, 1, 20, md(), true);
        all(&mut l);
        assert_eq!(l.touched(), Touched { highlighted: 0, wrapped: n });
        assert_eq!(all(&mut l), all(&mut built(&lines, 20, md())));
    }

    #[test]
    fn a_new_palette_starts_again() {
        let lines = split(DOC);
        let mut l = built(&lines, 40, md());
        all(&mut l);
        let light = Look {
            mode: theme::Mode::Light,
            ..md()
        };
        l.update(&lines, 1, 40, light, true);
        all(&mut l);
        assert_eq!(l.touched().highlighted, lines.len());
        assert_eq!(all(&mut l), all(&mut built(&lines, 40, light)));
    }

    #[test]
    fn the_same_text_under_a_new_revision_costs_nothing() {
        let lines = split(DOC);
        let mut l = built(&lines, 40, md());
        all(&mut l);
        l.update(&lines, 7, 40, md(), true);
        all(&mut l);
        assert_eq!(l.touched(), Touched::default());
        assert_eq!(l.rev(), Some(7), "and the layout is current for it");
    }

    #[test]
    fn a_document_the_editor_will_not_colour_is_drawn_plain() {
        let lines = split("# heading\nprose");
        let mut l = Layout::default();
        l.update(&lines, 1, 40, md(), false);
        assert!(l.plain());
        let rows = all(&mut l);
        assert_eq!(l.touched().highlighted, 0);
        assert!(rows.iter().flat_map(|r| &r.text.spans).all(|s| s.style.fg.is_none()));

        let mut l = Layout::default();
        l.update(&lines, 1, 40, md(), true);
        assert!(!l.plain());
        assert!(all(&mut l).iter().flat_map(|r| &r.text.spans).any(|s| s.style.fg.is_some()));
        assert!(past_cap(HIGHLIGHT_MAX_BYTES + 1) && !past_cap(HIGHLIGHT_MAX_BYTES));
    }

    #[test]
    fn rows_say_which_line_they_are_and_whether_they_begin_it() {
        let lines = split("one\ntwo three four five\nsix");
        let mut l = built(&lines, 5, md());
        let text = |rows: &[Row]| -> Vec<String> {
            rows.iter()
                .map(|r| r.text.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        // Hard-wrapped, not word-wrapped: a pad is cut where the cells run out.
        let every = l.rows(0, 99);
        assert_eq!(text(&every), ["one", "two t", "hree ", "four ", "five", "six"]);
        let gutter: Vec<(usize, bool)> = every.iter().map(|r| (r.line, r.first)).collect();
        assert_eq!(
            gutter,
            [(0, true), (1, true), (1, false), (1, false), (1, false), (2, true)]
        );
        assert_eq!(text(&l.rows(2, 2)), ["hree ", "four "]);
        assert_eq!(text(&l.rows(5, 9)), ["six"]);
        assert!(l.rows(9, 3).is_empty());
        assert_eq!(l.first_row_of(2), Some(5));
        assert_eq!(l.first_row_of(3), None);
        assert_eq!(l.hit(99, 99), Some((2, 3)), "below the last row is the last row");
        assert_eq!(l.hit(2, 1), Some((1, 6)));
        assert_eq!(l.caret_cell((1, 9)), Some((2, 4)));
    }

    #[test]
    fn a_line_the_highlighter_miscounted_is_drawn_plain_rather_than_shifted() {
        // The guard the layout leans on, exercised rather than described. It
        // cannot be reached through syntect — every markdown line anybody has
        // thrown at the real highlighter comes back exact — so the miscount is
        // supplied here, which is the whole reason `faithful` is a function.
        let line = "one two";
        let exact = vec![
            Span::styled("one".to_string(), err()),
            Span::raw(" two".to_string()),
        ];
        assert_eq!(
            faithful(Some(exact.clone()), line),
            exact,
            "an exact answer stands"
        );

        // One character short, which is what a grammar that swallowed a token
        // would produce: every colour after the gap would land a cell early.
        let short = vec![
            Span::styled("one".to_string(), err()),
            Span::raw(" tw".to_string()),
        ];
        let plain = vec![Span::raw(line.to_string())];
        assert_eq!(
            faithful(Some(short), line),
            plain,
            "a miscounted line kept its colours"
        );
        // ...one character too many, and no answer at all.
        let long = vec![Span::raw("one two three".to_string())];
        assert_eq!(faithful(Some(long), line), plain);
        assert_eq!(faithful(None, line), plain);
    }

    #[test]
    fn a_tabbed_line_keeps_its_colours_because_the_highlighter_was_handed_the_tab() {
        let lines = split("```rust\n\tlet x = 1;\n```");
        let mut l = built(&lines, 40, md());
        let rows = all(&mut l);
        assert!(
            rows[1].text.spans.iter().any(|s| s.style.fg.is_some()),
            "the indented line fell back to plain: {:?}",
            rows[1]
        );
        let text: String = rows[1].text.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "    let x = 1;");
    }
}
