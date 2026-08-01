//! `PtySession` is the seam the restructure created: new code sitting directly
//! on top of the most fragile thing in the project.
//!
//! `conpty.rs` proves what ConPTY does. These two prove that our wrapper still
//! does the right thing about it — that a caller who has never heard of a
//! Device Status Report gets a working session anyway, and that resizing moves
//! the pty and the parser together.

#![cfg(windows)]

use std::time::{Duration, Instant};

use forge_pty::{PtyConfig, PtySession};

/// The whole point of the crate, as one assertion: nobody outside answered the
/// DSR query, and the child ran anyway.
#[test]
fn session_answers_dsr_without_the_caller_helping() {
    let mut session = PtySession::spawn(
        PtyConfig::new("cmd.exe")
            .arg("/c")
            .arg("echo forge-session-marker")
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
        contents.contains("forge-session-marker"),
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
