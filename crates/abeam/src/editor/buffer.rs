//! The text being edited, and the caret in it.
//!
//! This was the first place in abeam where a caret lived *inside* text. The
//! queue's composer and the ask's composer are both append-only — `push` and
//! `pop` are their whole vocabulary — so neither has ever had to answer where
//! the insertion point is, and neither would survive being asked. A pad is a
//! document you go back into and change the middle of, so it has to, and a
//! file opened for editing is the same problem with a larger cap. It was the
//! pad's own module until the files view needed one too; what is policy rather
//! than mechanism — how much it may hold, what a tab becomes, and what a line
//! ending and a byte order mark will cost once the text is on disk — now
//! arrives as a [`Policy`] instead of being written in here.
//!
//! It is a `Vec<String>`, one entry per line, rather than one `String` with an
//! offset into it or a rope. A rope buys nothing at these sizes — the largest
//! document anything hands this is the files view's 512 KiB — and a single
//! `String` would put the line the caret is on at the far end of a scan, so
//! every keystroke would count newlines from the start of the document to work
//! out which row had changed. Lines are also the shape both consumers already
//! want: the highlighter takes and returns one line at a time, the layout
//! beside this keeps one entry per line and finds what an edit changed by
//! comparing two lists of them, and a caret is drawn at a row and a column.
//!
//! ## One writer
//!
//! Every change to the text goes through [`Buffer::splice`], and that is the
//! one rule in this file that something else depends on. Undo needs to know,
//! for every edit, what was there and what replaced it; a buffer with six
//! mutators each writing `lines` its own way would need six correct accounts
//! of that, and the first one to get it wrong would make `Ctrl+Z` put back a
//! document that never existed. With one writer there is one account, taken
//! where the write happens, and an edit method that forgot to report itself
//! is not a thing that can be written: it would have to bypass the only
//! function that can change the text. [`Buffer::take_change`] is where the
//! history collects it.
//!
//! There is still no selection and no word motion, and that is still a
//! decision rather than a stopping point. The pad is for a sentence you had
//! while the agent was busy and the files view for a fix too small to be worth
//! the editor in the window next door; both features are a second keymap to
//! learn and a second state to draw, spent on becoming that editor. Undo was
//! the exception, and it was made once editing reached files: a slip in a
//! scratch note costs a retyped word, a slip in a source file that is then
//! saved costs a diff somebody has to read.
//!
//! ## What a step is
//!
//! `left`, `right` and `backspace` move by `char` — a Unicode scalar value —
//! and not by grapheme cluster, and on `a` followed by U+0301 the difference is
//! visible. `Right` crosses that one letter in two presses, and the first of
//! them moves the index without moving anything the reader can see: a frame
//! spent re-rendering the agent's whole screen for a cursor that appeared to
//! stay where it was, which is exactly the cost [`crate::pane::Handled`] warns
//! about. `Backspace` there is worse than useless — it takes the `a` and leaves
//! the combining acute to settle onto whatever is now in front of it.
//!
//! It is still the trade to take here. Stepping by cluster means a
//! segmentation crate, and this codebase argues about every dependency it takes
//! in the manifest that names them; combining marks are rare in the prose
//! people type into a note, and the damage is one keystroke that looks odd
//! rather than a panic or a lost document. Changing it later is a dependency,
//! those three methods, and one decision that reaches further than they do:
//! [`Buffer::caret`] would then report a column counted in clusters, and the
//! layout places the cursor by measuring a prefix of that many *characters*.

use super::{Policy, TabKey, Tabs};

/// A place in the text, as `(row, col)` with `col` counted in `char`s — the
/// same pair [`Buffer::caret`] answers with.
pub type Pos = (usize, usize);

/// One change to the text, in the terms undo needs to take it back: where it
/// began, what was there, and what is there now.
///
/// Both strings are text as the buffer holds it — LF between lines, already
/// cleaned — so putting either back is a [`Buffer::replace`] and never another
/// trip through [`clean`], which could only make it something it was not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub at: Pos,
    pub removed: String,
    pub inserted: String,
}

/// Some text, with somewhere in it to type.
pub struct Buffer {
    /// What the caller decided this text may hold. Fixed for the buffer's
    /// life: a cap that moved under a document would make states the history
    /// can return to into states the buffer would refuse.
    policy: Policy,
    /// One entry per line, and **never empty**: a buffer with nothing in it is
    /// one empty line.
    ///
    /// Every method here may assume that, and every method must leave it true.
    /// It is what lets [`Buffer::caret`] hand back a `(row, col)` that indexes
    /// instead of an `Option`, and an `Option` is the only other answer: there
    /// is no honest row number in a buffer with no rows, and no caller with
    /// anywhere to put a `None`, because the pane draws a cursor at a cell.
    ///
    /// No line holds a `\n` or a `\r` either, which is the same invariant seen
    /// from the other side — a line that held one would make [`Buffer::lines`]
    /// and [`Buffer::text`] disagree about how many lines there are, and would
    /// draw as a control picture or return the cursor to the left margin and
    /// overwrite the row. Everything entering the buffer goes through
    /// [`Buffer::insert`] or [`clean`], and both take them out.
    lines: Vec<String>,
    /// Which line the caret is on. Always less than `lines.len()`.
    row: usize,
    /// How far along that line, counted in `char`s. Always no greater than the
    /// number of `char`s in the line, and equal to it when the caret is at the
    /// end.
    ///
    /// Characters rather than bytes, everywhere above [`byte_at`]. A column
    /// that was a byte offset would be handed straight to `String::remove`, and
    /// the first `é` anybody backspaced over would panic with `byte index 2 is
    /// not a char boundary` — in the draw path, taking the whole program with
    /// it. Display width is a third thing again and is not this module's
    /// problem: the layout measures cells, and what it measures is the prefix
    /// this column names.
    col: usize,
    /// The column [`Buffer::up`] and [`Buffer::down`] are trying to get back
    /// to.
    ///
    /// The only state here beyond the caret itself, and it earns its place
    /// within about four keystrokes. Without it, a vertical move clamps the
    /// column to whatever the row it lands on can hold and the clamp is
    /// permanent: crossing one two-character line on the way down a document
    /// leaves the caret in column two for every row after it, and the reader
    /// has to steer back by hand each time. With it the short line is passed
    /// over rather than fallen into.
    ///
    /// Every sideways key sets it back to the real column, because that is the
    /// moment the user has said where they want to be — and it does so whether
    /// or not the caret actually moved. `Home` on a row the caret is already at
    /// the start of moves nothing and still names column zero; a version that
    /// returned early before touching this left the next `down` travelling to a
    /// column the user had just pressed a key to leave. An edit sets it back as
    /// well, but only when the edit happened, because an edit that was refused
    /// moved nothing and named nothing.
    ///
    /// Only `up` and `down` read it, and neither of them writes it.
    desired: usize,
    /// This text arrived larger than the policy's cap and what is here is the
    /// front of it. Set by [`Buffer::from_text`], never cleared, and read
    /// through [`Buffer::truncated`], which carries the argument.
    truncated: bool,
    /// What the last edit did, waiting for [`Buffer::take_change`]. See the
    /// module doc's *One writer*.
    last: Option<Change>,
}

