//! Spike: can we host Claude Code (or any full-screen TUI) inside a pane we
//! draw ourselves, on Windows ConPTY, and have it look and behave correctly?
//!
//! Left pane  = the hosted program, rendered through vt100 + tui-term.
//! Right pane = live diagnostics about what the hosted program is asking the
//!              terminal to do. That right pane is the actual instrument: if
//!              alt-screen / mouse mode / bracketed paste light up as you use
//!              Claude, the emulation layer is keeping up.
//!
//! Usage:  cargo run                 (hosts `claude`)
//!         cargo run -- powershell   (hosts something else)
//!
//! Ctrl+] detaches and exits.

mod input;

use std::io::{Read, Stdout, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use portable_pty::{CommandBuilder, PtySize};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use tui_term::vt100;
use tui_term::widget::PseudoTerminal;

/// Rough stand-in for the real layout: hosted app left, "the pane that would be
/// git / files" right.
fn split(area: Rect) -> (Rect, Rect) {
    let parts =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    (parts[0], parts[1])
}

/// The pty must be sized to the *inner* area of the bordered block, not the
/// pane. Off-by-one here is what makes hosted apps wrap strangely.
fn pty_dims(full: Rect) -> (u16, u16) {
    let (left, _) = split(full);
    let inner = Block::bordered().inner(left);
    (inner.height.max(1), inner.width.max(1))
}

struct Shared {
    parser: Mutex<vt100::Parser>,
    dirty: AtomicBool,
    bytes: AtomicU64,
    eof: AtomicBool,
    /// Should be >= 1 almost immediately. Zero means we are about to hang.
    dsr_replies: AtomicU64,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (prog, prog_args) = match args.split_first() {
        Some((p, rest)) => (p.clone(), rest.to_vec()),
        None => ("claude".to_string(), Vec::new()),
    };

    // A panic inside raw mode leaves the terminal unusable otherwise.
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

    let result = run(&prog, &prog_args);
    restore()?;

    match result {
        Ok(Some(status)) => println!("hosted process exited: {status:?}"),
        Ok(None) => println!("detached"),
        Err(e) => return Err(e),
    }
    Ok(())
}

fn restore() -> Result<()> {
    execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    disable_raw_mode()?;
    Ok(())
}

fn run(prog: &str, prog_args: &[String]) -> Result<Option<portable_pty::ExitStatus>> {
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    terminal.clear()?;

    let size = terminal.size()?;
    let mut full = Rect::new(0, 0, size.width, size.height);
    let (mut rows, mut cols) = pty_dims(full);

    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new(prog);
    for a in prog_args {
        cmd.arg(a);
    }
    cmd.cwd(std::env::current_dir()?);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("failed to spawn `{prog}`"))?;
    // Dropping the slave handle is what lets the reader see EOF when the child
    // exits. Without this the read thread hangs forever on Windows.
    drop(pair.slave);

    let master = pair.master;
    let shared = Arc::new(Shared {
        parser: Mutex::new(vt100::Parser::new(rows, cols, 5000)),
        dirty: AtomicBool::new(true),
        bytes: AtomicU64::new(0),
        eof: AtomicBool::new(false),
        dsr_replies: AtomicU64::new(0),
    });

    // One writer, shared: the key loop uses it, and so does the reader thread
    // when it has to answer a DSR query.
    let writer = Arc::new(Mutex::new(master.take_writer()?));

    let mut reader = master.try_clone_reader()?;
    {
        let shared = Arc::clone(&shared);
        let writer = Arc::clone(&writer);
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

                        // Must be answered or the session stalls before the
                        // hosted program produces anything.
                        if queries > 0 {
                            let reply = input::dsr_reply(cursor.0, cursor.1);
                            let mut w = writer.lock().unwrap();
                            for _ in 0..queries {
                                let _ = w.write_all(&reply);
                            }
                            let _ = w.flush();
                            shared.dsr_replies.fetch_add(queries as u64, Ordering::Relaxed);
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

    let send = |bytes: &[u8]| -> Result<()> {
        let mut w = writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    };
    let mut keys_sent: u64 = 0;
    let mut resizes: u64 = 0;
    let mut redraw = true;

    let exit_status = loop {
        if let Some(status) = child.try_wait()? {
            // Give the reader a moment to drain the final output.
            std::thread::sleep(Duration::from_millis(50));
            draw(&mut terminal, &shared, keys_sent, resizes, (rows, cols))?;
            break Some(status);
        }

        if event::poll(Duration::from_millis(10))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Release
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char(']')
                    {
                        break None;
                    }
                    let app_cursor = shared.parser.lock().unwrap().screen().application_cursor();
                    if let Some(bytes) = input::encode_key(key, app_cursor) {
                        send(&bytes)?;
                        keys_sent += 1;
                    }
                }
                Event::Paste(text) => {
                    let bracketed = shared.parser.lock().unwrap().screen().bracketed_paste();
                    send(&input::encode_paste(&text, bracketed))?;
                }
                Event::Mouse(me) => {
                    let (left, _) = split(full);
                    let inner = Block::bordered().inner(left);
                    let inside = me.column >= inner.x
                        && me.column < inner.x + inner.width
                        && me.row >= inner.y
                        && me.row < inner.y + inner.height;
                    if inside {
                        let (mode, encoding) = {
                            let p = shared.parser.lock().unwrap();
                            let s = p.screen();
                            (s.mouse_protocol_mode(), s.mouse_protocol_encoding())
                        };
                        if let Some(bytes) = input::encode_mouse(
                            &me,
                            me.column - inner.x,
                            me.row - inner.y,
                            mode,
                            encoding,
                        ) {
                            send(&bytes)?;
                        }
                    }
                }
                Event::Resize(w, h) => {
                    full = Rect::new(0, 0, w, h);
                    let (r, c) = pty_dims(full);
                    if (r, c) != (rows, cols) {
                        rows = r;
                        cols = c;
                        master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        })?;
                        shared
                            .parser
                            .lock()
                            .unwrap()
                            .screen_mut()
                            .set_size(rows, cols);
                        resizes += 1;
                    }
                }
                _ => {}
            }
            redraw = true;
        }

        if shared.dirty.swap(false, Ordering::Relaxed) {
            redraw = true;
        }
        if redraw {
            draw(&mut terminal, &shared, keys_sent, resizes, (rows, cols))?;
            redraw = false;
        }
    };

    if exit_status.is_none() {
        let _ = child.kill();
    }
    Ok(exit_status)
}

