//! Separate the terminal state used for input from the last published repaint.
//!
//! A second persistent parser replays committed bytes. This doubles the screen
//! and scrollback storage, but never clones an entire transcript to publish one
//! frame. The pending byte buffer and every publication delay are bounded.

use std::time::{Duration, Instant};

pub(crate) const SYNC_TIMEOUT: Duration = Duration::from_millis(100);
const TAIL_QUIET: Duration = Duration::from_millis(4);
const MAX_TAIL: Duration = Duration::from_millis(16);
const MAX_PENDING: usize = 256 * 1024;

#[derive(Default)]
enum Event {
    #[default]
    None,
    Begin,
    End,
    Reset,
    CursorQuery,
}

impl vte::Perform for Event {
    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, c: char) {
        if ignore {
            return;
        }
        if intermediates == b"?" && params.iter().any(|p| p == [2026]) {
            match c {
                'h' => *self = Self::Begin,
                'l' => *self = Self::End,
                _ => {}
            }
        } else if intermediates.is_empty() && c == 'n' && params.iter().eq([&[6][..]]) {
            *self = Self::CursorQuery;
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if !ignore && intermediates.is_empty() && byte == b'c' {
            *self = Self::Reset;
        }
    }

    fn terminated(&self) -> bool {
        !matches!(self, Self::None)
    }
}

pub(crate) struct Screens {
    pub(crate) live: vt100::Parser,
    pub(crate) display: vt100::Parser,
    scanner: vte::Parser,
    pending: Vec<u8>,
    sync_since: Option<Instant>,
    tail_since: Option<Instant>,
    last_output: Option<Instant>,
    coalesce: bool,
    exited: bool,
    pub(crate) stopped: bool,
    pub(crate) publications: u64,
    pub(crate) sync_timeouts: u64,
}

