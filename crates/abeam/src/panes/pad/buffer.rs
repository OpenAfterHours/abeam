//! The scratch pad's text, and the caret in it.
//!
//! This is the first place in abeam where a caret lives *inside* text. The
//! queue's composer and the ask's composer are both append-only — `push` and
//! `pop` are their whole vocabulary — so neither has ever had to answer where
//! the insertion point is, and neither would survive being asked. A pad is a
//! document you go back into and change the middle of, so it has to.
//!
//! It is a `Vec<String>`, one entry per line, rather than one `String` with an
//! offset into it or a rope. A rope buys nothing at this size — the whole pad
//! is capped at [`MAX_BYTES`], and the pane hands the text to the highlighter
//! whole on every frame regardless — and a single `String` would put the line
//! the caret is on at the far end of a scan, so every keystroke would count
//! newlines from the start of the document to work out which row had changed.
//! Lines are also the shape both consumers already want: the highlighter
//! returns one row of spans per source line, and a caret is drawn at a row and
//! a column.
//!
//! There is no undo, no selection and no word motion, and that is a decision
//! rather than a stopping point. The pad is for a sentence you had while the
//! agent was busy; every one of those features is a second keymap to learn and
//! a second state to draw, spent on making the pad more like the editor the
//! user already has open in the window next door.
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
//! It is still the trade to take for a scratch pad. Stepping by cluster means a
//! segmentation crate, and this codebase argues about every dependency it takes
//! in the manifest that names them; combining marks are rare in the prose
//! people type into a note, and the damage is one keystroke that looks odd
//! rather than a panic or a lost document. Changing it later is a dependency,
//! those three methods, and one decision that reaches further than they do:
//! [`Buffer::caret`] would then report a column counted in clusters, and the
//! pane places the cursor by measuring a prefix of that many *characters*.

/// The most the pad will hold. An insert or a paste that would take it past
/// this is refused rather than trimmed to fit.
///
/// The same number as `crate::panes::viewer::source::HIGHLIGHT_MAX_BYTES`, and
/// deliberately the same rather than coincidentally: past that size the
/// highlighter gives up and returns plain text, so a pad allowed to grow beyond
/// it would go grey one keystroke after it was fine, with nothing on screen
/// saying why. What can be typed and what can be drawn in colour are one
/// decision. The value is written out rather than taken from that constant only
/// because `source` is private to the viewer; if it is ever widened these two
/// should be joined.
///
/// A paste that does not fit is refused whole. Half a pasted paragraph is worse
/// than none of it: the user has to notice the cut, and the place it happened
/// is off the bottom of a pane they had already stopped looking at.
pub const MAX_BYTES: usize = 64 * 1024;

/// What a tab types.
///
/// Two rather than four because the pad holds markdown, where two spaces is a
/// nesting level and four is a code block. Spaces rather than a tab because a
/// literal `\t` has no width the highlighter and the wrapper can agree on —
/// `viewer::source`'s own `TAB` constant carries the argument, and the caret
/// makes it sharper: [`Buffer::caret`] reports a column the pane measures by
/// taking that many characters off the front of the line, and a tab drawn as
/// four cells and measured as one would put the cursor somewhere the text is
/// not.
const TAB: &str = "  ";

/// A pad's worth of text, with somewhere in it to type.
pub struct Buffer {
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
    /// problem: the pane measures cells with `unicode_width`, and what it
    /// measures is the prefix this column names.
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
    /// This pad arrived larger than [`MAX_BYTES`] and what is here is the front
    /// of it. Set by [`Buffer::from_text`], never cleared, and read through
    /// [`Buffer::truncated`], which carries the argument.
    truncated: bool,
}

