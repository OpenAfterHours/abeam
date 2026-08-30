//! Where abeam is allowed to look for a program, and what it is allowed to do
//! with the one it finds.
//!
//! Two questions live here and they are answered separately on purpose.
//!
//! **Where.** This half is shared, because the hazard is. A bare name handed to
//! `CreateProcessW` is resolved against the *calling process's* current
//! directory before Windows consults `PATH`, so a repository containing
//! `claude.exe` is what runs, with the user's full token. `execve` has no such
//! rule of its own and Unix hands the same hole back through `PATH` instead: a
//! `.`, a `..`, any relative entry, and the empty string that a leading,
//! trailing or doubled separator leaves behind all name the directory abeam is
//! standing in — which under abeam is the repository on screen, the one
//! directory in this whole question that somebody else gets to write to.
//! Nothing leaves this module that is not an absolute path, on either platform.
//!
//! **What.** This half is not shared, because "a file abeam can start" is a
//! different sentence on each platform, and the difference in those sentences
//! is the whole difference in size between the two modules below.
//!
//! `windows.rs` is long. `CreateProcessW` starts a PE image and nothing else,
//! so the `.cmd` that `npm i -g` leaves behind has to be started by naming
//! `cmd.exe` in front of it, and the command line for that has to be quoted by
//! hand against a parser which reads `&`, `|`, `^`, `%` and `!` as syntax and
//! which has a length past which it silently runs nothing at all.
//!
//! `unix.rs` is three functions, and it is short because of a fact about the
//! platform rather than because it is unfinished: the kernel reads the `#!`
//! line itself, so the one extensionless file the same npm install drops is
//! directly executable. Its own module docs say so at length, because the
//! obvious "fix" for a module that short is to add shell routing, and that
//! would invent on Unix precisely the problem `windows.rs` spends four hundred
//! lines solving.
//!
//! What each of them owes this module is three answers, under the same three
//! names, so that everything below reads the same on both:
//!
//! - `probe` — what a name means inside one directory. Windows applies
//!   `PATHEXT` and prefers the file it can start; Unix looks for the name as it
//!   was given.
//! - `startable` — whether a file that has been found is one this platform will
//!   run at all. An extension on Windows, a mode bit on Unix.
//! - `into_launch` — how that file becomes something to hand a pty, or the
//!   sentence explaining why it cannot be.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use abeam_pty::PtyConfig;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::{into_launch, probe, startable};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix::{into_launch, probe, startable};

/// What the operating system will be asked to start, and with what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Always absolute, always something this platform can start. For a script
    /// routed through an interpreter — which on Windows means a `.cmd` or a
    /// `.bat`, and on Unix means nothing at all — this is the interpreter
    /// rather than the script.
    pub program: PathBuf,
    /// The file that will do the work: the same as [`program`](Self::program)
    /// for anything started directly, and the script itself when one is routed.
    /// What a border should name — "cmd" is a true answer to the wrong
    /// question.
    pub target: PathBuf,
    /// **The complete argument list, not a prefix.** For a routed script the
    /// caller's own arguments are already inside the command line this points
    /// the interpreter at; appending them again would pass them twice.
    pub args: Vec<String>,
    /// Set on the child. Empty unless a script was routed.
    pub env: Vec<(String, String)>,
}

impl Launch {
    /// The pty configuration that starts it, seeded so that no caller can
    /// forget the environment a routed launch arrived in. On Windows that is
    /// the variable carrying the command line, and forgetting it fails as
    /// `'%ABEAM_LAUNCH%' is not recognized`, which names neither the program
    /// nor the mistake.
    pub fn config(&self) -> PtyConfig {
        let mut cfg =
            PtyConfig::new(self.program.to_string_lossy()).args(self.args.iter().cloned());
        for (key, value) in &self.env {
            cfg = cfg.env(key, value);
        }
        cfg
    }
}

