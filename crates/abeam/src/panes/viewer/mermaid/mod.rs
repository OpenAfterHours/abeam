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
mod outline;
mod seq;
#[cfg(test)]
mod tests;

/// One row of a drawn diagram, unwrapped and already at most `width` cells.
/// Rows rather than `Line`s for the same reason `source` returns rows: the
/// caller owns the prefixes — a diagram inside a list item is indented under
/// its bullet like anything else — and gluing them on here would mean measuring
/// the width twice and getting it wrong once.
pub type Rows = Vec<Vec<Span<'static>>>;

/// Past this the source is shown instead. Not a memory bound — a quarter of a
/// megabyte of mermaid is a *generated* diagram, and there is no width at which
/// a four-hundred-node graph is a thing anybody reads on a terminal. The caps
/// exist so the draw path cannot be handed a layout problem that takes longer
/// than a keystroke; see `source::HIGHLIGHT_MAX_BYTES`, which is the same
/// argument about the same frame.
const MAX_BYTES: usize = 32 * 1024;
/// Nodes in a flowchart, or participants in a sequence diagram.
pub(super) const MAX_NODES: usize = 128;
/// Edges in a flowchart, or messages in a sequence diagram.
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
    // A single column can hold one character, which is not a diagram and is not
    // an outline either. Declining here means the drawers below never have to
    // reason about a width they cannot put an arrow in.
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
        // the second word; mermaid also allows `graph TD;` with the first
        // statement on the same line, which `lex::meaningful_lines` has
        // already split apart.
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
