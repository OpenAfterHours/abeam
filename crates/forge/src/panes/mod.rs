pub mod diag;
pub mod git;
pub mod terminal;
pub mod viewer;

pub use diag::DiagPane;
pub use git::GitPane;
pub use terminal::TerminalPane;
pub use viewer::ViewerPane;

/// Which of the right-hand views is showing.
///
/// All three objects stay alive for the whole session; this only selects which
/// one renders. Constructing them on toggle would throw away scroll position
/// and make the toggle feel expensive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightView {
    Git,
    Viewer,
    /// The pty instrument. Not one of the two workspace views — it is reached
    /// by a toggle that remembers what it displaced, because you go there to
    /// answer a question and then come back.
    Diag,
}