/// Turn the name of a program into something this platform can start, or into
/// the sentence explaining why there is not one.
///
/// The two halves are visible in the two lines below and they stay separate all
/// the way down: [`find`] answers *where*, identically everywhere, and only
/// ever with an absolute path; `into_launch` answers *what*, and is the only
/// part of this that knows which operating system it is on.
pub fn resolve(program: &str, args: &[String]) -> Result<Launch, String> {
    resolve_preferring(program, args, None)
}

/// [`resolve`], for a program abeam is already holding as a path.
///
/// **The same resolution with one conversion removed, and the conversion is
/// the point.** Every caller of [`resolve`] is holding a name somebody typed —
/// a command line, an environment variable, an entry in a candidate list — and
/// a `&str` is what those are. `crate::app::Recipe` is not: it holds a
/// [`Launch::target`] this module itself produced, and routing that back
/// through `to_string_lossy` to hand it in here would be a lossy conversion of
/// a path that is about to be compared against the filesystem. `crate::paths`
/// spends a module on what two spellings of one directory cost, and a path with
/// no faithful `String` — an unpaired surrogate on Windows, a non-UTF-8 byte on
/// Unix — comes back through `Path::new` as a *different* path, so the probe
/// below would fail to find a file that is sitting right there. A pane opened
/// on a keystroke would refuse where the session's own agent started fine, on a
/// machine where nothing else about abeam is unusual.
///
/// **What this removes is the asymmetry and not the limitation, and the
/// difference is worth being exact about.** [`Launch::config`] still spells the
/// program with `to_string_lossy` on the way to the pty, so an install path
/// that has no faithful `String` is one abeam cannot start *at all* — at
/// startup as much as on a keystroke. That is pre-existing, it is the same
/// answer for every pane, and it is a fix somewhere else entirely: it belongs
/// in `PtyConfig`, which takes a `String` where it wants an `OsString`. What
/// this stops is a later pane failing where the first one succeeded, which is
/// the shape nobody would think to look for.
pub fn resolve_at(program: &Path, args: &[String]) -> Result<Launch, String> {
    into_launch(find_at(program, None)?, args)
}

/// [`resolve`], with one path to try before the `PATH` walk.
///
/// The hint stays out of this module because it is not knowledge about
/// launching — it is knowledge about *shells*, that `cmd.exe` and
/// `powershell.exe` are operating-system components with one right answer each
/// and that PowerShell 7 has a usual home. Only the shell pane has a list of
/// programs that is short and fixed enough for such a table to exist, so the
/// table lives beside that list and its answer is passed in here.
pub fn resolve_preferring(
    program: &str,
    args: &[String],
    home: Option<PathBuf>,
) -> Result<Launch, String> {
    into_launch(find(program, home)?, args)
}

/// Where the file is, without yet asking whether it can be started.
fn find(program: &str, home: Option<PathBuf>) -> Result<PathBuf, String> {
    find_at(Path::new(program), home)
}

