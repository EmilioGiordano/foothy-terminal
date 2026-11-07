use std::time::Instant;

use render::{CursorLook, Renderer};
use vt_core::{SelectionType, Side, TermSize, Terminal};

fn time(label: &str, r: &mut Renderer, term: &Terminal, buf: &mut [u32], w: usize, h: usize) {
    // Warm the glyph cache so the first frame does not skew the average.
    r.draw(term, buf, w, h, CursorLook::Beam);

    let frames = 60;
    let t = Instant::now();
    for _ in 0..frames {
        r.draw(term, buf, w, h, CursorLook::Beam);
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    println!("{label:<34} {per:>7.2} ms/frame");
}

/// The xterm cube and ramp. An example supplies its own rather than `render`
/// taking a dependency on `config` for it.
fn palette() -> [u32; 256] {
    const ANSI16: [u32; 16] = [
        0x0000_0000,
        0x00aa_0000,
        0x0000_aa00,
        0x00aa_5500,
        0x0000_00aa,
        0x00aa_00aa,
        0x0000_aaaa,
        0x00aa_aaaa,
        0x0055_5555,
        0x00ff_5555,
        0x0055_ff55,
        0x00ff_ff55,
        0x0055_55ff,
        0x00ff_55ff,
        0x0055_ffff,
        0x00ff_ffff,
    ];
    const STEPS: [u32; 6] = [0, 95, 135, 175, 215, 255];
    std::array::from_fn(|i| match i {
        0..=15 => ANSI16[i],
        16..=231 => {
            let c = i - 16;
            (STEPS[c / 36] << 16) | (STEPS[(c / 6) % 6] << 8) | STEPS[c % 6]
        }
        _ => {
            let v = 8 + (i - 232) as u32 * 10;
            (v << 16) | (v << 8) | v
        }
    })
}

fn main() -> anyhow::Result<()> {
    let bytes = std::fs::read(r"C:\Windows\Fonts\CascadiaMono.ttf")?;
    let mut r = Renderer::new(&bytes, 16.0, &palette())?;
    let (cw, ch) = r.cell_size();

    let size = TermSize::new(200, 50);
    let (w, h) = (cw * size.columns, ch * size.screen_lines);
    let mut buf = vec![0u32; w * h];

    let mut term = Terminal::new(size);
    for i in 0..size.screen_lines {
        term.feed(
            format!("line {i} with padding chars used to fill the complete row\r\n").as_bytes(),
        );
    }

    println!(
        "grid {}x{}  ({w}x{h} px)\n",
        size.columns, size.screen_lines
    );

    time("no selection", &mut r, &term, &mut buf, w, h);

    term.start_selection(0, 0, Side::Left, SelectionType::Simple);
    term.update_selection(size.screen_lines - 1, size.columns - 1, Side::Right);
    time("Simple selection (screen)", &mut r, &term, &mut buf, w, h);

    term.start_selection(0, 0, Side::Left, SelectionType::Semantic);
    term.update_selection(size.screen_lines - 1, size.columns - 1, Side::Right);
    time("Semantic selection (word)", &mut r, &term, &mut buf, w, h);

    term.start_selection(0, 0, Side::Left, SelectionType::Lines);
    term.update_selection(size.screen_lines - 1, size.columns - 1, Side::Right);
    time("Lines selection (line)", &mut r, &term, &mut buf, w, h);

    let mut styled = Terminal::new(size);
    for i in 0..size.screen_lines {
        let sgr = ["0", "1", "2", "3", "4", "9", "1;4", "2;3"][i % 8];
        styled.feed(
            format!(
                "\x1b[{sgr}mline {i} with padding chars used to fill the complete row\x1b[0m\r\n"
            )
            .as_bytes(),
        );
    }
    time(
        "every attribute, one per line",
        &mut r,
        &styled,
        &mut buf,
        w,
        h,
    );

    Ok(())
}
