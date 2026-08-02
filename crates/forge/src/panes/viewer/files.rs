//! Finding the files that are already there.
//!
//! Noticing *new* markdown is the shell's job — one watcher serves both panes,
//! so it lives in `crate::watch` and this module borrows its idea of what is
//! worth reading. What is left here is the startup walk, so the pane opens on
//! the newest document instead of sitting empty until something changes.
//!
//! One walk answers two questions. `Tab` wants markdown in recency order,
//! because Claude touches source files constantly and a *recency* list that
//! follows every one of them is a list nobody can read. The find in `browse`
//! wants the opposite — every file there is, so that "view any file" is true —
//! and it wants them by name rather than by age. Both fall out of the same
//! traversal, and they have to: a second gitignore walk of the repository
//! would double the disk cost of startup to re-derive what the first one
//! already had in its hand.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use super::browse::ci_cmp;
use crate::watch::{in_noise, is_markdown};

/// Stop walking after this many entries. A monorepo's worth of gitignored
/// noise is not worth a second of a worker thread.
const MAX_ENTRIES: usize = 50_000;

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
const MAX_FILES: usize = 20_000;

/// What one walk of the root answers with.
pub struct Scan {
    /// Markdown under the root, newest first. `Tab` walks it.
    pub recent: Vec<PathBuf>,
    /// Every file under the root, as [`rel`] spells them, sorted by name. The
    /// find index.
    pub files: Vec<String>,
}

/// Walk `root`, gitignore-aware. Returns immediately; the answer arrives on
/// the channel when the worker is done.
pub fn spawn_scan(root: PathBuf) -> Receiver<Scan> {
    let (tx, rx) = mpsc::channel();
    // Detached on purpose. It holds nothing the pane needs to reclaim, and a
    // join on shutdown would be a wait on a filesystem walk.
    std::thread::spawn(move || {
        let _ = tx.send(scan(&root));
    });
    rx
}

fn scan(root: &Path) -> Scan {
    let mut markdown: Vec<(SystemTime, PathBuf)> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    // `ignore` reads .gitignore for us, which is the difference between a file
    // list and a list of build artefacts.
    let walk = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .parents(true)
        .build();

    for entry in walk.take(MAX_ENTRIES).flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if in_noise(root, path) {
            continue;
        }
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

    markdown.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    markdown.truncate(KEEP);
    // Sorted here, on the worker, because an empty query shows the index as it
    // stands and that listing is read by a person.
    files.sort_unstable_by(|a, b| ci_cmp(a, b));

    Scan {
        recent: markdown.into_iter().map(|(_, p)| p).collect(),
        files,
    }
}

/// A path as the index holds it: relative to the root, with `/` separators on
/// every platform.
///
/// Both halves earn their place. Relative, because that is the path a reader
/// recognises and the one a query gets typed against — nobody searches for
/// `C:\Users\...`. And `/`, because on Windows the walk spells the same path
/// with backslashes, so a query typed as `src/panes` would otherwise match
/// nothing at all.
fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::time::Duration;

    #[test]
    fn the_scan_finds_markdown_newest_first_and_ignores_the_rest() {
        let dir = TempDir::new("scan");
        dir.write("old.md", b"# old\n");
        dir.write("src.rs", b"fn main() {}\n");
        // mtime resolution on this filesystem is coarse enough that two writes
        // in a row can tie; a real gap is the only reliable ordering.
        std::thread::sleep(Duration::from_millis(20));
        dir.write("new.md", b"# new\n");

        let found = scan(dir.path()).recent;
        let names: Vec<String> = found
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["new.md", "old.md"]);
    }

    #[test]
    fn scanning_somewhere_that_does_not_exist_yields_nothing() {
        let dir = TempDir::new("scan-missing");
        let found = scan(&dir.path().join("gone"));
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
        std::fs::create_dir_all(&root).expect("create nested root");
        std::fs::write(root.join("plan.md"), b"# plan\n").expect("write");
        // ...while noise *inside* the root is still noise.
        std::fs::create_dir_all(root.join("target")).expect("create target");
        std::fs::write(root.join("target").join("built.md"), b"# built\n").expect("write");

        let names: Vec<String> = scan(&root)
            .recent
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["plan.md"]);
    }

    #[test]
    fn the_same_walk_answers_with_every_file_and_not_only_the_markdown() {
        // The find is the half of the file list that answers "view *any*
        // file", so an index that inherited the markdown filter would leave it
        // able to find only what `Tab` already reached.
        let dir = TempDir::new("scan-index");
        dir.write("plan.md", b"# plan\n");
        std::fs::create_dir_all(dir.path().join("src")).expect("create src");
        std::fs::write(dir.path().join("src").join("main.rs"), b"fn main() {}\n").expect("write");
        std::fs::create_dir_all(dir.path().join("target")).expect("create target");
        std::fs::write(dir.path().join("target").join("out.rs"), b"//\n").expect("write");

        let found = scan(dir.path());
        // Root-relative, `/`-separated, sorted, and without the build output.
        assert_eq!(found.files, ["plan.md", "src/main.rs"]);
        assert_eq!(found.recent.len(), 1, "the markdown list is unchanged");
    }
}
