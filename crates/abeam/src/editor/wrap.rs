//! Where a line breaks, and how wide the pieces of it are: the one table that
//! the drawing, the cursor and a click are all read out of.
//!
//! [`breaks`] hard-wraps a logical line to the pane's width and returns the
//! char index each visual row begins at. [`into_rows`] cuts the styled spans at
//! those indices to draw them, [`cells`] measures a prefix of a row to place
//! the cursor, and [`char_at`] reads the same widths backwards to turn a click
//! into a caret. They were the pad's, and they are here unchanged in what they
//! promise; the module doc on `crate::editor` says why there is one table.
//!
//! ## Cells, and tabs
//!
//! Columns are **cells**, never chars and never bytes. A caret is a char
//! index, and the width of the prefix in front of it is what the cursor column
//! is: `設計ab` is four characters and six cells, and a layout that confused
//! the two would put the caret two columns short of the letter it is in front
//! of on the first CJK line anybody wrote.
//!
//! A literal tab is the one character whose width depends on where it is. It
//! advances to the next multiple of `source::TAB` — the same four the reader
//! expands to — measured from the **start of the row it is drawn on**, and
//! every function here applies that one rule through [`width`]. The row start
//! rather than the line start, because a row is then a thing that can be
//! measured from its own first character: a tab on a continuation row is drawn
//! where its row says, and none of the three readings has to know how wide the
//! rows above it were. On a line that does not wrap — which is nearly every
//! line with a tab in it, since tabs lead lines — the two rules are the same
//! rule. The reader's `expand_tabs` counts its stops in characters rather than
//! cells and so differs from this on a line where a wide character comes before
//! a tab; drawing and measuring here have to agree with each other first, and
//! cells is the unit they agree in.
//!
//! The pad never holds a tab — its policy turns each into two spaces — so for
//! the pad all of this is the rule it always had.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::panes::viewer::source::TAB;

/// How many cells `ch` takes when it is drawn `used` cells into its row.
///
/// The one place a tab's width is decided. A tab fills to the next stop; every
/// other character is its `unicode_width`, and a character with none — a
/// combining mark, a control — is nothing at all, as it always was.
pub fn width(ch: char, used: usize) -> usize {
    if ch == '\t' {
        TAB - used % TAB
    } else {
        ch.width().unwrap_or(0)
    }
}

/// Where a logical line breaks when it is hard-wrapped to `width` cells.
///
/// The char index each visual row starts at, first one included, so the result
/// is never empty and `starts[0]` is always 0. This is the whole of the layout:
/// the drawing, the cursor and a click are all this table read one way or the
/// other, which is what stops the caret and the text it is in disagreeing about
/// which row they are on.
///
/// The `i > last` guard is what makes a character wider than the pane land
/// somewhere rather than spinning: in a one-column pane an ideograph does not
/// fit on any row, so it goes on the empty row it is already on and overflows
/// it. A row that is one cell too wide is a cosmetic fault in a pane nobody can
/// read anyway; a loop that never breaks the line is a hang in the draw path.
/// A tab that does not fit at the end of a row goes to the next one and is
/// measured again there, from that row's start, which is the rule on
/// [`width`]; in a pane narrower than one tab stop it is the ideograph's case.
pub fn breaks(line: &str, width_cells: usize) -> Vec<usize> {
    let limit = width_cells.max(1);
    let mut starts = vec![0usize];
    let mut used = 0usize;
    for (i, ch) in line.chars().enumerate() {
        let mut w = width(ch, used);
        if used + w > limit && i > starts[starts.len() - 1] {
            starts.push(i);
            used = 0;
            w = width(ch, 0);
        }
        used += w;
    }
    starts
}

/// The cells `line` occupies between two char indices, where `from` is the
/// start of a row.
///
/// The start of a row and not any index at all, because that is where a tab
/// measures from; every caller passes a `starts` entry, which is the only
/// place a row can begin.
pub fn cells(line: &str, from: usize, to: usize) -> usize {
    let mut used = 0usize;
    for ch in line.chars().skip(from).take(to.saturating_sub(from)) {
        used += width(ch, used);
    }
    used
}

