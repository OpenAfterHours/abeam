//! Stage a whole host frame, including the final native cursor position.
//!
//! Crossterm's cursor operations flush their writer. A BufWriter therefore
//! exposes several partial frames, and Ratatui shows the cursor before moving
//! it to its final position. This adapter delays those flushes and that show
//! until the entire frame has been assembled.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::QueueableCommand;
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::{Frame, Terminal};

const FRAME_CAPACITY: usize = 64 * 1024;
const RECOVER: &[u8] = b"\x1b[?2026l\x1b[?25h";

#[derive(Clone, Copy, Debug, Default)]
pub struct DrawStats {
    pub ui: Duration,
    pub backend: Duration,
    pub output: Duration,
    pub bytes: usize,
    pub writes: u64,
}

/// Ordinary writes pass through. During a frame, only `commit` can release
/// bytes; even frames larger than the initial capacity remain together.
struct FrameWriter<W: Write> {
    output: W,
    staged: Vec<u8>,
    active: bool,
}

impl<W: Write> FrameWriter<W> {
    fn new(output: W) -> Self {
        Self {
            output,
            staged: Vec::with_capacity(FRAME_CAPACITY),
            active: false,
        }
    }

    fn begin(&mut self) {
        assert!(!self.active, "nested host frame");
        self.staged.clear();
        self.active = true;
    }

    fn commit(&mut self) -> io::Result<(usize, u64)> {
        let bytes = self.staged.len();
        let mut written = 0;
        let mut writes = 0;
        while written < bytes {
            writes += 1;
            match self.output.write(&self.staged[written..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => written += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        self.output.flush()?;
        self.staged.clear();
        self.active = false;
        Ok((bytes, writes))
    }

    fn abort(&mut self) {
        // Never replay a partial or failed frame on a later flush or Drop.
        self.staged.clear();
        self.active = false;
        let _ = self.output.write_all(RECOVER);
        let _ = self.output.flush();
    }
}

impl<W: Write> Write for FrameWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.active {
            self.staged.extend_from_slice(bytes);
            Ok(bytes.len())
        } else {
            self.output.write(bytes)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.active {
            Ok(())
        } else {
            self.output.flush()
        }
    }
}

/// Retains Ratatui's cursor bookkeeping and frame API while postponing the
/// native show command until after its last cursor-position command.
pub struct FrameBackend<W: Write> {
    writer: FrameWriter<W>,
    show_at_commit: bool,
}

impl<W: Write> FrameBackend<W> {
    pub fn new(output: W) -> Self {
        Self {
            writer: FrameWriter::new(output),
            show_at_commit: false,
        }
    }

    // CrosstermBackend stores only its writer. Borrowing ours for each
    // operation preserves all of its rendering while keeping frame commits
    // available through stable APIs (its writer_mut accessor is unstable).
    fn crossterm(&mut self) -> CrosstermBackend<&mut FrameWriter<W>> {
        CrosstermBackend::new(&mut self.writer)
    }
}

impl<W: Write> Backend for FrameBackend<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.crossterm().draw(content)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.show_at_commit = false;
        self.crossterm().hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        if self.writer.active {
            self.show_at_commit = true;
            Ok(())
        } else {
            self.crossterm().show_cursor()
        }
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.crossterm().get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.crossterm().set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.crossterm().clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.crossterm().clear_region(clear_type)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.crossterm().append_lines(n)
    }

    fn size(&self) -> io::Result<Size> {
        // Like CrosstermBackend::size, this queries the host, not the writer.
        crossterm::terminal::size().map(|(width, height)| Size::new(width, height))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.crossterm().window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<W: Write> Write for FrameBackend<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

struct FrameGuard<'a, W: Write> {
    terminal: &'a mut Terminal<FrameBackend<W>>,
    committed: bool,
}

impl<W: Write> Drop for FrameGuard<'_, W> {
    fn drop(&mut self) {
        if !self.committed {
            self.terminal.backend_mut().writer.abort();
        }
    }
}

