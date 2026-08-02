//! The pty instrument, kept because it earned its place.
//!
//! Every field here was, at some point during the spike, the difference between
//! "the agent doesn't work" and a diagnosis. The one to look at first is **DSR
//! answered**: ConPTY opens a session by asking where the cursor is and blocks
//! until it is told, so a zero there — shown red — is not a statistic, it is an
//! imminent hang. `bytes_read` frozen while the child is alive means the reader
//! thread died. The two sizes are here to be compared with the pane you can see
//! — a hosted app wrapping in the wrong place is a size that does not match the
//! rect. All of it is written up in `docs/conpty-findings.md`.
//!
//! It is a third right-hand view rather than a startup flag because the failures
//! it explains are not reproducible on demand — you want it *while* the thing is
//! going wrong, without restarting the session you are trying to observe. F2
//! shows it and F2 puts back whatever was there before.
//!
//! The pane holds a snapshot rather than the session. It is reached through
//! `&mut dyn Pane` like every other pane, which rules out borrowing the terminal
//! pane it describes; the shell refreshes the snapshot on the frames that show
//! it, and on no others.

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::pane::{Handled, Pane};
use crate::panes::terminal::Diagnostics;
use crate::scroll::Scroll;
use crate::text::{self, clip_line, dim};

/// Wide enough for the longest label plus a value, narrow enough to leave room
/// for one in a 40% pane of an 80-column window.
const LABEL: usize = 16;

/// What the draw loop is managing, frozen at one instant.
///
/// Here rather than in [`Diagnostics`] because it describes the loop and not
/// the pty — and it is here at all because the alternative is guessing. The
/// question "is abeam keeping up" was, for the whole of its life before this,
/// answerable only by looking at the screen and forming an opinion.
///
/// Read them together: `fps` is what the last full second managed, and `worst`
/// is the single slowest frame in it. A healthy left pane under load sits at
/// the frame floor with a worst well under it. A `worst` that approaches the
/// gap between frames is the renderer, not the pacing.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub drawn: u64,
    pub last_ms: f32,
    pub worst_ms: f32,
    pub fps: f32,
}

pub struct DiagPane {
    /// `None` until the shell has shown this view once. It cannot be filled in
    /// at construction: the pane it describes is built first and owned
    /// elsewhere.
    state: Option<Diagnostics>,
    frames: Option<FrameStats>,
    scroll: Scroll,
}

impl DiagPane {
    pub fn new() -> Self {
        Self {
            state: None,
            frames: None,
            scroll: Scroll::default(),
        }
    }

    /// Called by the shell immediately before rendering, and only then. The
    /// numbers are live enough to watch a resize land.
    pub fn update(&mut self, state: Diagnostics) {
        self.state = Some(state);
    }

    /// The same contract, from the same place, for the numbers the shell owns
    /// rather than the pty.
    pub fn update_frames(&mut self, frames: FrameStats) {
        self.frames = Some(frames);
    }
}

impl Pane for DiagPane {
    fn title(&self) -> String {
        match &self.state {
            // Red in the body rather than the border: a title that shouts is a
            // title you stop reading.
            Some(d) if d.dsr_replies == 0 => "pty · no DSR reply".into(),
            Some(d) if d.reader_finished => "pty · closed".into(),
            _ => "pty diagnostics".into(),
        }
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let lines = match &self.state {
            Some(d) => rows(d, self.frames, inner.width as usize),
            None => vec![Line::from(Span::styled("no session", dim()))],
        };

        self.scroll.measure(lines.len(), inner.height as usize);
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(self.scroll.offset)
            .take(inner.height as usize)
            .collect();
        f.render_widget(Paragraph::new(visible), inner);
    }

    /// The same scroll vocabulary as the other two panes, from the same place —
    /// this pane used to have a hand-written subset of it, so `G` and `End` did
    /// nothing here and the bottom of a long report was unreachable. Esc and q
    /// fall through, as always.
    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        Ok(self.scroll.key(key).unwrap_or(Handled::No))
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        Ok(self.scroll.mouse(ev).unwrap_or(Handled::No))
    }
}

fn heading(s: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        s,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn row(label: &str, value: Span<'static>) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{label:<LABEL$}"), dim()), value])
}

fn flag(on: bool) -> Span<'static> {
    if on {
        Span::styled("on", Style::default().fg(Color::Green))
    } else {
        Span::styled("off", dim())
    }
}

