//! Finding the files that are already there.
//!
//! Noticing *new* markdown is the shell's job — one watcher serves both panes,
//! so it lives in `crate::watch` and this module borrows its idea of what is
//! worth reading. What is left here is the startup walk, so the pane opens on
//! the newest document instead of sitting empty until something changes.
//!
//! One walk answers two questions. `Tab` wants markdown in recency order,
//! because the agent touches source files constantly and a *recency* list that
//! follows every one of them is a list nobody can read. The find in `browse`
//! wants the opposite — every file there is, so that "view any file" is true —
//! and it wants them by name rather than by age. Both fall out of the same
//! traversal, and they have to: a second gitignore walk of the repository
//! would double the disk cost of startup to re-derive what the first one
//! already had in its hand.
//!
//! ## What is in the recency list
//!
//! Worth saying outright, because it decides what `Tab` opens on. Inside a
//! repository the walk does not refuse names beginning with a dot — see
//! [`walker`] — so `.claude/*.md` and `.github/**/*.md` are markdown like any
//! other, and both are in the list `Tab` reads. The `.claude` half is the point
//! rather than a side effect: a plan an agent wrote is exactly the document
//! this pane exists to put in front of someone.
//!
//! The cost lands on a fresh clone, where every file shares one checkout
//! timestamp and "newest" is decided by whatever order the filesystem hands
//! them back. The pane can open on `.github/ISSUE_TEMPLATE/bug.md`. That is
//! accepted rather than filtered: a rule that kept dot-directories out of
//! *recency* while the find and the grep could see them would be a third answer
//! to "what is in this repository", and the first write of any kind sorts it
//! out anyway.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use super::browse::ci_cmp;
use crate::paths;
use crate::watch::{in_noise, is_markdown};

/// Stop walking after this many entries. A monorepo's worth of gitignored
/// noise is not worth a second of a worker thread.
///
/// Reaching it is reported, in [`Scan::cut`], and that is not tidiness: this
/// cap counts *entries the walk visited*, while [`MAX_FILES`] counts files it
/// kept, and the two diverge for reasons nothing in the answer shows. Every
/// directory is an entry and no directory is a file; and past `MAX_FILES` the
/// walk goes on visiting entries it has stopped keeping. So a reader looking at
/// a short list and a small count has no way to tell a complete answer from a
/// truncated one, and `super::grep` reads this same list and would report a
/// definite count over a walk that stopped early.
///
/// What does *not* widen that gap is noise. [`walker`] prunes it — `.git`,
/// `target` and the rest are never descended into and never yielded — so they
/// cost no entries at all, and the budget is spent on the tree a reader would
/// recognise. Worth knowing when reading the number: a filter applied *after*
/// the yield produces the same list while charging the budget for every entry
/// it discards, and fifty thousand was not chosen for that walk.
///
/// Passed to [`scan`] rather than read there, on `super::browse::MAX_ENTRIES`'s
/// argument — so that the cap can be tested without materialising fifty
/// thousand entries to test it with. Here it buys more than convenience: the
/// difference between pruning noise and filtering it afterwards is *only* ever
/// visible at the cap, so without a cap a test can reach, nothing in this file
/// can fail when the filter is lifted back into the loop.
pub(super) const MAX_ENTRIES: usize = 50_000;

/// How much of the sorted markdown result to keep. Nobody tabs through two
/// hundred files; the tail is only there to be the tail.
const KEEP: usize = 200;

/// How many paths the find index holds.
///
/// Unlike [`KEEP`] this is not a list anyone scrolls, so patience is not the
/// bound — time is. The find is a linear pass over every entry on each
/// keystroke of a query, and a subsequence match over twenty thousand
/// root-relative paths costs well under a millisecond, which is the budget a
/// keystroke has. A repository larger than this is one nobody finds a file in
/// by typing three letters anyway. Past the cap the walk keeps whatever it saw
/// first, deliberately: there is no ranking that could be applied here — not
/// age, not depth — that a reader would recognise as "the ones it kept".
///
/// `super::grep` reads this same list and so inherits the cap without being
/// able to see it, which is why reaching it is reported in [`Scan::cut`]
/// alongside the walk's own.
const MAX_FILES: usize = 20_000;

/// What one walk of the root answers with.
pub struct Scan {
    /// Markdown under the root, newest first. `Tab` walks it.
    pub recent: Vec<PathBuf>,
    /// Every file under the root, as [`rel`] spells them, sorted by name. The
    /// find index.
    pub files: Vec<String>,
    /// The traversal stopped at [`MAX_ENTRIES`], or the index at [`MAX_FILES`],
    /// so `files` is a prefix of what is there rather than the whole of it.
    ///
    /// One flag for the two caps because the reader's position is the same
    /// under either: what they are looking at is short, and nothing they type
    /// will lengthen it. Neither is visible from `files` alone — `MAX_ENTRIES`
    /// counts entries rather than files, so a walk can be cut with the list
    /// nowhere near full.
    pub cut: bool,
}

/// Walk `root`, gitignore-aware. Returns immediately; the answer arrives on
/// the channel when the worker is done.
pub fn spawn_scan(root: PathBuf) -> Receiver<Scan> {
    let (tx, rx) = mpsc::channel();
    // Detached on purpose. It holds nothing the pane needs to reclaim, and a
    // join on shutdown would be a wait on a filesystem walk.
    std::thread::spawn(move || {
        let _ = tx.send(scan(&root, MAX_ENTRIES));
    });
    rx
}

