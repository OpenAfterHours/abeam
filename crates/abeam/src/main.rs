//! abeam — one window for a Claude Code session.
//!
//! Claude is hosted in the left pane, rendered by us: bytes come out of a pty,
//! through a vt100 parser, into a ratatui widget. Nothing passes through to the
//! real terminal. The right pane toggles between a git view and a file /
//! markdown viewer that follows what Claude just wrote.
//!
//! Usage:  abeam                 (hosts `claude`)
//!         abeam powershell      (hosts something else)
//!
//! Alt+Q quits, F1 lists the keys, F2 shows what the pty is doing.

mod app;
mod keys;
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

use abeam_pty::PtyConfig;
use anyhow::Result;

use crate::app::{App, Outcome};
use crate::panes::TerminalPane;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (program, program_args) = match args.split_first() {
        Some((p, rest)) => (p.clone(), rest.to_vec()),
        None => ("claude".to_string(), Vec::new()),
    };

    let root = std::env::current_dir()?;

    // `abeam ./tools/agent.exe` named a place, and meant it relative to where
    // abeam was run — which is about to stop being this process's directory.
    // Resolved here, while it still means that.
    let program = match Path::new(&program) {
        p if p.is_relative() && p.parent().is_some_and(|d| !d.as_os_str().is_empty()) => {
            root.join(p).to_string_lossy().into_owned()
        }
        _ => program,
    };

    // Then stand somewhere nobody else can write to, for the rest of the
    // session. Every program abeam starts, it starts by name, and Windows
    // resolves a bare name in `CreateProcessW` against *this* process's current
    // directory before it consults `PATH` — so with abeam sitting in a
    // repository, a file called `claude.exe` or `pwsh.exe` committed to that
    // repository is what runs, with the user's full token. It costs nothing
    // because every pty abeam opens is given an explicit working directory, and
    // it covers the panes as well as this line. A failure leaves us exactly
    // where the program stood before, which is why it is not fatal.
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        let _ = std::env::set_current_dir(system_root);
    }

    let mut terminal = term::setup()?;

    let result = (|| -> Result<Outcome> {
        let size = terminal.size()?;
        let full = ratatui::layout::Rect::new(0, 0, size.width, size.height);
        // A first guess only. The first frame re-sizes the pty from the rect it
        // actually drew, so this just avoids spawning Claude at 24x80 and
        // making it reflow immediately.
        let inner = layout::inner(layout::split(full, false).left);

        let left = TerminalPane::spawn_with(
            PtyConfig::new(&program)
                .args(program_args.iter().cloned())
                // Explicit, and it has to be: `PtySession::spawn` falls back to
                // the process's own directory, and after the line above that is
                // `%SystemRoot%`. Claude belongs in the repository it was
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
            println!("{program} exited: {status:?}");
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
