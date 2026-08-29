//! Comparing two paths that came from different places.
//!
//! Three sources of paths meet inside abeam and no two of them agree about how
//! to write a directory down. `std::env::current_dir` and `notify` hand back
//! the platform's own spelling, which on Windows means backslashes. Claude
//! writes into its session records whatever its own process was handed. And
//! `git worktree list --porcelain` prints **forward slashes on Windows** — this
//! is real output from this repository, on this machine:
//!
//! ```text
//! worktree C:/Users/philm/PycharmProjects/forge
//! HEAD 00b21a4321d91cefaa9f26c721a71c9c3001af4b
//! branch refs/heads/hand-the-command-line-to-the-agent
//! ```
//!
//! Every comparison the worktree feature makes crosses at least two of those
//! three, and a comparison that crossed them by `==` would answer that the
//! repository abeam is standing in is not the repository git just described.
//! So the rule lives in one module and is written down once, rather than being
//! re-derived — slightly differently each time — at every call site that needs
//! it.
//!
//! ## Three questions, one rule
//!
//! [`same_dir`] asks whether two spellings name the *same* directory.
//! [`under`] asks whether one is *inside* the other. The second is emphatically
//! not the first with a `starts_with` in front of it — see [`under`] for the
//! sibling directory that a byte prefix swallows — but they are answered by one
//! function, [`parts`], and that is a decision rather than tidiness.
//!
//! [`workspace_key`] is the third, and it is the same question wearing a hat:
//! how do you write a directory down as a *file name*, so that state kept per
//! workspace can be found again next week. A name derived from a spelling
//! changes when the spelling changes, so it is derived from [`parts`] like the
//! other two — a second normalisation living next to whichever pane needed one
//! would have to agree with this one about case, about `/` against `\`, and
//! about the trailing separator, for ever, in two places.
//!
//! This module came out of `crate::agentstate`, where the rule was a byte
//! comparison of one spelled string and there was only ever one question to ask
//! of it. Adding `under` beside it as a *second* rule lasted about an hour. A
//! component comparison and a string comparison disagree about a path with a
//! redundant `.` in it — `C:\repo\.` is inside itself under one and not the
//! same directory as itself under the other — and `abeam .` is a real command
//! somebody types. The failure that buys is not a wrong pane: it is
//! `crate::app`'s router finding an owner for every path and matching none of
//! them, so the watcher goes quiet and stays quiet, which looks exactly like
//! the feature having been deleted. One rule cannot disagree with itself.
//!
//! ## Why spellings and not `fs::canonicalize`
//!
//! That was `crate::agentstate`'s decision and it holds harder here.
//! Canonicalising touches the disk, answers on Windows with a `\\?\` path that
//! then has to be undone, and [`under`] is asked once per watched path per
//! known workspace — which under a `git checkout` is thousands of stat calls to
//! answer a question about two strings.
//!
//! That is an argument about *comparing*, and it does not reach [`resolve_root`],
//! which canonicalises exactly once for the whole session. The two decisions
//! look contradictory and are not: one is about the cost of a rule asked
//! thousands of times a second, and the other is about the one spelling every
//! one of those comparisons starts from. See [`resolve_root`] for what a
//! junction does to a session that skips it.
//!
//! ## Two things this rule does not do
//!
//! Both are worth naming, because both are places somebody could later reach
//! for a fix that would make the module disagree with itself.
//!
//! **`..` is not normalised.** [`parts`] compares the components a path is
//! written with, and `Path::components` folds `.` away and leaves `..` where it
//! is — so `under(C:\a, C:\a\..\b\x.md)` is `true`, and a containment rule can
//! in principle be walked out of. Nothing abeam has emits a `..`: `notify`
//! reports absolute resolved paths, `git worktree list` prints absolute ones,
//! a session record carries the agent's own `cwd`, and [`resolve_root`] has
//! already flattened the one path a person types. So it is unreachable today
//! and it is not unreachable by construction, which is the difference worth
//! writing down. Resolving `..` textually is *wrong* in the presence of
//! symlinks — `a/link/..` is not `a` — so the fix, if a source ever emits one,
//! is to resolve it at the source rather than to teach this rule a shortcut.
//!
//! **`\\?\C:\repo` compares unequal to `C:\repo`.** The verbatim prefix is a
//! different `Prefix` to the platform's own parser, so it is a different first
//! component and therefore a different directory here. That is consistent with
//! refusing to canonicalise per comparison — a rule that quietly accepted both
//! spellings would be half a canonicalisation, applied to the prefix and to
//! nothing else — and it is why [`resolve_root`] strips the prefix back off
//! rather than handing `\\?\`-shaped paths to the rest of abeam.

