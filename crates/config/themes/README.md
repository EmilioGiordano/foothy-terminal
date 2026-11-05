# Bundled themes

The 86 colour themes shipped with [foot](https://codeberg.org/dnkl/foot), copied
unchanged from `themes/` at commit `85655c7`, with foot's MIT `LICENSE` beside them.

They are embedded in the binary, so a `theme=` name resolves without the files being on
disk. A theme of the same name under `%APPDATA%\foothy\themes\` wins over the bundled one.

Sections: 62 carry only `[colors-dark]`, 17 only `[colors-light]`, 7 both. The light-only
ones declare `initial-color-theme=light` in `[main]`; ignore that key and they resolve to
nothing.

To refresh: `cp upstream/foot/themes/* crates/config/themes/` and update the commit above.
