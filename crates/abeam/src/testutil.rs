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
        // **Resolved, because `main` resolves.** `crate::main` calls
        // `crate::paths::resolve_root` on `current_dir` before it builds
        // anything from the answer, so every root in the program is a resolved
        // one. A fixture that skipped that step would be handing the code under
        // test a root the program can never actually be given, and the tests
        // that compare it with a path from somewhere else — git's
        // `worktree list`, `notify`, `fs::canonicalize` — would be asking
        // whether two spellings agree rather than whether the code is right.
        //
        // `std::env::temp_dir` is exactly where that bites. It answers with
        // `%TEMP%`, and on a Windows machine whose user name is longer than
        // eight characters `%TEMP%` is an 8.3 short name —
        // `C:\Users\RUNNER~1\AppData\Local\Temp` on every GitHub runner, where
        // git and `canonicalize` both say `runneradmin`. On a five-letter user
        // the two forms coincide and nothing here is visible, which is how a
        // whole class of test defect stayed hidden on one desktop and failed
        // five ways on a build server. `/var` symlinked to `/private/var` on
        // macOS is the same fixture telling the same lie in the other dialect.
        //
        // Resolved *after* the directory exists, because `canonicalize` opens
        // what it is given. A failure falls back to the path as written, which
        // is `resolve_root`'s own promise and leaves the fixture exactly as it
        // was before this line.
        Self(crate::paths::resolve_root(&path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }

    /// The same file, plus the one bit that decides whether it is a program.
    ///
    /// Unix-only because the question is: what a file called `claude` with a
    /// `#!` line on its first line *is* over there is decided by its mode,
    /// where on Windows it is decided by its extension and [`write`](Self::write)
    /// already says everything there is to say. A test that needs the
    /// difference between a shim somebody can run and the same bytes they
    /// cannot needs this to be a call rather than four lines of
    /// `set_permissions` copied about, because the four lines are what a test
    /// quietly leaves out.
    ///
    /// `0o755` rather than `0o700`: it is what `npm` and every installer leave
    /// behind, and a mode a test writes should be a mode a machine really has.
    #[cfg(unix)]
    pub fn write_exec(&self, name: &str, bytes: &[u8]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = self.write(name, bytes);
        let mut mode = std::fs::metadata(&path)
            .expect("stat temp file")
            .permissions();
        mode.set_mode(0o755);
        std::fs::set_permissions(&path, mode).expect("chmod temp file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best effort. A failure here means a stale directory under whatever
        // `std::env::temp_dir` answered with — `%TEMP%` on Windows, `$TMPDIR`
        // or `/tmp` on Unix — and not a failed test, and panicking in a drop
        // during unwind aborts the run.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