/// One walk, on whichever thread asked. `spawn_scan` is the only caller that
/// is not a test — and the tests that use it are not all in this file: the
/// repository search is defined by what this list contains, so its own suite
/// asserts against the real walk rather than a hand-written list that could
/// quietly stop resembling one.
///
/// `max_entries` is [`MAX_ENTRIES`] everywhere but a test; see the constant for
/// why it is a parameter at all.
pub(super) fn scan(root: &Path, max_entries: usize) -> Scan {
    let mut markdown: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut walk = walker(root);

    // Nothing is filtered in this loop, and that is the point rather than an
    // omission: `walker` refused the noise before `max_entries` was charged for
    // it, and a second copy of the rule here would be exactly the drift
    // [`off_the_index`] exists to prevent. The one entry the filter never sees
    // is the root, and the root is not a file.
    for entry in walk.by_ref().take(max_entries).flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if files.len() < MAX_FILES {
            files.push(rel(root, path));
        }
        if !is_markdown(path) {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        markdown.push((modified, path.to_path_buf()));
    }

    // Asked rather than inferred from a count. `take(max_entries)` leaves the
    // walk on exactly the entry it stopped before, so one more pull is the
    // difference between "there was more" and "it happened to end there" — and
    // a tree of exactly fifty thousand entries is otherwise reported as
    // truncated for ever.
    let cut = files.len() >= MAX_FILES || walk.next().is_some();

    markdown.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    markdown.truncate(KEEP);
    // Sorted here, on the worker, because an empty query shows the index as it
    // stands and that listing is read by a person.
    files.sort_unstable_by(|a, b| ci_cmp(a, b));

    Scan {
        recent: markdown.into_iter().map(|(_, p)| p).collect(),
        files,
        cut,
    }
}

/// The walk [`scan`] runs, built in a function so that a test can build the
/// same one.
///
/// Extracted for the reason `crate::watch::sift` is extracted, and the analogy
/// is only worth making because a test really does call this one: what matters
/// here is not what the walk *yields* but what it never visits, and that is
/// invisible from [`Scan`].
/// `noise_is_pruned_before_the_budget_is_charged_for_it` counts what comes out
/// of this function directly, and the `max_entries` parameter is what makes the
/// same fact visible through [`scan`] as well — see the `filter_entry` argument
/// below for what lifting the filter back into the loop would cost.
///
/// ## `hidden` is not a taste, and it is not unconditional either
///
/// `.claude`, `.github`, `.gitignore` — the dot-named places an agent's work
/// and a repository's rules actually live — were invisible to `Tab`, to the
/// find and to `super::grep` alike, and none of that was ever a decision about
/// hidden files. It was `.git` being kept out of the walk by a flag that kept
/// out everything else beginning with a dot as well.
///
/// The flag does a *second* job as well, and only outside a repository. abeam
/// does not require one: `main` resolves whatever directory it was started in,
/// `crate::workspace::discover` names "a directory that is not a repository" as
/// an ordinary case, and a home directory is somewhere a person can start abeam
/// — not a documented workflow, but nothing stops it and `+bash` is the reason
/// to. There, `hidden(true)` is the only thing keeping `.ssh`,
/// `.aws/credentials`, `.netrc`, `.git-credentials` and `.bash_history` out of
/// the find index — and out of the *lines* `f` prints into the pane, which is
/// worse, because a grep shows the secret rather than merely the file name.
///
/// The tempting answer is "gitignore covers that". It does not, and this is the
/// trap the whole of [`in_repository`] exists for: `ignore` 0.4.31 defaults
/// `require_git: true` and gates a gitignore match on an ancestor that has a
/// repository in it (`dir.rs`), so with no marker above the root, no gitignore
/// rule *above the root* can match.
///
/// Stated that precisely rather than as "no gitignore rules at all", which is
/// what an earlier draft said and is measurably false: `any_git` is recomputed
/// per matched entry over *that entry's* parents, so a repository sitting
/// **below** a non-repository root does make gitignore live for its own
/// subtree. That errs safe here — out there both guards are on — but this whole
/// design rests on the sentence, so the sentence has to be the true one.
///
/// So the dotfile guard is dropped exactly where its replacement is live, and
/// nowhere else. Inside a repository: `hidden(false)`, and gitignore is
/// *consulted*. That is the most that can be promised, and it is not the same
/// as gitignore *listing* any particular name — `.env` stays out because
/// essentially every repository ignores it, not because anything here
/// guarantees it. Outside one: `hidden(true)`, unchanged, because nothing else
/// would.
///
/// ## `filter_entry` rather than the same test in the loop
///
/// The difference is `max_entries`, and it is not a small one. That cap is
/// applied to what the walk *yields* — `take(max_entries)` — so a check made
/// after the yield has already been paid for. A repository with a few tens of
/// thousands of loose objects in `.git` would spend the entire budget inside
/// the object store and hand back an index that was both nearly empty and
/// flagged [`Scan::cut`]: the worst pair of answers available, a short list
/// *and* a warning that the short list is short.
///
/// `filter_entry` prunes. A directory it refuses is not descended into and is
/// not yielded, so `.git` costs nothing rather than costing everything.
///
/// The root is cloned into the closure because `ignore` wants the predicate
/// `Send + Sync + 'static`, and it is wanted at all because
/// [`off_the_index`] is relative to it. Depth 0 — the root itself — is never
/// offered to the filter, which is what stops a repository from filtering
/// itself out of existence; [`off_the_index`] says what depends on that.
fn walker(root: &Path) -> ignore::Walk {
    let owned = root.to_path_buf();
    // `ignore` reads .gitignore for us, which is the difference between a file
    // list and a list of build artefacts.
    ignore::WalkBuilder::new(root)
        .hidden(!in_repository(root))
        .git_ignore(true)
        .git_global(true)
        .parents(true)
        .filter_entry(move |entry| {
            // `is_dir` from the entry rather than from the disk, and the same
            // kind of guard as `is_worktree`'s own `is_file`: dropping it does
            // not change any answer — `<some-file>/.git` is not a readable
            // regular file either — it changes what the walk *pays*. Asking the
            // second clause of every file is up to twenty thousand extra stats
            // per walk for an answer that is always false.
            !off_the_index(
                &owned,
                entry.path(),
                entry.file_type().is_some_and(|t| t.is_dir()),
            )
        })
        .build()
}

