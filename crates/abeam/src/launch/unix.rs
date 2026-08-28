//! What abeam hands to `execve`, and why that turns out to be the whole of it.
//!
//! *Where* to look for the file is the other half of the question, it is the
//! same hazard on every platform, and it lives in the parent module. This half
//! is Unix's alone — and it is three functions long, which is the first thing
//! to explain, because a short module beside a four-hundred-line one reads like
//! an unfinished port.
//!
//! ## Why there is nothing here
//!
//! Read `windows.rs` next to this. All of it — the `%ABEAM_LAUNCH%` variable,
//! the quoting taken from the fix for CVE-2024-24576, the measured 8124-character
//! cliff — exists because `CreateProcessW` starts a PE image and nothing else.
//! An npm install drops `claude`, `claude.cmd` and `claude.ps1` into
//! `%APPDATA%\npm` and Windows can start none of them, so the `.cmd` has to be
//! started by naming `cmd.exe` in front of it; and once `cmd.exe` is on the
//! wire, everything abeam was handed gets re-parsed by something that reads
//! `&`, `|`, `<`, `>`, `^`, `(`, `)`, `%` and `!` as syntax.
//!
//! The same `npm i -g` on Linux writes one file, `~/.npm-global/bin/claude`,
//! with `#!/usr/bin/env node` on its first line and the execute bit set. The
//! *kernel* reads that line: `execve` finds the interpreter, rewrites the
//! argument vector and starts it, before any of abeam's code would have had a
//! chance to. So there is nothing to route. And because there is nothing to
//! route, there is no second parser between abeam and the child — which means
//! nothing to quote against, no character that has to be refused because it
//! cannot be escaped, and no command-line length belonging to a shell that is
//! not running. `execve` takes an argument *vector*, not a line, and the limit
//! it does have is `MAX_ARG_STRLEN` (128 KiB per argument on Linux) against
//! `cmd.exe`'s 8 KiB for the whole line.
//!
//! ## So abeam never puts an interpreter in front of anything here
//!
//! That is a decision and not an omission, and it is worth being blunt about
//! because the obvious "improvement" to a module this short is to add `sh -c`
//! for the scripts. Doing that would take a program the kernel was about to
//! start correctly and hand its arguments to a parser that reads them as
//! syntax — inventing on Unix, deliberately, precisely the problem `windows.rs`
//! spends four hundred lines solving, in exchange for nothing at all. A file
//! with a `#!` line already runs. A file without one that is marked executable
//! is retried against `/bin/sh` by glibc's `execvp` — the kernel refuses it
//! with `ENOEXEC` and libc does the rest, in `__execvpe`'s
//! `maybe_script_execute`, which runs on the branch for a path with a slash in
//! it as well as on the `PATH` walk, and the branch with the slash is the only
//! one abeam reaches. **musl declines to implement that retry at all**, as a
//! long-standing upstream decision rather than an oversight, so on Alpine — a
//! prime `uvx` target, and part of why this port exists — the same file is an
//! `Exec format error` and what the user sees is portable-pty's spawn failure
//! rather than one of abeam's sentences. Which libc is answering also turns on
//! a detail of a dependency: portable-pty sets `pre_exec`, which disqualifies
//! Rust's `posix_spawn` fast path, and glibc's `posix_spawn` uses the variant
//! that does *not* retry. What is left is a file that cannot be executed, and
//! the answer to that is a sentence, not a shell.
//!
//! What the platform does genuinely ask is one question: whether this file may
//! be run at all. That is a mode bit rather than an extension, and it is the
//! only thing standing between the parent module's answer and a pty.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::Launch;

/// `name` inside `dir`, if `dir` holds a file by that name.
///
/// The whole of what a name means in a directory here. There is no `PATHEXT`,
/// so there is one candidate rather than six, and the preference `windows.rs`
/// spends its `probe_with` on — the file it can start, ahead of the file that
/// is merely present — has nothing to choose between inside one directory. It
/// still has to happen, so it happens across the `PATH` walk instead; see
/// [`super::walk`].
///
/// `is_file` rather than `exists`, and it follows the symlink on purpose,
/// because on this platform the program almost always *is* one. `npm i -g`
/// links `bin/claude` to `lib/node_modules/@anthropic-ai/claude-code/cli.js`,
/// and Claude's own native installer links `~/.local/bin/claude` at the
/// versioned copy it unpacked; refusing to follow would refuse both of the two
/// installs this port exists for. Following also answers the question that
/// actually matters — a symlink to a directory, a dangling one, and a
/// directory named `claude` are all things abeam must not try to start, and
/// `is_file` is false for every one of them. It is false for a directory it may
/// not read, too, which is the right answer arrived at for the wrong reason;
/// that costs a "not found on PATH" where "not allowed to look" would have been
/// better, and it is not worth a second syscall to tell them apart.
pub(super) fn probe(dir: &Path, name: &str) -> Option<PathBuf> {
    let file = dir.join(name);
    file.is_file().then_some(file)
}

