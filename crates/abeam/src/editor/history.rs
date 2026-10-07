//! Undo and redo: what `Ctrl+Z` takes back, and how much of the past is kept.
//!
//! A step is one [`Change`] — where it began, what was there, what is there
//! now — and the two carets either side of it. That is enough to go both ways,
//! because `Buffer::splice` is the only thing that writes the text (the
//! buffer's module doc, *One writer*) and every edit it makes is a replacement
//! of one range by one string:
//! undo replaces what went in with what came out, and redo the other way. No
//! snapshot of the document is ever taken, so the cost of a step is the text it
//! touched and not the text it was made in.
//!
//! ## What one step is
//!
//! Not one keystroke. An undo that took back a character at a time would have
//! a sentence cost forty presses to unwrite, and a key held down to do that is
//! a key that overshoots. So the newest step stays *open*, and the next edit
//! joins it rather than starting another when it continues the same run:
//!
//! - **Typing** — characters, `Enter`, and the `Tab` key's text — joins while
//!   each insertion lands exactly where the last one finished, and until a word
//!   begins after a space or a punctuation mark. A step is therefore a word with
//!   whatever followed it, and `Ctrl+Z` takes a sentence back the way it was
//!   written, a word at a time from the end.
//! - **Backspace** joins a run of backspaces that each took the character
//!   before the last one taken; **Delete** a run that each took the character
//!   at the same place. The two never join each other, nor typing: "type a
//!   word, rub out three letters" is two steps, because the user did two
//!   things.
//! - **A paste** is always a step of its own and closes the one before it, so a
//!   paste can be taken back without the word typed in front of it.
//!
//! Anything that is not an edit closes the open step — an arrow, a click,
//! `Home`, `End`, an undo, a redo, and a save — so a word typed, a click
//! elsewhere and a word typed there are two steps even if the second happens
//! to begin where the first ended. Contiguity alone would join them, and a step
//! that spans a click is not a thing anybody did. A save is on that list for a
//! harder reason than taste, which the next section gives.
//!
//! ## The saved point
//!
//! Every step has an id, given in increasing order, and the history's position
//! is the id of the newest step on the undo stack — or, with that stack empty,
//! the id of the last step that fell off the bottom of it, which is `0` for a
//! text nobody has edited. [`History::mark_saved`] writes that position down
//! and [`History::is_modified`] compares the two, so "is this the text that was
//! saved" is one comparison however the user got here: by typing, by undoing
//! back to it, or by undoing past it and redoing forward again.
//!
//! The comparison is only honest if a step never changes after it has been
//! saved, and an open step is a step that changes. Type `abc`, save, type
//! `def`: had `def` joined the open step, the history would still be at the
//! saved id with `abcdef` on screen — reporting nothing unsaved — and `Ctrl+Z`
//! would take back all six letters, to a text that was never on disk. So
//! saving closes the open step, and [`History::record`] refuses to grow the
//! step a save is pointing at even if something forgot to.
//!
//! The comparison is of positions, not of text, which makes it strict in one
//! direction. Type a character and rub it out and the text is what was saved,
//! but the history is two steps on, so the document reads as modified — the
//! rule every editor that has a dirty marker follows, and the cheap one: the
//! other is a whole-text comparison on every key. A saved step that falls off
//! the bottom of the history can still be returned to while it is the floor —
//! undoing everything lands on the text it saved, and reads as unmodified,
//! because the floor *is* that position. Once a newer step has fallen off after
//! it the saved text is out of reach, and the document reads as modified until
//! it is saved again, which is true.
//!
//! The caret goes back with the text. Undo leaves it where it was before the
//! step began, and redo where it was after the step's last edit, because that
//! is where the user was looking when they did it — not merely somewhere near
//! the change.
//!
//! ## How much is kept
//!
//! **At most [`STEPS`] steps, and at most four times the policy's cap in
//! text**, counted over the undo and the redo stack together, whichever is
//! reached first; the oldest steps go first. For the pad that is 256 KiB of
//! text, and for a file at the 512 KiB edit cap, 2 MiB. Each step also carries
//! about a hundred bytes of bookkeeping — two strings and three positions —
//! so a full history of short steps is about another 100 KiB on top, and the
//! allocator's slack on a run that grew a character at a time can add as much
//! again as the text in the open step. Both numbers are set by what the step
//! *is*. An ordinary step holds at most one document — there is no selection
//! to replace, so a step either adds text or takes it away — and the one step
//! that does both is `Editor::replace_all`, which takes the whole of one text
//! out and puts the whole of another in: two documents, each inside the cap.
//! So four documents is room for two of the largest step there can be, and a
//! step is never too big to keep. The newest step is always kept whatever the
//! budget says.
//!
//! What falls off the end is the oldest part of the session, which is the part
//! a user is least likely to want and most likely to have saved since.

