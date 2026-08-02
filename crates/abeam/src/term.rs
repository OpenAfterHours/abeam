//! Host-terminal state: the modes abeam turns on, and getting them back off.
//!
//! This is deliberately abeam's job and not `abeam-pty`'s. A library that hosts
//! a pty must not seize raw mode — its user already owns the terminal and is
//! already inside a draw loop.

use std::io::BufWriter;

use anyhow::Result;
use crossterm::execute;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::Tui;

/// Big enough that a whole frame fits in it with room to spare.
///
/// A full repaint of a 60% pane in a 200-column window measures about 10 KB of
/// escape sequences — which matters because the thing underneath is
/// `std::io::Stdout`, and that is a `LineWriter` with a **1 KB** buffer. Frame
/// output contains no newlines, so a frame left to it becomes ten separate
/// writes into ConPTY instead of one. Measured, not assumed.
const FRAME_BUF: usize = 64 * 1024;

/// Enters raw mode and installs a panic hook that leaves it again. A panic
/// inside raw mode otherwise leaves the user with an unusable terminal and no
/// backtrace they can read.
///
/// Note that `EnableMouseCapture` disables the host terminal's own text
/// selection; copying out of abeam needs Shift+drag, and which terminals honour
/// that varies.
pub fn setup() -> Result<Tui> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        hook(info);
    }));

    enable_raw_mode()?;
    execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;

    let mut terminal = Terminal::new(CrosstermBackend::new(BufWriter::with_capacity(
        FRAME_BUF,
        std::io::stdout(),
    )))?;
    terminal.clear()?;
    Ok(terminal)
}

pub fn restore() -> Result<()> {
    execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    disable_raw_mode()?;
    Ok(())
}
