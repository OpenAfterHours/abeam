//! `PtySession` is the seam the restructure created: new code sitting directly
//! on top of the most fragile thing in the project.
//!
//! `conpty.rs` proves what ConPTY does. These prove that our wrapper still does
//! the right thing about it — that a caller who has never heard of a Device
//! Status Report gets a working session anyway, that resizing moves the pty and
//! the parser together, that the view into the scrollback goes where it is put
//! and stays there while output arrives, and that a dropped session does not
//! leave the child's children running.
//!
//! Scrollback lives down here rather than in the pane that drives it because
//! this is the layer where a child can be made to produce a known number of
//! lines and then stopped.
//!
//! Ungated, unlike `conpty.rs`: every claim above is a claim about our wrapper
//! rather than about a pseudoconsole, and all six hold on a Unix pty. It is also
//! the only coverage `src/tree/unix.rs` has, and the last test in this file is
//! the whole of it — see the comment there before weakening what it spawns.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use abeam_pty::{PtyConfig, PtySession};

/// A scratch directory. `crate::testutil::TempDir` belongs to the other crate.
struct Dir(std::path::PathBuf);

impl Dir {
    /// A process id and a counter, because these tests run in parallel and a
    /// fixed name would have two of them in one directory.
    ///
    /// Not the thread id, which is the obvious source of both: `ThreadId(2)`
    /// debug-prints with brackets in it, and `cmd` parses a path containing
    /// those as command syntax, so a batch file under one silently does nothing
    /// at all. `/bin/sh` is untroubled by brackets and would never have shown
    /// it — one naming scheme that is safe everywhere beats two that are each
    /// safe somewhere.
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "abeam-pty-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create a scratch directory");
        Dir(path)
    }

    fn write(&self, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, body).expect("write a scratch file");
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// How to print a file. The one thing these tests ask a shell for that the two
/// shells spell differently.
#[cfg(windows)]
const PRINT_FILE: &str = "type";
#[cfg(unix)]
const PRINT_FILE: &str = "cat";

/// A shell that runs `command` and then exits.
///
/// The two helpers below are the whole of the platform difference in this file.
/// They deliberately stop at "which shell, asked how": what each test runs stays
/// at the test, because a reader who cannot see what was spawned cannot tell
/// what the assertion means.
#[cfg(windows)]
fn shell_running(command: &str) -> PtyConfig {
    PtyConfig::new("cmd.exe").args(["/c", command])
}
#[cfg(unix)]
fn shell_running(command: &str) -> PtyConfig {
    PtyConfig::new("/bin/sh").args(["-c", command])
}

/// A shell that runs `command` if there is one and then stays at a prompt, so
/// the child is still there afterwards and the reader thread is still attached
/// to something.
#[cfg(windows)]
fn shell_staying(command: Option<&str>) -> PtyConfig {
    match command {
        Some(c) => PtyConfig::new("cmd.exe").args(["/k", c]),
        None => PtyConfig::new("cmd.exe").arg("/k"),
    }
}
#[cfg(unix)]
fn shell_staying(command: Option<&str>) -> PtyConfig {
    match command {
        // `exec`, so that what is left at the prompt is the process the session
        // spawned rather than a child of it. `try_wait` and the kill on drop
        // both mean that process, and a test that let a second shell in would be
        // asserting about the wrong one.
        Some(c) => PtyConfig::new("/bin/sh")
            .arg("-c")
            .arg(format!("{c}; exec /bin/sh")),
        None => PtyConfig::new("/bin/sh"),
    }
}

/// Poll until `f` holds, or fail. Nothing here can be waited on directly:
/// `try_wait` is the only reaping call there is (`docs/conpty-findings.md`,
/// constraint 2) and output arrives on a thread of its own.
fn until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("waited for {what} and it never happened");
}