/// Is `root` somewhere gitignore will actually be consulted?
///
/// The question decides one thing — whether [`walker`] and
/// [`super::browse::list`] drop the hidden-file guard — and it has to be this
/// question rather than "is there a `.git` here", because the guard and its
/// replacement must be decided by *the same ancestry walk* or they can
/// disagree. Disagreeing in one direction shows dot-named files that gitignore
/// was supposed to be filtering; in the other it hides `.claude` in a
/// repository that would have been perfectly happy to show it.
///
/// So this mirrors what `ignore` itself does, deliberately and in the same
/// shape. `ignore` walks the ancestors with `parents(true)` and marks each one
/// `has_git` if it holds `.git` **or** `.jj` (`dir.rs`), then requires at least
/// one such ancestor before any gitignore rule is allowed to match. `.jj` is in
/// there because a jujutsu checkout honours `.gitignore` without necessarily
/// having a `.git`, and leaving it out here would mean refusing to show
/// dot-names in a repository whose ignore rules are live.
///
/// **Ancestors, not the root alone.** abeam is started wherever someone happens
/// to be standing — `crate::workspace`'s `rows` calls out being started in a
/// subdirectory as ordinary, and `crate::panes::git` resolves the worktree top
/// level on every open for the same reason. `root.join(".git").exists()` would
/// answer "not a repository" for `<repo>/crates/abeam` and quietly restore the
/// old blindness for anyone who did not start at the top.
///
/// **Ancestors, but stopping below the home directory**, and that boundary is
/// the whole reason this is not three lines. `git init ~` is how a great many
/// people keep dotfiles — yadm and chezmoi both leave a real `.git` at `$HOME`
/// — and without the stop, `abeam +bash` in a home directory would find it,
/// drop the guard, and index `.ssh/id_ed25519`, `.aws/credentials`, `.netrc`,
/// `.git-credentials` and `.bash_history`. A bare `git init ~` has no
/// `.gitignore`, so nothing else would refuse them; and `f` prints the matching
/// *line*, so the secret is on screen rather than merely the file name. That is
/// the exact directory, and the exact five files, this guard exists for — so
/// the one repository that must not count is the one rooted at `$HOME`.
///
/// The two cases really are separable, which is what makes the stop the right
/// shape rather than a special case: `~/PycharmProjects/forge/crates/abeam`
/// finds its `.git` at `forge`, below the boundary, and is unaffected;
/// `~/.config/nvim` as its own repository finds one at the root itself, before
/// the walk has gone anywhere. Only a marker found *at or above* `$HOME` is
/// refused.
///
/// With no home directory to be had, the walk is the unbounded one it was
/// before — an unset `HOME` is not a reason to refuse a real repository, and a
/// process with none is not a person sitting in their profile.
///
/// Neither marker is opened or parsed: existence is all `ignore` tests, so it
/// is all this tests. A handful of `exists` calls once per walk.
pub(super) fn in_repository(root: &Path) -> bool {
    in_repository_from(root, crate::agentstate::home())
}

/// The rule above, over the home directory handed in rather than read.
///
/// Split for `crate::agentstate::sessions_path_from`'s reason, which is that
/// the process environment belongs to the whole test binary: a test that set
/// `HOME` to prove the boundary would be setting it for the eight hundred tests
/// running beside it, several of which spawn children that inherit it.
fn in_repository_from(root: &Path, home: Option<PathBuf>) -> bool {
    let has_marker = |dir: &Path| dir.join(".git").exists() || dir.join(".jj").exists();

    // Absolute or not at all, on `sessions_path_from`'s rule and for its
    // reason: a relative `HOME` names wherever this process is standing, which
    // `main` has deliberately moved elsewhere, and a boundary drawn there would
    // be drawn in the wrong place rather than not at all.
    //
    // Resolved, because `root` is — `crate::testutil` and `crate::paths` both
    // say why at length, and the short version is that `%USERPROFILE%` and a
    // canonicalised path disagree about 8.3 short names on every GitHub runner.
    let Some(home) = home
        .filter(|dir| dir.is_absolute())
        .map(|dir| paths::resolve_root(&dir))
    else {
        return root.ancestors().any(has_marker);
    };

    // `under` is reflexive, so this is "root *is* the home directory, or is
    // above it" in one question. Above it is refused for the same reason the
    // directory itself is: a marker up there is not this repository either.
    if paths::under(root, &home) {
        return false;
    }
    root.ancestors()
        .take_while(|dir| !paths::same_dir(dir, &home))
        .any(has_marker)
}

