//! The worktrees of the repository on screen, and which of them owns a path.
//!
//! Claude Code does not stay in one directory. It makes git worktrees — the
//! usual place is `<root>/.claude/worktrees/<name>` — and runs agents in them,
//! so a machine with two agents on one project has two working trees inside
//! one watched directory. `crate::watch` runs a single recursive watch of the
//! repository root and is right to; what it cannot do on its own is say *whose*
//! change it just saw.
//!
//! Without an answer, it does not say. Another agent working in
//! `<root>/.claude/worktrees/other` refreshed this window's git pane on every
//! file it wrote and pulled its scratch markdown into this window's reader,
//! with nothing on screen admitting where any of it came from — a pane that
//! reports somebody else's work as yours is worse than a pane that reports
//! nothing, because it is not obviously broken.
//!
//! ## Why the obvious fix is not the fix
//!
//! Route by path prefix: take the event if it starts with the workspace root.
//! It does not work, and understanding why is the whole of this module.
//! `<root>/.claude/worktrees/other/NOTES.md` **has `<root>` as a prefix**. It
//! genuinely is inside the repository — that is what makes the worktree layout
//! convenient — so a prefix test routes it straight to the workspace rooted at
//! `<root>`, which is precisely the case being complained about. The naive fix
//! is a no-op dressed as a rule.
//!
//! The rule that works is **innermost ownership**: a path belongs to the
//! *longest* workspace root that contains it, and a pane takes an event only
//! when that longest root is its own. Given `{R, R/.claude/worktrees/other}`,
//! a write under the worktree belongs to the worktree, and a pane looking at
//! `R` drops it.
//!
//! That is not a convention invented here to make the bug go away — it is
//! git's own model. `git status` in the main worktree does not report a nested
//! worktree's modifications; the nested tree has its own index and its own
//! HEAD, and git treats the directory as belonging to it. A pane that mirrors
//! `git status` should agree with `git status` about whose changes those are.
//!
//! ## Why `.claude` is not simply added to the watcher's noise list
//!
//! One line in `crate::watch`'s `NOISE` would stop the events at the door and
//! close the bug this afternoon. It would also blind abeam inside its own
//! worktrees for ever: the next step of this work re-roots the right-hand panes
//! *into* those worktrees, and a noise entry means the watcher never wakes them
//! — the whole feature deleted to fix its routing. Noise is for directories
//! nobody wants to read. `.claude/worktrees` is where the work is.
//!
//! ## What is pure, and why
//!
//! [`parse_worktrees`], [`owner`] and [`rows`] touch nothing: no git, no
//! filesystem, no clock. Only [`discover`] starts a process. That is the same
//! split `crate::panes::git` makes and for the same reason — its parser tests
//! never shell out — and here it buys something extra, because the interesting
//! cases are a detached worktree, a bare repository and a nested one, and
//! creating those to test a `match` arm is minutes of git for microseconds of
//! parsing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::agentstate::Session;
use crate::paths;

/// One entry of `git worktree list`.
///
/// A faithful reading of what git prints rather than what today's callers
/// happen to want, on `crate::agentstate::Session`'s argument: the next person
/// to need `head` should find it parsed and tested rather than re-derive which
/// porcelain line carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    /// Absolute, as git spelled it — which on Windows means forward slashes.
    /// Never compared with `==`; see `crate::paths`.
    pub root: PathBuf,
    /// `refs/heads/x` reduced to `x`. `None` when the worktree is detached or
    /// bare, because git prints no `branch` line for either.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub bare: bool,
}

/// Parse `git worktree list --porcelain -z`.
///
/// ## The trap in this format
///
/// Records are separated by an **empty record**, not by a separator between
/// records:
///
/// ```text
/// worktree PATH\0HEAD sha\0branch refs/heads/x\0\0worktree PATH\0…\0\0
/// ```
///
/// `crate::panes::git` parses `git status -z` with
/// `out.split('\0').filter(|r| !r.is_empty())`, which is right there and reads
/// like the idiom to copy. Copied here it deletes the only thing that says one
/// worktree has ended and the next has begun, so every worktree in the
/// repository folds into one record whose `root` is the first path and whose
/// branch is the last branch. Nothing errors. The list simply has one entry in
/// it, and one entry is exactly what a repository with no extra worktrees
/// looks like — so the bug is invisible on the machine of anyone who has not
/// made a second worktree to look at.
///
/// The empty field is therefore load-bearing and is matched first, before
/// anything looks at what a field says.
///
/// An unfamiliar field is skipped rather than failing the parse: git already
/// prints `locked` and `prunable` on worktrees this does not ask about, and a
/// future git that adds a line should cost the reader nothing.
pub fn parse_worktrees(out: &str) -> Vec<Worktree> {
    let mut found = Vec::new();
    let mut open: Option<Worktree> = None;

    for field in out.split('\0') {
        if field.is_empty() {
            found.extend(open.take());
            continue;
        }

        // `key value`, or a bare key: `detached` and `bare` carry nothing, and
        // `locked` may carry a reason or nothing at all.
        let (key, value) = match field.split_once(' ') {
            Some((key, value)) => (key, Some(value)),
            None => (field, None),
        };

        match (key, value) {
            ("worktree", Some(path)) => {
                // A record that opens while one is still open means the
                // separator was lost. Closing the old one keeps the fields of
                // two worktrees from being merged into one, which is the same
                // failure the empty-field check above prevents, arriving by a
                // different route.
                found.extend(open.take());
                open = Some(Worktree {
                    root: PathBuf::from(path),
                    branch: None,
                    head: None,
                    detached: false,
                    bare: false,
                });
            }
            ("HEAD", Some(sha)) => {
                if let Some(worktree) = open.as_mut() {
                    worktree.head = Some(sha.to_string());
                }
            }
            ("branch", Some(reference)) => {
                if let Some(worktree) = open.as_mut() {
                    // `refs/heads/x` down to `x`. Only that prefix is stripped:
                    // a `branch` line naming anything else is a git abeam has
                    // not met, and showing the ref in full is a better answer
                    // than showing a confidently wrong short name.
                    worktree.branch = Some(
                        reference
                            .strip_prefix("refs/heads/")
                            .unwrap_or(reference)
                            .to_string(),
                    );
                }
            }
            ("detached", _) => {
                if let Some(worktree) = open.as_mut() {
                    worktree.detached = true;
                }
            }
            ("bare", _) => {
                if let Some(worktree) = open.as_mut() {
                    worktree.bare = true;
                }
            }
            _ => {}
        }
    }

    // Git ends its output with the separator, so this is normally a no-op. It
    // is here so that a last record without one is kept rather than dropped.
    found.extend(open.take());
    found
}