/// Whether **this process** may execute this file.
///
/// `access(2)` rather than the mode bits, and the difference is the whole point
/// of the call. Reading the mode answers "may *anybody*", which is the wrong
/// question in the permissive direction: a `--x------` owned by somebody else,
/// or anything at all on a `noexec` mount, satisfies `mode & 0o111` and is then
/// started — so abeam hands it to the spawn, the kernel answers `EACCES`, and
/// what reaches the reader is portable-pty's raw error about a file abeam had
/// just decided was fine. Asking the kernel moves that refusal to the one place
/// that can explain it, which is [`into_launch`] below. It costs one syscall on
/// a path already doing several.
///
/// `access` asks about the **real** uid and gid rather than the effective ones,
/// which is a real distinction and not one that reaches abeam: the two differ
/// only for a setuid or setgid program, and abeam is neither. The call that
/// asks about the effective ids is `faccessat(…, AT_EACCESS)`, and choosing it
/// here would buy nothing while committing this module to an opinion about a
/// deployment that does not exist. If abeam ever ships setuid, this is the line
/// to revisit — but the answer then is not a different syscall, it is that
/// hosting an agent under someone else's authority is a different program.
///
/// A path that will not become a `CString` is not startable, and there is only
/// one way to fail: an interior NUL. No filesystem can hold such a name, so
/// this is unreachable through [`probe`], which only ever hands over paths that
/// `is_file()` answered `true` for — it is a `false` for completeness rather
/// than a case anybody will meet.
///
/// `access` follows symlinks, which matches [`probe`]'s `is_file` and is what
/// the kernel does at `execve`: the permission that decides is the target's,
/// and a symlink's own mode bits are never consulted by anything.
pub(super) fn startable(path: &Path) -> bool {
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // Safe: the pointer is to a NUL-terminated buffer that outlives the call,
    // and `access` neither retains it nor writes through it.
    unsafe { libc::access(c_path.as_ptr(), libc::X_OK) == 0 }
}

/// The file [`super::find`] settled on, turned into something to start — which
/// here means the file, unchanged.
pub(super) fn into_launch(found: PathBuf, args: &[String]) -> Result<Launch, String> {
    if !startable(&found) {
        // Its own sentence rather than the parent module's "was not found on
        // PATH", which would be a lie about a file the user can see with `ls`
        // and the single most misleading thing abeam could say here. Only
        // reachable for a file that exists and is regular, because that is all
        // `probe` and the hint filter ever hand over.
        //
        // It says "you may not" rather than "it is not executable", and offers
        // `chmod` as the likely fix rather than the fix, because [`startable`]
        // asks `access(X_OK)` and there are three ways to fail it. The common
        // one is a file carrying no execute bit at all — a `git clone` of
        // somebody's dotfiles, a `curl > ~/bin/claude`, a copy off a
        // FAT-formatted stick — and for that `chmod +x` is exactly right. The
        // other two are a file whose bits are set for somebody else, and a
        // filesystem mounted `noexec`; in both of those `chmod` either fails or
        // changes nothing, and a message that named it as *the* fix would send
        // the reader round a loop. Naming the mode bit alone would be worse
        // still — it would be a claim about the file that `ls -l` visibly
        // contradicts.
        return Err(format!(
            "abeam may not execute `{0}`. Usually that is a file with no \
             execute bit, and `chmod +x {0}` is the whole of it; where the bits \
             are already set the file belongs to somebody else, or its \
             filesystem is mounted `noexec`, and neither of those is yours to \
             chmod.",
            found.display()
        ));
    }

    // `program` and `target` are the same file and `env` is empty, and both of
    // those are the difference from Windows rather than a simplification of it.
    // There, `target` exists because a routed `.cmd` is started by a program
    // that is not it — `cmd.exe` — and a border has to be able to say which
    // file is doing the work; and `env` exists because the command line for
    // that shim travels in a variable. Here the file doing the work is the file
    // being started, for every shape of program there is: an ELF binary, a `#!`
    // script, a symlink to either. The arguments go through exactly as abeam
    // was given them, unquoted and uncounted, because there is nothing between
    // this and the child that will look at them.
    Ok(Launch {
        program: found.clone(),
        target: found,
        args: args.to_vec(),
        env: Vec::new(),
    })
}

