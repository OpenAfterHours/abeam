//! Host a child process in a pty, parse its output into a screen you can draw,
//! and encode input back into it — correctly, on Windows ConPTY and on a Unix
//! pty.
//!
//! ```no_run
//! # fn main() -> Result<(), abeam_pty::PtyError> {
//! use abeam_pty::{PtyConfig, PtySession};
//!
//! let session = PtySession::spawn(PtyConfig::new("claude").size(40, 100))?;
//! session.with_screen(|s| s.size());
//! # Ok(()) }
//! ```
//!
//! # What this crate is for
//!
//! Most of what it is for is Windows'. ConPTY has a startup handshake that
//! hangs you if you miss it, no usable EOF to wait on, and a console key-event
//! model that double-types everything if you take it at face value. All three
//! are handled here, and all three are written up in `docs/conpty-findings.md`
//! at the repository root. That document is the reason this crate exists; read
//! it before changing anything in [`session`] or [`input`].
//!
//! Read it knowing which of its five constraints are Windows telling you how it
//! is, and which are true wherever this runs. The handshake and the missing EOF
//! are ConPTY's alone — a Unix pty asks nothing at startup and gives a clean EOF
//! when the last slave descriptor closes — and the key-release filter is the
//! Windows console's. The other two are not about the platform at all: mouse
//! reports go only to a program that asked for that class of event, and
//! `Screen::contents()` rejoins wrapped rows, so anything positional has to read
//! `Screen::rows()`. Those two hold on Linux exactly as written, and the code
//! that keeps them is the same code on both.
//!
//! What a caller has to know about any of it: nothing. The differences are all
//! below this API, including how a dropped session takes the child's
//! descendants with it — a job object on Windows and a process group on Unix.
//!
//! The library owns nothing above the pty: no layout, no focus, no raw mode. It
//! assumes its user already owns the terminal and is already in a draw loop.
//! See `examples/host.rs` for a complete ratatui host in ~130 lines.

pub mod input;
mod session;
mod tree;

pub use session::{PtyConfig, PtyError, PtySession, PtyStats, ScreenGuard};

/// Re-exported so a caller can name the same `vt100` this crate parses into.
/// Handing a widget a `Screen` from a *different* vt100 produces an error that
/// reads "expected Screen, found Screen", which costs an hour to understand.
pub use vt100;

pub use portable_pty::ExitStatus;