/// Which character of `line` a click at cell `col` landed on, for the row that
/// begins at `from` and is followed by the row beginning at `next`.
///
/// The wrap table read backwards, and the one place a pointer becomes a text
/// position. `next` is `None` on the last row of a logical line, and it is an
/// `Option` rather than a second index because "is there another row after this
/// one" is precisely what the answer turns on: a caller handed two numbers
/// could get the relationship between them wrong, and a caller handed
/// `starts.get(r + 1).copied()` cannot.
///
/// **Past the end of a wrapped row the answer is one short of where the next
/// row starts**, and that subtraction is the whole of the invariant the editor's
/// module doc names. The caret positions belonging to a row are the indices
/// inside it; the index the next row begins at belongs to *that* row, because
/// the caret, read from the same table forwards, is drawn there. Answering
/// `next` would put the caret one line below the cell the pointer was over,
/// and it takes no exotic text to reproduce — `abc日def` at four columns wraps
/// after `abc`, so cell three of the first row is already past its content,
/// and any line with an ideograph or an emoji in it has such a cell on most of
/// its rows.
///
/// On the last row of a line there is no next row and the answer is the end of
/// the line, which is what makes a click in the empty space to the right of a
/// short line mean the end of that line — the commonest click there is, and the
/// one `Buffer::set_caret`'s documentation is written around.
///
/// Inside a wide character the caret goes in front of it rather than to the
/// nearer edge. Splitting an ideograph down the middle would make the answer
/// depend on which half of a two-cell glyph the pointer was over, which is not
/// something anybody aims at. A tab is a wide character for this purpose: a
/// click anywhere in the run of cells it fills puts the caret in front of it.
pub fn char_at(line: &str, from: usize, next: Option<usize>, col: usize) -> usize {
    let end = next.unwrap_or_else(|| line.chars().count());
    let mut used = 0usize;
    for (i, ch) in line
        .chars()
        .enumerate()
        .skip(from)
        .take(end.saturating_sub(from))
    {
        let w = width(ch, used);
        if used + w > col {
            return i;
        }
        used += w;
    }
    // `breaks` emits no empty row, so a wrapped row always has a character to
    // step back over and the floor below never fires. A floor rather than an
    // assertion because this runs under somebody's pointer: the cost of being
    // wrong about that should be a caret in a dull place, not the program going
    // down mid-click.
    match next {
        Some(next) => next.saturating_sub(1).max(from),
        None => end,
    }
}

/// Cut one logical line's spans at the indices [`breaks`] gave, keeping their
/// colours.
///
/// One walk over the characters, cutting wherever a row begins, and that is
/// not laziness: a cut has to be able to land in the middle of a span, spans do
/// not line up with rows, and the two accounts of where a character is have to
/// be the same walk or they are the drift this file exists to prevent. Runs of
/// one colour are joined as they are built, so a row of one colour is one span.
///
/// There is always one row per entry in `starts`, which is what lets the
/// drawing and the wrap table be indexed by the same number.
///
/// A tab is drawn here as the spaces [`width`] says it fills, in the colour of
/// the span it was in, because a `\t` written into a terminal cell is not a
/// character the cell can hold.
pub fn into_rows(spans: &[Span<'static>], starts: &[usize]) -> Vec<Line<'static>> {
    cut(spans.iter().map(|s| (s.content.as_ref(), s.style)), starts)
}

/// The rows of a line drawn without colour: [`into_rows`] for one unstyled
/// span, without first copying the line into one.
pub fn plain_rows(line: &str, starts: &[usize]) -> Vec<Line<'static>> {
    cut(std::iter::once((line, Style::default())), starts)
}

fn cut<'a>(pieces: impl Iterator<Item = (&'a str, Style)>, starts: &[usize]) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::with_capacity(starts.len());
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut at = 0usize;
    let mut next = starts.get(1).copied();
    for (text, style) in pieces {
        let mut run = String::with_capacity(text.len());
        for ch in text.chars() {
            if next == Some(at) {
                join(&mut row, std::mem::take(&mut run), style);
                rows.push(Line::from(std::mem::take(&mut row)));
                used = 0;
                next = starts.get(rows.len() + 1).copied();
            }
            let w = width(ch, used);
            used += w;
            if ch == '\t' {
                run.extend(std::iter::repeat_n(' ', w));
            } else {
                run.push(ch);
            }
            at += 1;
        }
        join(&mut row, run, style);
    }
    rows.push(Line::from(row));
    // Spans that came up short of the text — which `faithful` makes
    // impossible, and which would otherwise leave the drawing a row behind the
    // table — still give one row per start.
    while rows.len() < starts.len() {
        rows.push(Line::default());
    }
    rows
}

