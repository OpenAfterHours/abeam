//! A pane with a live child in it: a `PtySession` rendered through tui-term.
//!
//! Not "the agent pane", though that is what it began as. `panes::shell` hosts
//! one of these too, so every fact here has to be true of the agent on the left
//! *and* of whatever somebody typed into the command view on the right — which
//! is why nothing below names either of them.
//!
//! Almost everything hard about hosting a pty lives in `abeam-pty`. What is
//! left here is the part that needs to know it is a pane.

use abeam_pty::{ExitStatus, PtyConfig, PtySession};
use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
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
    /// A child and a size, for the tests that want nothing else.
    ///
    /// `main` used this until it acquired a working directory it had to pass —
    /// abeam now spends the session standing in a directory an ordinary user
    /// cannot write to, so a pty that is not told where to start starts there.
    /// Not one *nobody* can write to, and `main` is careful about the same
    /// thing: on Unix that directory is `/`, and a `uvx abeam` in a container
    /// is commonly running as root. Test-only rather than merely unused,
    /// because a constructor that silently accepts the process's own directory
    /// is exactly what should no longer be reachable from the program.
    #[cfg(test)]
    pub fn spawn(program: &str, args: &[String], rows: u16, cols: u16) -> Result<Self> {
        Self::spawn_with(
            PtyConfig::new(program)
                .args(args.iter().cloned())
                .size(rows, cols),
        )
    }

    /// A pane from a config the caller has already built — a working directory,
    /// extra environment, a different scrollback. The way in.
    pub fn spawn_with(cfg: PtyConfig) -> Result<Self> {
        // The program only when nobody said otherwise. Once something in front
        // of the pty resolves names, `cfg.program` is an absolute path — and on
        // Windows, for a script routed through an interpreter, `cmd.exe` — and
        // a border reading `C:\Users\…\npm\claude.cmd` or `cmd` is worse than
        // useless in 46 columns. See [`PtyConfig::title`]. Unix needs no
        // routing, so the second half of that is a Windows fact rather than a
        // general one; the first half is true everywhere, and is the half this
        // line is mostly for.
        let title = cfg.title.clone().unwrap_or_else(|| cfg.program.clone());
        Ok(Self {
            session: PtySession::spawn(cfg)?,
            title,
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

    /// What [`poll_exit`](Self::poll_exit) last saw, without asking the OS for
    /// a fresh answer. A title is built on `&self` during a frame, and a frame
    /// is not the place to make a syscall.
    pub fn exit_status(&self) -> Option<&ExitStatus> {
        self.exited.as_ref()
    }

    pub fn has_exited(&self) -> bool {
        self.exit_status().is_some()
    }

    /// The hosted child's process id — how `crate::agentstate` finds this
    /// session in the agent's own records. See [`PtySession::process_id`] for
    /// why it can be the wrong pid, and what falls back when it is.
    pub fn process_id(&self) -> Option<u32> {
        self.session.process_id()
    }

    /// Lets the app loop be told when the child has produced something, instead
    /// of asking on a timer. See [`PtySession::wake_on_output`] — the closure
    /// runs on the reader thread and must only ring a doorbell.
    pub fn wake_on_output(&self, notify: impl Fn() + Send + Sync + 'static) {
        self.session.wake_on_output(notify);
    }

    /// Put `text` in the hosted agent's composer, without submitting it.
    ///
    /// A bracketed paste rather than a run of keystrokes, and the difference is
    /// the whole reason this exists. A multi-line prompt sent as keys submits at
    /// the first newline, and the rest is typed at whatever the agent showed
    /// next; wrapped in `ESC[200~ … ESC[201~` the same text arrives as one
    /// insertion the composer takes verbatim. `abeam_pty` picks the encoding
    /// from the mode the child actually enabled, so this quietly degrades to
    /// raw bytes for a child that never asked for bracketed paste — which is
    /// why a caller sending text nobody typed has to check
    /// [`bracketed_paste`](Self::bracketed_paste) first.
    ///
    /// It deliberately does not submit. Submitting is a separate `Enter`, sent
    /// a beat later, because the two are different decisions: this one the user
    /// can still take back with a backspace, and the next one they cannot.
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.session.send_paste(text)?;
        Ok(())
    }

    /// Whether the hosted agent asked for bracketed paste.
    ///
    /// Without it every newline in a sent block is a submit, and one queued
    /// three-line prompt becomes three prompts — the second and third typed at
    /// an agent that is by then busy with the first.
    pub fn bracketed_paste(&self) -> bool {
        self.session.screen().bracketed_paste()
    }

    // --- scrollback ------------------------------------------------------
    //
    // Two forwards rather than an accessor for the session. The session is
    // private and stays private — it holds a writer shared with its reader
    // thread and hands it to nobody — and moving the view through what has
    // scrolled off is the only thing a pane hosting one needs from it that the
    // `Pane` trait does not already cover.

    /// How far back through the rows that have scrolled off the view is, in
    /// rows. `0` is the live screen.
    pub fn scrollback(&self) -> usize {
        self.session.scrollback()
    }

    /// Move it, clamped to the history that exists. True if the view actually
    /// moved: a pane that redraws for a key that changed nothing is spending a
    /// frame, and a frame re-renders the agent's whole screen.
    pub fn set_scrollback(&self, rows: usize) -> bool {
        self.session.set_scrollback(rows)
    }

    /// The same, relative to where the view already is. Not assembled from the
    /// two above by the caller, because the reader thread moves this offset
    /// too — see [`PtySession::scroll_by`].
    pub fn scroll_by(&self, delta: isize) -> bool {
        self.session.scroll_by(delta)
    }

    /// The hosted program's last screen, as plain rows with the trailing blank
    /// ones dropped.
    ///
    /// Printed to the primary buffer once abeam has left the alternate screen.
    /// Without it, `/exit` takes the whole session with it: everything the
    /// agent drew lived on abeam's alternate screen and goes when that does,
    /// which is a thing the plain terminal abeam replaces does not do.
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

    /// Rows `first..=last` of the hosted program's screen, as text, with
    /// wrapped rows rejoined into the lines they were written as.
    ///
    /// What a selection in this pane is copied from — see `crate::select`. It
    /// is `contents_between` rather than `rows()` and that is the whole point
    /// of asking the parser instead of reading the frame back: vt100 records
    /// which rows are continuations, and only omits the newline between a row
    /// and its continuation. A 200-column `cargo` diagnostic drawn over three
    /// rows of a 46-column pane comes back as one line.
    ///
    /// Rows are the *visible* ones, so a selection made while the view is
    /// scrolled back reads the history under it rather than the live screen —
    /// `Screen::rows` and `contents_between` both walk `visible_rows`, which is
    /// what the scrollback offset moves.
    ///
    /// Clamped rather than refused: `last` is a row on somebody's screen, and a
    /// pane one row taller than its pty for the frame between a resize and the
    /// `ResizePseudoConsole` that follows it is an ordinary state, not an error.
    pub fn rows_text(&self, first: u16, last: u16) -> String {
        let screen = self.session.screen();
        let (rows, cols) = screen.size();
        let Some(bottom) = rows.checked_sub(1) else {
            return String::new();
        };
        let last = last.min(bottom);
        if first > last {
            return String::new();
        }
        screen.contents_between(first, 0, last, cols)
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
    /// **The status is a number, not a `Debug` dump, and the difference is
    /// forty columns.** `{:?}` on a `portable_pty::ExitStatus` renders
    /// `ExitStatus { code: Some(0), signal: None }` — fifty-odd cells of a
    /// struct literal, spent to say `0`, in a border that is seventy cells wide
    /// and shared with everything the pane and the session have to report.
    /// Where that fell it was the queue's countdown and the confirmation before
    /// an irreversible close, so the cost was not cosmetic.
    ///
    /// `exit_code()` and the same shape `crate::panes::shell` has always drawn
    /// for a command that has finished, so both halves of the window say a
    /// child's ending the same way.
    fn title(&self) -> String {
        match &self.exited {
            Some(status) => format!("{} · exited ({})", self.title, status.exit_code()),
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
    /// reports `Yes` — an unbound key inside the child is still the child's
    /// business.
    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        self.session.send_key(key)?;
        Ok(Handled::Yes)
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        // Coordinates are already pane-relative; abeam-pty stays out of the
        // question of where the pane is, and gates on what the child enabled.
        Ok(self.session.send_mouse(ev, ev.column, ev.row)?.into())
    }

    /// Everything typed goes into the child, `Esc` and `q` included.
    fn takes_input(&self) -> bool {
        true
    }

    /// Pane-relative `(col, row)` of the text cursor, or `None` to hide it.
    ///
    /// The strongest focus signal available: if the cursor is not blinking in
    /// the agent's prompt, your keys are not going to the agent.
    fn cursor(&self) -> Option<(u16, u16)> {
        let screen = self.session.screen();
        if screen.hide_cursor() {
            return None;
        }
        // vt100 reports (row, col); ratatui wants (x, y).
        let (row, col) = screen.cursor_position();
        Some((col, row))
    }

    /// The only call into this pty's resize in the whole program. It is a no-op
    /// when the size has not changed, which is what lets the app call it
    /// unconditionally after every frame.
    fn on_resize(&mut self, inner: Rect) -> Result<()> {
        self.session.resize(inner.height, inner.width)?;
        Ok(())
    }

    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        self.session.send_paste(text)?;
        Ok(Handled::Yes)
    }
}

/// Windows-only: both of these start a real child in a real pty. The Unix
/// module below asks the same question of `/bin/sh`, because the thing under
/// test is a border and the child is only what makes one appear.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_border_says_the_name_it_was_given_rather_than_the_path_that_was_started() {
        // The regression this exists to stop. `main` resolves what it was asked
        // for into an absolute path before the pty sees it, and a script goes
        // further still and is started by naming `cmd.exe` in front of it — so
        // the two obvious things a border could show are a path nobody typed
        // and the name of an interpreter nobody chose. In 46 columns, clipped
        // from the right, an absolute path is a border that says `C:\Users\p`.
        let windows = std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
        let resolved = windows.join("System32").join("cmd.exe");
        let cfg = PtyConfig::new(resolved.to_string_lossy())
            .args(["/c".to_string(), "exit".to_string()])
            .size(10, 40);

        let named = TerminalPane::spawn_with(cfg.clone().title("claude")).unwrap();
        assert_eq!(named.title(), "claude");

        // Unset is the old behaviour exactly, which is what lets every caller
        // that does not care go on not caring.
        let bare = TerminalPane::spawn_with(cfg).unwrap();
        assert_eq!(bare.title(), resolved.to_string_lossy());
    }
}

/// The same question on Unix. A second module rather than a `cfg` inside the
/// first, because what differs is not one line of it: the child, its arguments
/// and the path that comes back are all different, and the only thing the two
/// share is what they assert about the border.
#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;

    #[test]
    fn a_border_says_the_name_it_was_given_rather_than_the_path_that_was_started() {
        // The regression this exists to stop, and it survives the port: `main`
        // resolves what it was asked for into an absolute path before the pty
        // sees it, so the obvious thing a border could show is a path nobody
        // typed. In 46 columns, clipped from the right, that is a border which
        // says `/home/p/.local`.
        //
        // `/bin/sh` by absolute path, so a failure here is a fact about this
        // pane rather than about the runner's `PATH`.
        let cfg = PtyConfig::new("/bin/sh")
            .args(["-c".to_string(), "exit".to_string()])
            .size(10, 40);

        let named = TerminalPane::spawn_with(cfg.clone().title("claude")).unwrap();
        assert_eq!(named.title(), "claude");

        // Unset is the old behaviour exactly, which is what lets every caller
        // that does not care go on not caring.
        let bare = TerminalPane::spawn_with(cfg).unwrap();
        assert_eq!(bare.title(), "/bin/sh");
    }
}
