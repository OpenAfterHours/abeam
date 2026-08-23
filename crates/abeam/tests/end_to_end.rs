//! The abeam binary, hosted in a pty, driven by keystrokes.
//!
//! Every other test in this repository builds a pane or an `App` in-process and
//! asks what it did. That is most of the value and none of the last mile: it
//! cannot tell you that the real binary starts, that raw mode and the alternate
//! screen survive being someone else's child, that a keystroke written as bytes
//! is decoded into the event a binding matches, or that the thing you asked for
//! is *legible* in a 46-column pane. Those are exactly the failures a user meets
//! first and a unit test never sees.
//!
//! So this suite does to abeam what abeam does to an agent: spawns it in a pty
//! through `abeam-pty` — a ConPTY on Windows, a Unix pty on Linux — types at it,
//! and reads the screen that comes back. The library being used to test the
//! binary is not a shortcut — it is the same code path the product runs on,
//! which is why a bug in it fails here too rather than hiding.
//!
//! **It runs on both platforms, and that is most of what it is for.** These are
//! the only tests that drive the compiled binary at all, so the `#![cfg(windows)]`
//! this file used to carry meant the Linux leg of CI reported zero tests from
//! here and went green — a port claiming abeam runs on Linux with nothing
//! whatsoever checking that the binary starts there. What genuinely differs
//! between the two platforms is which programs exist and what they print, and
//! all of that is in one block below rather than scattered through the tests.
//! Every keystroke is still written out in full — in a test body where it is the
//! same on both, and in that block where it is not — because a suite that
//! assembles its keystrokes cannot be read against the binary it drives.
//!
//! The children are chosen for determinism, not realism: `cmd.exe` on Windows
//! and `/bin/sh` on Unix, on both sides of the window. `pwsh` prints a banner
//! whose text varies by version and takes a second to do it, a login shell is
//! whatever the runner happens to have, and the assertions here are about abeam
//! rather than about a shell.
//!
//! **These tests are slow by the standards of this repository** — a few seconds
//! each, against milliseconds everywhere else — because they wait on real
//! process startup. They earn it by being the only tests that would notice
//! abeam failing to start at all.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use abeam_pty::{PtyConfig, PtySession};

/// Long enough for a cold shell on a loaded machine, short enough that a hang is
/// a test failure rather than a coffee break. Every wait here is polled, so a
/// passing run finishes as soon as the screen says what it should and never
/// spends this — which is what makes a number sized for the slowest GitHub
/// runner free on a desktop.
const DEADLINE: Duration = Duration::from_secs(20);

// --- the two platforms, in one place --------------------------------------
//
// Everything that differs between Windows and Unix is here, so that the three
// tests below read as one suite rather than as two suites interleaved by
// `#[cfg]`. Most of these answer "which program" or "what does it print"; the
// one that is a keystroke is written out as the literal bytes that go on the
// wire, and so is every keystroke left in the test bodies, because that is the
// part a reader has to be able to check against the binary by eye.

/// The program abeam is asked to host in the left pane.
///
/// It has to exist on a bare runner, start quickly, print nothing whose text
/// varies by version, and — because every test here ends by proving that
/// `Alt+Q` asks before it takes a live child down — stay alive until it is
/// killed. Named absolutely on Unix, exactly as `panes::shell`'s own suite
/// names it, so that a failure here is a fact about abeam and not about the
/// runner's `PATH`.
///
/// It is the program's *name*, without the `+` that asks abeam to host it, and
/// the two call sites below add the sigil themselves. That is the shape the
/// assertions want: the command line now belongs to the agent, so `+` is the
/// only way left to name a program to host — while the border still reads the
/// name that was typed with the sigil stripped, which is what every assertion
/// about the left title is checking. One constant that means "the program"
/// keeps those two facts from being written as one string that is right for
/// neither.
#[cfg(windows)]
const HOSTED: &str = "cmd.exe";
#[cfg(unix)]
const HOSTED: &str = "/bin/sh";

/// What `ABEAM_SHELL` names, so that the command view starts a known child
/// rather than whatever the candidate search would pick — `pwsh` on Windows, and
/// on Unix whatever `$SHELL` the person or the runner happens to have set. It is
/// a user-facing setting rather than a test hook, which is what makes it the
/// right seam to reach for.
#[cfg(windows)]
const SHELL: &str = "cmd.exe";
#[cfg(unix)]
const SHELL: &str = "/bin/sh";