/// Unix-only, like the Windows suite beside it and for the same reason: what
/// these are about is a `PATH` walk separated by `:` and a mode bit, and
/// neither of those exists on the other platform.
///
/// Every test here is the twin of one over there, asking the same question of a
/// different kernel — with the whole quoting half absent, because on Unix there
/// is no second parser to quote against. The spawn test at the bottom is what
/// stands in for it: `windows.rs` proves its quoting by putting a real shim in
/// a real pty and reading back what arrived, and this proves the *absence* of
/// any quoting the same way, with the same three hostile arguments.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::launch::{find, resolve, walk};
    use crate::testutil::TempDir;
    use abeam_pty::PtyConfig;
    use std::ffi::OsStr;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    /// A `PATH` out of these directories, joined the way this platform joins
    /// one. Written out rather than built with `std::env::join_paths`, which
    /// returns a `Result` for a case (a directory with a `:` in its name) that
    /// no test here has and that would only add an `unwrap` to read past.
    fn path_of(dirs: &[&Path]) -> String {
        dirs.iter()
            .map(|dir| dir.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// A relative `PATH` entry that really does hold a file, and the name of
    /// that file — so that a walk which had stopped refusing relative entries
    /// would answer `Some` rather than `None`.
    ///
    /// Discovered rather than written down, the same way
    /// `a_hint_that_is_not_absolute_is_trusted_no_further_than_a_path_entry_is`
    /// finds its own. `cargo test` runs with the crate directory as the current
    /// one, so `("src", "main.rs")` would do — and a test that pins the layout
    /// of the crate it lives in fails the day somebody moves a file, for a
    /// reason that has nothing to do with what it is about.
    fn a_relative_entry_holding_a_file() -> (PathBuf, String) {
        let here = std::env::current_dir().expect("a current directory");
        std::fs::read_dir(&here)
            .expect("read the current directory")
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .find_map(|entry| {
                let inside = std::fs::read_dir(entry.path())
                    .ok()?
                    .flatten()
                    .find(|inner| inner.path().is_file())?;
                Some((
                    PathBuf::from(entry.file_name()),
                    inside.file_name().to_string_lossy().into_owned(),
                ))
            })
            .expect("the current directory has a subdirectory with a file in it")
    }

    // --- what may be started ---------------------------------------------

    #[test]
    fn a_program_is_never_taken_from_the_directory_abeam_is_looking_at() {
        // The most important test in this file, as its twin is in `windows.rs`.
        // The hazard arrives by a different road here — `execve` has no
        // "current directory first" rule of its own, so nothing forces this on
        // abeam — and lands in exactly the same place, because a `PATH` is
        // free to name that directory itself. `.`, `..`, a relative entry, and
        // the empty string that a leading, trailing or doubled `:` leaves
        // behind all mean "where the process is standing", which under abeam is
        // the repository on screen: the one directory in this whole question
        // that somebody else gets to write to. `PATH=:$PATH` is one typo, and
        // an empty entry is what a shell profile that appends to an unset
        // `PATH` produces on its own.
        //
        // Driven through a `PATH` handed in rather than by standing in the
        // planted directory: the current directory belongs to the whole test
        // binary, and two hundred other tests are running beside this one.
        let dir = TempDir::new("launch-planted");
        dir.write_exec_unrun("abeam-planted", b"#!/bin/sh\n");
        let planted = "abeam-planted";

        for hostile in ["::", ":", ".", ".:", "./tools", "..", ""] {
            assert_eq!(
                walk(OsStr::new(hostile), planted),
                None,
                "PATH {hostile:?} was searched, and it names the current directory"
            );
        }

        // The loop above is necessary and it is not sufficient, and this test
        // claimed the opposite of that for as long as it existed. Nothing
        // called `abeam-planted` exists relative to the test binary's current
        // directory either — the planted copy is in a temp directory — so every
        // one of those entries answers `None` whether or not `walk` refuses a
        // relative one. Delete `.filter(|dir| dir.is_absolute())` and the loop
        // stays green, which is the one thing a test of a filter must not do.
        //
        // What catches it is a relative entry that genuinely holds a file: with
        // the filter it is `None`, and without it the walk returns the file.
        let (relative, name) = a_relative_entry_holding_a_file();
        assert!(
            relative.join(&name).is_file(),
            "the entry does name a file, relatively"
        );
        assert_eq!(
            walk(relative.as_os_str(), &name),
            None,
            "a relative PATH entry was searched, and what it names is the \
             current directory"
        );

        // ...and an absolute entry still is, so the refusals above are the
        // filter choosing rather than the walk having stopped working.
        assert_eq!(
            walk(dir.path().as_os_str(), planted),
            Some(dir.path().join(planted)),
            "an absolute PATH entry is still searched"
        );

        // The end of the same story: nothing on PATH, so nothing is started.
        assert!(resolve(planted, &[]).is_err());
    }

    #[test]
    fn a_relative_path_is_refused_rather_than_resolved() {
        // `./tools/sh` would be resolved against the repository on screen,
        // which is the one directory in this question somebody else gets to
        // write to. Refusing says so; resolving would not.
        for relative in ["./tools/sh", "tools/sh", "../sh"] {
            let refused = resolve(relative, &[]).expect_err("a relative path is not a program");
            assert!(refused.contains("relative"), "got: {refused}");
            assert!(refused.contains("absolute"), "the way out has to be named");
        }
    }

    #[test]
    fn an_executable_is_started_directly_and_nothing_is_added_to_it() {
        // The twin of `an_exe_is_started_directly_...`, and on this platform it
        // is the only case there is: nothing abeam starts here pays for an
        // interpreter or an environment variable, so if either ever appears in
        // one of these fields something has been added that does not belong.
        let dir = TempDir::new("launch-exec");
        // `write_exec_unrun`, because "started directly" here is a claim about
        // the `Launch` this resolves to and not about a process: the assertions
        // below read fields, and nothing in this test execs anything.
        let exe = dir.write_exec_unrun("abeam-direct", b"#!/bin/sh\nexit 0\n");
        let launch = resolve(&exe.to_string_lossy(), &args(&["--flag", "a b"])).unwrap();

        assert_eq!(launch.program, exe);
        assert_eq!(
            launch.target, exe,
            "the file started is the file that works"
        );
        assert_eq!(launch.args, args(&["--flag", "a b"]));
        assert!(launch.env.is_empty(), "no command line to carry");
    }

    #[test]
    fn a_shebang_script_is_started_directly_with_no_interpreter_in_front_of_it() {
        // The exact opposite number of `a_cmd_is_started_by_naming_cmd_exe_in_
        // front_of_it`, and the test that pins this module's one real claim:
        // the kernel reads the `#!` line, so abeam does not, and a script is
        // started the same way a binary is. Without this, the module reads like
        // a port somebody stopped halfway through and the next person
        // "finishes" it by putting `sh -c` in front — which would hand every
        // argument below to a parser that reads `&` as a command separator.
        //
        // This is also the shape of the file that broke abeam on Windows: one
        // extensionless script, which is exactly what `npm i -g` installs.
        let dir = TempDir::new("launch-shebang");
        let script = dir.write_exec_unrun("abeam-shim", b"#!/bin/sh\necho hello\n");
        let launch = resolve(&script.to_string_lossy(), &args(&["one", "a&b"])).unwrap();

        assert_eq!(
            launch.program, script,
            "a shell was named in front of the script"
        );
        assert_eq!(launch.target, script, "and it is its own target");
        assert_eq!(
            launch.args,
            args(&["one", "a&b"]),
            "the arguments are the caller's, all of them and only them"
        );
        assert!(
            launch.env.is_empty(),
            "no command line travels in the environment on this platform"
        );
    }

    #[test]
    fn a_file_that_may_not_be_executed_is_refused_with_the_way_out_named() {
        // The one thing that can go wrong here, and the sentence has to be
        // about the file rather than about the search: "was not found on PATH"
        // said of a file the user can see with `ls` sends them looking for an
        // install that is already there. In the common case eight characters
        // fix it, so the message names them.
        let dir = TempDir::new("launch-nonexec");
        let script = dir.write("abeam-shim", b"#!/bin/sh\n");
        let refused = resolve(&script.to_string_lossy(), &[])
            .expect_err("a file nobody may execute is not a program");

        assert!(
            refused.contains("abeam-shim"),
            "the file, not the name asked for: {refused}"
        );
        assert!(
            refused.contains("chmod"),
            "the way out has to be named: {refused}"
        );
        assert!(
            !refused.contains("not found"),
            "a file that is plainly there was reported missing: {refused}"
        );

        // ...and it must not promise `chmod` is *the* answer. `startable` asks
        // `access(X_OK)`, which is also false for a file whose execute bits are
        // set for somebody else and for anything on a `noexec` mount — and in
        // both of those a reader who has been told to chmod goes round a loop,
        // because chmod either fails or changes nothing. Only the first case is
        // reachable from a test running as one ordinary user, so this asserts
        // on the sentence rather than provoking the other two.
        assert!(
            refused.contains("noexec"),
            "the cases where chmod is not the fix have to be named too: {refused}"
        );
    }

    #[test]
    fn the_whole_npm_layout_resolves_to_the_one_file_it_installs_here() {
        // The mirror image of `the_whole_npm_layout_resolves_to_the_one_file_of
        // _the_three_that_can_run`, and the mirror is the point. That test
        // needs three files, a `PATHEXT`, a preference order and `cmd.exe` to
        // get from the name to a program. `npm i -g` on this platform writes
        // one file — extensionless, executable, `#!/usr/bin/env node` on the
        // first line — and getting from the name to the program is finding it.
        let dir = TempDir::new("launch-npm");
        // `write_exec_unrun` and not the probing twin, which matters more here
        // than anywhere else in this file: the first line names an interpreter
        // this crate has nothing to do with, so a probe would start whatever
        // `node` is on the machine, with an empty script, as a side effect of
        // writing a file. Nothing in this test runs the shim — it is found and
        // resolved, which is what `npm i -g` layout means.
        let shim = dir.write_exec_unrun("abeam-agent", b"#!/usr/bin/env node\n");

        // The way `abeam +abeam-agent` would reach it, with the process's own
        // `PATH` left alone — it is shared with every test running beside this.
        assert_eq!(
            walk(dir.path().as_os_str(), "abeam-agent"),
            Some(shim.clone())
        );

        let launch = resolve(&shim.to_string_lossy(), &[]).unwrap();
        assert_eq!(launch.target, shim);
        assert_eq!(launch.program, shim, "node was not named in front of it");
        assert!(launch.env.is_empty());
    }

    #[test]
    fn a_file_nobody_may_run_does_not_shadow_the_program_further_along_path() {
        // Windows makes this choice inside one directory — first match that can
        // be *started*, rather than Windows' own first match — and this is the
        // same choice lifted to the walk, because on this platform a directory
        // only ever holds one candidate. What it prevents: a stray unrunnable
        // `claude` early on `PATH` (a downloaded copy, a checked-out dotfile)
        // hiding the real install later on it, which would be reported as a
        // permissions problem on a file the user has never heard of.
        let earlier = TempDir::new("launch-shadow-earlier");
        let later = TempDir::new("launch-shadow-later");
        let unusable = earlier.write("abeam-agent", b"#!/bin/sh\n");
        let usable = later.write_exec_unrun("abeam-agent", b"#!/bin/sh\n");

        assert_eq!(
            walk(
                OsStr::new(&path_of(&[earlier.path(), later.path()])),
                "abeam-agent"
            ),
            Some(usable),
            "an unrunnable file earlier on PATH hid the program behind it"
        );

        // ...and with nothing runnable anywhere on it, the file that is there
        // is still the one worth naming, because the sentence it produces is
        // the one the user can act on.
        assert_eq!(
            walk(OsStr::new(&path_of(&[earlier.path()])), "abeam-agent"),
            Some(unusable.clone()),
            "the only copy on PATH was dropped, leaving nothing to name"
        );
        let refused = into_launch(unusable, &[]).expect_err("it still cannot be run");
        assert!(refused.contains("chmod"), "got: {refused}");
    }

    #[test]
    fn a_hint_that_is_not_absolute_is_trusted_no_further_than_a_path_entry_is() {
        // `panes::shell` builds its hint by joining onto an environment
        // variable, so an empty or relative one hands this module a relative
        // path — and `is_file()` on one asks about the current directory, which
        // under abeam is the repository on screen. Demonstrated without
        // standing anywhere in particular: a file that exists relative to
        // wherever the test binary happens to be is exactly the shape of answer
        // that must not be taken.
        let here = std::env::current_dir().expect("a current directory");
        let relative = std::fs::read_dir(&here)
            .expect("read the current directory")
            .flatten()
            .find(|entry| entry.path().is_file())
            .map(|entry| PathBuf::from(entry.file_name()))
            .expect("the current directory has a file in it");
        assert!(
            relative.is_file(),
            "the hint does name something, relatively"
        );

        assert!(
            find("abeam-no-such-hinted-program", Some(relative)).is_err(),
            "a relative hint was taken, and what it names is the current directory"
        );

        // The hint still works when it is what it is supposed to be, so the
        // refusal above is the check rather than the hint being ignored.
        let dir = TempDir::new("launch-hint");
        let exe = dir.write_exec_unrun("abeam-hinted", b"#!/bin/sh\n");
        assert_eq!(
            find("abeam-hinted", Some(exe.clone())).expect("an absolute hint"),
            exe
        );
    }

    // --- and it actually starts -------------------------------------------
    //
    // Everything above is a claim about what abeam decided. This is the only
    // place anything asks the kernel.

    /// A `#!/bin/sh` script that prints its whole argument list back, in a
    /// directory with a space in its name — the same hazard, and the same shim,
    /// as the Windows suite's.
    fn shim(dir: &TempDir) -> PathBuf {
        let home = dir.path().join("with space");
        std::fs::create_dir_all(&home).expect("a directory with a space in it");
        // Through `TempDir::write_exec` rather than the `write` and
        // `set_permissions` of its own that these six lines used to be. The
        // execute bit is now the smaller half of what that call does: it is
        // also the only thing that waits out the `ETXTBSY` window a shim
        // written and then *started* by a parallel test suite opens — see
        // `crate::testutil::past_text_file_busy`. This is one of the few
        // places that really does start what it wrote, so a shim written by
        // hand here would be a shim outside that.
        dir.write_exec(
            "with space/abeam-shim",
            b"#!/bin/sh\necho \"ABEAM-SHIM-OK [$@]\"\n",
        )
    }

    /// Run one and give back the screen it printed on.
    ///
    /// The same helper as the Windows suite's, deliberately: a twenty-second
    /// deadline copied is a twenty-second deadline that drifts, and these two
    /// files should fail the same way when a pty stops delivering output.
    fn shim_screen(cfg: PtyConfig) -> String {
        use crate::pane::Pane;
        use crate::panes::TerminalPane;
        use std::time::{Duration, Instant};

        let mut pane = TerminalPane::spawn_with(cfg).expect("spawn the shim");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            pane.tick();
            let screen = pane.last_screen().join("\n");
            if screen.contains("ABEAM-SHIM-OK") {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "the shim never printed anything:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_shebang_shim_really_runs_under_a_pty_with_its_arguments_intact() {
        // The honest counterpart to the Windows quoting suite. Over there,
        // `a&b`, `%VAR%` and `!BANG!` are the entire reason `command_line`
        // exists: unquoted, the first ends abeam's command and starts somebody
        // else's, the second is expanded by `cmd` before the child sees it, and
        // the third disappears into a delayed expansion. Here the same three
        // arrive as themselves because nothing on the way re-parses them —
        // there is no shell between `resolve` and the child — and a test is the
        // only way to say that without being taken on trust.
        //
        // The directory has a space in it for the same reason it does there: a
        // space is what breaks the moment anybody turns an argument vector back
        // into a command line, and this is where that would show.
        let dir = TempDir::new("launch-spawn");
        let script = shim(&dir);
        let home = script.parent().expect("the shim has a directory");

        // Found the way a `PATH` lookup would find it, so the resolution under
        // test is the one an `abeam +claude` performs...
        assert_eq!(
            walk(home.as_os_str(), "abeam-shim"),
            Some(script.clone()),
            "the shim is reachable by bare name"
        );
        // ...and then resolved by path, because putting a directory on this
        // process's `PATH` would put it on the `PATH` of every other test
        // running beside this one.
        let launch = resolve(
            &script.to_string_lossy(),
            &args(&["plain", "a&b", "%VAR%", "!BANG!"]),
        )
        .unwrap();
        assert_eq!(
            launch.program, script,
            "nothing was named in front of the shim"
        );

        // Wide enough that nothing on the line wraps: the assertion is about
        // the text, and a rejoin would hide a space that moved.
        let screen = shim_screen(launch.config().cwd(dir.path()).size(10, 200));
        assert!(
            screen.contains("ABEAM-SHIM-OK [plain a&b %VAR% !BANG!]"),
            "the shim was started but its arguments did not survive:\n{screen}"
        );
    }
}
