//! A child process hosted in a pty, its output parsed into a `vt100` screen.
//!
//! The awkward parts of this file are not accidents. ConPTY opens a session by
//! asking the host where the cursor is and blocks until it is answered, which
//! forces the writer to be shared with the reader thread; and there is no
//! reliable EOF on the master, which forces `try_wait` polling and a reader
//! thread that is never joined. See `docs/conpty-findings.md`.

use std::io::{self, Read, Write};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crossterm::event::{KeyEvent, MouseEvent};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};

use crate::input;

/// Errors are an enum rather than `anyhow::Error` so a caller can render
/// "claude is not installed" differently from an I/O failure. `anyhow` cannot
/// be eliminated — portable-pty's own API returns it — so it appears only as a
/// `#[source]`.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("failed to open a pty")]
    Open(#[source] anyhow::Error),
    #[error("failed to spawn `{program}`")]
    Spawn {
        program: String,
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to resize the pty")]
    Resize(#[source] anyhow::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// What to spawn, and how big its terminal is.
///
/// Plain fields *and* builder methods, deliberately. The fields are the whole
/// of the type — there is no invariant a setter could protect — so the builder
/// is sugar for the common case rather than an encapsulation boundary, and
/// [`PtySession::spawn`] clamps the size on the way in because a struct literal
/// can always sidestep [`size`](Self::size).
#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub program: String,
    pub args: Vec<String>,
    /// `None` means the current working directory at spawn time.
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
}

impl PtyConfig {
    /// Seeded with the environment a modern TUI expects. Omitting `TERM` gets
    /// you a hosted app that renders in monochrome and blames you for it.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
            ],
            rows: 24,
            cols: 80,
            scrollback: 5000,
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn size(mut self, rows: u16, cols: u16) -> Self {
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        let k = k.into();
        self.env.retain(|(existing, _)| *existing != k);
        self.env.push((k, v.into()));
        self
    }

    pub fn scrollback(mut self, lines: usize) -> Self {
        self.scrollback = lines;
        self
    }
}

/// Counters that make the ConPTY failure mode visible instead of mysterious.
#[derive(Clone, Copy, Debug, Default)]
pub struct PtyStats {
    pub bytes_read: u64,
    /// Should be >= 1 within the first moment of a session. **Zero means an
    /// imminent hang** — the host is not answering the DSR query.
    pub dsr_replies: u64,
    pub keys_sent: u64,
    pub resizes: u64,
    /// The reader loop ended. The pty is closed; nothing more will arrive.
    pub reader_finished: bool,
}

struct Shared {
    parser: Mutex<vt100::Parser>,
    dirty: AtomicBool,
    bytes: AtomicU64,
    eof: AtomicBool,
    dsr_replies: AtomicU64,
    keys_sent: AtomicU64,
    resizes: AtomicU64,
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Borrowed view of the parsed screen. Holds the parser lock, so the reader
/// thread is blocked for as long as it lives — scope it to a single draw.
pub struct ScreenGuard<'a>(MutexGuard<'a, vt100::Parser>);

impl Deref for ScreenGuard<'_> {
    type Target = vt100::Screen;
    fn deref(&self) -> &vt100::Screen {
        self.0.screen()
    }
}

/// A live pty session.
///
/// Everything that could be misused is private: there is no accessor handing
/// out the writer (the reader thread needs it to answer DSR), there is no
/// `wait()` (under ConPTY it never returns), and there is no `kill()` — a
/// dropped session kills its child, and a second way to do it is a second way
/// to do it while something else is still reading.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: SharedWriter,
    shared: Arc<Shared>,
}

impl PtySession {
    /// Spawns the child and starts the reader thread. The reader answers
    /// ConPTY's startup query, so the session is usable the moment this
    /// returns — the caller does not have to know the handshake exists.
    pub fn spawn(cfg: PtyConfig) -> Result<Self, PtyError> {
        let (rows, cols) = (cfg.rows.max(1), cfg.cols.max(1));

        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(pty_size(rows, cols))
            .map_err(PtyError::Open)?;

        let mut cmd = CommandBuilder::new(&cfg.program);
        for a in &cfg.args {
            cmd.arg(a);
        }
        match &cfg.cwd {
            Some(dir) => cmd.cwd(dir),
            None => cmd.cwd(std::env::current_dir()?),
        }
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|source| PtyError::Spawn {
                program: cfg.program.clone(),
                source,
            })?;
        // Dropping the slave handle is what lets the reader see EOF when the
        // child exits. Without this the read thread hangs forever on Windows.
        drop(pair.slave);

        let master = pair.master;
        let shared = Arc::new(Shared {
            parser: Mutex::new(vt100::Parser::new(rows, cols, cfg.scrollback)),
            dirty: AtomicBool::new(true),
            bytes: AtomicU64::new(0),
            eof: AtomicBool::new(false),
            dsr_replies: AtomicU64::new(0),
            keys_sent: AtomicU64::new(0),
            resizes: AtomicU64::new(0),
        });

        // One writer, shared: callers send input through it, and so does the
        // reader thread when it has to answer a DSR query.
        let writer: SharedWriter = Arc::new(Mutex::new(master.take_writer().map_err(PtyError::Open)?));
        let reader = master.try_clone_reader().map_err(PtyError::Open)?;

        spawn_reader(reader, Arc::clone(&shared), Arc::clone(&writer));

