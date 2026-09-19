//! Compare the previous buffered writer with the production frame adapter.
//!
//! `cargo run --release -p abeam --example render_bench`
//! This measures application work and writes into a recording sink, not PTY
//! transport or terminal display latency. No agent process or account is used.

use std::cell::RefCell;
use std::io::{self, BufWriter, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::QueueableCommand;
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
use tui_term::widget::{Cursor, PseudoTerminal};

#[path = "../src/term/frame.rs"]
mod frame;

#[derive(Clone, Default)]
struct Sink(Rc<RefCell<(usize, usize)>>);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut counts = self.0.borrow_mut();
        counts.0 += bytes.len();
        counts.1 += 1;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn contents(f: &mut Frame, parser: &abeam_pty::vt100::Screen, software_cursor: bool) {
    let widget = PseudoTerminal::new(parser).cursor(Cursor::default().visibility(software_cursor));
    f.render_widget(widget, f.area());
    let (row, col) = parser.cursor_position();
    f.set_cursor_position((col, row));
}

fn main() -> io::Result<()> {
    // Use the same parser implementation through its public re-export.
    for (cols, rows) in [(120, 38), (240, 58)] {
        for full in [false, true] {
            run(cols, rows, full)?;
        }
    }
    Ok(())
}

fn run(cols: u16, rows: u16, full: bool) -> io::Result<()> {
    let area = Rect::new(0, 0, cols, rows);
    let previous_sink = Sink::default();
    let current_sink = Sink::default();
    let options = TerminalOptions {
        viewport: Viewport::Fixed(area),
    };
    let mut previous = Terminal::with_options(
        CrosstermBackend::new(BufWriter::with_capacity(64 * 1024, previous_sink.clone())),
        options.clone(),
    )?;
    let mut current =
        Terminal::with_options(frame::FrameBackend::new(current_sink.clone()), options)?;
    let mut parser = abeam_pty::vt100::Parser::new(rows, cols, 5000);
    // Filled history makes accidental O(scrollback) work visible.
    parser.process(&b"history row\r\n".repeat(5000));
    let payloads: Vec<Vec<u8>> = (*b"AB")
        .into_iter()
        .map(|ch| {
            let mut bytes = b"\x1b[H".to_vec();
            if full {
                bytes.extend(std::iter::repeat_n(
                    ch,
                    usize::from(cols) * usize::from(rows),
                ));
            } else {
                bytes.push(ch);
            }
            bytes.extend_from_slice(format!("\x1b[{rows};1H").as_bytes());
            bytes
        })
        .collect();
    let mut old_time = Duration::ZERO;
    let mut new_time = Duration::ZERO;
    let mut parts = frame::DrawStats::default();
    let iterations = 1000;
    for i in 0..iterations {
        parser.process(&payloads[i % 2]);
        let started = Instant::now();
        previous.backend_mut().queue(BeginSynchronizedUpdate)?;
        previous.draw(|f| contents(f, parser.screen(), true))?;
        previous.backend_mut().queue(EndSynchronizedUpdate)?;
        Write::flush(previous.backend_mut())?;
        old_time += started.elapsed();

        let started = Instant::now();
        let sample = frame::draw(&mut current, |f| contents(f, parser.screen(), false))?;
        new_time += started.elapsed();
        parts.ui += sample.ui;
        parts.backend += sample.backend;
        parts.output += sample.output;
        parts.bytes += sample.bytes;
        parts.writes += sample.writes;
    }
    let mean_ms = |time: Duration| time.as_secs_f64() * 1000.0 / iterations as f64;
    let old_counts = previous_sink.0.borrow();
    let new_counts = current_sink.0.borrow();
    println!(
        "{cols}x{rows} full={full}: previous={:.3}ms/{}writes current={:.3}ms/{}writes; ui={:.3}ms encode={:.3}ms sink={:.3}ms bytes/frame={} (previous={})",
        mean_ms(old_time),
        old_counts.1 / iterations,
        mean_ms(new_time),
        new_counts.1 / iterations,
        mean_ms(parts.ui),
        mean_ms(parts.backend),
        mean_ms(parts.output),
        parts.bytes / iterations,
        old_counts.0 / iterations,
    );
    assert_eq!(
        parts.writes as usize, iterations,
        "one application write per frame"
    );
    assert_eq!(new_counts.0, parts.bytes);
    Ok(())
}
