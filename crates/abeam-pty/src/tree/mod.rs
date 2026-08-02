//! Killing a hosted shell has to kill what the shell started.
//!
//! Both operating systems have the same hole in the middle of them: the call
//! that ends a process ends *that* process. `TerminateProcess` does not end the
//! `cargo build` the shell launched, and neither does `SIGKILL`. Neither
//! kernel will hand you the descendants afterwards — Windows keeps no
//! parent/child relationship to walk at all, and a Unix parent that has already
//! died has handed its children to `init`. That is how `Alt+S`, `cargo build`,
//! `Alt+Q` used to leave `cargo.exe` and `rustc.exe` running against a
//! pseudoconsole being torn down, in exactly the case the command view exists
//! for. Watched happen on Windows; the same failure with `SIGKILL` in place of
//! `TerminateProcess` is what the Unix half is here to prevent.
//!
//! Both operating systems also have an answer, and they are not the same
//! answer. Windows has the job object: a container a process is put into, that
//! everything it starts joins with it, and that kills its members when the last
//! handle closes. Unix has the process group: a number a process already
//! belongs to, that everything it starts inherits, and that `killpg` signals in
//! one call. See [`windows`] and [`unix`] for how each is used and what each
//! costs.
//!
//! # The shape both halves have
//!
//! One type, [`Tree`], built from the child a session just spawned:
//!
//! - `Tree::holding(child) -> Option<Tree>` — `None` when the platform would
//!   not give us one, which is not an error a caller can do anything about.
//! - `Drop` is the kill. There is no explicit `kill()`, on either platform,
//!   because there is exactly one moment at which this should happen and it is
//!   the one the session already has.
//!
//! That is the whole of it, and it is the same on both, so `session.rs` spawns
//! and tears down without a `cfg` anywhere near it.
//!
//! # Best effort, throughout
//!
//! Every failure in here leaves the caller exactly where it stood before this
//! module existed. A shell whose children outlive it is worse than one
//! contained; a shell that refuses to start because the operating system would
//! not give us a job object, or because the child was gone before we could read
//! its pid, is worse than both. Nothing here returns an error and nothing here
//! panics.
//!
//! # The asymmetry, which is not in this module's favour
//!
//! A Windows job dies with its last handle, and the handle goes when the
//! process goes — so even if abeam is killed outright, the tree goes with it.
//! Unix has nothing of the kind. `Drop` does not run when abeam is `SIGKILL`ed,
//! and a process group is not a container anybody is holding, so the tree
//! survives.
//!
//! What is left in that case is the kernel's own backstop, and it is a good
//! deal weaker: when the master closes, the session leader loses its
//! controlling terminal and its foreground process group is sent `SIGHUP`. That
//! reaches a shell sitting at a prompt. It does not reach a build that ignores
//! `SIGHUP`, and it does not reach one an interactive shell put in a process
//! group of its own, which is what job control does to every job it starts.
//! Written down here because it is the sort of thing that is discovered twice:
//! once when it is designed around, and once by whoever assumes the two
//! platforms are now equivalent.
//!
//! # One place Unix is ahead
//!
//! [`windows`] names a gap it cannot close: the child is adopted *after*
//! `CreateProcessW` returns rather than being created suspended, so anything it
//! spawns in those first microseconds is outside the job. Unix has no such
//! window. The group is not something we put the child into — the child made
//! itself the leader of it, inside `setsid()`, before it had exec'd the program
//! at all. There is no instant at which the child exists outside the group it
//! leads.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// The Windows half: a job object holding the child and its descendants.
#[cfg(windows)]
pub use windows::Job as Tree;

/// The Unix half: the process group the child leads.
#[cfg(unix)]
pub use unix::Group as Tree;