/// What tells this suite the command view has a child in it — and it is not the
/// same *kind* of fact on the two platforms, which is worth being plain about
/// rather than papering over.
///
/// `cmd` announces itself before it prompts, so waiting for that means "the
/// child is up" rather than "the pane exists". Unix has no such line: `/bin/sh`
/// on `ubuntu-latest` is `dash`, which prints `$ ` and nothing else — one
/// character, and the same character the shell abeam hosts in the *other* pane
/// is printing at the same time. So the Unix leg waits for abeam's own border
/// naming the child it started, which `panes::shell` only reaches after the
/// spawn has succeeded and which is drawn on the frame after it.
///
/// That is one step weaker, and the step is bought straight back: the
/// arithmetic below cannot reach the screen unless a shell really is there to
/// run it. Nothing is typed at the pane before this string appears either way,
/// which is the only thing the wait is actually protecting — a key sent while
/// the pane is still `Cold` is dropped rather than queued.
///
/// The middle dot is not a leap of faith: a run of this suite with an added
/// `assert!(screen.contains("shell · cmd"))` passed on Windows, so the border's
/// non-ASCII survives being written by ratatui, carried through a pty and
/// parsed by `vt100`. A Unix pty does strictly less to those bytes than ConPTY.
#[cfg(windows)]
const SHELL_IS_UP: &str = "Microsoft Windows";
#[cfg(unix)]
const SHELL_IS_UP: &str = "shell · sh";

/// The one command this suite types at a shell, and what comes back proves two
/// separate things.
///
/// Arithmetic rather than an echo of a literal, on both: the answer must not
/// appear in the command that produced it, or the assertion passes on the text
/// of the keystrokes and proves nothing ran. `set /a` and `$(( ))` are builtins
/// of their respective shells, so neither line depends on a program being
/// installed on the runner.
///
/// `pwd` is on the Unix line only, and it is there because only `cmd` puts the
/// working directory in the prompt for free. Asserting that abeam's root
/// appeared in `dash`'s prompt would be asserting something `bash` does and
/// `ubuntu-latest`'s `/bin/sh` does not — a green test on a developer's Fedora
/// and a red one on CI. `pwd` asks the child the same question outright, and it
/// runs *first* so that by the time the number this test waits for is on screen
/// the path already is.
#[cfg(windows)]
const ARITHMETIC: &[u8] = b"set /a 123*456\r";
#[cfg(unix)]
const ARITHMETIC: &[u8] = b"pwd; echo $((123*456))\r";

/// The name the planted shell is given: junk, so that nothing on the machine
/// running this has it and the only way it can be found is the way that must
/// not work.
#[cfg(windows)]
const PLANTED: &str = "zz-planted-shell.exe";
#[cfg(unix)]
const PLANTED: &str = "zz-planted-shell";

/// Put a copy of a real shell in the repository, under [`PLANTED`].
///
/// A copy of a *shell* rather than a marker program is what makes the test it
/// serves checkable without a compiler; see that test for the whole argument.
#[cfg(windows)]
fn plant(dir: &Dir) {
    let real = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe");
    std::fs::copy(&real, dir.0.join(PLANTED)).expect("plant a shell in the repository");
}

/// Plant the executable shape the official npm package gives Codex on Windows.
///
/// The batch file stays alive after reporting its arguments so the assertion
/// observes a hosted interactive process, not a launch that happened to print
/// before failing. Going through `.cmd` also exercises the launcher route the
/// real `@openai/codex` package uses on this platform.
#[cfg(windows)]
fn plant_codex(dir: &Dir) {
    dir.write(
        "codex.cmd",
        "@echo off\r\necho codex-pty-ready\r\necho arg1=[%~1]\r\necho arg2=[%~2]\r\n:wait\r\nset /p line=\r\necho codex-input=[%line%]\r\ngoto wait\r\n",
    );
}

