//! A complete pty host in one file: the spike's instrument, preserved.
//!
//! Left pane  = the hosted program, rendered through vt100 + tui-term.
//! Right pane = live diagnostics about what the hosted program is asking the
//!              terminal to do. That right pane is the actual instrument: if
//!              alt-screen / mouse mode / bracketed paste light up as you use
//!              Claude, the emulation layer is keeping up.
//!
//! This is not forge — forge's right pane is git and files. This is the manual
//! regression harness for the six pass criteria in `docs/conpty-findings.md`,
//! which have no automated equivalent, and it doubles as the crate's worked
//! example. If it stops compiling, the public API has lost something.
//!
//! Usage:  cargo run -p forge-pty --example host
//!         cargo run -p forge-pty --example host -- powershell
//!
//! Alt+Q detaches and exits.

use std::io::Stdout;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use forge_pty::{ExitStatus, PtyConfig, PtySession};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use tui_term::widget::PseudoTerminal;

fn split(area: Rect) -> (Rect, Rect) {
    let parts =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    (parts[0], parts[1])
}

/// The area the hosted program is drawn into: inside the border, not the pane.
///
/// One function, used for sizing the pty *and* for the mouse hit test. Two
/// calculations that have to agree is exactly where "off-by-one here is what
/// makes hosted apps wrap strangely" comes from — forge stashes the rect its
/// last frame drew for the same reason.
fn host_area(full: Rect) -> Rect {
    let (left, _) = split(full);
    Block::bordered().inner(left)
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

fn run(prog: &str, prog_args: &[String]) -> Result<Option<ExitStatus>> {
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    terminal.clear()?;

    let size = terminal.size()?;
    let mut full = Rect::new(0, 0, size.width, size.height);
    let mut host = host_area(full);

    let mut session = PtySession::spawn(
        PtyConfig::new(prog)
            .args(prog_args.iter().cloned())
            .size(host.height.max(1), host.width.max(1)),
    )?;

    let mut redraw = true;
    let exit_status = loop {
        if let Some(status) = session.try_wait()? {
            // Give the reader a moment to drain the final output.
            std::thread::sleep(Duration::from_millis(50));
            draw(&mut terminal, &session)?;
            break Some(status);
        }

        if event::poll(Duration::from_millis(10))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if key.modifiers.contains(KeyModifiers::ALT)
                        && key.code == KeyCode::Char('q')
                    {
                        break None;
                    }
                    session.send_key(key)?;
                }
                Event::Paste(text) => session.send_paste(&text)?,
                Event::Mouse(me) => {
                    let inside = me.column >= host.x
                        && me.column < host.x + host.width
                        && me.row >= host.y
                        && me.row < host.y + host.height;
                    if inside {
                        session.send_mouse(&me, me.column - host.x, me.row - host.y)?;
                    }
                }
                Event::Resize(w, h) => {
                    full = Rect::new(0, 0, w, h);
                    host = host_area(full);
                    session.resize(host.height.max(1), host.width.max(1))?;
                }
                _ => {}
            }
            redraw = true;
        }

        if session.take_dirty() {
            redraw = true;
        }
        if redraw {
            draw(&mut terminal, &session)?;
            redraw = false;
        }
    };

    Ok(exit_status)
}

// Concrete in the backend: ratatui 0.30's `Backend::Error` is an associated
// type with no Send + Sync bound, so a generic version can't use `?` here.
fn draw(terminal: &mut Terminal<CrosstermBackend<Stdout>>, session: &PtySession) -> Result<()> {
    terminal.draw(|f| ui(f, session))?;
    Ok(())
}

fn ui(f: &mut Frame, session: &PtySession) {
    let (left, right) = split(f.area());
    let screen = session.screen();
    let stats = session.stats();

    let host_block = Block::bordered()
        .title(" hosted process ")
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(PseudoTerminal::new(&*screen).block(host_block), left);

    let (srows, scols) = screen.size();
    let pty = session.pty_size().unwrap_or((0, 0));
    let flag = |on: bool| {
        if on {
            Span::styled("on ", Style::default().fg(Color::Green))
        } else {
            Span::styled("off", Style::default().fg(Color::DarkGray))
        }
    };
    let label = |s: &'static str| Span::styled(s, Style::default().fg(Color::Gray));
    let heading = |s: &'static str| {
        Line::from(Span::styled(
            s,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let mut lines = vec![
        heading("emulation state"),
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
        heading("plumbing"),
        Line::from(""),
        Line::from(format!("pty size        {}x{}", pty.1, pty.0)),
        Line::from(format!("parser size     {scols}x{srows}")),
        Line::from(format!("bytes read      {}", stats.bytes_read)),
        Line::from(vec![label("DSR answered    "), {
            let n = stats.dsr_replies;
            Span::styled(
                n.to_string(),
                Style::default().fg(if n > 0 { Color::Green } else { Color::Red }),
            )
        }]),
        Line::from(format!("keys sent       {}", stats.keys_sent)),
        Line::from(format!("resizes         {}", stats.resizes)),
        Line::from(format!("cursor          {:?}", screen.cursor_position())),
    ];

    if stats.reader_finished {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "pty closed",
            Style::default().fg(Color::Red),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Alt+Q to detach",
        Style::default().fg(Color::Magenta),
    )));

    let block = Block::bordered()
        .title(" pty diagnostics ")
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        right,
    );
}
