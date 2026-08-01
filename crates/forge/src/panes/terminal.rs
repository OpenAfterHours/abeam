//! The Claude pane: a `PtySession` rendered through tui-term.
//!
//! Almost everything hard about hosting a pty lives in `forge-pty`. What is
//! left here is the part that needs to know it is a pane.

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use forge_pty::{ExitStatus, PtyConfig, PtySession};
use ratatui::Frame;
use ratatui::layout::Rect;
use tui_term::widget::PseudoTerminal;

use crate::pane::{Handled, Pane};

pub struct TerminalPane {
    session: PtySession,
    title: String,
    exited: Option<ExitStatus>,
}

impl TerminalPane {
    pub fn spawn(program: &str, args: &[String], rows: u16, cols: u16) -> Result<Self> {
        let session = PtySession::spawn(
            PtyConfig::new(program)
                .args(args.iter().cloned())
                .size(rows, cols),
        )?;
        Ok(Self {
            session,
            title: program.to_string(),
            exited: None,
        })
    }

    /// Non-blocking; the app loop calls this every iteration. Once it reports
    /// an exit it keeps reporting the same one, so the final frame can be drawn
    /// without racing the reader thread to a second answer.
    pub fn poll_exit(&mut self) -> Result<Option<ExitStatus>> {
        if self.exited.is_none() {
            self.exited = self.session.try_wait()?;
        }
        Ok(self.exited.clone())
    }

    pub fn has_exited(&self) -> bool {
        self.exited.is_some()
    }

    /// The hosted program's last screen, as plain rows with the trailing blank
    /// ones dropped.
    ///
    /// Printed to the primary buffer once forge has left the alternate screen.
    /// Without it, `/exit` takes the whole session with it: everything Claude
    /// drew lived on forge's alternate screen and goes when that does, which is
    /// a thing the plain terminal forge replaces does not do.
    ///
    /// `rows()` rather than `contents()` — the latter rejoins wrapped rows into
    /// logical lines and tells you nothing about layout
    /// (`docs/conpty-findings.md`, constraint 5).
    pub fn last_screen(&self) -> Vec<String> {
        let screen = self.session.screen();
        let (_, cols) = screen.size();
        let mut rows: Vec<String> = screen
            .rows(0, cols)
            .map(|r| r.trim_end().to_string())
            .collect();
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        rows
    }

    /// Everything below here is the terminal pane's alone, and is reached
    /// concretely rather than through `dyn Pane`. See the `Pane` trait.
    pub fn handle_paste(&mut self, text: &str) -> Result<()> {
        self.session.send_paste(text)?;
        Ok(())
    }

    /// The only call into the pty's resize in the whole program. It is a no-op
    /// when the size has not changed, which is what lets the app call it
    /// unconditionally after every frame.
    pub fn on_resize(&mut self, inner: Rect) -> Result<()> {
        self.session.resize(inner.height, inner.width)?;
        Ok(())
    }

    /// Pane-relative `(col, row)` of the text cursor, or `None` to hide it.
    ///
    /// The strongest focus signal available: if the cursor is not blinking in
    /// Claude's prompt, your keys are not going to Claude.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        let screen = self.session.screen();
        if screen.hide_cursor() {
            return None;
        }
        // vt100 reports (row, col); ratatui wants (x, y).
        let (row, col) = screen.cursor_position();
        Some((col, row))
    }

    /// A copy of everything the diagnostics view shows, taken on the frame that
    /// shows it.
    ///
    /// A snapshot rather than a borrow because the diagnostics pane is a pane
    /// like any other: the shell reaches it through `&mut dyn Pane`, which
    /// cannot coexist with a live borrow of the session it is describing. It is
    /// a dozen scalars and two short strings, once per frame, and only while
    /// the view is open.
    pub fn diagnostics(&self) -> Diagnostics {
        let screen = self.session.screen();
        let stats = self.session.stats();
        let (parser_rows, parser_cols) = screen.size();
        Diagnostics {
            alt_screen: screen.alternate_screen(),
            app_cursor: screen.application_cursor(),
            app_keypad: screen.application_keypad(),
            bracketed_paste: screen.bracketed_paste(),
            mouse_mode: format!("{:?}", screen.mouse_protocol_mode()),
            mouse_encoding: format!("{:?}", screen.mouse_protocol_encoding()),
            cursor: screen.cursor_position(),
            parser_size: (parser_rows, parser_cols),
            // Asked of the pty itself, not remembered from the last resize:
            // the point of showing both is to catch them disagreeing.
            pty_size: self.session.pty_size().ok(),
            bytes_read: stats.bytes_read,
            dsr_replies: stats.dsr_replies,
            keys_sent: stats.keys_sent,
            resizes: stats.resizes,
            reader_finished: stats.reader_finished,
            exited: self.exited.as_ref().map(|s| format!("{s:?}")),
        }
    }
}

/// What the pty layer is doing, frozen at one instant.
///
/// Every field here earns its place by having been the answer to a real
/// mystery during the spike — see the diagnostics table in
/// `docs/conpty-findings.md`. `dsr_replies` is the one to look at first.
pub struct Diagnostics {
    pub alt_screen: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse_mode: String,
    pub mouse_encoding: String,
    pub cursor: (u16, u16),
    /// `(rows, cols)` as the vt100 parser understands them.
    pub parser_size: (u16, u16),
    /// `(rows, cols)` as the pty reports them, or `None` if it would not say.
    pub pty_size: Option<(u16, u16)>,
    pub bytes_read: u64,
    pub dsr_replies: u64,
    pub keys_sent: u64,
    pub resizes: u64,
    pub reader_finished: bool,
    pub exited: Option<String>,
}

impl Pane for TerminalPane {
    fn title(&self) -> String {
        match &self.exited {
            Some(status) => format!("{} · exited ({:?})", self.title, status),
            None => self.title.clone(),
        }
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        f.render_widget(PseudoTerminal::new(&*self.session.screen()), inner);
    }

    fn tick(&mut self) -> bool {
        self.session.take_dirty()
    }

    /// The hosted app gets everything the shell did not claim, so this always
    /// reports `Yes` — an unbound key inside Claude is still Claude's business.
    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        self.session.send_key(key)?;
        Ok(Handled::Yes)
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        // Coordinates are already pane-relative; forge-pty stays out of the
        // question of where the pane is, and gates on what Claude enabled.
        Ok(self.session.send_mouse(ev, ev.column, ev.row)?.into())
    }
}
