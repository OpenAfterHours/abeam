//! Where a reader is in a list, kept the same way in every list there is.
//!
//! [`crate::scroll`] exists because the F1 overlay promises one set of keys in
//! every pane, and written out per pane that promise held in two of three. This
//! is the same argument one level down. A pane's *view* is a `Scroll`; a pane's
//! *choice* is this. A list has both — the row `Enter` acts on, and the window
//! that row has to stay inside — and the second is only ever right relative to
//! the first.
//!
//! The viewer's directory listing is not the only list of its kind. The find
//! over file names is a second and the results of a search over their contents
//! are a third, all in the same pane and behind the same keys — and each of
//! them would otherwise have been a copy of this. Copies of it do not fail
//! loudly: what goes wrong is `End` landing on
//! the first row of the last page instead of the last row, or a page that keeps
//! no row of overlap, or a selection that stops being brought back on screen
//! when the window is dragged narrower. Each of those is a key that half works,
//! which is harder to notice — and to report — than one that does nothing.
//!
//! ## The table, and not only the state
//!
//! [`Cursor::key`] owns the list flavour of that table, the way `Scroll::key`
//! owns the document one. Owning it is the only way either of them gets to say
//! the promise holds by construction: a type that kept the state and left every
//! caller to wire `End` up to it would have moved the duplication rather than
//! removed it.
//!
//! It is not the only flavour a list can want. A type-to-filter box takes every
//! printable key as text, so the two find boxes in the viewer — over file names
//! in [`super::browse`], over file *contents* in [`super::grep`] — each spell
//! out a shorter table beside this one.
//!
//! Two of them, which is worth being exact about now that it is not one. What
//! they share with this table is the half a filter box cannot take: the paging
//! and half-paging keys, which go on meaning what the F1 overlay says because
//! an open query is not a reason for a documented key to go quietly dead. What
//! they hold separately is the rest, because in a box `j` is a letter — and
//! they hold it *identically*, so it is now the same table written twice rather
//! than a different table for a stated reason. That is real duplication and it
//! is the visible kind: both are a dozen lines in plain sight, both are covered
//! by their own tests, and the failure this module was created to prevent is
//! the silent kind. It is left alone deliberately rather than overlooked.
//!
//! ## The rows belong to the caller
//!
//! Every method that can move takes the row count as an argument rather than
//! the cursor keeping one. The three lists in the viewer are three different
//! `Vec`s and any of them can be replaced underneath the reader — by a
//! directory re-read, by a background walk that finished. The results list is
//! the sharpest case and it arrived last: rows are appended to it *by a worker
//! thread between frames*, so a count cached here would be a copy going stale
//! several times a second, and the frame that noticed would be the one that had
//! already drawn a selection past the end of a list.
//!
//! ## A cursor each, and a view that travels between two of the three
//!
//! Two lists in one pane are two cursors, because the row chosen in the listing
//! has to survive a find opening over it and closing again: that is the whole
//! difference between `Esc` meaning "never mind" and `Esc` meaning "start
//! over".
//!
//! The listing and its find are not two views. How tall the pane is can only be
//! learned from a frame, and a second `Scroll` starting from nothing believes
//! the pane is zero rows tall — so the first `PageDown` after `/`, drained from
//! the same batch of keys and answered before any frame, would move by a single
//! row. So the view travels rather than being copied: [`Cursor::over`] takes it
//! up and [`Cursor::take_view`] brings it back down.
//!
//! The results list does **not** take part in that. It is a separate view of
//! the pane rather than a list raised over another one, so it starts from
//! [`Cursor::new`] and the same guessed viewport `Browser::new` uses — which is
//! what that guess is for, and is why `new` takes one at all.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::pane::Handled;
use crate::scroll::Scroll;

/// Which row of a list is chosen, and where the window onto the rows sits.
#[derive(Debug)]
pub struct Cursor {
    /// The chosen row, as an index into rows this type never sees. Every method
    /// here clamps it to the count it was handed; assignment does not, so a
    /// caller that sets it directly — `browse.rs` does, on a directory it has
    /// just re-read — clamps it itself.
    pub sel: usize,
    /// The window onto the rows: where it sits, and how tall the last frame
    /// found the pane.
    ///
    /// Two things about it are worth knowing before reading anything below. It
    /// travels between cursors rather than there being one per list — see
    /// [`Cursor::over`] — and it carries a row count of its own, which nothing
    /// here consults but which `Scroll::to` clamps against, and which is
    /// therefore stale for as long as it takes a frame to arrive.
    pub scroll: Scroll,
    /// The selection moved, or the list under it changed, since the last frame.
    /// See [`Cursor::reveal`].
    follow: bool,
}

