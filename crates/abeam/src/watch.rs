//! Noticing what the agent just did.
//!
//! This is the thing abeam has that three separate windows do not. A wezterm
//! pane running lazygit next to one running glow is two programs that have to
//! be told; here, one watcher tells both panes, and neither of them has to be
//! asked.
//!
//! It lives in the shell rather than in a pane for two reasons. One recursive
//! watch of a repository root is enough — a second would double the OS-level
//! event traffic for the same information. And the two consumers want different
//! slices of the same stream: the viewer wants markdown paths, git wants to
//! know which paths changed. Splitting one stream is the shell's job; neither
//! pane can do it without knowing about the other.
//!
//! ## Why git is handed paths rather than a `bool`
//!
//! It used to be handed a `bool`, and the `bool` was a bug. Claude Code makes
//! git worktrees *inside* the repository — `<root>/.claude/worktrees/<name>` —
//! and runs other agents in them, so one recursive watch covers two working
//! trees belonging to two different people. A batch that says only "something
//! changed" cannot be routed, so every file a neighbouring agent wrote
//! refreshed this window's git pane and pulled that agent's scratch markdown
//! into this window's reader.
//!
//! The paths are therefore carried, and `crate::workspace` decides whose they
//! are. That decision is deliberately *not* made here: this module knows one
//! root — the one it was started on — and which of several workspaces a pane
//! happens to be looking at is the shell's business, not the watcher's.
//!
//! Worth saying plainly, because the one-line fix is right there: **`.claude`
//! is not in the noise list and must not be added to it.** It would close the
//! bug this afternoon and blind abeam inside its own worktrees for ever — the
//! panes are about to be re-rootable *into* those directories, and a watcher
//! that never fires there is the feature deleted rather than fixed. Noise is
//! for output nobody reads. `.claude/worktrees` is where the work is.
//!
//! ## What is filtered, and where
//!
//! Filtering happens on the debouncer's own thread, before anything reaches the
//! channel the UI polls. A `cargo build` inside the watched root generates
//! thousands of events a second and none of them should cost the draw loop a
//! `try_recv`, let alone a redraw. `notify` has no path-exclusion API, so the
//! events are still generated — they just die before they matter.
//!
//! `.git` is in the noise list, which means a commit made in another terminal
//! is *not* seen here. That is deliberate: watching `.git` on Windows means
//! watching lock files being created and deleted by every git command in the
//! session, including ours. The git pane's own two-second poll is the safety
//! net for changes the watcher cannot see, and two seconds is the right latency
//! for "someone committed elsewhere".
//!
//! The noise list is filtering by *path*. There is a second filter, by what the
//! event says happened, and it is a Linux fact rather than a preference — see
//! [`is_change`].

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    DebounceEventResult, DebouncedEvent, Debouncer, NoCache, new_debouncer_opt,
};

/// One save from an editor is several filesystem events, and an agent writing a
/// file is several more. Long enough to coalesce them, short enough that the
/// pane still feels like it reacted to the write rather than to a timer.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Directories `.gitignore` often does not mention but nobody wants to read.
/// The gitignore-aware walk gets this for free from `ignore`; the watcher has
/// no such help and has to be told.
const NOISE: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    ".next",
    ".idea",
];

pub fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(e) if e.eq_ignore_ascii_case("md")
            || e.eq_ignore_ascii_case("markdown")
            || e.eq_ignore_ascii_case("mdx")
    )
}

/// Is this path inside a directory nobody wants to read?
///
/// **Relative to the watched root**, and that is the whole of it: `notify`
/// hands out absolute paths, so testing every component tests the root's own
/// ancestors too. A repository living under any directory called `dist`,
/// `target`, `venv`, `node_modules` — `D:\work\dist\myrepo`, or just running
/// abeam from inside one — would then match on the ancestor and drop *every*
/// event, silently, while still reporting a watcher that started. The same
/// predicate walks the startup scan, so the file list would come back empty
/// too, and abeam's one reason to exist would be quietly gone.
pub fn in_noise(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|c| NOISE.contains(&c))
}

