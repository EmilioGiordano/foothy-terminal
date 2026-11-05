use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=themes");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let themes = manifest.join("themes");

    let mut names: Vec<String> = std::fs::read_dir(&themes)
        .expect("themes/ is missing")
        .map(|e| e.expect("unreadable themes/ entry").file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| n != "README.md" && n != "LICENSE")
        .collect();
    // read_dir order is not stable; bundled_names() must be.
    names.sort();

    let mut out = String::from("pub static BUNDLED: &[(&str, &str)] = &[\n");
    for name in &names {
        let path = themes.join(name);
        writeln!(
            out,
            "    ({:?}, include_str!({:?})),",
            name,
            path.to_string_lossy()
        )
        .unwrap();
    }
    out.push_str("];\n\npub static NAMES: &[&str] = &[\n");
    for name in &names {
        writeln!(out, "    {name:?},").unwrap();
    }
    out.push_str("];\n");

    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bundled.rs");
    std::fs::write(dest, out).unwrap();
}