impl Buffer {
    /// An empty buffer, which is one empty line with the caret at the start of
    /// it.
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            lines: vec![String::new()],
            row: 0,
            col: 0,
            desired: 0,
            truncated: false,
            last: None,
        }
    }

    /// A buffer loaded from what was saved, with the caret at the start of it.
    ///
    /// The start because that is the only place every caller can agree on
    /// before it has said anything: where the caret should *go* is a question
    /// about what the text is for. The pad moves it to the end, because the
    /// note you are about to add goes after the ones already made; the files
    /// view moves it to where the reader was looking. Both say so with
    /// [`Buffer::set_caret`], which clamps, so "the end" is `usize::MAX`.
    ///
    /// The text is cleaned on the way in for the reason [`clean`] gives. That
    /// makes `from_text` lossy for a CRLF file, which comes back with LF
    /// endings, and the alternative was to hold a `\r` that the pane cannot
    /// draw and the caret cannot count. A caller that has to give the file
    /// back with the endings it had records them itself; that is not a
    /// question about where a caret is.
    ///
    /// Capped like every other way in — in the bytes the text will have on
    /// disk, see [`Buffer::bytes`] — and the *only* one where the overflow is
    /// cut rather than refused, because a constructor has no way to say no.
    /// Cleaning is what makes that necessary rather than tidy: a pad file of
    /// tabs is inside the cap on disk and twice the size the moment every tab
    /// has become two spaces, so abeam would load 64 KiB, hold 128 KiB, and
    /// save that back. The session after it found a file it could not take
    /// whole, and the pad was read-only from then on — made too big by abeam
    /// itself, with nothing in the loop that could have noticed. The cut lands
    /// on a character boundary; through one it would be a panic on load, with
    /// the file already written.
    ///
    /// [`Buffer::truncated`] is how the pane finds out, and it has to ask,
    /// because a cut nobody was told about is a save that puts the front of a
    /// document over the whole of it.
    pub fn from_text(policy: Policy, text: &str) -> Self {
        let mut text = clean(text, policy);
        let end = fit(&text, policy);
        let truncated = end < text.len();
        text.truncate(end);
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        Self {
            policy,
            lines,
            row: 0,
            col: 0,
            desired: 0,
            truncated,
            last: None,
        }
    }

    /// The whole text as one string, which is what gets saved.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The lines, for drawing. Never empty.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The policy this buffer was made with.
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Whether there is anything here worth keeping.
    ///
    /// One empty line is empty; two empty lines are not, because somebody
    /// pressed a key to make the second one and a pad that quietly discarded
    /// that would be deciding what counts as a document.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Whether one more character would be refused.
    ///
    /// The pane draws a notice off this, and the notice is the whole point of
    /// the method. Refusing a paste whole is only defensible if the user is
    /// told: the argument on [`Policy::max_bytes`] is that they must never be
    /// handed half a sentence they have to notice, and a `Ctrl+V` that does
    /// nothing and says nothing fails that same test — a full pad and a broken
    /// one look identical from the outside.
    ///
    /// It reports the buffer's state rather than the last refusal, and those
    /// are not quite the same question: a paste can be turned away with room to
    /// spare, when what is left is smaller than what arrived. That case is
    /// still tellable apart, because the only other way [`Buffer::insert_str`]
    /// returns `false` is an empty argument and the caller knows what it
    /// passed — but this is the cheap signal rather than the complete one, and
    /// it is the one that can be read *before* a paste as well as after.
    pub fn is_full(&self) -> bool {
        self.bytes() >= self.policy.max_bytes
    }

    /// Whether this text is the front of a larger one.
    ///
    /// True when [`Buffer::from_text`] was handed more than the cap and kept
    /// the beginning of it. The pane must read this and refuse to save,
    /// because the alternative is the quietest data loss in the program: the
    /// tail the user cannot see is written away by the first flush after they
    /// type a character, and nothing on screen was ever different.
    ///
    /// It never goes back to false, and that is the whole of its value. An edit
    /// that makes room does not bring the tail back with it, so a flag that
    /// cleared itself on the first `backspace` would hand the save path
    /// permission to overwrite exactly the document it was there to protect.
    /// Undo does not clear it either, for the same reason: the oldest state
    /// the history can return to is the truncated one.
    ///
    /// **A `dead_code` waiver used to stand here and it was never true**, which
    /// is worth a sentence because it is a sharper lesson than a waiver going
    /// stale over time. It said the pane latched `store::Loaded::truncated` and
    /// "not yet this one", and that the two would be ORed together in a later
    /// wiring pass — and `PadPane::ensure_read` ORs them in the *same commit*
    /// that wrote the waiver. The pass it was waiting for had already happened
    /// when the words were typed. Nothing ever said so, because an `#[allow]`
    /// says nothing when the lint stops firing; an `#[expect]` would have
    /// failed the build that introduced it. The crate root has the rule that
    /// came out of this.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Where the caret is, as `(row, col)` with `col` counted in `char`s.
    ///
    /// Always inside the text — see the invariants on `lines` and `col` — so a
    /// caller may index with it.
    pub fn caret(&self) -> Pos {
        (self.row, self.col)
    }

    /// Type one character. Returns whether anything changed.
    ///
    /// Tabs and line endings are dealt with here rather than at the key
    /// handler, so that there is one answer for every caller: the pad, the
    /// files view, a paste, and whatever the next thing to type into one turns
    /// out to be. A `\n` that was written into a line instead of splitting it
    /// would break the invariant on `lines` quietly, and the symptom — a row
    /// that renders as two and a caret column measured against the wrong
    /// half — would not point back here.
    ///
    /// A tab is the policy's: kept as a literal `\t` under [`Tabs::Keep`], and
    /// otherwise replaced by spaces the way a pasted one is. This is the
    /// character arriving, not the `Tab` key; the key is
    /// [`super::Editor::tab`], which types [`Buffer::tab_text`] under either.
    pub fn insert(&mut self, c: char) -> bool {
        match c {
            '\t' if matches!(self.policy.tabs, Tabs::Spaces(_)) => {
                return self.insert_str(&self.tab_text());
            }
            '\n' | '\r' => return self.newline(),
            _ => {}
        }
        if self.bytes() + c.len_utf8() > self.policy.max_bytes {
            return false;
        }
        let at = self.caret();
        self.edit(at, at, c.encode_utf8(&mut [0; 4]))
    }

    /// Paste. Returns whether anything changed.
    ///
    /// The text is cleaned first and then split, so a clipboard carrying `\r\n`
    /// — which is every clipboard on the machine this was written on — becomes
    /// the lines it looks like rather than one line with holes in it. The caret
    /// finishes after the last character that went in, which is where the next
    /// thing typed belongs.
    ///
    /// Refused whole when it will not fit, for the reason on
    /// [`Policy::max_bytes`].
    pub fn insert_str(&mut self, s: &str) -> bool {
        let text = clean(s, self.policy);
        if text.is_empty() {
            return false;
        }
        if self.bytes() + self.on_disk(&text) > self.policy.max_bytes {
            return false;
        }
        let at = self.caret();
        self.edit(at, at, &text)
    }

    /// Replace the whole text with `s`, as one edit: `None` when it would not
    /// fit, `Some(false)` when it is the text already here, `Some(true)` when
    /// it went in.
    ///
    /// One splice from the start of the text to its end, so the change undo
    /// records is the whole of what was there and the whole of what replaced
    /// it, and one `Ctrl+Z` puts the old text back. Cleaned like a paste and
    /// refused whole like one, against the cap counted on disk. The caret stays
    /// where it was, clamped into the new text, because the commonest caller is
    /// a document coming back from disk with a line changed somewhere else and
    /// the reader still looking where they were.
    pub fn replace_all(&mut self, s: &str) -> Option<bool> {
        let text = clean(s, self.policy);
        if self.policy.bom_bytes + self.on_disk(&text) > self.policy.max_bytes {
            return None;
        }
        if text == self.text() {
            return Some(false);
        }
        let (row, col) = self.caret();
        let last = self.lines.len() - 1;
        let end = (last, self.lines[last].chars().count());
        self.edit((0, 0), end, &text);
        // Back where it was rather than at the end of what went in, and not
        // reported as a move: the change already says where the caret is.
        let row = row.min(self.lines.len() - 1);
        self.row = row;
        self.col = col.min(self.lines[row].chars().count());
        self.desired = self.col;
        Some(true)
    }

    /// Break the line at the caret. Returns whether anything changed.
    ///
    /// Capped like every other way in, which the API sketch did not ask for
    /// and which the cap is worthless without: a limit that only the paste path
    /// honours is one that a held-down `Enter` walks straight past, and the
    /// result is a saved pad the highlighter has already given up on.
    pub fn newline(&mut self) -> bool {
        if self.bytes() + self.policy.line_ending_bytes > self.policy.max_bytes {
            return false;
        }
        let at = self.caret();
        self.edit(at, at, "\n")
    }

    /// Delete backwards. At the start of a line this joins it to the one above,
    /// leaving the caret at the seam. Returns whether anything changed.
    pub fn backspace(&mut self) -> bool {
        if self.col > 0 {
            self.edit((self.row, self.col - 1), self.caret(), "")
        } else if self.row > 0 {
            let above = self.row - 1;
            let end = self.lines[above].chars().count();
            self.edit((above, end), self.caret(), "")
        } else {
            // The start of the first line, which is the one place there is
            // nothing behind the caret to take. The last line is never removed
            // here, which is half of why `lines` is never empty.
            false
        }
    }

    /// Delete forwards. At the end of a line this pulls the next one up without
    /// moving the caret. Returns whether anything changed.
    pub fn delete(&mut self) -> bool {
        if self.col < self.row_len() {
            self.edit(self.caret(), (self.row, self.col + 1), "")
        } else if self.row + 1 < self.lines.len() {
            self.edit(self.caret(), (self.row + 1, 0), "")
        } else {
            false
        }
    }

    /// One character left, over the line ending if there is one. Returns
    /// whether anything changed.
    pub fn left(&mut self) -> bool {
        let moved = if self.col > 0 {
            self.col -= 1;
            true
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.row_len();
            true
        } else {
            // The start of the document, where there is nowhere to go and the
            // key still names a column. See `desired`.
            false
        };
        self.desired = self.col;
        moved
    }

    /// One character right, over the line ending if there is one. Returns
    /// whether anything changed.
    pub fn right(&mut self) -> bool {
        let moved = if self.col < self.row_len() {
            self.col += 1;
            true
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
            true
        } else {
            false
        };
        self.desired = self.col;
        moved
    }

    /// One row up, keeping the column asked for rather than the column
    /// available. Returns whether anything changed.
    ///
    /// On the top row this reports no rather than sliding to the start of the
    /// document. `Home` is the key for that, and a vertical key that sometimes
    /// moves horizontally is one the reader cannot predict from where they are
    /// looking.
    pub fn up(&mut self) -> bool {
        if self.row == 0 {
            return false;
        }
        self.row -= 1;
        self.col = self.desired.min(self.row_len());
        true
    }

    /// One row down, keeping the column asked for. Returns whether anything
    /// changed.
    pub fn down(&mut self) -> bool {
        if self.row + 1 >= self.lines.len() {
            return false;
        }
        self.row += 1;
        self.col = self.desired.min(self.row_len());
        true
    }

    /// The start of the row. Returns whether anything changed.
    ///
    /// The wish is given up even when the caret was already there, which is the
    /// one thing about this method worth reading twice: `Home` on a row the
    /// caret is at the start of is still the user saying column zero, and an
    /// early return that skipped the assignment left the next `down` walking
    /// back out to a column they had just pressed a key to leave. `end`,
    /// `left`, `right` and [`Buffer::set_caret`] are the same shape for the
    /// same reason — see `desired`, where the rule is written down once.
    pub fn home(&mut self) -> bool {
        let moved = self.col != 0;
        self.col = 0;
        self.desired = 0;
        moved
    }

    /// The end of the row. Returns whether anything changed.
    pub fn end(&mut self) -> bool {
        let end = self.row_len();
        let moved = self.col != end;
        self.col = end;
        self.desired = end;
        moved
    }

    /// Put the caret at `row` and `col`, clamped into the text. Returns whether
    /// it moved.
    ///
    /// This is what a mouse click arrives as, and the clamping is the method.
    /// The pane knows where the pointer was and the buffer knows where the text
    /// ends, and neither knows the other half: a click in the empty space to
    /// the right of a short line is the commonest press there is — the pointer
    /// lands at column forty on a row holding `x`, because every other row in
    /// the document is eighty wide — and it has to mean the end of that line
    /// rather than an index into the row below or a panic inside [`byte_at`].
    /// A click below the last line means the last line, for the same reason.
    ///
    /// The sticky column is given up, as it is for every other horizontal move,
    /// and it is given up whether or not the caret moved: a click that landed
    /// where the caret already was is still the user naming that column, and a
    /// wish left over from an earlier `down` would then steer the next one
    /// somewhere they had just pointed away from.
    pub fn set_caret(&mut self, row: usize, col: usize) -> bool {
        // `lines` is never empty, so there is always a row to clamp to.
        let row = row.min(self.lines.len() - 1);
        let col = col.min(self.lines[row].chars().count());
        let moved = (row, col) != (self.row, self.col);
        self.row = row;
        self.col = col;
        self.desired = col;
        moved
    }

    /// What the last edit did, once. `None` when nothing has been edited since
    /// the last time this was asked, which a move or a refused edit leaves it
    /// as.
    pub fn take_change(&mut self) -> Option<Change> {
        self.last.take()
    }

    /// Replace the text between `from` and `to` with `with`, verbatim, for
    /// the history putting a step back or forward.
    ///
    /// Neither capped nor cleaned, and both on purpose. Everything `with` can
    /// be came out of this buffer through [`Buffer::splice`], so it is already
    /// clean, and cleaning it again could only change it — which for undo is
    /// the one thing it may not do. And every state undo or redo can reach is
    /// a state the buffer was in, so it is inside the cap the buffer was held
    /// to at the time; the policy never changes, so it still is. A cap
    /// consulted here could only ever refuse to put back what the user had.
    ///
    /// It is not reported to [`Buffer::take_change`]: the history is the
    /// caller, and it already knows what it did.
    pub fn replace(&mut self, from: Pos, to: Pos, with: &str) {
        debug_assert!(!with.contains('\r'), "history handed back text the buffer never held");
        self.splice(from, to, with);
    }

    /// The one place `lines` is written. See the module doc's *One writer*.
    ///
    /// `from` must not be after `to`, and both must be inside the text; every
    /// caller computes them from the caret or from a change this function
    /// produced, so both hold by construction. The caret finishes at the end of
    /// what went in, which is right for every edit there is: after a typed
    /// character or a paste, at the seam after a deletion either way, and at
    /// the start of the new line after `Enter`.
    ///
    /// The single-line case is its own branch because it is nearly every
    /// keystroke, and it is the only shape that can be done in place. The
    /// general one rebuilds the lines between `from` and `to` and splices them
    /// back, which is a move of the vector's tail — the same cost the
    /// `insert` and `remove` on `lines` it replaced already paid.
    fn splice(&mut self, from: Pos, to: Pos, with: &str) -> Change {
        debug_assert!(from <= to, "a splice that ends before it starts");
        let (fr, fc) = from;
        let (tr, tc) = to;
        let fb = byte_at(&self.lines[fr], fc);
        let tb = byte_at(&self.lines[tr], tc);

        let removed = if fr == tr {
            self.lines[fr][fb..tb].to_string()
        } else {
            let mut out = self.lines[fr][fb..].to_string();
            for line in &self.lines[fr + 1..tr] {
                out.push('\n');
                out.push_str(line);
            }
            out.push('\n');
            out.push_str(&self.lines[tr][..tb]);
            out
        };

        if fr == tr && !with.contains('\n') {
            self.lines[fr].replace_range(fb..tb, with);
        } else {
            let tail = self.lines[tr][tb..].to_string();
            let mut head = std::mem::take(&mut self.lines[fr]);
            head.truncate(fb);
            let mut pieces = with.split('\n');
            head.push_str(pieces.next().unwrap_or_default());
            let mut added = vec![head];
            added.extend(pieces.map(str::to_string));
            // `split` yields at least one piece, so `added` has a last line.
            if let Some(last) = added.last_mut() {
                last.push_str(&tail);
            }
            self.lines.splice(fr..=tr, added);
        }

        let (row, col) = end_of(from, with);
        self.row = row;
        self.col = col;
        self.desired = col;
        Change {
            at: from,
            removed,
            inserted: with.to_string(),
        }
    }

    /// An edit that is going to happen: the splice, and the record of it.
    /// Always true, so the mutators above can return it as their answer.
    fn edit(&mut self, from: Pos, to: Pos, with: &str) -> bool {
        let change = self.splice(from, to, with);
        self.last = Some(change);
        true
    }

    /// How many `char`s are on the row the caret is on.
    fn row_len(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    /// What the text will take on disk, in bytes, without building it: what
    /// the cap is counted in.
    ///
    /// On disk rather than in memory, because the cap is a promise about the
    /// file. The buffer holds `\n` between lines whatever the file had, so a
    /// CRLF file counted in memory is a line ending's worth of bytes short on
    /// every line — and a file edited up to a 512 KiB cap counted that way
    /// would be saved larger than the reader's 512 KiB will open again. So the
    /// policy says what a line ending and a byte order mark will cost, and the
    /// count is of the bytes the save will write. For the pad, whose file is
    /// LF with no mark, the two counts are the same number.
    ///
    /// Recomputed on every edit rather than carried as a field. A cached length
    /// is a second copy of the truth maintained by every writer, and the first
    /// one to forget it leaves a buffer that refuses a paste it has room for,
    /// or takes one it does not — a bug that only shows up at the size where
    /// nobody is looking. At the files view's 512 KiB this walks some thousands
    /// of short strings, which is microseconds beside the frame the same
    /// keystroke is about to pay for.
    ///
    /// The `- 1` is safe because `lines` is never empty, and the joining
    /// line endings are counted because the save writes them.
    pub fn bytes(&self) -> usize {
        self.policy.bom_bytes
            + self.lines.iter().map(String::len).sum::<usize>()
            + (self.lines.len() - 1) * self.policy.line_ending_bytes
    }

    /// The length of [`Buffer::text`]: the text as it is held, `\n` between
    /// lines and no mark. What the highlighter's own cap is measured in, which
    /// is the reader's unit and not the file's.
    pub fn text_bytes(&self) -> usize {
        self.lines.iter().map(String::len).sum::<usize>() + self.lines.len() - 1
    }

    /// What `s`, already clean, adds to [`Buffer::bytes`] when it goes in.
    fn on_disk(&self, s: &str) -> usize {
        let breaks = s.bytes().filter(|&b| b == b'\n').count();
        s.len() + breaks * (self.policy.line_ending_bytes.saturating_sub(1))
    }

    /// What the `Tab` key types, and what a tab arriving under
    /// [`Tabs::Spaces`] becomes.
    pub fn tab_text(&self) -> String {
        match self.policy.tabs {
            Tabs::Spaces(n) | Tabs::Keep { key: TabKey::Spaces(n) } => " ".repeat(n),
            Tabs::Keep { key: TabKey::Tab } => "\t".to_string(),
        }
    }
}