/// Put `run` on the end of `row`, as part of the last span when it is the same
/// colour.
fn join(row: &mut Vec<Span<'static>>, run: String, style: Style) {
    if run.is_empty() {
        return;
    }
    match row.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push_str(&run),
        _ => row.push(Span::styled(run, style)),
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::err;

    fn texts(rows: &[Line<'static>]) -> Vec<String> {
        rows.iter()
            .map(|row| row.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn a_line_breaks_where_the_cells_run_out_and_never_before() {
        assert_eq!(breaks("", 10), [0]);
        assert_eq!(
            breaks("abcdefghij", 10),
            [0],
            "an exactly full row is one row"
        );
        assert_eq!(breaks("abcdefghijk", 10), [0, 10]);
        // Cells, not characters: two ideographs fill four columns.
        assert_eq!(breaks("設計設計設", 4), [0, 2, 4]);
        // A character wider than the pane still lands somewhere.
        assert_eq!(breaks("設計", 1), [0, 1]);
    }

    #[test]
    fn the_rows_and_the_caret_are_cut_from_the_same_table() {
        // `into_rows` is handed the indices `breaks` produced, so a row's text
        // is by construction the characters the caret map says are on it.
        let line = "設計abcdefgh";
        let starts = breaks(line, 10);
        let rows = into_rows(&[Span::raw(line.to_string())], &starts);
        assert_eq!(texts(&rows), ["設計abcdef", "gh"]);
        assert_eq!(cells(line, starts[0], 4), 6);
        // Past the end of the last row is the end of the line; past the end of
        // a wrapped one stops short of where the next row begins, so that the
        // forward reading draws it back on the row that was clicked.
        assert_eq!(char_at(line, starts[1], None, 99), 10);
        assert_eq!(char_at(line, starts[0], Some(starts[1]), 99), starts[1] - 1);
    }

    #[test]
    fn colours_are_cut_at_the_place_the_text_is() {
        let styled = vec![
            Span::styled("abc".to_string(), err()),
            Span::raw("defgh".to_string()),
        ];
        let rows = into_rows(&styled, &[0, 4]);
        assert_eq!(rows[0].spans.len(), 2, "the cut fell inside a span");
        assert_eq!(rows[0].spans[0].content.as_ref(), "abc");
        assert_eq!(rows[0].spans[0].style, err());
        assert_eq!(rows[0].spans[1].content.as_ref(), "d");
        assert_eq!(rows[1].spans[0].content.as_ref(), "efgh");
    }

    // --- tabs -------------------------------------------------------------

    #[test]
    fn a_tab_fills_to_the_next_stop_and_is_drawn_as_the_spaces_it_fills() {
        assert_eq!(width('\t', 0), 4);
        assert_eq!(width('\t', 1), 3);
        assert_eq!(width('\t', 3), 1);
        assert_eq!(width('\t', 4), 4);

        let line = "a\tb\t\tc";
        assert_eq!(cells(line, 0, 2), 4, "a, then the tab to column four");
        assert_eq!(cells(line, 0, 6), 13);
        let rows = into_rows(&[Span::raw(line.to_string())], &[0]);
        assert_eq!(texts(&rows), ["a   b       c"]);
        assert!(!texts(&rows)[0].contains('\t'), "a cell cannot hold a tab");
    }

    #[test]
    fn a_tab_keeps_the_colour_of_the_span_it_was_in() {
        let spans = vec![Span::styled("\t".to_string(), err()), Span::raw("x".to_string())];
        let rows = into_rows(&spans, &[0]);
        assert_eq!(rows[0].spans[0].content.as_ref(), "    ");
        assert_eq!(rows[0].spans[0].style, err());
        assert_eq!(rows[0].spans[1].content.as_ref(), "x");
    }

    #[test]
    fn a_tab_that_does_not_fit_wraps_and_is_measured_from_its_new_row() {
        // Ten cells: `abcdefghi` is nine, and the tab would fill to twelve.
        // It goes to the next row and fills a whole stop there.
        let line = "abcdefghi\tz";
        let starts = breaks(line, 10);
        assert_eq!(starts, [0, 9]);
        let rows = into_rows(&[Span::raw(line.to_string())], &starts);
        assert_eq!(texts(&rows), ["abcdefghi", "    z"]);
        assert_eq!(cells(line, starts[1], 11), 5, "the caret after the z");
    }

    #[test]
    fn plain_rows_are_the_rows_of_one_unstyled_span() {
        for (line, w) in [("", 5), ("abcdefghij", 4), ("a\t設計\tb", 5), ("x", 1)] {
            let starts = breaks(line, w);
            assert_eq!(
                plain_rows(line, &starts),
                into_rows(&[Span::raw(line.to_string())], &starts),
                "{line:?} at {w}"
            );
            assert_eq!(plain_rows(line, &starts).len(), starts.len());
        }
    }

    #[test]
    fn a_click_anywhere_in_a_tab_puts_the_caret_in_front_of_it() {
        let line = "a\tb";
        for col in 1..4 {
            assert_eq!(char_at(line, 0, None, col), 1, "cell {col} is inside the tab");
        }
        assert_eq!(char_at(line, 0, None, 4), 2, "cell four is the b");
        assert_eq!(char_at(line, 0, None, 0), 0);
    }

    #[test]
    fn every_caret_position_is_drawn_where_a_click_on_it_would_put_it() {
        // The three readings agree on a line with tabs, ideographs and a
        // combining mark at several widths: for each caret index, the cell the
        // cursor is drawn in is a cell a click turns back into that index — or,
        // where the caret sits in front of the zero-width mark, which has no
        // cell of its own, into the index just past the mark.
        let line = "\ta設\tb\u{301}計\t\tcd";
        let chars: Vec<char> = line.chars().collect();
        for w in [3usize, 4, 5, 7, 10, 40] {
            let starts = breaks(line, w);
            for caret in 0..=chars.len() {
                let r = starts.partition_point(|&at| at <= caret).saturating_sub(1);
                let col = cells(line, starts[r], caret);
                let back = char_at(line, starts[r], starts.get(r + 1).copied(), col);
                let only_marks_between =
                    back > caret && chars[caret..back].iter().all(|&c| width(c, 0) == 0);
                assert!(
                    back == caret || only_marks_between,
                    "width {w}, caret {caret}: drawn at cell {col}, clicked back to {back}"
                );
            }
        }
    }
}
