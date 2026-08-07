pub mod ask;
pub mod diag;
pub mod git;
pub mod queue;
pub mod shell;
pub mod terminal;
pub mod viewer;

pub use ask::{AskContext, AskPane};
pub use diag::{DiagPane, FrameStats};
pub use git::GitPane;
pub use queue::QueuePane;
pub use shell::ShellPane;
pub use terminal::TerminalPane;
pub use viewer::ViewerPane;

/// Which of the right-hand views is showing.
///
/// Every one of these objects stays alive for the whole session; this only
/// selects which one renders. Constructing them on toggle would throw away
/// scroll position, and for the shell and the ask it would throw away a child,
/// which is worse than expensive — it is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightView {
    Git,
    Viewer,
    /// A shell in a pty. The one right-hand view that hosts a terminal.
    Shell,
    /// Work lined up for the agent: typed into the live session when it goes
    /// idle, or dispatched as background agents. A workspace view like the
    /// three above — `F2` remembers it, and it is reached by `Alt+A`.
    Queue,
    /// The pty instrument. Not one of the workspace views — it is reached by a
    /// toggle that remembers what it displaced, because you go there to answer
    /// a question and then come back.
    Diag,
    /// A second Claude, which may read and may not write, asked about the file
    /// the pane you came from was showing.
    ///
    /// **Not a workspace view either, and for `Diag`'s reason rather than by
    /// analogy with it.** There is no global key that shows this: it is reached
    /// by `?` in a focused viewer or git pane, which means it is always
    /// somewhere you went *from* something, and `Esc` puts that something back.
    /// A view you can only arrive at from another one has to remember which,
    /// so it is left out of `App::last_workspace_view` exactly as `Diag` is.
    ///
    /// It is per workspace, beside the shell, because the child's working
    /// directory belongs to the child — see `crate::app::Space`. Everything
    /// else about it is `crate::panes::ask` and `crate::ask`.
    Ask,
}

/// `?`: a pane handing the ask view what it was looking at.
///
/// A newtype over an `Option` rather than a bare `Option<PathBuf>`, and the
/// inner `None` is the whole reason it exists. `?` is the *only* way to this
/// view — there is no `Alt` key for it, deliberately — so a `?` pressed in a
/// reader with nothing open, or in a git pane whose selected row names nothing
/// that can be read, still has to open it. Squashed into one `Option` the two
/// cases would be indistinguishable, and the pane would be unreachable in a
/// repository with no markdown in it and nothing yet changed.
///
/// The label the pane draws is not in here: it is built where the context is,
/// from the path, so there is one rule about what a forty-six-column pane shows
/// rather than one per pane that can ask.
pub struct AskRequest(pub Option<std::path::PathBuf>);
