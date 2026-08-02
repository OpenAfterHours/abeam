pub mod diag;
pub mod git;
pub mod queue;
pub mod shell;
pub mod terminal;
pub mod viewer;

pub use diag::{DiagPane, FrameStats};
pub use git::GitPane;
pub use queue::QueuePane;
pub use shell::ShellPane;
pub use terminal::TerminalPane;
pub use viewer::ViewerPane;

/// Which of the right-hand views is showing.
///
/// All four objects stay alive for the whole session; this only selects which
/// one renders. Constructing them on toggle would throw away scroll position,
/// and for the shell it would throw away the session, which is worse than
/// expensive — it is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightView {
    Git,
    Viewer,
    /// A shell in a pty. The one right-hand view that takes typing.
    Shell,
    /// Work lined up for the agent: typed into the live session when it goes
    /// idle, or dispatched as background agents. A workspace view like the
    /// three above — `F2` remembers it, and it is reached by `Alt+A`.
    Queue,
    /// The pty instrument. Not one of the workspace views — it is reached by a
    /// toggle that remembers what it displaced, because you go there to answer
    /// a question and then come back.
    Diag,
}
