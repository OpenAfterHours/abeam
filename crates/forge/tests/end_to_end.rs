//! Forge, hosted in a pty, driven by keystrokes.
//!
//! Every other test in this repository builds a pane or an `App` in-process and
//! asks what it did. That is most of the value and none of the last mile: it
//! cannot tell you that the real binary starts, that raw mode and the alternate
//! screen survive being someone else's child, that a keystroke written as bytes
//! is decoded into the event a binding matches, or that the thing you asked for
//! is *legible* in a 46-column pane. Those are exactly the failures a user meets
//! first and a unit test never sees.
//!
//! So this suite does to forge what forge does to Claude: spawns it in a
//! ConPTY through `forge-pty`, types at it, and reads the screen that comes
//! back. The library being used to test the binary is not a shortcut — it is
//! the same code path the product runs on, which is why a bug in it fails here
//! too rather than hiding.
//!
//! The children are chosen for determinism, not realism: `cmd.exe` on both
//! sides, because `pwsh` prints a banner whose text varies by version and takes
//! a second to do it, and because the assertions are about forge rather than
//! about a shell.
//!
//! **These tests are slow by the standards of this repository** — a few seconds
//! each, against milliseconds everywhere else — because they wait on real
//! process startup. They earn it by being the only tests that would notice
//! forge failing to start at all.

#![cfg(windows)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use forge_pty::{PtyConfig, PtySession};

/// Long enough for a cold `cmd.exe` on a loaded machine, short enough that a
/// hang is a test failure rather than a coffee break. Every wait here is
/// polled, so a passing run finishes as soon as the screen says what it should
/// and never spends this.
const DEADLINE: Duration = Duration::from_secs(20);