/// See the Windows twin above. The mode is set outright rather than left to
/// `fs::copy`, which does carry it across on this platform: the premise of the
/// test is that *nothing but abeam's resolver* stands between this file and a
/// pty, and a premise resting on a side effect of a copy is a premise that could
/// hold for the wrong reason.
#[cfg(unix)]
fn plant(dir: &Dir) {
    use std::os::unix::fs::PermissionsExt;

    let planted = dir.0.join(PLANTED);
    std::fs::copy("/bin/sh", &planted).expect("plant a shell in the repository");
    let mut mode = std::fs::metadata(&planted)
        .expect("stat the planted shell")
        .permissions();
    mode.set_mode(0o755);
    std::fs::set_permissions(&planted, mode).expect("make the planted shell executable");
}

/// Plant the extensionless executable shape npm gives Codex on Unix.
#[cfg(unix)]
fn plant_codex(dir: &Dir) {
    use std::os::unix::fs::PermissionsExt;

    let planted = dir.0.join("codex");
    dir.write(
        "codex",
        "#!/bin/sh\nprintf 'codex-pty-ready\\n'\nprintf 'arg1=[%s]\\n' \"$1\"\nprintf 'arg2=[%s]\\n' \"$2\"\nwhile IFS= read -r line; do printf 'codex-input=[%s]\\n' \"$line\"; done\n",
    );
    let mut mode = std::fs::metadata(&planted)
        .expect("stat the planted Codex shim")
        .permissions();
    mode.set_mode(0o755);
    std::fs::set_permissions(&planted, mode).expect("make the Codex shim executable");
}

/// The `PATH` abeam is given while a shell is planted in the repository, or
/// `None` to leave the inherited one alone.
///
/// Nothing to do on Windows: there the hazard is `CreateProcessW` resolving a
/// bare name against the calling process's own directory, which no `PATH`
/// arranges and no `PATH` prevents.
#[cfg(windows)]
fn a_path_that_reaches_the_repository() -> Option<String> {
    None
}

/// See the Windows twin above. `PATH=:$PATH` — one leading empty entry, which
/// is what a shell profile appending to an unset `PATH` produces on its own and
/// what one typo produces deliberately, and which `crate::launch`'s walk lists
/// among the entries it refuses.
///
/// Without it this test could not witness the vulnerability even in principle:
/// portable-pty's walk computes `cwd.join(entry).join(name)`, and every entry on
/// a runner's `PATH` is absolute, so `cwd.join(entry)` is just the entry and the
/// planted file is out of reach whatever abeam does. With it, the repository is
/// genuinely on the `PATH` that portable-pty would search, and the only thing
/// left between the plant and a pty is the resolver — which is the claim.
#[cfg(unix)]
fn a_path_that_reaches_the_repository() -> Option<String> {
    Some(format!(":{}", std::env::var("PATH").unwrap_or_default()))
}

/// A scratch directory. `crate::testutil::TempDir` is `#[cfg(test)]` inside the
/// binary and an integration test is a different crate, so this is the same
/// idea in the eight lines it takes.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "abeam-e2e-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create a scratch directory");
        Dir(path)
    }

    /// `fs::write` translates nothing on either platform, so what the viewer
    /// reads back is exactly the bytes given here — which is why every scratch
    /// file in this file ends its lines with `\n` and not `\r\n`. A file written
    /// with the platform's line ending would carry a stray carriage return into
    /// a pane that renders what it is given.
    fn write(&self, name: &str, body: &str) {
        std::fs::write(self.0.join(name), body).expect("write a scratch file");
    }

    fn mkdir(&self, name: &str) {
        std::fs::create_dir_all(self.0.join(name)).expect("create a scratch subdirectory");
    }

    /// Run git here, and say whether it worked.
    ///
    /// An identity is handed in rather than assumed: a machine whose global
    /// config has no `user.email` on it cannot commit, and a repository with no
    /// commit has no HEAD to add a worktree at. `GIT_OPTIONAL_LOCKS=0` is the
    /// same setting `panes::git` runs with, and for the same reason — abeam is
    /// polling this repository while this runs.
    fn git(&self, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_AUTHOR_NAME", "abeam")
            .env("GIT_AUTHOR_EMAIL", "abeam@example.invalid")
            .env("GIT_COMMITTER_NAME", "abeam")
            .env("GIT_COMMITTER_EMAIL", "abeam@example.invalid")
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|out| out.status.success())
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        // Best effort: on Windows a directory can stay locked briefly after the
        // process that had it as a working directory dies, and a test that
        // fails because of *that* is a test nobody trusts. Unix has no such
        // window, and one shape that is right on both is worth more than a
        // `cfg` here.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Start abeam itself, hosting [`HOSTED`], in a pane-sized pty.