fn rows(d: &Diagnostics, f: Option<FrameStats>, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        heading("emulation state"),
        Line::default(),
        row("alt screen", flag(d.alt_screen)),
        row("app cursor", flag(d.app_cursor)),
        row("app keypad", flag(d.app_keypad)),
        row("bracketed paste", flag(d.bracketed_paste)),
        row(
            "mouse mode",
            Span::styled(d.mouse_mode.clone(), Style::default().fg(Color::Yellow)),
        ),
        row(
            "mouse encoding",
            Span::styled(d.mouse_encoding.clone(), Style::default().fg(Color::Yellow)),
        ),
        Line::default(),
        heading("plumbing"),
        Line::default(),
    ];

    // Shown as cols x rows, which is the order everyone says sizes in, and side
    // by side because what matters is whether they match the pane you can see.
    //
    // Deliberately *not* flagged when they differ. portable-pty answers
    // `get_size` on Windows from a field it wrote itself during the last
    // successful resize, and `PtySession::resize` updates the parser from the
    // same call — so the two are structurally incapable of disagreeing, and a
    // red row here would be an instrument reporting on itself. What they are
    // good for is the check no code can make: reading them off the screen and
    // comparing them with the inner area of the pane you are looking at.
    let pty = match d.pty_size {
        Some((r, c)) => format!("{c}x{r}"),
        None => "unknown".to_string(),
    };
    let (prows, pcols) = d.parser_size;
    lines.push(row("pty size (set)", Span::raw(pty)));
    lines.push(row("parser size", Span::raw(format!("{pcols}x{prows}"))));
    lines.push(row("bytes read", Span::raw(d.bytes_read.to_string())));
    lines.push(row(
        "DSR answered",
        Span::styled(
            d.dsr_replies.to_string(),
            Style::default().fg(if d.dsr_replies > 0 {
                Color::Green
            } else {
                Color::Red
            }),
        ),
    ));
    lines.push(row("keys sent", Span::raw(d.keys_sent.to_string())));
    lines.push(row("resizes", Span::raw(d.resizes.to_string())));
    lines.push(row(
        "cursor",
        Span::raw(format!("{},{}", d.cursor.1, d.cursor.0)),
    ));

    if let Some(f) = f {
        lines.push(Line::default());
        lines.push(heading("drawing"));
        lines.push(Line::default());
        lines.push(row("frames", Span::raw(f.drawn.to_string())));
        // Idle is the common case and reports zero, which is the truth and not
        // a fault: a loop with nothing to draw draws nothing. The number to
        // read is what it climbs to while the agent is producing output.
        lines.push(row("fps (last 1s)", Span::raw(format!("{:.0}", f.fps))));
        lines.push(row("last frame", Span::raw(format!("{:.2} ms", f.last_ms))));
        // The one worth watching. An average would hide the single slow frame
        // that is the entire experience of a stutter.
        lines.push(row(
            "worst frame",
            Span::styled(
                format!("{:.2} ms", f.worst_ms),
                // Amber once a frame costs more than the gap the pacing is
                // trying to hold, because at that point the renderer is what is
                // setting the rate and no amount of pacing will help.
                if f.worst_ms >= 8.0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                },
            ),
        ));
    }

    if d.dsr_replies == 0 {
        lines.push(Line::default());
        lines.extend(note(
            "No DSR reply yet. ConPTY blocks until the query is answered, so if \
             this stays at zero the session is hung, not slow.",
            width,
            Color::Red,
        ));
    }
    if d.reader_finished {
        lines.push(Line::default());
        lines.extend(note(
            "The reader thread has finished: no more output will arrive.",
            width,
            Color::Red,
        ));
    }
    if let Some(status) = &d.exited {
        lines.push(Line::default());
        lines.extend(note(
            &format!("Child exited: {status}"),
            width,
            Color::Yellow,
        ));
    }

    lines.into_iter().map(|l| clip_line(l, width)).collect()
}

