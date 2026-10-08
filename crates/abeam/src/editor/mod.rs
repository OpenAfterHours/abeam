//! Text with a caret in it: what the scratch pad types into, and what the
//! files view will type into when it edits a file.
//!
//! This was the pad's, and it moved here the moment a second pane needed it,
//! because every hard part of it is about editing and none of it is about the
//! pad. A caret that is a char index measured in cells, a sticky column, a
//! click that lands on the row it was made on, a layout that the drawing and
//! the cursor are both read out of, the rule that a refused key is still
//! claimed — each of those cost the pad a round to get right, and a files view
//! with its own copy of them would be a second place for each to be got wrong.
//! [`Editor`] is the whole of it behind one type, and [`View`] is where it is
//! scrolled to:
//!
//! - `buffer` holds the text as lines and the caret in it, and is the only
//!   thing that ever writes the text;
//! - `history` is undo and redo, built on the record of every write the buffer
//!   makes, and knows which point in it was last saved;
//! - `wrap` is the one table — where each line breaks, how wide each piece is —
//!   that drawing, the cursor and a click are all read from;
//! - `layout` keeps that table between frames, mends it per line when the text
//!   changes, and colours only as far down as anybody has looked.
//!
//! ## What is shared, and what is still the caller's
//!
//! Shared is everything a keystroke does to the text and everything a frame
//! needs to draw it. What is **policy** arrives from the caller as a
//! [`Policy`]: how much the text may hold, counted in the bytes it will have on
//! disk, and what a tab becomes. The pad's is a 64 KiB cap and two spaces for
//! every tab, because that cap is the highlighter's and two spaces is a
//! markdown nesting level; a file's will be the reader's 512 KiB and its own
//! literal tabs, because a save has to give back the bytes it was given. Which
//! grammar colours the text and which palette it is drawn in arrive at each
//! frame as a [`Look`].
//!
//! Still the caller's: what a key outside the editing vocabulary means — `Esc`,
//! `Ctrl+S`, an `Alt` chord, `PgUp`/`PgDn`, which [`Editor::key`] hands back
//! untouched — and where the caret starts. `Editor::from_text` leaves it at
//! the start of the text; the pad moves it to the end, because a note is
//! reopened to be added to, and the files view will move it to where the reader
//! was looking. The pad's rendered form, `Alt+T`, its autosave and its notices
//! are the pad's alone. Line endings and a byte order mark are nobody's
//! business here beyond what they cost: the buffer holds lines joined by `\n`;
//! a caller that must write a file back with the `\r\n` it was read with, or
//! the mark it began with, records those on the way in, restores them on the
//! way out, and tells the policy their size so the cap is counted honestly.
//!
//! ## One layout, read forwards and backwards
//!
//! `crate::layout`'s module doc states the principle for the pane split and
//! says what it prevents: "two calculations that must agree is where off-by-one
//! here is what makes hosted apps wrap strangely". Here the two are the row a
//! character is *drawn* on and the row [`Editor::caret_cell`] *reports*, and a
//! pane whose caret sits one row from the text it is in is unusable in a way
//! that looks like a rendering bug rather than a caret bug.
//!
//! So there is one table. `wrap::breaks` returns the char index each visual
//! row of a line begins at, and everything else is that table read in one
//! direction or the other: `wrap::into_rows` cuts the styled spans at those
//! indices to draw them, [`Editor::caret_cell`] looks a char index up in them
//! to place the cursor, and [`Editor::click`] reads them backwards to turn a
//! pointer into a caret.
//!
//! Reading it backwards carries an invariant of its own, and a pointer is what
//! makes it visible: **a click on a row yields a caret the same table draws
//! back on that row.** Without it a click inside a wrapped row's rectangle can
//! answer with the index the *next* row begins at, which the forward reading
//! then draws — correctly, by its own rule — at the start of that next row, so
//! the caret appears one line below the cell the pointer was over. `abc日def`
//! at four columns is enough to show it. `wrap::char_at` spends one subtraction
//! on this.
//!
//! There is one seam in all of it, and it is [`Editor::click`]: the table it
//! reads was built by the last frame, and `App::run` drains every queued input
//! event before drawing another. A keystroke and a click arriving together
//! therefore put the pointer's question to a layout the keystroke has already
//! invalidated. So a click whose table is out of date is declined rather than
//! answered wrongly, and the frame the keystroke already asked for makes the
//! next one right.
//!
//! Hard-wrapped, with no horizontal scrolling. A pane is forty-odd columns; a
//! horizontal offset would mean a second scroll vocabulary, a second thing for
//! `G` to mean, and text the user wrote sitting off the side of the pane with
//! nothing saying so.
//!
//! **A tab is drawn by this layout's rule and not the reader's**, and the two
//! can disagree. Here a tab fills to the next stop of `source::TAB` counted in
//! cells from the start of its row, and the highlighter is handed the line with
//! its tabs in it; the reader's source view expands tabs to spaces counting
//! characters, and highlights the expanded line. On a line with a wide
//! character before a tab the stops land differently, and a grammar that reads
//! a leading tab differently from four spaces can colour the line differently.
//! That is accepted rather than unified for now: drawing and caret have to
//! agree with each other first, and cells is the only unit they can agree in.
//!
//! ## What it costs
//!
//! An edit re-wraps only the lines it changed, and colours again only what is
//! on screen, from the line it changed down to the first line after which the
//! highlighter is where it was. `layout` has the argument, and `editor_bench`
//! in the tests below has the numbers; on the 46 KB README at sixty columns, in
//! a release build on the machine this was written on:
//!
//! - a character typed into the middle of a paragraph cost 77–125 ms across
//!   runs when the pad laid the whole document out again, and costs about
//!   0.5 ms now, most of it syntect colouring the one line;
//! - typing in the info string of a fence over a 1500-line block, which really
//!   does change the colour of every line in the block, costs the screen and
//!   the lookahead — 12–14 ms — rather than the block;
//! - opening 64 KiB of markdown costs 4–6 ms to the first frame, against the
//!   reader's 135 ms to highlight the same text, because only the first screen
//!   is coloured; colouring all of it, by scrolling to the end, costs what the
//!   reader's highlight does, give or take a tenth. 512 KiB, wrapped whole and
//!   drawn plain, is 16–23 ms to the first frame.
//!
//! Laziness saves nothing when the first frame is near the end. A line cannot
//! be coloured without the state the line above left, so opening a 64 KiB
//! file at a fraction near its end — which is what the files view's `e` does
//! from a rendered page scrolled down — colours from the top on that first
//! frame, about 130 ms: the reader's own cost for the same text, paid once.
//!
//! `an_edit_to_a_long_document_costs_a_line_and_not_the_document` is the guard
//! that fails if an edit goes back to costing the document.

mod buffer;
mod history;
mod layout;
mod wrap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::pane::Handled;
use crate::scroll::Scroll;
use buffer::Buffer;
use history::{History, Kind};
use layout::Layout;
pub use layout::{Look, Row};

/// What a caller decides about the text it hands an [`Editor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// The most the text may hold, in bytes **as it will be on disk**: with
    /// [`line_ending_bytes`](Policy::line_ending_bytes) between lines and
    /// [`bom_bytes`](Policy::bom_bytes) in front.
    ///
    /// An insert or a paste that would take it past this is refused rather
    /// than trimmed to fit, and the refusal has to be told: the caller draws a
    /// notice off [`Editor::is_full`] and off [`Outcome::Refused`]. A paste
    /// that does not fit is refused whole, because half a pasted paragraph is
    /// worse than none of it: the user has to notice the cut, and the place it
    /// happened is off the bottom of a pane they had already stopped looking
    /// at.
    pub max_bytes: usize,
    /// What a tab becomes, and what the `Tab` key types.
    pub tabs: Tabs,
    /// What one line ending will cost when the text is written: 1 for `\n`, 2
    /// for `\r\n`. The buffer only ever holds `\n`; this is how the cap knows
    /// what the save will write.
    pub line_ending_bytes: usize,
    /// What the file will begin with before the text: 3 for a UTF-8 byte
    /// order mark, 0 for none.
    pub bom_bytes: usize,
}

