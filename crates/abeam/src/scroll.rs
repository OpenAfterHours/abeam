//! One scroll vocabulary, implemented once.
//!
//! All three right-hand panes scroll, and the F1 overlay promises the same keys
//! in each of them: `j`/`k` and arrows, `space`/`b` and PgDn/PgUp, `Ctrl+D` /
//! `Ctrl+U`, `g`/`G` and Home/End, and the wheel. Written out per pane, that
//! promise held in two of the three — the diagnostics view had no `G`, no `End`
//! and no half-page, so the bottom of a long report was unreachable and the key
//! was silently dead. The table is true here by construction instead.
//!
//! Scrolling is by *physical row* everywhere: `len` is the number of rows the
//! pane has laid out, not logical lines. That is what makes `G` land where it
//! was asked to land in a pane full of wrapped text.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::pane::Handled;
use crate::text::dim;

/// Rows moved by one notch of the wheel. Three is the terminal convention and
/// no pane has any business inventing its own.
const WHEEL: usize = 3;

/// Narrower than this and the scrollbar's column is worth more as text.
const BAR_MIN_WIDTH: u16 = 24;

/// Columns a pane must keep back for the bar.
///
/// Reserved whether or not the bar is currently drawn. Deciding per frame makes
/// every row shift one column sideways the moment the content crosses the pane
/// height — and in the viewer it would re-wrap the whole document to do it.
pub fn bar_width(pane_width: u16) -> u16 {
    u16::from(pane_width >= BAR_MIN_WIDTH)
}

/// A pane's scroll position, and the keys that move it.
///
/// `len` and `viewport` are refreshed by the pane on the frames it draws —
/// there is no other honest moment, because both depend on the rect it was
/// given. Between frames they are the last known good values, which is exactly
/// what a key pressed before the next frame should act on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Scroll {
    pub offset: usize,
    len: usize,
    viewport: usize,
}

impl Scroll {
    /// Tell it what the last frame actually laid out. Clamps the offset, so a
    /// document that shrank cannot leave the view past its end.
    pub fn measure(&mut self, len: usize, viewport: usize) {
        self.len = len;
        self.viewport = viewport;
        self.offset = self.offset.min(self.max());
    }

    pub fn max(&self) -> usize {
        self.len.saturating_sub(self.viewport)
    }

    pub fn viewport(&self) -> usize {
        self.viewport
    }

    /// One line of overlap, so the eye has an anchor across the jump.
    fn page(&self) -> usize {
        self.viewport.saturating_sub(1).max(1)
    }

    fn half(&self) -> usize {
        (self.viewport / 2).max(1)
    }

    /// `Handled::No` when nothing moved: a key that changes nothing has not
    /// been acted on, and the shell reads that as the pane declining it.
    pub fn to(&mut self, to: usize) -> Handled {
        let to = to.min(self.max());
        let moved = to != self.offset;
        self.offset = to;
        moved.into()
    }

    pub fn by(&mut self, delta: isize) -> Handled {
        let to = if delta < 0 {
            self.offset.saturating_sub(delta.unsigned_abs())
        } else {
            self.offset.saturating_add(delta as usize)
        };
        self.to(to)
    }

