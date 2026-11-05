use render::Renderer;

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
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"C:\Windows\Fonts\CascadiaMono.ttf".to_string());
    let bytes = std::fs::read(&path)?;
    let r = Renderer::new(&bytes, 16.0, &palette())?;
    println!("{path}  cell {:?}\n", r.cell_size());

    let groups: [(&str, &str); 6] = [
        ("ascii", "Aa1@#"),
        ("latin ext", "áéñüÇ"),
        ("box drawing", "┌─┐│└┘├┼"),
        ("blocks/powerline", "█▀▄"),
        ("CJK", "中文日本語"),
        ("emoji", "😀🚀✅"),
    ];

    for (name, chars) in groups {
        let mut ok = String::new();
        let mut missing = String::new();
        for c in chars.chars() {
            if r.has_glyph(c) {
                ok.push(c)
            } else {
                missing.push(c)
            }
        }
        println!("{name:<18} present: {ok:<12} MISSING: {missing}");
    }
    Ok(())
}
