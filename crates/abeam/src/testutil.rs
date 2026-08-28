//! A throwaway directory for the tests that need a real file, and the two
//! waits that sit either side of a child: one before it can be started at
//! all, one after it has been told to go.
//!
//! Most of what abeam has to survive — a path that is gone, a directory where a
//! file was, a 200 MB blob, bytes that are not text, a watcher firing on a file
//! that has already been deleted — cannot honestly be faked behind a trait. The
//! whole of it is a unique directory under the system temp, removed on drop.
//!
//! At the crate root rather than inside the viewer, because the watcher and the
//! shell need real files too, and a helper only one module can reach is a
//! helper the next test rewrites. That last sentence is why [`until`] moved
//! here as well: two `#[cfg(test)]` modules in this crate needed the same poll
//! and there was nowhere between them to put it. The cost is that this module
//! is no longer only about a directory, which the first line now says.

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
    ///
    /// **It also starts the file once before handing it back**, which is
    /// [`past_text_file_busy`]'s doing and the whole of what separates this
    /// from [`write_exec_unrun`](Self::write_exec_unrun). Reach for this one
    /// when the test really does start what it wrote, and for that one when it
    /// only resolves, finds or reads it. The probe is not free and it is not
    /// invisible: `#!/usr/bin/env node` written through here starts whatever
    /// `node` is on the machine's `PATH`, which is not a thing a call spelled
    /// "write a file" should be doing on a test's behalf.
    ///
    /// **A shim with a side effect — one that appends to a file, counts its own
    /// invocations, or leaves a marker behind — must not go through here**, for
    /// the same reason: it will be run one more time than the test asked for.
    #[cfg(unix)]
    pub fn write_exec(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.write_exec_unrun(name, bytes);
        past_text_file_busy(&path);
        path
    }

    /// The execute bit without the throwaway run: a shim this test will
    /// *resolve* rather than start.
    ///
    /// The pair exists because the run is the expensive and surprising half,
    /// and most of the shims in this crate are never started at all.
    /// `crate::launch`'s resolver asks `access(X_OK)` and execs nothing, so a
    /// test about which file was found, or about what ended up in a `Launch`,
    /// needs the bit and only the bit — and the `ETXTBSY` window
    /// [`past_text_file_busy`] waits out is a window on an exec that never
    /// happens there.
    ///
    /// Saying so at the call site is worth more than the cycles it saves.
    /// **"This shim is never started" is a fact about the test**, and it now
    /// reads that way in one word instead of having to be worked out from what
    /// the assertions below it do not do. It also keeps the probe's own `fork`
    /// out of runs that had no use for it — see the last paragraph of
    /// [`past_text_file_busy`], which is about that fork.
    #[cfg(unix)]
    pub fn write_exec_unrun(&self, name: &str, bytes: &[u8]) -> PathBuf {
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

/// Poll until `f` holds, or fail loudly.
///
/// **There is nothing here to wait *on*, which is the whole reason this is a
/// poll rather than a block.** Ending a process is not synchronous on either
/// platform: `TerminateProcess` returns once the kill has been asked for, and
/// the pid goes on being listed by `tasklist` until the kernel has finished
/// tearing the process down and the last handle to it is closed; `SIGKILL` is
/// the same sentence in the other dialect. A single sample taken the instant
/// after a drop therefore asks the question too early and answers "still
/// there" about a process that is going — which is how `crate::app`'s quit
/// test failed once in eight full local runs and passed every single time it
/// was run on its own.
///
/// **One home for two callers, which is why it is here and not in either of
/// them.** `crate::app` and `crate::ask` both drop something and then ask the
/// operating system whether it went, and they are two `#[cfg(test)]` modules
/// with nowhere between them to put a helper — so this used to be a local `fn`
/// in one of them and a bare `assert!` in the other, which is exactly how the
/// second one came to have the defect the first one had already had fixed.
/// `abeam-pty`'s integration tests keep a third copy of this shape and cannot
/// be given this one: they are a different crate, and nothing `#[cfg(test)]`
/// in this one is reachable from outside it. That copy is the precedent this
/// follows rather than a duplicate to be collapsed.
///
/// Bounded, and it panics rather than returning a `bool`, because the
/// alternative to a deadline here is a suite that hangs when the thing really
/// never happens — and a caller who has to remember to assert.
pub fn until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("waited for {what} and it never happened");
}

