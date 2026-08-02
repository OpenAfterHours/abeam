//! A child process hosted in a pty, its output parsed into a `vt100` screen.
//!
//! The awkward parts of this file are not accidents, and they are ConPTY's.
//! ConPTY opens a session by asking the host where the cursor is and blocks
//! until it is answered, which forces the writer to be shared with the reader
//! thread; and there is no reliable EOF on its master, which forces `try_wait`
//! polling and a reader thread that is never joined. See
//! `docs/conpty-findings.md`.
//!
//! A Unix pty does neither of those things, and none of it is written twice.
//! Answering a query nobody asked costs nothing, and polling a child that could
//! have been waited on costs a poll — one shape that is right on both platforms
//! is worth more here than a `cfg` for each.

use std::io::{self, Read, Write};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crossterm::event::{KeyEvent, MouseEvent};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};

use crate::input;
use crate::tree::Tree;

/// Errors are an enum rather than `anyhow::Error` so a caller can render "the
/// agent is not installed" differently from an I/O failure. `anyhow` cannot be
/// eliminated — portable-pty's own API returns it — so it appears only as a
/// `#[source]`.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("failed to open a pty")]
    Open(#[source] anyhow::Error),
    /// `name` is [`PtyConfig::title`] when the caller supplied one and
    /// [`PtyConfig::program`] otherwise, and it is named for the job rather
    /// than for where it comes from. An npm shim is spawned as
    /// `C:\Windows\System32\cmd.exe`, so reporting the program would say
    /// "failed to spawn `C:\Windows\System32\cmd.exe`" about a file nobody
    /// asked for and about an installation that is not the one at fault —
    /// which is the exact confusion `title` was added to prevent, in the enum
    /// whose whole reason for existing is that a caller can tell "not
    /// installed" from "the pty broke".
    #[error("failed to spawn `{name}`")]
    Spawn {
        name: String,
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
    /// What a host should *call* this session, when that is not the program it
    /// starts. `None` means the program, which is right until something in
    /// front of the pty resolves names: `claude` reaching here as
    /// `C:\Users\…\AppData\Roaming\npm\claude.cmd` is the same session, and an
    /// npm shim reaching here as `cmd.exe` is not even the same word.
    ///
    /// Deliberately not derived from `program` — a host that wanted the file
    /// name could take it, and the cases where the two differ are exactly the
    /// cases where only the caller knows what was meant.
    ///
    /// A carrier, and the only field here that is one. Every other field is
    /// consumed by [`PtySession::spawn`] and changes what the child is or how
    /// it starts; this one changes nothing about the session. The crate stores
    /// it, reads it in exactly one place — [`PtyError::Spawn`], which has to
    /// name something the caller recognises — and otherwise hands it straight
    /// back for the caller to put on a border.
    ///
    /// Which means `abeam-pty` has no test for it: there is no behaviour here
    /// to assert, so what coverage exists is in the host that renders it.
    pub title: Option<String>,
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
            title: None,
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

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
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
    /// Rung by the reader thread every time `dirty` goes up. See
    /// [`PtySession::wake_on_output`] for why a session that is only polled is
    /// a session that renders late.
    ///
    /// A `OnceLock` rather than a `Mutex<Option<_>>` so the reader reads it
    /// without taking a lock on the hot path, and so there is no window in
    /// which a caller swaps the waker out from under a ring in flight.
    waker: OnceLock<Box<dyn Fn() + Send + Sync>>,
    bytes: AtomicU64,
    eof: AtomicBool,
    dsr_replies: AtomicU64,
    keys_sent: AtomicU64,
    resizes: AtomicU64,
}

impl Shared {
    /// Marks the screen changed and tells whoever is waiting. Always in that
    /// order: a waker that fires before the flag is set can be answered by a
    /// consumer that then sees nothing to do and goes back to sleep.
    ///
    /// Only the reader thread calls this. The other two places that dirty the
    /// screen — a resize and a scrollback move — run on the caller's own thread,
    /// which is by definition already awake, and one of them holds the parser
    /// lock while it does it. Ringing from under that lock would run somebody
    /// else's closure inside our critical section, which is a deadlock waiting
    /// for its first careless waker.
    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Relaxed);
        if let Some(wake) = self.waker.get() {
            wake();
        }
    }
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
/// `wait()` (see [`try_wait`](Self::try_wait)), and there is no `kill()` — a
/// dropped session kills its child, and a second way to do it is a second way
/// to do it while something else is still reading.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: SharedWriter,
    shared: Arc<Shared>,
    /// Holds the child *and its descendants*, so that dropping this session
    /// does not leave the `cargo build` it started running. `None` if the
    /// platform would not give us one, which leaves the session working exactly
    /// as it did before [`crate::tree`] existed. A job object on Windows and a
    /// process group on Unix; the same three lines of session either way.
    tree: Option<Tree>,
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
                // The title if there is one: what failed to start, from the
                // caller's side, is `claude` — `cmd.exe` is how abeam was
                // spelling it.
                name: cfg.title.clone().unwrap_or_else(|| cfg.program.clone()),
                source,
            })?;
        // Closes the write end on unix, where that is what lets the reader see
        // EOF. On Windows it drops a refcount and does nothing else —
        // `ConPtySlavePty` and `ConPtyMasterPty` share one `Arc<Mutex<Inner>>`,
        // and EOF arrives from `ClosePseudoConsole` when the *master* goes.
        // Correct on both; do not read it as the reason the reader thread ever
        // finishes on the platform these tests actually run on.
        drop(pair.slave);

        // Before anything can be written to the child, and before any error
        // path can drop it: on Windows the sooner this happens the smaller the
        // window in which a grandchild is born outside the job. Unix has no
        // such window — the child made its own group before it exec'd — and the
        // call is still made here, because one order that is right on both
        // beats two that are each right on one.
        let tree = Tree::holding(&*child);

        let master = pair.master;
        let shared = Arc::new(Shared {
            parser: Mutex::new(vt100::Parser::new(rows, cols, cfg.scrollback)),
            dirty: AtomicBool::new(true),
            waker: OnceLock::new(),
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
            tree,
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

    /// Called on the reader thread whenever output arrives, so a draw loop can
    /// wait to be told rather than asking on a timer.
    ///
    /// This exists because polling sets a floor on latency that has nothing to
    /// do with how fast anything actually is: a loop that asks every 10 ms
    /// renders a keystroke 5 ms late on average however quick the renderer is,
    /// and — worse for the look of it — quantises the agent's output onto a
    /// grid it has no relationship with, so frames land unevenly. Uneven frames
    /// read as jitter at any rate.
    ///
    /// `notify` runs **on the reader thread, holding nothing**, so it must not
    /// block and must not touch the session. Ring a doorbell and return; the
    /// news itself is [`take_dirty`](Self::take_dirty), which is sticky, so a
    /// ring that is dropped costs nothing.
    ///
    /// The first caller wins and later ones are ignored — there is one draw
    /// loop, it installs this before the first frame, and a waker that could be
    /// replaced mid-session is a waker that can be replaced mid-ring.
    pub fn wake_on_output(&self, notify: impl Fn() + Send + Sync + 'static) {
        let _ = self.shared.waker.set(Box::new(notify));
    }

    // --- scrollback ------------------------------------------------------

    /// How far back through the rows that have scrolled off the view is, in
    /// rows. `0` is the live screen.
    pub fn scrollback(&self) -> usize {
        self.with_screen(vt100::Screen::scrollback)
    }

    /// Move that view to an absolute distance from the live screen, clamped to
    /// the history that actually exists — [`PtyConfig::scrollback`] is a
    /// ceiling, not a promise, and a fresh session has nothing behind it.
    pub fn set_scrollback(&self, rows: usize) -> bool {
        self.move_view(|_| rows)
    }

    /// Move it *relatively*: positive is backwards, into the history.
    ///
    /// The relative form exists because a caller cannot safely assemble it out
    /// of the other two. Between reading [`scrollback`](Self::scrollback) and
    /// calling [`set_scrollback`](Self::set_scrollback) the reader thread can
    /// take this lock, push rows, and advance the same offset itself — it does
    /// that on purpose, so that someone who has scrolled back stays where they
    /// are looking while output flows. A `PgUp` timed into that window would
    /// compute its destination from a base that had already moved.
    pub fn scroll_by(&self, delta: isize) -> bool {
        self.move_view(|at| {
            if delta < 0 {
                at.saturating_sub(delta.unsigned_abs())
            } else {
                at.saturating_add(delta as usize)
            }
        })
    }

    /// The one place the offset is written, so that every move is a
    /// read-modify-write under a single lock.
    ///
    /// Reports whether the view moved rather than leaving the caller to ask
    /// again, for the same reason: the answer is only true at the instant the
    /// lock was held. The caller is a pane deciding whether to spend a frame,
    /// and a frame re-renders the agent's whole screen.
    fn move_view(&self, to: impl FnOnce(usize) -> usize) -> bool {
        let mut parser = self.shared.parser.lock().unwrap();
        let screen = parser.screen_mut();
        let before = screen.scrollback();
        screen.set_scrollback(to(before));
        let moved = screen.scrollback() != before;
        if moved {
            self.shared.dirty.store(true, Ordering::Relaxed);
        }
        moved
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

    /// Non-blocking. There is deliberately no `wait()`, and the rule holds on
    /// both platforms out of two different strengths of reason: under ConPTY
    /// `wait()` never returns at all, and on a Unix pty it returns perfectly
    /// well — having stopped the caller's draw loop until the agent exits. The
    /// Windows failure is the one that taught it; the Unix one is the one that
    /// would get called a design decision.
    ///
    /// Output may still be in flight when this yields `Some`, so give the
    /// reader a moment before drawing the final frame.
    pub fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>, PtyError> {
        Ok(self.child.try_wait()?)
    }

    /// The child's process id, when the platform will say.
    ///
    /// The one thing a host can use to find the child in something *else's*
    /// records — which is what `abeam::agentstate` does, to read the session
    /// file Claude keeps per pid. Deliberately not a handle: this is for
    /// identifying the process, never for signalling it. Killing is `Drop`'s,
    /// and there is exactly one way to do it.
    ///
    /// Note for callers matching on it: this is the process abeam *started*,
    /// which for a script routed through an interpreter is the interpreter.
    /// See `abeam::launch`.
    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
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
    /// A dropped session must not leave the agent's process running — nor,
    /// since the command view landed, the `cargo build` a hosted shell started.
    ///
    /// The order is load-bearing on both platforms, and not for the same
    /// reason on either.
    ///
    /// The child is killed first. On Windows that is `TerminateProcess` and it
    /// reaches one process; on Unix portable-pty sends `SIGHUP` first and only
    /// escalates if the child is still there, which is worth having in that
    /// order — `SIGHUP` is what an interactive shell answers by hanging up its
    /// own jobs, and those live in process groups `crate::tree` cannot reach.
    /// It also means this line can spend up to 200 ms inside portable-pty
    /// waiting for a live child to take the hint. That is the only blocking
    /// thing in this function, it is not ours to remove, and it is why
    /// `crate::tree` does not put a grace period of its own on top of it.
    ///
    /// One thing that first step is *not* on Unix, and the port turned it into
    /// this quietly: over there `Child::kill` is `libc::kill(pid, SIGHUP)` on a
    /// bare pid — portable-pty 0.9.0, `src/lib.rs`, `impl ChildKiller for
    /// std::process::Child` — sent without asking whether this process has
    /// already reaped that pid, and [`try_wait`](Self::try_wait) is called every
    /// frame by the host, so by the time this runs it usually has. On Windows
    /// the same call is `TerminateProcess` on an `OwnedHandle`, and a handle
    /// names one process for as long as it is open: it cannot come to mean
    /// somebody else's. So this line carries exactly the pid-reuse exposure
    /// `crate::tree`'s Unix half documents at length, on the same number and in
    /// the same window, and it is no more fixable from here than that one is —
    /// the fix for both is a `pidfd`, taken at spawn time, in portable-pty.
    ///
    /// Then the tree closes, taking with it every descendant that one signal or
    /// one `TerminateProcess` does not: the job object's members on Windows, the
    /// child's process group on Unix.
    ///
    /// Only then does `master` drop. `ClosePseudoConsole` can block while
    /// clients are still attached, so by then there must be none; and closing
    /// the master is what triggers the kernel's own hangup on Unix, which is to
    /// say it is what starts emptying the group the line above wants full. Both
    /// of the first two steps therefore happen *here*, in the body, rather than
    /// by field order — `master` is declared first and would otherwise go first.
    ///
    /// The reader thread is deliberately *not* joined. It has no reliable EOF
    /// to return from and joining it hangs the process — let it die with us.
    fn drop(&mut self) {
        let _ = self.child.kill();
        drop(self.tree.take());
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
                    shared.mark_dirty();
                }
            }
        }
        shared.eof.store(true, Ordering::Relaxed);
        // The last ring, and the one that matters most: the loop has to wake to
        // notice the child has gone rather than finding out on its next tick.
        shared.mark_dirty();
    });
}