use std::path::{Path, PathBuf};

/// Whether two paths name the same directory, as far as comparing two
/// spellings can tell.
///
/// The sides arrive by different routes — abeam's from
/// `std::env::current_dir`, Claude's from whatever its own process was handed,
/// git's from `git worktree list` — so they agree about the directory and not
/// always about how to write it down. Which of those disagreements is still
/// the same *place* is a fact about the filesystem underneath, and that is
/// [`parts`]'s question rather than this one's.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    parts(a) == parts(b)
}

/// Whether `path` is inside `root`, under the same platform rule [`same_dir`]
/// uses.
///
/// **Components, not bytes, and that is the whole of this function.** A byte
/// prefix test says that `/home/me/forgery` is inside `/home/me/forge`, and
/// that `C:\work\forge-old` is inside `C:\work\forge`. Those are two different
/// checkouts, usually two different branches of the same project, and the
/// mistake is invisible: the wrong one simply refreshes whenever the right one
/// is written to. Comparing the pieces a path is made of asks whether the
/// names match, not whether the letters do, and `forge` is not `forgery`.
/// There is a test for exactly that pair below.
///
/// Reflexive: a directory is inside itself. That is a decision rather than an
/// accident of the implementation. `notify` reports events on the watched
/// directory itself, and `crate::workspace::owner` answers with the *longest*
/// root that contains a path — so if a root did not contain itself, a change
/// reported against a workspace root would belong to that workspace's parent,
/// or to nobody. Both of those are the routing bug this module exists to fix,
/// arriving through the back door.
pub fn under(root: &Path, path: &Path) -> bool {
    parts(path).starts_with(&parts(root))
}

/// How long the readable half of a [`workspace_key`] is allowed to be.
///
/// Long enough to hold every repository name anybody actually types, short
/// enough that the whole key and the two directories above it stay well inside
/// the 260 characters a Windows path is held to unless every program in the
/// chain has opted out of it.
const PREFIX: usize = 32;

/// The file name a workspace root's own state is written down under.
///
/// abeam keeps one thing per workspace outside the workspace —
/// `crate::panes::pad::store`'s scratch pad — in a directory shared with every
/// other workspace on the machine, so the root has to become a name. That is
/// this module's question rather than the pad's, and the reason it is answered
/// here rather than three lines from the caller is the failure a second answer
/// would have: `C:\Repo` and `c:\repo` are one directory to [`same_dir`], and a
/// naming rule written elsewhere would have to keep agreeing with [`parts`]
/// about case, separators and the trailing `\` in order to say so. The first
/// time it stopped agreeing, the pad somebody typed yesterday would not be
/// there today — no error, no message, an empty pane that looks exactly like a
/// pad they never wrote and a file on disk that nothing will ever open again.
///
/// Built from [`parts`], so it inherits the platform rule whole: case-folded
/// and separator-agnostic on Windows, exact on Unix.
///
/// One thing that inheritance does not cover, and it is named because the
/// failure it would make is exactly the one this function exists to prevent.
/// The Windows half of [`parts`] folds *ASCII* case, and Windows itself folds
/// more than that: `C:\Projekt-Ärger` and `C:\projekt-ärger` are one
/// directory to that filesystem and two keys here, so a pad written under one
/// spelling would not be there under the other. Nothing can reach it today,
/// because every root in this program has been through [`resolve_root`] first
/// and that answers with the spelling the disk itself holds — one spelling in,
/// one key out. Unreachable by that route rather than by construction, which is
/// the distinction worth having on the page: a caller that ever asked this
/// about a root which had not been resolved would get the "typed yesterday, not
/// there today" this module is written against. Folding the rest would mean
/// carrying Windows' own case table, which is a larger and more breakable thing
/// than the guarantee already in place.
///
/// ## The shape, and which half of it means anything
///
/// A readable prefix, a `-`, and sixteen hex digits. The prefix is the root's
/// last component with everything a file name should not carry taken out of it,
/// and it exists for the person who opens that directory and wants to know
/// which of these is theirs. Nothing reads it back and nothing compares it; two
/// roots whose last component is the same word are told apart entirely by the
/// other half. The hash is the half that has to be right.
///
/// ## Not `DefaultHasher`, and not as a matter of taste
///
/// [`std::collections::hash_map::DefaultHasher`] documents in as many words
/// that its output is not guaranteed to be stable across Rust releases, which
/// makes it the wrong number to write on somebody's disk. A toolchain bump
/// would rename every pad on the machine at once — each of them still there,
/// none of them reachable, every pane opening empty — and the person it
/// happened to would have nothing on screen to connect the two events.
///
/// FNV-1a is six lines, it is the same six lines it was in 1991, and it can be
/// read back off this page by anybody who needs to find a file by hand. Nothing
/// here is defending against an adversary — a collision costs two workspaces
/// one shared pad rather than anything a person could aim — and sixty-four bits
/// puts that far past the number of directories a machine has.
///
/// ## The separator inside the hash
///
/// Feeding the components in end to end with nothing between them makes
/// `/a/bc` and `/ab/c` the same run of bytes and therefore the same file: two
/// unrelated checkouts sharing one pad, each overwriting the other's notes on
/// every keystroke. A zero byte cannot appear inside a path component on either
/// platform, so it separates them and can never be mistaken for one. There is a
/// test for exactly that pair below.
pub fn workspace_key(root: &Path) -> String {
    let parts = parts(root);

    // The last component, in the one alphabet Windows, Linux and macOS all
    // agree is safe in a file name. A run of bytes outside it collapses to a
    // single `-` rather than becoming one `-` each, because a directory named
    // in a script that is not Latin would otherwise come out as thirty-two
    // dashes — noise, where the whole point of this half was recognising the
    // place at a glance.
    let mut prefix = String::new();
    for byte in parts.last().map_or(&[][..], |part| part.as_slice()) {
        let byte = byte.to_ascii_lowercase();
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        {
            prefix.push(char::from(byte));
        } else if !prefix.ends_with('-') {
            prefix.push('-');
        }
        if prefix.len() >= PREFIX {
            break;
        }
    }

    // `/` has one component and nothing of it survives the alphabet above, and
    // a path with no components at all has nothing to survive; both would
    // otherwise open the file name with the separator or with nothing.
    let prefix = match prefix.trim_matches('-') {
        "" => "workspace",
        trimmed => trimmed,
    };

    // A key therefore always ends in a hex digit, which quietly settles two
    // Windows rules the prefix alone could break: a file name may not end in a
    // `.`, and `con`, `nul` and `com1` are devices rather than names.
    format!("{prefix}-{:016x}", fnv1a(&parts))
}