        Ok(Self {
            master,
            child,
            writer,
            shared,
        })
    }

    // --- rendering -------------------------------------------------------

    /// The parsed screen, ready to hand to a widget.
    ///
    /// Note when reading it: `Screen::contents()` rejoins wrapped rows into
    /// logical lines and so tells you nothing about layout. Use
    /// `Screen::rows()` for anything positional.
    pub fn screen(&self) -> ScreenGuard<'_> {
        ScreenGuard(self.shared.parser.lock().unwrap())
    }

    /// Same access, without the guard living across your frame.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(self.shared.parser.lock().unwrap().screen())
    }

    /// True, once, if output has arrived since the last call. Drives redraws
    /// without polling the screen contents.
    pub fn take_dirty(&self) -> bool {
        self.shared.dirty.swap(false, Ordering::Relaxed)
    }

    // --- input -----------------------------------------------------------

    /// `Ok(false)` means the key has no byte representation — a bare modifier,
    /// or a Release event, which Windows sends for every keystroke and which
    /// would double-type everything if forwarded.
    pub fn send_key(&self, key: KeyEvent) -> io::Result<bool> {
        let app_cursor = self.with_screen(|s| s.application_cursor());
        match input::encode_key(key, app_cursor) {
            Some(bytes) => {
                self.write(&bytes)?;
                self.shared.keys_sent.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn send_paste(&self, text: &str) -> io::Result<()> {
        let bracketed = self.with_screen(|s| s.bracketed_paste());
        self.write(&input::encode_paste(text, bracketed))
    }

    /// `col`/`row` are 0-based and pane-relative; the hit test belongs to the
    /// caller, which is the only party that knows where the pane is.
    ///
    /// `Ok(false)` means the hosted app has not asked for this class of event.
    /// Sending it anyway dumps escape sequences into its prompt.
    pub fn send_mouse(&self, ev: &MouseEvent, col: u16, row: u16) -> io::Result<bool> {
        let (mode, encoding) = {
            let p = self.shared.parser.lock().unwrap();
            let s = p.screen();
            (s.mouse_protocol_mode(), s.mouse_protocol_encoding())
        };
        match input::encode_mouse(ev, col, row, mode, encoding) {
            Some(bytes) => {
                self.write(&bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Escape hatch for bytes this crate has no opinion about.
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()
    }

    // --- lifecycle -------------------------------------------------------

    /// Resizes the pty and the parser together, under one lock. They must
    /// never disagree; a caller that could resize one without the other would
    /// eventually do exactly that.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), PtyError> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        let mut parser = self.shared.parser.lock().unwrap();
        if parser.screen().size() == (rows, cols) {
            return Ok(());
        }
        self.master
            .resize(pty_size(rows, cols))
            .map_err(PtyError::Resize)?;
        parser.screen_mut().set_size(rows, cols);
        self.shared.resizes.fetch_add(1, Ordering::Relaxed);
        self.shared.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// `(rows, cols)`, read off the parser — which is the same number the pty
    /// has, because [`resize`](Self::resize) is the only way to change either.
    pub fn size(&self) -> (u16, u16) {
        self.with_screen(|s| s.size())
    }

    /// `(rows, cols)` as the pty reports them.
    ///
    /// Worth knowing before trusting it: on Windows this is *not* a question
    /// put to ConPTY. portable-pty answers from a field it wrote itself during
    /// the last successful resize, and [`resize`](Self::resize) updates the
    /// parser from that same call — so this and [`size`](Self::size) agree by
    /// construction, and their agreeing is not evidence of anything. It is
    /// still the number to compare against the rect you drew.
    pub fn pty_size(&self) -> Result<(u16, u16), PtyError> {
        let s = self.master.get_size().map_err(PtyError::Resize)?;
        Ok((s.rows, s.cols))
    }

    /// Non-blocking. There is deliberately no `wait()`: under ConPTY it never
    /// returns. Output may still be in flight when this yields `Some`, so give
    /// the reader a moment before drawing the final frame.
    pub fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>, PtyError> {
        Ok(self.child.try_wait()?)
    }

    pub fn stats(&self) -> PtyStats {
        PtyStats {
            bytes_read: self.shared.bytes.load(Ordering::Relaxed),
            dsr_replies: self.shared.dsr_replies.load(Ordering::Relaxed),
            keys_sent: self.shared.keys_sent.load(Ordering::Relaxed),
            resizes: self.shared.resizes.load(Ordering::Relaxed),
            reader_finished: self.shared.eof.load(Ordering::Relaxed),
        }
    }
}

impl Drop for PtySession {
    /// A dropped session must not leave a Claude process running.
    ///
    /// The reader thread is deliberately *not* joined. It has no reliable EOF
    /// to return from and joining it hangs the process — let it die with us.
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// The reader loop, transcribed from the spike that proved it works.
///
/// The order here matters: bytes are scanned for DSR queries, then fed to the
/// parser, and only then is the cursor read — the reply has to report where the
/// cursor is *after* this chunk, not before it.
fn spawn_reader(mut reader: Box<dyn Read + Send>, shared: Arc<Shared>, writer: SharedWriter) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut dsr = input::DsrScanner::default();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    let queries = dsr.scan(chunk);

                    let cursor = {
                        let mut p = shared.parser.lock().unwrap();
                        p.process(chunk);
                        p.screen().cursor_position()
                    };

                    // Must be answered or the session stalls before the hosted
                    // program produces anything.
                    if queries > 0 {
                        let reply = input::dsr_reply(cursor.0, cursor.1);
                        let mut w = writer.lock().unwrap();
                        for _ in 0..queries {
                            let _ = w.write_all(&reply);
                        }
                        let _ = w.flush();
                        shared
                            .dsr_replies
                            .fetch_add(queries as u64, Ordering::Relaxed);
                    }

                    shared.bytes.fetch_add(n as u64, Ordering::Relaxed);
                    shared.dirty.store(true, Ordering::Relaxed);
                }
            }
        }
        shared.eof.store(true, Ordering::Relaxed);
        shared.dirty.store(true, Ordering::Relaxed);
    });
}
