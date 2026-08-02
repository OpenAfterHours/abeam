//! A throwaway directory for the tests that need a real file.
//!
//! Most of what abeam has to survive — a path that is gone, a directory where a
//! file was, a 200 MB blob, bytes that are not text, a watcher firing on a file
//! that has already been deleted — cannot honestly be faked behind a trait. The
//! whole of it is a unique directory under the system temp, removed on drop.
//!
//! At the crate root rather than inside the viewer, because the watcher and the
//! shell need real files too, and a helper only one module can reach is a
//! helper the next test rewrites.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        // Process id and a counter: `cargo test` runs these in parallel
        // threads of one process, and two runs can overlap.
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("abeam-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best effort. A failure here means a stale directory in %TEMP%, not a
        // failed test, and panicking in a drop during unwind aborts the run.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