/// The same, over a path rather than a name.
///
/// **This is where the work always happened** — `find` began by calling
/// `Path::new` on its argument and never used the `&str` again except to write
/// it into a message, and `Path::display` writes the same characters back. So
/// this is a rename rather than a second implementation, and [`resolve_at`] is
/// what it exists for: a caller already holding a path does not have to spell
/// it as a `String` on the way in.
fn find_at(named: &Path, home: Option<PathBuf>) -> Result<PathBuf, String> {
    let program = named.display();

    let found = if named.is_absolute() {
        // Probed rather than trusted: a path to something that has since been
        // uninstalled should arrive as "not found" rather than as whatever the
        // spawn makes of it. On Windows probing also supplies the extension, so
        // an absolute `…\npm\claude` still finds `claude.cmd` beside it.
        probe(named.parent().unwrap_or(named), &file_name_of(named))
    } else if named.parent().is_some_and(|p| !p.as_os_str().is_empty()) {
        // A relative path names a place, and the place it names is relative to
        // the repository on screen — the one directory in this whole question
        // that somebody else gets to write to. `main` resolves the one it is
        // given against the directory abeam was *run* in before it gets here,
        // which is a different question with a different answer.
        return Err(format!(
            "`{program}` is a relative path, and abeam will not resolve one \
             here: it would be resolved against the repository on screen. \
             Give an absolute path, or a bare name to look up on PATH."
        ));
    } else {
        // `is_absolute` before `is_file`, and the order is not the point — the
        // check is. The caller's hint is a guess and it is not trusted to be
        // absolute any more than a `PATH` entry is: `panes::shell`'s
        // `known_home` builds it by joining onto an environment variable, so
        // one that is relative or empty makes a relative path, and `is_file()`
        // on one of those asks about the current directory — which is the
        // repository, and the whole thing this module exists to stop. With
        // `SystemRoot=Windows` this returned `Windows\System32\cmd.exe` and
        // meant it relative to whatever abeam was standing in.
        //
        // Worth being plain about, because it looks covered and is not: on
        // Windows `main` stands in `%SystemRoot%` for exactly this hazard, but
        // it is the *same variable*, so a broken `SystemRoot` takes out both
        // defences at once and this is the one of the two that can still say
        // no.
        // `to_str` rather than a lossy conversion, and the `None` is honest
        // rather than lazy: `PATH` is walked by *name*, and a name with no
        // faithful `String` is one no entry on `PATH` was ever going to match
        // under a substituted spelling. Every caller that reaches this branch
        // arrived holding a `&str` anyway — a bare name is what a command line,
        // an environment variable and a candidate list all are — so this is
        // unreachable in practice and says so by declining rather than by
        // guessing.
        home.filter(|home| home.is_absolute() && home.is_file())
            .or_else(|| named.to_str().and_then(walk_path))
    };

    found.ok_or_else(|| format!("`{program}` was not found on PATH."))
}

// --- finding it -----------------------------------------------------------

/// Look `name` up on `PATH`, and answer only with absolute paths.
fn walk_path(name: &str) -> Option<PathBuf> {
    walk(&std::env::var_os("PATH")?, name)
}

/// The search itself, over a `PATH` handed in rather than read.
///
/// Split out so that the entries this refuses to look in can be pinned by a
/// test without a test reaching for the process's environment or its current
/// directory — both of which are shared by every other test running beside it.
///
/// Two passes, which is the preference `windows.rs` already makes inside one
/// directory — the first file it can actually start, and only then whatever is
/// merely there — lifted to the walk, because on Unix a directory holds one
/// candidate and there is nothing for that preference to do down there.
///
/// It costs nothing in the ordinary case: the first pass is the one that finds
/// the program, and the second runs only when the walk is already on its way to
/// an error message. What the second pass buys is the file that message gets to
/// name, which is the whole difference between "`claude` was not found on PATH"
/// and a sentence about the `claude` in `~/bin` that nobody may execute, or the
/// `claude.ps1` in `%APPDATA%\npm` with no `.cmd` beside it.
///
/// It does mean an earlier entry holding something unstartable no longer hides
/// a later entry holding the program — on Windows as well as on Unix. That is
/// the same answer `probe` gives inside one directory, for the same reason: the
/// user asked abeam to run their agent, and a file abeam has no way to start is
/// not an answer to that question wherever on `PATH` it sits.
fn walk(path: &OsStr, name: &str) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = std::env::split_paths(path)
        // Drops both the empty entry a `;;` — or a `::` — leaves behind and any
        // genuinely relative one. Joining a name onto either gives a *relative*
        // path, and checking whether that exists asks about the current
        // directory, which is the repository and the whole thing this module
        // exists to stop.
        .filter(|dir| dir.is_absolute())
        .collect();

    dirs.iter()
        .find_map(|dir| probe(dir, name).filter(|found| startable(found)))
        .or_else(|| dirs.iter().find_map(|dir| probe(dir, name)))
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
