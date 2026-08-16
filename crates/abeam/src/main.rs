//! abeam — one window for an AI coding session.
//!
//! The agent is hosted in the left pane, rendered by us: bytes come out of a
//! pty, through a vt100 parser, into a ratatui widget. Nothing passes through
//! to the real terminal. The right pane toggles between a git view and a file /
//! markdown viewer that follows what the agent just wrote.
//!
//! Usage:  abeam [args...]           (the default agent, handed the whole line)
//!         abeam +copilot [args...]  (a known agent — see `crate::agent`)
//!         abeam +pwsh               (anything else)
//!         abeam +help               (abeam's own; `--help` is the agent's)
//!
//! Alt+Q quits, F1 lists the keys, F2 shows what the pty is doing.

mod agent;
mod agentstate;
mod app;
mod ask;
mod config;
mod dispatch;
mod keys;
mod launch;
mod layout;
mod pane;
mod panes;
mod paths;
mod scroll;
mod select;
mod term;
mod text;
mod watch;
mod workspace;

#[cfg(test)]
mod testutil;

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::app::{App, Outcome};
use crate::panes::TerminalPane;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Before `term::setup` for the same reason everything else on this side of
    // it is, and before the parse because the parse needs what it produces: the
    // table a `+` is read against is the built-ins plus this file's presets, so
    // `abeam fleet` cannot be refused as a preset name until the presets are
    // known.
    //
    // A file that will not parse is fatal rather than ignored, which
    // `crate::config` argues at length and the short version of which is that
    // this file names programs to start. What it costs is that `+help` on a
    // machine with a broken config answers with the config error instead of the
    // help — and of those two answers, the one naming the file and the line is
    // the one worth having in front of you.
    let config = match config::load() {
        Ok(config) => config,
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(REFUSED);
        }
    };
    let table = config.table();

    // First of the command line, and for a stronger version of the reason the
    // resolution below is where it is: until this existed, `abeam --help`
    // reached `CreateProcessW` as a program called `--help`, and the answer to
    // a question about abeam was a spawn failure naming a flag.
    //
    // That papercut is worth keeping written down after the fact, because
    // "we added a check for it" and "it stopped being expressible" are very
    // different guarantees and the rule `crate::agent` now parses under buys
    // the second one — for the papercut, and *only* for the papercut. This
    // comment used to claim more, and the correction is the useful part. It
    // said a dashed token could no longer become a program name however it is
    // spelled. It can: `abeam +--help` names one, `ABEAM_AGENT=--help` names
    // one with no `+` on the line at all, and `abeam +./-weird` reaches a
    // dash-named file through the relative-path branch in `host` below. All
    // three are somebody asking for a dash-named program, which abeam has
    // always allowed on purpose and has a test pinning.
    //
    // What the rule deleted is the *route* — a dashed token becoming a program
    // name with nobody having named one. `--help` is Claude's question now, and
    // `+help` is the only spelling that reaches the arm below. What keeps a
    // dashed name that somebody *did* ask for off `CreateProcessW` is not this
    // rule and never was: it is `launch::find`, which answers only with a path
    // it has located and otherwise returns abeam's own "`--help` was not found
    // on PATH". That predates all of this.
    //
    // What this still catches before `term::setup` is the two answer-and-exit
    // words and the parser's refusals, which is the same reason as ever: a
    // message printed after raw mode is on is a message on a screen about to be
    // thrown away.
    let (choice, program_args) = match reading(&args, table) {
        Reading::Answered(said) => {
            println!("{said}");
            return Ok(());
        }
        Reading::Refused(why) => {
            eprintln!("{why}");
            std::process::exit(REFUSED);
        }
        Reading::Host(choice, args) => (choice, args),
    };

    // Read from the selection rather than from the command line, because a
    // preset can be selected three ways — `+fleet`, `ABEAM_AGENT=fleet`, and
    // `abeam` on its own if somebody makes one the default — and only the
    // parser knows which of them happened. A program named outright brings no
    // opening state with it, so it gets `[defaults]` like every other session.
    let opening = config.opening(match &choice {
        agent::Choice::Known(agent) => Some(agent.name),
        agent::Choice::Program { .. } => None,
    });

    // Resolved once, here, and never again: this is the only spelling of the
    // repository that anything downstream is allowed to hold — the pty's
    // working directory on the line below, the watcher, the first workspace,
    // the git pane, the reader and the readiness probe.
    //
    // `current_dir` is `GetCurrentDirectoryW` on Windows, which reports the
    // path this process was *given*. `git worktree list --porcelain` reports
    // the path the filesystem resolves to. A junction, a `subst` drive or an
    // 8.3 short name between them is enough to make those two different
    // strings for one directory, and `crate::paths` is explicit that two
    // spellings are two directories — so the watcher's events would be owned by
    // a root git never named, every neighbouring agent's writes would land in
    // this window, and the workspace list would say you are nowhere.
    // `crate::paths::resolve_root` is where the whole failure is written out.
    let root = paths::resolve_root(&std::env::current_dir()?);

    // Before `term::setup`, and that is the whole reason it is on this line. A
    // program that cannot be found is the single most likely way for abeam to
    // fail, and failing after raw mode and the alternate screen are on leaves
    // the console in a state the shell underneath does not recover from — the
    // message is there, on a screen that has just been thrown away.
    let hosted = match host(choice, &program_args, &root, table) {
        Ok(hosted) => hosted,
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(REFUSED);
        }
    };

    // Then stand somewhere nobody else can write to, for the rest of the
    // session.
    //
    // The defence a reader is looking for is `crate::launch`, which resolves
    // every program to an absolute path before the operating system's spawn
    // ever sees it — nothing leaves that module that is not absolute, so with
    // the resolver working a `claude.exe` committed to the repository on screen
    // is already unreachable. What this line adds on the day that is bypassed —
    // one missed call site, one library spawning something on its own behalf —
    // is a whole line of defence on Windows and *not one on Unix*, and saying
    // otherwise would tell the next reader they are covered where they are not.
    //
    // On Windows it is real. `CreateProcessW` resolves a bare name against
    // *this* process's current directory before it consults `PATH` at all, and
    // portable-pty's Windows `search_path` hands a name it could not find
    // through unchanged rather than binding it to anything — so where abeam is
    // standing is the answer, and standing in `%SystemRoot%` is what makes that
    // answer harmless.
    //
    // On Unix it buys nothing against that hazard, because abeam never reaches
    // `execvp`'s own `PATH` walk holding this process's directory. Every spawn
    // goes through `portable_pty::CommandBuilder::search_path`, which resolves
    // against `PtyConfig.cwd` and not against the process: for a bare name it
    // computes `cwd.join(entry).join(name)`, and for a `./x` it computes
    // `cwd.join(name)` (0.9.0, `src/cmdbuilder.rs`, `search_path` and
    // `as_command`). Every pty abeam opens is given `.cwd(&root)`, which
    // is the repository on screen — so a future call site building a
    // `PtyConfig` with a bare name would resolve it against the repository
    // however far this process has walked from it. There the backstop is
    // `launch::resolve` returning absolute paths, and there is nothing behind
    // it.
    //
    // Kept on both all the same, because it still covers anything that does
    // consult the process's own directory, and because it costs nothing: every
    // pty abeam opens is given an explicit working directory, so this covers
    // the panes as well as this line. A failure leaves us exactly where the
    // program stood before, which is why it is not fatal. And the residual
    // value on Unix is smaller again than `/` suggests — `uvx abeam` inside a
    // container commonly runs as root, where `/` is writable like anywhere
    // else.
    if let Some(unwritable) = somewhere_unwritable() {
        let _ = std::env::set_current_dir(unwritable);
    }

    let mut terminal = term::setup()?;

    let result = (|| -> Result<Outcome> {
        let size = terminal.size()?;
        let full = ratatui::layout::Rect::new(0, 0, size.width, size.height);
        // A first guess only. The first frame re-sizes the pty from the rect it
        // actually drew, so this just avoids spawning the agent at 24x80 and
        // making it reflow immediately.
        let inner = layout::inner(layout::split(full, false).left);

        let left = TerminalPane::spawn_with(
            hosted
                .launch
                .config()
                // The name that was chosen, not the path it turned into. An npm
                // install makes those `claude` and `cmd.exe` respectively, and
                // only one of them is worth a border in 46 columns.
                .title(&hosted.name)
                // Explicit, and it has to be: `PtySession::spawn` falls back to
                // the process's own directory, and after the line above that is
                // wherever abeam went to stand. The agent belongs in the
                // repository it was opened on.
                //
                // The *resolved* spelling of it, which is the second reason
                // that resolution happens before this line rather than inside
                // `App`. A child handed `…\link` reports `…\link` as its own
                // `cwd` — Node's `process.cwd()` is `GetCurrentDirectoryW` too
                // — and that is the string Claude writes into its session
                // record. `crate::agentstate::Probe` compares that record's
                // `cwd` with abeam's root to decide whether a queued prompt may
                // be typed, so starting the agent under one spelling while
                // holding the other is the silent, permanent stall that module
                // is written to make impossible.
                .cwd(&root)
                .size(inner.height.max(1), inner.width.max(1)),
        )?;
        // `hosted.agent` and not `hosted.name`: what the shell needs here is
        // what is actually taking the typing, because the question it goes on
        // to ask is whether `--bg` is a flag it has. A preset hosting Claude
        // answers `claude`, and a border reading `fleet` does not cost the
        // queue its dispatch mode.
        App::new(left, root, &hosted.agent, opening).run(&mut terminal)
    })();

    // Every frame ends by emptying the frame buffer, so this should have
    // nothing to do — but the alternate screen disappears on the next line, and
    // anything still held would land on the primary buffer as debris. The
    // certainty is worth one syscall on the way out.
    let _ = std::io::Write::flush(terminal.backend_mut());
    term::restore()?;

    match result? {
        Outcome::Exited { status, screen } => {
            // Onto the primary buffer, now that the alternate one is gone with
            // everything that was ever drawn on it. A session that leaves no
            // trace in your scrollback is a worse terminal than the plain one
            // abeam replaced.
            for line in screen {
                println!("{line}");
            }
            // The name that was chosen, for the same reason the border uses it.
            println!("{} exited: {status:?}", hosted.name);
            std::io::Write::flush(&mut std::io::stdout())?;
            // Anything scripting abeam — `abeam -p "…" && next-step`, a CI
            // wrapper — reads this, and a failed child reported as success is
            // the kind of thing that is only noticed much later.
            std::process::exit(status.exit_code() as i32);
        }
        Outcome::Detached => {}
    }
    Ok(())
}