/// How many distinct paths one drained batch will carry before it gives up on
/// carrying them.
///
/// A `git checkout` of a large branch is thousands of paths inside one
/// debounce, and both lists here are deduplicated by scanning themselves — the
/// same retain-then-push that keeps the *last* mention last. That is free at
/// the size of an agent writing files and quadratic at the size of a branch
/// switch, on the debouncer's own thread. Past this many the list has also
/// stopped being worth anything: nobody routes ten thousand paths one at a
/// time to decide whether to run one `git status`.
///
/// So the cap is a statement about what the batch is *for* rather than a
/// memory limit. Under it, the batch says exactly what changed. Over it, it
/// says [`Change::overflowed`], which every reader is expected to take as
/// "assume everything changed" — the answer the panes gave unconditionally
/// before any of them could tell one workspace from another.
const MAX_PATHS: usize = 1024;

/// One batch of "something happened", already split for its readers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// Markdown the viewer should follow, in the order it was last mentioned.
    pub markdown: Vec<PathBuf>,
    /// Every non-noise path in the batch, deduplicated, last mention last.
    ///
    /// The paths and not just the fact of them, because the shell has to decide
    /// *whose* change each one is: with two git worktrees inside one watched
    /// root, a bare "something changed" cannot tell the agent's own repository
    /// from a neighbouring agent's — see `crate::workspace`. This used to be a
    /// `bool`, and that `bool` was the bug.
    ///
    /// `markdown` is a subset of this, so anything asking "was there any news
    /// at all" asks this one.
    pub changed: Vec<PathBuf>,
    /// The batch went past [`MAX_PATHS`] and the list below it is therefore
    /// incomplete. Read it as "assume everything changed": a reader that routes
    /// by path must fall back to refreshing whatever it is showing, because the
    /// paths that would have said otherwise were the ones dropped.
    pub overflowed: bool,
}

impl Change {
    /// Something under a worktree changed, so git's answer is stale. True for
    /// source files too — most of what an agent writes is not markdown, and the
    /// git pane cares about all of it.
    ///
    /// A method rather than the field it used to be, and kept at all rather
    /// than folded into `changed.is_empty()` at the call sites, because it is
    /// the question three tests below are really asking and the answer has to
    /// go on including an overflowed batch that kept none of its paths.
    pub fn worktree(&self) -> bool {
        self.overflowed || !self.changed.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        !self.worktree() && self.markdown.is_empty()
    }

    /// Fold another batch in. A rename fires under both names and a save fires
    /// more than once even after debouncing, so only the last mention of a path
    /// decides its place in the order.
    fn absorb(&mut self, other: Change) {
        self.overflowed |= other.overflowed;
        for path in other.changed {
            self.note_changed(&path);
        }
        for path in other.markdown {
            self.markdown.retain(|p| p != &path);
            self.markdown.push(path);
        }
    }

    /// Record one changed path, and say whether the list still knows about it.
    ///
    /// The answer is what keeps `markdown` a subset of `changed`: a path the
    /// cap turned away is not one the viewer should be offered either, since
    /// nothing downstream could work out whose it was.
    fn note_changed(&mut self, path: &Path) -> bool {
        // A path already in the list costs nothing to move, so the cap is
        // measured in *distinct* paths. An editor saving one file forty times
        // is one path, and must not be mistaken for a branch switch.
        let known = self.changed.iter().any(|seen| seen == path);
        if !known && self.changed.len() >= MAX_PATHS {
            self.overflowed = true;
            return false;
        }
        self.changed.retain(|seen| seen != path);
        self.changed.push(path.to_path_buf());
        true
    }
}

