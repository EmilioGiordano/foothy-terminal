use std::time::{Duration, Instant};

use portable_pty::CommandBuilder;
use vt_core::pty::Pty;
use vt_core::{TermSize, Terminal};

fn main() -> anyhow::Result<()> {
    let cmd_arg = std::env::args().nth(1).unwrap_or_else(|| "dir".to_string());
    let size = TermSize::new(100, 30);
    let started = Instant::now();

    let mut term = Terminal::new(size);
    let mut cmd = CommandBuilder::new("powershell.exe");
    cmd.args(["-NoLogo", "-NoProfile", "-Command", &cmd_arg]);
    cmd.cwd(std::env::current_dir()?);
    let mut pty = Pty::spawn(cmd, size)?;

    let spawn_ms = started.elapsed().as_secs_f64() * 1000.0;

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut total_bytes = 0usize;
    loop {
        let d = pty.drain(10);
        let (bytes, eof) = (d.bytes, d.eof);
        if !bytes.is_empty() {
            total_bytes += bytes.len();
            term.feed(&bytes);

            let replies = term.take_pty_replies();
            if !replies.is_empty() {
                pty.write(&replies)?;
            }
        }
        if eof {
            break;
        }
        if let Some(status) = pty.child_exited() {
            // The child exiting says nothing about the pipe being empty: bytes are
            // still in flight in the reader thread. Draining once truncates output.
            let quiet_until = Instant::now() + Duration::from_millis(50);
            while Instant::now() < quiet_until {
                let rest = pty.drain(100).bytes;
                if rest.is_empty() {
                    std::thread::sleep(Duration::from_millis(2));
                } else {
                    total_bytes += rest.len();
                    term.feed(&rest);
                }
            }
            eprintln!("(child exited: {status:?})");
            break;
        }
        if Instant::now() > deadline {
            eprintln!("(timeout: the child did not exit within 10 s)");
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    let total_ms = started.elapsed().as_secs_f64() * 1000.0;

    println!("--- grid ({}x{}) ---", size.columns, size.screen_lines);
    println!("{}", term.screen_text());
    println!("--- end ---");

    let (row, col) = term.cursor();
    eprintln!(
        "spawn {spawn_ms:.1} ms · total {total_ms:.1} ms · {total_bytes} bytes · cursor ({row},{col})"
    );

    Ok(())
}