// Concrete in the backend: ratatui 0.30's `Backend::Error` is an associated
// type with no Send + Sync bound, so a generic version can't use `?` here.
fn draw(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    shared: &Shared,
    keys_sent: u64,
    resizes: u64,
    pty: (u16, u16),
) -> Result<()> {
    terminal.draw(|f| ui(f, shared, keys_sent, resizes, pty))?;
    Ok(())
}

fn ui(f: &mut Frame, shared: &Shared, keys_sent: u64, resizes: u64, pty: (u16, u16)) {
    let (left, right) = split(f.area());
    let parser = shared.parser.lock().unwrap();
    let screen = parser.screen();

    let host_block = Block::bordered()
        .title(" hosted process ")
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(PseudoTerminal::new(screen).block(host_block), left);

    let (srows, scols) = screen.size();
    let flag = |on: bool| {
        if on {
            Span::styled("on ", Style::default().fg(Color::Green))
        } else {
            Span::styled("off", Style::default().fg(Color::DarkGray))
        }
    };
    let label = |s: &'static str| Span::styled(s, Style::default().fg(Color::Gray));

    let mut lines = vec![
        Line::from(Span::styled(
            "emulation state",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            label("alt screen      "),
            flag(screen.alternate_screen()),
        ]),
        Line::from(vec![
            label("app cursor      "),
            flag(screen.application_cursor()),
        ]),
        Line::from(vec![
            label("app keypad      "),
            flag(screen.application_keypad()),
        ]),
        Line::from(vec![
            label("bracketed paste "),
            flag(screen.bracketed_paste()),
        ]),
        Line::from(vec![
            label("mouse mode      "),
            Span::styled(
                format!("{:?}", screen.mouse_protocol_mode()),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            label("mouse encoding  "),
            Span::styled(
                format!("{:?}", screen.mouse_protocol_encoding()),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "plumbing",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("pty size        {}x{}", pty.1, pty.0)),
        Line::from(format!("parser size     {scols}x{srows}")),
        Line::from(format!(
            "bytes read      {}",
            shared.bytes.load(Ordering::Relaxed)
        )),
        Line::from(vec![
            label("DSR answered    "),
            {
                let n = shared.dsr_replies.load(Ordering::Relaxed);
                Span::styled(
                    n.to_string(),
                    Style::default().fg(if n > 0 { Color::Green } else { Color::Red }),
                )
            },
        ]),
        Line::from(format!("keys sent       {keys_sent}")),
        Line::from(format!("resizes         {resizes}")),
        Line::from(format!("cursor          {:?}", screen.cursor_position())),
    ];

    if shared.eof.load(Ordering::Relaxed) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "pty closed",
            Style::default().fg(Color::Red),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Ctrl+] to detach",
        Style::default().fg(Color::Magenta),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "This pane is where the git / file viewer would live. \
         Resize the window and watch the hosted app reflow.",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::bordered()
        .title(" spike diagnostics ")
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        right,
    );
}