/// The exit code every refusal on this side of `term::setup` leaves with.
///
/// One constant rather than the same literal at three call sites, because it is
/// a promise to whatever is scripting abeam — `abeam -p "…" && next-step`, a CI
/// wrapper — and the three are one promise rather than three. A `2` that drifted
/// to a `1` in one of them would be a promise kept in the other two, which is
/// the shape of bug nobody notices until a pipeline behaves differently
/// depending on which thing was wrong.
const REFUSED: i32 = 2;

/// What reading the command line settled, before anything has been started.
///
/// Three answers because `main` does three different things with them: print on
/// standard output and leave happy, print on standard error and leave with
/// [`REFUSED`], or go on and host something.
#[derive(Debug)]
enum Reading {
    /// abeam's own two words, answered.
    Answered(String),
    /// A command line abeam will not act on — the refusals in `crate::agent`.
    Refused(String),
    /// Something to host, and the arguments left for it.
    Host(agent::Choice, Vec<String>),
}

/// [`main`]'s first decision, as a value rather than as three `exit`s.
///
/// Split out for one reason and it is a testing one: `main` ends every one of
/// these arms in `println!` or `std::process::exit`, and no in-process test can
/// observe either. So the decision is here, where a test can hold it in its
/// hand, and `main` above is the three lines that turn it into output and a
/// status. What that leaves untested is `exit` being called at all, which is
/// the part `crates/abeam/tests/end_to_end.rs` reaches by spawning the real
/// binary; what it pins is the half that has actually gone wrong before, which
/// is *which* answer a command line produces.
fn reading(args: &[String], table: &'static [agent::Agent]) -> Reading {
    match agent::parse(args, table) {
        Ok(agent::Cli::Help) => Reading::Answered(agent::help(table)),
        Ok(agent::Cli::Version) => Reading::Answered(agent::version()),
        Ok(agent::Cli::Host { choice, args }) => Reading::Host(choice, args),
        Err(why) => Reading::Refused(why),
    }
}