/// The whole point of the crate, as one assertion: nobody outside answered the
/// DSR query, and the child ran anyway. On a Unix pty there is no query to
/// answer, so the same test says something smaller and still worth having — a
/// child spawned into a pty runs, exits, and arrives on the screen.
#[test]
fn session_answers_dsr_without_the_caller_helping() {
    let mut session =
        PtySession::spawn(shell_running("echo abeam-session-marker").size(24, 80)).expect("spawn");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut exited = false;
    while Instant::now() < deadline {
        if session.try_wait().expect("try_wait").is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        exited,
        "child never exited — dsr_replies = {}, which on Windows is the first \
         thing to check when a session looks dead. On Linux nothing asks, so a \
         zero there means nothing at all and the fault is elsewhere",
        session.stats().dsr_replies
    );

    // The child is gone but its last writes may still be in flight.
    std::thread::sleep(Duration::from_millis(300));

    let stats = session.stats();
    // ConPTY opens every session with `ESC [ 6 n` and blocks until it is
    // answered; a Unix pty opens by saying nothing. So this is the crate's
    // entire reason for existing on one platform and vacuous on the other, and
    // asserting it everywhere would only mean asserting `0 > 0` on Linux.
    #[cfg(windows)]
    assert!(
        stats.dsr_replies > 0,
        "session must answer the startup query itself"
    );
    assert!(stats.bytes_read > 0, "no output reached the parser");

    let contents = session.with_screen(|s| s.contents());
    assert!(
        contents.contains("abeam-session-marker"),
        "expected marker in rendered screen, got: {contents:?}"
    );
}

/// The README's "pty size vs parser size must agree" diagnostic, automated.
#[test]
fn session_resize_keeps_pty_and_parser_in_agreement() {
    let session = PtySession::spawn(shell_staying(None).size(24, 80)).expect("spawn");

    // Let the shell finish starting before resizing under it.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(session.size(), (24, 80));
    assert_eq!(session.pty_size().expect("get_size"), (24, 80));

    for (rows, cols) in [(40u16, 120u16), (20, 60)] {
        session.resize(rows, cols).expect("resize");
        assert_eq!(session.size(), (rows, cols), "parser did not follow");
        assert_eq!(
            session.pty_size().expect("get_size"),
            (rows, cols),
            "pty did not follow"
        );
    }

    // A resize to the size we already are must not churn the pty — the event
    // loop calls this on every window event.
    let before = session.stats().resizes;
    session.resize(20, 60).expect("no-op resize");
    assert_eq!(session.stats().resizes, before);
}

/// How much history the tests below reach into, with margin. Waiting for less
/// than they ask for is the whole of the bug described on [`with_history`].
const HISTORY: usize = 8;

/// Sixty lines into a six-row pty, so there is a known amount behind the screen.
///
/// Waits for **depth** and then for **quiet**, and both halves are load-bearing.
///
/// Depth, because the first row scrolls off long before the sixtieth, and a
/// machine slower than the one this was written on ends up somewhere in
/// between. This fixture used to wait for *any* scrollback at all: it passed on
/// every desktop it ran on and failed on the first CI runner, which arrived at
/// `scroll_by(4)` with three rows of history to move through.
///
/// Quiet, because the reader thread moves the offset itself as rows arrive, to
/// keep a scrolled-back view where its reader left it — that is the behaviour
/// `a_reader_who_has_scrolled_back_stays_where_they_are_looking` exists to pin.
/// One row arriving between a `scroll_by` and the assertion after it is an
/// off-by-one in a test that is *right* about everything it means to say.
fn with_history(dir: &Dir) -> PtySession {
    let body: String = (1..=60).map(|i| format!("line-{i}\r\n")).collect();
    dir.write("many.txt", &body);
    // Staying rather than exiting, so the child is still there afterwards and
    // the reader thread is still attached to something: the anchoring assertion
    // below needs both.
    let session = PtySession::spawn(
        shell_staying(Some(&format!("{PRINT_FILE} many.txt")))
            .cwd(&dir.0)
            .size(6, 30),
    )
    .expect("spawn");

    until("enough output to scroll a six-row screen past HISTORY rows", || {
        // Asked by going there and reading back where we landed. The return
        // value of `set_scrollback` answers "did the view move", which a
        // one-row history satisfies as readily as a fifty-row one — asking it
        // instead is how the too-weak wait got written in the first place.
        session.set_scrollback(HISTORY);
        let reached = session.scrollback();
        session.set_scrollback(0);
        reached >= HISTORY
    });

    // Three consecutive quiet polls rather than one: the file finishes printing
    // and then the shell prints its prompt, so a single quiet sample can land in
    // the gap between the two and call a pause the end.
    let (mut last, mut quiet) = (0, 0);
    until("the child to stop printing", || {
        let read = session.stats().bytes_read;
        quiet = if read == last { quiet + 1 } else { 0 };
        last = read;
        quiet >= 3
    });

    session
}

