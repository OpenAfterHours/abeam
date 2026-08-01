//! Wrapping styled text to a column count.
//!
//! Everything the viewer draws is pre-wrapped to a known width and handed to a
//! `Paragraph` with wrapping *off*. That is deliberate: ratatui's own wrap
//! reflows at draw time, so the widget's line count and the pane's scroll
//! offset are measured in different units and `G` lands in the wrong place on
//! any document containing a long line. Wrapping here means one unit — the
//! physical row — for scrolling, paging and the scrollbar alike.
//!
//! Two wrappers, because prose and code want different things. Prose reflows on
//! word boundaries and collapses runs of whitespace; code breaks wherever the
//! column runs out, because its whitespace is load-bearing.
//!
//! Both take a *first* and a *rest* prefix so a bullet, a quote gutter or a
//! code gutter is part of the wrap rather than something glued on afterwards —
//! that is what makes a continuation line indent under its own bullet.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Display columns, not bytes and not chars. A CJK ideograph is two cells and
/// `str::len` is wrong about it twice over.
pub fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Word-wrap prose. Runs of whitespace collapse to a single space, which is
/// what CommonMark means by a soft break anyway.
pub fn wrap_spans(
    content: Vec<Span<'static>>,
    width: usize,
    first: &[Span<'static>],
    rest: &[Span<'static>],
) -> Vec<Line<'static>> {
    let first_limit = limit(width, first);
    let rest_limit = limit(width, rest);
    // Split over-long words up front against the *continuation* limit, which
    // is the one that applies to every line but one. A word that survives this
    // fits on any continuation line, so the packer below only ever has to
    // decide *where* a token goes, never whether it can go anywhere. A
    // 90-character URL in a 38-column pane is the normal case here.
    let toks = tokenize(content, rest_limit);

    let mut lines = Vec::new();
    let mut cur = first.to_vec();
    let mut used = 0usize;
    let mut lim = first_limit;
    let mut on_first = true;
    let mut pending_space: Option<Style> = None;

    for tok in toks {
        match tok {
            Tok::Space(style) => {
                // Leading space on a line is dropped, not carried.
                if used > 0 {
                    pending_space = Some(style);
                }
            }
            Tok::Word(text, style) => {
                let w = text.width();
                let gap = usize::from(pending_space.is_some());
                // The second clause is for a wide first-line prefix — a table
                // record's `description: ` label, say. Nothing fits beside it,
                // so the label gets a line and the value starts below it.
                if used + gap + w > lim && (used > 0 || on_first) {
                    lines.push(Line::from(std::mem::take(&mut cur)));
                    cur = rest.to_vec();
                    used = 0;
                    lim = rest_limit;
                    on_first = false;
                    pending_space = None;
                }
                if let Some(style) = pending_space.take() {
                    cur.push(Span::styled(" ", style));
                    used += 1;
                }
                cur.push(Span::styled(text, style));
                used += w;
            }
        }
    }

    // An empty input still produces one line: a blank line inside a block quote
    // has to keep drawing the gutter or the quote looks like two quotes.
    if used > 0 || lines.is_empty() {
        lines.push(Line::from(cur));
    }
    lines
}

/// Break at the column boundary and nowhere else. For code, where a break
/// inserted at a "word" boundary would be a lie about the source.
pub fn hard_wrap(
    content: Vec<Span<'static>>,
    width: usize,
    first: &[Span<'static>],
    rest: &[Span<'static>],
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut cur = first.to_vec();
    let mut used = 0usize;
    let mut lim = limit(width, first);
    let rest_limit = limit(width, rest);

    for span in content {
        let style = span.style;
        let mut buf = String::new();
        for ch in span.content.chars() {
            let w = ch.width().unwrap_or(0);
            if used > 0 && used + w > lim {
                if !buf.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut buf), style));
                }
                lines.push(Line::from(std::mem::take(&mut cur)));
                cur = rest.to_vec();
                used = 0;
                lim = rest_limit;
            }
            buf.push(ch);
            used += w;
        }
        if !buf.is_empty() {
            cur.push(Span::styled(buf, style));
        }
    }
    lines.push(Line::from(cur));
    lines
}

