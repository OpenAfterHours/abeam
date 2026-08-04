//! Mermaid diagrams, drawn on the character grid.
//!
//! A ```` ```mermaid ```` fence used to arrive here as source and leave as
//! source: syntect has no grammar for it, so the block was reproduced verbatim
//! and uncoloured. That is the one fenced language in a design document whose
//! *point* is not the text — nobody writes `A --> B` to be read as `A --> B` —
//! and a pane that exists to show the reader what the agent just wrote was
//! showing them the least useful half of it.
//!
//! So the diagram is laid out here instead, in box-drawing characters, for the
//! width the pane happens to be. Two families are drawn: `graph`/`flowchart`
//! and `sequenceDiagram`. Everything else — state, class, ER, gantt, pie,
//! mindmap, and whatever mermaid adds next — returns `None` and gets the code
//! block it has always had.
//!
//! ## Three outcomes, never a fourth
//!
//! Every path through here ends in one of:
//!
//! - the diagram, drawn to fit;
//! - the same diagram as an **outline** — indented, arrowed, one edge per row —
//!   because the pane is too narrow for boxes. Tables already do exactly this
//!   when a grid will not fit (`markdown::emit_table_as_records`), and the
//!   reason is the same: at forty columns a four-cell-wide box is not a
//!   diagram, it is a puzzle;
//! - `None`, and the caller shows the mermaid source.
//!
//! There is no fourth outcome where content is dropped. A node the parser does
//! not understand, an arrow spelled a way this does not know, a diagram larger
//! than the caps below — all of them abandon the drawing and hand back `None`,
//! because the source is always *true*, and a diagram missing an edge is worse
//! than one nobody drew. That is the single rule this module is built around.
//!
//! ## What is coloured, and what is not
//!
//! Two families are drawn by two sets of code, and a reader looking at one
//! document containing both must not be able to tell. So the palette is
//! assigned by *role* here, once, rather than by each drawer for itself:
//!
//! - **text** — node labels, participant names, edge labels, message captions,
//!   note text — is [`Theme::fg`], the body colour. All of it. It is what the
//!   reader came to read, and an edge label is not chrome because it is short.
//! - **structure** — box frames, lifelines, connectors, block gutters, the
//!   rules under a heading row — is [`Theme::dim`]. Every connector, whatever
//!   its stroke: solid, dotted and thick are told apart by their *glyphs*
//!   (`─`, `┄`, `━`), never by their colour, because colour alone is not a
//!   signal every reader receives. That rule is `theme`'s, and it is the same
//!   reason a link here is underlined as well as blue.
//! - **a severed or destructive end** — `--x` in a flowchart, `-x` in a
//!   sequence diagram — is [`Theme::danger`], and is the glyph `╳` in both.
//!   The two grammars spell one concept and it is drawn one way.
//! - **a keyword the diagram did not get from the document** — `loop`, `alt`,
//!   `opt`, and the marker on an edge cut to break a cycle — is
//!   [`Theme::special`], because it is abeam speaking rather than the author.
//!
//! Nothing here names a colour of its own. The pane paints its own background
//! (see `viewer::theme`), so an ANSI name resolved against the terminal's own
//! profile lands on it unreadably; there is a test asserting every span a
//! diagram emits is coloured from the palette above.
//!
//! ## Width
//!
//! `render` is given the columns it may use and every row it returns is at most
//! that wide. It does not get to defer wrapping the way prose does: a box
//! reflowed at a word boundary is not a smaller box, it is rubble. So each
//! drawer measures first, decides between boxes and the outline, and only then
//! emits. The caller hard-wraps anyway, as insurance rather than as layout — a
//! row that overflows is a bug in here, and hard-wrapping it keeps that bug
//! from spilling over the pane while the tests catch it.

use ratatui::text::Span;

use super::theme::Theme;
use crate::text::wrap::spans_width;

mod flow;
mod lex;
mod seq;
#[cfg(test)]
mod tests;

/// One row of a drawn diagram, unwrapped and already at most `width` cells.
/// Rows rather than `Line`s for the same reason `source` returns rows: the
/// caller owns the prefixes — a diagram inside a list item is indented under
/// its bullet like anything else — and gluing them on here would mean measuring
/// the width twice and getting it wrong once.
pub type Rows = Vec<Vec<Span<'static>>>;

/// Past this the source is shown instead. Thirty-two kilobytes of mermaid is a
/// *generated* diagram — hand-written ones in this repository's own docs run to
/// a few hundred bytes — and it is the cheap check that stops the three below
/// from having to be done on a megabyte of text first.
const MAX_BYTES: usize = 32 * 1024;