/// FNV-1a, 64 bits, over the components with a zero byte after each.
///
/// Hand-rolled for [`workspace_key`]'s reason: this number is part of a file
/// name that has to mean the same thing after the next `rustup update`, and the
/// standard library's hasher promises the opposite of that.
fn fnv1a(parts: &[Vec<u8>]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    for part in parts {
        for byte in part.iter().chain(std::iter::once(&0)) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}

/// The directory abeam is standing in, spelled the way git will spell it.
///
/// Called **once, at startup**, on the answer `std::env::current_dir` gave, and
/// the result is the root everything downstream is built from: the watcher, the
/// first workspace, the git pane, the reader, the probe, and the working
/// directory the agent's pty is opened in. One resolution, one spelling, and
/// nothing left holding the other one.
///
/// ## The failure this closes, which is a Windows failure
///
/// `GetCurrentDirectoryW` reports the path the process was *given*. It does not
/// resolve a junction, a `subst` drive or an 8.3 short name. `git worktree list
/// --porcelain` resolves all three. So a session started through a junction —
/// this is real output, from a real shell, on this machine:
///
/// ```text
/// > cmd /c "cd /d C:\…\junc\link && cd && git rev-parse --show-toplevel"
/// C:\…\junc\link
/// C:/…/junc/real
/// ```
///
/// — is standing in `…\link` while every root git names is `…/real`, and
/// **those are two different directories to every comparison in this module.**
/// That is not a cosmetic disagreement. `crate::app::workspace_roots` then holds
/// one root that contains the watcher's events (the agent's own, spelled the
/// link's way) and a set of git's that contain none of them, so
/// `crate::workspace::owner` hands *every* watched path to the agent's
/// workspace — including the paths inside a neighbouring agent's worktree, which
/// no longer have an innermost root to belong to. `is_evidence` has nothing to
/// suppress, and the bug two commits exist to close is silently reinstated. The
/// agent's own workspace also appears twice in that list, under two spellings,
/// and no row in the `w` list is marked `here` or `agent_here` — so the list
/// that switches workspaces says you are nowhere.
///
/// Unix is unaffected and still goes through this. `getcwd(3)` resolves
/// symlinks, so `current_dir` and git already agree there, and canonicalising a
/// path that is already canonical is one `realpath` call for the whole process.
/// One code path that is right everywhere beats two, one of which is only ever
/// exercised by half the machines — `crate::agentstate::roster` makes the same
/// argument about `crate::launch`.
///
/// ## Why this is not the `fs::canonicalize` this module refuses
///
/// The module header's refusal is a *cost* argument about [`under`], which is
/// asked once per watched path per known workspace — thousands of stat calls
/// under a `git checkout`, to answer a question about two strings. This is one
/// call, at startup, on one path. The rule for comparing is untouched: what
/// changes is the spelling that goes into it.
///
/// ## What comes back, and what is undone before it does
///
/// On Windows `fs::canonicalize` answers with a verbatim path — `\\?\C:\…`, or
/// `\\?\UNC\server\share\…` for a network path. [`parts`] reads that prefix as
/// a different first component, so a verbatim root would be a *third* spelling
/// that matches neither git's nor the watcher's, which is the same bug wearing
/// a different hat. So the prefix is taken back off, and the UNC form is
/// respelled as `\\server\share\…` rather than being mangled into a path
/// beginning with a directory called `UNC`.
///
/// A verbatim prefix with no ordinary spelling — `\\?\Volume{…}`, a volume
/// mounted with no drive letter — is left exactly as it is. There is nothing to
/// rewrite it to, and a wrong guess is worse than a long path.
///
/// **Any failure answers with the path it was given.** A directory that cannot
/// be opened, a permission error, a platform that will not say: a worse
/// spelling is better than not starting, and the behaviour that falls back to is
/// precisely the behaviour of every abeam before this function existed.
///
/// ## What somebody sees change
///
/// One thing, and it is worth being honest about it rather than describing this
/// as free. A person who works on a `subst` drive — `X:` standing for
/// `C:\src\forge` — gets an agent whose own `pwd` says `C:\src\forge`, because
/// that is the directory abeam hands its pty. The alternative is the spelling
/// they typed and a window that cannot tell their worktrees apart, and between
/// a name that surprises somebody once and a routing rule that is wrong all
/// session, the name is the cheaper of the two.
pub fn resolve_root(root: &Path) -> PathBuf {
    match std::fs::canonicalize(root) {
        Ok(resolved) => plain(resolved),
        Err(_) => root.to_path_buf(),
    }
}

/// A canonical path written the way the rest of the machine writes one.
///
/// See [`resolve_root`]. Split out so the respelling can be tested on paths
/// this machine does not have to have — a UNC share, a bare volume GUID — which
/// is most of what there is to get wrong here.
#[cfg(windows)]
fn plain(path: PathBuf) -> PathBuf {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};

    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return path;
    };

    // Built as `OsString` rather than through `to_string_lossy`, for the reason
    // the Unix half of `parts` gives: a lossy conversion of a name that is not
    // valid Unicode comes back as replacement characters, and this is a path
    // abeam is about to spawn a child in.
    let mut head = OsString::new();
    match prefix.kind() {
        Prefix::VerbatimDisk(letter) => head.push(format!("{}:\\", letter as char)),
        Prefix::VerbatimUNC(server, share) => {
            head.push(r"\\");
            head.push(server);
            head.push("\\");
            head.push(share);
            head.push("\\");
        }
        // Already ordinary, or verbatim with nothing to be ordinary as.
        _ => return path,
    }

    // The root component belongs to the prefix and has been written back above;
    // keeping it would leave a `\` between `C:\` and the first directory.
    Path::new(&head).join(
        parts
            .filter(|part| !matches!(part, Component::RootDir))
            .collect::<PathBuf>(),
    )
}