/// A scratch directory. `crate::testutil::TempDir` is `#[cfg(test)]` inside the
/// binary and an integration test is a different crate, so this is the same
/// idea in the eight lines it takes.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "forge-e2e-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create a scratch directory");
        Dir(path)
    }

    fn write(&self, name: &str, body: &str) {
        std::fs::write(self.0.join(name), body).expect("write a scratch file");
    }

    fn mkdir(&self, name: &str) {
        std::fs::create_dir_all(self.0.join(name)).expect("create a scratch subdirectory");
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        // Best effort: on Windows a directory can stay locked briefly after the
        // process that had it as a working directory dies, and a test that
        // fails because of *that* is a test nobody trusts.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Start forge itself, hosting `cmd.exe`, in a pane-sized pty.
///
/// 120x40 is the smallest window that still splits — below `MIN_SPLIT_COLS` the
/// right pane collapses and every assertion here would be about a pane that was
/// never drawn.
fn forge(dir: &Dir) -> PtySession {
    PtySession::spawn(
        PtyConfig::new(env!("CARGO_BIN_EXE_forge"))
            .arg("cmd.exe")
            .cwd(&dir.0)
            // The command view would otherwise pick `pwsh`, whose banner and
            // startup time vary by machine. This is the seam that makes it
            // testable, and it is a user-facing setting rather than a test hook.
            .env("FORGE_SHELL", "cmd.exe")
            .size(40, 120),
    )
    .expect("spawn forge in a pty")
}

/// Everything currently on screen, wrapped rows rejoined.
///
/// `contents()` rather than `rows()` deliberately, and it is the one place in
/// this repository where that is right: these assertions ask whether a string
/// is present, never where it is, so the thing `contents()` throws away
/// (`docs/conpty-findings.md`, constraint 5) is the thing that would make them
/// flap when a pane is a column narrower than expected.
fn screen(session: &PtySession) -> String {
    session.with_screen(|s| s.contents())
}

/// Wait until `needle` is on screen, or fail with what was there instead.
fn wait_for(session: &PtySession, needle: &str) -> String {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let text = screen(session);
        if text.contains(needle) {
            return text;
        }
        if Instant::now() >= deadline {
            panic!("waited {DEADLINE:?} for {needle:?}; the screen said:\n{text}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Type at forge, as bytes on the pty exactly as a terminal would send them.
///
/// The pause is not superstition: forge drains every pending event before
/// drawing a frame, so keys sent in one burst can be handled before the frame
/// that would spawn the pane they were aimed at. A person cannot type faster
/// than a frame; a test can.
fn send(session: &PtySession, bytes: &[u8]) {
    session.write(bytes).expect("write to forge's pty");
    std::thread::sleep(Duration::from_millis(250));
}

/// `Alt`+letter, as a terminal encodes it: escape, then the letter.
fn alt(c: char) -> Vec<u8> {
    vec![0x1b, c as u8]
}

#[test]
fn a_command_typed_into_the_shell_view_runs_and_its_output_is_on_screen() {
    let dir = Dir::new("shell");
    dir.write("notes.md", "# notes\n");
    let session = forge(&dir);

    // The git view is what forge opens on, so its border is the proof that
    // forge started, sized itself and drew — before any key is sent.
    wait_for(&session, "git");

    send(&session, &alt('s'));
    // cmd announces itself before it prompts, which is the cheapest thing to
    // wait for that means "the child is up" rather than "the pane exists".
    wait_for(&session, "Microsoft Windows");

    // Arithmetic rather than an echo: the answer must not appear in the command
    // that produced it, or the assertion passes on the text of the keystrokes
    // and proves nothing ran.
    send(&session, b"set /a 123*456\r");
    let text = wait_for(&session, "56088");

    // ...and it ran in the directory forge was pointed at, which is the whole
    // point of the pane being here rather than in another window.
    assert!(
        text.contains("forge-e2e-shell"),
        "the shell's prompt should name forge's root; got:\n{text}"
    );

    // Claude is still live, so the first Alt+Q asks and the second answers.
    send(&session, &alt('q'));
    assert!(
        screen(&session).contains("again to quit"),
        "quitting a live session asks first"
    );
    send(&session, &alt('q'));
    drop(session);
}

/// The one test here that is about an attack rather than a feature.
///
/// Windows resolves a bare program name in `CreateProcessW` against the calling
/// process's current directory before it consults `PATH`, and portable-pty
/// hands the bare name straight through when its own `PATH` walk finds nothing.
/// Forge runs with the repository as its directory, which is the one directory
/// in the whole question that somebody else gets to write to — so `Alt+S`
/// falling back through a list of shells was one `git clone` away from
/// executing a file out of the repo, in a pty, with the user's full token.
///
/// Planting a *copy of a real shell* rather than a marker program is what makes
/// this checkable without a compiler: if the planted file ran, it would start
/// and print a banner, and the message this test waits for would never appear.
/// The wait failing **is** the vulnerability reproducing.
#[test]
fn a_shell_planted_in_the_repository_is_not_what_alt_s_runs() {
    let dir = Dir::new("planted");
    let system32 = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe");
    // A name nothing on this machine has, so the only way it can be found is
    // the way that must not work.
    let planted = "zz-planted-shell.exe";
    std::fs::copy(&system32, dir.0.join(planted)).expect("plant a shell in the repository");

    let session = PtySession::spawn(
        PtyConfig::new(env!("CARGO_BIN_EXE_forge"))
            .arg("cmd.exe")
            .cwd(&dir.0)
            .env("FORGE_SHELL", planted)
            .size(40, 120),
    )
    .expect("spawn forge in a pty");

    wait_for(&session, "git");
    send(&session, &alt('s'));

    // Refused, and said why. Had the bare name reached `CreateProcessW`, the
    // planted copy would be running and this string would never arrive.
    let text = wait_for(&session, "not found on PATH");
    assert!(
        text.contains(planted),
        "the pane should name what it would not run; got:\n{text}"
    );

    send(&session, &alt('q'));
    drop(session);
}

#[test]
fn the_second_alt_e_opens_a_file_list_that_can_be_walked_to_a_file() {
    let dir = Dir::new("files");
    dir.write("notes.md", "# notes\n\nthe document forge opens on.\n");
    dir.mkdir("subdir");
    dir.write("subdir/target-file.md", "# found me\n");
    let session = forge(&dir);

    wait_for(&session, "git");

    // First press shows the viewer, which has already opened the newest
    // markdown under the root without being asked — the behaviour forge exists
    // for. Which file that is depends on mtimes this test does not control, so
    // what is asserted is the rendering: the heading arrives styled, not as its
    // source, which is true of either document here.
    send(&session, &alt('e'));
    wait_for(&session, "rendered");

    // Second press is the file list. A directory is the thing a document view
    // could never show, so it is what distinguishes the two on screen.
    send(&session, &alt('e'));
    wait_for(&session, "subdir");

    // Then reach a file the pane was never pointed at, from the key that is the
    // real answer to "view any file": focus, `/`, type, Enter. Nothing has
    // opened `target-file.md`, no watcher has mentioned it, and it is not in
    // the directory the list started in.
    send(&session, &[0x1b, b'[', b'1', b';', b'3', b'C']); // Alt+Right
    send(&session, b"/");
    send(&session, b"target");
    send(&session, b"\r");
    wait_for(&session, "# found me");

    send(&session, &alt('q'));
    send(&session, &alt('q'));
    drop(session);
}
