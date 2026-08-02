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

    #[test]
    fn a_block_of_prose_fits_the_width_it_was_given() {
        let lines = block("one two three four five six", 10, dim());
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(wrap::spans_width(&line.spans) <= 10);
        }
    }
}