/// One application-level write for an ordinary frame. A short-writing device
/// can require more writes, which diagnostics count rather than concealing.
pub fn draw<W: Write>(
    terminal: &mut Terminal<FrameBackend<W>>,
    render: impl FnOnce(&mut Frame),
) -> io::Result<DrawStats> {
    let began = Instant::now();
    terminal.backend_mut().writer.begin();
    let mut guard = FrameGuard {
        terminal,
        committed: false,
    };
    guard
        .terminal
        .backend_mut()
        .queue(BeginSynchronizedUpdate)?;
    // Via Terminal, so Ratatui knows the cursor is hidden even on consecutive
    // visible frames. Unsupported synchronized updates still leave it hidden
    // during every cursor movement in the cell diff.
    guard.terminal.hide_cursor()?;
    let mut ui = Duration::ZERO;
    guard.terminal.draw(|frame| {
        let began = Instant::now();
        render(frame);
        ui = began.elapsed();
    })?;
    let backend = guard.terminal.backend_mut();
    if backend.show_at_commit {
        backend.crossterm().show_cursor()?;
    }
    backend.queue(EndSynchronizedUpdate)?;
    let backend_time = began.elapsed().saturating_sub(ui);
    let output_began = Instant::now();
    let (bytes, writes) = backend.writer.commit()?;
    let output = output_began.elapsed();
    guard.committed = true;
    Ok(DrawStats {
        ui,
        backend: backend_time,
        output,
        bytes,
        writes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use ratatui::layout::Rect;
    use ratatui::{TerminalOptions, Viewport};

    #[derive(Default)]
    struct Recording {
        chunks: Vec<Vec<u8>>,
        write_calls: usize,
        flush_calls: usize,
        max_write: Option<usize>,
        fail_write: Option<usize>,
        fail_flush: Option<usize>,
        interrupt_write: Option<usize>,
        zero_write: Option<usize>,
    }

    #[derive(Clone, Default)]
    struct RecordingWriter(Rc<RefCell<Recording>>);

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut recording = self.0.borrow_mut();
            recording.write_calls += 1;
            if recording.fail_write == Some(recording.write_calls) {
                return Err(io::Error::other("injected write failure"));
            }
            if recording.interrupt_write == Some(recording.write_calls) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            if recording.zero_write == Some(recording.write_calls) {
                return Ok(0);
            }
            let n = bytes.len().min(recording.max_write.unwrap_or(usize::MAX));
            recording.chunks.push(bytes[..n].to_vec());
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            let mut recording = self.0.borrow_mut();
            recording.flush_calls += 1;
            if recording.fail_flush == Some(recording.flush_calls) {
                return Err(io::Error::other("injected flush failure"));
            }
            Ok(())
        }
    }

    fn fixture(writer: RecordingWriter) -> Terminal<FrameBackend<RecordingWriter>> {
        Terminal::with_options(
            FrameBackend::new(writer),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 20, 4)),
            },
        )
        .unwrap()
    }

    #[test]
    fn complete_frames_hide_before_diff_and_move_before_show() {
        let writer = RecordingWriter::default();
        let mut terminal = fixture(writer.clone());
        // Two successive visible frames, then focus to a view without a
        // cursor, then back. This catches stale deferred-show bookkeeping.
        for (index, cursor) in [Some((7, 2)), Some((8, 1)), None, Some((1, 3))]
            .into_iter()
            .enumerate()
        {
            let label = format!("DIFF{index}");
            let stats = draw(&mut terminal, |f| {
                f.render_widget(label.as_str(), f.area());
                if let Some(position) = cursor {
                    f.set_cursor_position(position);
                }
            })
            .unwrap();
            let recording = writer.0.borrow();
            assert_eq!(recording.chunks.len(), index + 1);
            assert_eq!(recording.flush_calls, index + 1);
            assert_eq!(stats.writes, 1);
            let bytes = &recording.chunks[index];
            assert_eq!(stats.bytes, bytes.len());
            let frame = String::from_utf8_lossy(bytes);
            assert!(frame.starts_with("\x1b[?2026h\x1b[?25l"), "{frame:?}");
            assert!(frame.ends_with("\x1b[?2026l"), "{frame:?}");
            if let Some((x, y)) = cursor {
                let suffix = format!("\x1b[{};{}H\x1b[?25h\x1b[?2026l", y + 1, x + 1);
                assert!(frame.ends_with(&suffix), "{frame:?}");
                assert_eq!(frame.matches("\x1b[?25h").count(), 1);
            } else {
                assert!(!frame.contains("\x1b[?25h"), "{frame:?}");
            }
        }
    }

    #[test]
    fn backend_flushes_and_frames_over_64_kib_cannot_escape_staging() {
        let writer = RecordingWriter::default();
        let mut stage = FrameWriter::new(writer.clone());
        let bytes = vec![b'x'; FRAME_CAPACITY * 3];
        stage.begin();
        stage.write_all(&bytes[..17]).unwrap();
        stage.flush().unwrap();
        stage.write_all(&bytes[17..]).unwrap();
        stage.flush().unwrap();
        assert!(writer.0.borrow().chunks.is_empty());
        assert_eq!(writer.0.borrow().flush_calls, 0);
        assert_eq!(stage.commit().unwrap(), (bytes.len(), 1));
        assert_eq!(writer.0.borrow().chunks, vec![bytes]);
        assert_eq!(writer.0.borrow().flush_calls, 1);
    }

    #[test]
    fn partial_write_failure_closes_update_and_never_replays_failed_frame() {
        let writer = RecordingWriter::default();
        {
            let mut recording = writer.0.borrow_mut();
            recording.max_write = Some(12);
            recording.fail_write = Some(2);
        }
        let mut terminal = fixture(writer.clone());
        let result = draw(&mut terminal, |f| {
            f.render_widget("FAILED FRAME", f.area());
            f.set_cursor_position((7, 2));
        });
        assert!(result.is_err());
        let before = writer.0.borrow().chunks.concat();
        assert!(before.ends_with(RECOVER));
        assert_eq!(before.len(), 12 + RECOVER.len());
        Write::flush(terminal.backend_mut()).unwrap();
        assert_eq!(writer.0.borrow().chunks.concat(), before);
    }

    #[test]
    fn failed_output_flush_recovers_cursor_and_synchronized_mode() {
        let writer = RecordingWriter::default();
        writer.0.borrow_mut().fail_flush = Some(1);
        let mut terminal = fixture(writer.clone());
        assert!(draw(&mut terminal, |f| f.render_widget("FRAME", f.area())).is_err());
        let recording = writer.0.borrow();
        assert_eq!(recording.chunks.len(), 2);
        assert_eq!(recording.chunks[1], RECOVER);
        assert_eq!(recording.flush_calls, 2);
    }

    #[test]
    fn panic_discards_uncommitted_frame_and_restores_cursor() {
        let writer = RecordingWriter::default();
        let mut terminal = fixture(writer.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = draw(&mut terminal, |f| {
                f.render_widget("NEVER DISPLAY THIS", f.area());
                panic!("injected renderer panic");
            });
        }));
        assert!(result.is_err());
        assert_eq!(writer.0.borrow().chunks, vec![RECOVER.to_vec()]);
        Write::flush(terminal.backend_mut()).unwrap();
        assert_eq!(writer.0.borrow().chunks, vec![RECOVER.to_vec()]);
    }

    #[test]
    fn interrupted_and_short_writes_complete_without_losing_bytes() {
        let writer = RecordingWriter::default();
        {
            let mut recording = writer.0.borrow_mut();
            recording.max_write = Some(7);
            recording.interrupt_write = Some(1);
        }
        let mut stage = FrameWriter::new(writer.clone());
        stage.begin();
        stage.write_all(b"a complete staged frame").unwrap();
        let (bytes, writes) = stage.commit().unwrap();
        assert_eq!(bytes, 23);
        assert_eq!(writes, 5);
        assert_eq!(
            writer.0.borrow().chunks.concat(),
            b"a complete staged frame"
        );
    }

    #[test]
    fn zero_write_fails_and_restores_instead_of_spinning() {
        let writer = RecordingWriter::default();
        writer.0.borrow_mut().zero_write = Some(1);
        let mut terminal = fixture(writer.clone());
        let err = draw(&mut terminal, |_| {}).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WriteZero);
        assert_eq!(writer.0.borrow().chunks, vec![RECOVER.to_vec()]);
    }
}