/// What the *index* will not look inside — `Tab`'s recency list, the find, and
/// `super::grep`'s corpus, which are all one walk.
///
/// Not shared with [`super::browse::list`], and the asymmetry is the design
/// rather than a gap. The index answers "the files of **this** workspace"; the
/// listing answers "what is in this directory", which is navigation, and
/// pruning navigation takes places away from someone who is walking to them.
/// What the two do share is `crate::watch::in_noise` and [`in_repository`] —
/// the parts where one answer really is wanted in both.
///
/// ## Noise
///
/// `crate::watch::in_noise`, the watcher's own list, so a directory the
/// watcher refuses to carry news from is not one the index offers to open.
/// Root-relative, which is that function's whole subject: a checkout that
/// happens to live under a directory called `dist` is still a checkout.
///
/// Asked first because it costs no syscall, which is what keeps the clause
/// below off every path under `.git`, `target` and `.jj`.
///
/// ## A directory that is a worktree of another repository
///
/// Claude Code makes git worktrees at `<root>/.claude/worktrees/<name>` and
/// runs other agents in them, so one repository root can contain two working
/// trees belonging to two different people. [`scan`] has no `workspace::owner`
/// check — it walks the root it was handed — so without this clause a
/// neighbouring agent's scratch markdown arrives in this window's `Tab` list,
/// its find index and its grep corpus. That is the routing bug
/// `crate::workspace` exists to fix, arriving again by another road;
/// `hidden(true)` was covering the walk's half of it by accident, through
/// `.claude` beginning with a dot.
///
/// The rule is git's own containment model rather than one invented here:
/// `git status` in the main worktree says nothing about a nested worktree's
/// modifications, because the nested tree has its own index and its own HEAD.
///
/// **Narrower than "any nested working tree", deliberately.** A directory
/// holding a `.git` of any kind would also catch **submodules**, and pruning
/// those would be wrong twice over. They are in the index today, so a change
/// billed as "see more" would be quietly removing files. And
/// `workspace::owner` routes a submodule's paths to *this* workspace — no
/// worktree of it is ever listed by `git worktree list` — so `App::route`
/// would still follow a write inside one and hand `viewer.follow` a document
/// that this pane's own index said did not exist.
///
/// What separates them is what git writes in the file:
///
/// ```text
/// worktree:  gitdir: C:/repo/.git/worktrees/feature
/// submodule: gitdir: ../../.git/modules/vendor/lib
/// ```
///
/// So: a `.git` that is a **file**, whose `gitdir:` path ends
/// `worktrees/<id>` — the **penultimate component**, and nothing else about the
/// path. Git's own layout is the whole rule: a worktree's git dir is always
/// `$GIT_COMMON_DIR/worktrees/<id>`, exactly one component below `worktrees`,
/// and a submodule's is `$GIT_COMMON_DIR/modules/<path>`, which can be any
/// depth below `modules` but is never one below a `worktrees`.
///
/// **Anything that searches the whole path is wrong**, and the way it is wrong
/// is silent. An earlier draft asked for a `worktrees` component with no
/// `modules` component, and `git worktree add` writes an **absolute** gitdir —
/// so a checkout living under any directory called `modules` vetoed its own
/// rule. Verified against git 2.54.0: two repositories differing only in the
/// name of a parent directory,
///
/// ```text
/// …/gitlab/main1/.git/worktrees/other          pruned
/// …/gitlab/modules/proj/.git/worktrees/other   kept — the rule silently off
/// ```
///
/// A Java multi-module checkout, a Terraform layout, `~/work/modules/service`.
/// This is `crate::watch::in_noise`'s trap — a rule that reads a path the
/// caller does not control — arriving inside the function whose doc claims to
/// have taken that care. Looking at one component, at a position git fixes,
/// cannot be walked into from above.
///
/// The rule was checked against real fixtures rather than reasoned about; the
/// eight shapes and the gitdir git writes for each are in
/// `the_shapes_git_writes_into_a_dot_git_file`.
///
/// **It fails open.** A `.git` file that cannot be read, does not parse, or
/// carries no `gitdir:` line is not pruned. Pruning is the destructive
/// direction here — a directory silently missing from the file list is the
/// failure this whole module is written against — so an unrecognised marker
/// gets the benefit of the doubt.
///
/// A worktree **of a submodule** — `…/super/.git/modules/vendor/lib/worktrees/subwt`
/// — is pruned, and that is the rule being right rather than a cost accepted.
/// It is another working tree, with its own index and its own HEAD, so git's
/// containment model says the same thing about it as about any other. An
/// earlier draft documented keeping it as a fail-open cost; it was the old
/// rule's accident, not a decision.
///
/// ## Depth 0 is exempt, and has to be
///
/// Asked about a walk root this answers *true* for a repository checked out
/// under a name like `dist`, and for a root that is itself a worktree — which
/// is exactly what abeam is doing when the panes are re-rooted into one.
/// Neither walk ever asks it that. `ignore`'s `skip_entry` returns early at
/// depth 0 (`walk.rs`), so the root is yielded whatever a filter thinks of it,
/// and [`super::browse::list`] drops `dir` itself by name before it filters.
///
/// That exemption is what `the_walk_root_is_never_pruned_by_what_it_is` pins,
/// and losing it would not shorten the file list — it would empty it, in every
/// repository at once. Not
/// `a_root_under_a_noisy_name_is_scanned_rather_than_skipped`, which reads as
/// though it were about this and is not: `in_noise` is root-relative, so the
/// filter *accepts* that root and the exemption never comes into it.
///
/// ## What it costs
///
/// One `is_file` per surviving directory, and nothing per file — the `is_dir`
/// argument comes off the walk's own entry, so no path this predicate refuses
/// by name is ever stat-ed at all. Reading the marker happens only for the rare
/// directory whose `.git` really is a file. In [`scan`] that is one stat per
/// directory of the tree, on a worker thread, and none beneath anything already
/// refused.
fn off_the_index(root: &Path, path: &Path, is_dir: bool) -> bool {
    in_noise(root, path) || (is_dir && is_worktree(path))
}

/// How much of a `.git` file is read before the answer is decided.
///
/// A worktree marker is one short line. This is reached only for something
/// named `.git` that is a file rather than a directory, so it takes a planted
/// fixture to abuse — but reading an arbitrary file whole into memory because
/// of its *name* is a shape worth not having, and 4 KiB is three orders of
/// magnitude more than git has ever written here.
const MAX_MARKER: u64 = 4096;

/// Does this directory hold the `.git` **file** that `git worktree add` leaves
/// behind? See [`off_the_index`] for the rule, for the absolute-path trap the
/// obvious version of it walks into, and for why a submodule's `.git` file must
/// not be mistaken for one.
pub(super) fn is_worktree(dir: &Path) -> bool {
    use std::io::Read;

    let marker = dir.join(".git");
    // A `.git` **directory** is a repository someone cloned into place — a
    // vendored dependency, a nested checkout — and not a worktree of anything.
    //
    // **This is a cost guard, not a correctness one**, and it is worth being
    // exact about that because the name suggests otherwise. Without it the
    // answer does not change: `File::open` on a directory fails on Windows and
    // `read_to_string` fails with `EISDIR` on Linux, so a `.git` directory
    // falls out `false` either way. What the `is_file` buys is that the common
    // case — every directory in the tree, none of which has a `.git` at all —
    // costs one `stat` rather than a `stat` and a failed `open`. No test below
    // can pin it, and one that claimed to would be pinning the fixture.
    if !marker.is_file() {
        return false;
    }
    let Ok(file) = std::fs::File::open(&marker) else {
        return false;
    };
    let mut text = String::new();
    if file.take(MAX_MARKER).read_to_string(&mut text).is_err() {
        return false;
    }
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("gitdir:"))
        .any(|gitdir| {
            // The penultimate component, and only that. `rev().nth(1)` on a
            // one-component path is `None`, which is the fail-open answer.
            Path::new(gitdir.trim())
                .components()
                .rev()
                .nth(1)
                .is_some_and(|part| part.as_os_str() == "worktrees")
        })
}