/// Wrapped rather than handed to `Paragraph::wrap`, because the pane scrolls by
/// physical row and a widget that reflows at draw time cannot be scrolled.
/// `crate::text` does the wrapping, so a token wider than the pane is broken
/// rather than left to overflow and be clipped away without a word of warning.
fn note(body: &str, width: usize, colour: Color) -> Vec<Line<'static>> {
    text::block(body, width.max(1), Style::default().fg(colour))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use unicode_width::UnicodeWidthStr;

    fn sample() -> Diagnostics {
        Diagnostics {
            alt_screen: true,
            app_cursor: true,
            app_keypad: false,
            bracketed_paste: true,
            mouse_mode: "Press".into(),
            mouse_encoding: "Sgr".into(),
            cursor: (3, 12),
            parser_size: (22, 78),
            pty_size: Some((22, 78)),
            bytes_read: 4096,
            dsr_replies: 1,
            keys_sent: 17,
            resizes: 2,
            reader_finished: false,
            exited: None,
        }
    }

    fn styles_of(lines: &[Line<'_>], label: &str) -> Vec<Style> {
        lines
            .iter()
            .filter(|l| l.spans.first().is_some_and(|s| s.content.contains(label)))
            .flat_map(|l| l.spans.iter().skip(1).map(|s| s.style))
            .collect()
    }

    #[test]
    fn an_unanswered_dsr_query_is_red_and_says_why() {
        // The single most useful thing this pane does. A zero here means the
        // session is hung on ConPTY's opening question, and the spike burned
        // days on that before the counter existed.
        let mut d = sample();
        d.dsr_replies = 0;
        let lines = rows(&d, None, 46);
        assert_eq!(
            styles_of(&lines, "DSR answered"),
            [Style::default().fg(Color::Red)]
        );

        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("ConPTY blocks"), "got: {text}");

        // ...and green once it has been answered, with no alarm attached.
        let lines = rows(&sample(), None, 46);
        assert_eq!(
            styles_of(&lines, "DSR answered"),
            [Style::default().fg(Color::Green)]
        );
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(!text.contains("ConPTY blocks"));
    }

    #[test]
    fn both_sizes_are_reported_and_neither_is_dressed_up_as_a_check() {
        // The pane used to colour the two rows red when they differed, which
        // read as a check being performed. It is not one: portable-pty answers
        // `get_size` from its own cache and `PtySession::resize` writes the
        // parser from the same call, so they cannot disagree. Showing both is
        // still worth it — the comparison a human makes is against the pane on
        // screen — but the instrument must not claim more than it does.
        let mut d = sample();
        d.pty_size = Some((22, 80));
        let text: String = rows(&d, None, 46)
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("80x22"), "got: {text}");
        assert!(text.contains("78x22"), "got: {text}");
        for style in styles_of(&rows(&d, None, 46), "parser size") {
            assert_ne!(style.fg, Some(Color::Red), "not a check, so not an alarm");
        }
    }

    #[test]
    fn the_whole_scroll_vocabulary_works_here_too() {
        // This pane had a hand-written subset: no G, no End, no half page. In a
        // short window that made the bottom of the report unreachable, and the
        // key the F1 overlay advertises silently dead.
        let mut pane = DiagPane::new();
        pane.update(sample());
        let backend = ratatui::backend::TestBackend::new(30, 6);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| pane.render(f, f.area())).unwrap();
        assert!(pane.scroll.max() > 0, "the sample must overflow six rows");

        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert_eq!(pane.handle_key(key(KeyCode::Char('G'))).unwrap(), Handled::Yes);
        assert_eq!(pane.scroll.offset, pane.scroll.max());
        pane.handle_key(key(KeyCode::Char('g'))).unwrap();
        assert_eq!(pane.scroll.offset, 0);
        pane.handle_key(key(KeyCode::End)).unwrap();
        assert_eq!(pane.scroll.offset, pane.scroll.max());
        pane.handle_key(key(KeyCode::Home)).unwrap();
        assert_eq!(pane.scroll.offset, 0);
        pane.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(pane.scroll.offset, 3);

        // ...and Esc still is not ours.
        assert_eq!(pane.handle_key(key(KeyCode::Esc)).unwrap(), Handled::No);
    }

    #[test]
    fn the_frame_clock_is_reported_and_a_slow_frame_is_flagged() {
        // The reason this pane now says anything about drawing at all: abeam
        // shipped with a 10 ms poll in front of a renderer that turned out to
        // cost 0.75 ms, and nothing on screen could have told you that.
        let text = |f| -> String {
            rows(&sample(), Some(f), 46)
                .iter()
                .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
                .collect()
        };

        let healthy = FrameStats {
            drawn: 1200,
            last_ms: 0.71,
            worst_ms: 2.40,
            fps: 118.0,
        };
        let t = text(healthy);
        assert!(t.contains("118"), "got: {t}");
        assert!(t.contains("0.71 ms"), "got: {t}");
        assert!(t.contains("2.40 ms"), "got: {t}");
        assert_eq!(
            styles_of(&rows(&sample(), Some(healthy), 46), "worst frame"),
            [Style::default()],
            "a frame inside the floor is not worth colouring"
        );

        // Once the worst frame costs more than the gap the pacing is holding,
        // the renderer is setting the rate and the pacing cannot help.
        let slow = FrameStats {
            worst_ms: 9.5,
            ..healthy
        };
        assert_eq!(
            styles_of(&rows(&sample(), Some(slow), 46), "worst frame"),
            [Style::default().fg(Color::Yellow)]
        );

        // ...and a pane that has never been drawn says nothing rather than
        // reporting a confident zero.
        assert!(!text(healthy).is_empty());
        let none: String = rows(&sample(), None, 46)
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(!none.contains("worst frame"), "got: {none}");
    }

    #[test]
    fn every_row_fits_the_pane_at_any_width() {
        // The right pane can be 22 columns wide in a 60-column window, and a
        // row that overflows its rect corrupts the frame.
        let mut d = sample();
        d.dsr_replies = 0;
        d.reader_finished = true;
        d.exited = Some("ExitStatus { .. }".into());
        let f = Some(FrameStats {
            drawn: 999_999,
            last_ms: 12.345,
            worst_ms: 123.456,
            fps: 125.0,
        });
        for width in [1usize, 8, 17, 22, 46, 100] {
            for line in rows(&d, f, width) {
                let w: usize = line.spans.iter().map(|s| s.content.width()).sum();
                assert!(w <= width, "{w} cells at width {width}: {line:?}");
            }
        }
    }

    #[test]
    fn a_pane_that_has_never_been_shown_renders_instead_of_panicking() {
        // The shell only fills the snapshot in on frames that display this
        // view, so the empty state is reachable and has to draw something.
        let mut pane = DiagPane::new();
        let backend = ratatui::backend::TestBackend::new(20, 4);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        term.draw(|f| pane.render(f, f.area())).unwrap();
    }
}
