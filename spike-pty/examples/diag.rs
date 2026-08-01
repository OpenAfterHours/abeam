//! Diagnostic: what does a ConPTY child actually do on this machine?
//!
//! Prints, twice a second: whether `try_wait` reports exit, how many bytes have
//! arrived, and whether the reader saw EOF. Run with:
//!
//!   cargo run --example diag
//!   cargo run --example diag -- cmd.exe /c "echo hi"

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<String> = if args.is_empty() {
        vec!["cmd.exe".into(), "/c".into(), "echo hi".into()]
    } else {
        args
    };
    println!("argv: {argv:?}");

    let sys = portable_pty::native_pty_system();
    let pair = sys
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(&argv[0]);
    for a in &argv[1..] {
        cmd.arg(a);
    }

    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    println!("spawned, pid = {:?}", child.process_id());

    let collected = Arc::new(Mutex::new(Vec::new()));
    let eof = Arc::new(AtomicBool::new(false));
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");
    {
        let collected = Arc::clone(&collected);
        let eof = Arc::clone(&eof);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        println!("[reader] clean EOF");
                        break;
                    }
                    Ok(n) => {
                        let chunk = &buf[..n];
                        // ConPTY opens with a Device Status Report query and
                        // blocks until the host answers. Answer it.
                        if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                            println!("[reader] saw DSR query, replying");
                            let _ = writer.write_all(b"\x1b[1;1R");
                            let _ = writer.flush();
                        }
                        collected.lock().unwrap().extend_from_slice(chunk);
                    }
                    Err(e) => {
                        println!("[reader] error: {e}");
                        break;
                    }
                }
            }
            eof.store(true, Ordering::Relaxed);
        });
    }

    for i in 1..=12 {
        std::thread::sleep(Duration::from_millis(500));
        let status = child.try_wait().expect("try_wait");
        let bytes = collected.lock().unwrap().len();
        println!(
            "t={:>4.1}s  try_wait={:<12}  bytes={:<5}  reader_done={}",
            i as f32 * 0.5,
            format!("{status:?}"),
            bytes,
            eof.load(Ordering::Relaxed)
        );
        if status.is_some() {
            break;
        }
    }

    let raw = collected.lock().unwrap().clone();
    println!("\n--- raw output ({} bytes) ---", raw.len());
    println!("{:?}", String::from_utf8_lossy(&raw));

    let _ = child.kill();
}