/// What a tab is, under a [`Policy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tabs {
    /// Every tab that arrives — typed, pasted or loaded — becomes this many
    /// spaces, and the `Tab` key types them. The pad's, with two.
    Spaces(usize),
    /// Tabs are kept exactly as they arrive, and the `Tab` key types `key`.
    /// A file's: it is given back with the bytes it had, and the caller
    /// decides what new indentation is from the file — a literal tab where it
    /// is already indented with them, spaces where it is not.
    Keep { key: TabKey },
}

/// What the `Tab` key types where tabs are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKey {
    Tab,
    Spaces(usize),
}

/// What a key or a paste did to an [`Editor`], for the caller to answer.
///
/// Four answers rather than "did it change", because the caller owes each a
/// different thing: an edit marks the text unsaved and brings the caret into
/// view, a refusal puts a notice up, a move brings the caret into view and
/// changes nothing else, and a key that did nothing must not cost a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The text changed.
    Edited,
    /// Something typed or pasted was turned away for want of room.
    ///
    /// **Claimed rather than declined, and both halves of that matter**, which
    /// is why [`Outcome::handled`] says yes to it. `crate::app` reads a bare
    /// `q` a right pane did not want as "the user is done with this pane" and
    /// moves focus to the agent — so an editor that declined the letters it
    /// could not fit would eject the writer into the agent's prompt at the
    /// exact moment it was trying to say it was full, with every letter after
    /// that going into a conversation. In a pane being typed into `q` is text,
    /// and it has to go on being text at the cap, because that is the one
    /// moment the claim is load-bearing.
    ///
    /// The frame is the same answer to a different question. A refusal changes
    /// what the caller draws — the notice saying why — and `App::handle_event`
    /// paints only for an event something came of, so a declined refusal
    /// would set the notice and never put it on screen. The cost is a frame per
    /// repeat while a key is held down against a full text, which
    /// `crate::pane::Handled` warns about in general and which is the right
    /// side of that trade here: the alternative is not a wasted frame but a
    /// lost pane.
    Refused,
    /// The caret moved, and the text did not.
    Moved,
    /// The key is the editor's and did nothing: `Backspace` at the start, an
    /// undo with nothing to take back, `Up` on the top row.
    ///
    /// Declined, because a `Yes` for a press that moved nothing spends a frame
    /// re-rendering the agent's entire screen to redraw what is already there,
    /// at the key-repeat rate for somebody holding `Up` at the top. Safe to
    /// decline because nothing that can come back as this is `Esc` or `q`: a
    /// printable key either goes in or is [`Outcome::Refused`].
    Still,
}

impl Outcome {
    /// What the caller tells the shell. See each variant for why.
    pub fn handled(self) -> Handled {
        match self {
            Outcome::Still => Handled::No,
            Outcome::Edited | Outcome::Refused | Outcome::Moved => Handled::Yes,
        }
    }

    fn typed(did: bool) -> Self {
        if did { Outcome::Edited } else { Outcome::Refused }
    }

    fn edited(did: bool) -> Self {
        if did { Outcome::Edited } else { Outcome::Still }
    }

    fn moved(did: bool) -> Self {
        if did { Outcome::Moved } else { Outcome::Still }
    }
}

/// A text, a caret, the past, and the rows it was last drawn as.
pub struct Editor {
    buffer: Buffer,
    history: History,
    layout: Layout,
    /// Bumped on every change to the text, so the layout can tell whether it
    /// was built from this text or the one before it.
    rev: u64,
    /// Whether this text is coloured at all: decided from its size when it
    /// arrived, and turned off — once, for good — by an edit that takes it past
    /// the cap. See [`Editor::past_highlight_cap`].
    colour: bool,
}

impl Editor {
    /// An empty text held to `policy`.
    pub fn new(policy: Policy) -> Self {
        Self::of(Buffer::new(policy))
    }

    /// `text`, held to `policy`, with the caret at its start. See
    /// [`Buffer::from_text`] for what is cleaned and what is cut.
    pub fn from_text(policy: Policy, text: &str) -> Self {
        Self::of(Buffer::from_text(policy, text))
    }

    fn of(buffer: Buffer) -> Self {
        Self {
            history: History::new(buffer.policy().max_bytes),
            colour: !layout::past_cap(buffer.text_bytes()),
            buffer,
            layout: Layout::default(),
            rev: 0,
        }
    }

    // --- reading ------------------------------------------------------------

    /// The whole text as one string, which is what gets saved.
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// See [`Buffer::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// See [`Buffer::is_full`].
    pub fn is_full(&self) -> bool {
        self.buffer.is_full()
    }

    /// See [`Buffer::truncated`].
    pub fn truncated(&self) -> bool {
        self.buffer.truncated()
    }

