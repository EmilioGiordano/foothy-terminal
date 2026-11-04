include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

pub fn bundled(name: &str) -> Option<&'static str> {
    BUNDLED
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| BUNDLED[i].1)
}

pub fn bundled_names() -> &'static [&'static str] {
    NAMES
}