/// Which workspace a path belongs to: the **longest** root that contains it.
///
/// The module docs argue the rule; this is the whole of it. `None` means no
/// known workspace contains the path, which a caller should read as "not mine"
/// rather than as "everyone's" — the watcher only ever reports paths under a
/// root it was given, so `None` here means the roots are wrong.
///
/// "Longest" is decided with [`paths::under`] rather than by counting
/// components or characters, and that is deliberate. Every root that survives
/// the filter is an ancestor of the same path, so they are totally ordered by
/// containment, and asking "is `best` inside `root`" is the same comparison
/// that decided membership in the first place. A second measure of length —
/// bytes, components, anything — would be a second rule that could disagree
/// with the first about a trailing separator or a `C:/` against a `C:\`.
pub fn owner<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a Path> {
    roots
        .iter()
        .map(PathBuf::as_path)
        .filter(|root| paths::under(root, path))
        .reduce(|best, root| if paths::under(best, root) { root } else { best })
}

/// Whether a changed path says anything about the workspace that [`owner`]
/// gives it to.
///
/// Ownership alone is not enough, and the gap is not a corner case — it is the
/// ordinary shape of the very event this module exists to route. Writing one
/// file inside a nested worktree makes the watcher report the **parent
/// directories** as changed too: for a Claude worktree, `<root>/.claude` and
/// `<root>/.claude/worktrees` arrive in the same debounced batch as the file
/// itself. The file belongs to the worktree and is dropped correctly. Those two
/// directories belong to the *enclosing* workspace, because nothing nested is
/// an ancestor of them — so routed on ownership alone they are indistinguishable
/// from somebody editing in the root by hand, and the neighbour's write still
/// costs this window a frame and a `git status`. Which is the entire thing
/// ownership routing was built to stop.
///
/// So the rule has a second half: **a directory that contains another
/// workspace's root is not evidence about its own workspace.** The only way it
/// could have changed is that something inside the nested workspace did, and
/// that something has already been reported under its own name and routed to
/// the workspace that owns it.
///
/// A path that *is* a workspace root stays evidence about itself, which is the
/// first arm below and not a special case to be tidied away later: every root
/// contains a nested root somewhere below it, so without that arm the agent's
/// own root would be silenced the moment a worktree existed under it — the
/// routing bug arriving through the back door, in the one workspace that is
/// always on the list.
///
/// Nothing here touches the filesystem. It is two containment questions asked
/// of the same [`paths`] rule that decided ownership, so the two cannot drift
/// apart about a trailing separator or a `C:/` against a `C:\`.
///
/// ## What `roots` is, and how stale it can be
///
/// It is what [`discover`] last said, and `crate::app` asks about every ten
/// seconds (`WORKTREES_EVERY`). So this rule is exactly as current as that poll
/// and no more, and both directions of the lag are worth naming rather than
/// leaving to be found:
///
/// - **A worktree somebody just added is not on the list yet.** For up to ten
///   seconds its whole checkout has no innermost root of its own, so [`owner`]
///   hands every path in it to the enclosing workspace and this function calls
///   all of it evidence. That is the routing bug, for ten seconds, in a
///   worktree that has existed for ten seconds.
/// - **A worktree somebody just removed is still on it.** Its former parents go
///   on being suppressed for up to ten seconds — `<root>/.claude` is not
///   evidence about the root while anything nested is believed to be under it —
///   so a real edit there is dropped rather than misrouted.
///
/// Neither is fixable by being cleverer here, because both are the list being
/// out of date rather than the rule being wrong, and the rule has nothing but
/// the list. What makes them tolerable is that the *cost* of each is one poll
/// of the wrong answer in a workspace that has just changed shape, and that
/// `crate::panes::git`'s own two-second refresh catches up regardless. What
/// would not be tolerable is watching `.git/worktrees` to find out sooner:
/// `crate::watch` runs one recursive watch on purpose, and a second one to
/// shorten a ten-second window is the trade that module already declines.
pub fn is_evidence(roots: &[PathBuf], path: &Path) -> bool {
    if roots.iter().any(|root| paths::same_dir(root, path)) {
        return true;
    }
    !roots.iter().any(|root| paths::under(path, root))
}