    /// `None` for a key this vocabulary has no opinion about, so the pane can
    /// go on to match its own — `Tab`, `Enter`, `r` — and so `Esc` and `q` fall
    /// through to the shell as "give focus back to Claude".
    pub fn key(&mut self, key: KeyEvent) -> Option<Handled> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        Some(match key.code {
            KeyCode::Char('d') if ctrl => self.by(self.half() as isize),
            KeyCode::Char('u') if ctrl => self.by(-(self.half() as isize)),
            // Ctrl+letter is Claude's everywhere else in the program; inside a
            // focused pane only the two above mean anything, and the rest must
            // not be swallowed by the plain-letter arms below.
            KeyCode::Char(_) if ctrl => return None,

            KeyCode::Char('j') | KeyCode::Down => self.by(1),
            KeyCode::Char('k') | KeyCode::Up => self.by(-1),
            KeyCode::Char(' ') | KeyCode::PageDown => self.by(self.page() as isize),
            KeyCode::Char('b') | KeyCode::PageUp => self.by(-(self.page() as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.to(0),
            KeyCode::Char('G') | KeyCode::End => self.to(usize::MAX),
            _ => return None,
        })
    }

    pub fn mouse(&mut self, ev: &MouseEvent) -> Option<Handled> {
        Some(match ev.kind {
            MouseEventKind::ScrollUp => self.by(-(WHEEL as isize)),
            MouseEventKind::ScrollDown => self.by(WHEEL as isize),
            _ => return None,
        })
    }

    /// Draw the bar into the column [`bar_width`] reserved for it, and only
    /// when there is something to scroll.
    ///
    /// `content_length` is the number of distinct offsets rather than the row
    /// count, which is what makes the thumb reach the bottom of the track
    /// exactly when the last row is on screen. Getting that convention wrong is
    /// invisible in a screenshot and obvious in use.
    pub fn render_bar(&self, f: &mut Frame, area: Rect) {
        if self.max() == 0 || bar_width(area.width) == 0 {
            return;
        }
        let mut state = ScrollbarState::new(self.max() + 1)
            .viewport_content_length(self.viewport)
            .position(self.offset);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(dim()),
            area,
            &mut state,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// A hundred rows in a ten-row pane.
    fn scroll() -> Scroll {
        let mut s = Scroll::default();
        s.measure(100, 10);
        s
    }

    #[test]
    fn every_key_the_help_overlay_advertises_moves_the_view() {
        // The whole reason this type exists: the F1 table is one list and it
        // has to be true in every pane that holds one of these.
        let mut s = scroll();
        assert_eq!(s.key(key(KeyCode::Char('j'))), Some(Handled::Yes));
        assert_eq!(s.offset, 1);
        assert_eq!(s.key(key(KeyCode::Down)), Some(Handled::Yes));
        assert_eq!(s.offset, 2);
        s.key(key(KeyCode::Char(' ')));
        assert_eq!(s.offset, 2 + 9, "a page keeps one line of overlap");
        s.key(key(KeyCode::Char('b')));
        assert_eq!(s.offset, 2);
        s.key(ctrl('d'));
        assert_eq!(s.offset, 7);
        s.key(ctrl('u'));
        assert_eq!(s.offset, 2);
        s.key(key(KeyCode::PageDown));
        assert_eq!(s.offset, 11);
        s.key(key(KeyCode::PageUp));
        assert_eq!(s.offset, 2);
        s.key(key(KeyCode::Char('k')));
        assert_eq!(s.offset, 1);

        s.key(key(KeyCode::Char('G')));
        assert_eq!(s.offset, 90, "G reaches the last screenful and no further");
        s.key(key(KeyCode::Char('g')));
        assert_eq!(s.offset, 0);
        s.key(key(KeyCode::End));
        assert_eq!(s.offset, 90);
        s.key(key(KeyCode::Home));
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn scrolling_stops_at_both_ends_and_says_it_did_nothing() {
        let mut s = scroll();
        assert_eq!(s.key(key(KeyCode::Char('k'))), Some(Handled::No));
        assert_eq!(s.offset, 0);
        s.key(key(KeyCode::Char('G')));
        assert_eq!(s.key(key(KeyCode::Char('j'))), Some(Handled::No));
        assert_eq!(s.offset, s.max());
    }

    #[test]
    fn a_view_bigger_than_its_content_does_not_scroll_at_all() {
        let mut s = Scroll::default();
        s.measure(4, 20);
        assert_eq!(s.max(), 0);
        assert_eq!(s.key(key(KeyCode::Char('G'))), Some(Handled::No));
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn a_document_that_shrank_takes_the_offset_with_it() {
        let mut s = scroll();
        s.key(key(KeyCode::Char('G')));
        assert_eq!(s.offset, 90);
        s.measure(20, 10);
        assert_eq!(s.offset, 10, "still on the last screenful, not past the end");
    }

    #[test]
    fn keys_that_are_not_ours_are_left_for_the_pane_and_the_shell() {
        let mut s = scroll();
        for code in [
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Enter,
            KeyCode::Char('r'),
        ] {
            assert_eq!(s.key(key(code)), None, "{code:?} is not a scroll key");
        }
        // Ctrl+C must reach nothing here; only Ctrl+D and Ctrl+U are claimed.
        assert_eq!(s.key(ctrl('c')), None);
        assert_eq!(s.key(ctrl('g')), None);
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn the_wheel_moves_three_rows_and_ignores_everything_else() {
        let mut s = scroll();
        let ev = |kind| MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        s.mouse(&ev(MouseEventKind::ScrollDown));
        assert_eq!(s.offset, WHEEL);
        s.mouse(&ev(MouseEventKind::ScrollUp));
        assert_eq!(s.offset, 0);
        assert_eq!(s.mouse(&ev(MouseEventKind::Moved)), None);
    }
}