use std::collections::VecDeque;

use super::buffer::{Buffer, Change, Pos, end_of};

/// The most steps the history holds. See the module doc.
pub const STEPS: usize = 1000;

/// What a single edit was, which is what decides whether it may join the step
/// before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A character, `Enter`, or the text the `Tab` key types.
    Typing,
    /// One character taken from in front of the caret.
    Backspace,
    /// One character taken from under it.
    Delete,
    /// A paste, which never joins anything.
    Paste,
}

/// One thing undo can take back.
#[derive(Debug)]
struct Step {
    /// Given in increasing order, never reused. See *The saved point*.
    id: u64,
    change: Change,
    /// The caret before the step's first edit.
    before: Pos,
    /// The caret after its last.
    after: Pos,
}

impl Step {
    fn bytes(&self) -> usize {
        self.change.removed.len() + self.change.inserted.len()
    }
}

/// The past and, after an undo, the future.
pub struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    /// The kind of the newest undo step while it may still grow, and `None`
    /// once anything has closed it.
    open: Option<Kind>,
    /// Text held across both stacks, in bytes.
    held: usize,
    /// The bound on steps. [`STEPS`] outside tests.
    steps: usize,
    /// The bound on text.
    bytes: usize,
    /// The id the next step will be given.
    next: u64,
    /// The id of the newest step dropped off the bottom of the undo stack, or
    /// `0` while none has been. The history's position when that stack is
    /// empty.
    floor: u64,
    /// The position at the last [`History::mark_saved`], `0` — the text as it
    /// arrived — until there has been one.
    saved: u64,
}

impl History {
    /// An empty history for a document held to `cap` bytes. See the module
    /// doc for the bound this sets.
    pub fn new(cap: usize) -> Self {
        Self::bounded(STEPS, cap.saturating_mul(4))
    }