#[test]
fn the_view_into_the_scrollback_goes_where_it_is_put_and_says_whether_it_moved() {
    let dir = Dir::new("scrollback");
    let session = with_history(&dir);

    assert_eq!(session.scrollback(), 0, "a session starts on the live screen");
    // `false` is not a failure, it is the answer a pane needs: a key that moved
    // nothing must not cost a frame, and a frame here redraws the agent as well.
    assert!(!session.scroll_by(-1), "already at the bottom");

    assert!(session.scroll_by(4));
    assert_eq!(session.scrollback(), 4);
    assert!(session.scroll_by(-3));
    assert_eq!(session.scrollback(), 1);

    // Clamped to the history that exists rather than refused: the caller asks
    // for a direction and a distance, not for a guarantee there is that much.
    assert!(session.set_scrollback(usize::MAX));
    let oldest = session.scrollback();
    assert!(oldest > 0 && oldest < usize::MAX, "clamped to {oldest}");
    assert!(!session.scroll_by(1), "there is nothing older to reach");

    assert!(session.set_scrollback(0));
    assert_eq!(session.scrollback(), 0);
}

#[test]
fn a_reader_who_has_scrolled_back_stays_where_they_are_looking() {
    // The behaviour that makes the relative `scroll_by` necessary rather than
    // merely tidy. The reader thread advances this same offset as rows arrive,
    // so that output does not drag the view down under someone reading it —
    // and so a caller who read the offset, then wrote a new one, would compute
    // from a base that had already moved.
    let dir = Dir::new("anchor");
    let session = with_history(&dir);

    session.set_scrollback(3);
    let anchored = session.with_screen(|s| s.contents());

    session
        .write(format!("{PRINT_FILE} many.txt\r").as_bytes())
        .expect("write to the child");
    until("the child to produce another sixty lines", || {
        session.stats().bytes_read > 0 && session.scrollback() > 3
    });

    // The offset moved, and the *rows on screen* did not — which is the half
    // that matters, and the half a caller cannot arrange for itself.
    assert_eq!(
        session.with_screen(|s| s.contents()),
        anchored,
        "output pulled the view along with it"
    );
}