/// How much of `text`, already clean, fits the policy's cap on disk: the byte
/// length of the longest prefix that does, which is all of it when it fits.
///
/// Walked a character at a time, because a line ending's cost on disk is the
/// policy's and not one byte, and because the cut must land on a character
/// boundary — through one it would be a panic on load, with the file already
/// written.
fn fit(text: &str, policy: Policy) -> usize {
    let budget = policy.max_bytes.saturating_sub(policy.bom_bytes);
    let mut used = 0usize;
    for (at, ch) in text.char_indices() {
        let cost = if ch == '\n' {
            policy.line_ending_bytes
        } else {
            ch.len_utf8()
        };
        if used + cost > budget {
            return at;
        }
        used += cost;
    }
    text.len()
}

/// Where the caret finishes after `text` is written at `at`: the end of it.
///
/// Shared with the history, which has to work out the far end of a change it
/// is putting back and must get the answer the buffer would. Two copies of
/// this arithmetic would be two places for a step that crosses a line ending
/// to land one character out.
pub fn end_of(at: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => (at.0, at.1 + text.chars().count()),
        Some(i) => (
            at.0 + text.bytes().filter(|&b| b == b'\n').count(),
            text[i + 1..].chars().count(),
        ),
    }
}

/// The byte offset of character `col` in `line`, or the line's length when
/// `col` is past the last one.
///
/// The single place a column becomes an index, and it is one place on purpose.
/// `String::insert` and `String::remove` take byte offsets and panic on
/// anything that is not a character boundary, so every one of them in this file
/// is fed from here; a column arithmetic'd into an index anywhere else would
/// work on the ASCII everybody tests with and take the program down on the
/// first accented word somebody actually wrote.
fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map_or(line.len(), |(byte, _)| byte)
}