/// Cut a string into chunks no wider than `width` cells. Used for words that
/// cannot fit on a line at all, and by the table layout for cell text.
pub fn split_to_width(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(ch);
        used += w;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Pad `spans` out to `width` cells, or truncate them to it.
pub fn pad_to(mut spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let have = spans_width(&spans);
    if have < width {
        spans.push(Span::raw(" ".repeat(width - have)));
    }
    spans
}

/// Columns left for content once the prefix has had its share. Never zero: a
/// limit of zero makes every packing loop in here run forever.
fn limit(width: usize, prefix: &[Span<'static>]) -> usize {
    width.saturating_sub(spans_width(prefix)).max(1)
}

enum Tok {
    Word(String, Style),
    Space(Style),
}

fn tokenize(spans: Vec<Span<'static>>, max_word: usize) -> Vec<Tok> {
    let mut out = Vec::new();
    for span in spans {
        let style = span.style;
        let mut cur = String::new();
        let mut in_ws = false;
        for ch in span.content.chars() {
            let ws = ch.is_whitespace();
            if ws != in_ws && !cur.is_empty() {
                emit(&mut out, std::mem::take(&mut cur), in_ws, style, max_word);
            }
            in_ws = ws;
            cur.push(ch);
        }
        if !cur.is_empty() {
            emit(&mut out, cur, in_ws, style, max_word);
        }
    }
    out
}

fn emit(out: &mut Vec<Tok>, text: String, is_ws: bool, style: Style, max_word: usize) {
    if is_ws {
        out.push(Tok::Space(style));
    } else if text.width() <= max_word {
        out.push(Tok::Word(text, style));
    } else {
        for chunk in split_to_width(&text, max_word) {
            out.push(Tok::Word(chunk, style));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn spans(s: &str) -> Vec<Span<'static>> {
        vec![Span::raw(s.to_string())]
    }

    fn assert_fits(lines: &[Line<'_>], width: usize) {
        for line in lines {
            assert!(spans_width(&line.spans) <= width, "{:?} overflows", line);
        }
    }

    #[test]
    fn wraps_on_word_boundaries() {
        let out = wrap_spans(spans("the quick brown fox jumps"), 11, &[], &[]);
        assert_eq!(text(&out), ["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_rather_than_dropped() {
        // A bare URL in a 40-column pane. Losing it silently would be worse
        // than an ugly break, and overflowing it makes ratatui truncate.
        let out = wrap_spans(spans("see https://example.com/a/very/long/path"), 12, &[], &[]);
        let joined: String = text(&out).join("");
        assert!(joined.contains("https://example.com/a/very/long/path"));
        for line in &out {
            assert!(spans_width(&line.spans) <= 12, "{:?} overflows", line);
        }
    }

    #[test]
    fn continuation_lines_indent_under_the_bullet() {
        let bullet = vec![Span::raw("• ")];
        let hang = vec![Span::raw("  ")];
        let out = wrap_spans(spans("alpha beta gamma delta"), 12, &bullet, &hang);
        assert_eq!(text(&out), ["• alpha beta", "  gamma", "  delta"]);
    }

    #[test]
    fn style_survives_a_break_in_the_middle_of_a_span() {
        let content = vec![
            Span::styled("bold words here", Style::default().fg(ratatui::style::Color::Red)),
        ];
        let out = wrap_spans(content, 6, &[], &[]);
        assert_eq!(text(&out), ["bold", "words", "here"]);
        for line in &out {
            for span in &line.spans {
                assert_eq!(span.style.fg, Some(ratatui::style::Color::Red));
            }
        }
    }

    #[test]
    fn hard_wrap_keeps_the_whitespace_code_depends_on() {
        let out = hard_wrap(spans("    indented(arg)"), 10, &[], &[]);
        assert_eq!(text(&out), ["    indent", "ed(arg)"]);
    }

    #[test]
    fn a_first_prefix_that_fills_the_line_pushes_the_content_below_it() {
        // The narrow-table fallback: `description: ` leaves nothing beside it,
        // and overflowing the pane to keep them on one row would be worse.
        let label = vec![Span::raw("description: ")];
        let hang = vec![Span::raw("  ")];
        let out = wrap_spans(spans("the first one"), 14, &label, &hang);
        assert_eq!(text(&out), ["description: ", "  the first", "  one"]);
        assert_fits(&out, 14);
    }

    #[test]
    fn an_empty_block_still_draws_its_gutter() {
        let gutter = vec![Span::raw("│ ")];
        let out = wrap_spans(vec![], 20, &gutter, &gutter);
        assert_eq!(text(&out), ["│ "]);
    }

    #[test]
    fn a_prefix_wider_than_the_pane_does_not_hang() {
        // Deeply nested list in a 4-column pane. The limit floors at 1 so the
        // packing loop always makes progress; it looks terrible and terminates,
        // which is the correct trade at this width.
        let deep = vec![Span::raw("            • ")];
        let out = wrap_spans(spans("alpha beta"), 4, &deep, &deep);
        assert!(!out.is_empty());
    }

    #[test]
    fn wide_characters_are_measured_in_cells_not_chars() {
        // Four ideographs are eight cells, so only three fit in seven columns.
        let out = wrap_spans(spans("日本語版"), 7, &[], &[]);
        assert_eq!(text(&out), ["日本語", "版"]);
    }
}