/// A path as the index holds it: relative to the root, with `/` separators on
/// every platform.
///
/// Both halves earn their place. Relative, because that is the path a reader
/// recognises and the one a query gets typed against — nobody searches for
/// `C:\Users\...`. And `/`, because on Windows the walk spells the same path
/// with backslashes, so a query typed as `src/panes` would otherwise match
/// nothing at all.
///
/// The rewrite is Windows-only, and that is not tidiness. On Unix a backslash
/// is an ordinary byte in a file name — `weird\name.rs` is one file in one
/// directory — so rewriting it would invent a directory that is not there,
/// show the reader a path they cannot find, and put a `/` into the string the
/// query is matched against. There is nothing to rewrite on Unix anyway: the
/// walk already hands back `/`.
///
/// Shared with `browse`, which shows the same path in its breadcrumb and must
/// spell it the same way. Two implementations of "how a path is written down"
/// is two places for the answer to drift.
pub(super) fn rel(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    #[cfg(windows)]
    {
        rel.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        rel.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::time::Duration;

    /// A temp directory `ignore` will recognise as a repository.
    ///
    /// `super::browse` and `super::grep` both have one of these and both say
    /// why: `ignore` applies no `.gitignore` rule at all outside something it
    /// recognises as a repository, so a fixture without a `.git` makes every
    /// "excluded because gitignored" assertion pass for the wrong reason. This
    /// module needed one more urgently than either of them once
    /// [`in_repository`] arrived, because that fact now decides whether the walk
    /// shows dot-names at all — a bare `TempDir` is not a repository, and a
    /// dotfile test written against one would be asserting the *old* behaviour
    /// while looking like it asserted the new.
    fn repo(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        std::fs::create_dir_all(dir.path().join(".git")).expect("create .git");
        dir
    }

    /// A file at a path with directories in it, all of them made on the way.
    fn at(root: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture directory");
        }
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    /// The `.git` **file** `git worktree add` leaves in a worktree root, and
    /// the `.git` file a submodule gets instead. Written as bytes rather than
    /// faked with a directory, because the difference between the two *is* what
    /// [`is_worktree`] reads.
    fn worktree_marker(dir: &Path, gitdir: &str) {
        std::fs::create_dir_all(dir).expect("create the worktree directory");
        std::fs::write(dir.join(".git"), format!("gitdir: {gitdir}\n")).expect("write .git");
    }

    fn names(scan: &Scan) -> Vec<String> {
        scan.recent
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_scan_finds_markdown_newest_first_and_ignores_the_rest() {
        let dir = repo("scan");
        dir.write("old.md", b"# old\n");
        dir.write("src.rs", b"fn main() {}\n");
        // mtime resolution on this filesystem is coarse enough that two writes
        // in a row can tie; a real gap is the only reliable ordering.
        std::thread::sleep(Duration::from_millis(20));
        dir.write("new.md", b"# new\n");

        assert_eq!(names(&scan(dir.path(), MAX_ENTRIES)), ["new.md", "old.md"]);
    }

    #[test]
    fn the_index_spells_every_path_the_way_a_query_gets_typed() {
        // What a person types is `src/panes`, on both platforms, and the index
        // is what that is matched against — so on Windows the walk's
        // backslashes have to be rewritten or the query matches nothing.
        let root = Path::new(if cfg!(windows) { r"C:\repo" } else { "/repo" });
        let deep = root.join("src").join("panes");
        assert_eq!(rel(root, &deep), "src/panes");

        // ...and on Unix that rewrite must not happen, which is the half a
        // Windows-only suite could never have caught. A backslash there is an
        // ordinary byte in a file name, so rewriting it would invent a
        // directory that is not there and hand the reader a path that opens
        // nothing.
        #[cfg(unix)]
        {
            let odd = root.join(r"na\me.rs");
            assert_eq!(rel(root, &odd), r"na\me.rs");
        }
    }

    #[test]
    fn scanning_somewhere_that_does_not_exist_yields_nothing() {
        let dir = repo("scan-missing");
        let found = scan(&dir.path().join("gone"), MAX_ENTRIES);
        assert!(found.recent.is_empty());
        assert!(found.files.is_empty());
    }

    #[test]
    fn a_root_under_a_noisy_name_is_scanned_rather_than_skipped() {
        // The walk hands `in_noise` absolute paths, so a root under a directory
        // called `dist` used to filter out every file it found and leave the
        // pane sitting on its empty hint forever.
        let dir = TempDir::new("scan-nested");
        let root = dir.path().join("dist").join("myrepo");
        std::fs::create_dir_all(root.join(".git")).expect("create nested root");
        at(&root, "plan.md", b"# plan\n");
        // ...while noise *inside* the root is still noise.
        at(&root, "target/built.md", b"# built\n");

        assert_eq!(names(&scan(&root, MAX_ENTRIES)), ["plan.md"]);
    }

    #[test]
    fn the_same_walk_answers_with_every_file_and_not_only_the_markdown() {
        // The find is the half of the file list that answers "view *any*
        // file", so an index that inherited the markdown filter would leave it
        // able to find only what `Tab` already reached.
        let dir = repo("scan-index");
        dir.write("plan.md", b"# plan\n");
        at(dir.path(), "src/main.rs", b"fn main() {}\n");
        at(dir.path(), "target/out.rs", b"//\n");

        let found = scan(dir.path(), MAX_ENTRIES);
        // Root-relative, `/`-separated, sorted, and without the build output.
        assert_eq!(found.files, ["plan.md", "src/main.rs"]);
        assert_eq!(found.recent.len(), 1, "the markdown list is unchanged");
        assert!(!found.cut, "a tree this size is not a truncated answer");
    }

    #[test]
    fn a_walk_that_stopped_early_says_so_where_it_can_be_seen() {
        // Neither cap is visible from the list itself, and `MAX_ENTRIES` is the
        // one that hides best: it counts entries the walk *visited*, and a
        // directory is an entry that is never a file — so a tree can exhaust it
        // while the list stays short, and a reader would see that short list
        // with no sign anything was left out. `super::grep` reports a count
        // over this list and would call it definite.
        //
        // Through `scan` itself, which `max_entries` is what makes possible. A
        // hand-rolled `WalkBuilder` here would have demonstrated `take` rather
        // than pinned `cut`, and `cut` is the field with a decision in it.
        let dir = repo("scan-cut");
        for i in 0..6 {
            dir.write(&format!("f{i}.md"), b"# x\n");
        }

        // The root itself is an entry, so a budget of three reaches two files
        // and there is provably more behind it.
        let short = scan(dir.path(), 3);
        assert_eq!(short.files.len(), 2);
        assert!(
            short.cut,
            "asked rather than inferred: one more pull is what makes `cut` true"
        );

        // ...and the same walk over the same tree, with room, is not cut. This
        // is the pair that matters: with only the negative half, `let cut =
        // false` passes the whole suite.
        let whole = scan(dir.path(), MAX_ENTRIES);
        assert_eq!(whole.files.len(), 6);
        assert!(!whole.cut);
    }

    #[test]
    fn a_git_managed_home_directory_does_not_switch_the_guard_off() {
        // `git init ~` for dotfiles — yadm and chezmoi both leave a real `.git`
        // at `$HOME` — used to make `in_repository($HOME)` true, drop the
        // hidden guard, and index `.ssh/id_ed25519` in the one directory the
        // guard was written for. A bare `git init ~` has no `.gitignore`, so
        // nothing else refuses them, and `f` prints the matching *line*.
        //
        // Over a home directory handed in rather than read: the process
        // environment belongs to all eight hundred tests in this binary. It is
        // also why the precondition assert in
        // `outside_a_repository_the_hidden_guard_stays_on` can never see this
        // case — the temp directory simply is not anybody's home.
        let fake = TempDir::new("scan-home");
        let home = fake.path().to_path_buf();
        std::fs::create_dir_all(home.join(".git")).expect("git init ~");
        let some = Some(home.clone());

        assert!(
            !in_repository_from(&home, some.clone()),
            "a repository rooted at $HOME is the one that must not count"
        );
        // ...and above it, for the same reason: a marker up there is not this
        // repository either.
        assert!(!in_repository_from(
            home.parent().expect("a parent"),
            some.clone()
        ));

        // The two cases really are separable, which is the whole reason the
        // boundary is the right shape. A checkout below home is unaffected...
        let below = home.join("PycharmProjects").join("forge");
        std::fs::create_dir_all(below.join(".git")).expect("create checkout");
        let deep = below.join("crates").join("abeam");
        std::fs::create_dir_all(&deep).expect("create subdirectory");
        assert!(in_repository_from(&deep, some.clone()), "stops at `forge`");

        // ...and a dotfile directory that is its own repository is found at the
        // root itself, before the walk has gone anywhere.
        let nvim = home.join(".config").join("nvim");
        std::fs::create_dir_all(nvim.join(".git")).expect("create nvim repo");
        assert!(in_repository_from(&nvim, some.clone()));
        // Its parent is not, which is what says the marker above did the work.
        assert!(!in_repository_from(&home.join(".config"), some.clone()));

        // Somewhere else on the disk entirely is the unbounded walk, unchanged.
        assert!(in_repository_from(&deep, None));
        // And a home directory nobody can name is not a reason to refuse a real
        // repository.
        assert!(in_repository_from(&below, None));
    }

    #[test]
    fn a_dot_name_is_a_file_this_window_can_see() {
        // Pins the whole point of dropping the hidden guard. `.claude`,
        // `.github` and `.gitignore` are where an agent's work and a
        // repository's rules actually live, and every one of them was missing
        // from `Tab`, from the find index and from `super::grep`'s corpus at
        // once — one flag, three silent absences, and nothing on screen
        // admitting to any of them.
        let dir = repo("scan-dotted");
        dir.write(".gitignore", b"*.key\n");
        at(dir.path(), ".github/workflows/ci.yml", b"on: push\n");
        at(dir.path(), ".claude/plan.md", b"# plan\n");
        // ...and gitignore is live here, which is what makes the guard safe to
        // drop rather than merely convenient to drop.
        dir.write("secret.key", b"shhh\n");

        let found = scan(dir.path(), MAX_ENTRIES);
        assert_eq!(
            found.files,
            [".claude/plan.md", ".github/workflows/ci.yml", ".gitignore"]
        );
        // The recency list too, which is the half `Tab` reads: a dot-named
        // directory is where notes get written, not merely where config lives.
        assert_eq!(names(&found), ["plan.md"]);
    }

    #[test]
    fn a_root_inside_a_repository_is_in_one_however_far_down_it_sits() {
        // abeam is started wherever someone is standing, and `crate::workspace`
        // calls a subdirectory of a checkout an ordinary place to start. A test
        // of `root.join(".git")` would answer "not a repository" for
        // `<repo>/crates/abeam` and put the old blindness back for everyone who
        // did not start at the top.
        let dir = repo("scan-subdir");
        let root = dir.path().join("crates").join("abeam");
        at(&root, ".claude/plan.md", b"# plan\n");
        at(&root, "src/main.rs", b"fn main() {}\n");

        assert!(in_repository(&root), "two levels down is still inside");
        assert_eq!(
            scan(&root, MAX_ENTRIES).files,
            [".claude/plan.md", "src/main.rs"]
        );

        // The same ancestry walk `ignore` does, including the marker it counts
        // besides `.git`: a jujutsu checkout honours `.gitignore` without
        // necessarily having one.
        let jj = TempDir::new("scan-jj-root");
        std::fs::create_dir_all(jj.path().join(".jj")).expect("create .jj");
        assert!(in_repository(&jj.path().join("sub")));
    }

    #[test]
    fn outside_a_repository_the_hidden_guard_stays_on() {
        // The half that is not about seeing more. `abeam +bash` in `$HOME` is
        // documented, and `ignore` 0.4.31 matches no gitignore rule at all
        // without a repository in the ancestry — `require_git` defaults true —
        // so out here `hidden(true)` is the only thing standing between the
        // find index, `f`'s printed lines, and `.ssh/id_ed25519`.
        let dir = TempDir::new("scan-homedir");
        assert!(
            !in_repository(dir.path()),
            "the temp directory has a repository above it; this test cannot say anything"
        );
        at(dir.path(), ".ssh/id_ed25519", b"PRIVATE KEY\n");
        at(
            dir.path(),
            ".aws/credentials",
            b"aws_secret_access_key = hunter2\n",
        );
        dir.write("notes.md", b"# notes\n");

        assert_eq!(scan(dir.path(), MAX_ENTRIES).files, ["notes.md"]);
    }

    #[test]
    fn noise_is_pruned_before_the_budget_is_charged_for_it() {
        // The reason the filter is `filter_entry` and not a `continue` in the
        // loop, and the only shape that can tell the two apart. `ignore` reads
        // `.git` to find ignore files and never prunes it — `hidden` was the
        // one thing that used to stop the descent — and the entry cap is
        // charged against what the walk *yields*, so a loop filter pays for
        // every loose object before discarding it.
        //
        // Three real files under a budget of six. Pruned, the walk visits the
        // root and the three and stops with nothing left. Post-filtered, it
        // spends the six inside the object store and comes back both short and
        // `cut`.
        let dir = repo("scan-prune");
        for i in 0..20 {
            at(dir.path(), &format!(".git/objects/deadbeef{i:02}"), b"x");
        }
        for name in ["a.md", "b.md", "c.md"] {
            dir.write(name, b"# x\n");
        }

        let found = scan(dir.path(), 6);
        assert!(
            !found.cut,
            "the object store was charged against the budget"
        );
        assert_eq!(found.files, ["a.md", "b.md", "c.md"]);

        // `cut` is the assertion that carries this, because it is the one that
        // does not depend on the order `read_dir` happens to return `.git` in —
        // and that order differs between NTFS and ext4. The list above is the
        // corroboration, not the proof.
        //
        // And directly, on what `walker` yields: the root and three files.
        // `.git` does not cost one entry, it costs none.
        assert_eq!(walker(dir.path()).count(), 4);
    }

    #[test]
    fn a_jj_object_store_is_pruned_like_a_git_one() {
        // `ignore` knows the name `.jj` only well enough to decide that
        // gitignore applies, and never skips it — so in a colocated jujutsu
        // repository `.jj/repo/store` is the whole object store sitting at
        // depth 1 with nothing refusing it. The worktree rule cannot help
        // either: `.jj/repo/store/git` is a *bare* repository and has no `.git`
        // entry to find. The noise list is the only thing there is.
        let dir = repo("scan-jj");
        for i in 0..20 {
            at(
                dir.path(),
                &format!(".jj/repo/store/git/objects/pack{i:02}"),
                b"x",
            );
        }
        at(dir.path(), ".jj/working_copy/checkout", b"x");
        for name in ["a.md", "b.md", "c.md"] {
            dir.write(name, b"# x\n");
        }

        let found = scan(dir.path(), 6);
        assert_eq!(found.files, ["a.md", "b.md", "c.md"]);
        assert!(!found.cut, "the jj store was charged against the budget");
    }

    #[test]
    fn a_directory_that_is_itself_a_worktree_is_not_part_of_this_one() {
        // `scan` has no `workspace::owner` check — it walks the root it was
        // handed — and `hidden(true)` used to keep Claude Code's worktrees out
        // of the index by accident, through `.claude` beginning with a dot.
        // Without this rule, dropping that flag would put a neighbouring
        // agent's scratch markdown into this window's `Tab` list, find index
        // and grep corpus: the routing bug `crate::workspace` exists to fix,
        // arriving again by another road.
        let dir = repo("scan-worktree");
        at(dir.path(), ".claude/plan.md", b"# plan\n");
        let other = dir.path().join(".claude").join("worktrees").join("other");
        worktree_marker(&other, "/repo/.git/worktrees/other");
        std::fs::write(other.join("NOTES.md"), b"# not ours\n").expect("write");

        let found = scan(dir.path(), MAX_ENTRIES);
        // `.claude` itself is reachable, and that is not a detail: the rule has
        // to prune the checkout without blinding the window to the directory
        // above it, which `crate::watch` argues at length is where the work is.
        assert_eq!(found.files, [".claude/plan.md"]);
        assert_eq!(
            names(&found),
            ["plan.md"],
            "a neighbouring agent's markdown reached `Tab`"
        );
    }

    #[test]
    fn a_submodule_is_part_of_this_repository_and_stays_in_the_index() {
        // The rule is "a worktree of another repository", not "anything with a
        // `.git` in it", and a submodule is where the difference bites. It is
        // in the index today, so pruning it would be a regression inside a
        // change billed as showing more. Worse, `git worktree list` never names
        // one — so `workspace::owner` routes its paths to *this* workspace,
        // `App::route` follows a write inside it, and `viewer.follow` would be
        // handed a document this pane's own index said did not exist.
        //
        // What tells them apart is the `gitdir:` line, and only that.
        let dir = repo("scan-submodule");
        let sub = dir.path().join("vendor").join("lib");
        worktree_marker(&sub, "../../.git/modules/vendor/lib");
        std::fs::write(sub.join("README.md"), b"# vendored\n").expect("write");
        dir.write("plan.md", b"# plan\n");

        let found = scan(dir.path(), MAX_ENTRIES);
        assert_eq!(found.files, ["plan.md", "vendor/lib/README.md"]);

        // ...and a marker nobody can make sense of fails *open*, because a file
        // silently missing from the list is the failure this module is written
        // against.
        let odd = dir.path().join("vendor").join("odd");
        std::fs::create_dir_all(&odd).expect("create");
        std::fs::write(odd.join(".git"), b"not a gitdir line at all\n").expect("write");
        std::fs::write(odd.join("kept.md"), b"# kept\n").expect("write");
        assert!(
            scan(dir.path(), MAX_ENTRIES)
                .files
                .contains(&"vendor/odd/kept.md".to_string())
        );
        // The predicate directly, on the three shapes it has to tell apart
        // here: a submodule, a submodule that happens to be *called*
        // `worktrees` — where the penultimate component is `vendor`, so the
        // name of the directory itself never enters into it — and a repository
        // with an ordinary `.git` directory, which is not a worktree of
        // anything. The full table is
        // `the_shapes_git_writes_into_a_dot_git_file`.
        assert!(!is_worktree(&sub));
        let named = dir.path().join("vendor").join("worktrees");
        worktree_marker(&named, "../../.git/modules/vendor/worktrees");
        assert!(!is_worktree(&named));
        assert!(!is_worktree(dir.path()));
    }

    #[test]
    fn the_shapes_git_writes_into_a_dot_git_file() {
        // The table this rule was built from, and it is a *table* because the
        // rule was got wrong once by reasoning about it. Every line was
        // produced by real git (2.54.0) under a scratch directory: `git
        // worktree add`, `git submodule add`, `git init --separate-git-dir`.
        //
        // Row two is the bug that made the rewrite necessary. `git worktree
        // add` writes an **absolute** gitdir, so an earlier rule that asked for
        // a `worktrees` component and no `modules` component turned itself off
        // for any repository living under a directory called `modules` — a Java
        // multi-module checkout, a Terraform layout — and failed open, so
        // nobody would have seen it.
        for (gitdir, want, what) in [
            (
                "C:/tmp/gitfix/main1/.git/worktrees/wt1",
                true,
                "a plain worktree",
            ),
            (
                "C:/tmp/gitfix/gitlab/modules/proj/.git/worktrees/other",
                true,
                "a worktree of a repository living under `modules/`",
            ),
            (
                "/srv/modules/repo/.git/worktrees/other",
                true,
                "the same bug in its posix spelling",
            ),
            (
                "../main2/.git/worktrees/wt_rel",
                true,
                "a worktree with worktree.useRelativePaths=true",
            ),
            ("../../.git/modules/vendor/lib", false, "a submodule"),
            (
                "../.git/modules/worktrees",
                false,
                "a submodule *named* worktrees",
            ),
            (
                "../../../super3/.git/worktrees/superwt/modules/vendor/lib",
                false,
                "a submodule inside a worktree",
            ),
            (
                "C:/tmp/gitfix/worktrees/store/sub",
                false,
                "a checkout with --separate-git-dir under a worktrees/ path",
            ),
            (
                "C:/tmp/gitfix/super1/.git/modules/vendor/lib/worktrees/subwt",
                true,
                "a worktree *of* a submodule — another working tree, so pruned",
            ),
        ] {
            let dir = TempDir::new("scan-gitdir");
            worktree_marker(dir.path(), gitdir);
            assert_eq!(is_worktree(dir.path()), want, "{what}: {gitdir}");
        }
    }

    #[test]
    fn a_vendored_clone_and_a_file_called_dot_git_are_both_kept() {
        // The two shapes that sit closest to a worktree without being one.
        // Named for what it pins, because the guards themselves — `is_dir` in
        // `walker`, `is_file` in `is_worktree` — provably cannot be pinned:
        // removing either leaves every answer unchanged and only costs syscalls
        // (both functions say so where they are written). What is worth holding
        // is the behaviour, which is that neither of these leaves the index.
        let dir = repo("scan-guards");

        // A file that merely ends in `.git`. Reached as a file, so the second
        // clause is never asked; reached as anything, `notes.git/.git` is not a
        // readable regular file.
        let mistaken = at(dir.path(), "notes.git", b"gitdir: /x/.git/worktrees/y\n");
        assert!(!off_the_index(dir.path(), &mistaken, false));
        assert!(!off_the_index(dir.path(), &mistaken, true));

        // A vendored checkout: a real `.git` **directory**, which is a
        // repository someone cloned into place and not a worktree of anything.
        let vendored = dir.path().join("vendor").join("clone");
        std::fs::create_dir_all(vendored.join(".git")).expect("create nested clone");
        std::fs::write(vendored.join("README.md"), b"# vendored\n").expect("write");
        assert!(vendored.join(".git").exists(), "the fixture proves nothing");
        assert!(!is_worktree(&vendored));
        assert!(
            scan(dir.path(), MAX_ENTRIES)
                .files
                .contains(&"vendor/clone/README.md".to_string())
        );
    }

    #[test]
    fn the_walk_root_is_never_pruned_by_what_it_is() {
        // Depth 0 is exempt from `ignore`'s filter (`walk.rs`), and everything
        // rests on it. A repository root holds `.git`; a root the panes have
        // been re-rooted *into* is a worktree by construction, which is the
        // whole point of re-rooting. If the exemption ever went, the file list
        // would not get shorter — it would be empty, in every repository at
        // once.
        let dir = TempDir::new("scan-root-worktree");
        worktree_marker(dir.path(), "C:/repo/.git/worktrees/mine");
        dir.write("plan.md", b"# plan\n");
        at(dir.path(), ".claude/notes.md", b"# notes\n");

        assert!(is_worktree(dir.path()), "the fixture is not a worktree");
        assert!(in_repository(dir.path()), "a `.git` file is still a `.git`");
        let found = scan(dir.path(), MAX_ENTRIES);
        assert_eq!(found.files, [".claude/notes.md", "plan.md"]);
    }
}