/// Nothing to undo. `realpath(3)` answers with an ordinary absolute path, which
/// is the spelling everything else on this platform already uses.
#[cfg(unix)]
fn plain(path: PathBuf) -> PathBuf {
    path
}

/// The one form of a path this module compares, for the platform it was built
/// for, cut into the pieces a path is actually made of.
///
/// There is no rule that is right on both, and the reason to insist on that
/// rather than pick the more forgiving one is that the two mistakes cost
/// different amounts. Windows folds case, reads `/` and `\` as one separator,
/// and calls `C:\` and `C:` the same place; a comparison there that did none of
/// those would refuse a record that is genuinely ours, and the queue would
/// drain by hand for no reason anybody could see. Unix does none of the three.
/// `/home/phil/Work` and `/home/phil/work` are two directories, `\` is
/// an ordinary byte in a file name rather than a separator, and a rule that
/// folded either of those would be declaring two different places equal — in
/// the one function that decides whether a queued prompt may be typed into an
/// agent. That mistake sends somebody's prompt to a session in another
/// checkout, and it is not one they would see happen.
///
/// macOS is the awkward middle: its kernel distinguishes case and its default
/// volume does not. It takes the strict reading with the rest of Unix, because
/// being strict costs a queue that drains by hand and being loose costs the
/// paragraph above.
///
/// ## Why the pieces rather than the string
///
/// [`Path::components`] does three things by hand-rolling standards, and each
/// of them is a bug this used to have or would have had.
///
/// It knows where a component *begins*, which is what stops `/home/me/forge`
/// containing `/home/me/forgery`. It drops a trailing separator, which is a
/// spelling and not a place — but, unlike the `trim_end_matches` this replaced,
/// it does not take `/` itself down to the empty string, and an empty string is
/// what a record carrying no `cwd` comes out as. A root equal to a record that
/// names nowhere, in the comparison deciding whether a prompt may be typed, is
/// not a hypothetical: a Claude running at `/` is ordinary in a container. And
/// on Windows the drive prefix and the root are two components written as one
/// run of characters, and `\\server\share` is one component containing three
/// separators — only the platform's own parser gets either right.
#[cfg(windows)]
fn parts(path: &Path) -> Vec<Vec<u8>> {
    let spelled = path
        .to_string_lossy()
        .replace('/', "\\")
        // `C:\` and `C:` both come out as `C:`, which is what this platform
        // calls the same directory. `Path::components` would keep them apart —
        // it reads the first as a drive root and the second as drive-relative —
        // so this one piece of normalisation happens before it and not after.
        .trim_end_matches('\\')
        .to_ascii_lowercase();

    Path::new(&spelled)
        .components()
        // Lossless: the string above was built with `to_string_lossy`, so
        // whatever survived that survives this.
        .map(|part| part.as_os_str().to_string_lossy().into_owned().into_bytes())
        .collect()
}

