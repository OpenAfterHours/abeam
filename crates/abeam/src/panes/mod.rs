pub mod ask;
pub mod diag;
pub mod git;
pub mod pad;
pub mod queue;
pub mod shell;
pub mod terminal;
pub mod viewer;

pub use ask::{AskContext, AskPane};
pub use diag::{DiagPane, FrameStats};
pub use git::GitPane;
pub use pad::PadPane;
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
    /// three above — `F2` remembers it, and it is reached by `F8`.
    Queue,
    /// A markdown scratch pad, one per workspace: the sentence somebody had
    /// while the agent was mid-task, kept between sessions.
    ///
    /// A workspace view like the four above, so `F2` and `Esc` put it back and
    /// `App::last_workspace_view` remembers it. That is a claim about the key
    /// rather than about the pane, and it is the claim `Diag` and `Ask` below
    /// fail: both of those are reached *from* somewhere and both put that
    /// somewhere back, so a view that remembered them would be a key that could
    /// never leave. `F9` is not pressed about anything — it is pressed because
    /// you have had a thought — so there is no somewhere for it to restore.
    ///
    /// It takes focus, which is `Shell`'s property and not a second exception:
    /// a pad you have to press a second key to type into is a picture of one.
    /// Pressed again from inside, it hands focus back.
    ///
    /// Per workspace, and for a reason weaker than the shell's and the ask's
    /// rather than for theirs — `crate::app::Space` is where that argument
    /// belongs and where it is made. Everything else about it is
    /// `crate::panes::pad`.
    Pad,
    /// The pty instrument. Not one of the workspace views — it is reached by a
    /// toggle that remembers what it displaced, because you go there to answer
    /// a question and then come back.
    Diag,
    /// A second copy of the hosted agent, which may read and may not write,
    /// asked about the file the pane you came from was showing — or about
    /// nothing in particular, which is what `F6` is for.
    ///
    /// **Not a workspace view either, and for `Diag`'s reason rather than by
    /// analogy with it.** Both keys that reach it are keys you press while
    /// looking at something else — `?` in a focused viewer or git pane, and
    /// `F6` from anywhere — so it is always somewhere you went *from*
    /// something, and `Esc` puts that something back. A view you can only
    /// arrive at from another one has to remember which, so it is left out of
    /// `App::last_workspace_view` exactly as `Diag` is. `F6` is a global key
    /// and does not change that: it is `Diag`'s `F2` rather than the queue's
    /// `F8`, which is the same distinction seen from the key's side.
    ///
    /// It is per workspace, beside the shell, because the child's working
    /// directory belongs to the child — see `crate::app::Space`. Everything
    /// else about it is `crate::panes::ask` and `crate::ask`.
    Ask,
}

/// `?`: a pane handing the ask view what it was looking at.
///
/// A newtype over an `Option` rather than a bare `Option<PathBuf>`, and the
/// inner `None` is the whole reason it exists. A `?` pressed in a reader with
/// nothing open, or in a git pane whose selected row names nothing that can be
/// read, still has to open the view. Squashed into one `Option` the two cases
/// would be indistinguishable, and the pane would be unreachable in a repository
/// with no markdown in it and nothing yet changed.
///
/// `?` is no longer the only way in — `F6` opens the same view from anywhere,
/// with nothing attached — and this type is untouched by that, which is worth a
/// sentence because it looks like it should not be. `F6` is not a request from a
/// pane: nothing was pointed at, so there is nothing to hand over, and
/// `crate::app` calls `AskPane::attach(None)` outright. What this carries is
/// still "a pane asked, and here is what it was showing", where the inner `None`
/// means it was showing nothing.
///
/// The label the pane draws is not in here: it is built where the context is,
/// from the path, so there is one rule about what a forty-six-column pane shows
/// rather than one per pane that can ask.
pub struct AskRequest(pub Option<std::path::PathBuf>);
