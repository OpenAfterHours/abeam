//! Host a child process in a pty, parse its output into a screen you can draw,
//! and encode input back into it — correctly, on Windows ConPTY.
//!
//! ```no_run
//! # fn main() -> Result<(), forge_pty::PtyError> {
//! use forge_pty::{PtyConfig, PtySession};
//!
//! let session = PtySession::spawn(PtyConfig::new("claude").size(40, 100))?;
//! session.with_screen(|s| s.size());
//! # Ok(()) }
//! ```
//!
//! # What this crate is for
//!
//! ConPTY has a startup handshake that hangs you if you miss it, no usable EOF
//! to wait on, and a Windows key-event model that double-types everything if
//! you take it at face value. All three are handled here, and all three are
//! written up in `docs/conpty-findings.md` at the repository root. That
//! document is the reason this crate exists; read it before changing anything
//! in [`session`] or [`input`].
//!
//! The library owns nothing above the pty: no layout, no focus, no raw mode. It
//! assumes its user already owns the terminal and is already in a draw loop.
//! See `examples/host.rs` for a complete ratatui host in ~130 lines.

pub mod input;
mod session;

pub use session::{PtyConfig, PtyError, PtySession, PtyStats, ScreenGuard};

/// Re-exported so a caller can name the same `vt100` this crate parses into.
/// Handing a widget a `Screen` from a *different* vt100 produces an error that
/// reads "expected Screen, found Screen", which costs an hour to understand.
pub use vt100;

pub use portable_pty::ExitStatus;
