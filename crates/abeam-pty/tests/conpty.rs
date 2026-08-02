//! Does the ConPTY -> vt100 pipeline actually work on this machine?
//!
//! These don't test the UI (that needs a human at a terminal). They test the
//! layer underneath it: spawn a real process under a real pty, and confirm its
//! output arrives and parses into a screen we could render.
//!
//! ## The finding that nearly sank the spike
//!
//! ConPTY opens every session by writing a Device Status Report query —
//! `ESC [ 6 n`, "where is the cursor?" — and **blocks until the host answers**.
//! A host that ignores it sees exactly four bytes of output and a child that
//! never runs its command, never exits, and never reports a status. It looks
//! for all the world like a dead pty or a `wait()` bug.
//!
//! Answering it with `ESC [ row ; col R` makes everything work: the same
//! `cmd /c echo hi` then completes in under half a second.
//!
//! Two corollaries, both baked into `session.rs`:
//!
//! - Answer DSR from the reader thread, which means the pty writer has to be
//!   shared with the input loop rather than owned by it.
//! - Use `try_wait()` polling, never `wait()`. Even once DSR is answered, the
//!   reader has no reliable EOF to block on.
//!
//! These deliberately hand-roll their own pty plumbing instead of using
//! `PtySession`. They pin *ConPTY's* behaviour, not ours; routing them through
//! our own code would mean a bug in `session.rs` could make them agree with it.

// Gated, and staying gated now that the crate is not Windows-only: these pin
// ConPTY's own behaviour, and the thing they pin hardest — a child that stalls
// until `ESC [ 6 n` is answered — has no Unix analogue to write a weaker version
// of. A Unix pty asks nothing at startup, so on Linux this file would not be the
// same test proving less, it would be a test of nothing. `tests/session.rs` is
// where the cross-platform half lives and it runs everywhere.
#![cfg(windows)]

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Poll for exit rather than blocking in `wait()`. Returns false on timeout.
fn wait_for_exit(
    child: &mut Box<dyn portable_pty::Child + Send + Sync>,
    within: Duration,
) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait").is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

use abeam_pty::vt100;
use portable_pty::{CommandBuilder, PtySize};

const ROWS: u16 = 24;
const COLS: u16 = 80;

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Run a command under a real pty and return the parsed screen state.
///
/// Note when asserting: `Screen::contents()` rejoins wrapped rows into logical
/// lines, so it cannot tell you anything about wrapping. Use `Screen::rows()`
/// for anything layout-related.
fn render(args: &[&str]) -> vt100::Parser {
    let sys = portable_pty::native_pty_system();
    let pair = sys.openpty(size(ROWS, COLS)).expect("openpty");

    let mut cmd = CommandBuilder::new("cmd.exe");
    for a in args {
        cmd.arg(a);
    }

    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let collected = Arc::new(Mutex::new(Vec::new()));
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    {
        let collected = Arc::clone(&collected);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                // Without this the child never runs. See the module docs.
                if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = writer.write_all(b"\x1b[1;1R");
                    let _ = writer.flush();
                }
                collected.lock().unwrap().extend_from_slice(chunk);
            }
        });
    }

    assert!(
        wait_for_exit(&mut child, Duration::from_secs(10)),
        "child never exited"
    );
    // The child is gone but its last writes may still be in flight.
    std::thread::sleep(Duration::from_millis(300));

    let raw = collected.lock().unwrap().clone();
    let mut parser = vt100::Parser::new(ROWS, COLS, 0);
    parser.process(&raw);
    parser
}

/// The core finding, as an executable regression test. If the "ignore DSR" arm
/// ever starts passing, Windows has changed and `session.rs` can be simplified.
#[test]
fn conpty_stalls_until_the_dsr_query_is_answered() {
    let run = |answer_dsr: bool| {
        let sys = portable_pty::native_pty_system();
        let pair = sys.openpty(size(ROWS, COLS)).expect("openpty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/c");
        cmd.arg("echo hi");
        let mut child = pair.slave.spawn_command(cmd).expect("spawn");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("reader");
        let mut writer = pair.master.take_writer().expect("writer");
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if answer_dsr && buf[..n].windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = writer.write_all(b"\x1b[1;1R");
                    let _ = writer.flush();
                }
            }
        });

        let exited = wait_for_exit(&mut child, Duration::from_secs(5));
        let _ = child.kill();
        exited
    };

    assert!(run(true), "answering DSR should let the child run and exit");
    assert!(
        !run(false),
        "ignoring DSR should stall the child. If this fails, ConPTY no longer \
         requires the reply and main.rs can drop its DSR handling."
    );
}

#[test]
fn child_output_reaches_the_parser() {
    let parser = render(&["/c", "echo abeam-spike-marker"]);
    let screen = parser.screen().contents();
    assert!(
        screen.contains("abeam-spike-marker"),
        "expected marker in rendered screen, got: {screen:?}"
    );
}

#[test]
fn escape_sequences_are_interpreted_not_printed() {
    // `prompt $E` is the cmd-native way to emit a real 0x1b byte. If SGR
    // handling were broken we would see the escape sequence as literal text
    // instead of it being consumed as styling.
    let parser = render(&["/c", "prompt $E[31m& echo RED"]);
    let screen = parser.screen().contents();
    assert!(
        !screen.contains('\x1b'),
        "escape sequences leaked into rendered text: {screen:?}"
    );
    assert!(
        screen.contains("RED"),
        "expected output text, got: {screen:?}"
    );
}

#[test]
fn long_output_wraps_onto_a_second_row() {
    // ConPTY does not wrap for us - it emits all 100 characters in one run and
    // leaves the host terminal to lay them out. This checks our side does it.
    let parser = render(&["/c", &format!("echo {}", "x".repeat(100))]);
    let rows: Vec<String> = parser.screen().rows(0, COLS).collect();

    let first = rows[0].trim_end();
    let second = rows[1].trim_end();

    assert_eq!(
        first.len(),
        COLS as usize,
        "first row should be filled to the pty width, got {first:?}"
    );
    assert_eq!(
        second.len(),
        100 - COLS as usize,
        "remainder should continue on the next row, got {second:?}"
    );
    assert!(second.chars().all(|c| c == 'x'), "got {second:?}");
}

#[test]
fn resize_is_accepted_while_a_child_is_attached() {
    // Resize is the flakiest part of ConPTY and the thing most likely to sink
    // the design, so it gets its own check.
    let sys = portable_pty::native_pty_system();
    let pair = sys.openpty(size(ROWS, COLS)).expect("openpty");

    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.arg("/k");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    // Let the shell finish starting before resizing under it.
    std::thread::sleep(Duration::from_millis(300));

    pair.master.resize(size(40, 120)).expect("resize");
    let got = pair.master.get_size().expect("get_size");
    assert_eq!((got.rows, got.cols), (40, 120));

    // Shrinking is the direction that reflows content, so test it too.
    pair.master.resize(size(20, 60)).expect("shrink");
    let got = pair.master.get_size().expect("get_size");
    assert_eq!((got.rows, got.cols), (20, 60));

    child.kill().expect("kill");
    assert!(
        wait_for_exit(&mut child, Duration::from_secs(10)),
        "killed child never reaped"
    );
}