/// Whether an event is news that something *changed*, or somebody merely
/// reading.
///
/// **This is a Linux question, and it is not a small one.** `notify`'s inotify
/// backend registers `IN_OPEN` alongside the write masks, so every `open(2)`
/// anywhere inside the watched tree arrives here as an event carrying that
/// path: a `grep`, a build reading a source file, the reader opening a
/// document, `git status` re-hashing a file whose timestamp is too close to the
/// index's to be trusted. The other three backends have nothing like it —
/// `ReadDirectoryChangesW`, FSEvents and kqueue report writes, and `notify`
/// constructs no `Access` event on any of them. So a rule that took the paths
/// out of every event is right on three platforms and, on the fourth, cannot
/// tell a file being read from a file being written.
///
/// What that costs is worse than a stray refresh, because two of the readers
/// are abeam itself and both loops feed themselves:
///
/// - `crate::panes::git` runs `git status` in the watched root on every change
///   it is told about. On Linux that status opens working-tree files, which is
///   reported as a change, which asks for another status.
/// - `crate::panes::viewer` opens the document it is about to show. That open
///   is reported, `crate::app::route` follows it, and the pane is handed back
///   the file it just displayed.
///
/// Neither loop shows up as anything on screen. The window is busy, the panes
/// are correct, and the agent's whole screen is being re-rendered on a timer
/// for news nobody generated — which is exactly the discipline
/// `crate::app::route` says it exists to keep.
///
/// The rule is the one `AccessKind` already implies: **an access is somebody
/// reading, unless it is the close of a write.** `Access(Close(Write))` is kept
/// because on Linux it is the honest end of a write and costs nothing to keep;
/// every other access is a read, and a write that matters is reported as
/// `Create`, `Modify` or `Remove` besides.
fn is_change(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

/// Split a debounced burst. Separated from the callback so the one part with a
/// decision in it can be tested against a directory it was handed.
fn classify<I: IntoIterator<Item = PathBuf>>(root: &Path, paths: I) -> Change {
    let mut change = Change::default();
    for path in paths {
        if in_noise(root, &path) {
            continue;
        }
        // A delete or a rename is real news for git — something moved — but it
        // is not a document to show. Without this check the viewer replaces
        // whatever someone is reading with "no such file" the moment an agent
        // tidies up a scratch note, and a rename fires under both names so
        // which one won would come down to the order the debouncer listed them.
        //
        // Asked before the path is recorded, because `is_file` is a syscall and
        // the answer decides nothing about `changed` — keeping it here means
        // one stat per event rather than one per event that survives the cap.
        let is_document = is_markdown(&path) && path.is_file();
        if !change.note_changed(&path) {
            continue;
        }
        if is_document {
            change.markdown.retain(|p| p != &path);
            change.markdown.push(path);
        }
    }
    change
}

/// One debounced burst: filtered by what happened, then split for its readers.
///
/// Separated from the callback for the same reason [`classify`] is, and it buys
/// something [`is_change`]'s own table cannot. That table is a pure function and
/// the live test beside it can only *fail* on Linux, so with the filter merely
/// present in this file and not wired in front of the split, every test here
/// would still be green and the bug would be back. This is the seam where a
/// burst that is half reads can be handed in by hand, on any platform.
fn sift<I: IntoIterator<Item = DebouncedEvent>>(root: &Path, events: I) -> Change {
    classify(
        root,
        events
            .into_iter()
            .filter(|event| is_change(&event.event.kind))
            .flat_map(|event| event.event.paths),
    )
}

pub struct Watch {
    // Held only to keep the watcher alive; dropping it stops the thread.
    _debouncer: Debouncer<RecommendedWatcher, NoCache>,
    rx: Receiver<Change>,
}

impl Watch {
    /// `None` if the platform refuses to watch — a network share, a path that
    /// vanished between startup and here. In either case abeam degrades to
    /// manual refresh and says so on screen rather than failing to start.
    pub fn start(root: &Path) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        // The root travels with the callback because the noise filter is
        // relative to it, and `notify` reports absolute paths.
        let owned_root = root.to_path_buf();
        let debounced = move |result: DebounceEventResult| {
            let Ok(events) = result else { return };
            let change = sift(&owned_root, events);
            if !change.is_empty() {
                let _ = tx.send(change);
            }
        };

        // `NoCache`, spelled out, rather than the `new_debouncer` convenience —
        // and this is the whole of abeam's startup time rather than a taste.
        //
        // `new_debouncer` hands the debouncer a `RecommendedCache`, which on
        // Windows and macOS is `FileIdMap` and on Linux is already this. The
        // watch call below then goes through `FileIdMap::add_path`, which
        // *synchronously walks the entire tree* — `WalkDir` at unlimited depth,
        // `get_file_id` on every entry — before it returns. On Windows a file
        // id is a `CreateFileW` plus a `GetFileInformationByHandle`, so that is
        // one file open per entry, each of them inspected by the virus scanner
        // on the way past. Measured on this repository with a populated
        // `target/`: 34,279 entries, and `debouncer.watch()` took **5.0 s**.
        // The whole of the rest of abeam's startup is 40 ms, and every one of
        // those 5 s is spent after `term::setup` has switched to the alternate
        // screen — so what it looks like from outside is `uvx abeam` finishing
        // its download and then a blank terminal for five seconds. It scales
        // with whatever build output happens to be lying around, which is why
        // it is worse on a machine that has been working than on a fresh clone.
        //
        // What the cache buys is rename stitching, and abeam does not use it.
        // The debouncer pairs a rename when `trackers_match || file_ids_match`;
        // notify's Windows backend sets no tracker (`src/windows.rs` maps
        // `FILE_ACTION_RENAMED_OLD_NAME`/`_NEW_NAME` straight to
        // `RenameMode::From`/`To` with no `attrs`), so on Windows the file ids
        // are the only thing that can pair one. Paired, a rename arrives as one
        // event with `paths = [from, to]`; unpaired, as two events carrying one
        // path each. `is_change` admits both kinds and `classify` below records
        // every path it is handed — so the set of paths reaching the panes is
        // the same either way, which is what `classify`'s own comment about a
        // rename firing under both names already assumes. Identical news, five
        // seconds cheaper.
        //
        // Cheap to get wrong in the other direction, so: this is not a case of
        // filtering the walk down to the paths `in_noise` would keep. There is
        // no walk left to filter. The scan was never what made the watcher
        // work — `ReadDirectoryChangesW` on the root is — it only ever
        // pre-populated a lookup table for events abeam throws the distinction
        // away from.
        let mut debouncer = new_debouncer_opt::<_, RecommendedWatcher, NoCache>(
            DEBOUNCE,
            None,
            debounced,
            NoCache::new(),
            notify::Config::default(),
        )
        .ok()?;
        debouncer.watch(root, RecursiveMode::Recursive).ok()?;
        Some(Self {
            _debouncer: debouncer,
            rx,
        })
    }

    /// Everything seen since the last call, merged. Never blocks — this is
    /// called from the app's loop, once per iteration.
    pub fn drain(&self) -> Change {
        let mut out = Change::default();
        for change in self.rx.try_iter() {
            out.absorb(change);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::time::Instant;

    /// classify() asks the filesystem whether a path is still a file, so the
    /// fixtures have to be real. The root is a temp directory rather than a
    /// literal, which is also what stops these tests passing while the noise
    /// filter is measuring the wrong thing.
    struct Fixture(TempDir);

    impl Fixture {
        fn new(tag: &str) -> Self {
            Self(TempDir::new(tag))
        }

        fn root(&self) -> &Path {
            self.0.path()
        }

        /// An existing file under the root, at a path with directories in it.
        fn touch(&self, rel: &str) -> PathBuf {
            let path = self.root().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixture directory");
            }
            std::fs::write(&path, b"x").expect("write fixture");
            path
        }

        /// A path under the root that does not exist — a delete, or the losing
        /// half of a rename.
        fn gone(&self, rel: &str) -> PathBuf {
            self.root().join(rel)
        }

        fn classify<I: IntoIterator<Item = PathBuf>>(&self, paths: I) -> Change {
            super::classify(self.root(), paths)
        }
    }

    #[test]
    fn markdown_is_recognised_by_extension_whatever_its_case() {
        assert!(is_markdown(Path::new("a/b/PLAN.MD")));
        assert!(is_markdown(Path::new("notes.markdown")));
        assert!(!is_markdown(Path::new("main.rs")));
        assert!(!is_markdown(Path::new("README")));
    }

    #[test]
    fn build_output_is_not_a_change() {
        let root = Path::new(r"C:\work\myrepo");
        assert!(in_noise(root, &root.join("target/doc/index.md")));
        assert!(in_noise(root, &root.join("a/node_modules/x/readme.md")));
        assert!(!in_noise(root, &root.join("docs/design.md")));
    }

    #[test]
    fn a_repo_that_happens_to_live_under_a_noisy_name_is_still_watched() {
        // notify reports absolute paths, so testing every component tests the
        // root's own ancestors. This exact shape — a checkout under a directory
        // called `dist` — used to disable the watcher and the startup scan
        // completely, while still reporting a watcher that had started.
        let root = Path::new(r"D:\work\dist\myrepo");
        assert!(!in_noise(root, &root.join("notes.md")));
        assert!(!in_noise(root, &root.join("src/main.rs")));
        // ...and the filter still bites on the parts that are actually noise.
        assert!(in_noise(root, &root.join("dist/bundle.md")));
        assert!(in_noise(root, &root.join(".git/COMMIT_EDITMSG")));
    }

    #[test]
    fn a_source_edit_is_gits_business_and_not_the_viewers() {
        // The asymmetry is the whole point of splitting the stream: git wants
        // every write, the viewer wants the ones a human would read.
        let fx = Fixture::new("watch-source");
        let source = fx.touch("src/main.rs");
        let change = fx.classify([source.clone()]);
        assert!(change.worktree());
        assert!(change.markdown.is_empty());
        // ...and git is told *which* file, because that is what decides whose
        // worktree it was in.
        assert_eq!(change.changed, [source]);
    }

    #[test]
    fn a_cargo_build_reaches_neither_pane() {
        let fx = Fixture::new("watch-build");
        let change = fx.classify([
            fx.touch("target/debug/abeam.exe"),
            fx.touch("target/doc/x.md"),
        ]);
        assert!(change.is_empty(), "build output must not wake anything");
    }

    #[test]
    fn a_path_written_twice_is_followed_once_at_its_latest_mention() {
        let fx = Fixture::new("watch-twice");
        let (a, b) = (fx.touch("a.md"), fx.touch("b.md"));
        let change = fx.classify([a.clone(), b.clone(), a.clone()]);
        assert_eq!(change.markdown, [b, a]);
    }

    #[test]
    fn a_deleted_document_is_news_for_git_and_not_for_the_reader() {
        // A rename fires under both names and a delete fires under one, and
        // neither of those paths exists any more. Following one replaces the
        // document someone is reading with "no such file"; for a rename it is
        // worse, because which name wins is whatever order the debouncer
        // happened to list them in.
        let fx = Fixture::new("watch-deleted");
        let moved_to = fx.touch("docs/NOTES.md");
        let gone = fx.gone("NOTES.md");
        let change = fx.classify([gone.clone(), moved_to.clone()]);
        assert!(change.worktree(), "something moved; git's answer is stale");
        assert_eq!(
            change.markdown,
            std::slice::from_ref(&moved_to),
            "only the surviving name"
        );
        // Both names reach git, and they have to: a rename out of one worktree
        // and into another is two workspaces' news, and dropping the name that
        // no longer exists would lose one of them.
        assert_eq!(change.changed, [gone, moved_to]);

        let change = fx.classify([fx.gone("TODO.md")]);
        assert!(change.worktree());
        assert!(change.markdown.is_empty(), "nothing to open");
        assert!(!change.is_empty(), "git still has to hear about it");
    }

    #[test]
    fn batches_merge_rather_than_replacing_each_other() {
        // Two debounced bursts can land between one tick and the next; losing
        // the first one loses the file the agent wrote before the one it is
        // writing now.
        let fx = Fixture::new("watch-merge");
        let (first, second) = (fx.touch("first.md"), fx.touch("second.md"));
        let source = fx.touch("src/lib.rs");
        let mut acc = fx.classify([first.clone()]);
        acc.absorb(fx.classify([source.clone()]));
        acc.absorb(fx.classify([second.clone()]));
        assert_eq!(acc.markdown, [first.clone(), second.clone()]);
        assert!(acc.worktree());
        // The same fold, on the list the routing reads. A batch merged into
        // another must not lose the workspace the first one was about.
        assert_eq!(acc.changed, [first.clone(), source, second.clone()]);

        // And a path mentioned again by a later batch keeps one place in the
        // order — its latest — in both lists at once.
        acc.absorb(fx.classify([first.clone()]));
        assert_eq!(acc.markdown, [second.clone(), first.clone()]);
        assert_eq!(acc.changed.len(), 3, "three files, however often written");
        assert_eq!(acc.changed.last(), Some(&first));
    }

    #[test]
    fn a_branch_switch_gives_up_on_the_list_rather_than_on_the_news() {
        // A `git checkout` of a large branch is thousands of paths in one
        // debounce, and both lists deduplicate by scanning themselves. Past the
        // cap the batch stops trying to say *which* files and says only that it
        // cannot — which is the answer every pane acted on before any of them
        // could tell one workspace from another, so nothing is missed by it.
        let fx = Fixture::new("watch-flood");
        let flood: Vec<PathBuf> = (0..MAX_PATHS + 50)
            .map(|n| fx.gone(&format!("src/file{n}.rs")))
            .collect();
        let change = fx.classify(flood);

        assert!(change.overflowed, "the cap never bit");
        assert_eq!(change.changed.len(), MAX_PATHS, "and it bit exactly there");
        assert!(change.worktree(), "git must still refresh");
        assert!(!change.is_empty());

        // An overflowed batch that kept none of its paths is still news. The
        // shell's routing reads `overflowed` on its own, so this is the shape
        // that must not report itself as nothing having happened.
        let empty_but_flooded = Change {
            markdown: Vec::new(),
            changed: Vec::new(),
            overflowed: true,
        };
        assert!(empty_but_flooded.worktree());
        assert!(!empty_but_flooded.is_empty());
    }

    #[test]
    fn one_file_saved_a_thousand_times_is_one_file() {
        // The cap counts distinct paths, not events. An editor with autosave on
        // and a compiler watching it can produce this, and a batch that
        // overflowed on it would throw away a list it could easily have kept.
        let fx = Fixture::new("watch-resaved");
        let one = fx.touch("notes.md");
        let change = fx.classify(std::iter::repeat_n(one.clone(), MAX_PATHS * 2));
        assert!(!change.overflowed);
        assert_eq!(change.changed, std::slice::from_ref(&one));
        assert_eq!(change.markdown, [one]);
    }

    #[test]
    fn the_viewer_is_never_offered_a_path_git_was_not_told_about() {
        // `markdown` is a subset of `changed`, and the shell relies on it: a
        // document whose path was dropped by the cap is one whose workspace
        // nothing downstream could work out, so following it would put another
        // agent's scratch note in front of the reader — the very bug the paths
        // are carried to fix.
        let fx = Fixture::new("watch-subset");
        let mut paths: Vec<PathBuf> = (0..MAX_PATHS)
            .map(|n| fx.gone(&format!("src/file{n}.rs")))
            .collect();
        paths.push(fx.touch("late.md"));
        let change = fx.classify(paths);

        assert!(change.overflowed);
        assert!(
            change.markdown.is_empty(),
            "a document past the cap must not be followed"
        );
        assert!(
            change.markdown.iter().all(|p| change.changed.contains(p)),
            "markdown is a subset of changed"
        );
    }

    /// The one test with a real watcher in it. Everything above calls
    /// `classify` directly, which is exactly how a filter that measured the
    /// wrong thing survived: relative literals cannot see the bug.
    #[test]
    fn a_real_watcher_on_a_root_under_a_noisy_name_still_reports_writes() {
        let fx = Fixture::new("watch-live");
        let root = fx.root().join("dist").join("myrepo");
        std::fs::create_dir_all(&root).expect("create nested root");
        let watch = Watch::start(&root).expect("watch a temp directory");

        std::fs::write(root.join("note.md"), b"# hello\n").expect("write");

        // One debounce plus slack. Polled rather than slept through, so the
        // test is quick when the platform is quick.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Change::default();
        while Instant::now() < deadline {
            seen.absorb(watch.drain());
            if !seen.markdown.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        assert!(seen.worktree(), "git was never told anything changed");
        assert_eq!(
            seen.markdown.last().map(|p| p.file_name()),
            Some(Some(std::ffi::OsStr::new("note.md"))),
            "the viewer was never handed the document"
        );
        // The path git is given is an absolute one under the root that was
        // watched, which is the whole premise of routing it: `crate::workspace`
        // is asked which root contains it, and a relative path is contained by
        // nothing.
        assert!(
            seen.changed.iter().all(|p| p.starts_with(&root)),
            "notify reported something outside the watched root: {:?}",
            seen.changed
        );
    }

    #[test]
    fn being_read_is_not_being_written() {
        use notify::event::{
            CreateKind, DataChange, MetadataKind, ModifyKind, RemoveKind, RenameMode,
        };

        // Every shape Linux's inotify backend constructs, sorted by whether
        // anything happened. `Open` is the row this function exists for: it is
        // what `git status` hashing a file and the reader opening a document
        // both arrive as, and both used to be taken for writes.
        //
        // `Open(Write)` is on this side too, which is a decision rather than an
        // oversight — an open for writing is an intention, and the write itself
        // still arrives as `Modify` and as `Close(Write)` below.
        for read in [
            EventKind::Access(AccessKind::Any),
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Open(AccessMode::Write)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Access(AccessKind::Other),
        ] {
            assert!(!is_change(&read), "{read:?} is somebody reading");
        }

        for wrote in [
            // The one access that is the end of a write rather than a read.
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            EventKind::Create(CreateKind::File),
            EventKind::Create(CreateKind::Folder),
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            EventKind::Remove(RemoveKind::File),
            // A backend that will not say must be taken at its word, or the
            // one platform that reports nothing more specific goes quiet.
            EventKind::Any,
        ] {
            assert!(is_change(&wrote), "{wrote:?} is news");
        }
    }

    #[test]
    fn a_burst_that_is_half_reads_reaches_the_panes_as_the_other_half() {
        use notify::Event;
        use notify::event::{DataChange, ModifyKind};

        // The wiring, and it is asserted here rather than left to the live test
        // below because that one cannot fail anywhere but Linux. Without this,
        // the filter could be lifted straight back out of `Watch::start` and
        // every test in this file would still pass.
        let fx = Fixture::new("watch-sift");
        let read = fx.touch("read.md");
        let wrote = fx.touch("wrote.md");

        let at = Instant::now();
        let change = sift(
            fx.root(),
            [
                DebouncedEvent::new(
                    Event::new(EventKind::Access(AccessKind::Open(AccessMode::Any)))
                        .add_path(read.clone()),
                    at,
                ),
                DebouncedEvent::new(
                    Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
                        .add_path(wrote.clone()),
                    at,
                ),
            ],
        );

        assert_eq!(
            change.markdown,
            std::slice::from_ref(&wrote),
            "the document somebody only read was put in front of the reader"
        );
        assert_eq!(
            change.changed,
            std::slice::from_ref(&wrote),
            "...and git was asked to refresh over it"
        );
        assert!(!change.overflowed);
    }

    /// The mirror of the live test above, and the pair is the point: that one
    /// proves a write is reported, this one proves a read is not.
    ///
    /// It can only *fail* on Linux — `IN_OPEN` has no counterpart in
    /// `ReadDirectoryChangesW`, FSEvents or kqueue, so on every other platform
    /// this passes without the filter existing. It is still run everywhere,
    /// because the rule is one rule and the day a backend grows an equivalent
    /// is the day this should start failing there too.
    #[test]
    fn a_real_watcher_says_nothing_about_a_file_that_was_only_read() {
        let fx = Fixture::new("watch-read");
        // Before the watch, so the only thing this test can be about is the
        // read below.
        let note = fx.touch("note.md");
        let watch = Watch::start(fx.root()).expect("watch a temp directory");

        assert_eq!(std::fs::read(&note).expect("read the document"), b"x");

        // Several debounces, which is long enough that an event still on its
        // way would have arrived.
        let settle = Instant::now() + Duration::from_millis(1500);
        let mut seen = Change::default();
        while Instant::now() < settle {
            seen.absorb(watch.drain());
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            seen.is_empty(),
            "reading a file was reported as changing it: {seen:?}"
        );

        // ...and the watcher was alive the whole time, which is what stops the
        // assertion above from passing for the worst possible reason.
        std::fs::write(&note, b"# hello\n").expect("write");
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            seen.absorb(watch.drain());
            if !seen.markdown.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(
            seen.markdown.last().map(|p| p.file_name()),
            Some(Some(std::ffi::OsStr::new("note.md"))),
            "the watcher stopped reporting writes too"
        );
    }
}
