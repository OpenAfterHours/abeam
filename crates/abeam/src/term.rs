//! Host-terminal state: the modes abeam turns on, and getting them back off.
//!
//! This is deliberately abeam's job and not `abeam-pty`'s. A library that hosts
//! a pty must not seize raw mode — its user already owns the terminal and is
//! already inside a draw loop.

use anyhow::Result;
use crossterm::clipboard::CopyToClipboard;
use crossterm::cursor::Show;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use ratatui::Terminal;

use crate::app::Tui;

mod frame;
pub use frame::{DrawStats, FrameBackend, draw};

/// Enters raw mode and installs a panic hook that leaves it again. A panic
/// inside raw mode otherwise leaves the user with an unusable terminal and no
/// backtrace they can read.
///
/// Note that `EnableMouseCapture` disables the host terminal's own text
/// selection. Shift+drag is the terminal's own way back to it and which
/// terminals honour that varies, which is why abeam has a selection of its own:
/// `F7` and a drag both drive `crate::select`, and
/// [`copy_to_clipboard`] below is where what it names goes.
pub fn setup() -> Result<Tui> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        hook(info);
    }));

    let result = (|| {
        enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;

        let mut terminal = Terminal::new(FrameBackend::new(std::io::stdout()))?;
        terminal.clear()?;
        Ok(terminal)
    })();
    if result.is_err() {
        let _ = restore();
    }
    result
}

/// Put `text` on the host terminal's clipboard, over OSC 52.
///
/// **The escape sequence is the whole mechanism, and abeam never sees a
/// clipboard.** That is what makes it work in the one place a clipboard library
/// cannot: a session over SSH, where the terminal holding the clipboard is on
/// the other machine. It costs a dependency abeam already has — `crossterm`'s
/// `osc52` feature, and `base64` under it — rather than a per-platform
/// clipboard stack with X11 and Wayland behind it on Linux.
///
/// **There is no reply, so there is nothing to check.** A terminal that
/// honoured it says nothing; a terminal that does not implement it says nothing
/// either. Windows Terminal, VS Code, iTerm2, kitty, WezTerm and Alacritty
/// honour it; a legacy `conhost` without VT does not, and that one at least
/// fails loudly here, because `CopyToClipboard` has no Windows API fallback and
/// reports as much. `tmux` needs `set -g set-clipboard on` and passes it
/// through. `crate::app` says "copied" on the strength of the write, which is
/// what abeam actually did, and the note is deliberately not a promise about
/// somebody else's terminal.
///
/// Written straight to `stdout` rather than through the frame writer, which is
/// safe for one reason worth writing down: `App::draw` flushes at the end of
/// every frame, and this is only ever reached from a keystroke — between
/// frames, never inside one. A write that landed mid-frame would appear in the
/// middle of a repaint, and OSC 52 is one sequence the terminal would then eat
/// half of.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    execute!(std::io::stdout(), CopyToClipboard::to_clipboard_from(text))?;
    Ok(())
}

pub fn restore() -> Result<()> {
    let output = execute!(
        std::io::stdout(),
        EndSynchronizedUpdate,
        Show,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    // Raw mode must be restored even if the output handle has disappeared.
    let raw = disable_raw_mode();
    output?;
    raw?;
    Ok(())
}
