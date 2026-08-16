//! One selection over the right pane, and the keys that move it.
//!
//! What this exists for is the round trip abeam otherwise cannot make: a
//! command runs in the shell view, and the thing it printed has to reach the
//! agent's composer on the other side of the divider. Before this there was no
//! route at all. `EnableMouseCapture` takes the host terminal's own drag-select
//! away (`crate::term`), and even without it a drag over the right pane crosses
//! both panes and the border between them, because the two share every screen
//! row — a linear selection would hand back the agent's text with the shell's
//! interleaved a column at a time.
//!
//! ## What a selection *is* here
//!
//! **Whole rows, of the pane, as they are on screen.** Not a range in the
//! content behind them, and the difference is worth stating because it is the
//! one thing that could surprise: scroll the pane under a selection and the
//! highlight stays where it is, naming whatever is under it now. The text is
//! read at the moment `y` or `Enter` is pressed, so what is highlighted is
//! always exactly what will be copied — which is the property worth keeping,
//! and the only one a rule this simple can promise.
//!
//! Rows rather than cells because the six panes on the right are six different
//! kinds of thing. The shell is a terminal grid; the reader is wrapped markdown
//! with quote gutters and bullet indents; git is a column-aligned status list.
//! A cell-precise selection means something different in each of them, and
//! three of the six would need it drawn in a coordinate space they do not have.
//! A row means the same thing in all six, and a row is what somebody copying a
//! path, a hash, a stack trace or a test failure is after.
//!
//! ## Why the vocabulary is the scroll vocabulary
//!
//! `j`/`k`, `space`/`b`, `Ctrl+D`/`Ctrl+U`, `g`/`G` and the arrows move the
//! caret here, and they are the same keys `crate::scroll` moves a view with —
//! deliberately, because the F1 overlay already promises them and a mode that
//! rebound them would be a mode nobody could hold in their head. What is new is
//! three keys and no more: `v` anchors, `y` copies, `Enter` sends.
//!
//! The caret opens on the **first** row rather than the last, which costs the
//! shell case one keystroke — output collects at the bottom of a terminal, so
//! `G` is usually the first thing pressed there. It is one rule for six panes
//! instead of a rule per pane: the reader, git, the queue and the diagnostics
//! view all put their first line of content at the top, and a caret that landed
//! somewhere different depending on which view was up is a caret nobody can
//! predict.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::pane::Handled;

/// Rows moved by one notch of the wheel, matching `crate::scroll`.
const WHEEL: i32 = 3;

/// A linewise selection over the rows of whichever pane is on the right.
///
/// Held by `App` rather than by a pane, because it is not any one pane's: the
/// same three keys have to work over the shell, the reader, git, the queue, the
/// ask and the diagnostics view, and none of those six can be asked to
/// implement a selection to be copied *from*.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Select {
    /// The row the caret is on, pane-relative and 0-based.
    caret: u16,
    /// Where a selection was anchored, if `v` or a drag has anchored one.
    /// `None` is a caret on a single row, which is still a selection of one.
    anchor: Option<u16>,
    /// Rows the pane was drawn with, as of the last frame. Refreshed by
    /// [`measure`](Self::measure); until the first frame it is `0` and the
    /// caret cannot move, which is the honest answer for a pane whose height
    /// nothing has established yet.
    viewport: u16,
    /// What the last `y` did, until the next key.
    ///
    /// OSC 52 is a write with no reply — a terminal that ignored it says
    /// nothing and a terminal that honoured it says nothing either — so a copy
    /// that reported nothing would be indistinguishable from a dead key. This
    /// is the whole of the acknowledgement, and it is deliberately not needed
    /// for `Enter`: text arriving in the agent's composer shows itself.
    note: Option<String>,
}

impl Select {
    /// A caret on the first row, with nothing anchored.
    pub fn new() -> Self {
        Self::default()
    }

    /// A selection dragged with the mouse: anchored where the button went down,
    /// reaching to wherever the pointer is now.
    ///
    /// Rebuilt from scratch on every drag event rather than mutated, and that is
    /// what keeps the drag stateless: both ends are facts the app already has —
    /// the row of the press it remembered, and the row of the event in hand — so
    /// there is no third thing to keep in step with them.
    pub fn dragged(from: u16, to: u16) -> Self {
        Self {
            caret: to,
            anchor: Some(from),
            viewport: 0,
            note: None,
        }
    }

    /// Tell it how tall the pane was drawn, and clamp into it.
    ///
    /// Called on every frame a selection is up. A pane that shrank — a window
    /// resize, a zoom — must not leave the caret off the bottom of it, because
    /// the rows it names are read out of the frame that was drawn.
    pub fn measure(&mut self, rows: u16) {
        self.viewport = rows;
        let last = rows.saturating_sub(1);
        self.caret = self.caret.min(last);
        self.anchor = self.anchor.map(|anchor| anchor.min(last));
    }