/// Turn what was asked for into something to start.
///
/// An agent is `crate::agent`'s question entirely — a table of candidates and
/// nothing else. A program named outright is this function's, and stays here
/// for one reason: `root`. It is the directory abeam was *run* in, and this is
/// the last moment at which that is also this process's current directory.
fn host(
    choice: agent::Choice,
    args: &[String],
    root: &Path,
    table: &[agent::Agent],
) -> Result<agent::Hosted, String> {
    let (asked, whence) = match choice {
        agent::Choice::Known(agent) => return agent::resolve_within(agent, args, table),
        agent::Choice::Program { name, whence } => (name, whence),
    };

    // `abeam +./tools/agent.exe` named a place, and meant it relative to where
    // abeam was run — which is about to stop being this process's directory.
    // Resolved here, while it still means that. The sigil is gone by now: it
    // said which token was abeam's and never formed part of the path.
    //
    // It has to happen before `launch::resolve`, which refuses a relative path
    // outright: the two are asking different questions. There, a relative path
    // arrives from `ABEAM_SHELL` or from a candidate list and would be resolved
    // against the repository on screen, which somebody else writes to. Here it
    // arrives from the command line a person just typed, and `root` is the
    // directory they typed it in. Joining it makes it absolute, which is the
    // property everything downstream depends on.
    //
    // An agent never reaches this: every candidate in the table is a bare name,
    // which `launch::resolve` looks up on `PATH` and which has no directory
    // component to be relative to anything.
    let program = match Path::new(&asked) {
        p if p.is_relative() && p.parent().is_some_and(|d| !d.as_os_str().is_empty()) => {
            root.join(p).to_string_lossy().into_owned()
        }
        _ => asked.clone(),
    };

    // `launch`'s own sentence names the file it went looking for and stops
    // there, which is right for a module whose whole subject is the search.
    // What it cannot know is *why abeam was looking for that*, and that is the
    // half a reader is most often missing here — one of them typed a `+` in
    // front of a prompt, and the other has a variable set from another year.
    // `agent::nowhere` writes the paragraph; `whence` is the only fact it needs
    // and the only one that cannot be recovered from a failed `PATH` walk.
    launch::resolve(&program, args)
        .map(|launch| agent::Hosted::plain(&asked, launch))
        .map_err(|why| agent::nowhere(&asked, whence, &why))
}

