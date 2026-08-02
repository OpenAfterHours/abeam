//! abeam — one window for an AI coding session.
//!
//! The agent is hosted in the left pane, rendered by us: bytes come out of a
//! pty, through a vt100 parser, into a ratatui widget. Nothing passes through
//! to the real terminal. The right pane toggles between a git view and a file /
//! markdown viewer that follows what the agent just wrote.
//!
//! Usage:  abeam                 (hosts the default agent)
//!         abeam copilot         (hosts a known agent — see `crate::agent`)
//!         abeam powershell      (hosts anything else)
//!
//! Alt+Q quits, F1 lists the keys, F2 shows what the pty is doing.

mod agent;
mod app;
mod keys;
mod launch;
mod layout;
mod pane;
mod panes;
mod scroll;
mod term;
mod text;
mod watch;

#[cfg(test)]
mod testutil;

use std::path::Path;

use anyhow::Result;

use crate::app::{App, Outcome};
use crate::panes::TerminalPane;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // First of everything, and for a stronger version of the reason the
    // resolution below is where it is: until this existed, `abeam --help`
    // reached `CreateProcessW` as a program called `--help`, and the answer to
    // a question about abeam was a spawn failure naming a flag.
    let (choice, program_args) = match agent::parse(&args) {
        Ok(agent::Cli::Help) => {
            println!("{}", agent::help());
            return Ok(());
        }
        Ok(agent::Cli::Version) => {
            println!("{}", agent::version());
            return Ok(());
        }
        Ok(agent::Cli::Host { choice, args }) => (choice, args),
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(2);
        }
    };

    let root = std::env::current_dir()?;

    // Before `term::setup`, and that is the whole reason it is on this line. A
    // program that cannot be found is the single most likely way for abeam to
    // fail, and failing after raw mode and the alternate screen are on leaves
    // the console in a state the shell underneath does not recover from — the
    // message is there, on a screen that has just been thrown away.
    let hosted = match host(choice, &program_args, &root) {
        Ok(hosted) => hosted,
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(2);
        }
    };

    // Then stand somewhere nobody else can write to, for the rest of the
    // session. This is the second half of a two-part defence, and the half that
    // is kept precisely because it does not depend on the first one holding.
    //
    // The first half is `crate::launch`, which resolves every program to an
    // absolute path before `CreateProcessW` ever sees it — nothing leaves that
    // module that is not absolute, and a bare name is exactly what Windows
    // resolves against *this* process's current directory before it consults
    // `PATH`. So with the resolver working, a `claude.exe` committed to the
    // repository on screen is already unreachable. This line is what stands
    // between a reader and that same file on the day the resolver is wrong: one
    // module's invariant, one missed call site, one library spawning something
    // on its own behalf. The cost of being wrong is a program running with the
    // user's full token, so it is worth paying for twice.
    //
    // It costs nothing, either: every pty abeam opens is given an explicit
    // working directory, and this covers the panes as well as this line. A
    // failure leaves us exactly where the program stood before, which is why it
    // is not fatal.
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        let _ = std::env::set_current_dir(system_root);
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
                // `%SystemRoot%`. The agent belongs in the repository it was
                // opened on.
                .cwd(&root)
                .size(inner.height.max(1), inner.width.max(1)),
        )?;
        App::new(left, root).run(&mut terminal)
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
            // Anything scripting abeam — `abeam claude -p "…" && next-step`, a
            // CI wrapper — reads this, and a failed child reported as success
            // is the kind of thing that is only noticed much later.
            std::process::exit(status.exit_code() as i32);
        }
        Outcome::Detached => {}
    }
    Ok(())
}

/// Turn what was asked for into something to start.
///
/// An agent is `crate::agent`'s question entirely — a table of candidates and
/// nothing else. A program named outright is this function's, and stays here
/// for one reason: `root`. It is the directory abeam was *run* in, and this is
/// the last moment at which that is also this process's current directory.
fn host(choice: agent::Choice, args: &[String], root: &Path) -> Result<agent::Hosted, String> {
    let asked = match choice {
        agent::Choice::Known(agent) => return agent::resolve(agent, args),
        agent::Choice::Program(asked) => asked,
    };

    // `abeam ./tools/agent.exe` named a place, and meant it relative to where
    // abeam was run — which is about to stop being this process's directory.
    // Resolved here, while it still means that.
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

    launch::resolve(&program, args).map(|launch| agent::Hosted::plain(&asked, launch))
}

/// Windows-only like the rest of the suite: what is under test is a `PATH` walk
/// and a path joined onto the directory abeam was started in.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn program(name: &str) -> agent::Choice {
        agent::Choice::Program(name.to_string())
    }

    #[test]
    fn a_program_named_outright_is_still_shown_under_the_name_that_was_typed() {
        // Today's behaviour, and the case the agent table must not have
        // changed: `abeam powershell` resolves a program and the border says
        // the word that was typed, not the absolute path it became.
        let root = std::env::temp_dir();
        let hosted = host(program("cmd.exe"), &[], &root).expect("cmd.exe is on PATH");

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
        let hosted = host(program(typed), &[], dir.path()).expect("joined onto the root");

        assert!(hosted.launch.program.is_absolute());
        assert!(hosted.launch.program.starts_with(dir.path()));
        assert!(hosted.launch.program.ends_with("abeam-rel.exe"));
        // Still the name that was typed. A border is 46 columns and an absolute
        // path is not what a reader needs in them.
        assert_eq!(hosted.name, typed);

        // ...and the same name without a directory component in it is a bare
        // name, which is `PATH`'s question and not this one's.
        assert!(host(program("abeam-rel.exe"), &[], dir.path()).is_err());
    }

    #[test]
    fn an_agent_is_resolved_by_its_table_rather_than_by_the_path_rule_above() {
        // The default is `claude`, which may or may not be installed on the
        // machine running this — so what is asserted is the routing, not the
        // outcome: whichever way it goes, it goes through the table, and the
        // name that comes back is the table's rather than a path.
        let agent = agent::find("claude").expect("claude is a known agent");
        match host(agent::Choice::Known(agent), &[], &std::env::temp_dir()) {
            Ok(hosted) => assert_eq!(hosted.name, "claude"),
            Err(why) => assert!(
                why.contains("`claude`") && why.contains("Tried:"),
                "an agent that is missing says what it looked for: {why}"
            ),
        }
    }
}