/// Every worktree of the repository at `root`.
///
/// **Blocking** — it starts a process. Call it from a worker thread, never
/// from `Pane::tick` (`crate::pane`, and `docs/conpty-findings.md`
/// constraint 2).
///
/// An empty list is what every failure comes back as: no git on the machine,
/// no repository at `root`, a git too old for `-z` on this subcommand (it
/// arrived in 2.36). That is a safe answer rather than a lossy one *because of
/// how `crate::app` uses it*: the agent's own root is in the routing list
/// whether or not discovery ever answers, so an empty list routes every change
/// to the pane exactly as abeam did before any of this existed. A failure here
/// costs the nested-worktree fix and nothing else.
pub fn discover(root: &Path) -> Vec<Worktree> {
    match run(root, &["worktree", "list", "--porcelain", "-z"]) {
        Some(out) => parse_worktrees(&out),
        None => Vec::new(),
    }
}

/// Run git the way `crate::panes::git` runs it.
///
/// Copied rather than reinvented, down to the two settings that are not
/// obvious:
///
/// `GIT_OPTIONAL_LOCKS=0`, because the agent in the left pane is running real
/// git commands and abeam is polling beside it. Without this the two collide
/// over `.git/index.lock`, and the command that fails is the *agent's*.
///
/// `stdin(Stdio::null())`, because nothing here may ever prompt. This runs on
/// a worker thread with nowhere to type an answer, so a child that stopped to
/// ask one would hold the thread for as long as abeam is running.
///
/// `Command::new("git")` with a bare name matches `crate::panes::git` too, and
/// that is a deliberate refusal to be cleverer here than there. On Windows a
/// bare name reaching `CreateProcessW` is resolved against the current
/// directory before `PATH`, which is the hazard `crate::launch` exists to close
/// for `claude` — but abeam has one way of running git today, and two, differing
/// only in which module you happened to be reading, is worse than one. If that
/// resolution is worth hardening it is worth hardening in both places at once,
/// and `crate::panes::git` is where the decision lives.
fn run(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    // Lossy, and the loss is worth naming: a worktree whose path is not valid
    // UTF-8 — possible on Unix, where a path is bytes — comes back with
    // replacement characters in it and then matches nothing, so its writes fall
    // to the nearest enclosing root instead of to itself. That is the old
    // behaviour for that one worktree rather than a wrong answer, and the
    // alternative is an `OsString` parser that could not be a pure function
    // over `&str` and so could not be tested without git.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One worktree as a list of them would show it.
///
/// Rendered from a [`Worktree`] and everything abeam knows *about* being in it,
/// joined here so that the join is a pure function with a test rather than a
/// paragraph of conditionals inside a `render`.
/// The join is here rather than in the `render` that draws it for the reason
/// `crate::agentstate::Session`'s unread fields are kept: it is the part with
/// the decisions in it, it is pure, and it is tested. A `render` that finds this
/// waiting is a `render`; one that has to work out `here` against `agent_here`
/// for itself is where those two quietly become the same field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The branch name, or the directory's own name when there is no branch to
    /// use — a detached worktree has a name a person chose and a HEAD nobody
    /// can read at a glance.
    pub label: String,
    pub root: PathBuf,
    /// The right pane is on this workspace.
    pub here: bool,
    /// The hosted agent's own root. Distinct from `here` on purpose: the point
    /// of the list is to look at a workspace the agent is *not* in.
    pub agent_here: bool,
    /// Who is working in it, in Claude's own words — `"a1b2c3d4 · working"`.
    pub occupant: Option<String>,
    /// The one workspace the watcher can see changes in. Everything else on
    /// the list refreshes on the git pane's own timer, and saying so is the
    /// difference between a pane that is slow and a pane that looks broken.
    pub watched: bool,
}

