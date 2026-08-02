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

#![cfg(windows)]

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use abeam_pty::{PtyConfig, PtySession};

/// A scratch directory. `crate::testutil::TempDir` belongs to the other crate.
struct Dir(std::path::PathBuf);

impl Dir {
    /// A process id and a counter rather than the thread id: `ThreadId(2)`
    /// debug-prints with brackets in it, and `cmd` parses a path containing
    /// those as command syntax, so a batch file under one silently does
    /// nothing at all.
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
/// DSR query, and the child ran anyway.
#[test]
fn session_answers_dsr_without_the_caller_helping() {
    let mut session = PtySession::spawn(
        PtyConfig::new("cmd.exe")
            .arg("/c")
            .arg("echo abeam-session-marker")
            .size(24, 80),
    )
    .expect("spawn");

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
        "child never exited — dsr_replies = {}, which is the first thing to \
         check when a session looks dead",
        session.stats().dsr_replies
    );

    // The child is gone but its last writes may still be in flight.
    std::thread::sleep(Duration::from_millis(300));

    let stats = session.stats();
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
    let session =
        PtySession::spawn(PtyConfig::new("cmd.exe").arg("/k").size(24, 80)).expect("spawn");

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

    // A resize to the size we already are must not churn ConPTY — the event
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
    // `/k`, so the child is still there afterwards and the reader thread is
    // still attached to something: the anchoring assertion below needs both.
    let session = PtySession::spawn(
        PtyConfig::new("cmd.exe")
            .args(["/k", "type", "many.txt"])
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

    // Three consecutive quiet polls rather than one: `type` finishes and then
    // `cmd` prints its prompt, so a single quiet sample can land in the gap
    // between the two and call a pause the end.
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
    // nothing must not cost a frame, and a frame here redraws Claude as well.
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

    session.write(b"type many.txt\r").expect("write to the child");
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

#[test]
fn a_dropped_session_does_not_leave_the_childs_children_running() {
    // `TerminateProcess` reaches one process. The `cargo build` a shell started
    // is not that process, and Windows keeps no relationship for anything to
    // walk afterwards — so without a job object, Alt+S, cargo build, Alt+Q
    // leaves `cargo.exe` running against a pseudoconsole being torn down.
    let dir = Dir::new("orphan");
    let started = dir.0.join("started.txt");
    let finished = dir.0.join("finished.txt");
    // Two markers, and the first is what keeps this test honest. Asserting only
    // that the second never appears would pass just as well if the grandchild
    // had never run — a broken fixture and a working job object are the same
    // empty directory.
    let script = dir.write(
        "spawn-a-grandchild.cmd",
        // `start /min` rather than `start /b`, so the grandchild gets a console
        // of its own. That is the case a job object is the only answer to:
        // closing the pseudoconsole takes the processes attached to *it* down
        // with it, which quietly disguises the problem for anything that stayed
        // attached, and a `cargo build` is precisely the sort of long-running
        // thing that need not have.
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
    // Still counting down at this point, which is the only interesting moment
    // to drop the session at.
    assert!(!finished.exists(), "the grandchild finished before the test began");
    drop(session);

    // Comfortably past its own timer: still running, it would have written.
    std::thread::sleep(Duration::from_secs(10));
    assert!(
        !Path::new(&finished).exists(),
        "a grandchild outlived the session that started it"
    );
}