/// Text with the things a line may not contain taken out of it.
///
/// Line endings first. A Windows clipboard hands over `\r\n` and an old Mac
/// file hands over a lone `\r`; either one left inside a line is a character
/// the terminal cannot draw, so it comes out as a control picture or returns
/// the cursor to the left margin and lets the rest of the row overwrite what
/// was already there. It would also make the buffer's own two accounts of
/// itself disagree, since [`Buffer::lines`] would report one line where
/// [`Buffer::text`] round-tripped through a file gives two.
///
/// Tabs second, and only under [`Tabs::Spaces`]: then each one becomes those
/// spaces, the same text the `Tab` key types. That is the pad's rule, and its
/// reason was that a caret measured in characters could not stand next to a
/// tab drawn four cells wide. The layout now measures cells and draws a tab at
/// its stop, so the reason is gone and the rule is policy: the pad keeps it
/// because two spaces is a nesting level in the markdown it holds, and a file
/// keeps its tabs because a save has to give back the bytes it was given.
fn clean(text: &str, policy: Policy) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\t' => match policy.tabs {
                Tabs::Spaces(n) => out.extend(std::iter::repeat_n(' ', n)),
                Tabs::Keep { .. } => out.push('\t'),
            },
            _ => out.push(ch),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The pad's policy, written out here rather than borrowed from the pad:
    /// the editor is below the pad and must not reach up into it, and these
    /// tests were written against these numbers when the buffer was the pad's.
    const NOTE: Policy = Policy {
        max_bytes: 64 * 1024,
        tabs: Tabs::Spaces(2),
        line_ending_bytes: 1,
        bom_bytes: 0,
    };

    /// The cap under [`NOTE`], by the name these tests have always used.
    const MAX_BYTES: usize = NOTE.max_bytes;

    /// What a file opened for editing is held to: a larger cap, and its tabs.
    const FILE: Policy = Policy {
        max_bytes: 512 * 1024,
        tabs: Tabs::Keep { key: TabKey::Tab },
        line_ending_bytes: 1,
        bom_bytes: 0,
    };

    /// `text` loaded and the caret moved to the end of it, which is what the
    /// pad does and what these tests were written against when `from_text`
    /// did it itself.
    fn loaded(policy: Policy, text: &str) -> Buffer {
        let mut b = Buffer::from_text(policy, text);
        b.set_caret(usize::MAX, usize::MAX);
        b
    }

    /// The lines as string slices, so a test can compare against an array
    /// literal and read like the screen it is describing.
    fn rows(b: &Buffer) -> Vec<&str> {
        b.lines().iter().map(String::as_str).collect()
    }

    /// One key of the walk below: the press, and what it promises to do to the
    /// sticky column.
    type Key = (Box<dyn Fn(&mut Buffer) -> bool>, Wish);

    /// What a key promises to do to the sticky column.
    ///
    /// The walk below asserts against this, and it exists because the walk did
    /// not use to: `desired` was the one piece of state nothing checked, and a
    /// `home` that skipped naming its column went through four thousand
    /// keystrokes without being noticed.
    #[derive(Clone, Copy)]
    enum Wish {
        /// `up` and `down`, which read the wish and never write it.
        Kept,
        /// Every sideways key, including the ones that could not move.
        Named,
        /// An edit, which names a column when it happens and names nothing when
        /// it is refused.
        NamedIfItHappened,
    }

    /// A buffer with the caret put where a test needs it, rather than five
    /// lines of walking there with the methods under test.
    fn at(text: &str, row: usize, col: usize) -> Buffer {
        let mut b = Buffer::from_text(NOTE, text);
        b.row = row;
        b.col = col;
        b.desired = col;
        b
    }

    // --- the invariant everything else rests on ---------------------------

    #[test]
    fn an_empty_buffer_is_one_empty_line_and_cannot_be_emptied_further() {
        let mut b = Buffer::new(NOTE);
        assert_eq!(rows(&b), [""]);
        assert_eq!(b.caret(), (0, 0));
        assert!(b.is_empty());

        // Everything the user can do to nothing at all.
        for _ in 0..5 {
            b.backspace();
            b.delete();
            b.left();
            b.up();
        }
        assert_eq!(rows(&b), [""], "the last line cannot be deleted away");

        // And after a document has been typed and then taken back again, which
        // is the path that actually reaches zero in use.
        b.insert_str("one\ntwo\nthree");
        for _ in 0..100 {
            b.backspace();
        }
        assert_eq!(rows(&b), [""]);
        assert_eq!(b.caret(), (0, 0));
        assert!(b.is_empty());
    }

    #[test]
    fn the_caret_stays_inside_the_text_whatever_order_the_keys_arrive_in() {
        // A kilobyte at a time, so that the walk actually reaches the cap and
        // spends the second half of itself there. A version of this that only
        // ever added a character or two could not exceed about sixteen
        // kilobytes in the iterations it runs, which meant every refusal path
        // in the file went untested under sequences — the states this module's
        // own constant defines were the states the walk could not get to.
        let paste = "lorem ipsum dolor sit amet\n".repeat(38);
        let keys: Vec<Key> = vec![
            (Box::new(Buffer::left), Wish::Named),
            (Box::new(Buffer::right), Wish::Named),
            (Box::new(Buffer::up), Wish::Kept),
            (Box::new(Buffer::down), Wish::Kept),
            (Box::new(Buffer::home), Wish::Named),
            (Box::new(Buffer::end), Wish::Named),
            (Box::new(Buffer::backspace), Wish::NamedIfItHappened),
            (Box::new(Buffer::delete), Wish::NamedIfItHappened),
            (Box::new(Buffer::newline), Wish::NamedIfItHappened),
            (
                Box::new(|b: &mut Buffer| b.insert('é')),
                Wish::NamedIfItHappened,
            ),
            (
                Box::new(|b: &mut Buffer| b.insert('\t')),
                Wish::NamedIfItHappened,
            ),
            (
                Box::new(|b: &mut Buffer| b.insert_str("x\r\ny")),
                Wish::NamedIfItHappened,
            ),
            (
                Box::new(move |b: &mut Buffer| b.insert_str(&paste)),
                Wish::NamedIfItHappened,
            ),
            // Clicks: two that usually land inside the document, and one that
            // is past both ends of any document there could ever be.
            (Box::new(|b: &mut Buffer| b.set_caret(2, 40)), Wish::Named),
            (Box::new(|b: &mut Buffer| b.set_caret(0, 0)), Wish::Named),
            (
                Box::new(|b: &mut Buffer| b.set_caret(usize::MAX, usize::MAX)),
                Wish::Named,
            ),
        ];

        let mut b = Buffer::from_text(NOTE, "one\n\nthrée\n日本\nfour");
        // A fixed sequence rather than a random one, so a failure here is a
        // failure anybody can reproduce from the file alone.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut ever_full = false;
        for step in 0..4000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let (press, wish) = &keys[(seed >> 33) as usize % keys.len()];
            let changed = press(&mut b);
            ever_full |= b.is_full();

            let (row, col) = b.caret();
            assert!(!b.lines().is_empty(), "the buffer lost its last line");
            assert!(row < b.lines().len(), "the caret left the document");
            assert!(
                col <= b.lines()[row].chars().count(),
                "the caret left its line"
            );
            assert!(b.bytes() <= MAX_BYTES, "the pad grew past its own cap");

            // The sticky column, which nothing here used to look at — and which
            // is why a `home` that forgot to name its column went unnoticed
            // through four thousand keystrokes.
            match wish {
                Wish::Kept => {}
                Wish::Named => assert_eq!(b.desired, col, "a sideways key left the wish behind"),
                Wish::NamedIfItHappened if changed => {
                    assert_eq!(b.desired, col, "an edit left the wish behind");
                }
                Wish::NamedIfItHappened => {}
            }

            // The expensive one, which walks the whole document. Often enough
            // to catch a bad line within a few keystrokes of it appearing, and
            // rarely enough that a full pad does not turn this into a
            // sixty-four-kilobyte scan four thousand times over.
            if step % 100 == 0 {
                assert!(
                    b.lines().iter().all(|l| !l.contains(['\n', '\r', '\t'])),
                    "a line took in something it cannot draw"
                );
            }
        }
        assert!(
            ever_full,
            "the walk never filled the pad, so it never tried a refusal"
        );
    }

    #[test]
    fn a_buffer_is_empty_only_while_there_is_nothing_in_it_to_lose() {
        let mut b = Buffer::new(NOTE);
        assert!(b.is_empty());
        assert!(b.newline());
        assert!(!b.is_empty(), "somebody pressed a key to make that line");
        assert!(b.backspace());
        assert!(b.is_empty());
        assert!(b.insert(' '));
        assert!(!b.is_empty(), "a space is text");
    }

    // --- one writer -------------------------------------------------------

    #[test]
    fn every_edit_leaves_a_change_that_puts_the_text_back_exactly() {
        // The property undo stands on, checked under the same kind of walk as
        // the caret: after every press that changed the text, the change it
        // left must be the whole truth about what happened — replacing what
        // went in with what came out has to give back the text from before,
        // character for character, including across line endings.
        let mut b = Buffer::from_text(NOTE, "one\n\nthrée\n日本\nfour");
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..3000 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let before = b.text();
            let changed = match (seed >> 33) % 10 {
                0 => b.insert('x'),
                1 => b.insert('é'),
                2 => b.newline(),
                3 => b.backspace(),
                4 => b.delete(),
                5 => b.insert_str("pa\r\nste\n"),
                6 => b.left(),
                7 => b.up(),
                8 => b.set_caret((seed >> 40) as usize % 8, (seed >> 50) as usize % 12),
                _ => b.insert('\t'),
            };
            let change = b.take_change();
            let moved_only = (seed >> 33) % 10 >= 6 && (seed >> 33) % 10 <= 8;
            if moved_only {
                assert_eq!(change, None, "a move reported an edit");
                continue;
            }
            assert_eq!(changed, change.is_some(), "an edit and its record disagree");
            let Some(change) = change else { continue };
            assert_eq!(
                b.caret(),
                end_of(change.at, &change.inserted),
                "the caret is not at the end of what went in"
            );

            let after = b.text();
            let end = end_of(change.at, &change.inserted);
            b.replace(change.at, end, &change.removed);
            assert_eq!(b.text(), before, "undoing {change:?} did not restore the text");
            let end = end_of(change.at, &change.removed);
            b.replace(change.at, end, &change.inserted);
            assert_eq!(b.text(), after, "redoing {change:?} did not restore the text");
            assert_eq!(b.take_change(), None, "the history's own writes were reported");
        }
    }

    #[test]
    fn the_far_end_of_a_change_is_counted_in_characters_across_line_endings() {
        assert_eq!(end_of((2, 3), ""), (2, 3));
        assert_eq!(end_of((2, 3), "héllo"), (2, 8));
        assert_eq!(end_of((2, 3), "a\n"), (3, 0));
        assert_eq!(end_of((2, 3), "a\nb\n日本"), (4, 2));
    }

    // --- what goes in comes out -------------------------------------------

    #[test]
    fn text_gives_back_exactly_what_from_text_was_given() {
        for source in [
            "",
            "one",
            "one\ntwo",
            "one\n\nthree\n",
            "\n",
            "héllo\n日本語",
        ] {
            assert_eq!(Buffer::from_text(NOTE, source).text(), source);
        }
        // The one input that comes back changed, and on purpose: a `\r` is not
        // something a line is allowed to hold.
        assert_eq!(Buffer::from_text(NOTE, "one\r\ntwo").text(), "one\ntwo");
    }

    #[test]
    fn from_text_leaves_the_caret_at_the_start_and_the_end_is_one_clamped_move_away() {
        // The start, because where the caret goes is the caller's: the pad
        // wants the end and the files view where the reader was. This test
        // used to pin the end, when the pad's choice was made in here.
        assert_eq!(Buffer::from_text(NOTE, "a note\nand another").caret(), (0, 0));
        assert_eq!(loaded(NOTE, "a note\nand another").caret(), (1, 11));
        assert_eq!(loaded(NOTE, "").caret(), (0, 0));
        assert_eq!(loaded(NOTE, "done\n").caret(), (1, 0));
    }

    // --- characters, not bytes --------------------------------------------

    #[test]
    fn a_column_counts_characters_so_an_accent_is_one_step_and_not_two() {
        let mut b = loaded(NOTE, "héllo");
        assert_eq!(b.caret(), (0, 5), "five characters, six bytes");

        assert!(b.left());
        assert!(b.left());
        assert!(b.left());
        assert_eq!(b.caret(), (0, 2), "the caret is just past the é");

        // The press that panics a buffer indexed by bytes.
        assert!(b.backspace());
        assert_eq!(b.text(), "hllo");
        assert_eq!(b.caret(), (0, 1));
    }

    #[test]
    fn a_cjk_line_can_be_edited_from_either_end() {
        let mut b = loaded(NOTE, "日本語");
        assert_eq!(b.caret(), (0, 3), "three characters, nine bytes");

        assert!(b.home());
        assert!(b.delete());
        assert_eq!(b.text(), "本語");

        assert!(b.right());
        assert!(b.insert('ご'));
        assert_eq!(b.text(), "本ご語");
        assert_eq!(b.caret(), (0, 2));

        assert!(b.end());
        assert!(b.backspace());
        assert_eq!(b.text(), "本ご");
        assert_eq!(b.caret(), (0, 2));
    }

    #[test]
    fn a_line_of_mixed_widths_joins_and_splits_on_character_boundaries() {
        let mut b = at("héllo\n日本", 1, 0);
        assert!(b.backspace(), "join the second line onto the first");
        assert_eq!(rows(&b), ["héllo日本"]);
        assert_eq!(b.caret(), (0, 5));

        assert!(b.newline());
        assert_eq!(rows(&b), ["héllo", "日本"]);
    }

    // --- the sticky column ------------------------------------------------

    #[test]
    fn up_and_down_keep_the_column_they_started_in_across_a_short_line() {
        let mut b = at("first line here\nx\nthird line here", 0, 12);

        assert!(b.down());
        assert_eq!(b.caret(), (1, 1), "the short line can hold no more");
        assert!(b.down());
        assert_eq!(
            b.caret(),
            (2, 12),
            "and the column comes back on the far side"
        );

        assert!(b.up());
        assert_eq!(b.caret(), (1, 1));
        assert!(b.up());
        assert_eq!(b.caret(), (0, 12), "which is where the caret set off from");
    }

    #[test]
    fn a_sideways_move_gives_up_the_remembered_column() {
        let mut b = at("first line here\nx\nthird line here", 0, 12);
        assert!(b.down());
        assert!(b.left(), "on the short line, so this is column zero");
        assert!(b.down());
        assert_eq!(
            b.caret(),
            (2, 0),
            "the wish is the column the user just chose"
        );
    }

    #[test]
    fn an_edit_gives_up_the_remembered_column_the_way_a_sideways_move_does() {
        let mut b = at("a long first line\nx\nanother long line", 0, 15);
        assert!(b.down());
        assert_eq!(b.caret(), (1, 1));

        assert!(b.insert('y'));
        assert!(b.down());
        assert_eq!(
            b.caret(),
            (2, 2),
            "the desire came from the edit, not from before it"
        );
    }

    // --- crossing the line ending -----------------------------------------

    #[test]
    fn backspace_at_the_start_of_a_line_joins_it_to_the_one_above() {
        let mut b = at("one\ntwo", 1, 0);
        assert!(b.backspace());
        assert_eq!(rows(&b), ["onetwo"]);
        assert_eq!(b.caret(), (0, 3), "the caret sits at the seam");
    }

    #[test]
    fn delete_at_the_end_of_a_line_pulls_the_next_one_up() {
        let mut b = at("one\ntwo", 0, 3);
        assert!(b.delete());
        assert_eq!(rows(&b), ["onetwo"]);
        assert_eq!(b.caret(), (0, 3), "and leaves the caret where it was");
    }

    #[test]
    fn left_and_right_cross_the_line_ending_in_both_directions() {
        let mut b = at("one\ntwo", 1, 0);
        assert!(b.left());
        assert_eq!(b.caret(), (0, 3), "the end of the line above");
        assert!(b.right());
        assert_eq!(b.caret(), (1, 0), "and back");
    }

    #[test]
    fn newline_takes_the_rest_of_the_line_down_with_it() {
        let mut b = at("onetwo", 0, 3);
        assert!(b.newline());
        assert_eq!(rows(&b), ["one", "two"]);
        assert_eq!(b.caret(), (1, 0));
    }

    #[test]
    fn home_and_end_move_along_the_row_and_never_off_it() {
        let mut b = at("first\nsecond line", 1, 3);
        assert!(b.end());
        assert_eq!(b.caret(), (1, 11));
        assert!(b.home());
        assert_eq!(b.caret(), (1, 0));
        assert!(b.end());
        assert_eq!(b.caret().0, 1, "neither of them changes the row");
    }

    #[test]
    fn a_key_that_names_a_column_is_heard_even_when_it_moves_nothing() {
        // Column twelve, down onto a blank line, then `Home` — which has
        // nothing to do, because the caret is already at column zero. The next
        // `down` used to travel back out to column twelve: the user pressed a
        // key that names a column and the buffer went on wanting the old one.
        let mut b = at("first line here\n\nthird line here", 0, 12);
        assert!(b.down());
        assert!(!b.home(), "already at the start of that row");
        assert!(b.down());
        assert_eq!(b.caret(), (2, 0));

        // `end` on a row the caret is already at the end of, which is the same
        // shape from the other side.
        let mut b = at("first line here\nx\nthird line here", 0, 12);
        assert!(b.down());
        assert!(!b.end(), "column one is the end of `x`");
        assert!(b.down());
        assert_eq!(b.caret(), (2, 1));

        // And the two document ends, where `left` and `right` have nowhere to
        // go. A weaker case — nobody presses these on purpose — but a rule with
        // four exceptions is a rule nobody can hold in their head.
        let mut b = at("\nthird line here", 1, 12);
        assert!(b.up());
        assert_eq!(b.caret(), (0, 0));
        assert!(!b.left(), "the start of the document");
        assert!(b.down());
        assert_eq!(b.caret(), (1, 0));

        let mut b = at("third line here\nx", 0, 12);
        assert!(b.down());
        assert!(!b.right(), "the last column of the last row");
        assert!(b.up());
        assert_eq!(b.caret(), (0, 1));
    }

    // --- what arrives from disk -------------------------------------------

    #[test]
    fn a_pad_that_arrives_too_big_is_cut_to_the_cap_and_says_that_it_was() {
        // The loop this closes. `clean` turns every tab into two spaces, so a
        // file of tabs is inside the cap on disk and twice the cap in memory;
        // the pad then saved that back, and the session after it found a file
        // it could not take whole and went read-only — made too big by abeam,
        // with nothing in the round trip that could have said so.
        let b = Buffer::from_text(NOTE, &"\t".repeat(MAX_BYTES));
        assert_eq!(
            b.text().len(),
            MAX_BYTES,
            "cut to the cap, not doubled past it"
        );
        assert!(b.truncated(), "and the pane can find out that it was");
        assert!(b.is_full());

        let b = Buffer::from_text(NOTE, "an ordinary note");
        assert!(!b.truncated());
        assert!(!Buffer::new(NOTE).truncated());
    }

    #[test]
    fn a_pad_cut_to_the_cap_is_cut_between_two_characters() {
        // The cap is even and `é` is two bytes, so a cut at exactly the cap
        // lands inside one. `String::truncate` panics on that — on load, in a
        // constructor, with the file already on disk and nothing the user could
        // do about it — so this test failing at all is the whole finding.
        let b = Buffer::from_text(NOTE, &format!("a{}", "é".repeat(MAX_BYTES)));
        assert!(b.truncated());
        assert_eq!(
            b.text().len(),
            MAX_BYTES - 1,
            "back to the last character boundary rather than through an é"
        );
        assert!(b.text().ends_with('é'));
    }

    #[test]
    fn a_pad_that_was_cut_goes_on_saying_so_after_there_is_room_again() {
        let mut b = loaded(NOTE, &"\t".repeat(MAX_BYTES));
        assert!(b.truncated());
        for _ in 0..100 {
            assert!(b.backspace());
        }
        assert!(!b.is_full(), "there is room now");
        assert!(
            b.truncated(),
            "the room came back and the tail did not, so a save would still lose it"
        );
    }

    // --- clicking ---------------------------------------------------------

    #[test]
    fn a_click_past_the_end_of_a_short_line_lands_at_the_end_of_it() {
        let mut b = Buffer::from_text(NOTE, "a long first line\nx\nanother long line");
        assert!(b.set_caret(1, 40));
        assert_eq!(
            b.caret(),
            (1, 1),
            "the end of the short row, not an index into the one below"
        );
    }

    #[test]
    fn a_click_below_the_last_row_lands_on_the_last_row() {
        let mut b = at("one\ntwo", 0, 0);
        assert!(b.set_caret(99, 99));
        assert_eq!(b.caret(), (1, 3));

        // And on an empty pad, where the only row is the one that is always
        // there.
        let mut b = Buffer::new(NOTE);
        assert!(!b.set_caret(9, 9), "there is nowhere else to be");
        assert_eq!(b.caret(), (0, 0));
    }

    #[test]
    fn a_click_where_the_caret_already_is_changes_nothing_and_says_so() {
        let mut b = at("one\ntwo", 1, 3);
        assert!(!b.set_caret(1, 3));
        assert!(
            !b.set_caret(1, 99),
            "clamped back to where it was is still nowhere new"
        );
        assert!(!b.set_caret(9, 99), "and past the last row with it");
        assert_eq!(b.caret(), (1, 3));
    }

    #[test]
    fn a_click_gives_up_the_remembered_column_the_way_a_sideways_move_does() {
        let mut b = at("first line here\nx\nthird line here", 0, 12);
        assert!(b.down(), "and column twelve is still the wish");
        assert!(b.set_caret(1, 0));
        assert!(b.down());
        assert_eq!(
            b.caret(),
            (2, 0),
            "the click named the column, and it outranks the wish"
        );
    }

    // --- pasting ----------------------------------------------------------

    #[test]
    fn a_paste_from_a_windows_clipboard_arrives_as_ordinary_lines() {
        let mut b = Buffer::new(NOTE);
        assert!(b.insert_str("one\r\ntwo\rthree\n"));
        assert_eq!(rows(&b), ["one", "two", "three", ""]);
        assert_eq!(b.text(), "one\ntwo\nthree\n");
        assert!(
            !b.text().contains('\r'),
            "a carriage return left in a line overwrites the row it is on"
        );
        assert_eq!(b.caret(), (3, 0));
    }

    #[test]
    fn a_paste_lands_at_the_caret_and_leaves_it_after_the_last_character() {
        let mut b = at("before after", 0, 7);
        assert!(b.insert_str("one\ntwo"));
        assert_eq!(rows(&b), ["before one", "twoafter"]);
        assert_eq!(b.caret(), (1, 3));

        // The single-line case splits nothing and still moves the caret to the
        // end of what arrived.
        let mut b = at("ab", 0, 1);
        assert!(b.insert_str("XY"));
        assert_eq!(rows(&b), ["aXYb"]);
        assert_eq!(b.caret(), (0, 3));
    }

    // --- the cap ----------------------------------------------------------

    #[test]
    fn a_paste_that_would_not_fit_is_refused_whole_rather_than_trimmed() {
        let mut b = loaded(NOTE, &"a".repeat(MAX_BYTES - 10));
        let before = b.text();

        assert!(!b.insert_str(&"b".repeat(11)));
        assert_eq!(b.text(), before, "not one character of it went in");
        assert_eq!(b.caret(), (0, MAX_BYTES - 10), "and the caret did not move");
        assert_eq!(b.take_change(), None, "and nothing was recorded as done");

        // The boundary is not off by one: the ten that do fit are taken.
        assert!(b.insert_str(&"b".repeat(10)));
        assert_eq!(b.text().len(), MAX_BYTES);
    }

    #[test]
    fn a_full_pad_refuses_every_way_of_making_it_bigger() {
        let mut b = loaded(NOTE, &"a".repeat(MAX_BYTES));
        assert!(!b.insert('c'));
        assert!(!b.insert('\t'));
        assert!(!b.insert_str("c"));
        assert!(
            !b.newline(),
            "a joining newline is a byte like any other, and Enter repeats"
        );
        assert_eq!(b.text().len(), MAX_BYTES);

        // It is a cap on growth and not a freeze: taking something out still
        // works, and then there is room again.
        assert!(b.backspace());
        assert!(b.insert('c'));
    }

    #[test]
    fn a_full_pad_says_so_rather_than_leaving_a_paste_to_fail_silently() {
        assert!(!Buffer::from_text(NOTE, "an ordinary note").is_full());

        let mut b = loaded(NOTE, &"a".repeat(MAX_BYTES));
        assert!(b.is_full());
        assert!(!b.insert_str("one more thought"));
        assert!(
            b.is_full(),
            "still full, which is the refused paste having gone nowhere"
        );
        assert_eq!(b.text().len(), MAX_BYTES);

        assert!(b.backspace());
        assert!(!b.is_full(), "a cap on growth, not a state to be stuck in");
    }

    #[test]
    fn the_cap_is_counted_in_the_bytes_the_file_will_have() {
        // A CRLF file with a byte order mark: every line ending is two bytes on
        // disk and the mark is three more, though the buffer holds neither.
        // Counted in memory, a file edited to the cap would be saved larger
        // than the reader's cap will open again.
        let crlf = Policy {
            max_bytes: 20,
            line_ending_bytes: 2,
            bom_bytes: 3,
            ..FILE
        };
        let mut b = loaded(crlf, "abc\ndef");
        assert_eq!(b.bytes(), 3 + 3 + 2 + 3, "mark, line, CRLF, line");
        assert_eq!(b.text_bytes(), 7, "and the text itself is still LF");

        // Nine bytes of room: a line ending costs two of them, not one.
        assert!(b.insert_str("1234567"));
        assert_eq!(b.bytes(), 18);
        assert!(!b.insert_str("\nx"), "two bytes of CRLF and an x is three");
        assert!(b.newline(), "and two bytes is exactly what is left");
        assert_eq!(b.bytes(), 20);
        assert!(!b.newline());
        assert!(b.is_full());

        // A load is cut in the same unit, and on a character boundary.
        let cut = Buffer::from_text(crlf, "aaaa\nbbbb\ncccc\n");
        assert!(cut.truncated());
        assert!(cut.bytes() <= 20, "{}", cut.bytes());
        assert_eq!(
            cut.text(),
            "aaaa\nbbbb\ncccc",
            "3 + 4 + 2 + 4 + 2 + 4 is nineteen, and the last line ending would make it 21"
        );
    }

    #[test]
    fn replacing_the_whole_text_is_one_change_that_puts_the_old_text_back() {
        let mut b = loaded(NOTE, "one\ntwo\nthree");
        b.set_caret(2, 4);
        assert_eq!(b.replace_all("uno\r\ndos\tx"), Some(true));
        assert_eq!(b.text(), "uno\ndos  x", "cleaned like a paste");
        assert_eq!(b.caret(), (1, 4), "where it was, clamped into the new text");
        let change = b.take_change().expect("one change, recorded");
        assert_eq!(change.at, (0, 0));
        assert_eq!(change.removed, "one\ntwo\nthree");
        assert_eq!(change.inserted, "uno\ndos  x");

        assert_eq!(b.replace_all("uno\ndos\tx"), Some(false), "already this text");
        assert_eq!(b.take_change(), None);

        let mut b = loaded(NOTE, "small");
        assert_eq!(b.replace_all(&"x".repeat(MAX_BYTES + 1)), None, "refused whole");
        assert_eq!(b.text(), "small");
        assert_eq!(b.replace_all(&"x".repeat(MAX_BYTES)), Some(true), "the cap itself fits");
    }

    #[test]
    fn the_cap_is_the_policys_and_a_file_may_hold_more_than_a_pad() {
        // The pad's 64 KiB is the pad's decision, not the buffer's: a file
        // opened for editing is held to the reader's 512 KiB instead, and a
        // paste the pad would refuse goes into it whole.
        let big = "x".repeat(MAX_BYTES + 1);
        let mut pad = Buffer::new(NOTE);
        assert!(!pad.insert_str(&big));
        let mut file = Buffer::new(FILE);
        assert!(file.insert_str(&big));
        assert_eq!(file.bytes(), MAX_BYTES + 1);

        let cut = Buffer::from_text(FILE, &"y".repeat(FILE.max_bytes + 5));
        assert!(cut.truncated());
        assert_eq!(cut.bytes(), FILE.max_bytes);
    }

    // --- keys that change nothing -----------------------------------------

    #[test]
    fn every_mutator_says_no_when_it_changes_nothing() {
        // `pane.rs` on `Handled`: a pane that reports `Yes` for a key that
        // changed nothing is spending a frame — including re-rendering the
        // agent's whole screen — on it. Every one of these is a key somebody
        // holds down.
        let mut b = Buffer::new(NOTE);
        assert!(!b.backspace());
        assert!(!b.delete());
        assert!(!b.left());
        assert!(!b.right());
        assert!(!b.up());
        assert!(!b.down());
        assert!(!b.home());
        assert!(!b.end());
        assert!(!b.insert_str(""), "an empty clipboard is not an edit");
        assert_eq!(b.text(), "");
        assert_eq!(b.take_change(), None);

        // The far corner of a real document, where the ends are the other ends.
        let mut b = loaded(NOTE, "one\ntwo");
        assert!(!b.right());
        assert!(!b.down());
        assert!(!b.delete());
        assert!(!b.end());
        assert!(b.home());
        assert!(!b.home());
        assert!(b.up(), "there is a row above this one");
        assert!(!b.up(), "and none above that");
        assert_eq!(b.text(), "one\ntwo", "and none of that touched the text");
    }

    // --- tabs -------------------------------------------------------------

    #[test]
    fn tab_types_two_spaces_in_the_pad_because_two_is_a_markdown_nesting_level() {
        let mut b = Buffer::new(NOTE);
        assert!(b.insert('\t'));
        assert_eq!(b.text(), "  ");
        assert_eq!(b.caret(), (0, 2), "two characters typed, two columns moved");

        // And on the paste path as well, so a snippet copied out of a code
        // block cannot smuggle one in behind the same argument.
        assert!(b.insert_str("a\tb"));
        assert_eq!(b.text(), "  a  b");
        assert!(!b.text().contains('\t'));
        assert_eq!(Buffer::from_text(NOTE, "\tindented").text(), "  indented");
    }

    #[test]
    fn the_tab_key_types_what_the_policy_says_and_a_kept_tab_stays_a_tab() {
        assert_eq!(Buffer::new(NOTE).tab_text(), "  ");
        assert_eq!(Buffer::new(FILE).tab_text(), "\t");
        let spaced = Policy {
            tabs: Tabs::Keep {
                key: TabKey::Spaces(2),
            },
            ..FILE
        };
        // A file with no tab-indented line keeps the tabs it has and types
        // spaces for new indentation.
        let mut b = Buffer::from_text(spaced, "a\tb");
        assert_eq!(b.tab_text(), "  ");
        assert!(b.insert('\t'), "the character, not the key");
        assert_eq!(b.text(), "\ta\tb");
    }

    #[test]
    fn a_file_keeps_a_literal_tab_wherever_it_arrives_from() {
        // A save has to give back the bytes it was given, so a policy that
        // keeps tabs keeps them on every way in: typed, pasted and loaded.
        let mut b = loaded(FILE, "\tindented\n");
        assert_eq!(b.text(), "\tindented\n", "loaded");
        assert!(b.insert('\t'));
        assert!(b.insert_str("a\tb"));
        assert_eq!(b.text(), "\tindented\n\ta\tb", "typed and pasted");
        assert_eq!(b.caret(), (1, 4), "a tab is one character to the caret");

        // Line endings are still the buffer's, whatever the policy: a `\r` is
        // never something a line may hold.
        assert!(b.insert_str("\r\nnext"));
        assert_eq!(rows(&b), ["\tindented", "\ta\tb", "next"]);
    }

    #[test]
    fn no_line_ever_holds_a_newline_however_one_arrives() {
        let mut b = at("ab", 0, 1);
        assert!(b.insert('\n'));
        assert_eq!(rows(&b), ["a", "b"]);
        assert!(b.insert('\r'));
        assert_eq!(rows(&b), ["a", "", "b"]);
        assert!(b.lines().iter().all(|l| !l.contains(['\n', '\r'])));
    }
}