/// Where abeam goes to stand for the rest of the session, or `None` if this
/// machine will not say.
///
/// Asked of the environment rather than spelled `C:\Windows`: the Windows
/// directory is not always on `C:` and not always called that, and a hardcoded
/// path that is wrong on a machine is a `set_current_dir` that quietly does
/// nothing on it.
#[cfg(windows)]
fn somewhere_unwritable() -> Option<PathBuf> {
    std::env::var_os("SystemRoot").map(PathBuf::from)
}

/// The Unix answer to the same question, and there is nothing to ask: `/` is
/// the one directory a Unix cannot be missing, every user can read it and an
/// ordinary one cannot write to it. `Some` unconditionally, so that the caller
/// reads the same on both platforms rather than growing a `cfg` of its own.
#[cfg(unix)]
fn somewhere_unwritable() -> Option<PathBuf> {
    Some(PathBuf::from("/"))
}

/// The one question here that is about neither platform's `PATH`.
#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn an_agent_is_resolved_by_its_table_rather_than_by_the_path_rule_above() {
        // The default is `claude`, which may or may not be installed on the
        // machine running this — so what is asserted is the routing, not the
        // outcome: whichever way it goes, it goes through the table, and the
        // name that comes back is the table's rather than a path.
        //
        // Ungated, and that is the point of it being here rather than in either
        // module below: the routing is the same decision on every platform, and
        // a copy of this test per platform would be two places to notice that
        // an agent had started going through the path rule instead.
        let agent = agent::find("claude").expect("claude is a known agent");
        match host(
            agent::Choice::Known(agent),
            &[],
            &std::env::temp_dir(),
            agent::AGENTS,
        ) {
            Ok(hosted) => assert_eq!(hosted.name, "claude"),
            Err(why) => assert!(
                why.contains("`claude`") && why.contains("Tried:"),
                "an agent that is missing says what it looked for: {why}"
            ),
        }
    }

    #[test]
    fn a_command_line_that_used_to_select_is_refused_with_the_code_a_script_reads() {
        // `abeam claude` is what every older copy of the README says to type,
        // and it exits 2 rather than starting anything — which matters to
        // whatever is on the other side of an `&&`. This is as close to `main`
        // as an in-process test gets: `main` ends these arms in `exit`, which
        // no test in this binary can observe, so what is pinned here is that
        // the refusal reaches the arm holding the code and that the code is 2.
        // The `exit` itself is `tests/end_to_end.rs`'s job.
        let Reading::Refused(why) = reading(&[String::from("claude")], agent::AGENTS) else {
            panic!("`abeam claude` is refused rather than hosted");
        };
        assert!(why.contains("used to host"), "got: {why}");
        assert_eq!(
            REFUSED, 2,
            "the exit code is a promise to whatever is scripting abeam"
        );

        // The other two answers, so that this is a test of the routing rather
        // than of one arm of it. Neither of these depends on the process
        // environment: `+help` is answered before the default is read, and a
        // `+` token overrides `ABEAM_AGENT` outright — which matters, because
        // this binary shares its environment with three hundred other tests.
        assert!(matches!(
            reading(&[String::from("+help")], agent::AGENTS),
            Reading::Answered(_)
        ));
        assert!(matches!(
            reading(&[String::from("+version")], agent::AGENTS),
            Reading::Answered(_)
        ));
        assert!(matches!(
            reading(&[String::from("+claude")], agent::AGENTS),
            Reading::Host(..)
        ));
    }
}