    /// Whether the text is anything other than what was last saved, or — with
    /// no save yet — than what it arrived as. One comparison; see
    /// `history`'s *The saved point* for what it counts and the one way it is
    /// strict.
    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
    }

    // --- saving ---------------------------------------------------------------

    /// The text as it is now is on disk. The caller says so after a save that
    /// worked, and [`Editor::is_modified`] answers from here on. Closes the
    /// open undo step, because a step that grew after a save would make the
    /// saved point a lie — see `history`.
    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    /// Close the open undo step, so the next edit starts another. What a save
    /// that is about to be attempted asks for, whether or not it works: the
    /// user pressed a key that means "this is a point I might come back to".
    pub fn end_step(&mut self) {
        self.history.seal();
    }

    // --- keys -----------------------------------------------------------------

    /// One key, if it is one of the editor's: what it did, or `None` for a key
    /// the caller decides about.
    ///
    /// The editor's are the ones that mean the same thing wherever there is a
    /// caret: every printable character (AltGr ones included — see
    /// `crate::keys::is_text`), `Tab`, `Enter`, `Backspace`, `Delete`, the
    /// arrows, `Home`, `End`, and `Ctrl+Z` / `Ctrl+Y` for undo and redo, with
    /// `Ctrl+Shift+Z` redoing too where the terminal reports the Shift. A
    /// capital `Z` that arrives *without* Shift — Caps Lock — is still undo,
    /// because it is still the key that was pressed.
    ///
    /// Everything else comes back `None`: `Esc`, `Ctrl+S`, every other Ctrl
    /// chord, every `Alt` chord, `PgUp`/`PgDn`, the F-keys. Those are questions
    /// about the pane — leave it, save it, turn it over, page it — and the
    /// answer to each is different in the pad and in the files view.
    ///
    /// So is `Tab`, `Enter`, `Backspace`, `Delete`, an arrow, `Home` or `End`
    /// with `Ctrl` or `Alt` held. Every editor the user has met gives those a
    /// meaning of their own — a word left, a word rubbed out — and this one has
    /// no word motion, for the reason `buffer` gives; a `Ctrl+Backspace` that
    /// took one character would be a promise half kept, and declined it does
    /// nothing at all, which the user can see. `Shift` is let through, because
    /// it is what a hand still on Shift after a capital adds to the next
    /// `Enter`.
    ///
    /// **No key that comes back `Some` is ever `Esc` or a bare `q` declined**,
    /// which is what makes the [`Outcome`] answers safe to hand to
    /// `crate::app`: a printable key is always claimed, even when it was
    /// refused.
    pub fn key(&mut self, key: &KeyEvent) -> Option<Outcome> {
        let ctrl = crate::keys::ctrl_chord(key);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let bare = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        Some(match key.code {
            KeyCode::Char('z' | 'Z') if ctrl && shift => Outcome::edited(self.redo()),
            KeyCode::Char('z' | 'Z') if ctrl => Outcome::edited(self.undo()),
            KeyCode::Char('y' | 'Y') if ctrl => Outcome::edited(self.redo()),
            KeyCode::Char(c) if crate::keys::is_text(key) => Outcome::typed(self.insert(c)),
            KeyCode::Tab if bare => Outcome::typed(self.tab()),
            KeyCode::Enter if bare => Outcome::typed(self.newline()),
            KeyCode::Backspace if bare => Outcome::edited(self.backspace()),
            KeyCode::Delete if bare => Outcome::edited(self.delete()),
            KeyCode::Left if bare => Outcome::moved(self.left()),
            KeyCode::Right if bare => Outcome::moved(self.right()),
            KeyCode::Up if bare => Outcome::moved(self.up()),
            KeyCode::Down if bare => Outcome::moved(self.down()),
            KeyCode::Home if bare => Outcome::moved(self.home()),
            KeyCode::End if bare => Outcome::moved(self.end()),
            _ => return None,
        })
    }

    /// Replace the whole text with `text` as one step of undo, the caret staying
    /// where it was, clamped: a recovered draft put over the file it was
    /// written against, or a file reloaded from disk with its history kept.
    ///
    /// The two callers want one property and it is the reason this is not
    /// `from_text`: the step can be undone. A draft restored over the disk text
    /// reads as modified, and `Ctrl+Z` gives back exactly what is on disk; a
    /// reload followed by [`Editor::mark_saved`] reads as saved, and `Ctrl+Z`
    /// gives back what the user had before it. Cleaned, and refused whole when
    /// it would not fit, like a paste; [`Outcome::Still`] when it is the text
    /// already here, which records nothing.
    pub fn replace_all(&mut self, text: &str) -> Outcome {
        let mut fits = true;
        let did = self.edit(Kind::Paste, |b| match b.replace_all(text) {
            Some(changed) => changed,
            None => {
                fits = false;
                false
            }
        });
        match (did, fits) {
            (true, _) => Outcome::Edited,
            (false, false) => Outcome::Refused,
            (false, true) => Outcome::Still,
        }
    }

    /// A paste, which is one step of undo however long it is.
    ///
    /// An empty paste is [`Outcome::Still`] and not a refusal, though the
    /// buffer answers both with the same `false`: without the distinction an
    /// empty clipboard would put a notice on screen saying the text was too
    /// big to hold it.
    pub fn paste(&mut self, text: &str) -> Outcome {
        if text.is_empty() {
            return Outcome::Still;
        }
        Outcome::typed(self.insert_str(text))
    }

    // --- editing --------------------------------------------------------------
    //
    // Each returns whether the text changed. An edit that happened is recorded
    // for undo; one that was refused is not, and leaves the open step open,
    // because nothing happened.

    /// Type one character.
    pub fn insert(&mut self, c: char) -> bool {
        self.edit(Kind::Typing, |b| b.insert(c))
    }

    /// The `Tab` key: what [`Tabs`] says it types.
    pub fn tab(&mut self) -> bool {
        let text = self.buffer.tab_text();
        self.edit(Kind::Typing, |b| b.insert_str(&text))
    }

    /// `Enter`.
    pub fn newline(&mut self) -> bool {
        self.edit(Kind::Typing, Buffer::newline)
    }

    /// Insert `s` as one step of undo. See [`Editor::paste`], which is what a
    /// pane should call for a paste.
    pub fn insert_str(&mut self, s: &str) -> bool {
        self.edit(Kind::Paste, |b| b.insert_str(s))
    }

    pub fn backspace(&mut self) -> bool {
        self.edit(Kind::Backspace, Buffer::backspace)
    }

    pub fn delete(&mut self) -> bool {
        self.edit(Kind::Delete, Buffer::delete)
    }

    /// Take the newest step back, caret and all. See `history`.
    pub fn undo(&mut self) -> bool {
        let did = self.history.undo(&mut self.buffer);
        self.changed(did)
    }

    /// Put back the step last taken.
    pub fn redo(&mut self) -> bool {
        let did = self.history.redo(&mut self.buffer);
        self.changed(did)
    }

    fn edit(&mut self, kind: Kind, f: impl FnOnce(&mut Buffer) -> bool) -> bool {
        let before = self.buffer.caret();
        let did = f(&mut self.buffer);
        // The buffer leaves a change exactly when it wrote, so the two answers
        // agree; recording the change rather than trusting `did` is what keeps
        // a write from ever reaching the text without reaching the history.
        let change = self.buffer.take_change();
        debug_assert_eq!(did, change.is_some(), "an edit and its record disagree");
        if let Some(change) = change {
            self.history.record(kind, change, before, self.buffer.caret());
        }
        self.changed(did)
    }

    fn changed(&mut self, did: bool) -> bool {
        if did {
            self.rev = self.rev.wrapping_add(1);
            // One way only: see `past_highlight_cap`.
            if self.colour && layout::past_cap(self.buffer.text_bytes()) {
                self.colour = false;
            }
        }
        did
    }

    // --- moving ---------------------------------------------------------------
    //
    // Each returns whether the caret moved, and each ends the open undo step
    // whether it moved or not: a key that is not an edit is the end of a run of
    // typing. See `history`'s *What one step is*.

    pub fn left(&mut self) -> bool {
        self.moved(Buffer::left)
    }

    pub fn right(&mut self) -> bool {
        self.moved(Buffer::right)
    }

    pub fn up(&mut self) -> bool {
        self.moved(Buffer::up)
    }

    pub fn down(&mut self) -> bool {
        self.moved(Buffer::down)
    }

    pub fn home(&mut self) -> bool {
        self.moved(Buffer::home)
    }

    pub fn end(&mut self) -> bool {
        self.moved(Buffer::end)
    }

    /// Put the caret at `row`, `col`, clamped into the text — so `usize::MAX`
    /// for both is the end of it, and `(line, 0)` is the start of a line. See
    /// [`Buffer::set_caret`], which carries the clamping argument.
    pub fn set_caret(&mut self, row: usize, col: usize) -> bool {
        self.moved(|b| b.set_caret(row, col))
    }

    fn moved(&mut self, f: impl FnOnce(&mut Buffer) -> bool) -> bool {
        self.history.seal();
        f(&mut self.buffer)
    }

    // --- drawing --------------------------------------------------------------

    /// Bring the layout up to date for a frame `width` cells wide drawn in
    /// `look`: wrap what changed, and forget the colours that can no longer be
    /// trusted. Colouring happens in [`Editor::rows`], for the rows drawn.
    pub fn lay_out(&mut self, width: usize, look: Look) {
        self.layout.update(
            self.buffer.lines(),
            self.rev,
            width,
            look,
            self.colour,
        );
    }

    /// How many rows the text was laid out as.
    pub fn row_count(&self) -> usize {
        self.layout.row_count()
    }

    /// The rows from `from`, at most `n` of them, coloured, for drawing — and
    /// for a gutter, which [`Row::line`] and [`Row::first`] are for.
    ///
    /// `&mut` because this is where colouring happens: only as far down as the
    /// rows asked for, and a little further. See `layout`.
    pub fn rows(&mut self, from: usize, n: usize) -> Vec<Row> {
        self.layout.rows(from, n)
    }

    /// Whether the layout in hand describes the text as it is now.
    fn current(&self) -> bool {
        self.layout.rev() == Some(self.rev)
    }

    /// Where the caret is in the laid-out document, as `(row, column in
    /// cells)`, or `None` when the layout in hand is not of this text.
    ///
    /// The wrap table read forwards. `partition_point` finds the last row whose
    /// start is at or before the caret, which resolves the one genuine
    /// ambiguity in a wrapped line the way the drawing does: a caret sitting on
    /// the index where a row begins is at the *start of that row*, in front of
    /// the character that was pushed down onto it, rather than hanging off the
    /// end of the row above. `wrap::char_at` is written to agree with that,
    /// which is what keeps a click and the cursor it produces on one row.
    ///
    /// ## A column past the last cell of a row
    ///
    /// This happens, in two shapes, and they are one fact seen twice: the caret
    /// is at a char index the row's cells do not reach.
    ///
    /// The expected shape is the end of a logical line that filled its last row
    /// exactly. There is no next row for the caret to be at the start of, so
    /// the column is the row's full width — the terminal's own deferred-wrap
    /// position, and where the next character really will go.
    ///
    /// The other is mid-line. A zero-width character — a combining accent, a
    /// variation selector — that falls after the last cell of a *full* row is
    /// on that row by index and adds nothing to its width, so
    /// `"aa\u{301}bb\u{301}cc"` at two columns reports column two on row zero.
    /// It is the same overflow and wants no separate handling.
    ///
    /// What a reader sees depends on whether a scrollbar column was kept back.
    /// From twenty-four columns up `scroll::bar_width` keeps one, and the caret
    /// sits in it for a frame, which is honest. Below that there is no spare
    /// column — a sixty-column terminal gives a right pane twenty-two — and
    /// `crate::app` clamps the cursor to the last one, so it is drawn *on* the
    /// final character rather than after it and reads as being one place back.
    ///
    /// That is left alone, and the alternative is the reason. Making the
    /// position drawable means giving a line that fills its last row exactly an
    /// extra empty row to hold the caret: either for every such line, which
    /// litters a page of full-width prose with blank rows, or only for the line
    /// the caret is on, which makes the document's row count change as the
    /// caret moves and the view jump under somebody who pressed `Down`. A caret
    /// one cell left of true, in the narrowest pane abeam will split at all, is
    /// the cheapest of the three.
    pub fn caret_cell(&self) -> Option<(usize, usize)> {
        if !self.current() {
            return None;
        }
        self.layout.caret_cell(self.buffer.caret())
    }

    /// Put the caret where a click at document row `row`, cell `col` landed,
    /// and say whether it moved — or `None` when there is no honest answer.
    ///
    /// `row` is counted in the laid-out document, so the caller has already
    /// added its scroll offset and taken off whatever it drew above the text;
    /// [`View::click`] does the first. A click below the last row means the
    /// last row, which is [`Buffer::set_caret`]'s own rule and the same one
    /// that makes a click past the end of a short line mean the end of it.
    ///
    /// Two clicks are declined.
    ///
    /// **One against a layout an edit has already invalidated.** This is the
    /// one place the wrap table is read from outside the frame that built it,
    /// and so the one place it can be stale: paste three lines, press `Enter`
    /// at the start of the first, click the third row before a frame is drawn,
    /// and a stale table puts the caret on the second — a line the pointer was
    /// never over, with no panic to mark it, because `set_caret` clamps
    /// whatever it is handed. The frame the keystroke already asked for makes
    /// the next click right.
    ///
    /// **One on a column the layout was not given.** The caller keeps a column
    /// back for its scrollbar whether or not a bar is drawn in it, and without
    /// this a click there lands at the end of the row: a plausible answer to a
    /// question nobody asked, and — the day the bar can be dragged — a press
    /// that has moved the caret before the bar ever sees it.
    pub fn click(&mut self, row: usize, col: usize) -> Option<bool> {
        if !self.current() || col >= self.layout.width()? {
            return None;
        }
        let (row, col) = self.layout.hit(row, col)?;
        Some(self.set_caret(row, col))
    }
}