    /// The inclusive span of pane rows this selection names, lowest first.
    pub fn rows(&self) -> (u16, u16) {
        match self.anchor {
            Some(anchor) if anchor < self.caret => (anchor, self.caret),
            Some(anchor) => (self.caret, anchor),
            None => (self.caret, self.caret),
        }
    }

    /// How many rows that is. Never zero: a caret with no anchor selects the
    /// row it is on, so there is no state in which `y` has nothing to act on.
    pub fn height(&self) -> u16 {
        let (lo, hi) = self.rows();
        hi - lo + 1
    }

    pub fn anchored(&self) -> bool {
        self.anchor.is_some()
    }

    /// `v`: drop an anchor where the caret is, or pick it up again.
    pub fn toggle_anchor(&mut self) {
        self.note = None;
        self.anchor = match self.anchor {
            Some(_) => None,
            None => Some(self.caret),
        };
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Say something about the last copy, until the next key.
    pub fn say(&mut self, note: impl Into<String>) {
        self.note = Some(note.into());
    }

    /// Put the caret on `row`, dragging the far end of the selection with it
    /// when nothing is anchored. Clamped to the pane.
    pub fn to(&mut self, row: u16) -> Handled {
        let row = row.min(self.viewport.saturating_sub(1));
        let moved = row != self.caret;
        self.caret = row;
        moved.into()
    }

    fn by(&mut self, delta: i32) -> Handled {
        let to = i32::from(self.caret).saturating_add(delta).max(0);
        self.to(u16::try_from(to).unwrap_or(u16::MAX))
    }

    /// One row of overlap, so the eye has an anchor across the jump — the same
    /// arithmetic as `crate::scroll::Scroll::page`.
    fn page(&self) -> i32 {
        i32::from(self.viewport.saturating_sub(1).max(1))
    }

    fn half(&self) -> i32 {
        i32::from((self.viewport / 2).max(1))
    }

    /// The motion half of the vocabulary, or `None` for a key it has no opinion
    /// about — which is what lets `v`, `y`, `Enter`, `Esc` and `q` be matched
    /// after it by the caller, exactly as [`crate::scroll::Scroll::key`] leaves
    /// room for a pane's own keys.
    ///
    /// Any motion it claims clears the note: a reader who has pressed another
    /// key has read it.
    pub fn key(&mut self, key: KeyEvent) -> Option<Handled> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let moved = match key.code {
            KeyCode::Char('d') if ctrl => self.by(self.half()),
            KeyCode::Char('u') if ctrl => self.by(-self.half()),
            // Ctrl+letter belongs to whatever is hosted, everywhere else in the
            // program. Nothing is hosted while a selection is up — this mode
            // swallows every key — but the two arms above are the only two this
            // vocabulary has ever claimed, and the plain-letter arms below must
            // not quietly take the rest.
            KeyCode::Char(_) if ctrl => return None,

            KeyCode::Char('j') | KeyCode::Down => self.by(1),
            KeyCode::Char('k') | KeyCode::Up => self.by(-1),
            KeyCode::Char(' ') | KeyCode::PageDown => self.by(self.page()),
            KeyCode::Char('b') | KeyCode::PageUp => self.by(-self.page()),
            KeyCode::Char('g') | KeyCode::Home => self.to(0),
            KeyCode::Char('G') | KeyCode::End => self.to(u16::MAX),
            _ => return None,
        };
        self.note = None;
        Some(moved)
    }