/// Windows-side: what is under test is a `PATH` walk and a path joined onto the
/// directory abeam was started in, and both of those are spelled differently
/// enough on Unix to want the twin below rather than a `cfg` in the middle of
/// each assertion.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// A program named by a `+` token, which is the way one is usually named.
    /// Which of the two ways in it was changes nothing about the resolution —
    /// `crate::agent::Whence` is read on the failure path and nowhere else —
    /// so these tests pick one and say so rather than running each twice.
    fn program(name: &str) -> agent::Choice {
        agent::Choice::Program {
            name: name.to_string(),
            whence: agent::Whence::Sigil,
        }
    }

    #[test]
    fn a_program_named_outright_is_still_shown_under_the_name_that_was_typed() {
        // Today's behaviour, and the case neither the agent table nor the sigil
        // may have changed: `abeam +powershell` resolves a program and the
        // border says the word that was typed, not the absolute path it became.
        let root = std::env::temp_dir();
        let hosted =
            host(program("cmd.exe"), &[], &root, agent::AGENTS).expect("cmd.exe is on PATH");

        assert_eq!(hosted.name, "cmd.exe");
        assert!(hosted.launch.program.is_absolute());
    }

    #[test]
    fn a_relative_path_is_resolved_against_the_directory_abeam_was_run_in() {
        // `crate::launch` refuses a relative path outright, because there one
        // arrives from `ABEAM_SHELL` or a candidate list and would be resolved
        // against the repository on screen. One typed on the command line is a
        // different question with a different answer, and this is the only
        // place that knows which directory it was typed in.
        let dir = TempDir::new("main-relative");
        std::fs::create_dir_all(dir.path().join("tools")).expect("a subdirectory");
        std::fs::write(dir.path().join("tools").join("abeam-rel.exe"), b"MZ").expect("a program");

        let typed = r".\tools\abeam-rel.exe";
        let hosted =
            host(program(typed), &[], dir.path(), agent::AGENTS).expect("joined onto the root");

        assert!(hosted.launch.program.is_absolute());
        assert!(hosted.launch.program.starts_with(dir.path()));
        assert!(hosted.launch.program.ends_with("abeam-rel.exe"));
        // Still the name that was typed. A border is 46 columns and an absolute
        // path is not what a reader needs in them.
        assert_eq!(hosted.name, typed);

        // ...and the same name without a directory component in it is a bare
        // name, which is `PATH`'s question and not this one's.
        assert!(host(program("abeam-rel.exe"), &[], dir.path(), agent::AGENTS).is_err());
    }
}

