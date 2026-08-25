use render::Renderer;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"C:\Windows\Fonts\CascadiaMono.ttf".to_string());
    let bytes = std::fs::read(&path)?;
    let r = Renderer::new(&bytes, 16.0)?;
    println!("{path}  celda {:?}\n", r.cell_size());

    let groups: [(&str, &str); 6] = [
        ("ascii", "Aa1@#"),
        ("latin ext", "áéñüÇ"),
        ("box drawing", "┌─┐│└┘├┼"),
        ("bloques/powerline", "█▀▄"),
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
        println!("{name:<18} presentes: {ok:<12} FALTAN: {missing}");
    }
    Ok(())
}
