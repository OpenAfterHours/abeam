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
/// spending a frame — including re-rendering Claude's whole screen — on it.
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

/// What the shell needs from a right-hand view.
///
/// Deliberately only that. The terminal pane is reached concretely — it is a
/// field, never a `dyn Pane` — so anything only it can do is an inherent method
/// on it rather than a trait method three views would have to read and dismiss:
/// pasting, resizing the pty, and owning the cursor are all its alone, and
/// "only the terminal pane resizes the pty" is then a fact about the types
/// rather than a comment asking to be believed.
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
}