/// What the files view reads and the pad does not: the caret as an index, the
/// size decision, and what a gutter and the `e` key need to place things.
impl Editor {
    /// Where the caret is, as `(row, col)` with `col` counted in `char`s.
    pub fn caret(&self) -> (usize, usize) {
        self.buffer.caret()
    }

    /// Whether this text is drawn without colour because it is, or has been,
    /// too large to highlight, so the caller can say why.
    ///
    /// **One way, for the session.** A text that arrives past
    /// `source::HIGHLIGHT_MAX_BYTES` is plain from the start; one that arrives
    /// under it is coloured until an edit takes it past, and is plain from then
    /// on, however small it gets again. Never back on, because a document that
    /// crossed the cap by one character and back would otherwise go grey and
    /// be recoloured from the top on two keystrokes, with a notice flickering
    /// on and off between them. And off at the crossing rather than never,
    /// because laziness only bounds what colouring costs until somebody
    /// glances at the end: a file pasted up to the 512 KiB edit cap and still
    /// coloured would spend over a second of syntect on that one glance, which
    /// is the time budget the cap exists to keep. The pad can never be here:
    /// its cap is that number, which the pad asserts at compile time.
    pub fn past_highlight_cap(&self) -> bool {
        !self.colour
    }

    /// How many logical lines the text holds. Never zero.
    pub fn line_count(&self) -> usize {
        self.buffer.lines().len()
    }

    /// The document row logical line `line` begins on, in the layout in hand.
    pub fn first_row_of(&self, line: usize) -> Option<usize> {
        self.layout.first_row_of(line)
    }

    /// Put the caret at the start of the line `at / of` of the way through the
    /// text — where the files view opens an edit from a rendered page, whose
    /// rows correspond to no source line, so that how far down the reader was
    /// is the nearest honest answer. `of` of zero is the top.
    pub fn caret_to_fraction(&mut self, at: usize, of: usize) -> bool {
        let line = (at.saturating_mul(self.line_count())).checked_div(of).unwrap_or(0);
        self.set_caret(line, 0)
    }
}

/// Where an editor is scrolled to, and whether the next frame owes the caret a
/// place on screen.
///
/// Every pane that holds an [`Editor`] has the same three problems between the
/// editor and the screen, and they are this type. **The caret has to stay in
/// view**: a key that moved it asks [`View::follow`], and the next frame —
/// the first thing that knows how the text wrapped — scrolls just far enough to
/// show it. **The view has to move without the caret**: a glance at a pane
/// somebody is not typing in, and `PgUp`/`PgDn` in one they are, scroll the
/// page and leave the caret where it was, which is what [`View::glance`] does
/// and the reason it is a separate method from anything that edits. **A click
/// has to be measured against the rows that were drawn**, offset and all, which
/// is [`View::click`].
#[derive(Debug, Default)]
pub struct View {
    /// The offset and the measurements, and the vocabulary that moves them.
    /// Public so a caller with a second form of the same text — the pad's
    /// rendering — can scroll that form with the same offset.
    pub scroll: Scroll,
    follow: bool,
    /// A line the next frame should put at the top, if one was asked for.
    top: Option<usize>,
    /// The caret as the last frame drew it, `(column, row)` within the rows
    /// that frame returned.
    cursor: Option<(usize, usize)>,
}

impl View {
    /// The next frame should bring the caret into view.
    pub fn follow(&mut self) {
        self.follow = true;
    }

    /// The next frame should show logical line `line` on its top row — where
    /// the files view opens an edit from the line the reader had at the top of
    /// the page.
    ///
    /// Pending rather than done now, for two reasons that are the same reason.
    /// The row that line begins on is not known until the frame has wrapped
    /// the text, and `Scroll::to` clamps to the last measurement, so a view
    /// that had never been drawn would clamp any offset to zero. It is applied
    /// in [`View::frame`] after measuring, and it outranks a
    /// [`View::follow`] asked for in the same breath: a caret placed on that
    /// line is on screen anyway, and a caret that is not — below the screen
    /// — would otherwise be brought in on the bottom row, and the line asked
    /// for would not be at the top.
    ///
    /// Not the pad's: its own turn between forms keeps a fraction of the
    /// *rendering*, whose rows are no line of the source, so this has nothing
    /// to say there.
    pub fn show_line_at_top(&mut self, line: usize) {
        self.top = Some(line);
    }

    /// Measure, bring the caret into view if it was asked for, and hand back
    /// the `height` rows from the offset — coloured, because asking for them is
    /// what colours them. Call after [`Editor::lay_out`] for the same width.
    pub fn frame(&mut self, editor: &mut Editor, height: usize) -> Vec<Row> {
        self.scroll.measure(editor.row_count(), height);
        if let Some(line) = self.top.take()
            && let Some(row) = editor.first_row_of(line)
        {
            self.scroll.to(row);
            self.follow = false;
        }
        if std::mem::take(&mut self.follow)
            && let Some((row, _)) = editor.caret_cell()
        {
            if row < self.scroll.offset {
                self.scroll.to(row);
            } else if height > 0 && row >= self.scroll.offset + height {
                self.scroll.to(row + 1 - height);
            }
        }
        let offset = self.scroll.offset;
        let rows = editor.rows(offset, height);
        self.cursor = editor
            .caret_cell()
            .filter(|&(row, _)| row >= offset && row < offset + height)
            .map(|(row, col)| (col, row - offset));
        rows
    }

