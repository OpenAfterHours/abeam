//! abeam's own directory in the user's profile: where the scratch pad keeps its
//! notes, and where an unsaved edit keeps its recovery copy.
//!
//! This was `crate::panes::pad::store`'s, and it moved down here the day a
//! second writer needed it, for the reason `crate::paths::workspace_key` gives
//! about a naming rule: two copies of a rule about where somebody's text lives
//! are two answers that must agree for ever, and the first time they stop, a
//! file written under one is not there under the other. The pad's module docs
//! still carry the argument for *which* directory — the profile and never the
//! repository, and `XDG_DATA_HOME` rather than the configuration directory —
//! because that argument is about what a note is. This module is only the rule.
//!
//! What sits inside the directory is each caller's: `scratch/` for the pad and
//! `drafts/` for `crate::disk::drafts`, siblings so that a person who opens one
//! finds the other beside it.

use std::path::PathBuf;

/// abeam's own directory inside the profile root, which is shared with every
/// other program on the machine. The same name `crate::config` uses, because it
/// is the same program.
pub(crate) const DIR: &str = "abeam";

/// What names the profile on this platform, for the messages that have to tell
/// somebody why their text has nowhere to go.
#[cfg(windows)]
pub(crate) const PROFILE: &str = "%APPDATA%";
#[cfg(unix)]
pub(crate) const PROFILE: &str = "$XDG_DATA_HOME and $HOME";

/// abeam's directory in this machine's profile, or `None` when the environment
/// will not say where the profile is.
///
/// Read from the environment on every call, which is cheap and is not a second
/// derivation of anything: the variables are the input, and the rule below is
/// the one place that turns them into a path. A caller that has to keep the
/// answer stable for a session — the pad does — asks once and keeps it.
#[cfg(windows)]
pub(crate) fn dir() -> Option<PathBuf> {
    from_appdata(std::env::var_os("APPDATA").map(PathBuf::from))
}

/// The Unix twin, over the two variables the XDG base directory specification
/// names for user data in the order it names them.
#[cfg(unix)]
pub(crate) fn dir() -> Option<PathBuf> {
    from_xdg(
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// Windows' answer, over the variable handed in rather than read.
///
/// Split out for `crate::config::from_appdata`'s reason, which is that the
/// process environment belongs to the whole test binary: a test that set
/// `APPDATA` to prove this rule would be setting it for every test running
/// beside it, several of which spawn children that inherit it.
///
/// `%APPDATA%` and nothing behind it, for the reasons `crate::config::path`
/// gives at length: `USERPROFILE` would leave a bare `abeam` directory in
/// somebody's home, and `HOME` on Windows is git-bash's, which frequently names
/// a POSIX-shaped path no Windows program has ever written to. The difference
/// there was that abeam was choosing where to look for its own file; here it is
/// choosing where to *put* one, which is the same requirement with the stakes
/// the other way up.
///
/// **A relative variable is refused rather than followed**, which is
/// `crate::config`'s rule unchanged and for exactly the same reason. Joining
/// onto a relative path leaves a relative path, so the write that follows stops
/// being a question about the user's profile and becomes one about wherever
/// this process happens to be standing — which `main` deliberately moves to
/// `%SystemRoot%` or `/`, and which before that line is the repository on
/// screen. An `APPDATA=.` left in a shell for some other program's benefit
/// would then drop an `abeam` directory into a clone, which is the one place
/// everything kept here exists to stay out of. Absoluteness rather than mere
/// blankness, because blank is only the loudest way of being relative, and
/// PowerShell leaves `$env:APPDATA = ""` behind when somebody clears it.
///
/// Compiled on both platforms and gated only at its caller, so that a machine
/// of either kind can prove both rules. This is string arithmetic with no
/// filesystem in it, and the Unix rule is the one most likely to be broken by
/// somebody who cannot run it.
#[cfg_attr(
    unix,
    // `#[allow]` and not `#[expect]`: the condition is a `cfg`, so on the
    // platform this *is* used an expectation would be unfulfilled. The crate
    // root's module docs have the rule and why it is written down.
    allow(dead_code, reason = "the other platform's rule, tested on both")
)]
pub(crate) fn from_appdata(appdata: Option<PathBuf>) -> Option<PathBuf> {
    Some(appdata.filter(|dir| dir.is_absolute())?.join(DIR))
}

/// Unix's answer, over the two variables handed in rather than read.
///
/// `XDG_DATA_HOME` when it is set to something absolute, and `~/.local/share`
/// otherwise, which is the fallback the specification names rather than abeam's
/// own invention. Both are held to the absoluteness rule above, and the home
/// directory is the reason it is applied twice rather than once: a container or
/// a service unit can export an empty `HOME`, and `.local/share/abeam` resolved
/// against `/` is a directory belonging to nobody that root can write.
///
/// A **relative** `XDG_DATA_HOME` falls through to `HOME` rather than ending
/// the search, which is the one place this differs from simply refusing.
/// `crate::config::from_xdg` makes the whole argument and it carries over
/// without a word changed: the variable is discarded either way, so the only
/// question left is whether one bad variable costs the user their text, and
/// the specification's own instruction is to consider a relative path invalid
/// and ignore it.
#[cfg_attr(
    windows,
    // `#[allow]` and not `#[expect]`: the condition is a `cfg`, so on the
    // platform this *is* used an expectation would be unfulfilled. The crate
    // root's module docs have the rule and why it is written down.
    allow(dead_code, reason = "the other platform's rule, tested on both")
)]
pub(crate) fn from_xdg(data: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = match data.filter(|dir| dir.is_absolute()) {
        Some(data) => data,
        None => home
            .filter(|dir| dir.is_absolute())?
            .join(".local")
            .join("share"),
    };
    Some(base.join(DIR))
}
