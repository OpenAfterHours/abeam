//! Manual ConPTY transport check. Synthetic by default; an explicitly supplied
//! ABEAM_REPAINT_PROBE_PROGRAM captures eight seconds of a real CLI startup.

use super::*;
use std::io::{Read, Write};
use std::sync::mpsc;

const CHILD: &str = "ABEAM_REPAINT_PROBE_CHILD";
const ROWS: u16 = 24;
const COLS: u16 = 120;

struct ProbeChild {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    tree: Option<crate::tree::Tree>,
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.tree.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Run with `cargo test --release -p abeam-pty conpty_animated_repaint -- --ignored --nocapture`.
/// A test executable serves as its own child so neither a shell nor an agent
/// installation changes the workload. Timing results are deliberately not CI gates.
#[test]
#[ignore = "manual ConPTY transport and publication timing probe"]
fn conpty_animated_repaint() {
    if std::env::var_os(CHILD).is_some() {
        animate();
        return;
    }
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let external = std::env::var_os("ABEAM_REPAINT_PROBE_PROGRAM");
    let mut command = portable_pty::CommandBuilder::new(
        external
            .clone()
            .unwrap_or_else(|| std::env::current_exe().unwrap().into()),
    );
    command.cwd(std::env::current_dir().unwrap());
    if external.is_none() {
        command.args([
            "--exact",
            "frames::probe::conpty_animated_repaint",
            "--ignored",
            "--nocapture",
        ]);
        command.env(CHILD, "1");
    }
    let child = pair.slave.spawn_command(command).unwrap();
    let child = ProbeChild {
        tree: crate::tree::Tree::holding(&*child),
        child,
    };
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = [0; 8192];
        while let Ok(n) = reader.read(&mut bytes) {
            if n == 0 || tx.send((Instant::now(), bytes[..n].to_vec())).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut screens = Screens::new(ROWS, COLS, 0);
    let mut reads = 0;
    let mut bytes = 0;
    let mut observed = 0;
    let mut bad_cursor = 0;
    let mut armed = false;
    let mut recent = std::collections::VecDeque::new();
    let mut finished = false;
    let mut publication_delays = Vec::new();
    let mut typed = 0;
    let typing = external.is_some() && std::env::var_os("ABEAM_REPAINT_PROBE_TYPE").is_some();
    let sample_text = b"renderprobe";
    while started.elapsed() < Duration::from_secs(if external.is_some() { 8 } else { 15 }) {
        if typing
            && typed < sample_text.len()
            && started.elapsed() >= Duration::from_millis(2000 + typed as u64 * 50)
        {
            // No Enter: this only exercises editing the new process's prompt.
            writer.write_all(&sample_text[typed..typed + 1]).unwrap();
            writer.flush().unwrap();
            typed += 1;
        }
        let wait = screens.deadline().map_or(Duration::from_millis(50), |at| {
            at.saturating_duration_since(Instant::now())
        });
        let before = screens.publications;
        match rx.recv_timeout(wait) {
            Ok((at, chunk)) => {
                reads += 1;
                bytes += chunk.len();
                if recent.len() == 6 {
                    recent.pop_front();
                }
                recent.push_back(format!(
                    "{:.3}ms {:?}",
                    at.duration_since(started).as_secs_f64() * 1000.0,
                    String::from_utf8_lossy(&chunk)
                ));
                if external.is_some() {
                    println!("READ {}", recent.back().unwrap());
                }
                for (row, col) in screens.process(&chunk, at) {
                    writer
                        .write_all(&crate::input::dsr_reply(row, col))
                        .unwrap();
                    writer.flush().unwrap();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                screens.publish_due(Instant::now());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if screens.publications != before {
            if let Some(publication) = screens.unpresented.take() {
                publication_delays.push(
                    publication
                        .published_at
                        .saturating_duration_since(publication.received_at),
                );
            }
            let screen = screens.display.screen();
            if external.is_some() {
                println!(
                    "PUBLISH {:.3}ms cursor={:?} hidden={}",
                    started.elapsed().as_secs_f64() * 1000.0,
                    screen.cursor_position(),
                    screen.hide_cursor()
                );
            }
            if screen.contents().contains("PROBE-DONE") {
                finished = true;
                break;
            }
            armed |=
                screen.contents().contains("PROBE-READY") && screen.cursor_position().0 == ROWS - 2;
            if armed {
                observed += 1;
                if !screen.hide_cursor() && screen.cursor_position().0 != ROWS - 2 {
                    bad_cursor += 1;
                    if bad_cursor <= 3 {
                        println!(
                            "visible intermediate cursor {:?}; recent reads: {recent:#?}",
                            screen.cursor_position()
                        );
                    }
                }
            }
        }
    }
    drop(child);
    publication_delays.sort_unstable();
    let percentile = |percent: usize| {
        publication_delays
            .get(
                (publication_delays.len() * percent)
                    .div_ceil(100)
                    .saturating_sub(1),
            )
            .copied()
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.0
    };
    println!(
        "reads={reads} bytes={bytes} publications={} synthetic_observed={observed} synthetic_bad_cursors={bad_cursor} sync_timeouts={} tail_timeouts={} receipt_to_publication_p95={:.3}ms p99={:.3}ms typed={typed}",
        screens.publications,
        screens.sync_timeouts,
        screens.tail_timeouts,
        percentile(95),
        percentile(99)
    );
    assert!(reads > 0, "child produced no output");
    if external.is_none() {
        assert!(
            finished && observed > 0,
            "synthetic animation did not complete"
        );
        assert_eq!(
            bad_cursor, 0,
            "published a visible cursor outside the input row"
        );
    }
}

fn animate() {
    use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
    let mut out = std::io::stdout();
    // Also enables Windows VT processing through Crossterm's command path.
    crossterm::execute!(out, BeginSynchronizedUpdate).unwrap();
    write!(
        out,
        "\x1b[?25l\x1b[2J\x1b[HPROBE-READY\x1b[{};3H\x1b[?25h",
        ROWS - 1
    )
    .unwrap();
    crossterm::execute!(out, EndSynchronizedUpdate).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    for frame in 0..180 {
        crossterm::queue!(out, BeginSynchronizedUpdate).unwrap();
        write!(out, "\x1b[?25l").unwrap();
        for star in 0..40 {
            let row = 2 + (star * 7 + frame) % (usize::from(ROWS) - 5);
            let col = 1 + (star * 17 + frame) % (usize::from(COLS) - 1);
            write!(
                out,
                "\x1b[{row};{col}H{}",
                if frame % 2 == 0 { '*' } else { '.' }
            )
            .unwrap();
        }
        write!(out, "\x1b[{};3Htyping {:03}\x1b[?25h", ROWS - 1, frame).unwrap();
        crossterm::execute!(out, EndSynchronizedUpdate).unwrap();
        std::thread::sleep(Duration::from_millis(16));
    }
    write!(out, "\x1b[HPROBE-DONE").unwrap();
    out.flush().unwrap();
    std::thread::sleep(Duration::from_millis(100));
}