    /// Where the last frame drew the caret, as `(column, row)` within the rows
    /// it returned, or `None` when the caret was not among them.
    pub fn cursor(&self) -> Option<(usize, usize)> {
        self.cursor
    }

    /// Forget the caret, for a frame that drew something else.
    pub fn hide_cursor(&mut self) {
        self.cursor = None;
    }

    /// A click at `row` of the rows the last frame returned, cell `col`. See
    /// [`Editor::click`] for the two it declines.
    pub fn click(&mut self, editor: &mut Editor, row: usize, col: usize) -> Option<bool> {
        editor.click(self.scroll.offset + row, col)
    }

    /// Scroll for a key in the shared scroll vocabulary, and never move the
    /// caret: the glance bindings, and `PgUp`/`PgDn` while typing. Declined
    /// for anything that vocabulary does not know.
    pub fn glance(&mut self, key: KeyEvent) -> Handled {
        self.scroll.key(key).unwrap_or(Handled::No)
    }

    /// The wheel, which means the same thing on a page being written and a
    /// page being read.
    pub fn wheel(&mut self, ev: &MouseEvent) -> Option<Handled> {
        self.scroll.mouse(ev)
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::viewer::source::{self, Grammar};
    use crate::panes::viewer::theme;

    const NOTE: Policy = Policy {
        max_bytes: 64 * 1024,
        tabs: Tabs::Spaces(2),
        line_ending_bytes: 1,
        bom_bytes: 0,
    };

    const FILE: Policy = Policy {
        max_bytes: 512 * 1024,
        tabs: Tabs::Keep { key: TabKey::Tab },
        line_ending_bytes: 1,
        bom_bytes: 0,
    };

    fn md() -> Look {
        Look {
            mode: theme::Mode::Dark,
            grammar: Grammar::for_code("markdown"),
        }
    }

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    /// `text` with the caret at its end, the way the pad opens a note.
    fn loaded(policy: Policy, text: &str) -> Editor {
        let mut e = Editor::from_text(policy, text);
        e.set_caret(usize::MAX, usize::MAX);
        e
    }

    #[test]
    fn the_tab_key_types_what_the_policy_says() {
        let mut e = Editor::new(NOTE);
        assert!(e.tab());
        assert_eq!(e.text(), "  ");

        let mut e = Editor::new(FILE);
        assert!(e.tab());
        assert_eq!(e.text(), "\t");

        // A file with no tab-indented line: the caller keeps its tabs and has
        // the key type spaces.
        let mut e = Editor::new(Policy {
            tabs: Tabs::Keep {
                key: TabKey::Spaces(2),
            },
            ..FILE
        });
        assert!(e.tab());
        assert!(e.insert_str("a\tb"));
        assert_eq!(e.text(), "  a\tb");
    }

    // --- keys ---------------------------------------------------------------

    #[test]
    fn the_editors_keys_are_answered_and_the_panes_are_handed_back() {
        let mut e = loaded(NOTE, "ab");
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(e.key(&ev(KeyCode::Char('c'), none)), Some(Outcome::Edited));
        assert_eq!(e.key(&ev(KeyCode::Left, none)), Some(Outcome::Moved));
        assert_eq!(e.key(&ev(KeyCode::Home, none)), Some(Outcome::Moved));
        assert_eq!(e.key(&ev(KeyCode::Home, none)), Some(Outcome::Still));
        assert_eq!(e.key(&ev(KeyCode::Backspace, none)), Some(Outcome::Still));
        assert_eq!(e.text(), "abc");

        // The pane's questions, untouched and unanswered.
        for (code, mods) in [
            (KeyCode::Esc, none),
            (KeyCode::Char('s'), ctrl),
            (KeyCode::Char('d'), ctrl),
            (KeyCode::Char('t'), KeyModifiers::ALT),
            (KeyCode::PageUp, none),
            (KeyCode::PageDown, none),
            (KeyCode::F(4), none),
        ] {
            assert_eq!(e.key(&ev(code, mods)), None, "{code:?} {mods:?}");
        }
        assert_eq!(e.text(), "abc", "and none of them typed anything");

        // AltGr is text, not a chord.
        let altgr = KeyModifiers::ALT | KeyModifiers::CONTROL;
        assert_eq!(e.key(&ev(KeyCode::Char('€'), altgr)), Some(Outcome::Edited));
        assert_eq!(e.key(&ev(KeyCode::Char('z'), altgr)), Some(Outcome::Edited));
        assert_eq!(e.text(), "€zabc");
    }

    #[test]
    fn a_letter_that_will_not_fit_is_refused_and_still_claimed() {
        let mut e = loaded(NOTE, &"x".repeat(NOTE.max_bytes));
        let q = ev(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(e.key(&q), Some(Outcome::Refused));
        assert_eq!(Outcome::Refused.handled(), Handled::Yes, "q stays text at the cap");
        assert_eq!(e.paste("more"), Outcome::Refused);
        assert_eq!(e.paste(""), Outcome::Still, "an empty clipboard is not too big");
    }

    #[test]
    fn ctrl_shift_z_redoes_where_shift_is_reported_and_caps_lock_z_still_undoes() {
        let mut e = loaded(NOTE, "");
        for c in "one two".chars() {
            e.insert(c);
        }
        let ctrl = KeyModifiers::CONTROL;
        let shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(e.key(&ev(KeyCode::Char('z'), ctrl)), Some(Outcome::Edited));
        assert_eq!(e.text(), "one ");
        assert_eq!(e.key(&ev(KeyCode::Char('Z'), shift)), Some(Outcome::Edited));
        assert_eq!(e.text(), "one two", "Ctrl+Shift+Z is redo");
        assert_eq!(e.key(&ev(KeyCode::Char('Z'), ctrl)), Some(Outcome::Edited));
        assert_eq!(e.text(), "one ", "a capital Z with no Shift is Caps Lock, and undo");
        assert_eq!(e.key(&ev(KeyCode::Char('y'), ctrl)), Some(Outcome::Edited));
        assert_eq!(e.text(), "one two");
        assert_eq!(e.key(&ev(KeyCode::Char('y'), ctrl)), Some(Outcome::Still));
    }

    // --- the saved point ----------------------------------------------------

    #[test]
    fn an_edit_after_a_save_is_undone_back_to_the_save_and_not_past_it() {
        // The bug the saved point fixes, through the editor.
        let mut e = loaded(NOTE, "");
        assert!(!e.is_modified());
        for c in "abc".chars() {
            e.insert(c);
        }
        assert!(e.is_modified());
        e.mark_saved();
        assert!(!e.is_modified());
        for c in "def".chars() {
            e.insert(c);
        }
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        assert!(!e.is_modified(), "undone back to what is on disk");
    }

    #[test]
    fn a_draft_put_over_the_disk_text_is_unsaved_and_one_undo_from_it() {
        // The recovery copy: the file as it is on disk, and the draft from the
        // profile over it.
        let disk = "# Notes\n\nthe old text\n";
        let mut e = Editor::from_text(FILE, disk);
        e.set_caret(2, 4);
        assert_eq!(e.replace_all("# Notes\n\nthe new text, typed last session\n"), Outcome::Edited);
        assert!(e.is_modified(), "a restored draft is unsaved text");
        assert_eq!(e.caret(), (2, 4), "the caret stayed where it was");
        assert!(e.undo());
        assert_eq!(e.text(), disk, "one undo is exactly what is on disk");
        assert!(!e.is_modified());
        assert!(e.redo());
        assert!(e.is_modified());
    }

    #[test]
    fn a_reload_that_keeps_the_history_reads_as_saved_and_undoes_to_what_was_typed() {
        let mut e = loaded(FILE, "line one\n");
        for c in "more".chars() {
            e.insert(c);
        }
        let typed = e.text();
        assert_eq!(e.replace_all("line one\nchanged by the agent\n"), Outcome::Edited);
        e.mark_saved();
        assert!(!e.is_modified(), "the reloaded text is the text on disk");
        assert!(e.undo());
        assert_eq!(e.text(), typed, "and the step before it is what the user had");
        assert!(e.is_modified());

        // The same text is no step at all, and a text too big is refused
        // whole and leaves nothing to undo.
        let mut e = loaded(FILE, "same");
        assert_eq!(e.replace_all("same"), Outcome::Still);
        assert!(!e.undo());
        assert_eq!(e.replace_all(&"x".repeat(FILE.max_bytes + 1)), Outcome::Refused);
        assert_eq!(e.text(), "same");
        assert!(!e.undo());
    }

    #[test]
    fn two_whole_document_steps_both_fit_the_history() {
        // A step that replaces the whole text holds two documents, and the
        // history's budget is four: the two largest steps there can be.
        let small = Policy {
            max_bytes: 100,
            ..FILE
        };
        let mut e = Editor::from_text(small, &"a".repeat(100));
        assert_eq!(e.replace_all(&"b".repeat(100)), Outcome::Edited);
        assert_eq!(e.replace_all(&"c".repeat(100)), Outcome::Edited);
        assert!(e.undo());
        assert!(e.undo());
        assert_eq!(e.text(), "a".repeat(100), "both whole-document steps were kept");
    }

    #[test]
    fn ending_a_step_splits_typing_without_saving_anything() {
        let mut e = loaded(NOTE, "");
        for c in "abc".chars() {
            e.insert(c);
        }
        e.end_step();
        for c in "def".chars() {
            e.insert(c);
        }
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        assert!(e.is_modified(), "ending a step is not a save");
    }

    // --- placing and reading -------------------------------------------------

    #[test]
    fn from_text_starts_at_the_top_and_the_caller_places_the_caret() {
        let mut e = Editor::from_text(NOTE, "one\ntwo\nthree\nfour");
        assert_eq!(e.caret(), (0, 0));
        assert_eq!(e.line_count(), 4);
        assert!(e.caret_to_fraction(1, 2));
        assert_eq!(e.caret(), (2, 0), "half way through four lines");
        e.caret_to_fraction(0, 0);
        assert_eq!(e.caret(), (0, 0), "of nothing is the top");
        assert!(e.caret_to_fraction(9, 1));
        assert_eq!(e.caret(), (3, 0), "past the end is the last line");
        e.lay_out(3, md());
        assert_eq!(e.first_row_of(0), Some(0));
        assert_eq!(e.first_row_of(2), Some(2));
        assert_eq!(e.first_row_of(3), Some(4), "`three` wraps onto two rows at three");
        assert_eq!(e.first_row_of(4), None);
    }

    #[test]
    fn undo_and_redo_go_through_the_editor_and_tell_the_layout() {
        let mut e = loaded(NOTE, "one");
        e.lay_out(20, md());
        for c in " two".chars() {
            e.insert(c);
        }
        e.lay_out(20, md());
        assert!(e.undo());
        assert_eq!(e.text(), "one ");
        assert_eq!(e.caret_cell(), None, "the layout is of the text before the undo");
        e.lay_out(20, md());
        assert_eq!(e.caret_cell(), Some((0, 4)));
        assert!(e.redo());
        assert_eq!(e.text(), "one two");
        assert!(!e.redo());
    }

    #[test]
    fn a_refused_edit_is_not_a_step() {
        let mut e = loaded(NOTE, &"a".repeat(NOTE.max_bytes - 1));
        assert!(!e.insert('é'), "two bytes into one byte of room");
        assert!(e.insert('b'));
        assert!(e.undo());
        assert_eq!(e.text().len(), NOTE.max_bytes - 1);
        assert!(!e.undo(), "the refusal left nothing to take back");
    }

    #[test]
    fn a_click_is_declined_against_a_stale_layout_and_on_the_kept_back_column() {
        let mut e = loaded(NOTE, "alpha\nbravo\ncharlie");
        e.lay_out(10, md());
        assert_eq!(e.click(1, 2), Some(true));
        assert_eq!(e.caret(), (1, 2));
        assert_eq!(e.click(1, 10), None, "column ten was never given to the layout");

        e.newline();
        assert_eq!(e.click(0, 0), None, "the table is of the text before Enter");
        e.lay_out(10, md());
        assert_eq!(e.click(0, 0), Some(true));
        assert_eq!(e.caret(), (0, 0));
    }

    #[test]
    fn a_tab_is_drawn_and_clicked_in_cells() {
        let mut e = loaded(FILE, "\tx = 1");
        e.lay_out(40, Look { mode: theme::Mode::Dark, grammar: None });
        let row: String = e.rows(0, 1)[0].text.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(row, "    x = 1");
        assert_eq!(e.caret_cell(), Some((0, 9)));
        assert_eq!(e.click(0, 2), Some(true));
        assert_eq!(e.caret(), (0, 0), "inside the tab is in front of it");
        assert_eq!(e.click(0, 4), Some(true));
        assert_eq!(e.caret(), (0, 1));
    }

    #[test]
    fn colour_goes_off_once_for_good_when_the_text_passes_the_highlight_cap() {
        let cap = source::HIGHLIGHT_MAX_BYTES;
        // At the cap: coloured. One character past it: plain, and the layout
        // in hand gives its colours up on the next frame.
        let mut e = loaded(FILE, &"x".repeat(cap));
        assert!(!e.past_highlight_cap());
        e.lay_out(40, md());
        assert!(!e.layout.plain());
        e.insert('y');
        assert!(e.past_highlight_cap(), "an edit past the cap takes the colour away");
        e.lay_out(40, md());
        assert!(e.layout.plain());

        // And it does not come back, however small the text gets again.
        e.backspace();
        e.backspace();
        assert!(e.past_highlight_cap(), "back under the cap and still plain");
        let mut e = loaded(FILE, &"x".repeat(cap + 1));
        assert!(e.past_highlight_cap(), "too big on arrival is plain from the start");
        e.replace_all("small");
        assert!(e.past_highlight_cap(), "replacing the text is an edit, not a load");
        e.lay_out(40, md());
        assert!(e.layout.plain(), "and the layout draws it without colour");
    }

    #[test]
    fn a_ctrl_or_alt_on_a_key_that_is_not_a_character_hands_it_back() {
        let mut e = loaded(NOTE, "one\ntwo");
        e.set_caret(1, 1);
        for (code, mods) in [
            (KeyCode::Tab, KeyModifiers::CONTROL),
            (KeyCode::Enter, KeyModifiers::ALT),
            (KeyCode::Backspace, KeyModifiers::CONTROL),
            (KeyCode::Delete, KeyModifiers::CONTROL),
            (KeyCode::Left, KeyModifiers::CONTROL),
            (KeyCode::Up, KeyModifiers::ALT),
            (KeyCode::Home, KeyModifiers::CONTROL),
            (KeyCode::End, KeyModifiers::ALT | KeyModifiers::CONTROL),
        ] {
            assert_eq!(e.key(&ev(code, mods)), None, "{code:?} with {mods:?}");
        }
        assert_eq!(e.text(), "one\ntwo");
        assert_eq!(e.caret(), (1, 1), "and nothing moved");

        // Shift is what a hand still on it after a capital adds.
        assert_eq!(e.key(&ev(KeyCode::Enter, KeyModifiers::SHIFT)), Some(Outcome::Edited));
        assert_eq!(e.key(&ev(KeyCode::Left, KeyModifiers::SHIFT)), Some(Outcome::Moved));
        assert_eq!(e.text(), "one\nt\nwo");
    }

    // --- the view -------------------------------------------------------------

    #[test]
    fn the_view_follows_the_caret_when_asked_and_a_glance_never_moves_it() {
        let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let mut e = loaded(NOTE, &text);
        let mut v = View::default();
        v.follow();
        e.lay_out(20, md());
        let rows = v.frame(&mut e, 6);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows.last().map(|r| r.line), Some(40), "the end, where the caret is");
        assert_eq!(v.cursor(), Some((0, 5)));

        // A glance scrolls the page and leaves the caret where it was.
        let caret = e.caret();
        assert_eq!(v.glance(ev(KeyCode::PageUp, KeyModifiers::NONE)), Handled::Yes);
        e.lay_out(20, md());
        v.frame(&mut e, 6);
        assert_eq!(e.caret(), caret);
        assert_eq!(v.cursor(), None, "the caret is off screen now, and not dragged on");

        // A click is measured against the rows drawn, offset and all.
        let top = v.scroll.offset;
        assert_eq!(v.click(&mut e, 1, 2), Some(true));
        assert_eq!(e.caret(), (top + 1, 2));
    }

    #[test]
    fn a_line_asked_for_at_the_top_is_at_the_top_of_the_first_frame() {
        // A view that has never been drawn: `Scroll::to` would clamp anything
        // to zero, so the line waits for the frame. The caret is at the end,
        // below the screen, and asking it to be followed in the same breath
        // must not pull the page back down to it.
        let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let mut e = loaded(NOTE, &text);
        let mut v = View::default();
        v.follow();
        v.show_line_at_top(10);
        e.lay_out(20, md());
        let rows = v.frame(&mut e, 6);
        assert_eq!(rows[0].line, 10);
        assert_eq!(v.cursor(), None, "the caret was not dragged into view");

        // Near the end it is as near as the page can go.
        v.show_line_at_top(39);
        e.lay_out(20, md());
        let rows = v.frame(&mut e, 6);
        assert_eq!(rows.last().map(|r| r.line), Some(40));
        assert_eq!(v.scroll.offset, 41 - 6);
    }

    // --- the layout, from every angle ---------------------------------------

    /// A markdown document with tabs, wide characters, fences and blank lines.
    const DOC: &str = "# Notes\n\nThe retry *budget* is wrong.\n\n```rust\nfn main() {\n\
                       \tlet x = 1;\n}\n```\n\n- one\n\t- 二つ\n\n> quote\n\nThe end.";

    #[test]
    fn a_mended_layout_is_the_layout_built_from_nothing_from_every_angle() {
        // The property the layout is for, through the editor that drives it:
        // a random walk of edits — typed, pasted, rubbed out, undone and redone
        // — with widths and palettes changing between them and frames asked
        // for at random offsets, as a scrolling reader asks for them. After
        // each step the mended layout must agree with one built from nothing
        // on every row, wherever the rows are asked from; on where every caret
        // position is drawn; and on what every cell a pointer could press
        // means.
        let mut e = Editor::from_text(FILE, DOC);
        let pieces = ["x", "```", "\t", "日本", "<div>", "*", "\n", "- ", "py", "\n\n"];
        let mut width = 24usize;
        let mut look = md();
        let mut seed = 0x853C_49E6_748F_EA9Bu64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        for step in 0..150 {
            let lines = e.line_count();
            e.set_caret(rand() % lines, rand() % 30);
            match rand() % 9 {
                0..=2 => {
                    e.insert_str(pieces[rand() % pieces.len()]);
                }
                3 => {
                    for _ in 0..1 + rand() % 6 {
                        e.backspace();
                    }
                }
                4 => {
                    e.delete();
                }
                5 => {
                    e.undo();
                }
                6 => {
                    e.redo();
                }
                // A frame at another width, with no edit at all.
                7 => width = [7usize, 13, 24, 40][rand() % 4],
                // F3.
                _ => {
                    look.mode = match look.mode {
                        theme::Mode::Dark => theme::Mode::Light,
                        theme::Mode::Light => theme::Mode::Dark,
                    }
                }
            }
            e.lay_out(width, look);
            // A frame somewhere, coloured lazily from wherever colouring had got.
            let n = e.row_count();
            e.rows(rand() % (n + 1), 4);

            let mut fresh = Editor::from_text(FILE, &e.text());
            fresh.lay_out(width, look);
            assert_eq!(n, fresh.row_count(), "step {step}: the row counts differ");
            for k in 0..n {
                assert_eq!(e.rows(k, 3), fresh.rows(k, 3), "step {step}: rows from {k}");
            }
            let text = e.text();
            for (r, line) in text.split('\n').enumerate() {
                for c in 0..=line.chars().count() {
                    assert_eq!(
                        e.layout.caret_cell((r, c)),
                        fresh.layout.caret_cell((r, c)),
                        "step {step}: caret {r},{c}"
                    );
                }
            }
            for row in 0..n {
                for col in 0..width {
                    assert_eq!(
                        e.layout.hit(row, col),
                        fresh.layout.hit(row, col),
                        "step {step}: a click at {row},{col}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_mended_layout_drawn_a_window_at_a_time_is_the_layout_built_from_nothing() {
        // The test above colours everything on every step, so the chain below
        // an edit is always consumed before the next edit can meet it. Here
        // frames are a window at a time, as a reader scrolling a long file
        // draws them, so edits arrive while colouring is part way down and a
        // chain is still waiting: a second edit before any frame — on the line
        // the first was on, or inside the chain it left — an edit below the
        // watermark, a multi-line paste across it and its undo. Each window is
        // checked against a fresh layout, and the whole of it every tenth step;
        // and the three ways `patch` keeps a chain must each have happened, or
        // the walk proved nothing about them.
        //
        // Rust rather than markdown, because the chain cares only that the
        // grammar's state changes from line to line — a block comment opened
        // and closed does that — and syntect in a debug build is the whole cost
        // of this test, at a fraction of markdown's price per line.
        let rust = Look {
            mode: theme::Mode::Dark,
            grammar: Grammar::for_code("rust"),
        };
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(match i % 10 {
                0 => "/* a block",
                1 => "comment */",
                4 => "fn f() {",
                5 => "\tlet s = 1;",
                6 => "}",
                _ => "let a = 2;",
            });
            text.push('\n');
        }
        let mut e = Editor::from_text(FILE, &text);
        let width = 30;
        let pieces = ["x", "/*", "*/", "\t", "\"", "\n", "//", "日本", "a\n/*\nb"];
        let mut seed = 0x6A09_E667_F3BC_C908u64;
        let mut rand = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        e.lay_out(width, rust);
        e.rows(0, 4);
        for step in 0..200 {
            let lines = e.line_count();
            let valid = e.layout.valid().min(lines);
            match rand() % 7 {
                // An edit inside what has been coloured.
                0 | 1 => {
                    e.set_caret(rand() % valid.max(1), rand() % 12);
                    e.insert_str(pieces[rand() % pieces.len()]);
                }
                // Two edits with no frame between them: the second meets the
                // chain the first left, on the same line or inside it.
                2 => {
                    let row = rand() % valid.max(1);
                    e.set_caret(row, 2);
                    e.insert_str("x");
                    let again = if rand() % 2 == 0 {
                        row
                    } else {
                        row + 1 + rand() % valid.saturating_sub(row + 1).max(1)
                    };
                    e.set_caret(again, 3);
                    e.insert_str("y");
                }
                // Below the watermark.
                3 => {
                    e.set_caret(valid + rand() % (lines - valid).max(1), rand() % 12);
                    e.insert_str(pieces[rand() % pieces.len()]);
                }
                // A paste of several lines across the watermark, a window, and
                // the paste undone.
                4 => {
                    e.set_caret(valid.saturating_sub(1), 0);
                    e.insert_str("one\n/* two\nthree */\nfour\n");
                    e.lay_out(width, rust);
                    let n = e.row_count();
                    e.rows(rand() % (n + 1), 4);
                    e.undo();
                }
                5 => {
                    e.set_caret(rand() % lines, rand() % 12);
                    for _ in 0..1 + rand() % 4 {
                        e.backspace();
                    }
                }
                _ => {
                    e.undo();
                }
            }
            e.lay_out(width, rust);
            let mut fresh = Editor::from_text(FILE, &e.text());
            fresh.lay_out(width, rust);
            let n = e.row_count();
            assert_eq!(n, fresh.row_count(), "step {step}: the row counts differ");
            let k = rand() % (n + 1);
            assert_eq!(e.rows(k, 4), fresh.rows(k, 4), "step {step}: the window at {k}");
            if step % 10 == 9 {
                assert_eq!(e.rows(0, n), fresh.rows(0, n), "step {step}: the whole of it");
                // Start partial again, so later steps have a watermark to work
                // against.
                let at = rand() % e.line_count();
                e.set_caret(at, 0);
                e.insert_str("z");
                e.lay_out(width, rust);
                e.rows(0, 4);
            }
        }
        let hits = e.layout.chain_hits();
        assert!(hits.inside > 0, "no edit inside the coloured part: {hits:?}");
        assert!(hits.kept > 0, "no edit above a waiting chain: {hits:?}");
        assert!(hits.cut > 0, "no edit inside a waiting chain: {hits:?}");
    }

    #[test]
    fn a_fresh_layout_draws_what_the_whole_document_path_drew() {
        // The pad laid itself out with `whole_document` on every keystroke, and
        // the editor must draw what it drew: the same rows, the same colours.
        // The front of the README, which is long enough to hold every
        // construct it has and short enough for a debug build.
        let readme = readme();
        let front: String = readme.split('\n').take(220).collect::<Vec<_>>().join("\n");
        let mut e = Editor::from_text(NOTE, &front);
        e.lay_out(60, md());
        let n = e.row_count();
        let mine: Vec<_> = e.rows(0, n).into_iter().map(|r| r.text).collect();
        assert_eq!(mine, whole_document(&front, 60));
    }

    // --- what it costs --------------------------------------------------------

    /// The README, which is the document the performance target names.
    const README: &str = include_str!("../../../../README.md");

    /// The README as the pad would hold it. A Windows checkout has it with
    /// `\r\n` endings, which the buffer cleans on the way in; the measurement
    /// of the old path below has to be handed the same text the new one holds,
    /// or it is timing carriage returns the pad never sees.
    fn readme() -> String {
        README.replace("\r\n", "\n")
    }

    /// What the pad's `ensure_layout` used to do on every keystroke: highlight
    /// the whole text and wrap every line. Kept here as the yardstick, and as
    /// the reference a fresh layout is checked against once.
    fn whole_document(text: &str, width: usize) -> Vec<ratatui::text::Line<'static>> {
        let styled = source::highlight_code(text, "markdown", theme::Mode::Dark);
        let mut rows = Vec::new();
        for (i, line) in text.split('\n').enumerate() {
            let spans = layout::faithful(styled.get(i).cloned(), line);
            let starts = wrap::breaks(line, width);
            rows.extend(wrap::into_rows(&spans, &starts));
        }
        rows
    }

    #[test]
    fn an_edit_to_a_long_document_costs_a_line_and_not_the_document() {
        // The regression this module exists to prevent, caught two ways. The
        // first cannot flake: a character typed into the middle of a 46 KB
        // document recolours and re-wraps a handful of lines, not a thousand.
        // The second is a clock with a bound generous enough for a debug build
        // on a slow machine and still far below what laying the whole document
        // out costs — which this test also measures, so the margin is visible
        // in the failure message rather than assumed.
        let readme = readme();
        assert!(readme.len() > 40_000, "the fixture is the long document it claims");
        let all: Vec<&str> = readme.split('\n').collect();
        let lines = all.len();
        let mut e = Editor::from_text(NOTE, &readme);
        let mut v = View::default();
        let started = std::time::Instant::now();
        e.lay_out(60, md());
        e.rows(0, e.row_count());
        let whole = started.elapsed();

        // Somewhere in the middle, inside a paragraph, on screen.
        let row = (lines / 2..lines)
            .find(|&r| all[r].len() > 40 && all[r].starts_with(char::is_alphabetic))
            .expect("a prose line in the second half of the README");
        e.set_caret(row, 10);
        v.follow();
        e.lay_out(60, md());
        v.frame(&mut e, 30);
        e.insert('x');
        e.lay_out(60, md());
        v.frame(&mut e, 30);
        let touched = e.layout.touched();
        assert!(
            touched.highlighted <= 3 && touched.wrapped == 1,
            "one character typed did {touched:?} on a {lines}-line document"
        );

        let edits = 20u32;
        let started = std::time::Instant::now();
        for _ in 0..edits {
            e.insert('y');
            e.lay_out(60, md());
            v.frame(&mut e, 30);
        }
        let each = started.elapsed() / edits;
        assert!(
            each < whole / 10 && each < std::time::Duration::from_millis(50),
            "an edit cost {each:?}; laying the whole document out cost {whole:?}"
        );
    }

    /// The numbers the module doc quotes.
    ///
    /// `cargo test --release -p abeam editor_bench -- --ignored --nocapture`.
    /// Ignored because a timing is a report rather than a test: the regression
    /// guards are the tests above, and this is where their margins come from.
    #[test]
    #[ignore = "a measurement to read, not an assertion: run it in release"]
    fn editor_bench() {
        use std::time::{Duration, Instant};
        let width = 60;
        let height = 40;
        let readme = readme();
        let lines: Vec<&str> = readme.split('\n').collect();

        let time = |rounds: u32, f: &mut dyn FnMut()| -> Duration {
            let started = Instant::now();
            for _ in 0..rounds {
                f();
            }
            started.elapsed() / rounds
        };

        // Before: what the pad's `ensure_layout` did on every keystroke.
        let before = time(10, &mut || {
            std::hint::black_box(whole_document(&readme, width));
        });

        // After: one character typed into the middle of a paragraph, on screen.
        let mut e = Editor::from_text(NOTE, &readme);
        let mut v = View::default();
        e.set_caret(lines.len() / 2, 0);
        v.follow();
        e.lay_out(width, md());
        v.frame(&mut e, height);
        let typing = time(200, &mut || {
            e.insert('y');
            e.lay_out(width, md());
            v.frame(&mut e, height);
        });

        // Typing in a fence's info string at the top of a long code block,
        // which changes the state of every line in the block.
        let mut block = String::from("```py\n");
        for i in 0..1500 {
            block.push_str(&format!("    value_{i} = compute({i}, \"text\")\n"));
        }
        block.push_str("```\n");
        let mut e = Editor::from_text(NOTE, &block);
        let mut v = View::default();
        e.set_caret(0, 5);
        e.lay_out(width, md());
        v.frame(&mut e, height);
        let fence = time(20, &mut || {
            e.insert('t');
            e.lay_out(width, md());
            v.frame(&mut e, height);
        });

        // Opening: 64 KiB of markdown, coloured, and 512 KiB, plain; the first
        // frame of each, at the top. And the reader's whole-file highlight of
        // the same 64 KiB, which is what an open used to be measured against.
        // Whole lines of the README, repeated, up to `max` bytes.
        let lines_up_to = |max: usize| -> String {
            let mut out = String::new();
            for line in readme.split('\n').cycle() {
                if out.len() + line.len() + 1 > max {
                    break;
                }
                out.push_str(line);
                out.push('\n');
            }
            out
        };
        let big = lines_up_to(source::HIGHLIGHT_MAX_BYTES);
        let open_64 = time(10, &mut || {
            let mut e = Editor::from_text(FILE, &big);
            let mut v = View::default();
            e.lay_out(width, md());
            std::hint::black_box(v.frame(&mut e, height));
        });
        let reader_64 = time(5, &mut || {
            std::hint::black_box(source::highlight_code(&big, "markdown", theme::Mode::Dark));
        });
        let colour_all_64 = time(5, &mut || {
            let mut e = Editor::from_text(FILE, &big);
            e.lay_out(width, md());
            let n = e.row_count();
            std::hint::black_box(e.rows(0, n));
        });
        let huge = lines_up_to(FILE.max_bytes);
        let open_512 = time(5, &mut || {
            let mut e = Editor::from_text(FILE, &huge);
            let mut v = View::default();
            e.lay_out(width, md());
            std::hint::black_box(v.frame(&mut e, height));
        });

        println!(
            "README {} bytes, {} lines, {width}x{height}: \
             whole-document layout {before:?} per edit; typing {typing:?} per edit; \
             typing in a 1500-line block's info string {fence:?} per edit; \
             open {} KiB coloured {open_64:?} (reader {reader_64:?}, colouring all of it \
             {colour_all_64:?}); open {} KiB plain {open_512:?}",
            readme.len(),
            lines.len(),
            big.len() / 1024,
            huge.len() / 1024,
        );
    }
}
