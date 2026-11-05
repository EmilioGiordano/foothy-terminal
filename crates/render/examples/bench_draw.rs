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

fn main() -> anyhow::Result<()> {
    let bytes = std::fs::read(r"C:\Windows\Fonts\CascadiaMono.ttf")?;
    let mut r = Renderer::new(&bytes, 16.0)?;
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

    Ok(())
}