impl Screens {
    pub(crate) fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self::with_coalescing(rows, cols, scrollback, cfg!(windows))
    }

    fn with_coalescing(rows: u16, cols: u16, scrollback: usize, coalesce: bool) -> Self {
        Self {
            live: vt100::Parser::new(rows, cols, scrollback),
            display: vt100::Parser::new(rows, cols, scrollback),
            scanner: vte::Parser::new(),
            pending: Vec::new(),
            sync_since: None,
            tail_since: None,
            last_output: None,
            coalesce,
            exited: false,
            stopped: false,
            publications: 0,
            sync_timeouts: 0,
        }
    }

    /// Parse immediately, recording each DSR cursor at the query itself. The
    /// caller writes replies after releasing the screen lock, even while a
    /// synchronized repaint is held. Publication never gates input modes.
    pub(crate) fn process(&mut self, mut bytes: &[u8], now: Instant) -> Vec<(u16, u16)> {
        let mut replies = Vec::new();
        self.publish_due(now);
        while !bytes.is_empty() {
            let mut event = Event::None;
            // Bound each segment too: even a single huge caller-provided chunk
            // cannot allocate beyond the pending limit.
            let limit = bytes.len().min(MAX_PENDING - self.pending.len());
            let used = self
                .scanner
                .advance_until_terminated(&mut event, &bytes[..limit]);
            let segment = &bytes[..used];
            self.live.process(segment);
            self.pending.extend_from_slice(segment);
            self.last_output = Some(now);
            self.tail_since.get_or_insert(now);
            bytes = &bytes[used..];

            match event {
                Event::Begin if self.sync_since.is_none() && !self.exited => {
                    // A completed A followed by the start of B in one read
                    // must leave A visible while B is held. Withhold the final
                    // dispatch byte: a combined CSI ?2026;1049h also clears the
                    // alternate screen, and that belongs to B. Feeding the
                    // prefix is safe even if it spans already published reads;
                    // the display parser keeps its incomplete CSI state.
                    let dispatch = self.pending.pop().unwrap();
                    debug_assert_eq!(dispatch, b'h');
                    self.publish();
                    self.pending.push(dispatch);
                    self.sync_since = Some(now);
                }
                Event::End => {
                    if self.sync_since.take().is_some() {
                        // ConPTY can forward END before its final screen bytes.
                        // Give those trailing bytes their own short quiet window.
                        self.tail_since = Some(now);
                    }
                }
                Event::Reset => {
                    self.recover();
                }
                Event::CursorQuery => replies.push(self.live.screen().cursor_position()),
                Event::None | Event::Begin => {}
            }

            if self.pending.len() == MAX_PENDING {
                // A pathological repaint cannot grow pending memory without
                // limit. Production calls are separately bounded by the
                // reader's 8KiB buffer. Recover as for a missing END.
                self.recover();
            }
        }
        if self.exited || (!self.coalesce && self.sync_since.is_none()) {
            self.publish();
        }
        replies
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        if let Some(since) = self.sync_since {
            return Some(since + SYNC_TIMEOUT);
        }
        let since = self.tail_since?;
        Some((self.last_output? + TAIL_QUIET).min(since + MAX_TAIL))
    }

    pub(crate) fn publish_due(&mut self, now: Instant) -> bool {
        if self.deadline().is_none_or(|deadline| now < deadline) {
            return false;
        }
        if self.sync_since.is_some() {
            self.sync_timeouts += 1;
        }
        // Cancel a timed-out mode, rather than imposing another 100ms wait on
        // every subsequent write from a child that never sends END.
        self.recover()
    }

    fn publish(&mut self) -> bool {
        self.tail_since = None;
        self.last_output = None;
        if self.pending.is_empty() {
            return false;
        }
        self.display.process(&self.pending);
        self.pending.clear();
        self.publications += 1;
        true
    }

    pub(crate) fn recover(&mut self) -> bool {
        self.sync_since = None;
        self.publish()
    }

    pub(crate) fn child_exited(&mut self) -> bool {
        self.exited = true;
        self.recover()
    }

    pub(crate) fn resize(&mut self, rows: u16, cols: u16) {
        // Replay under the old geometry before resizing either parser.
        self.recover();
        self.live.screen_mut().set_size(rows, cols);
        self.display.screen_mut().set_size(rows, cols);
    }

    pub(crate) fn move_view(&mut self, to: impl FnOnce(usize) -> usize) -> bool {
        let before = self.live.screen().scrollback();
        self.live.screen_mut().set_scrollback(to(before));
        let after = self.live.screen().scrollback();
        if before == after {
            return false;
        }
        // Restore the old view for replay, then apply the user's move to both.
        self.live.screen_mut().set_scrollback(before);
        self.recover();
        self.live.screen_mut().set_scrollback(after);
        self.display.screen_mut().set_scrollback(after);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screens(coalesce: bool) -> Screens {
        Screens::with_coalescing(6, 24, 5000, coalesce)
    }

    #[test]
    fn split_repaint_keeps_committed_cells_and_cursor_until_end() {
        let now = Instant::now();
        let mut s = screens(false);
        s.process(b"READY\x1b[2;3H", now);
        let cursor = s.display.screen().cursor_position();
        for chunk in [b"\x1b[?20".as_slice(), b"26h\x1b[HPARTIAL\x1b[4;8H"] {
            s.process(chunk, now);
        }
        // This is the read an unrelated pane's redraw makes during the hold.
        assert_eq!(s.display.screen().contents(), "READY");
        assert_eq!(s.display.screen().cursor_position(), cursor);
        assert_eq!(s.live.screen().cursor_position(), (3, 7));
        s.process(b"\x1b[HFINAL  \x1b[5;4H\x1b[?202", now);
        assert_eq!(s.display.screen().contents(), "READY");
        s.process(b"6l", now);
        assert_eq!(s.display.screen().contents(), "FINAL  ");
        assert_eq!(s.display.screen().cursor_position(), (4, 3));
    }

    #[test]
    fn a_finished_frame_is_published_before_the_next_begin_in_the_same_read() {
        let now = Instant::now();
        let mut s = screens(true);
        s.process(b"\x1b[?2026hA\x1b[?2026l\x1b[?2026h\rB", now);
        assert_eq!(s.display.screen().contents(), "A");
        assert_eq!(s.live.screen().contents(), "B");
        assert!(!s.publish_due(now + Duration::from_millis(99)));
        s.process(b"\x1b[?2026l", now + Duration::from_millis(99));
        assert!(s.publish_due(now + Duration::from_millis(103)));
        assert_eq!(s.display.screen().contents(), "B");
    }

    #[test]
    fn combined_begin_and_alternate_screen_modes_keep_all_visual_effects_held() {
        let now = Instant::now();
        for begin in [b"\x1b[?2026;1049h".as_slice(), b"\x1b[?1049;2026h"] {
            for split in 0..=begin.len() {
                let mut s = screens(false);
                s.process(b"READY\x1b[3;4H", now);
                let cursor = s.display.screen().cursor_position();
                s.process(&begin[..split], now);
                s.process(&begin[split..], now);
                s.process(b"PARTIAL\x1b[4;8H", now);
                assert_eq!(s.display.screen().contents(), "READY", "split {split}");
                assert!(!s.display.screen().alternate_screen());
                assert_eq!(s.display.screen().cursor_position(), cursor);
                assert!(s.live.screen().alternate_screen());
                s.process(b"\x1b[2J\x1b[HFINAL\x1b[?2026l", now);
                assert!(s.display.screen().alternate_screen());
                assert_eq!(s.display.screen().contents(), "FINAL");
            }
        }
    }

    #[test]
    fn combined_begin_keeps_cursor_modes_held_and_timeout_replays_dispatch() {
        let now = Instant::now();
        let mut s = screens(false);
        s.process(b"READY\x1b[3;4H", now);
        s.process(b"\x1b[?2026;6h", now);
        assert_eq!(s.live.screen().cursor_position(), (0, 0));
        assert_eq!(s.display.screen().cursor_position(), (2, 3));
        assert!(s.publish_due(now + SYNC_TIMEOUT));
        assert_eq!(
            s.live.screen().cursor_position(),
            s.display.screen().cursor_position()
        );
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());

        s.process(b"\x1b[?1049;2026hNEW", now + SYNC_TIMEOUT);
        assert!(!s.display.screen().alternate_screen());
        assert!(s.publish_due(now + SYNC_TIMEOUT * 2));
        assert!(s.display.screen().alternate_screen());
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());
    }

    #[test]
    fn conpty_end_before_final_bytes_waits_for_a_short_quiet_tail() {
        let now = Instant::now();
        let mut s = screens(true);
        s.process(b"READY", now);
        s.publish_due(now + TAIL_QUIET);
        let start = now + Duration::from_millis(10);
        s.process(b"\x1b[?2026h\rPARTIAL\x1b[?2026l", start);
        assert!(!s.publish_due(start + Duration::from_millis(2)));
        s.process(b"\rFINAL  \x1b[3;2H", start + Duration::from_millis(3));
        assert!(!s.publish_due(start + Duration::from_millis(6)));
        assert_eq!(s.display.screen().contents(), "READY");
        assert!(s.publish_due(start + Duration::from_millis(7)));
        assert_eq!(s.display.screen().contents(), "FINAL  ");
        assert_eq!(s.display.screen().cursor_position(), (2, 1));
    }

    #[test]
    fn unmarked_continuous_output_and_duplicate_ends_cannot_extend_tail_forever() {
        let now = Instant::now();
        let mut s = screens(true);
        for ms in [0, 3, 6, 9, 12, 15] {
            s.process(b"x\x1b[?2026l", now + Duration::from_millis(ms));
            assert_eq!(s.display.screen().contents(), "");
        }
        assert!(s.publish_due(now + MAX_TAIL));
        assert_eq!(s.display.screen().contents(), "xxxxxx");
    }

    #[test]
    fn duplicate_begins_do_not_extend_timeout_and_missing_end_recovers_once() {
        let now = Instant::now();
        let mut s = screens(false);
        s.process(b"\x1b[?2026hHELD", now);
        s.process(b"\x1b[?2026h", now + Duration::from_millis(90));
        assert!(s.publish_due(now + SYNC_TIMEOUT));
        assert_eq!(s.sync_timeouts, 1);
        assert_eq!(s.display.screen().contents(), "HELD");
        s.process(b" MORE", now + SYNC_TIMEOUT);
        assert_eq!(s.display.screen().contents(), "HELD MORE");
        assert_eq!(s.deadline(), None);
    }

    #[test]
    fn dsr_uses_each_live_cursor_and_input_modes_change_during_a_hold() {
        let now = Instant::now();
        let mut s = screens(false);
        let replies = s.process(
            b"\x1b[?2026h\x1b[?1;2004;1003;1006h\x1b[2;3H\x1b[6n\x1b[4;9H\x1b[6n\x1b[H",
            now,
        );
        assert_eq!(replies, [(1, 2), (3, 8)]);
        assert!(s.live.screen().application_cursor());
        assert!(s.live.screen().bracketed_paste());
        assert_eq!(
            s.live.screen().mouse_protocol_mode(),
            vt100::MouseProtocolMode::AnyMotion
        );
        assert_eq!(
            s.live.screen().mouse_protocol_encoding(),
            vt100::MouseProtocolEncoding::Sgr
        );
        assert!(!s.display.screen().application_cursor());
    }

    #[test]
    fn streaming_parser_handles_utf8_private_mode_lists_and_control_strings() {
        let now = Instant::now();
        let bytes = "\x1b[?25;2026h\x1b[31m界é\x1b[?2026l".as_bytes();
        for split in 0..=bytes.len() {
            let mut s = screens(false);
            s.process(&bytes[..split], now);
            s.process(&bytes[split..], now);
            assert_eq!(s.display.screen().contents(), "界é", "split {split}");
            assert_eq!(
                s.display.screen().cell(0, 0).unwrap().fgcolor(),
                vt100::Color::Idx(1)
            );
            assert!(!s.display.screen().hide_cursor());
        }
        let mut s = screens(false);
        // A DCS payload contains the printable spelling of a mode, not a CSI.
        s.process(b"\x1bPq[?2026h\x1b\\\x1b]2;[?2026h\x07OK", now);
        assert_eq!(s.display.screen().contents(), "OK");
        assert_eq!(s.deadline(), None);
    }

    #[test]
    fn reset_exit_resize_and_scrollback_reconcile_both_parsers() {
        let now = Instant::now();
        let mut s = screens(false);
        s.process(b"\x1b[?2026hOLD\x1b", now);
        s.process(b"cNEW", now);
        assert_eq!(s.display.screen().contents(), "NEW");
        assert_eq!(s.deadline(), None);

        s.process(b"\x1b[?2026h\rWIDE CONTENT", now);
        s.resize(4, 8);
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());
        assert_eq!(s.live.screen().size(), s.display.screen().size());
        assert_eq!(s.deadline(), None);

        for _ in 0..20 {
            s.process(b"\r\nHISTORY", now);
        }
        s.process(b"\x1b[?2026h\r\nHELD", now);
        assert!(s.move_view(|_| 3));
        assert_eq!(s.live.screen().scrollback(), 3);
        assert_eq!(s.display.screen().scrollback(), 3);
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());
        s.process(b"\r\nMORE", now);
        assert_eq!(
            s.live.screen().scrollback(),
            s.display.screen().scrollback()
        );
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());

        s.move_view(|_| 0);
        s.process(b"\x1b[?2026h\rEXIT", now);
        assert!(s.child_exited());
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());
        s.process(b"\rTAIL", now);
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());
        assert_eq!(s.deadline(), None);
    }

    #[test]
    fn byte_budget_bounds_a_repaint_even_before_the_deadline() {
        let now = Instant::now();
        let mut s = screens(false);
        s.process(b"\x1b[?2026h", now);
        s.process(&vec![b'x'; MAX_PENDING + 7], now);
        assert!(s.pending.len() < MAX_PENDING);
        assert_eq!(s.deadline(), None);
        assert_eq!(s.live.screen().contents(), s.display.screen().contents());
    }

    /// Repeatable parser-only cost, deliberately separate from display latency.
    /// cargo test -p abeam-pty --release benchmark_publication -- --ignored --nocapture
    #[test]
    #[ignore = "manual release-mode timing capture"]
    fn benchmark_publication_with_populated_history() {
        use std::hint::black_box;

        for (rows, cols) in [(38, 120), (58, 240)] {
            let history = b"history entry\r\n".repeat(5100);
            let full: Vec<u8> = (1..=rows)
                .flat_map(|row| format!("\x1b[{row};1H{}", "x".repeat(cols as usize)).into_bytes())
                .collect();
            for (name, payload, iterations) in [
                ("one cell", b"\x1b[H!".to_vec(), 2000),
                ("full pane", full, 2000),
                ("256KiB burst", b"x".repeat(MAX_PENDING), 100),
            ] {
                let mut old = vt100::Parser::new(rows, cols, 5000);
                old.process(&history);
                let mut new = Screens::with_coalescing(rows, cols, 5000, false);
                new.process(&history, Instant::now());
                let started = Instant::now();
                for _ in 0..iterations {
                    old.process(black_box(&payload));
                }
                let old_ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations);
                let started = Instant::now();
                for _ in 0..iterations {
                    new.process(black_box(&payload), Instant::now());
                }
                let new_ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations);
                black_box((old.screen(), new.display.screen()));
                println!("{cols}x{rows} {name}: old={old_ms:.4}ms publication={new_ms:.4}ms");
            }
        }
    }
}
