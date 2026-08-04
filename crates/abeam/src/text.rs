//! Measuring, clipping and styling text for a pane.
//!
//! Everything here is shared because the alternative was measured: three panes
//! each grew their own `dim()`, and two of them grew a `clip()` with different
//! ideas about whether truncation should be visible. A reader in a narrow
//! window could then see a value cut short with a `…` in the git pane and cut
//! short *silently* in the diagnostics pane — in the one view whose whole job is
//! to be trusted about what the pty is doing.
//!
//! One rule: **truncation is always marked**. A row that overflows its rect
//! corrupts the frame rather than merely looking wrong, so clipping happens at
//! one point per pane and says so when it happens.
//!
//! Widths are terminal cells throughout, never bytes and never chars. A CJK
//! ideograph is two cells and `str::len` is wrong about it twice over.
//!
//! [`restyle`] is the one exception, and it is an exception to the *unit* and
//! not to the rule. Everything else here is deciding what fits, which is a
//! question about cells; that one is being told where something was *found*,
//! which is a position, and a position in a row is counted in characters. It
//! carries the argument for why the pair must not be cells, and the caveat that
//! characters are not quite the last word either — a terminal draws grapheme
//! clusters, and a cut inside one changes the width of the row.

pub mod wrap;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The chrome colour: everything a pane says in its own voice, which should
/// never be mistaken for content.
pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Something is wrong and the reader has to notice.
pub fn err() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

/// Truncate to `max` cells, marking the cut with `…`.
pub fn clip(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        // The marker costs a cell, so it has to be budgeted for before the cut
        // rather than appended over the edge afterwards.
        if w + cw + 1 > max {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Clip a run of spans as one unit, keeping their styles.
pub fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let mut out = Vec::with_capacity(spans.len());
    let mut w = 0usize;
    for s in spans {
        if w >= max {
            break;
        }
        let sw = s.content.width();
        if w + sw <= max {
            w += sw;
            out.push(s);
        } else {
            out.push(Span::styled(clip(&s.content, max - w), s.style));
            break;
        }
    }
    out
}

/// The same, in place on a whole line. For panes that build lines first and
/// clip them all on the way out.
pub fn clip_line(line: Line<'static>, width: usize) -> Line<'static> {
    let style = line.style;
    let mut out = Line::from(clip_spans(line.spans, width));
    out.style = style;
    out
}

/// Patch `style` onto one run of a line's text, splitting spans at both ends
/// of the run.
///
/// **The one thing in this module counted in characters.** Everything else here
/// measures cells, because everything else is deciding what fits. This is not:
/// it is being told where a match was *found*, and a match is found by walking
/// the row's characters. The two numbers are different — `設計文書` is four
/// characters and eight cells — and a helper that took cells would need the
/// caller to convert, at which point a match that starts inside a wide
/// character becomes representable and the first CJK document would paint half
/// an ideograph. Splitting by character and leaving the widths to the renderer
/// is what makes that unrepresentable rather than merely unlikely.
///
/// ## Clusters, which characters are one level short of
///
/// A terminal draws grapheme clusters, and a cut inside one is worse than ugly:
/// `"cafe\u{301}"` cut after the `e` loses the acute off the highlighted letter,
/// and a ZWJ family emoji cut after its first component becomes *two* glyphs —
/// two cells become four, on a row that was pre-wrapped to the pane's exact
/// width, and everything after it slides off the end. That breaks the
/// invariant the whole viewer rests on, so the run is snapped **outward** to a
/// boundary before anything is cut. A highlight that covers one character more
/// than was asked for is a cosmetic overshoot; a row that is wider than the
/// pane corrupts the frame.
///
/// [`is_join`] is the boundary test, and it is an **approximation of UAX #29,
/// not an implementation of it**, because implementing UAX #29 means a table
/// and a table means a dependency. It treats a character as belonging to the
/// one before it when the character has zero width — every combining mark,
/// every variation selector, ZWJ itself — or when the character before it was
/// a ZWJ. That covers the two cases above and, with them, every combining
/// sequence and every ZWJ emoji sequence.
///
/// It does **not** cover the clusters built from characters that all have a
/// width of their own: a regional-indicator flag pair, a Hangul jamo sequence,
/// an emoji with a skin-tone modifier. Those can still be cut in two. The pane
/// can live with it because it is already living with it: `unicode-width` gives
/// each of those characters its own width and the terminal draws the pair as
/// one, so `wrap` has been disagreeing with the terminal about those rows since
/// before there was a search — a highlight that splits one makes the row no
/// wider than the layout already believed it was.
///
/// `patch` rather than replace, so a highlight that names only a background
/// keeps the syntax colour under it and a highlight that names both wins
/// outright. `start` is an offset into the line's text with the spans ignored,
/// and applying several runs in turn is safe: splitting a span never moves any
/// character.
pub fn restyle(line: &mut Line<'static>, start: usize, len: usize, style: Style) {
    if len == 0 {
        return;
    }
    // One vector per call, bounded by the row rather than by the document —
    // and only visible rows are ever restyled.
    let chars: Vec<char> = line.spans.iter().flat_map(|s| s.content.chars()).collect();
    let end = snap_out(&chars, (start + len).min(chars.len()), 1);
    let start = snap_out(&chars, start.min(chars.len()), -1);
    if start >= end {
        return;
    }

    let mut out: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 2);
    let mut at = 0usize;

    for span in std::mem::take(&mut line.spans) {
        let lo = at;
        at += span.content.chars().count();
        if at <= start || lo >= end {
            out.push(span);
            continue;
        }
        let text = span.content;
        let a = byte_at(&text, start.saturating_sub(lo));
        let b = byte_at(&text, end - lo);
        if a > 0 {
            out.push(Span::styled(text[..a].to_string(), span.style));
        }
        out.push(Span::styled(
            text[a..b].to_string(),
            span.style.patch(style),
        ));
        if b < text.len() {
            out.push(Span::styled(text[b..].to_string(), span.style));
        }
    }
    line.spans = out;
}

/// Where the `n`th character of `s` starts, or the end of `s` if there is no
/// such character.
fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

/// Zero-width joiner. A character with a width of its own, joined to the one
/// before it by the character between them.
const ZWJ: char = '\u{200d}';

/// Does `chars[i]` belong to the cluster that started before it?
///
/// The approximation [`restyle`] documents: a zero-width character attaches to
/// what precedes it — that is every combining mark, every variation selector,
/// and ZWJ itself — and so does whatever follows a ZWJ. Deliberately blind to
/// regional indicators, jamo and emoji modifiers; see [`restyle`] for why the
/// pane can carry that.
fn is_join(chars: &[char], i: usize) -> bool {
    i > 0 && i < chars.len() && (chars[i].width().unwrap_or(0) == 0 || chars[i - 1] == ZWJ)
}

/// Move `i` off a cluster boundary it is not on, in the direction that makes
/// the run cover more rather than less. See [`restyle`].
fn snap_out(chars: &[char], mut i: usize, step: isize) -> usize {
    while is_join(chars, i) {
        if step < 0 {
            i -= 1;
        } else {
            i += 1;
        }
    }
    i
}

/// Truncate from the *left*: given `crates/abeam/src/panes/git.rs` and no room,
/// the half worth keeping is the one with the filename in it.
pub fn elide_left(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut tail = String::new();
    let mut w = 0;
    for ch in s.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        tail.push(ch);
        w += cw;
    }
    let mut out = String::from("…");
    out.extend(tail.chars().rev());
    out
}