    fn bounded(steps: usize, bytes: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            open: None,
            held: 0,
            steps: steps.max(1),
            bytes,
            next: 1,
            floor: 0,
            saved: 0,
        }
    }

    /// Where the history is: the id of the newest step that has happened.
    fn here(&self) -> u64 {
        self.undo.back().map_or(self.floor, |step| step.id)
    }

    /// The text as it is now is the text that was saved. Closes the open step,
    /// for the reason *The saved point* gives.
    pub fn mark_saved(&mut self) {
        self.seal();
        self.saved = self.here();
    }

    /// Whether the text is anything other than what was last saved — or, before
    /// any save, what arrived. See *The saved point* for the one way this is
    /// strict.
    pub fn is_modified(&self) -> bool {
        self.here() != self.saved
    }

    /// Note an edit that has just happened: `change` is what it did, and
    /// `before` and `after` are where the caret was either side of it.
    ///
    /// Any redo there was is gone, because the future it described started
    /// from a text that is no longer the text. That is every editor's rule and
    /// the alternative — a branching history — is a second thing to draw.
    pub fn record(&mut self, kind: Kind, change: Change, before: Pos, after: Pos) {
        self.forget_redo();
        let saved = self.saved;
        let grown = self.open == Some(kind)
            && self
                .undo
                .back_mut()
                .is_some_and(|top| top.id != saved && join(kind, top, &change, after));
        if grown {
            self.held += change.removed.len() + change.inserted.len();
        } else {
            let step = Step {
                id: self.next,
                change,
                before,
                after,
            };
            self.next += 1;
            self.held += step.bytes();
            self.undo.push_back(step);
        }
        // A paste closes behind itself: nothing joins it, and the next edit is
        // a step of its own.
        self.open = (kind != Kind::Paste).then_some(kind);
        self.trim();
    }

    /// End the open step, so the next edit starts another. What every key that
    /// is not an edit says to the history.
    pub fn seal(&mut self) {
        self.open = None;
    }

    /// Take the newest step back. Returns whether there was one.
    pub fn undo(&mut self, buffer: &mut Buffer) -> bool {
        let Some(step) = self.undo.pop_back() else {
            return false;
        };
        let end = end_of(step.change.at, &step.change.inserted);
        buffer.replace(step.change.at, end, &step.change.removed);
        buffer.set_caret(step.before.0, step.before.1);
        self.redo.push(step);
        self.open = None;
        true
    }

    /// Put the last step taken back in again. Returns whether there was one.
    pub fn redo(&mut self, buffer: &mut Buffer) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        let end = end_of(step.change.at, &step.change.removed);
        buffer.replace(step.change.at, end, &step.change.inserted);
        buffer.set_caret(step.after.0, step.after.1);
        self.undo.push_back(step);
        self.open = None;
        true
    }

    fn forget_redo(&mut self) {
        for step in self.redo.drain(..) {
            self.held -= step.bytes();
        }
    }

    /// Drop the oldest steps until the history is inside both bounds, keeping
    /// the newest whatever they say. Called after a record, which is the only
    /// thing that grows it; undo and redo move steps between the stacks and
    /// change neither count.
    fn trim(&mut self) {
        while self.undo.len() > 1 && (self.undo.len() > self.steps || self.held > self.bytes) {
            if let Some(old) = self.undo.pop_front() {
                self.held -= old.bytes();
                self.floor = old.id;
            }
        }
    }
}

/// Fold `change` into the open step `top` if it continues the same run, and
/// say whether it did. See the module doc's *What one step is*.
fn join(kind: Kind, top: &mut Step, change: &Change, after: Pos) -> bool {
    let fits = match kind {
        Kind::Typing => {
            top.change.removed.is_empty()
                && change.removed.is_empty()
                && change.at == end_of(top.change.at, &top.change.inserted)
                && !starts_a_word(&top.change.inserted, &change.inserted)
        }
        Kind::Backspace => {
            top.change.inserted.is_empty()
                && change.inserted.is_empty()
                && end_of(change.at, &change.removed) == top.change.at
        }
        Kind::Delete => {
            top.change.inserted.is_empty()
                && change.inserted.is_empty()
                && change.at == top.change.at
        }
        Kind::Paste => false,
    };
    if !fits {
        return false;
    }
    match kind {
        Kind::Typing => top.change.inserted.push_str(&change.inserted),
        Kind::Backspace => {
            top.change.removed.insert_str(0, &change.removed);
            top.change.at = change.at;
        }
        Kind::Delete => top.change.removed.push_str(&change.removed),
        Kind::Paste => {}
    }
    top.after = after;
    true
}