#[cfg(unix)]
fn parts(path: &Path) -> Vec<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;

    // Bytes rather than `to_string_lossy`, because a Unix path is bytes and not
    // text: two different names that are not UTF-8 both come back from a lossy
    // conversion as the same run of replacement characters, and this is the
    // comparison where calling two different directories equal is the answer
    // that costs something. Nothing is respelled first — there is no case to
    // fold and no separator to swap — so this side is exact.
    path.components()
        .map(|part| part.as_os_str().as_bytes().to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A repository, and the neighbour it must never be confused with: one
    /// whose *name begins with its own*.
    ///
    /// Spelled for the platform running the test rather than written once and
    /// asserted everywhere, for the reason `crate::agentstate`'s own table
    /// gives: these are places, and which spellings name the same place is not
    /// the same question on the two platforms. A test that used Windows paths
    /// on Linux would pass through the case-sensitive comparison without ever
    /// touching it.
    #[cfg(windows)]
    const ROOT: &str = r"C:\Users\philm\PycharmProjects\forge";
    #[cfg(windows)]
    const SIBLING: &str = r"C:\Users\philm\PycharmProjects\forgery";
    #[cfg(unix)]
    const ROOT: &str = "/home/philm/PycharmProjects/forge";
    #[cfg(unix)]
    const SIBLING: &str = "/home/philm/PycharmProjects/forgery";

    /// Every way the two sides might have written [`ROOT`] down, each with
    /// whether this platform's filesystem calls it the same directory.
    ///
    /// Lifted from `crate::agentstate`, where it guarded the session record's
    /// `cwd`, because it is guarding the same rule — and it now has two more
    /// sources of spellings to survive. Git's, which on Windows is the
    /// forward-slash row. And a person's: `abeam .` resolves to a root with a
    /// `.` on the end of it, which is the row that proved [`same_dir`] and
    /// [`under`] could not be two rules.
    #[cfg(windows)]
    fn spellings() -> Vec<(String, bool)> {
        vec![
            (ROOT.to_string(), true),
            (ROOT.to_ascii_lowercase(), true),
            (ROOT.to_ascii_uppercase(), true),
            (ROOT.replace('\\', "/"), true),
            (format!("{ROOT}\\"), true),
            (format!("{}/", ROOT.replace('\\', "/")), true),
            (format!(r"{ROOT}\."), true),
            (
                ROOT.replace(r"\PycharmProjects\", r"\\PycharmProjects\\"),
                true,
            ),
        ]
    }

    #[cfg(unix)]
    fn spellings() -> Vec<(String, bool)> {
        vec![
            (ROOT.to_string(), true),
            (format!("{ROOT}/"), true),
            (format!("{ROOT}/."), true),
            (
                ROOT.replace("/PycharmProjects/", "//PycharmProjects//"),
                true,
            ),
            // Two directories, and this filesystem will happily hold both —
            // which is what the capital letters in [`ROOT`] are there for.
            (ROOT.to_ascii_lowercase(), false),
            (ROOT.to_ascii_uppercase(), false),
            // Not a separator here. `\` is an ordinary byte in a file name, so
            // this names one file with a very odd name and not a path at all.
            (ROOT.replace('/', "\\"), false),
        ]
    }

    #[test]
    fn one_directory_written_two_ways_is_still_one_directory() {
        for (spelling, is_the_same) in spellings() {
            assert_eq!(
                same_dir(Path::new(&spelling), Path::new(ROOT)),
                is_the_same,
                "`{spelling}` should {}be the same directory as `{ROOT}`",
                if is_the_same { "" } else { "not " }
            );
        }
    }

    #[test]
    fn a_sibling_whose_name_merely_starts_the_same_is_not_inside() {
        // The failure this whole function exists for. `/home/me/forgery`
        // begins, byte for byte, with `/home/me/forge` — so a `starts_with` on
        // the strings routes every write in one checkout to the pane watching
        // the other, silently and for ever.
        assert!(!under(Path::new(ROOT), Path::new(SIBLING)));
        assert!(!under(Path::new(SIBLING), Path::new(ROOT)));
        assert!(!same_dir(Path::new(ROOT), Path::new(SIBLING)));

        // ...and a file inside the sibling is no better disguised.
        let inside_sibling = Path::new(SIBLING).join("src").join("main.rs");
        assert!(!under(Path::new(ROOT), &inside_sibling));
        assert!(under(Path::new(SIBLING), &inside_sibling));
    }

    #[test]
    fn a_nested_worktree_is_inside_the_repository_that_holds_it() {
        // The shape this feature is about: Claude Code puts its worktrees under
        // `.claude/worktrees/<name>` in the repository it was started in, so
        // every path in one of them really is inside the outer root. That is
        // why a prefix test cannot be the routing rule and
        // `crate::workspace::owner` has to take the longest match instead.
        let root = PathBuf::from(ROOT);
        let nested = root.join(".claude").join("worktrees").join("other");
        let note = nested.join("NOTES.md");

        assert!(under(&root, &nested));
        assert!(under(&root, &note), "the outer root really does contain it");
        assert!(under(&nested, &note));
        assert!(!under(&note, &nested));
    }

    #[test]
    fn the_two_questions_are_one_rule_and_cannot_drift_apart() {
        // Two paths each inside the other are the same directory. Anything else
        // means `same_dir` and `under` have become two rules, which is a state
        // this module has already been in: `crate::app` finds an owner for
        // every path and matches none of them, so the watcher goes quiet and
        // nothing on screen says why.
        for (spelling, is_the_same) in spellings() {
            let spelled = PathBuf::from(&spelling);
            assert_eq!(
                under(&spelled, Path::new(ROOT)),
                is_the_same,
                "`{spelling}` against `{ROOT}`"
            );
            assert_eq!(
                under(&spelled, Path::new(ROOT)) && under(Path::new(ROOT), &spelled),
                same_dir(&spelled, Path::new(ROOT)),
                "`{spelling}` is inside `{ROOT}` and vice versa, or neither"
            );
        }
    }

    #[test]
    fn the_filesystem_root_contains_everything_under_it() {
        // A Claude running at `/` is not a hypothetical in a container, and
        // trimming the spelled form's trailing separator by hand gets this one
        // wrong: `/` becomes the empty string, which is also what a record
        // carrying no `cwd` comes out as, so the root would be equal to a
        // record that names nowhere.
        #[cfg(windows)]
        let (top, below) = (Path::new(r"C:\"), Path::new(r"C:\Users\philm"));
        #[cfg(unix)]
        let (top, below) = (Path::new("/"), Path::new("/home/philm"));

        assert!(under(top, below));
        assert!(under(top, top));
        assert!(!under(below, top));
        assert!(!same_dir(top, Path::new("")), "nowhere is not the root");
        assert!(!under(top, Path::new("")));
    }

    // --- the name a root is written down under -----------------------------

    #[test]
    fn one_directory_written_two_ways_is_written_down_under_one_name() {
        // The same table as `same_dir`'s, asserted against the same answers,
        // because a key that disagreed with it would be a second rule — and
        // the cost of the disagreement is a pad that was typed yesterday and
        // is not there today.
        for (spelling, is_the_same) in spellings() {
            assert_eq!(
                workspace_key(Path::new(&spelling)) == workspace_key(Path::new(ROOT)),
                is_the_same,
                "`{spelling}` and `{ROOT}` should {}be written down the same way",
                if is_the_same { "" } else { "not " }
            );
        }
    }

    #[test]
    fn two_roots_that_differ_only_in_where_the_separator_falls_are_two_names() {
        // With the components fed to the hash end to end, these two are the
        // same three bytes — so two unrelated checkouts would share one file,
        // and each would overwrite the other's notes on every save.
        #[cfg(windows)]
        let (one, other) = (Path::new(r"C:\a\bc"), Path::new(r"C:\ab\c"));
        #[cfg(unix)]
        let (one, other) = (Path::new("/a/bc"), Path::new("/ab/c"));

        assert_ne!(workspace_key(one), workspace_key(other));
    }

    #[test]
    fn a_key_is_one_file_name_and_one_a_filesystem_will_take() {
        // Everything awkward a root can be: a name with spaces and brackets in
        // it, a name in another script, the top of a drive, and the path that
        // has no components at all.
        #[cfg(windows)]
        let roots = [
            ROOT.to_string(),
            r"C:\".to_string(),
            r"C:\Users\philm\My Repo (2)\проект".to_string(),
            String::new(),
        ];
        #[cfg(unix)]
        let roots = [
            ROOT.to_string(),
            "/".to_string(),
            "/home/philm/My Repo (2)/проект".to_string(),
            String::new(),
        ];

        let keys: Vec<String> = roots
            .iter()
            .map(|root| workspace_key(Path::new(root)))
            .collect();

        for (root, key) in roots.iter().zip(&keys) {
            assert!(!key.is_empty(), "`{root}` was written down as nothing");
            assert!(
                key.bytes().all(|byte| byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')),
                "`{key}` is not a name every filesystem here will take"
            );
            // One component, so a root can never name a directory of its own or
            // climb out of the one it is put in.
            assert_eq!(
                Path::new(key).components().count(),
                1,
                "`{key}` is more than a file name"
            );
            assert!(
                key.len() <= PREFIX + 1 + 16,
                "`{key}` is longer than it may be"
            );
            // Windows refuses a name ending in either, and the hash is what
            // guarantees neither can be the last character.
            assert!(!key.ends_with('.') && !key.ends_with(' '), "`{key}`");
        }

        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "two of {keys:?} are one file");
    }

    #[test]
    fn a_root_is_written_down_under_the_same_name_it_was_last_year() {
        // The number this test exists to freeze. `DefaultHasher` was refused
        // because its output may change between Rust releases, and a key that
        // changed under somebody would leave every pad they have written on
        // disk and unreachable at once — so the value is written down here,
        // where changing it has to be deliberate and has to be explained.
        //
        // Per platform, because `parts` is per platform: Windows folds the
        // case of a root that Unix keeps.
        #[cfg(windows)]
        assert_eq!(workspace_key(Path::new(ROOT)), "forge-545f840039f981c1");
        #[cfg(unix)]
        assert_eq!(workspace_key(Path::new(ROOT)), "forge-94b2adccc4d44b8a");
    }

    #[test]
    fn the_hash_is_the_fnv1a_that_everybody_elses_is() {
        // The two constants, against the reference implementation's published
        // vectors rather than against whatever this file last produced. A typo
        // in either of them is the same failure `DefaultHasher` was refused
        // over — every pad on the machine renamed at once — arriving from
        // inside the house instead of from a toolchain bump, and a test that
        // pinned this function's own output would agree with the typo.
        //
        // The empty input under FNV-1a 64 is the offset basis itself, and
        // `foobar` is `0x85944171f73967e8`. What this function hashes is each
        // component followed by a zero byte, so the second of those is that
        // vector plus one more round of the same two lines.
        assert_eq!(fnv1a(&[]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(&[b"foobar".to_vec()]), 0x3453_1ca7_168b_8f38);
    }

    // --- the one spelling everything else starts from ----------------------

    #[test]
    fn a_root_that_cannot_be_resolved_is_the_root_it_was_given() {
        // The fallback, and it is the whole of what `resolve_root` promises on
        // a bad day: a directory that is gone, a permission error, a platform
        // that will not answer. A worse spelling is better than not starting,
        // and what it falls back to is what every abeam before this function
        // did unconditionally.
        //
        // Hung off a directory that really exists, so that the one thing this
        // cannot resolve is the one component that is missing. Built from a
        // literal — some machine's home directory — it would pass just as
        // cheerfully on a machine where *none* of the path is there, which is
        // the same assertion proving nothing.
        let dir = crate::testutil::TempDir::new("paths-fallback");
        let nowhere = dir.path().join("no-such-directory-abeam-ever-made");
        assert_eq!(resolve_root(&nowhere), nowhere);
    }

    /// The other half of the promise, and the half a five-letter user name
    /// hides.
    ///
    /// `std::env::temp_dir` answers with `%TEMP%`, and on a Windows machine
    /// whose user name is longer than eight characters that is an 8.3 short
    /// name — `C:\Users\RUNNER~1\AppData\Local\Temp` on every GitHub runner,
    /// where `fs::canonicalize` and git both say `runneradmin`. That is the
    /// disagreement [`resolve_root`] exists to end, and it is the same one a
    /// junction makes; it simply arrives without anybody having to build it.
    ///
    /// So the spelling this starts from is taken from the platform rather than
    /// from `crate::testutil::TempDir`, whose whole job is now to have resolved
    /// it already. Where the two forms coincide — most desktops — this still
    /// asserts something true and cannot fail, which is precisely why the
    /// spelling has to come from somewhere that has not been fixed up.
    #[test]
    fn a_root_resolves_to_the_one_spelling_everything_else_starts_from() {
        let dir = crate::testutil::TempDir::new("paths-resolve");
        let raw =
            std::env::temp_dir().join(dir.path().file_name().expect("a temp directory has a name"));

        let resolved = resolve_root(&raw);
        assert!(
            same_dir(&resolved, dir.path()),
            "{} is not {}",
            resolved.display(),
            dir.path().display()
        );
        assert!(resolved.is_absolute());
        assert!(resolved.is_dir(), "...and it is still the directory itself");

        // Idempotent, which is what "one resolution, one spelling, and nothing
        // left holding the other one" has to mean: everything downstream is
        // built from this answer, and an answer that resolved again to
        // something else would be two spellings after all.
        assert_eq!(resolve_root(&resolved), resolved);

        // The assertion that would fail if the verbatim prefix were left on:
        // `\\?\C:\…` is a different first component, so it would match neither
        // git's spelling nor the watcher's.
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "{} still carries the verbatim prefix",
            resolved.display()
        );
    }

    /// The respelling, on paths this machine does not have to have.
    ///
    /// `resolve_root` can only be tested against directories that exist, and
    /// the two shapes most worth getting right — a UNC share and a volume with
    /// no drive letter — are not on every machine that builds this. What they
    /// have in common is that they are pure string surgery once
    /// `fs::canonicalize` has answered, so they are tested as string surgery.
    #[test]
    #[cfg(windows)]
    fn a_canonical_path_is_respelled_the_way_the_rest_of_the_machine_writes_one() {
        let plain_of = |raw: &str| plain(PathBuf::from(raw)).to_string_lossy().into_owned();

        // The ordinary case: the prefix comes off and the drive root stays.
        assert_eq!(
            plain_of(r"\\?\C:\Users\philm\forge"),
            r"C:\Users\philm\forge"
        );
        assert_eq!(plain_of(r"\\?\C:\"), r"C:\");

        // The UNC form, which is the one a naive `strip_prefix(r"\\?\")` turns
        // into a path beginning with a directory called `UNC` — a directory
        // that is on no machine anywhere, so the failure is a root that names
        // nothing rather than a root that names the wrong thing.
        assert_eq!(
            plain_of(r"\\?\UNC\server\share\repo"),
            r"\\server\share\repo"
        );
        assert_eq!(plain_of(r"\\?\UNC\server\share"), r"\\server\share\");

        // Nothing to undo, and nothing to guess at. A volume mounted without a
        // drive letter has no ordinary spelling, and a wrong guess is worse
        // than a long path.
        for untouched in [
            r"C:\Users\philm\forge",
            r"\\server\share\repo",
            r"\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\repo",
        ] {
            assert_eq!(plain_of(untouched), untouched);
        }
    }

    /// The failure the whole function exists for, built rather than described.
    ///
    /// A real `mklink /J` junction over a real directory, and the two answers
    /// abeam has to reconcile: `GetCurrentDirectoryW` reports the link, and
    /// anything that opens the directory reports the target. A test that
    /// asserted this with two string constants would pass on a machine where
    /// junctions do not work at all.
    #[test]
    #[cfg(windows)]
    fn a_root_reached_through_a_junction_resolves_to_the_directory_git_will_name() {
        let dir = crate::testutil::TempDir::new("paths-junction");
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        std::fs::create_dir_all(&real).expect("create the target directory");

        let made = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&real)
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|out| out.status.success());
        assert!(
            made,
            "this test needs `mklink /J`, which needs no elevation"
        );

        // The disagreement, stated before it is fixed: these are two spellings
        // of one directory and this module calls them two directories — which
        // is correct, and is exactly why the resolution has to happen once, at
        // the top, rather than being smuggled into the comparison.
        assert!(!same_dir(&link, &real), "the fixture proves nothing");

        let resolved = resolve_root(&link);
        assert!(
            same_dir(&resolved, &real),
            "{} is not {}",
            resolved.display(),
            real.display()
        );
        // ...and an ordinary drive path rather than a verbatim one, because a
        // verbatim root is a third spelling that matches neither git's nor the
        // watcher's.
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "{} still carries the verbatim prefix",
            resolved.display()
        );
    }
}
