//! The metric `docs/02` §7 calls the one that defines the project: opening a
//! pane must cost under 50 ms **and stay constant with the number of panes**.
//! That is what Warp fails and what motivated this terminal.
//!
//!     cargo run --release -p vt-core --example pane_cost

use std::time::Instant;

use portable_pty::CommandBuilder;
use vt_core::pty::Pty;
use vt_core::{TermSize, Terminal};

struct Pane {
    _term: Terminal,
    _pty: Pty,
}

fn main() -> anyhow::Result<()> {
    let size = TermSize::new(80, 24);
    let total = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(12usize);

    println!(
        "opening {total} panes of {}x{}\n",
        size.columns, size.screen_lines
    );
    println!("{:>5}  {:>9}", "pane", "ms");

    let mut panes = Vec::new();
    let mut times = Vec::new();

    for i in 1..=total {
        let started = Instant::now();
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.cwd(std::env::current_dir()?);
        let pty = Pty::spawn(cmd, size)?;
        let term = Terminal::with_cursor(size, vt_core::CursorShape::Beam);
        let ms = started.elapsed().as_secs_f64() * 1000.0;

        panes.push(Pane {
            _term: term,
            _pty: pty,
        });
        times.push(ms);
        println!("{i:>5}  {ms:>9.1}");
    }

    let first = times[0];
    let last = times[times.len() - 1];
    let worst = times.iter().cloned().fold(f64::MIN, f64::max);
    let mean = times.iter().sum::<f64>() / times.len() as f64;

    println!("\nmean {mean:.1} ms · worst {worst:.1} ms");
    println!("first {first:.1} ms · last {last:.1} ms");

    // Constant with N is the point: the last pane must not cost meaningfully
    // more than the first. A generous factor still catches per-pane growth.
    let budget = 50.0;
    let ok_budget = worst < budget;
    let ok_constant = last < first.max(mean) * 2.0 + 5.0;
    println!(
        "\n< {budget:.0} ms per pane : {}",
        if ok_budget { "OK" } else { "FAIL" }
    );
    println!(
        "constant with N  : {}",
        if ok_constant { "OK" } else { "FAIL" }
    );

    // Dropping happens off-thread, so give the closes a moment to land.
    drop(panes);
    std::thread::sleep(std::time::Duration::from_millis(300));
    Ok(())
}