///
/// 120x40 is the smallest window that still splits — below `MIN_SPLIT_COLS` the
/// right pane collapses and every assertion here would be about a pane that was
/// never drawn.
fn abeam(dir: &Dir) -> PtySession {
    PtySession::spawn(
        PtyConfig::new(env!("CARGO_BIN_EXE_abeam"))
            .arg(format!("+{HOSTED}"))
            .cwd(&dir.0)
            // The command view would otherwise search: `pwsh` on Windows, whose
            // banner and startup time vary by machine, and `$SHELL` on Unix,
            // which is whatever the person or the runner happens to have. This
            // is the seam that makes it testable, and it is a user-facing
            // setting rather than a test hook.
            .env("ABEAM_SHELL", SHELL)
            .size(40, 120),
    )
    .expect("spawn abeam in a pty")
}

/// Start the real abeam binary through its first-class Codex selection.
fn abeam_with_codex(dir: &Dir) -> PtySession {
    plant_codex(dir);
    let path = std::env::join_paths(
        std::iter::once(dir.0.clone()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .expect("construct an absolute PATH for the Codex shim")
    .to_string_lossy()
    .into_owned();

    PtySession::spawn(
        PtyConfig::new(env!("CARGO_BIN_EXE_abeam"))
            .arg("+codex")
            .arg("--search")
            .arg("a prompt with spaces")
            .cwd(&dir.0)
            .env("PATH", path)
            .env("ABEAM_SHELL", SHELL)
            .size(40, 120),
    )
    .expect("spawn abeam with Codex in a pty")
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

/// Type at abeam, as bytes on the pty exactly as a terminal would send them.
///
/// The pause is not superstition: abeam drains every pending event before
/// drawing a frame, so keys sent in one burst can be handled before the frame
/// that would spawn the pane they were aimed at. A person cannot type faster
/// than a frame; a test can.
fn send(session: &PtySession, bytes: &[u8]) {
    session.write(bytes).expect("write to abeam's pty");
    std::thread::sleep(Duration::from_millis(250));
}

/// `Alt`+letter, as a terminal encodes it: escape, then the letter.
///
/// The same two bytes on both platforms, and this is bytes on a pty rather than
/// a synthesised crossterm event, so there is nothing platform-specific for it
/// to get wrong. On Unix crossterm's parser turns `ESC` followed by a character
/// in the same read into that character with `ALT` set; on Windows ConPTY
/// translates the pair into the console input record that says the same thing.
/// Both bytes go out in one `write`, which is what keeps them in one read — an
/// `ESC` arriving alone is `Esc`, and this would then read as abeam ignoring a
/// binding it in fact never received.
fn alt(c: char) -> Vec<u8> {
    vec![0x1b, c as u8]
}

#[test]
fn the_codex_executable_shape_and_arguments_survive_the_real_binary_and_pty() {
    let dir = Dir::new("codex");
    let session = abeam_with_codex(&dir);

    let text = wait_for(&session, "codex-pty-ready");
    assert!(text.contains("arg1=[--search]"), "first argument changed: {text}");
    assert!(
        text.contains("arg2=[a prompt with spaces]"),
        "a spaced Codex argument did not survive the launch: {text}"
    );

    // A ready line alone could have come from a child that exited immediately.
    // The response to input sent through abeam proves the interactive child is
    // still hosted and that the whole input path reaches it.
    send(&session, b"still-hosted\r");
    wait_for(&session, "codex-input=[still-hosted]");
}

#[test]
#[ignore = "requires ABEAM_TEST_CODEX pointing at an official Codex CLI binary"]
fn an_official_codex_reaches_its_auth_ui_resizes_and_quits_cleanly() {
    let executable = PathBuf::from(
        std::env::var_os("ABEAM_TEST_CODEX")
            .expect("set ABEAM_TEST_CODEX to an official Codex CLI executable"),
    );
    let bin = executable
        .parent()
        .expect("the Codex executable has a parent directory");
    let dir = Dir::new("codex-live");
    let codex_home = dir.0.join("codex-home");
    std::fs::create_dir_all(&codex_home).expect("create an isolated Codex home");
    let path = std::env::join_paths(
        std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .expect("construct PATH for the official Codex binary")
    .to_string_lossy()
    .into_owned();

    let mut session = PtySession::spawn(
        PtyConfig::new(env!("CARGO_BIN_EXE_abeam"))
            .arg("+codex")
            .arg("--no-alt-screen")
            .cwd(&dir.0)
            .env("PATH", path)
            .env("CODEX_HOME", codex_home.to_string_lossy())
            .env("ABEAM_SHELL", SHELL)
            .size(40, 120),
    )
    .expect("spawn abeam with the official Codex binary");

    wait_for(&session, "Welcome to Codex");
    let before = screen(&session);
    session.resize(32, 100).expect("resize abeam's outer pty");
    assert_eq!(session.size(), (32, 100));
    wait_for(&session, "Sign in with ChatGPT");

    // Navigation is safe before authentication and proves Codex still parses
    // its ordinary legacy-terminal input after the resize.
    send(&session, b"\x1b[B");
    let after = screen(&session);
    assert_ne!(before, after, "Codex did not react to a Down-arrow key");

    send(&session, &alt('q'));
    wait_for(&session, "Alt+Q again to quit");
    send(&session, &alt('q'));
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if session.try_wait().expect("poll abeam after quit").is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("abeam did not exit after the confirmed quit");
}

#[test]
fn a_command_typed_into_the_shell_view_runs_and_its_output_is_on_screen() {
    // Delete this and nothing in the repository notices that `Alt+S` no longer
    // reaches a shell, that the shell no longer starts, or that what it prints
    // never arrives on screen — every other test of that pane builds it
    // in-process and never presses a key at the binary.
    let dir = Dir::new("shell");
    dir.write("notes.md", "# notes\n");
    let session = abeam(&dir);

    // The git view is what abeam opens on, so its border is the proof that
    // abeam started, sized itself and drew — before any key is sent.
    wait_for(&session, "git");

    send(&session, &alt('s'));
    wait_for(&session, SHELL_IS_UP);

    send(&session, ARITHMETIC);
    let text = wait_for(&session, "56088");

    // ...and it ran in the directory abeam was pointed at, which is the whole
    // point of the pane being here rather than in another window.
    //
    // One assertion, two different pieces of evidence: on Windows the root is in
    // `cmd`'s prompt, and on Unix it is what `pwd` answered — see [`ARITHMETIC`]
    // for why the second cannot be read off `dash`'s prompt. The directory's
    // *name* rather than its whole path, because `pwd` resolves symlinks in the
    // path to a temporary directory and the leaf is the part that survives that.
    //
    // The two are not equally strong, and the weaker one is Windows'. `cmd` is
    // also what abeam hosts in the *left* pane, with the same root, so its
    // prompt carries this string too and the assertion would pass on that alone.
    // The Unix half cannot: `dash` prints no directory anywhere, so the only
    // thing on that screen naming the root is the `pwd` typed into this pane.
    // Left as it is rather than tightened, because tightening it means asking
    // where on screen a string is, and this whole suite is built on not doing
    // that (see [`screen`]).
    assert!(
        text.contains("abeam-e2e-shell"),
        "the shell ran somewhere other than abeam's root; got:\n{text}"
    );

    // Both children are live — the one in the left pane and the shell in the
    // right — so the first Alt+Q asks and the second answers.
    send(&session, &alt('q'));
    assert!(
        screen(&session).contains("again to quit"),
        "quitting a live session asks first"
    );
    send(&session, &alt('q'));
    drop(session);
}

/// The one test here that is about an attack rather than a feature, and the one
/// whose argument is genuinely different on the two platforms rather than the
/// same argument spelled twice. Delete it and a planted shell in a cloned
/// repository becomes a thing abeam will start, in a pty, with the user's full
/// authority, and nothing anywhere says otherwise.
///
/// **Windows.** `CreateProcessW` resolves a bare program name against the
/// *calling process's* current directory before it consults `PATH`, and
/// portable-pty hands the bare name straight through when its own `PATH` walk
/// finds nothing. The directory abeam runs with is the repository, which is the
/// one directory in the whole question that somebody else gets to write to — so
/// `Alt+S` falling back through a list of shells was one `git clone` away from
/// executing a file out of the repo. `main` standing in `%SystemRoot%` is a
/// second line of defence behind the resolver.
///
/// **Unix.** The first reading of this was that a twin proves nothing here,
/// because `execvp` has no "current directory first" rule. That reading is
/// wrong, and it is wrong because abeam never reaches `execvp`'s own `PATH`
/// walk. Every spawn goes through `portable_pty::CommandBuilder::as_command`,
/// which takes `dir` from `PtyConfig.cwd` (0.9.0, `src/cmdbuilder.rs`, 502-507)
/// and hands it to `search_path` (519) — and the walk there computes
/// `cwd.join(entry).join(name)` for every `PATH` entry (451), with an explicit
/// `cwd.join(exe_path)` for anything spelled `./x` (426-435). Every pty abeam
/// opens is given the repository on screen as its `cwd`. So an empty or relative
/// `PATH` entry names the repository however far this process has walked from
/// it, and `main`'s chdir to `/` does *not* back the resolver up the way
/// `%SystemRoot%` does on Windows — `main` says so at length, on the line that
/// does it. On this platform `crate::launch::resolve` refusing the bare name is
/// the whole of the defence, with nothing behind it, which makes this twin the
/// more valuable of the two rather than the ceremonial one.
///
/// That is also why the Unix leg hands abeam a `PATH` with an empty entry on it;
/// see [`a_path_that_reaches_the_repository`]. Without one the repository is not
/// on any `PATH` portable-pty would search, and this test could not witness the
/// vulnerability even with every check in `crate::launch` deleted.
///
/// Planting a *copy of a real shell* rather than a marker program is what makes
/// all of that checkable without a compiler: if the planted file ran it would
/// start and prompt, and the message this test waits for would never appear. The
/// wait failing **is** the vulnerability reproducing.
///
/// That last sentence was measured rather than assumed, because a security test
/// nobody has seen fail is a security test nobody knows works. On Windows, with
/// `crate::launch::find` patched to hand the bare name back and
/// `main::somewhere_unwritable` patched to answer `None`, this test failed after
/// its full twenty seconds with the planted copy's own banner and a prompt
/// sitting in the scratch repository, under a border reading
/// `shell · zz-planted-shell`. Both patches were reverted; neither is anywhere
/// in this branch.
#[test]
fn a_shell_planted_in_the_repository_is_not_what_alt_s_runs() {
    let dir = Dir::new("planted");
    plant(&dir);

    let mut cfg = PtyConfig::new(env!("CARGO_BIN_EXE_abeam"))
        .arg(format!("+{HOSTED}"))
        .cwd(&dir.0)
        // The bare name, which is the only spelling that asks the question this
        // test is about: an absolute one is abeam being told exactly what to
        // start, and a `./` one is refused by a different sentence.
        .env("ABEAM_SHELL", PLANTED)
        .size(40, 120);
    if let Some(path) = a_path_that_reaches_the_repository() {
        cfg = cfg.env("PATH", path);
    }
    let session = PtySession::spawn(cfg).expect("spawn abeam in a pty");

    wait_for(&session, "git");
    send(&session, &alt('s'));

    // Refused, and said why. Had the bare name reached the spawn, the planted
    // copy would be running and this string would never arrive.
    let text = wait_for(&session, "not found on PATH");
    assert!(
        text.contains(PLANTED),
        "the pane should name what it would not run; got:\n{text}"
    );

    // One Alt+Q rather than two: no shell ever started, so the only live child
    // is the one in the left pane, and the press that asks is the last thing
    // this test needs. Dropping the session takes the rest down.
    send(&session, &alt('q'));
    drop(session);
}

#[test]
fn the_second_alt_e_opens_a_file_list_that_can_be_walked_to_a_file() {
    // Delete this and the file list can stop opening, stop finding, or stop
    // showing what it found, and the only thing that would notice is a person.
    let dir = Dir::new("files");
    dir.write("notes.md", "# notes\n\nthe document abeam opens on.\n");
    dir.mkdir("subdir");
    dir.write("subdir/target-file.md", "# found me\n");
    let session = abeam(&dir);

    wait_for(&session, "git");

    // First press shows the viewer, which has already opened the newest
    // markdown under the root without being asked — the behaviour abeam exists
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
    //
    // Every byte below is the same on both platforms: `\x1b[15~` is F5 as any
    // terminal sends it, and the rest is plain typing.
    send(&session, b"\x1b[15~"); // F5, the focus-right key
    send(&session, b"/");
    send(&session, b"target");
    send(&session, b"\r");
    wait_for(&session, "# found me");

    send(&session, &alt('q'));
    send(&session, &alt('q'));
    drop(session);
}

#[test]
fn the_right_pane_can_be_pointed_at_a_worktree_and_both_of_its_views_follow() {
    // The one test that drives a workspace switch end to end, and every step of
    // it belongs to somebody else: git decides where the worktree lives and how
    // it spells the path, `notify` and the filesystem decide what comes back,
    // ConPTY or a Unix pty decides what the keystrokes look like on the wire.
    // A unit test can prove `set_root` clears what it should; only this can
    // prove that `w` reaches the pane at all, that the list is legible in 46
    // columns, and that the two right-hand views really do land in the other
    // worktree.
    //
    // The left pane is deliberately not in any of it: a live child's working
    // directory belongs to the child, so the agent stays where it started. That
    // asymmetry is what the border's workspace label exists to say out loud,
    // and it is the last thing this test checks.
    let dir = Dir::new("worktrees");

    // Committed, so there is a HEAD to add a worktree at. `notes.md` is left
    // untracked on purpose: the reader opens the *newest* markdown under its
    // root, and one document per tree written in a known order is what makes
    // "which one did it open" a question with an answer.
    dir.write("README.md", "# repo\n");
    if !dir.git(&["init", "-q", "-b", "main", "."])
        || !dir.git(&["add", "-A"])
        || !dir.git(&["commit", "-qm", "first"])
        || !dir.git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "review",
            ".claude/worktrees/review",
        ])
    {
        panic!("this test needs git on PATH; without it the switch is untested");
    }

    // One document and one untracked file per worktree, each naming its own, so
    // that both right-hand views can be caught looking at the wrong tree rather
    // than merely at a plausible one.
    dir.write("notes.md", "# the agent's own root\n");
    dir.write("only-in-main.txt", "x\n");
    dir.write(
        ".claude/worktrees/review/review-note.md",
        "# only in the review worktree\n",
    );
    dir.write(".claude/worktrees/review/only-in-review.txt", "x\n");

    let session = abeam(&dir);
    wait_for(&session, "git");
    // The agent's own root, before anything is switched: its untracked file is
    // in the git pane, and the border says nothing about a workspace because
    // there is nothing to say — the pane is 46 columns and a label that is true
    // by default is one that costs a branch name.
    let start = wait_for(&session, "only-in-main.txt");
    assert!(
        !start.contains("review · git"),
        "the border named the agent's own workspace: {start}"
    );

    // `w` is pane-local, so the right pane has to have the keyboard: it is only
    // ever delivered while the git view has focus, which is what exempts it
    // from the invariant in `crate::keys`.
    send(&session, b"\x1b[15~"); // F5, the focus-right key
    send(&session, b"w");
    let list = wait_for(&session, "worktrees");
    assert!(
        list.contains("review"),
        "the worktree git just made is not on the list: {list}"
    );

    // git prints the main worktree first, so one Tab is the neighbour.
    send(&session, b"\t");
    send(&session, b"\r");

    // The border now names the workspace, and the git pane is reporting the
    // other tree: `review` is both the label before the pane title and the
    // branch inside it.
    let switched = wait_for(&session, "review · git");
    let switched = if switched.contains("only-in-review.txt") {
        switched
    } else {
        wait_for(&session, "only-in-review.txt")
    };
    assert!(
        !switched.contains("only-in-main.txt"),
        "the git pane is still reporting the workspace it was pointed away \
         from: {switched}"
    );

    // ...and the reader followed, without being told which file: `set_root`
    // leaves it empty and the startup walk of the *new* root opens the newest
    // document it finds there.
    send(&session, &alt('e'));
    let read = wait_for(&session, "only in the review worktree");
    assert!(
        !read.contains("the agent's own root"),
        "the reader is showing a document of the workspace that was left: {read}"
    );
    assert!(
        read.contains("review · "),
        "the border stopped naming the workspace the reader is in: {read}"
    );

    send(&session, &alt('q'));
    send(&session, &alt('q'));
    drop(session);
}

#[test]
fn rows_of_the_shell_view_are_selected_and_copied_and_the_child_never_sees_the_keys() {
    // Two things no in-process test can reach, and the second is the one that
    // would hurt.
    //
    // `F7` has to *arrive* as `F7`. Every other test of this feature builds a
    // `KeyEvent` and hands it to `App`, which assumes the answer to the only
    // question worth asking on Windows: whether `ESC [ 1 8 ~` written at a pty
    // comes back out of ConPTY as the function key it names. `F5` in the test
    // above says the shape works; nothing said it for this key.
    //
    // And the mode has to swallow what it claims to swallow, in front of a real
    // child. In-process, "the pane did not act on it" is asserted against a
    // pane. Here there is a live shell with a prompt on screen, and a key that
    // leaked would be visible in it — which is exactly the failure the mode
    // exists to prevent, since the keys somebody presses while aiming a caret
    // are letters, and letters at a prompt are the beginning of a command.
    let dir = Dir::new("select");
    let session = abeam(&dir);
    wait_for(&session, "git");

    send(&session, &alt('s'));
    wait_for(&session, SHELL_IS_UP);
    send(&session, ARITHMETIC);
    wait_for(&session, "56088");

    // The path that needs no keys at all, and the one most people will ever
    // use: press, drag, let go. SGR mouse reports, which is what a terminal
    // sends once abeam has asked for them — column 80 is inside the right pane
    // of a 120-column window, and the rows are where a shell that has just
    // printed something puts it.
    send(&session, b"\x1b[<0;80;2M");
    send(&session, b"\x1b[<32;80;5M");
    send(&session, b"\x1b[<0;80;5m");
    let dragged = wait_for(&session, "copied");
    assert!(
        dragged.contains("agent"),
        "the note never said what to do with what it took:\n{dragged}"
    );

    // Out again, so what follows is not reading the drag's own selection.
    send(&session, b"\x1b");

    // Into the mode by keyboard. The border is the proof it engaged, and it is
    // the same border that tells a user what the keys are.
    send(&session, b"\x1b[18~");
    wait_for(&session, "y copy");

    // Now the part that matters. `jjj` moves the caret three rows; at a prompt
    // it is three characters somebody would then have to notice and delete.
    send(&session, b"jjj");
    let quiet = screen(&session);
    assert!(
        !quiet.contains("jjj"),
        "keys aimed at the caret reached the shell behind it:\n{quiet}"
    );

    // Anchor, run to the bottom, copy. What can be asserted from out here is
    // that abeam says it copied — whether the terminal on the other end kept it
    // is not knowable from either side of this pty, which is the same limit the
    // feature has in real use.
    send(&session, b"vGy");
    let copied = wait_for(&session, "copied");

    // The arithmetic is still on screen underneath the highlight, which is what
    // says the pane was selected *from* rather than replaced by a mode.
    assert!(
        copied.contains("56088"),
        "the shell's output went away while it was being copied:\n{copied}"
    );

    // `Ctrl+C` is the other key a hand reaches for with something highlighted,
    // and it is the one place in abeam where a `Ctrl`+letter is not the child's
    // — reachable only here, where the child is being offered nothing anyway.
    send(&session, b"\x03");
    wait_for(&session, "copied");

    // And leaving gives the keys back: the same `jjj` now reaches the prompt it
    // was kept away from, which is the other half of the claim and the half a
    // mode that never exited would pass without.
    //
    // No `Alt+S` between the two, and that is the rule rather than a shortcut:
    // `Esc` out of a selection lands where a second `F7` would, and this
    // selection took no focus — `Alt+S` had focused the shell long before `F7`
    // was pressed. Putting the highlight away therefore hands nothing back, so
    // the keys are the shell's already. `Alt+S` here would have walked *out* to
    // the agent and typed three letters into a prompt.
    send(&session, b"\x1b");
    send(&session, b"jjj");
    let typed = wait_for(&session, "jjj");
    assert!(typed.contains("jjj"));

    send(&session, &alt('q'));
    send(&session, &alt('q'));
    drop(session);
}