/// Join what git said, what Claude said, and where abeam is standing.
///
/// Pure and I/O-free, like everything above [`discover`]. `at` is the
/// workspace the right pane is on and `agent_root` is the hosted agent's, and
/// the two really do differ now: the right pane can be pointed at a worktree
/// and the left one — a live child's pty — can never be moved at all.
///
/// ## Two rows that are always here, whatever git said
///
/// **`at` and `agent_root` each get a row, discovered or not**, and that is a
/// guarantee rather than a tidy-up. This list is *how the right pane is
/// switched* — `crate::panes::git` sends the selected row's root back to the
/// shell — so a workspace with no row on it is a workspace nobody can get back
/// to, and `crate::app` keeps `spaces[0]`, the agent's own root, for the whole
/// session. Built from discovery alone, switching away from a root git did not
/// name was a one-way trip, and no row was marked `here` or `agent_here`, so
/// the list also said you were nowhere.
///
/// Neither absence is exotic. **The agent's root is not a worktree root
/// whenever abeam was started in a subdirectory of the repository** — an
/// ordinary thing to do, and something `crate::panes::git` fully supports,
/// since it resolves `toplevel` for every open. `git worktree list` names the
/// repository, not the directory somebody was standing in. And `at` drops off
/// the list whenever `crate::app::sync_workspaces` retains a workspace git has
/// stopped naming because a child is still running in it, which is exactly the
/// moment the right pane may be pointed at it.
///
/// They are added in front rather than appended, so that the workspace you are
/// in and the one the agent is in are the first things on a list you opened to
/// find them, and git's own order is otherwise untouched. Nothing is added when
/// git named the directory itself: a discovered row carries a branch name and a
/// synthesised one can only carry a directory name, and the branch is the better
/// answer wherever it is available.
pub fn rows(
    worktrees: &[Worktree],
    roster: &[Session],
    agent_root: &Path,
    at: &Path,
    watch_root: Option<&Path>,
) -> Vec<Row> {
    let row_for = |root: &Path, label: String| Row {
        label,
        root: root.to_path_buf(),
        here: paths::same_dir(root, at),
        agent_here: paths::same_dir(root, agent_root),
        occupant: occupant_of(roster, root),
        // `under` rather than `same_dir`: one recursive watch of the agent's
        // root sees every worktree nested inside it, which is exactly the
        // layout Claude Code creates and exactly the case the routing above
        // exists for. A worktree somewhere else on the disk is out of its
        // reach.
        watched: watch_root.is_some_and(|watched| paths::under(watched, root)),
    };

    let mut rows: Vec<Row> = Vec::with_capacity(worktrees.len() + 2);
    for root in [agent_root, at] {
        let known = worktrees
            .iter()
            .any(|worktree| paths::same_dir(&worktree.root, root))
            // ...and against what has already been added, because `at` and
            // `agent_root` are the same directory in most sessions.
            || rows.iter().any(|row| paths::same_dir(&row.root, root));
        if !known {
            rows.push(row_for(root, dir_label(root)));
        }
    }
    rows.extend(
        worktrees
            .iter()
            .map(|worktree| row_for(&worktree.root, label_of(worktree))),
    );
    rows
}

/// What a worktree goes by in a list, and in the border of the pane looking at
/// it.
///
/// Public because `crate::app` names a workspace in two places where there is
/// no [`Row`] to read it off: the agent's own root, which exists before any
/// discovery has answered, and the reconciliation that folds a fresh discovery
/// into the workspaces already open. One rule for a worktree's name, so a
/// window cannot call the same directory two things.
pub fn label_of(worktree: &Worktree) -> String {
    if let Some(branch) = &worktree.branch {
        return branch.clone();
    }
    dir_label(&worktree.root)
}

