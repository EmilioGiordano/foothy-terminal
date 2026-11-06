//! Runs a PowerShell script through `-NoExit -EncodedCommand` under ConPTY and
//! dumps the grid, before and after a command. Diagnostics for prompt work: it
//! shows what the shell actually emits without opening a window.
//!
//! `cargo run -p vt-core --example prompt_probe -- <script.ps1>`

use std::time::{Duration, Instant};

use portable_pty::CommandBuilder;
use vt_core::pty::Pty;
use vt_core::{TermSize, Terminal};

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn pump(term: &mut Terminal, pty: &mut Pty, for_ms: u64) {
    let until = Instant::now() + Duration::from_millis(for_ms);
    while Instant::now() < until {
        let d = pty.drain(50);
        if d.bytes.is_empty() {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        term.feed(&d.bytes);
        let replies = term.take_pty_replies();
        if !replies.is_empty() {
            let _ = pty.write(&replies);
        }
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: prompt_probe <script.ps1>"))?;
    let script = std::fs::read_to_string(&path)?;

    let size = TermSize::new(100, 20);
    let mut term = Terminal::new(size);

    let mut cmd = CommandBuilder::new("powershell.exe");
    cmd.args([
        "-NoLogo",
        "-NoExit",
        "-EncodedCommand",
        &b64(&utf16le(&script)),
    ]);
    cmd.cwd(std::env::current_dir()?);
    let mut pty = Pty::spawn(cmd, size)?;

    pump(&mut term, &mut pty, 1800);
    println!("--- startup ---");
    println!("{}", term.screen_text().trim_end());

    // Enter is CR. Sending CRLF makes PowerShell open a continuation prompt.
    pty.write(b"cd ..\r")?;
    pump(&mut term, &mut pty, 1200);
    pty.write(b"echo listo\r")?;
    pump(&mut term, &mut pty, 1200);
    println!("--- after cd .. and a command ---");
    println!("{}", term.screen_text().trim_end());

    pty.write(b"exit\r")?;
    pump(&mut term, &mut pty, 600);
    eprintln!("(child: {:?})", pty.child_exited());
    Ok(())
}