/// Nodes in a flowchart, or participants in a sequence diagram.
///
/// This is a *legibility* bound that the clock happens to sit well inside, and
/// the two were measured rather than assumed. A graph at exactly these caps
/// lays out in **4.7 ms** at forty columns and **3.8 ms** at eighty, in a
/// release build on the machine this was written on; a sequence diagram at them
/// costs 5.2 ms. Against `source::HIGHLIGHT_MAX_BYTES`, which is a 170 ms
/// budget on this same frame, that is a rounding error — so the clock is not
/// what sets these.
///
/// What sets them is that the same graph draws **668 rows**. A hundred and
/// twenty-eight nodes is already fourteen screens of diagram, which is past the
/// point where anyone is reading a picture, and the caps are held here so that
/// raising them has to be argued from something other than "it is still fast".
/// The row caps in `flow` are the same argument made about one drawing.
pub(super) const MAX_NODES: usize = 128;
/// Edges in a flowchart, or messages in a sequence diagram. Twice the nodes,
/// because a diagram whose edges outnumber its nodes two to one is a mesh, and
/// a mesh drawn in one column of connectors is not readable at any width.
pub(super) const MAX_EDGES: usize = 256;

/// Whether a fence's info string names a mermaid diagram.
///
/// Matched on the first token only, and case-insensitively, because a fence is
/// written by hand: ```` ```mermaid ````, ```` ```Mermaid ```` and
/// ```` ```mermaid title=flow ```` are all the same request.
pub fn is_mermaid(info: &str) -> bool {
    info.split_whitespace()
        .next()
        .is_some_and(|token| token.eq_ignore_ascii_case("mermaid"))
}

/// Draw `source` into at most `width` columns, or decline.
///
/// `None` means "this is not a diagram I can draw" and is not an error — it is
/// most of mermaid, by diagram type, and it is the fence the caller was going
/// to render as code anyway.
pub fn render(source: &str, width: usize, theme: &'static Theme) -> Option<Rows> {
    // Four is what the narrowest thing either drawer emits actually costs: the
    // outline's bare form is `└─▶ ` before a single character of label, and the
    // sequence list's is `1. `. Below that there is no row to put a diagram on,
    // and declining here means neither drawer has to reason about a width it
    // cannot put an arrow in. It is *not* the width at which they start
    // drawing — both decline a good way above this when the longest word in the
    // diagram would have to be broken in half to fit. See the module note.
    if width < 4 || source.len() > MAX_BYTES {
        return None;
    }

    let lines = lex::meaningful_lines(source);
    let (header, rest) = lines.split_first()?;
    // `graph TD; A-->B` is a whole diagram on the header line. Splitting it
    // here rather than in each parser is what lets `Kind::of` read a header
    // that is only ever two words, and costs the sequence parser nothing: no
    // `sequenceDiagram` header carries a statement behind a semicolon.
    let mut header = lex::split_statements(header);
    let tail = header.split_off(1.min(header.len()));
    let kind = Kind::of(header.first()?)?;
    let body: Vec<String> = tail.into_iter().chain(rest.iter().cloned()).collect();

    let rows = match kind {
        Kind::Flow(direction) => flow::render(direction, &body, width, theme),
        Kind::Sequence => seq::render(&body, width, theme),
    }?;

    // Nothing below is allowed to have produced an over-wide row. Checked here
    // rather than in each drawer so that the guarantee has one owner, and
    // debug-only because the caller hard-wraps and the tests assert it for
    // every width from four columns up.
    debug_assert!(
        rows.iter().all(|row| spans_width(row) <= width),
        "a drawer returned a row wider than the {width} columns it was given"
    );
    Some(rows)
}

/// Which diagram this is, and the only thing read off the header line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Flow(Direction),
    Sequence,
}

impl Kind {
    /// Read from the first meaningful line. Mermaid is case-sensitive about
    /// these keywords and so is this: `Graph TD` is not a diagram mermaid
    /// itself would draw, and quietly drawing one here would put abeam and the
    /// reader's browser in disagreement about the same file.
    fn of(header: &str) -> Option<Self> {
        let mut words = header.split_whitespace();
        let keyword = words.next()?;
        // `graph TD;` and `flowchart LR` both reach here with the direction as
        // the second word. A header carrying its first statement behind a
        // semicolon has already been split by the `lex::split_statements` call
        // in `render`, so this only ever sees the keyword and the direction.
        match keyword.trim_end_matches(';') {
            "graph" | "flowchart" => {
                let direction = words
                    .next()
                    .map(|d| d.trim_end_matches(';'))
                    .map_or(Some(Direction::Down), Direction::of)?;
                Some(Kind::Flow(direction))
            }
            "sequenceDiagram" => Some(Kind::Sequence),
            _ => None,
        }
    }
}

/// The four directions mermaid spells six ways.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Down,
    Up,
    Right,
    Left,
}

impl Direction {
    fn of(word: &str) -> Option<Self> {
        match word {
            "TD" | "TB" => Some(Direction::Down),
            "BT" => Some(Direction::Up),
            "LR" => Some(Direction::Right),
            "RL" => Some(Direction::Left),
            _ => None,
        }
    }

    /// Whether ranks advance down the page. `LR` and `RL` lay out across it,
    /// which in a pane forty columns wide is usually the layout that does not
    /// fit — but that is the drawer's decision to make and the outline's to
    /// catch, not something to silently rewrite here.
    pub(super) fn is_vertical(self) -> bool {
        matches!(self, Direction::Down | Direction::Up)
    }

    /// Whether the *last* rank is drawn first: `BT` and `RL` are `TD` and `LR`
    /// with the ranks reversed, which is the whole of their implementation.
    pub(super) fn is_reversed(self) -> bool {
        matches!(self, Direction::Up | Direction::Left)
    }
}
