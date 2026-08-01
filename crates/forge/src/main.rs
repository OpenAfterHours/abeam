//! forge — one window for a Claude Code session.
//!
//! Claude is hosted in the left pane, rendered by us: bytes come out of a pty,
//! through a vt100 parser, into a ratatui widget. Nothing passes through to the
//! real terminal. The right pane toggles between a git view and a file /
//! markdown viewer that follows what Claude just wrote.
//!
//! Usage:  forge                 (hosts `claude`)
//!         forge powershell      (hosts something else)
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
    let mut terminal = term::setup()?;

    let result = (|| -> Result<Outcome> {
        let size = terminal.size()?;
        let full = ratatui::layout::Rect::new(0, 0, size.width, size.height);
        // A first guess only. The first frame re-sizes the pty from the rect it
        // actually drew, so this just avoids spawning Claude at 24x80 and
        // making it reflow immediately.
        let inner = layout::inner(layout::split(full, false).left);

        let left = TerminalPane::spawn(
            &program,
            &program_args,
            inner.height.max(1),
            inner.width.max(1),
        )?;
        App::new(left, root).run(&mut terminal)
    })();

    term::restore()?;

    match result? {
        Outcome::Exited { status, screen } => {
            // Onto the primary buffer, now that the alternate one is gone with
            // everything that was ever drawn on it. A session that leaves no
            // trace in your scrollback is a worse terminal than the plain one
            // forge replaced.
            for line in screen {
                println!("{line}");
            }
            println!("{program} exited: {status:?}");
            std::io::Write::flush(&mut std::io::stdout())?;
            // Anything scripting forge — `forge claude -p "…" && next-step`, a
            // CI wrapper — reads this, and a failed child reported as success
            // is the kind of thing that is only noticed much later.
            std::process::exit(status.exit_code() as i32);
        }
        Outcome::Detached => {}
    }
    Ok(())
}