/// Plain wrapped prose in one style. For notices, hints and anything else a
/// pane says about itself rather than about its content.
pub fn block(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    text.split('\n')
        .flat_map(|para| {
            if para.is_empty() {
                vec![Line::default()]
            } else {
                wrap::wrap_spans(vec![Span::styled(para.to_string(), style)], width, &[], &[])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_measured_in_cells_and_always_marked() {
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("hello world", 8), "hello w…");
        assert_eq!(clip("", 0), "");
        assert_eq!(clip("abc", 0), "");
        // CJK is two cells per char: four of them do not fit in five columns.
        assert_eq!("設計文書".width(), 8);
        assert!(clip("設計文書", 5).width() <= 5);
        // The mark is the point: a reader must be able to tell a value that was
        // cut from one that happened to end there.
        assert!(clip("something long", 6).ends_with('…'));
    }

    #[test]
    fn eliding_a_path_keeps_the_filename() {
        assert_eq!(elide_left("src/app.rs", 20), "src/app.rs");
        assert_eq!(elide_left("crates/abeam/src/panes/git.rs", 12), "…anes/git.rs");
        let out = elide_left("crates/abeam/src/panes/git.rs", 12);
        assert!(out.width() <= 12, "{out:?} is {} cells", out.width());
        assert!(out.ends_with("git.rs"), "{out:?} lost the filename");
        assert!(out.starts_with('…'));
        assert_eq!(elide_left("anything", 0), "");
    }

    #[test]
    fn clipping_a_line_keeps_the_spans_it_can_and_marks_the_one_it_cuts() {
        let line = Line::from(vec![
            Span::styled("label ", dim()),
            Span::styled("a value that is far too long", err()),
        ]);
        let out = clip_line(line, 12);
        let text: String = out.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.width(), 12);
        assert!(text.ends_with('…'), "{text:?}");
        // Styles survive the cut, or the truncated span changes colour.
        assert_eq!(out.spans[1].style, err());
    }

    /// The style the viewer's search paints a hit with.
    fn hit() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::UNDERLINED)
    }

    fn parts(line: &Line<'_>) -> Vec<(String, bool)> {
        line.spans
            .iter()
            .map(|s| (s.content.to_string(), s.style.bg == Some(Color::Yellow)))
            .collect()
    }

    #[test]
    fn a_run_is_restyled_where_it_is_and_the_rest_of_the_line_is_not() {
        let mut line = Line::from(vec![Span::raw("the quick brown fox".to_string())]);
        restyle(&mut line, 4, 5, hit());
        assert_eq!(
            parts(&line),
            [
                ("the ".into(), false),
                ("quick".into(), true),
                (" brown fox".into(), false),
            ]
        );
    }

    #[test]
    fn a_run_that_crosses_a_span_boundary_is_cut_at_both_ends() {
        // What a match in rendered markdown actually looks like: the word is
        // half plain and half bold, because the document said so.
        let mut line = Line::from(vec![
            Span::styled("do the ".to_string(), dim()),
            Span::styled("thing".to_string(), err()),
        ]);
        restyle(&mut line, 4, 6, hit());
        assert_eq!(
            parts(&line),
            [
                ("do t".into(), false),
                ("he ".into(), true),
                ("thi".into(), true),
                ("ng".into(), false),
            ]
        );
        // Patched, not replaced: the bold the document asked for survives
        // inside the highlight, and the styles outside it are untouched.
        assert!(line.spans[3].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[0].style, dim());
    }

    #[test]
    fn a_run_lands_on_the_right_cells_of_a_wide_row() {
        // Characters in, cells out. Four ideographs are eight cells; a match on
        // the middle two is characters 1..3 and cells 2..6, and confusing the
        // two paints half an ideograph.
        let mut line = Line::from(vec![Span::raw("設計文書".to_string())]);
        restyle(&mut line, 1, 2, hit());
        assert_eq!(
            parts(&line),
            [
                ("設".into(), false),
                ("計文".into(), true),
                ("書".into(), false),
            ]
        );
        assert_eq!(line.spans[0].content.width(), 2, "one ideograph, two cells");
        assert_eq!(line.spans[1].content.width(), 4);
    }

    #[test]
    fn a_run_is_snapped_out_to_a_cluster_boundary_before_anything_is_cut() {
        // A terminal draws clusters, not characters. `cafe` plus a combining
        // acute, cut after the `e`, loses the accent off the very letter it is
        // highlighting.
        let mut line = Line::from(vec![Span::raw("cafe\u{301} x".to_string())]);
        restyle(&mut line, 3, 1, hit());
        assert_eq!(
            parts(&line),
            [
                ("caf".into(), false),
                ("e\u{301}".into(), true),
                (" x".into(), false),
            ]
        );

        // The serious one. A ZWJ family cut after its first component is drawn
        // as two glyphs: two cells become four, on a row pre-wrapped to the
        // pane's exact width, and everything after it slides off the end.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let mut line = Line::from(vec![Span::raw(format!("{family}!"))]);
        restyle(&mut line, 0, 1, hit());
        assert_eq!(
            parts(&line),
            [(family.to_string(), true), ("!".into(), false)],
            "the cluster was cut in two"
        );

        // ...and from the other end: a run that starts inside the cluster is
        // snapped back to the front of it rather than into it.
        let mut line = Line::from(vec![Span::raw(format!("{family}!"))]);
        restyle(&mut line, 4, 1, hit());
        assert_eq!(
            parts(&line),
            [(family.to_string(), true), ("!".into(), false)]
        );
    }

    #[test]
    fn the_snap_stops_exactly_where_the_panes_own_width_model_already_stopped() {
        // Documented rather than fixed, and pinned so the documentation cannot
        // quietly start claiming more. A flag is two regional indicators, each
        // with a width of its own, so this splits it — and `unicode_width` has
        // been measuring that row as two characters since before there was a
        // search, so the split leaves it no wider than `wrap` already believed.
        let flag = "\u{1f1ec}\u{1f1e7}";
        let mut line = Line::from(vec![Span::raw(flag.to_string())]);
        restyle(&mut line, 0, 1, hit());
        assert_eq!(
            parts(&line),
            [("\u{1f1ec}".into(), true), ("\u{1f1e7}".into(), false)]
        );
    }

    #[test]
    fn a_run_at_either_edge_does_not_grow_an_empty_span() {
        let mut line = Line::from(vec![Span::raw("abcd".to_string())]);
        restyle(&mut line, 0, 2, hit());
        assert_eq!(parts(&line), [("ab".into(), true), ("cd".into(), false)]);

        let mut line = Line::from(vec![Span::raw("abcd".to_string())]);
        restyle(&mut line, 2, 2, hit());
        assert_eq!(parts(&line), [("ab".into(), false), ("cd".into(), true)]);

        // A whole line, and a run of nothing at all.
        let mut line = Line::from(vec![Span::raw("abcd".to_string())]);
        restyle(&mut line, 0, 4, hit());
        assert_eq!(parts(&line), [("abcd".into(), true)]);
        restyle(&mut line, 1, 0, hit());
        assert_eq!(parts(&line), [("abcd".into(), true)], "nothing to restyle");
    }

    #[test]
    fn a_block_of_prose_fits_the_width_it_was_given() {
        let lines = block("one two three four five six", 10, dim());
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(wrap::spans_width(&line.spans) <= 10);
        }
    }
}