/// The name a directory goes by when there is no branch to use it instead.
///
/// A detached or bare worktree still lives somewhere a person named, and so
/// does a repository git has not been asked about yet. Only a path with no last
/// component at all — a drive root, `/` — falls through to being written out in
/// full, and it is better to be long than nameless.
pub fn dir_label(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

/// Who is in this directory, and what they are doing, from the roster.
///
/// The word is Claude's rather than abeam's. `status` is one of `busy`,
/// `shell`, `idle`, `waiting` and `state` is one of `working`, `blocked`,
/// `failed`; `crate::agentstate::Session::readiness` is where both lists are
/// written down and where the precedence is argued, and this reads them in the
/// same order it does — `status` first, because that is the field an
/// interactive session's record is written with, and an interactive session is
/// what occupies a worktree somebody is working in. (`crate::panes::queue`
/// reads them the other way round, and is answering a different question: it
/// has an `id` in hand and wants the most descriptive word about *that*
/// dispatched agent, where this wants the word most likely to be there at all.)
///
/// Inventing a third vocabulary — "active", "in use" — was the alternative and
/// it is worse than it looks. It would mean a row saying "working" while the
/// agent's own record says `blocked`, which is the state a person is being
/// waited on in; there is no word abeam can coin that is more accurate than the
/// one the agent published.
///
/// The newest session wins where several name the same directory. `roster` is
/// `claude agents --json --all`, so it carries *finished* background agents
/// too, and a task that failed last Tuesday must not be reported as the
/// occupant of a worktree somebody is typing in now.
///
/// ## What is missing, and it is a name
///
/// The identifier here is the roster's short `id`, which a background agent
/// has and an interactive session does not — so the common case renders as the
/// bare status word. Claude's records and roster entries both carry a `name`
/// (`"forge-c5"`, or a dispatched task's own title) and it is the field this
/// wants; `crate::agentstate::Wire` does not parse it, and adding it reaches
/// outside the files this change owns. It is one field on `Wire`, one on
/// `Session`, and one line of a struct literal in `crate::panes::queue`'s
/// tests.
fn occupant_of(roster: &[Session], root: &Path) -> Option<String> {
    let session = roster
        .iter()
        .filter(|session| {
            session
                .cwd
                .as_deref()
                .is_some_and(|cwd| paths::same_dir(cwd, root))
        })
        .max_by_key(|session| session.started_at)?;

    let who = session.id.as_deref();
    let word = session.status.as_deref().or(session.state.as_deref());
    match (who, word) {
        (Some(who), Some(word)) => Some(format!("{who} · {word}")),
        (Some(who), None) => Some(who.to_string()),
        (None, Some(word)) => Some(word.to_string()),
        // A roster entry with neither is a session abeam can say nothing about,
        // and an empty row reads better than a row that says so.
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentstate::parse_roster;
    use crate::testutil::TempDir;

    /// Real output of `git worktree list --porcelain -z`, captured from this
    /// repository with two scratch worktrees added to it — one on a branch,
    /// one detached — and read back through `od -c` so that every `\0` in it is
    /// where git put it rather than where it was assumed to be.
    ///
    /// Written out as one field per line, because the one thing this fixture
    /// exists to preserve is which fields are followed by an empty one.
    const PORCELAIN: &str = concat!(
        "worktree C:/Users/philm/PycharmProjects/forge\0",
        "HEAD 00b21a4321d91cefaa9f26c721a71c9c3001af4b\0",
        "branch refs/heads/hand-the-command-line-to-the-agent\0",
        "\0",
        "worktree C:/Users/philm/PycharmProjects/forge/.claude/worktrees/scratch-a\0",
        "HEAD 00b21a4321d91cefaa9f26c721a71c9c3001af4b\0",
        "branch refs/heads/scratch-a\0",
        "\0",
        "worktree C:/Users/philm/PycharmProjects/forge/.claude/worktrees/scratch-b\0",
        "HEAD 00b21a4321d91cefaa9f26c721a71c9c3001af4b\0",
        "detached\0",
        "\0",
    );

    /// The same command in a bare repository, captured the same way. Two
    /// fields, no HEAD and no branch — the shape that would panic a parser
    /// which assumed every record has three lines.
    const BARE: &str = concat!(
        "worktree C:/Users/philm/AppData/Local/Temp/abeam-bare\0",
        "bare\0",
        "\0",
    );

    /// A repository root and a nested worktree under it, spelled for the
    /// platform running the test.
    ///
    /// Places, not strings: what is being asserted goes through
    /// `crate::paths`, whose rule differs by platform, so a Windows path
    /// asserted on Linux would take the case-sensitive comparison without ever
    /// exercising it.
    #[cfg(windows)]
    const ROOT: &str = r"C:\Users\philm\PycharmProjects\forge";
    #[cfg(unix)]
    const ROOT: &str = "/home/philm/PycharmProjects/forge";

    fn nested(name: &str) -> PathBuf {
        Path::new(ROOT).join(".claude").join("worktrees").join(name)
    }

    fn on_branch(root: &Path, branch: &str) -> Worktree {
        Worktree {
            root: root.to_path_buf(),
            branch: Some(branch.to_string()),
            head: None,
            detached: false,
            bare: false,
        }
    }

    // --- the format -------------------------------------------------------

    #[test]
    fn every_record_git_prints_is_read_as_its_own_worktree() {
        let found = parse_worktrees(PORCELAIN);
        assert_eq!(found.len(), 3, "records were merged or dropped");

        assert_eq!(
            found[0].root,
            PathBuf::from("C:/Users/philm/PycharmProjects/forge")
        );
        assert_eq!(
            found[0].branch.as_deref(),
            Some("hand-the-command-line-to-the-agent"),
            "`refs/heads/` should be gone and nothing else with it"
        );
        assert_eq!(
            found[0].head.as_deref(),
            Some("00b21a4321d91cefaa9f26c721a71c9c3001af4b")
        );
        assert!(!found[0].detached && !found[0].bare);

        assert_eq!(
            found[1].root,
            PathBuf::from("C:/Users/philm/PycharmProjects/forge/.claude/worktrees/scratch-a")
        );
        assert_eq!(found[1].branch.as_deref(), Some("scratch-a"));

        // The detached one: a HEAD, no branch, and the flag that says which.
        assert!(found[2].detached);
        assert_eq!(found[2].branch, None);
        assert_eq!(
            found[2].head.as_deref(),
            Some("00b21a4321d91cefaa9f26c721a71c9c3001af4b")
        );
    }

    #[test]
    fn the_empty_record_is_the_separator_and_dropping_it_merges_everything() {
        // The bug this test exists for is not hypothetical: `panes::git` parses
        // `git status -z` with exactly the filter below, it is the idiom in
        // front of anyone writing this function, and copying it here produces
        // one worktree instead of three — silently, and only on the machine of
        // somebody who has made a second worktree.
        let as_git_status_would: Vec<&str> = PORCELAIN
            .split('\0')
            .filter(|record| !record.is_empty())
            .collect();
        assert_eq!(
            as_git_status_would.len(),
            9,
            "nine fields and nothing saying where one worktree ends"
        );
        assert_eq!(parse_worktrees(PORCELAIN).len(), 3);
    }

    #[test]
    fn a_bare_repository_has_no_head_and_no_branch_and_is_still_a_worktree() {
        let found = parse_worktrees(BARE);
        assert_eq!(found.len(), 1);
        assert!(found[0].bare);
        assert_eq!(found[0].branch, None);
        assert_eq!(found[0].head, None);
        // ...and it has a name to show, taken from the directory because there
        // is no branch to take it from.
        assert_eq!(label_of(&found[0]), "abeam-bare");
    }

    #[test]
    fn nothing_git_might_add_next_costs_the_reader_a_worktree() {
        // `locked` with a reason, `locked` without one, and a field invented
        // for this test. A parser that failed on any of them would take the
        // whole list down over an entry it did not need to understand.
        let out = concat!(
            "worktree /a\0HEAD abc\0branch refs/heads/main\0locked under review\0\0",
            "worktree /b\0HEAD def\0detached\0locked\0prunable gitdir file points to non-existent location\0\0",
            "worktree /c\0HEAD ghi\0somethingnew whatever\0branch refs/heads/next\0\0",
        );
        let found = parse_worktrees(out);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].branch.as_deref(), Some("main"));
        assert!(found[1].detached);
        assert_eq!(found[2].branch.as_deref(), Some("next"));
    }

    #[test]
    fn an_empty_answer_is_an_empty_list_rather_than_a_worktree_named_nothing() {
        // What `run` hands back when git printed nothing, which is what a
        // failure looks like from here.
        assert!(parse_worktrees("").is_empty());
        assert!(parse_worktrees("\0\0").is_empty());
    }

    // --- ownership --------------------------------------------------------

    #[test]
    fn a_path_belongs_to_the_innermost_worktree_that_holds_it() {
        // The bug, stated as an assertion. Both roots contain the file — that
        // is what makes a prefix test useless — and only the inner one owns it.
        let roots = vec![PathBuf::from(ROOT), nested("other")];
        let theirs = nested("other").join("NOTES.md");
        let mine = Path::new(ROOT).join("README.md");

        assert_eq!(owner(&roots, &theirs), Some(nested("other").as_path()));
        assert_eq!(owner(&roots, &mine), Some(Path::new(ROOT)));

        // Order must not decide it. Written the other way round, a `find` that
        // took the first match instead of the longest would flip this answer
        // and nothing on screen would say so.
        let reversed = vec![nested("other"), PathBuf::from(ROOT)];
        assert_eq!(owner(&reversed, &theirs), Some(nested("other").as_path()));
        assert_eq!(owner(&reversed, &mine), Some(Path::new(ROOT)));
    }

    #[test]
    fn the_parent_of_a_worktree_is_not_news_about_the_repository_holding_it() {
        // The other half of the routing rule, and the half whose absence made
        // ownership alone look like it worked while it did not. Writing one
        // file in a nested worktree makes the watcher report the directories
        // above it in the same batch, and every one of those is owned by the
        // *enclosing* workspace — so `owner` hands them to the root, exactly as
        // it hands it somebody editing there by hand.
        let roots = vec![PathBuf::from(ROOT), nested("other")];

        for parent in [
            Path::new(ROOT).join(".claude"),
            Path::new(ROOT).join(".claude/worktrees"),
        ] {
            // Owned by the root — which is the trap, not a mistake in `owner`.
            assert_eq!(owner(&roots, &parent), Some(Path::new(ROOT)));
            // ...and still not evidence about it.
            assert!(
                !is_evidence(&roots, &parent),
                "{} is only ever news about what is nested under it",
                parent.display()
            );
        }

        // A root is always evidence about itself. Every root contains a nested
        // root somewhere below it once a worktree exists, so without this the
        // agent's own workspace would be silenced the moment one was created —
        // the routing bug arriving through the back door.
        assert!(is_evidence(&roots, Path::new(ROOT)));
        assert!(is_evidence(&roots, &nested("other")));

        // Ordinary files on both sides are untouched by the second rule; it is
        // `owner` that separates them, and it still does.
        let mine = Path::new(ROOT).join("README.md");
        let theirs = nested("other").join("NOTES.md");
        assert!(is_evidence(&roots, &mine));
        assert!(is_evidence(&roots, &theirs));

        // And a file that merely lives beside the worktrees directory is a real
        // edit in the root, not a parent of anything. This is the line between
        // the two rules: `.claude/settings.json` is somebody's own file.
        let settings = Path::new(ROOT).join(".claude/settings.json");
        assert!(is_evidence(&roots, &settings));
        assert_eq!(owner(&roots, &settings), Some(Path::new(ROOT)));

        // With no nested workspace there is nothing to suppress, so the rule
        // has to be inert rather than merely quiet.
        let alone = vec![PathBuf::from(ROOT)];
        assert!(is_evidence(&alone, &Path::new(ROOT).join(".claude")));
    }

    #[test]
    fn a_root_spelled_gits_way_still_owns_what_the_watcher_reports() {
        // The comparison this whole feature turns on. `git worktree list`
        // prints forward slashes on Windows and `notify` reports backslashes,
        // so a routing rule built on `==` or `starts_with` finds no owner at
        // all — and "no owner" means every change is dropped, which is the
        // watcher silently switched off.
        //
        // On Unix the two spellings are one string to begin with, so over there
        // this asserts only that nothing was broken by the rule that fixes
        // Windows. That asymmetry is the point of writing it with `replace`
        // rather than as two literals: the line means "however git spelled it".
        let gits_way = PathBuf::from(ROOT.replace('\\', "/"));
        let roots = vec![gits_way.clone()];
        let watched = Path::new(ROOT).join("src").join("main.rs");

        assert_eq!(owner(&roots, &watched), Some(gits_way.as_path()));
    }

    #[test]
    fn a_worktree_outside_the_repository_owns_only_its_own() {
        // Worktrees do not have to be nested; `git worktree add ../elsewhere`
        // is ordinary. Nothing about the rule assumes they are.
        let outside = Path::new(ROOT).parent().unwrap().join("elsewhere");
        let roots = vec![PathBuf::from(ROOT), outside.clone()];

        assert_eq!(
            owner(&roots, &outside.join("src/lib.rs")),
            Some(outside.as_path())
        );
        assert_eq!(
            owner(&roots, &Path::new(ROOT).join("src/lib.rs")),
            Some(Path::new(ROOT))
        );

        // And a path in neither belongs to neither. `None` is "not mine", not
        // "everyone's".
        let stranger = Path::new(ROOT).parent().unwrap().join("some-other-project");
        assert_eq!(owner(&roots, &stranger.join("x.md")), None);
        assert_eq!(owner(&[], &Path::new(ROOT).join("x.md")), None);
    }

    // --- the join ---------------------------------------------------------

    /// A roster in the shape `claude agents --json --all` really prints, with
    /// the `cwd` values put through serde rather than pasted in — a Windows
    /// path in JSON is a string full of escapes, and a fixture that doubles its
    /// own backslashes is one edit from being about something else.
    fn roster(entries: &[(&str, &Path, &str)]) -> Vec<Session> {
        let body: Vec<String> = entries
            .iter()
            .enumerate()
            .map(|(n, (id, cwd, state))| {
                format!(
                    r#"{{"id":"{id}","cwd":{},"kind":"background","startedAt":{},"state":"{state}"}}"#,
                    serde_json::to_string(&cwd.to_string_lossy()).expect("a JSON string"),
                    1_785_680_468_000u64 + n as u64
                )
            })
            .collect();
        parse_roster(&format!("[{}]", body.join(","))).expect("a roster")
    }

    #[test]
    fn a_row_says_where_you_are_who_is_there_and_whether_it_is_watched() {
        let here = nested("review");
        let worktrees = vec![
            on_branch(Path::new(ROOT), "main"),
            on_branch(&here, "review"),
            on_branch(&Path::new(ROOT).parent().unwrap().join("elsewhere"), "old"),
        ];
        let roster = roster(&[("a1b2c3d4", &here, "working")]);

        let rows = rows(
            &worktrees,
            &roster,
            Path::new(ROOT),
            &here,
            Some(Path::new(ROOT)),
        );

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "main");
        assert!(rows[0].agent_here, "the agent is in the main worktree");
        assert!(!rows[0].here, "the pane is not");
        assert_eq!(rows[0].occupant, None);
        assert!(rows[0].watched);

        assert!(rows[1].here, "the pane is on the nested one");
        assert!(!rows[1].agent_here);
        assert_eq!(
            rows[1].occupant.as_deref(),
            Some("a1b2c3d4 · working"),
            "Claude's own word, not one abeam made up"
        );
        assert!(rows[1].watched, "nested under the watched root");

        // The one outside the repository: real, listed, and beyond the reach of
        // the single watcher abeam runs.
        assert_eq!(rows[2].label, "old");
        assert!(!rows[2].watched);
    }

    #[test]
    fn a_finished_agent_does_not_hold_a_worktree_a_live_one_is_working_in() {
        // `--all` includes background agents that have finished, so a directory
        // can have several entries naming it. Reporting the oldest would leave
        // a row reading `failed` over a worktree somebody is typing in.
        let root = nested("shared");
        let roster = roster(&[("old", &root, "failed"), ("new", &root, "working")]);
        // Standing in the worktree, so that this list is the one worktree and
        // the assertions below stay about occupancy rather than about the two
        // rows `rows` guarantees.
        let rows = rows(&[on_branch(&root, "shared")], &roster, &root, &root, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].occupant.as_deref(), Some("new · working"));
        assert!(!rows[0].watched, "no watcher at all is not a watched root");
    }

    #[test]
    fn the_workspace_you_are_in_always_has_a_row_and_so_does_the_agents_own() {
        // The list is how you switch workspaces, so a workspace with no row on
        // it is a workspace you cannot get back to — and `crate::app` keeps
        // `spaces[0]`, the agent's own root, for ever. Switching away from it
        // was a one-way trip whenever git did not happen to name it, and
        // nothing on the list said `here` or `agent_here`, so the list also
        // said you were nowhere.
        //
        // The way git does not name it is ordinary: **abeam started in a
        // subdirectory of the repository.** `git worktree list` names the
        // repository root, not the directory you were standing in, and the git
        // pane supports being started there — it resolves `toplevel` for every
        // open.
        let below = Path::new(ROOT).join("crates").join("abeam");
        let worktrees = vec![
            on_branch(Path::new(ROOT), "main"),
            on_branch(&nested("other"), "other"),
        ];

        let listed = rows(&worktrees, &[], &below, &below, Some(Path::new(ROOT)));
        assert_eq!(listed.len(), 3, "the agent's own workspace has no row");
        assert_eq!(listed[0].label, "abeam", "named by its directory");
        assert!(paths::same_dir(&listed[0].root, &below));
        assert!(listed[0].here && listed[0].agent_here);
        assert!(listed[0].watched, "the one watcher covers it");
        // ...and git's own list is still all there, in git's own order.
        assert_eq!(listed[1].label, "main");
        assert_eq!(listed[2].label, "other");
        assert!(!listed[1].here && !listed[1].agent_here);

        // The right pane pointed somewhere else: two workspaces to guarantee
        // and two rows added, `here` and `agent_here` on different ones. This
        // is the case the two fields exist for.
        let split = rows(
            &worktrees,
            &[],
            &below,
            &nested("kept"),
            Some(Path::new(ROOT)),
        );
        assert_eq!(split.len(), 4);
        assert!(split[0].agent_here && !split[0].here);
        assert!(split[1].here && !split[1].agent_here);
        assert_eq!(split[1].label, "kept");

        // A workspace `crate::app` retained because a child is still running in
        // it drops off git's list too, and it is the one `at` can be pointing
        // at while that happens — so this is the same guarantee covering the
        // other way a row goes missing.
        let undiscovered = rows(&[], &[], Path::new(ROOT), &nested("kept"), None);
        assert_eq!(
            undiscovered.len(),
            2,
            "nothing discovered is not nothing to show"
        );
        assert!(undiscovered[0].agent_here);
        assert!(undiscovered[1].here);

        // And when git *does* name them — the ordinary case, and every session
        // that started at the top of a repository — nothing is added and
        // nothing is duplicated.
        let ordinary = rows(
            &worktrees,
            &[],
            Path::new(ROOT),
            Path::new(ROOT),
            Some(Path::new(ROOT)),
        );
        assert_eq!(ordinary.len(), 2);
        assert_eq!(
            ordinary[0].label, "main",
            "the branch name, not the directory"
        );
        assert!(ordinary[0].here && ordinary[0].agent_here);
    }

    #[test]
    fn a_detached_worktree_is_labelled_by_the_directory_somebody_named() {
        // The alternative is a forty-character sha, which is not a thing anyone
        // recognises their own work by.
        let root = nested("scratch-b");
        let detached = Worktree {
            root: root.clone(),
            branch: None,
            head: Some("00b21a4321d91cefaa9f26c721a71c9c3001af4b".into()),
            detached: true,
            bare: false,
        };
        // Standing in it, for the reason the test above gives: one worktree,
        // one row, and nothing in the way of the label being the subject.
        let rows = rows(&[detached], &[], &root, &root, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "scratch-b");
    }

    // --- against a real repository ----------------------------------------

    /// The one test here that starts a process, and it earns it.
    ///
    /// Everything above is a decision about strings, and a decision about
    /// strings can be proved with strings. This one is the claim the whole
    /// change rests on — *a file written in a nested worktree belongs to the
    /// nested worktree* — and every step of it is somebody else's: git decides
    /// where a worktree lives and how it prints the path, the filesystem
    /// decides what the path looks like coming back. A fixture cannot be wrong
    /// about those in the same direction the code is wrong, which is exactly
    /// what a fixture written from the code would do.
    ///
    /// It needs git on `PATH`, which every machine that can build this already
    /// has; a machine without it fails here rather than passing quietly,
    /// because `discover` returning nothing is indistinguishable from the
    /// feature being deleted.
    #[test]
    fn a_file_written_in_a_nested_worktree_belongs_to_the_nested_worktree() {
        let dir = TempDir::new("workspace-discover");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("create the repository directory");

        let git = |cwd: &Path, args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_OPTIONAL_LOCKS", "0")
                // A machine whose global config has no identity on it cannot
                // commit, and a repository with no commit has no HEAD to add a
                // worktree at.
                .env("GIT_AUTHOR_NAME", "abeam")
                .env("GIT_AUTHOR_EMAIL", "abeam@example.invalid")
                .env("GIT_COMMITTER_NAME", "abeam")
                .env("GIT_COMMITTER_EMAIL", "abeam@example.invalid")
                .stdin(Stdio::null())
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };

        git(&root, &["init", "-q", "-b", "main", "."]);
        std::fs::write(root.join("README.md"), b"# repo\n").expect("write");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "first"]);
        // Exactly where Claude Code puts them.
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "other",
                ".claude/worktrees/other",
            ],
        );
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                ".claude/worktrees/loose",
            ],
        );

        let found = discover(&root);
        assert_eq!(found.len(), 3, "git printed {found:#?}");
        let roots: Vec<PathBuf> = found.iter().map(|w| w.root.clone()).collect();

        // Real files, in real worktrees, at the paths a watcher would report.
        let theirs = root.join(".claude/worktrees/other/NOTES.md");
        let loose = root.join(".claude/worktrees/loose/scratch.md");
        let mine = root.join("src/main.rs");
        for path in [&theirs, &loose, &mine] {
            std::fs::create_dir_all(path.parent().unwrap()).expect("create");
            std::fs::write(path, b"x").expect("write");
        }

        // The whole bug, in three assertions: everything below is inside
        // `root`, and only one of the three belongs to it.
        let owns = |path: &Path| {
            owner(&roots, path)
                .map(Path::to_path_buf)
                .expect("every one of these is inside the repository")
        };
        assert!(paths::same_dir(&owns(&mine), &root));
        assert!(!paths::same_dir(&owns(&theirs), &root));
        assert!(!paths::same_dir(&owns(&loose), &root));
        assert!(paths::same_dir(
            &owns(&theirs),
            &root.join(".claude/worktrees/other")
        ));
        assert!(paths::same_dir(
            &owns(&loose),
            &root.join(".claude/worktrees/loose")
        ));

        // And what git said about them, read back through the parser: the
        // branch names survive, the detached one says so.
        let mut labels: Vec<String> = rows(&found, &[], &root, &root, Some(&root))
            .into_iter()
            .map(|row| row.label)
            .collect();
        labels.sort();
        assert_eq!(labels, ["loose", "main", "other"]);

        // Every one of them is nested, so the single watcher on the agent's
        // root really does see all three — which is why `.claude` must stay out
        // of the noise list.
        assert!(
            rows(&found, &[], &root, &root, Some(&root))
                .iter()
                .all(|row| row.watched)
        );
    }
}