impl Cursor {
    /// A cursor for a list no frame has measured yet.
    ///
    /// `viewport` is what the pane's height is assumed to be until one does,
    /// and it is a caller's guess rather than zero for a reason: keys can be
    /// drained in the same batch as the one that opened the list, before
    /// anything has been drawn, and a page measured against a viewport of zero
    /// is a page of one row.
    pub fn new(viewport: usize) -> Self {
        let mut scroll = Scroll::default();
        scroll.measure(0, viewport);
        Self {
            sel: 0,
            scroll,
            follow: false,
        }
    }

    /// A cursor for a list opening over `under`, in the same pane — a find
    /// raised over the listing it was asked for from.
    ///
    /// It starts at the top, because a list nobody has looked at yet has no row
    /// anyone chose, and it deliberately leaves `under`'s selection alone: that
    /// is the row the reader comes back to.
    ///
    /// It takes the *view*, for the reason the module doc gives — a `Scroll`
    /// starting from nothing does not know how tall the pane is, and the first
    /// key pressed in the new list is answered before any frame could tell it.
    /// The row count inside that `Scroll` comes along too, and that part is
    /// survivable rather than intended: it describes the list left behind,
    /// nothing reads it but the clamp in `Scroll::to`, and the reveal-then-frame
    /// pair on [`Cursor::reveal`] is already there to correct exactly that.
    pub fn over(under: &Cursor) -> Self {
        Self {
            sel: 0,
            scroll: under.scroll,
            follow: false,
        }
    }

    /// Take the view back from a list that had opened over this one and has now
    /// closed.
    ///
    /// The other half of [`Cursor::over`], named rather than left as a field
    /// copy at the call site because the two have to stay a pair: a hand-off
    /// with only one end is a pane with two views in it.
    pub fn take_view(&mut self, from: &Cursor) {
        self.scroll = from.scroll;
    }

    /// What a frame knows and a key cannot: how many rows there are, and how
    /// tall the pane turned out to be.
    ///
    /// The row count is only passed on, to `Scroll`'s own clamp. A list that
    /// merely grew needs nothing done to it, which is what makes this safe to
    /// call on a frame drawn while rows are still arriving.
    ///
    /// The height is the half that can move something. A pane whose height
    /// changed can have left the selection below the fold, and `Scroll::measure`
    /// only clamps the offset — it never looks at what is selected. Treated as
    /// a move for exactly that reason.
    ///
    /// Deliberately *not* a scroll-into-view on every frame. The wheel is
    /// allowed to move the view away from the selection, and a frame that
    /// dragged it back would make scrolling by wheel impossible.
    pub fn measure(&mut self, rows: usize, height: usize) {
        let resized = height != self.scroll.viewport();
        self.scroll.measure(rows, height);
        if std::mem::take(&mut self.follow) || resized {
            self.scroll_into_view();
        }
    }

