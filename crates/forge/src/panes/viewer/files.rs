//! Finding the markdown that is already there.
//!
//! Noticing *new* markdown is the shell's job — one watcher serves both panes,
//! so it lives in `crate::watch` and this module borrows its idea of what is
//! worth reading. What is left here is the startup walk, so the pane opens on
//! the newest document instead of sitting empty until something changes.
//!
//! Only markdown is listed. Claude touches source files constantly and a list
//! that follows every one of them is a list nobody can read; `ViewerPane::show`
//! still renders anything it is handed, with highlighting.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use crate::watch::{in_noise, is_markdown};

/// Stop walking after this many entries. A monorepo's worth of gitignored
/// noise is not worth a second of a worker thread.
const MAX_ENTRIES: usize = 50_000;

/// How much of the sorted result to keep. Nobody tabs through two hundred
/// files; the tail is only there to be the tail.
const KEEP: usize = 200;

/// Walk `root` for markdown, newest first. Returns immediately; the answer
/// arrives on the channel when the worker is done.
pub fn spawn_scan(root: PathBuf) -> Receiver<Vec<PathBuf>> {
    let (tx, rx) = mpsc::channel();
    // Detached on purpose. It holds nothing the pane needs to reclaim, and a
    // join on shutdown would be a wait on a filesystem walk.
    std::thread::spawn(move || {
        let _ = tx.send(scan(&root));
    });
    rx
}

fn scan(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
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
        if !is_markdown(path) || in_noise(root, path) {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        found.push((modified, path.to_path_buf()));
    }

    found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    found.truncate(KEEP);
    found.into_iter().map(|(_, p)| p).collect()
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

        let found = scan(dir.path());
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
        assert!(scan(&dir.path().join("gone")).is_empty());
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
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["plan.md"]);
    }
}