/// Whether typing `next` after `typed` begins a new word: a letter or a digit
/// arriving after something that is neither.
fn starts_a_word(typed: &str, next: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let after_gap = typed.chars().next_back().is_some_and(|c| !word(c));
    let begins = next.chars().next().is_some_and(word);
    after_gap && begins
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Policy;

    const NOTE: Policy = Policy {
        max_bytes: 64 * 1024,
        tabs: crate::editor::Tabs::Spaces(2),
        line_ending_bytes: 1,
        bom_bytes: 0,
    };

    /// A buffer and its history, driven the way `Editor` drives them: the
    /// edit, then the record of it, with a seal for every key that is not an
    /// edit. Kept here rather than going through `Editor` so these tests are
    /// about the history alone.
    struct Rig {
        b: Buffer,
        h: History,
    }

    impl Rig {
        fn new(text: &str) -> Self {
            let mut b = Buffer::from_text(NOTE, text);
            b.set_caret(usize::MAX, usize::MAX);
            Self {
                b,
                h: History::new(NOTE.max_bytes),
            }
        }

        fn edit(&mut self, kind: Kind, f: impl FnOnce(&mut Buffer) -> bool) -> bool {
            let before = self.b.caret();
            let did = f(&mut self.b);
            if let Some(change) = self.b.take_change() {
                self.h.record(kind, change, before, self.b.caret());
            }
            did
        }

        fn typed(&mut self, s: &str) {
            for c in s.chars() {
                let kind = Kind::Typing;
                self.edit(kind, |b| if c == '\n' { b.newline() } else { b.insert(c) });
            }
        }

        fn moved(&mut self, f: impl FnOnce(&mut Buffer) -> bool) {
            f(&mut self.b);
            self.h.seal();
        }

        fn undo(&mut self) -> bool {
            self.h.undo(&mut self.b)
        }

        fn redo(&mut self) -> bool {
            self.h.redo(&mut self.b)
        }
    }

    // --- what one step is -------------------------------------------------

    #[test]
    fn a_sentence_comes_back_a_word_at_a_time_from_the_end() {
        let mut r = Rig::new("");
        r.typed("the retry budget.");
        assert!(r.undo());
        assert_eq!(r.b.text(), "the retry ", "the last word and what followed it");
        assert!(r.undo());
        assert_eq!(r.b.text(), "the ");
        assert!(r.undo());
        assert_eq!(r.b.text(), "");
        assert!(!r.undo(), "and nothing before the first word");
    }

    #[test]
    fn enter_ends_a_line_inside_the_word_it_follows() {
        let mut r = Rig::new("");
        r.typed("one\ntwo");
        assert!(r.undo());
        assert_eq!(r.b.text(), "one\n");
        assert!(r.undo());
        assert_eq!(r.b.text(), "");
    }

    #[test]
    fn a_run_of_backspaces_is_one_step_and_so_is_a_run_of_deletes() {
        let mut r = Rig::new("alpha beta\ngamma");
        r.moved(|b| b.set_caret(1, 2));
        for _ in 0..5 {
            r.edit(Kind::Backspace, Buffer::backspace);
        }
        assert_eq!(r.b.text(), "alpha bemma", "across the line ending, too");
        assert!(r.undo());
        assert_eq!(r.b.text(), "alpha beta\ngamma");
        assert_eq!(r.b.caret(), (1, 2), "the caret is back where the run began");

        r.moved(|b| b.set_caret(0, 0));
        for _ in 0..6 {
            r.edit(Kind::Delete, Buffer::delete);
        }
        assert_eq!(r.b.text(), "beta\ngamma");
        assert!(r.undo());
        assert_eq!(r.b.text(), "alpha beta\ngamma");
        assert_eq!(r.b.caret(), (0, 0));
        assert!(!r.undo(), "the six deletes were one step, and the only one left");
    }

    #[test]
    fn typing_then_rubbing_out_is_two_steps_because_it_was_two_things() {
        let mut r = Rig::new("");
        r.typed("helo");
        for _ in 0..2 {
            r.edit(Kind::Backspace, Buffer::backspace);
        }
        r.typed("llo");
        assert_eq!(r.b.text(), "hello");
        assert!(r.undo());
        assert_eq!(r.b.text(), "he");
        assert!(r.undo());
        assert_eq!(r.b.text(), "helo");
        assert!(r.undo());
        assert_eq!(r.b.text(), "");
    }

    #[test]
    fn a_paste_is_one_step_and_joins_nothing_either_side_of_it() {
        let mut r = Rig::new("");
        r.typed("see");
        r.edit(Kind::Paste, |b| b.insert_str("the\nstack trace"));
        r.typed("here");
        assert_eq!(r.b.text(), "seethe\nstack tracehere");
        assert!(r.undo());
        assert_eq!(r.b.text(), "seethe\nstack trace", "the typing after it");
        assert!(r.undo());
        assert_eq!(r.b.text(), "see", "the paste, whole");
        assert!(r.undo());
        assert_eq!(r.b.text(), "");
    }

    #[test]
    fn a_move_ends_the_step_even_when_the_next_word_lands_where_the_last_ended() {
        let mut r = Rig::new("");
        r.typed("abc");
        r.moved(Buffer::left);
        r.moved(Buffer::right);
        r.typed("def");
        assert!(r.undo());
        assert_eq!(r.b.text(), "abc");
    }

    #[test]
    fn the_caret_goes_back_to_where_it_was_before_and_after() {
        let mut r = Rig::new("one\ntwo");
        r.moved(|b| b.set_caret(0, 1));
        r.typed("XY");
        assert_eq!(r.b.caret(), (0, 3));
        r.moved(|b| b.set_caret(1, 3));

        assert!(r.undo());
        assert_eq!(r.b.text(), "one\ntwo");
        assert_eq!(r.b.caret(), (0, 1), "before the step, not where the caret wandered to");
        assert!(r.redo());
        assert_eq!(r.b.text(), "oXYne\ntwo");
        assert_eq!(r.b.caret(), (0, 3), "after the step's last edit");
    }

    #[test]
    fn an_edit_after_an_undo_forgets_the_redo_it_had() {
        let mut r = Rig::new("");
        r.typed("one two");
        assert!(r.undo());
        r.typed("three");
        assert_eq!(r.b.text(), "one three");
        assert!(!r.redo(), "the future it described started from another text");
        assert_eq!(r.h.held, "one ".len() + "three".len());
    }

    #[test]
    fn undo_and_redo_are_inverse_however_often_they_alternate() {
        let mut r = Rig::new("start\n");
        r.typed("alpha beta\ngamma");
        r.edit(Kind::Backspace, Buffer::backspace);
        let end = r.b.text();
        for _ in 0..5 {
            while r.undo() {}
            assert_eq!(r.b.text(), "start\n");
            while r.redo() {}
            assert_eq!(r.b.text(), end);
        }
    }

    // --- the saved point ---------------------------------------------------

    #[test]
    fn typing_after_a_save_is_a_step_of_its_own_and_undo_stops_at_the_save() {
        // The bug this section exists for: the word typed after a save joined
        // the step the save pointed at, so the history reported nothing unsaved
        // and one undo took back text that had been on disk.
        let mut r = Rig::new("");
        r.typed("abc");
        r.h.mark_saved();
        assert!(!r.h.is_modified());
        r.typed("def");
        assert!(r.h.is_modified());
        assert!(r.undo());
        assert_eq!(r.b.text(), "abc");
        assert!(!r.h.is_modified(), "back at the saved text, and it says so");
        assert!(r.redo());
        assert!(r.h.is_modified());
    }

    #[test]
    fn the_saved_point_is_a_position_and_survives_undoing_past_it() {
        let mut r = Rig::new("");
        assert!(!r.h.is_modified(), "a text nobody touched is as it arrived");
        r.typed("one two three");
        r.h.mark_saved();
        assert!(r.undo());
        assert!(r.undo());
        assert!(r.h.is_modified());
        assert!(r.redo());
        assert!(r.redo());
        assert!(!r.h.is_modified(), "redone back to what was saved");

        // Typing a character and rubbing it out is two steps on, so it is
        // modified, which is the strict direction the module doc names.
        r.typed("x");
        r.edit(Kind::Backspace, Buffer::backspace);
        assert_eq!(r.b.text(), "one two three");
        assert!(r.h.is_modified());

        // An edit after undoing past the save forgets the way back to it.
        let mut r = Rig::new("");
        r.typed("one two");
        r.h.mark_saved();
        assert!(r.undo());
        r.typed("three");
        assert!(r.h.is_modified());
        assert!(!r.redo());
        assert!(r.undo());
        assert_eq!(r.b.text(), "one ");
        assert!(r.h.is_modified(), "this text was never saved");
    }

    #[test]
    fn a_saved_step_that_fell_off_the_bottom_is_still_the_floor_it_left() {
        let mut r = Rig::new("");
        r.h = History::bounded(2, usize::MAX);
        r.typed("one ");
        r.h.mark_saved();
        r.typed("two three ");
        assert_eq!(r.h.undo.len(), 2, "the saved step is gone from the stack");
        while r.undo() {}
        assert_eq!(r.b.text(), "one ");
        assert!(!r.h.is_modified(), "the bottom of the stack is the saved text");
    }

    // --- the property -----------------------------------------------------

    #[test]
    fn undoing_everything_gives_back_the_original_and_redoing_it_the_final_text() {
        // A random walk over every edit and every move there is, from a
        // document with accents, ideographs and blank lines in it. Before each
        // edit that happens, the text and the caret are written down; then
        // every undo must land on one of those states — the text *and* the
        // caret — strictly earlier than the last one it landed on, the last
        // undo on the original, and redo must walk back up through the same
        // states to the text the walk ended with.
        let start = "one\n\nthrée\n日本\nfour";
        let mut r = Rig::new(start);
        r.h = History::bounded(usize::MAX, usize::MAX);
        let mut seen: Vec<(String, Pos)> = Vec::new();
        // The state just after each edit that happened, for the redo half:
        // redo must land on one of these, caret and all.
        let mut reached: Vec<(String, Pos)> = Vec::new();
        let mut seed = 0xD1B5_4A32_D192_ED03u64;
        for _ in 0..2500 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let pick = (seed >> 33) % 13;
            let state = (r.b.text(), r.b.caret());
            let did = match pick {
                0..=3 => {
                    let c = ['a', 'é', ' ', '.'][(seed >> 40) as usize % 4];
                    r.edit(Kind::Typing, |b| b.insert(c))
                }
                4 => r.edit(Kind::Typing, Buffer::newline),
                5 => r.edit(Kind::Typing, |b| b.insert_str("  ")),
                6 => r.edit(Kind::Backspace, Buffer::backspace),
                7 => r.edit(Kind::Delete, Buffer::delete),
                8 => r.edit(Kind::Paste, |b| b.insert_str("pasted\r\nlines")),
                9 => {
                    r.moved(Buffer::left);
                    false
                }
                10 => {
                    r.moved(Buffer::down);
                    false
                }
                11 => {
                    let (row, col) = ((seed >> 44) as usize % 9, (seed >> 52) as usize % 9);
                    r.moved(|b| b.set_caret(row, col));
                    false
                }
                // Undo and redo in the middle of the walk, so the stacks are
                // exercised while they are being built and not only after.
                _ => {
                    if (seed >> 45) & 1 == 0 {
                        r.undo();
                    } else {
                        r.redo();
                    }
                    // A state reached by undo is a state like any other: the
                    // next edit's "before" is recorded from here.
                    false
                }
            };
            if did {
                seen.push(state);
                reached.push((r.b.text(), r.b.caret()));
            }
        }
        let end = r.b.text();
        // Undo in the middle of the walk may have left redo steps that the next
        // edit forgot; what the walk ended on is the reference point now, so
        // the states to land on are the ones still reachable from it. Re-walk
        // them by undoing and checking each against the record.
        let mut landed: Vec<(String, Pos)> = Vec::new();
        let mut floor = seen.len();
        while r.undo() {
            let now = (r.b.text(), r.b.caret());
            let found = seen[..floor].iter().rposition(|s| *s == now);
            let Some(at) = found else {
                panic!("undo landed on {now:?}, which the walk was never in");
            };
            floor = at;
            landed.push(now);
        }
        assert_eq!(r.b.text(), start, "undoing everything is not the original");
        assert!(!landed.is_empty(), "the walk made no steps at all");

        let mut ceiling = 0;
        for want in landed.iter().rev().skip(1) {
            assert!(r.redo());
            assert_eq!(r.b.text(), want.0, "redo did not retrace undo");
            let now = (r.b.text(), r.b.caret());
            let Some(at) = reached[ceiling..].iter().position(|s| *s == now) else {
                panic!("redo landed on {now:?}, which no edit ever left behind");
            };
            ceiling += at;
        }
        assert!(r.redo());
        assert_eq!(r.b.text(), end, "redoing everything is not the final text");
        assert!(
            reached[ceiling..].contains(&(r.b.text(), r.b.caret())),
            "the last redo put the caret somewhere no edit left it"
        );
        assert!(!r.redo());
    }

    #[test]
    fn a_bounded_history_keeps_its_count_of_text_true_under_any_walk() {
        // The bound under sequences rather than one case: tight enough that
        // steps fall off every few presses, with undo and redo mixed in, and
        // after every press the count of text held must be the text the two
        // stacks hold and inside the budget — the newest step excepted.
        let mut r = Rig::new("seed text\n");
        r.h = History::bounded(7, 40);
        let mut seed = 0x2F69_3A8B_1C55_E0D7u64;
        for _ in 0..3000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            match (seed >> 33) % 8 {
                0 | 1 => {
                    let c = ['w', ' ', '\n'][(seed >> 41) as usize % 3];
                    r.edit(Kind::Typing, |b| b.insert(c));
                }
                2 => {
                    r.edit(Kind::Backspace, Buffer::backspace);
                }
                3 => {
                    r.edit(Kind::Delete, Buffer::delete);
                }
                4 => {
                    r.edit(Kind::Paste, |b| b.insert_str("pasted words"));
                }
                5 => r.moved(Buffer::left),
                6 => {
                    r.undo();
                }
                _ => {
                    r.redo();
                }
            }
            let undo: usize = r.h.undo.iter().map(Step::bytes).sum();
            let redo: usize = r.h.redo.iter().map(Step::bytes).sum();
            assert_eq!(r.h.held, undo + redo, "the count of text held drifted");
            assert!(r.h.undo.len() <= 7, "{} steps past a bound of 7", r.h.undo.len());
            if r.h.undo.len() > 1 && r.h.redo.is_empty() {
                assert!(r.h.held <= 40, "held {} past a budget of 40", r.h.held);
            }
        }
    }

    // --- the bound ----------------------------------------------------------

    #[test]
    fn the_oldest_steps_go_first_once_there_are_too_many() {
        let mut r = Rig::new("");
        r.h = History::bounded(3, usize::MAX);
        r.typed("one two three four five");
        assert_eq!(r.h.undo.len(), 3);
        while r.undo() {}
        assert_eq!(r.b.text(), "one two ", "the three newest went back, and no more");
    }

    #[test]
    fn the_text_held_stays_inside_its_budget_and_the_newest_step_is_always_kept() {
        let mut r = Rig::new("");
        r.h = History::bounded(usize::MAX, 64);
        for _ in 0..20 {
            r.edit(Kind::Paste, |b| b.insert_str("0123456789"));
        }
        assert!(r.h.held <= 64, "held {} past a budget of 64", r.h.held);
        assert_eq!(r.h.held, r.h.undo.iter().map(Step::bytes).sum::<usize>());

        // A single step bigger than the whole budget is still kept: an undo
        // that silently could not take back the paste just made would be worse
        // than a history briefly over its bound.
        r.edit(Kind::Paste, |b| b.insert_str(&"x".repeat(200)));
        assert_eq!(r.h.undo.len(), 1);
        assert!(r.undo());
        assert!(!r.b.text().contains('x'));
    }

    #[test]
    fn the_real_bound_is_a_thousand_steps_and_four_documents() {
        let h = History::new(NOTE.max_bytes);
        assert_eq!(h.steps, STEPS);
        assert_eq!(h.bytes, 4 * NOTE.max_bytes);
    }
}