impl Buffer {
    /// An empty pad, which is one empty line with the caret at the start of it.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            desired: 0,
            truncated: false,
        }
    }

    /// A pad loaded from what was saved, with the caret at the end of it.
    ///
    /// At the end rather than at the start because of what reopening a pad is
    /// for: the note you are about to add goes after the notes you already
    /// made. A caret parked at `0,0` would put the next sentence in front of
    /// them.
    ///
    /// The text is cleaned on the way in for the reason [`clean`] gives. That
    /// makes `from_text` lossy for exactly one input — a CRLF file, which comes
    /// back with LF endings — and the alternative was to hold a `\r` that the
    /// pane cannot draw and the caret cannot count.
    ///
    /// Capped like every other way in, and the *only* one where the overflow is
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
    pub fn from_text(text: &str) -> Self {
        let mut text = clean(text);
        let truncated = text.len() > MAX_BYTES;
        if truncated {
            let mut end = MAX_BYTES;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let row = lines.len() - 1;
        let col = lines[row].chars().count();
        Self {
            lines,
            row,
            col,
            desired: col,
            truncated,
        }
    }

    /// The whole pad as one string, which is what gets saved.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The lines, for drawing. Never empty.
    pub fn lines(&self) -> &[String] {
        &self.lines
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
    /// told: the argument on [`MAX_BYTES`] is that they must never be handed
    /// half a sentence they have to notice, and a `Ctrl+V` that does nothing
    /// and says nothing fails that same test — a full pad and a broken one look
    /// identical from the outside.
    ///
    /// It reports the pad's state rather than the last refusal, and those are
    /// not quite the same question: a paste can be turned away with room to
    /// spare, when what is left is smaller than what arrived. That case is
    /// still tellable apart, because the only other way [`Buffer::insert_str`]
    /// returns `false` is an empty argument and the caller knows what it
    /// passed — but this is the cheap signal rather than the complete one, and
    /// it is the one that can be read *before* a paste as well as after.
    pub fn is_full(&self) -> bool {
        self.bytes() >= MAX_BYTES
    }

    /// Whether this pad is the front of a larger one.
    ///
    /// True when [`Buffer::from_text`] was handed more than [`MAX_BYTES`] and
    /// kept the beginning of it. The pane must read this and refuse to save,
    /// because the alternative is the quietest data loss in the program: the
    /// tail the user cannot see is written away by the first flush after they
    /// type a character, and nothing on screen was ever different.
    ///
    /// It never goes back to false, and that is the whole of its value. An edit
    /// that makes room does not bring the tail back with it, so a flag that
    /// cleared itself on the first `backspace` would hand the save path
    /// permission to overwrite exactly the document it was there to protect.
    ///
    /// **The waiver that used to stand here has gone because the wiring pass it
    /// was waiting for happened.** It said the pane latched
    /// `store::Loaded::truncated` and not yet this one, and that the two would
    /// be ORed together when it did; `PadPane::ensure_read` does exactly that. A
    /// waiver is the shape of thing that outlives its own argument, so it is
    /// worth one sentence saying which argument this one was.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Where the caret is, as `(row, col)` with `col` counted in `char`s.
    ///
    /// Always inside the text — see the invariants on `lines` and `col` — so a
    /// caller may index with it.
    pub fn caret(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// Type one character. Returns whether anything changed.
    ///
    /// Tabs and line endings are dealt with here rather than at the key
    /// handler, so that there is one answer for every caller: the pane, a
    /// paste, and whatever the next thing to type into a pad turns out to be.
    /// A `\n` that was written into a line instead of splitting it would break
    /// the invariant on `lines` quietly, and the symptom — a row that renders
    /// as two and a caret column measured against the wrong half — would not
    /// point back here.
    pub fn insert(&mut self, c: char) -> bool {
        match c {
            '\t' => return self.insert_str(TAB),
            '\n' | '\r' => return self.newline(),
            _ => {}
        }
        if self.bytes() + c.len_utf8() > MAX_BYTES {
            return false;
        }
        let byte = byte_at(&self.lines[self.row], self.col);
        self.lines[self.row].insert(byte, c);
        self.col += 1;
        self.desired = self.col;
        true
    }

    /// Paste. Returns whether anything changed.
    ///
    /// The text is cleaned first and then split, so a clipboard carrying `\r\n`
    /// — which is every clipboard on the machine this was written on — becomes
    /// the lines it looks like rather than one line with holes in it. The caret
    /// finishes after the last character that went in, which is where the next
    /// thing typed belongs.
    ///
    /// Refused whole when it will not fit, for the reason on [`MAX_BYTES`].
    pub fn insert_str(&mut self, s: &str) -> bool {
        let text = clean(s);
        if text.is_empty() {
            return false;
        }
        if self.bytes() + text.len() > MAX_BYTES {
            return false;
        }

        // The caret splits the line it is on; the first piece of the paste
        // finishes that line, and whatever was to the right of the caret goes
        // on the end of the last piece.
        let byte = byte_at(&self.lines[self.row], self.col);
        let tail = self.lines[self.row].split_off(byte);
        let mut pieces = text.split('\n');
        let first = pieces.next().unwrap_or_default();
        self.lines[self.row].push_str(first);
        let mut added: Vec<String> = pieces.map(str::to_string).collect();

        if added.is_empty() {
            self.col += first.chars().count();
            self.lines[self.row].push_str(&tail);
        } else {
            let last = added.len() - 1;
            self.col = added[last].chars().count();
            added[last].push_str(&tail);
            let at = self.row + 1;
            self.row += added.len();
            let after = self.lines.split_off(at);
            self.lines.extend(added);
            self.lines.extend(after);
        }
        self.desired = self.col;
        true
    }

    /// Break the line at the caret. Returns whether anything changed.
    ///
    /// Capped like every other way in, which the API sketch did not ask for
    /// and which the cap is worthless without: a limit that only the paste path
    /// honours is one that a held-down `Enter` walks straight past, and the
    /// result is a saved pad the highlighter has already given up on.
    pub fn newline(&mut self) -> bool {
        if self.bytes() + 1 > MAX_BYTES {
            return false;
        }
        let byte = byte_at(&self.lines[self.row], self.col);
        let tail = self.lines[self.row].split_off(byte);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
        self.desired = 0;
        true
    }

    /// Delete backwards. At the start of a line this joins it to the one above,
    /// leaving the caret at the seam. Returns whether anything changed.
    pub fn backspace(&mut self) -> bool {
        if self.col > 0 {
            let byte = byte_at(&self.lines[self.row], self.col - 1);
            self.lines[self.row].remove(byte);
            self.col -= 1;
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.row_len();
            self.lines[self.row].push_str(&line);
        } else {
            // The start of the first line, which is the one place there is
            // nothing behind the caret to take. The last line is never removed
            // here, which is half of why `lines` is never empty.
            return false;
        }
        self.desired = self.col;
        true
    }

    /// Delete forwards. At the end of a line this pulls the next one up without
    /// moving the caret. Returns whether anything changed.
    pub fn delete(&mut self) -> bool {
        if self.col < self.row_len() {
            let byte = byte_at(&self.lines[self.row], self.col);
            self.lines[self.row].remove(byte);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        } else {
            return false;
        }
        self.desired = self.col;
        true
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

    /// How many `char`s are on the row the caret is on.
    fn row_len(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    /// What [`Buffer::text`] would be the length of, without building it.
    ///
    /// Recomputed on every edit rather than carried as a field. A cached length
    /// is a second copy of the truth maintained by nine methods, and the first
    /// one to forget it leaves a pad that refuses a paste it has room for, or
    /// takes one it does not — a bug that only shows up at the size where
    /// nobody is looking. At [`MAX_BYTES`] this walks a few thousand short
    /// strings, which is nothing beside the syntax highlighting the same
    /// keystroke is about to pay for.
    ///
    /// The `- 1` is safe because `lines` is never empty, and the joining
    /// newlines are counted because [`Buffer::text`] writes them.
    fn bytes(&self) -> usize {
        self.lines.iter().map(String::len).sum::<usize>() + self.lines.len() - 1
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
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

/// Text with the two things a line may not contain taken out of it.
///
/// Line endings first. A Windows clipboard hands over `\r\n` and an old Mac
/// file hands over a lone `\r`; either one left inside a line is a character
/// the terminal cannot draw, so it comes out as a control picture or returns
/// the cursor to the left margin and lets the rest of the row overwrite what
/// was already there. It would also make the buffer's own two accounts of
/// itself disagree, since [`Buffer::lines`] would report one line where
/// [`Buffer::text`] round-tripped through a file gives two.
///
/// Tabs second, to the same [`TAB`] that [`Buffer::insert`] writes. A pasted
/// tab is the same problem the typed one is — the viewer expands tabs to
/// four-column stops when it draws, while a column here counts characters, so a
/// line with one in it is drawn at a different width than the caret is measured
/// against and the cursor sits away from the text.
fn clean(text: &str) -> String {
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
            '\t' => out.push_str(TAB),
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
        let mut b = Buffer::from_text(text);
        b.row = row;
        b.col = col;
        b.desired = col;
        b
    }

    // --- the invariant everything else rests on ---------------------------

    #[test]
    fn an_empty_buffer_is_one_empty_line_and_cannot_be_emptied_further() {
        let mut b = Buffer::new();
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

        let mut b = Buffer::from_text("one\n\nthrée\n日本\nfour");
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
        let mut b = Buffer::new();
        assert!(b.is_empty());
        assert!(b.newline());
        assert!(!b.is_empty(), "somebody pressed a key to make that line");
        assert!(b.backspace());
        assert!(b.is_empty());
        assert!(b.insert(' '));
        assert!(!b.is_empty(), "a space is text");
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
            assert_eq!(Buffer::from_text(source).text(), source);
        }
        // The one input that comes back changed, and on purpose: a `\r` is not
        // something a line is allowed to hold.
        assert_eq!(Buffer::from_text("one\r\ntwo").text(), "one\ntwo");
    }

    #[test]
    fn from_text_leaves_the_caret_at_the_end_because_a_pad_is_reopened_to_add_to() {
        assert_eq!(Buffer::from_text("a note\nand another").caret(), (1, 11));
        assert_eq!(Buffer::from_text("").caret(), (0, 0));
        assert_eq!(Buffer::from_text("done\n").caret(), (1, 0));
    }

    // --- characters, not bytes --------------------------------------------

    #[test]
    fn a_column_counts_characters_so_an_accent_is_one_step_and_not_two() {
        let mut b = Buffer::from_text("héllo");
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
        let mut b = Buffer::from_text("日本語");
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
        let b = Buffer::from_text(&"\t".repeat(MAX_BYTES));
        assert_eq!(
            b.text().len(),
            MAX_BYTES,
            "cut to the cap, not doubled past it"
        );
        assert!(b.truncated(), "and the pane can find out that it was");
        assert!(b.is_full());

        let b = Buffer::from_text("an ordinary note");
        assert!(!b.truncated());
        assert!(!Buffer::new().truncated());
    }

    #[test]
    fn a_pad_cut_to_the_cap_is_cut_between_two_characters() {
        // The cap is even and `é` is two bytes, so a cut at exactly the cap
        // lands inside one. `String::truncate` panics on that — on load, in a
        // constructor, with the file already on disk and nothing the user could
        // do about it — so this test failing at all is the whole finding.
        let b = Buffer::from_text(&format!("a{}", "é".repeat(MAX_BYTES)));
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
        let mut b = Buffer::from_text(&"\t".repeat(MAX_BYTES));
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
        let mut b = Buffer::from_text("a long first line\nx\nanother long line");
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
        let mut b = Buffer::new();
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
        let mut b = Buffer::new();
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
        let mut b = Buffer::from_text(&"a".repeat(MAX_BYTES - 10));
        let before = b.text();

        assert!(!b.insert_str(&"b".repeat(11)));
        assert_eq!(b.text(), before, "not one character of it went in");
        assert_eq!(b.caret(), (0, MAX_BYTES - 10), "and the caret did not move");

        // The boundary is not off by one: the ten that do fit are taken.
        assert!(b.insert_str(&"b".repeat(10)));
        assert_eq!(b.text().len(), MAX_BYTES);
    }

    #[test]
    fn a_full_pad_refuses_every_way_of_making_it_bigger() {
        let mut b = Buffer::from_text(&"a".repeat(MAX_BYTES));
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
        assert!(!Buffer::from_text("an ordinary note").is_full());

        let mut b = Buffer::from_text(&"a".repeat(MAX_BYTES));
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

    // --- keys that change nothing -----------------------------------------

    #[test]
    fn every_mutator_says_no_when_it_changes_nothing() {
        // `pane.rs` on `Handled`: a pane that reports `Yes` for a key that
        // changed nothing is spending a frame — including re-rendering the
        // agent's whole screen — on it. Every one of these is a key somebody
        // holds down.
        let mut b = Buffer::new();
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

        // The far corner of a real document, where the ends are the other ends.
        let mut b = Buffer::from_text("one\ntwo");
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
    fn tab_types_two_spaces_because_a_literal_tab_has_no_agreed_width() {
        let mut b = Buffer::new();
        assert!(b.insert('\t'));
        assert_eq!(b.text(), "  ");
        assert_eq!(b.caret(), (0, 2), "two characters typed, two columns moved");

        // And on the paste path as well, so a snippet copied out of a code
        // block cannot smuggle one in behind the same argument.
        assert!(b.insert_str("a\tb"));
        assert_eq!(b.text(), "  a  b");
        assert!(!b.text().contains('\t'));
        assert_eq!(Buffer::from_text("\tindented").text(), "  indented");
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