    /// A wheel notch, while a selection is up.
    ///
    /// It moves the *caret*, not the view, and that is the one place this
    /// deviates from `crate::scroll`: a pane scrolling under a fixed highlight
    /// is the surprise this module's own contract warns about, so while a
    /// selection is up the wheel is a motion like `j` and `k`. Alt+J / Alt+K
    /// still scroll the pane itself, for anyone who wants exactly that.
    pub fn wheel(&mut self, up: bool) -> Handled {
        let moved = if up { self.by(-WHEEL) } else { self.by(WHEEL) };
        self.note = None;
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(rows: u16) -> Select {
        let mut sel = Select::new();
        sel.measure(rows);
        sel
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_caret_with_no_anchor_still_selects_its_own_row() {
        // There is no state in which `y` has nothing to act on, which is what
        // makes "press F7 and copy this line" two keystrokes rather than three.
        let sel = sel(10);
        assert_eq!(sel.rows(), (0, 0));
        assert_eq!(sel.height(), 1);
        assert!(!sel.anchored());
    }

    #[test]
    fn an_anchor_holds_while_the_caret_moves() {
        let mut sel = sel(10);
        sel.by(2);
        sel.toggle_anchor();
        sel.by(3);
        assert_eq!(sel.rows(), (2, 5));
        assert_eq!(sel.height(), 4);
    }

    #[test]
    fn a_selection_upwards_is_the_same_selection() {
        // Normalised on the way out, so every reader of `rows` gets lowest
        // first and nobody has to sort two numbers they were handed.
        let mut sel = sel(10);
        sel.to(7);
        sel.toggle_anchor();
        sel.by(-4);
        assert_eq!(sel.rows(), (3, 7));
    }

    #[test]
    fn nothing_leaves_the_pane() {
        let mut sel = sel(4);
        assert_eq!(sel.key(key(KeyCode::Char('G'))), Some(Handled::Yes));
        assert_eq!(sel.rows(), (3, 3));
        // And a key that would go past the end is not a key that moved
        // anything: the app draws a frame for `Yes`, and a caret sitting on the
        // last row re-drawing on every `j` is a frame spent on nothing.
        assert_eq!(sel.key(key(KeyCode::Char('j'))), Some(Handled::No));
        assert_eq!(sel.key(key(KeyCode::Char('g'))), Some(Handled::Yes));
        assert_eq!(sel.key(key(KeyCode::Char('k'))), Some(Handled::No));
        assert_eq!(sel.rows(), (0, 0));
    }

    #[test]
    fn a_pane_that_shrank_takes_the_selection_with_it() {
        // The rows are read out of the frame that was drawn, so a caret left
        // off the bottom of a resized pane would name a row nothing drew.
        let mut sel = sel(20);
        sel.to(18);
        sel.toggle_anchor();
        sel.to(19);
        sel.measure(5);
        assert_eq!(sel.rows(), (4, 4));
        // Still anchored, because the resize is not the user changing their
        // mind. `v` is the only thing that picks an anchor up.
        assert!(sel.anchored());
    }

    #[test]
    fn an_undrawn_pane_pins_the_caret_at_the_top() {
        // `viewport` is 0 until the first frame measures it, and a mode entered
        // in the same batch of events as the key that revealed the pane is
        // exactly that case.
        let mut sel = Select::new();
        assert_eq!(sel.key(key(KeyCode::Char('G'))), Some(Handled::No));
        assert_eq!(sel.rows(), (0, 0));
    }

    #[test]
    fn the_scroll_vocabulary_is_the_one_the_overlay_promises() {
        // Written out rather than assumed, because these are the keys F1 has
        // promised since before this mode existed. A motion missing from here
        // is a key that silently does nothing in a mode that swallows
        // everything — the worst shape a dead key can have.
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char(' '),
            KeyCode::Char('b'),
            KeyCode::Char('g'),
            KeyCode::Char('G'),
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::PageDown,
            KeyCode::PageUp,
            KeyCode::Home,
            KeyCode::End,
        ] {
            let mut sel = sel(40);
            sel.to(20);
            assert!(sel.key(key(code)).is_some(), "{code:?} moves nothing");
        }
        for ctrl in ['d', 'u'] {
            let mut sel = sel(40);
            sel.to(20);
            assert!(
                sel.key(KeyEvent::new(KeyCode::Char(ctrl), KeyModifiers::CONTROL))
                    .is_some(),
                "Ctrl+{ctrl} moves nothing"
            );
        }
    }

    #[test]
    fn the_keys_this_vocabulary_does_not_claim_fall_through() {
        // `v`, `y` and `Enter` are matched by the caller after this, so
        // claiming them here would be claiming them twice.
        let mut sel = sel(10);
        for code in [
            KeyCode::Char('v'),
            KeyCode::Char('y'),
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Char('q'),
        ] {
            assert_eq!(sel.key(key(code)), None, "{code:?} is the caller's");
        }
        // And no Ctrl+letter beyond the two the scroll table names.
        for c in ['a', 'c', 'l', 'y'] {
            assert_eq!(
                sel.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)),
                None,
                "Ctrl+{c} is not this vocabulary's"
            );
        }
    }

    #[test]
    fn a_drag_is_two_rows_and_no_third_thing() {
        let sel = Select::dragged(9, 2);
        assert_eq!(sel.rows(), (2, 9));
        assert!(sel.anchored());
    }

    #[test]
    fn the_note_lasts_until_the_next_key() {
        let mut sel = sel(10);
        sel.say("copied 3 rows");
        assert_eq!(sel.note(), Some("copied 3 rows"));
        sel.key(key(KeyCode::Char('j')));
        assert_eq!(sel.note(), None, "a reader who pressed a key has read it");
    }
}
