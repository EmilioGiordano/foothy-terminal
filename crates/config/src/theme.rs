include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// The text of a theme embedded in the binary, by name.
pub fn bundled(name: &str) -> Option<&'static str> {
    BUNDLED
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| BUNDLED[i].1)
}

/// Every embedded theme name, sorted.
pub fn bundled_names() -> &'static [&'static str] {
    NAMES
}
