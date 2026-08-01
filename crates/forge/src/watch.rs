//! Noticing what the agent just did.
//!
//! This is the thing forge has that three separate windows do not. A wezterm
//! pane running lazygit next to one running glow is two programs that have to
//! be told; here, one watcher tells both panes, and neither of them has to be
//! asked.
//!
//! It lives in the shell rather than in a pane for two reasons. One recursive
//! watch of a repository root is enough — a second would double the OS-level
//! event traffic for the same information. And the two consumers want different
//! slices of the same stream: the viewer wants markdown paths, git wants to know
//! only *that* something changed. Splitting one stream is the shell's job;
//! neither pane can do it without knowing about the other.
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

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

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
/// forge from inside one — would then match on the ancestor and drop *every*
/// event, silently, while still reporting a watcher that started. The same
/// predicate walks the startup scan, so the file list would come back empty
/// too, and forge's one reason to exist would be quietly gone.
pub fn in_noise(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|c| NOISE.contains(&c))
}

/// One batch of "something happened", already split for its two readers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// Markdown the viewer should follow, in the order it was last mentioned.
    pub markdown: Vec<PathBuf>,
    /// Something under the worktree changed, so git's answer is stale. True for
    /// source files too — most of what an agent writes is not markdown, and the
    /// git pane cares about all of it.
    pub worktree: bool,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        !self.worktree && self.markdown.is_empty()
    }

    /// Fold another batch in. A rename fires under both names and a save fires
    /// more than once even after debouncing, so only the last mention of a path
    /// decides its place in the order.
    fn absorb(&mut self, other: Change) {
        self.worktree |= other.worktree;
        for path in other.markdown {
            self.markdown.retain(|p| p != &path);
            self.markdown.push(path);
        }
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
        change.worktree = true;
        if is_markdown(&path) && path.is_file() {
            change.markdown.retain(|p| p != &path);
            change.markdown.push(path);
        }
    }
    change
}

pub struct Watch {
    // Held only to keep the watcher alive; dropping it stops the thread.
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    rx: Receiver<Change>,
}

impl Watch {
    /// `None` if the platform refuses to watch — a network share, a path that
    /// vanished between startup and here. Forge degrades to manual refresh and
    /// says so on screen rather than failing to start.
    pub fn start(root: &Path) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        // The root travels with the callback because the noise filter is
        // relative to it, and `notify` reports absolute paths.
        let owned_root = root.to_path_buf();
        let debounced = move |result: DebounceEventResult| {
            let Ok(events) = result else { return };
            let change = classify(&owned_root, events.into_iter().flat_map(|e| e.event.paths));
            if !change.is_empty() {
                let _ = tx.send(change);
            }
        };

        let mut debouncer = new_debouncer(DEBOUNCE, None, debounced).ok()?;
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
        let change = fx.classify([fx.touch("src/main.rs")]);
        assert!(change.worktree);
        assert!(change.markdown.is_empty());
    }

    #[test]
    fn a_cargo_build_reaches_neither_pane() {
        let fx = Fixture::new("watch-build");
        let change = fx.classify([fx.touch("target/debug/forge.exe"), fx.touch("target/doc/x.md")]);
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
        let change = fx.classify([fx.gone("NOTES.md"), moved_to.clone()]);
        assert!(change.worktree, "something moved; git's answer is stale");
        assert_eq!(change.markdown, [moved_to], "only the surviving name");

        let change = fx.classify([fx.gone("TODO.md")]);
        assert!(change.worktree);
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
        let mut acc = fx.classify([first.clone()]);
        acc.absorb(fx.classify([fx.touch("src/lib.rs")]));
        acc.absorb(fx.classify([second.clone()]));
        assert_eq!(acc.markdown, [first, second]);
        assert!(acc.worktree);
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

        assert!(seen.worktree, "git was never told anything changed");
        assert_eq!(
            seen.markdown.last().map(|p| p.file_name()),
            Some(Some(std::ffi::OsStr::new("note.md"))),
            "the viewer was never handed the document"
        );
    }
}