    /// The list flavour of the vocabulary the F1 overlay promises.
    ///
    /// `None` for a key this table has no opinion about, so the caller can go
    /// on to match its own — `Enter`, `/`, `r` — and so `Esc` and `q` fall
    /// through to the shell as "give focus back to the agent".
    ///
    /// **One of the four callers takes `Esc` back before it gets there**, and
    /// it is worth naming because the sentence above is otherwise read as a
    /// promise this type makes rather than as what it does. `outline::View` is
    /// a layer over the document rather than a view of its own: `Esc` there has
    /// somewhere to go that is not the agent, so it means "back to the page"
    /// and never reaches the shell. It is the same shape `GitPane`'s worktree
    /// list has, and it makes the same two choices — `Esc` claimed, `q` left
    /// alone — so `q` in the outline still falls through exactly as this says.
    /// The other three callers take neither.
    ///
    /// `crate::scroll::key` carries this sentence with no carve-out and needs
    /// none: the outline routes only the *glance* keys through `Scroll::key`,
    /// and `Esc` is not one of them.
    ///
    /// `Ctrl`+letter is `None` as well, apart from the two claimed here. It is
    /// the agent's everywhere else in the program and must not be swallowed by
    /// the plain-letter arms, so it is handed back for the caller to decline
    /// deliberately — without which `Ctrl+R` would arrive at `browse.rs`'s `r`.
    ///
    /// `G` and `End` land on the last *row*, where `Scroll::key` stops at the
    /// first row of the last screenful. That difference is why this table sits
    /// beside that one instead of delegating to it: the row `End` lands on is
    /// the row `Enter` then opens.
    pub fn key(&mut self, rows: usize, key: KeyEvent) -> Option<Handled> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        Some(match key.code {
            KeyCode::Char('d') if ctrl => self.step(rows, self.half() as isize),
            KeyCode::Char('u') if ctrl => self.step(rows, -(self.half() as isize)),
            KeyCode::Char(_) if ctrl => return None,

            KeyCode::Char('j') | KeyCode::Down => self.step(rows, 1),
            KeyCode::Char('k') | KeyCode::Up => self.step(rows, -1),
            KeyCode::Char(' ') | KeyCode::PageDown => self.step(rows, self.page() as isize),
            KeyCode::Char('b') | KeyCode::PageUp => self.step(rows, -(self.page() as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.select(rows, 0),
            KeyCode::Char('G') | KeyCode::End => self.select(rows, usize::MAX),
            KeyCode::Tab => self.wrap(rows, 1),
            KeyCode::BackTab => self.wrap(rows, -1),
            _ => return None,
        })
    }

    /// The wheel moves the view; the left button chooses a row.
    ///
    /// `None` only for a button this has nothing to say about, matching
    /// [`Cursor::key`]. A click below the last row is not one of those: it is a
    /// click the list understood and declined, because below the last row there
    /// is nothing to choose. Snapping the selection to the end of the list
    /// instead would mean a click on the empty half of a short pane silently
    /// re-aiming the `Enter` the reader presses next.
    pub fn mouse(&mut self, rows: usize, ev: &MouseEvent) -> Option<Handled> {
        if let Some(handled) = self.scroll.mouse(ev) {
            return Some(handled);
        }
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let row = self.scroll.offset + ev.row as usize;
                if row >= rows {
                    return Some(Handled::No);
                }
                Some(self.select(rows, row))
            }
            _ => None,
        }
    }

    pub fn step(&mut self, rows: usize, delta: isize) -> Handled {
        if rows == 0 {
            return Handled::No;
        }
        let to = (self.sel as isize + delta).clamp(0, rows as isize - 1);
        self.select(rows, to as usize)
    }

    /// Tab wraps where `j` stops. With one screenful of files, Tab from the
    /// last entry back to the first is what a reader means by it; `j` at the
    /// bottom is someone who has arrived at the bottom.
    pub fn wrap(&mut self, rows: usize, delta: isize) -> Handled {
        let n = rows as isize;
        if n == 0 {
            return Handled::No;
        }
        self.select(rows, (((self.sel as isize + delta) % n + n) % n) as usize)
    }

    /// Choose a row, clamped to the list. `usize::MAX` is how `End` asks for
    /// the last one.
    pub fn select(&mut self, rows: usize, to: usize) -> Handled {
        if rows == 0 {
            return Handled::No;
        }
        let to = to.min(rows - 1);
        if to == self.sel {
            // Nothing moved, so nothing was acted on. Reporting otherwise
            // spends a frame — the agent's whole screen included — on a key
            // that could not do anything.
            return Handled::No;
        }
        self.sel = to;
        self.reveal();
        Handled::Yes
    }

    /// The row this cursor names has moved: the reader moved it, or the list
    /// was rebuilt under it and the same index now names something else.
    ///
    /// Not "the list changed". A list that merely grew, with the selection
    /// still on the row it was on, has nothing to bring back on screen — and a
    /// list that revealed once per batch of arriving rows would pin the view to
    /// the selection and take the wheel away, which is the thing
    /// [`Cursor::measure`] refuses to do once per frame for the same reason.
    ///
    /// Scrolled into view twice, and the second time is the one that has to be
    /// there. Here, on the numbers the last frame left behind, so a burst of
    /// keys drained before the next frame pages from roughly the right place.
    /// And again from [`Cursor::measure`], because `Scroll` is told the row
    /// count by a frame and by nothing else: climbing out of a one-file
    /// directory into a four-hundred-entry one leaves it believing the list is
    /// one row long, so it clamps the offset to zero and the selected row is
    /// off screen with nothing left to bring it back.
    pub fn reveal(&mut self) {
        self.follow = true;
        self.scroll_into_view();
    }

    /// Bring the selected row into view without recentring — a list that jumps
    /// under you is harder to read than one that scrolls by a line.
    fn scroll_into_view(&mut self) {
        let page = self.scroll.viewport().max(1);
        if self.sel < self.scroll.offset {
            self.scroll.to(self.sel);
        } else if self.sel >= self.scroll.offset + page {
            self.scroll.to(self.sel + 1 - page);
        }
    }

    pub fn page(&self) -> usize {
        // One row of overlap, the same as everywhere else that pages.
        self.scroll.viewport().saturating_sub(1).max(1)
    }

    pub fn half(&self) -> usize {
        (self.scroll.viewport() / 2).max(1)
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

    fn mouse(kind: MouseEventKind, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// A hundred rows in a ten-row pane, as a frame would have left it.
    fn cursor() -> Cursor {
        let mut c = Cursor::new(20);
        c.measure(100, 10);
        c
    }

    #[test]
    fn every_key_the_help_overlay_advertises_moves_the_selection() {
        // The reason this type owns the table and not only the state: the F1
        // overlay is one list for the whole program, and every pane that holds
        // a selection has to make every line of it true.
        let mut c = cursor();
        assert_eq!(c.key(100, key(KeyCode::Char('j'))), Some(Handled::Yes));
        assert_eq!(c.sel, 1);
        assert_eq!(c.key(100, key(KeyCode::Down)), Some(Handled::Yes));
        assert_eq!(c.sel, 2);
        c.key(100, key(KeyCode::Char(' ')));
        assert_eq!(c.sel, 2 + 9, "a page keeps one row of overlap");
        c.key(100, key(KeyCode::Char('b')));
        assert_eq!(c.sel, 2);
        c.key(100, key(KeyCode::PageDown));
        assert_eq!(c.sel, 11);
        c.key(100, key(KeyCode::PageUp));
        assert_eq!(c.sel, 2);
        c.key(100, ctrl('d'));
        assert_eq!(c.sel, 7);
        c.key(100, ctrl('u'));
        assert_eq!(c.sel, 2);
        c.key(100, key(KeyCode::Char('k')));
        assert_eq!(c.sel, 1);

        c.key(100, key(KeyCode::Char('G')));
        assert_eq!(c.sel, 99, "G reaches the last row, not the last page");
        assert_eq!(c.scroll.offset, 90, "and it is on screen when it lands");
        c.key(100, key(KeyCode::Char('g')));
        assert_eq!(c.sel, 0);
        c.key(100, key(KeyCode::End));
        assert_eq!(c.sel, 99);
        c.key(100, key(KeyCode::Home));
        assert_eq!(c.sel, 0);
    }

    #[test]
    fn keys_that_are_not_ours_are_left_for_the_list_that_owns_them() {
        // `Enter`, `/`, `r`, `-` and Backspace all mean something to the file
        // list; `Esc` and `q` mean something to the shell. A table that
        // swallowed any of them would be a key gone quietly dead one module
        // further out.
        let mut c = cursor();
        for code in [
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Enter,
            KeyCode::Char('r'),
            KeyCode::Char('/'),
            KeyCode::Char('-'),
            KeyCode::Backspace,
        ] {
            assert_eq!(c.key(100, key(code)), None, "{code:?} is not a list key");
        }
        // Ctrl+letter is the agent's; only Ctrl+D and Ctrl+U are claimed here,
        // and the rest must not fall through to the plain-letter arms — a
        // `Ctrl+R` reaching `r` would re-read a directory nobody asked about.
        assert_eq!(c.key(100, ctrl('c')), None);
        assert_eq!(c.key(100, ctrl('r')), None);
        assert_eq!(c.sel, 0);
    }

    #[test]
    fn the_wheel_moves_the_view_and_a_click_chooses_a_row() {
        let mut c = cursor();
        assert_eq!(
            c.mouse(100, &mouse(MouseEventKind::ScrollDown, 0)),
            Some(Handled::Yes)
        );
        assert_eq!(c.scroll.offset, 3);
        assert_eq!(c.sel, 0, "a wheel notch is a read, not a choice");

        assert_eq!(
            c.mouse(100, &mouse(MouseEventKind::Down(MouseButton::Left), 2)),
            Some(Handled::Yes)
        );
        assert_eq!(c.sel, 5, "the row clicked, counted from the offset");

        // Below the last row of a short list there is nothing to choose.
        // Snapping to the end instead would silently re-aim the next `Enter`.
        let mut short = Cursor::new(10);
        short.measure(2, 10);
        assert_eq!(
            short.mouse(2, &mouse(MouseEventKind::Down(MouseButton::Left), 7)),
            Some(Handled::No)
        );
        assert_eq!(short.sel, 0);
        assert_eq!(short.mouse(2, &mouse(MouseEventKind::Moved, 0)), None);
    }

    #[test]
    fn a_move_that_changes_nothing_says_it_did_nothing() {
        // A frame here re-renders the agent's whole screen, so `k` at the top
        // of a list must not cost one.
        let mut c = cursor();
        assert_eq!(c.step(100, -1), Handled::No);
        assert_eq!(c.select(100, 99), Handled::Yes);
        assert_eq!(c.step(100, 1), Handled::No);

        // ...and a list with nothing in it takes no move at all, rather than
        // reaching for a last row that is not there.
        let mut empty = Cursor::new(20);
        assert_eq!(empty.step(0, 1), Handled::No);
        assert_eq!(empty.wrap(0, 1), Handled::No);
        assert_eq!(empty.select(0, 0), Handled::No);
        assert_eq!(empty.key(0, key(KeyCode::End)), Some(Handled::No));
        assert_eq!(empty.sel, 0);
    }

    #[test]
    fn tab_wraps_where_j_stops() {
        let mut c = cursor();
        assert_eq!(c.key(100, key(KeyCode::BackTab)), Some(Handled::Yes));
        assert_eq!(c.sel, 99, "off the top is the bottom");
        assert_eq!(c.key(100, key(KeyCode::Tab)), Some(Handled::Yes));
        assert_eq!(c.sel, 0);

        // ...where `j` at the bottom is someone who has arrived at the bottom.
        c.select(100, 99);
        assert_eq!(c.key(100, key(KeyCode::Char('j'))), Some(Handled::No));
        assert_eq!(c.sel, 99);
    }

    #[test]
    fn the_selection_is_brought_into_view_a_row_at_a_time() {
        let mut c = cursor();
        c.select(100, 10);
        assert_eq!(c.scroll.offset, 1, "scrolled by a row, not recentred");
        c.select(100, 0);
        assert_eq!(c.scroll.offset, 0);
    }

    #[test]
    fn a_pane_that_shrank_still_shows_what_is_selected() {
        // `Scroll::measure` clamps the offset and never looks at the selection,
        // so a drag that halves the window can leave the chosen row below the
        // fold with nothing to bring it back until the next key.
        let mut c = cursor();
        c.select(100, 40);
        c.measure(100, 10);
        assert!(c.sel >= c.scroll.offset && c.sel < c.scroll.offset + 10);

        c.measure(100, 4);
        assert!(
            c.sel >= c.scroll.offset && c.sel < c.scroll.offset + 4,
            "row {} is outside the window of 4 at {}",
            c.sel,
            c.scroll.offset
        );
    }

    #[test]
    fn a_frame_leaves_a_view_the_wheel_moved_where_it_found_it() {
        // The other half of the rule above. Dragging the view back to the
        // selection every frame would make scrolling without moving the
        // selection — the wheel, and the glance keys — impossible.
        let mut c = cursor();
        c.scroll.by(20);
        c.measure(100, 10);
        assert_eq!(c.scroll.offset, 20);
        assert_eq!(c.sel, 0);
    }

    #[test]
    fn a_list_that_only_grew_is_left_where_the_reader_had_it() {
        // What makes `measure` safe to call on a frame drawn while rows are
        // still arriving: more rows is not news to a cursor, and a list that
        // reacted to every batch would pin the view to the selection.
        let mut c = cursor();
        c.scroll.by(20);
        c.measure(140, 10);
        assert_eq!(c.scroll.offset, 20);
        assert_eq!(c.sel, 0);
    }

    #[test]
    fn a_list_opening_over_another_takes_the_view_and_leaves_the_choice() {
        // Both halves matter, and each is something a reader would notice: the
        // row underneath is what `Esc` comes back to, and the view is what
        // makes the first page-down in the new list a page rather than a row.
        let mut under = cursor();
        under.select(100, 40);

        let mut over = Cursor::over(&under);
        assert_eq!(over.sel, 0);
        assert_eq!(over.scroll.offset, under.scroll.offset);
        assert_eq!(over.page(), under.page());
        assert_eq!(under.sel, 40, "the list underneath keeps its row");

        // ...and closing it hands the view back, so the pane never holds two.
        over.select(60, 50);
        under.take_view(&over);
        assert_eq!(under.scroll.offset, over.scroll.offset);
        assert_eq!(under.sel, 40, "still its own row, and not the other's");
    }
}