/// Wait until `path` can be executed at all, past the one error that means
/// "not yet".
///
/// **The race in full, because a loop like this reads like something to
/// delete.** `cargo test` runs tests in parallel threads of one process.
/// Thread A calls [`TempDir::write_exec`] and holds a write descriptor on its
/// shim for as long as the write takes. Thread B spawns a child inside that
/// window, and between B's `fork` and its `exec` that child holds a
/// *duplicate* of A's descriptor — `O_CLOEXEC` does not close that window, it
/// closes the descriptor at the exec that ends it. If A then execs its shim
/// before B's child has reached its own exec, Linux answers `ETXTBSY`: a file
/// that anybody has open for writing cannot be executed. It is the
/// multithreaded fork/exec race that Rust issue #103297 and Go's `os/exec`
/// both write up, it is nothing abeam did, and it took `crate::dispatch`'s
/// standard-input test down on CI as ``abeam could not start
/// `…/abeam-stdin`: Text file busy (os error 26)``.
///
/// **Why the wait is here and must never be in `crate::launch` or
/// `crate::dispatch`.** Those two exist to be strict about what starts, and an
/// `ETXTBSY` on a user's real agent binary is a true error — an install
/// halfway through, an editor writing over it — that has to reach them as one.
/// Retrying it there would turn "your `claude` is being rewritten as we speak"
/// into a pause followed by a program of unknown vintage. Nothing outside a
/// test reaches this file, which is the whole reason the fix lives in it.
///
/// **Why one exec here settles every later one.** The only writer this file
/// will ever have is the [`TempDir::write_exec`] above, and it has already
/// returned. So the only write descriptors that can exist are duplicates
/// leaked into children forked during that write, and every one of those
/// closes at its own exec, which is moments away. A spawn that succeeds here
/// proves none is open *now*, and nothing can open a new one afterwards — so
/// the file is executable from here on, for the life of the fixture, however
/// many times the test starts it. That is why this is one place rather than a
/// retry at each of the seven modules that write a shim and then run it.
///
/// **Why it has to really run the thing.** `ETXTBSY` is reported by `execve`
/// and by nothing else — there is no call that asks "does anybody hold this
/// open for writing" — so the probe is a real spawn, with nothing attached to
/// any of its three standard streams and killed and reaped on the spot. Every
/// shim written through [`TempDir::write_exec`] is an `echo`, a `read` and an
/// `exit`, and one extra run of one of those against `/dev/null` is invisible;
/// the shims for which that is *not* true are the ones
/// [`TempDir::write_exec_unrun`] exists for, and its doc says which is which.
///
/// **What this does not claim.** The probe is itself a `fork`, so between its
/// own fork and its exec it holds duplicates of every descriptor this process
/// has open — another thread's in-flight `fs::write` included. It is therefore
/// a small new source of the race it closes, and pretending otherwise would be
/// the weakest sentence in this comment. What saves it is that the leak is
/// self-healing rather than circular: every shim that will ever be executed is
/// now written through [`TempDir::write_exec`] and so waits its own window
/// out, which means a descriptor this probe leaks is one the next probe
/// retries past. So the honest claim is that the barrier makes the suite
/// **converge** — every such file ends up permanently executable, and no test
/// execs one before that is true — and not that the window has been removed
/// from the process. [`TempDir::write_exec_unrun`] narrows it further by not
/// forking at all where nothing is going to be started.
///
/// **Why the bound cannot hide a real failure.** `ETXTBSY` is the only error
/// retried; every other one returns at once and is left to the test's own
/// spawn, which has the better message for it. An `ETXTBSY` that outlives the
/// deadline panics carrying the original error, so a file that genuinely
/// cannot be executed fails loudly and quickly rather than hanging here or
/// passing.
#[cfg(unix)]
fn past_text_file_busy(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match std::process::Command::new(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                // Killed and reaped rather than left to finish. This run is not
                // the test's, nothing is reading a word of it, and a shim that
                // blocks on its standard input would otherwise be a process the
                // fixture leaks for as long as the suite runs.
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            Err(why) if why.raw_os_error() == Some(libc::ETXTBSY) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "`{}` was still `Text file busy` five seconds after it was \
                     written, which is far longer than any fork and exec: {why}",
                    path.display()
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // Not this race. Whatever it is, the test's own spawn is about to
            // meet it with a message written for what it was trying to do.
            Err(_) => return,
        }
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
