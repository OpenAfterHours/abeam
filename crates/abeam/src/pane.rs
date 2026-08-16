//! The contract between the app shell and the things it draws.
//!
//! The shell owns all chrome — border, title, focus highlight — so a pane only
//! ever sees the rect *inside* the border. That is why the pty can be sized
//! from the same number that gets drawn: there is no second calculation to
//! drift out of agreement with the first.

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;

/// Whether a pane acted on an event.
///
/// Doubles as the redraw signal, and genuinely: the shell draws a frame only
/// for events something came of, so `j` at the bottom of a document costs
/// nothing at all. A pane that reports `Yes` for a key that changed nothing is
/// spending a frame — including re-rendering the agent's whole screen — on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

impl Handled {
    pub fn is_yes(self) -> bool {
        self == Handled::Yes
    }
}

impl From<bool> for Handled {
    fn from(b: bool) -> Self {
        if b { Handled::Yes } else { Handled::No }
    }
}

/// Which half of the window keystrokes go to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Left,
    Right,
}

/// What the shell needs from a view.
///
/// The last four methods used to be inherent on `TerminalPane`, on the argument
/// that only it hosted a pty and a trait method three read-only views ignore is
/// a trait method that lies. That argument expired when the shell view landed:
/// there are now two panes with a live child in them, one of them reached only
/// as `&mut dyn Pane`, and the questions "where is your cursor", "what size were
/// you drawn at" and "do you take typing" are the shell's to ask of whichever
/// pane is on screen. They default to the read-only answer, so a pane that has
/// no child still says nothing about one.
pub trait Pane {
    /// Shown in the border. Owned: every caller ends up with a `String`
    /// anyway, and the git pane rebuilds its title from live state on every
    /// frame regardless.
    fn title(&self) -> String;

    /// Draw into `inner`. Do not draw a border; the shell already did.
    fn render(&mut self, f: &mut Frame, inner: Rect);

    /// Called once per loop iteration, focused or not. **Must not block** —
    /// slow work belongs on a worker thread that sets a flag this reads.
    /// Returns true if the pane wants to be redrawn.
    fn tick(&mut self) -> bool {
        false
    }

    /// Only called when this pane has focus, and never with a Release event.
    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        let _ = key;
        Ok(Handled::No)
    }

    /// `ev.column` and `ev.row` have already been made pane-relative and
    /// 0-based by the shell — the pane does not know where it is on screen and
    /// must not try to work it out.
    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        let _ = ev;
        Ok(Handled::No)
    }

    /// A glance binding — `Alt+J`/`Alt+K`/`Alt+PgDn`/`Alt+PgUp` — arriving as
    /// the bare key the pane would have seen had it been focused.
    ///
    /// In a read-only view `Down` and `Alt+J` mean the same thing, so the
    /// default is ordinary key handling and one implementation serves both.
    /// Where typing goes *into* the pane the two are opposites: a bare `Down`
    /// belongs to whatever is hosted there — shell history, a filter box — and
    /// only this path is the user asking to move the pane itself. So the
    /// default declines rather than forwarding, and a pane that can scroll
    /// while taking input says how. Getting this wrong is not a dead key: it is
    /// a glance at the command view silently walking its history.
    fn scroll_key(&mut self, key: KeyEvent) -> Result<Handled> {
        if self.takes_input() {
            Ok(Handled::No)
        } else {
            self.handle_key(key)
        }
    }

    /// Does typing go *into* this pane, right now?
    ///
    /// A question about this instant, not about the type: a shell whose child
    /// has exited takes nothing, and a read-only view with a filter box open
    /// takes everything until the box closes.
    ///
    /// Two things read it, and both are about what the *user* should be told or
    /// handed next — never about how an event is dispatched. Leaving such a
    /// pane for one that does not take typing hands focus back to the agent, so
    /// `Alt+G` means the same thing from everywhere. And a pane that takes
    /// typing has somewhere for a paste to go.
    ///
    /// Dispatch is [`Handled`]'s job. A pane that wants a key says so by
    /// claiming it, and no second predicate is consulted — one that could
    /// disagree with what `handle_key` actually did would be wrong for exactly
    /// the states above.
    fn takes_input(&self) -> bool {
        false
    }

    /// What the border promises as the way out, while this pane has focus.
    ///
    /// Not derivable from [`takes_input`](Pane::takes_input), which is why it
    /// is asked separately: there are three answers, not two. A read-only view
    /// gives focus back to the agent on `Esc`. A shell keeps `Esc` for its
    /// child, so the way out is `Alt+S`. A view with a filter box open takes
    /// `Esc` itself and gives you back the list — one press short of the agent,
    /// and a border that said `esc→agent` there would be naming a key that does
    /// something else.
    ///
    /// It says `agent` rather than the name of whatever is actually hosted, and
    /// that is a decision rather than a shortcut. Naming it would mean threading
    /// the hosted program's name through this signature and every pane that
    /// implements it, to earn a word the *left* border is already showing: the
    /// destination of `Esc` is the pane on the other side of the divider, and
    /// that pane is titled with the real program name. A hint that repeated it
    /// would cost four implementations and a lifetime parameter, to say
    /// `claude` twice on one screen.
    ///
    /// The border is the only place this is written down, so it has to be true
    /// in every state the pane can be in, including the ones it passes through.
    fn exit_hint(&self) -> &'static str {
        " · esc→agent"
    }

    /// Pane-relative `(col, row)` of a text cursor, or `None` for no cursor.
    ///
    /// Drawn only while the pane has focus. It is the strongest focus signal
    /// there is, because it is what a typist is already looking at.
    fn cursor(&self) -> Option<(u16, u16)> {
        None
    }

    /// The rect this pane was just drawn into, handed back after the frame.
    ///
    /// Only a pane with a pty behind it has anything to do here, and for those
    /// it is the *only* honest moment to resize: the size that was drawn and
    /// the size the child is told are then one number rather than two that can
    /// drift. Called every frame, so an implementation must be a no-op when
    /// nothing changed.
    fn on_resize(&mut self, inner: Rect) -> Result<()> {
        let _ = inner;
        Ok(())
    }

    /// The text on rows `first..=last`, if this pane can say it better than the
    /// screen can.
    ///
    /// `None` — the default, and the right answer for five of the six views —
    /// means "what was drawn is what there is", and the shell reads the rows
    /// back out of the frame it drew. See `crate::select` for why a selection
    /// here is whole rows of the *pane* rather than a range in the content.
    ///
    /// The one override is the shell view, and it earns itself: a terminal grid
    /// knows which rows are continuations of the row above, so a command line
    /// that wrapped over three rows comes back as the one line it was typed as.
    /// A frame cannot know that — a wrapped row and a row that happens to be
    /// full look identical once drawn — and a path or a URL rejoined with a
    /// newline in the middle of it is worse than not copying it at all.
    ///
    /// Rows are the pane's own, 0-based, as [`render`](Pane::render) was given
    /// them, and `last` may be past the end: it is a row on screen, and what is
    /// behind it is the pane's business to clamp.
    fn selected_text(&self, first: u16, last: u16) -> Option<String> {
        let _ = (first, last);
        None
    }

    /// A bracketed paste, offered to whichever pane has focus.
    ///
    /// Offered unconditionally, and declined by returning `No` — the same
    /// mechanism as every other event, rather than the shell deciding from
    /// [`takes_input`](Pane::takes_input) who deserves one. That leaves room for
    /// a read-only pane with a filter box in it, which wants a pasted path and
    /// still wants `Esc` to mean what it means there.
    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        let _ = text;
        Ok(Handled::No)
    }
}