/// Is there still a process with this id? Signal 0 delivers nothing and runs
/// only the checks that precede delivery, which is the one liveness question
/// that does not depend on the process cooperating by writing a file.
///
/// A zombie answers `true` as well, being a process table entry like any other.
/// That is harmless under an `init` that reaps promptly — every desktop, and the
/// CI runners — but under a pid 1 that does not reap, a bare container say, an
/// orphan can linger as one and the wait below would time out on a kill that
/// worked. If that ever happens the fix is to read the state out of
/// `/proc/<pid>/stat`, not to go back to asking the grandchild for a file.
#[cfg(unix)]
fn alive(pid: libc::pid_t) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn a_dropped_session_does_not_leave_the_childs_children_running() {
    // The test for `src/tree`, and the only one. Killing a process kills that
    // process: `TerminateProcess` reaches one, and so does `SIGKILL`. The
    // `cargo build` a shell started is not that process — Windows keeps no
    // relationship for anything to walk afterwards, and on Unix the orphan is
    // `init`'s the moment its parent goes — so without a job object on one side
    // and a process group on the other, Alt+S, cargo build, Alt+Q leaves the
    // build running against a terminal being torn down.
    //
    // Both halves below spawn a grandchild that outlives its parent, wait until
    // it certainly exists, drop the session, and require it to be gone. Both
    // are also written to survive the platform's own cleanup, which would
    // otherwise do this test's work for it and let `src/tree` be deleted with
    // everything still green. That is the part to read before changing what
    // they spawn.
    let dir = Dir::new("orphan");

    #[cfg(windows)]
    {
        let started = dir.0.join("started.txt");
        let finished = dir.0.join("finished.txt");
        // Two markers, and the first is what keeps this test honest. Asserting
        // only that the second never appears would pass just as well if the
        // grandchild had never run — a broken fixture and a working job object
        // are the same empty directory.
        let script = dir.write(
            "spawn-a-grandchild.cmd",
            // `start /min` rather than `start /b`, so the grandchild gets a
            // console of its own. That is the case a job object is the only
            // answer to: closing the pseudoconsole takes the processes attached
            // to *it* down with it, which quietly disguises the problem for
            // anything that stayed attached, and a `cargo build` is precisely
            // the sort of long-running thing that need not have.
            "@echo off\r\n\
             start \"\" /min cmd.exe /c \"echo x > %~dp0started.txt & ping -n 8 127.0.0.1 >nul \
             & echo x > %~dp0finished.txt\"\r\n",
        );

        let mut session = PtySession::spawn(
            PtyConfig::new("cmd.exe")
                .args(["/c", &script.to_string_lossy()])
                .cwd(&dir.0)
                .size(24, 80),
        )
        .expect("spawn");

        until("the grandchild to start", || started.exists());
        until("the direct child to exit", || {
            session.try_wait().expect("try_wait").is_some()
        });
        // Still counting down at this point, which is the only interesting
        // moment to drop the session at.
        assert!(
            !finished.exists(),
            "the grandchild finished before the test began"
        );
        drop(session);

        // Comfortably past its own timer: still running, it would have written.
        std::thread::sleep(Duration::from_secs(10));
        assert!(
            !finished.exists(),
            "a grandchild outlived the session that started it"
        );
    }

    #[cfg(unix)]
    {
        // `sh -c` runs without job control, so the backgrounded `sleep` stays in
        // the process group its parent leads instead of being given one of its
        // own. That is the case `killpg` is the answer to, and the shape a real
        // `cargo build` under a non-interactive shell has.
        //
        // `trap '' HUP` is what keeps this honest, and it is this branch's
        // counterpart to the other one's `start /min`. When the direct child
        // exits it is a session leader losing its controlling terminal, and the
        // kernel sends `SIGHUP` to the foreground process group on its way out
        // — which would take the grandchild with it and leave this test passing
        // with `src/tree/unix.rs` deleted. An ignored disposition survives both
        // `fork` and `exec`, so the `sleep` inherits it, and after that nothing
        // short of a signal that cannot be caught will end it.
        let started = dir.0.join("started");
        let mut session = PtySession::spawn(
            PtyConfig::new("/bin/sh")
                .args(["-c", "trap '' HUP; sleep 30 & echo $! > started"])
                .cwd(&dir.0)
                .size(24, 80),
        )
        .expect("spawn");

        // The pid *is* the marker: the file existing is not enough, because the
        // shell creates it before it has written the number into it.
        let mut grandchild = 0;
        until("the grandchild to start", || {
            grandchild = std::fs::read_to_string(&started)
                .ok()
                .and_then(|s| s.trim().parse::<libc::pid_t>().ok())
                .unwrap_or(0);
            grandchild > 0
        });
        until("the direct child to exit", || {
            session.try_wait().expect("try_wait").is_some()
        });
        // Twenty-odd seconds left on its clock at this point, which is the only
        // interesting moment to drop the session at.
        assert!(
            alive(grandchild),
            "the grandchild was gone before the session was dropped"
        );
        drop(session);

        // Bounded rather than a flat sleep, because the kill is a signal and
        // arrives when the scheduler gets to it; and asked of the process table
        // rather than of a file the grandchild would have had to write.
        until("the grandchild to go with the session that started it", || {
            !alive(grandchild)
        });
    }
}

/// The doorbell. Without it a draw loop can only poll, and polling is a floor
/// on latency that has nothing to do with how fast anything is.
#[test]
fn output_rings_the_waker_and_a_session_nobody_installed_one_on_still_works() {
    let rings = Arc::new(AtomicU32::new(0));
    // Keep the child waiting for input until the waker is installed. A one-shot
    // `/bin/sh -c 'echo ...'` can write and exit between `spawn` returning and
    // this thread reaching `wake_on_output`; ConPTY's startup handshake used to
    // hide that race when this test was Windows-only.
    let session = PtySession::spawn(shell_staying(None).size(24, 80)).expect("spawn");

    session.wake_on_output({
        let rings = Arc::clone(&rings);
        move || {
            rings.fetch_add(1, Ordering::Relaxed);
        }
    });
    session
        .write(b"echo abeam-waker-marker\r")
        .expect("write to the child");

    until("the waker to be rung", || rings.load(Ordering::Relaxed) > 0);

    // The ring is not the news — `take_dirty` is — so the two must agree, and
    // the flag must still be set by the time anyone gets round to reading it.
    assert!(session.take_dirty(), "rung without anything to show for it");

    // The reader thread is the only caller, so a ring per read is the most that
    // can have happened, and a shell starting plus one `echo` is only a handful
    // of those. This is not about the exact number: it is that nothing is
    // ringing in a loop.
    let rung = rings.load(Ordering::Relaxed);
    assert!(rung <= 8, "{rung} rings for one line of output");

    // ...and the default is silence, not a panic. Every test above this one is
    // a session with no waker installed, but only this one says so on purpose.
    let session =
        PtySession::spawn(shell_running("echo abeam-no-waker-marker").size(24, 80)).expect("spawn");
    until("output to arrive with nobody listening", || {
        session.stats().bytes_read > 0
    });
}