/// The Unix twin of the two above, asking the same two questions of a `PATH`
/// walk that answers with the execute bit rather than with an extension.
#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use crate::testutil::TempDir;

    /// A program named by a `+` token, which is the way one is usually named.
    /// Which of the two ways in it was changes nothing about the resolution —
    /// `crate::agent::Whence` is read on the failure path and nowhere else —
    /// so these tests pick one and say so rather than running each twice.
    fn program(name: &str) -> agent::Choice {
        agent::Choice::Program {
            name: name.to_string(),
            whence: agent::Whence::Sigil,
        }
    }

    #[test]
    fn a_program_named_outright_is_still_shown_under_the_name_that_was_typed() {
        // Today's behaviour, and the case neither the agent table nor the sigil
        // may have changed: `abeam +pwsh` resolves a program and the border
        // says the word that was typed, not the absolute path it became.
        //
        // `sh` because it is the one program name a Unix is not allowed to be
        // missing, so this test failing means the resolver and not the runner.
        let root = std::env::temp_dir();
        let hosted = host(program("sh"), &[], &root, agent::AGENTS).expect("sh is on PATH");

        assert_eq!(hosted.name, "sh");
        assert!(hosted.launch.program.is_absolute());
    }

    #[test]
    fn a_relative_path_is_resolved_against_the_directory_abeam_was_run_in() {
        // `crate::launch` refuses a relative path outright, because there one
        // arrives from `ABEAM_SHELL` or a candidate list and would be resolved
        // against the repository on screen. One typed on the command line is a
        // different question with a different answer, and this is the only
        // place that knows which directory it was typed in.
        let dir = TempDir::new("main-relative");
        std::fs::create_dir_all(dir.path().join("tools")).expect("a subdirectory");
        // With the execute bit, because that is the whole of what makes a file
        // a program here — the Windows twin can write two bytes and be believed,
        // and this one cannot.
        dir.write_exec("tools/abeam-rel", b"#!/bin/sh\nexit 0\n");

        let typed = "./tools/abeam-rel";
        let hosted =
            host(program(typed), &[], dir.path(), agent::AGENTS).expect("joined onto the root");

        assert!(hosted.launch.program.is_absolute());
        assert!(hosted.launch.program.starts_with(dir.path()));
        assert!(hosted.launch.program.ends_with("abeam-rel"));
        // Still the name that was typed. A border is 46 columns and an absolute
        // path is not what a reader needs in them.
        assert_eq!(hosted.name, typed);

        // ...and the same name without a directory component in it is a bare
        // name, which is `PATH`'s question and not this one's.
        assert!(host(program("abeam-rel"), &[], dir.path(), agent::AGENTS).is_err());
    }
}
