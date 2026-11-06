//! The prompt every pane's shell is started with.
//!
//! The chip is text PowerShell prints, not chrome we draw. `docs/05` puts the
//! prompt on the shell's side of the line, so what we own here is the script and
//! the glyphs it is allowed to use.

use config::{
    SENTINEL_BRANCH, SENTINEL_BRANCH_ICON, SENTINEL_FOLDER_ICON, SENTINEL_PATH, SENTINEL_PILL,
};

/// Glyphs the chip is drawn with.
pub struct Glyphs {
    pub pill_left: char,
    pub pill_right: char,
    pub folder: Option<char>,
    pub branch: Option<char>,
    pub chevron: char,
    pub ellipsis: char,
}

const PILL_LEFT: [char; 2] = ['\u{e0b6}', '\u{2590}'];
const PILL_RIGHT: [char; 2] = ['\u{e0b4}', '\u{258c}'];
const FOLDER: [char; 2] = ['\u{f07b}', '\u{e5ff}'];
const BRANCH: [char; 2] = ['\u{e0a0}', '\u{f418}'];
const CHEVRON: [char; 2] = ['\u{276f}', '>'];
const ELLIPSIS: [char; 2] = ['\u{2026}', '.'];

/// Measured against `C:\Windows\Fonts\CascadiaMono.ttf`: every candidate above
/// that the stock font does not carry. Standing in for a renderer with no
/// built-in shapes either, which is the only way the fallbacks are reached.
#[cfg(test)]
const CASCADIA_LACKS: [char; 6] = [
    '\u{e0b6}', '\u{e0b4}', '\u{f07b}', '\u{e5ff}', '\u{e0a0}', '\u{f418}',
];

impl Glyphs {
    /// Picks each glyph against what the renderer can actually draw.
    ///
    /// The rounded pill ends and both icons live in Nerd Font's private use
    /// area, which stock Cascadia does not carry; `render::builtin` synthesises
    /// those four, so in practice the first candidate wins. The fallbacks —
    /// half blocks, no icons — are what a font missing them all would get, and
    /// they still beat a row of tofu.
    pub fn pick(has: impl Fn(char) -> bool) -> Self {
        let first = |candidates: &[char], fallback: char| {
            candidates
                .iter()
                .copied()
                .find(|c| has(*c))
                .unwrap_or(fallback)
        };
        let optional = |candidates: &[char]| candidates.iter().copied().find(|c| has(*c));
        Self {
            pill_left: first(&PILL_LEFT, ' '),
            pill_right: first(&PILL_RIGHT, ' '),
            folder: optional(&FOLDER),
            branch: optional(&BRANCH),
            chevron: first(&CHEVRON, '>'),
            ellipsis: first(&ELLIPSIS, '.'),
        }
    }
}

fn rgb(color: u32) -> String {
    format!(
        "{};{};{}",
        (color >> 16) & 0xff,
        (color >> 8) & 0xff,
        color & 0xff
    )
}

fn ink(color: u32) -> String {
    format!("${{e}}[38;2;{}m", rgb(color))
}

/// The cap is ink, not a filled background: the glyph is a rounded half block,
/// and painting it as a background would square the end off.
fn pill_open(g: &Glyphs, icon: Option<char>, icon_ink: u32, text: u32) -> String {
    let mut s = format!(
        "${{e}}[0m{}{}${{e}}[48;2;{}m ",
        ink(SENTINEL_PILL),
        g.pill_left,
        rgb(SENTINEL_PILL)
    );
    if let Some(icon) = icon {
        s.push_str(&ink(icon_ink));
        s.push_str(&format!("{icon} "));
    }
    s.push_str(&ink(text));
    s
}

fn pill_close(g: &Glyphs) -> String {
    format!(" ${{e}}[0m{}{}${{e}}[0m", ink(SENTINEL_PILL), g.pill_right)
}

// .NET rather than cmdlets throughout: this runs before every command, and
// Split-Path in particular cannot take -LiteralPath and -Parent together in
// Windows PowerShell 5.1 -- the parameter sets conflict and it throws.
const TEMPLATE: &str = r#"
$global:__footChipFile = '@VALUES@'
$global:__footChipKeys = @('path-on', 'path-off', 'branch-on', 'branch-off', 'chevron')

function global:__footChipRead {
  $f = $global:__footChipFile
  if (-not $f) { return $null }
  $i = $null
  try { $i = [System.IO.FileInfo]::new($f) } catch { return $null }
  # Sized before it is read: a huge file would stall every prompt.
  if (-not $i.Exists -or $i.Length -lt 1 -or $i.Length -gt 4096) { return $null }
  $t = $null
  # Delete-sharing, or the app's rename over this file -- and its removal on
  # exit -- fails while a prompt is reading it.
  try {
    $s = [System.IO.File]::Open($f, 'Open', 'Read', 'ReadWrite,Delete')
    try {
      $r = [System.IO.StreamReader]::new($s)
      try { $t = $r.ReadToEnd() } finally { $r.Dispose() }
    } finally { $s.Dispose() }
  } catch { return $null }
  # The app may be renaming over this file while it is read, so a file that
  # does not end on a newline is a half-written one and gets no say.
  # Ordinal: the default EndsWith is culture-sensitive and takes a newline
  # followed by an ignorable character as a newline.
  if (-not $t.EndsWith("`n", [System.StringComparison]::Ordinal)) { return $null }
  $h = @{}
  # foreach, not ForEach-Object: 'return' inside a pipeline block leaves only
  # the block, and a rejected line would then be skipped instead of fatal.
  foreach ($line in $t.Split("`n")) {
    $line = $line.TrimEnd("`r")
    if ($line.Length -lt 1 -or $line[0] -eq '#') { continue }
    $j = $line.IndexOf('=')
    if ($j -lt 1) { return $null }
    $v = $line.Substring($j + 1)
    # Nothing but colour: any other escape could move the cursor, repaint the
    # screen or leave the terminal in a mode the shell cannot get out of.
    # -cnotmatch, not -notmatch: the default is case-insensitive and would take
    # ESC [ n M -- delete line -- as an SGR.
    if ($v -cnotmatch '^(?:\u001b\[[0-9;]*m|[^\u0000-\u001f\u007f-\u009f])*$') { return $null }
    $h[$line.Substring(0, $j)] = $v
  }
  foreach ($k in $global:__footChipKeys) {
    if (-not $h.ContainsKey($k)) { return $null }
  }
  return $h
}

function global:__footBranch {
  if ($PWD.Provider.Name -ne 'FileSystem') { return $null }
  $d = $PWD.ProviderPath
  while ($d) {
    $g = [System.IO.Path]::Combine($d, '.git')
    $isDir = [System.IO.Directory]::Exists($g)
    $isFile = [System.IO.File]::Exists($g)
    if ($isDir -or $isFile) {
      if ($isFile) {
        # A worktree or submodule: the file points at the real git directory.
        $l = [System.IO.File]::ReadAllText($g)
        if ($l -match 'gitdir:\s*(.+)') { $g = $Matches[1].Trim() } else { return $null }
        if (-not [System.IO.Path]::IsPathRooted($g)) {
          $g = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($d, $g))
        }
      }
      $h = [System.IO.Path]::Combine($g, 'HEAD')
      if (-not [System.IO.File]::Exists($h)) { return $null }
      $line = [System.IO.File]::ReadAllText($h).Trim()
      if ($line -match '^ref:\s*refs/heads/(.+)$') { return $Matches[1].Trim() }
      if ($line) { return $line.Substring(0, [Math]::Min(7, $line.Length)) }
      return $null
    }
    $d = [System.IO.Path]::GetDirectoryName($d)
  }
  return $null
}

function global:prompt {
  $e = [char]27
  $p = $PWD.Path
  # The prefix has to end on a separator, or a sibling of home whose name merely
  # starts with it -- giordano next to giord -- comes out as "~ano".
  $h = $HOME
  if ($h) { $h = $h.TrimEnd('\', '/') }
  if ($PWD.Provider.Name -eq 'FileSystem' -and $h -and
      ($p -eq $h -or $p.StartsWith($h + '\', [System.StringComparison]::OrdinalIgnoreCase))) {
    $p = '~' + $p.Substring($h.Length)
  }

  # A prompt that throws leaves PowerShell showing a bare "PS>", so anything
  # touching the filesystem degrades to no branch instead of no prompt.
  $b = $null
  try { $b = __footBranch } catch { }

  $w = 80
  try { $w = $Host.UI.RawUI.WindowSize.Width } catch { }
  # Only what the chip itself needs: the caps, the padding, and the branch pill
  # when there is one. A fixed reserve over-truncates a narrow pane with no repo.
  $reserve = 6
  if ($b) { $reserve += $b.Length + 5 }
  $budget = [Math]::Max(12, $w - $reserve)
  if ($p.Length -gt $budget) {
    $parts = $p -split '[\\/]'
    if ($parts.Count -gt 3) {
      $p = $parts[0] + '\' + '@ELL@' + '\' + $parts[$parts.Count - 2] + '\' + $parts[$parts.Count - 1]
    }
    if ($p.Length -gt $budget) { $p = '@ELL@' + $p.Substring($p.Length - $budget + 1) }
  }

  $c = @{
    'path-on' = "@PATH_ON@"; 'path-off' = "@PATH_OFF@";
    'branch-on' = "@BRANCH_ON@"; 'branch-off' = "@BRANCH_OFF@";
    'chevron' = "@CHEVRON@"
  }
  try { $o = __footChipRead; if ($o) { $c = $o } } catch { }

  # A hand-edited chip file need not balance its own SGR, so the chip is
  # bracketed with resets: a stray colour cannot outlive its own prompt.
  $out = "${e}[0m" + $c['path-on'] + $p + $c['path-off']
  if ($b) { $out += $c['branch-on'] + $b + $c['branch-off'] }
  $out += "${e}[0m"

  # The chip goes out as ordinary output and only the chevron is returned as the
  # prompt. A prompt PSReadLine believes is one line but that wraps after a
  # resize puts its cursor arithmetic out by a row, and it then reads part of
  # the chip back as typed input. Two cells cannot wrap.
  Write-Host $out
  $c['chevron'] + "${e}[0m" + ' '
}
"#;

pub const KEYS: [&str; 5] = ["path-on", "path-off", "branch-on", "branch-off", "chevron"];

fn fragments(g: &Glyphs) -> [String; 5] {
    [
        pill_open(g, g.folder, SENTINEL_FOLDER_ICON, SENTINEL_PATH),
        pill_close(g),
        pill_open(g, g.branch, SENTINEL_BRANCH_ICON, SENTINEL_BRANCH),
        pill_close(g),
        format!("${{e}}[36m{}${{e}}[0m", g.chevron),
    ]
}

/// The PowerShell that replaces the stock `PS C:\...>` prompt.
pub fn script(g: &Glyphs, values: Option<&std::path::Path>) -> String {
    let f = fragments(g);
    // A single-quoted PowerShell literal takes anything but a quote of its own.
    let path = values
        .map(|p| p.display().to_string().replace('\'', "''"))
        .unwrap_or_default();
    TEMPLATE
        .replace("@PATH_ON@", &f[0])
        .replace("@PATH_OFF@", &f[1])
        .replace("@BRANCH_ON@", &f[2])
        .replace("@BRANCH_OFF@", &f[3])
        .replace("@CHEVRON@", &f[4])
        .replace("@ELL@", &g.ellipsis.to_string())
        .replace("@VALUES@", &path)
}

pub fn values_file(g: &Glyphs) -> String {
    let mut out =
        String::from("# Generated by foothy for this window; edits are lost. Not configuration.\n");
    for (key, fragment) in KEYS.iter().zip(fragments(g)) {
        out.push_str(key);
        out.push('=');
        out.push_str(&fragment.replace("${e}", "\u{1b}"));
        out.push('\n');
    }
    out
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
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

/// The argument for `-EncodedCommand`. PowerShell wants base64 of UTF-16LE, and
/// going through it avoids quoting a multi-line script on a command line.
pub fn encoded_command(script: &str) -> String {
    base64(&utf16le(script))
}

#[cfg(test)]
fn test_colors() -> config::Colors {
    let mut c = config::Config::default().colors;
    c.foreground = 0x00dc_ecf5;
    c.background = 0x0016_2a33;
    c.palette[5] = 0x009d_7cd8;
    c.palette[6] = 0x005f_b3a1;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::ChipStyle;

    fn nerd(_: char) -> bool {
        true
    }

    fn cascadia(c: char) -> bool {
        !CASCADIA_LACKS.contains(&c)
    }

    #[test]
    fn a_font_without_the_private_use_area_gets_no_tofu() {
        let g = Glyphs::pick(cascadia);
        assert_eq!(g.pill_left, '\u{2590}');
        assert_eq!(g.pill_right, '\u{258c}');
        assert_eq!(g.folder, None, "no icon beats a missing one");
        assert_eq!(g.branch, None);
        assert_eq!(g.chevron, '\u{276f}');
    }

    #[test]
    fn a_nerd_font_gets_the_rounded_pill_and_the_icons() {
        let g = Glyphs::pick(nerd);
        assert_eq!(g.pill_left, '\u{e0b6}');
        assert_eq!(g.pill_right, '\u{e0b4}');
        assert_eq!(g.folder, Some('\u{f07b}'));
        assert_eq!(g.branch, Some('\u{e0a0}'));
    }

    #[test]
    fn the_chip_asks_for_glyphs_the_renderer_can_draw() {
        // If the two lists drift apart the chip silently falls back to half
        // blocks and no icons, which looks fine and is not what was intended.
        for (what, candidates) in [
            ("pill left", PILL_LEFT),
            ("pill right", PILL_RIGHT),
            ("folder", FOLDER),
            ("branch", BRANCH),
        ] {
            assert!(
                render::SYNTHESISED_GLYPHS.contains(&candidates[0]),
                "{what}: {:?} is not one the renderer draws",
                candidates[0]
            );
        }
    }

    #[test]
    fn every_placeholder_is_filled_in() {
        let s = script(&Glyphs::pick(cascadia), None);
        // PowerShell's own @{ and @( use the same character, so only an
        // @NAME@ in capitals counts as one of ours.
        let left: Vec<&str> = s
            .match_indices('@')
            .filter_map(|(i, _)| {
                let rest = &s[i + 1..];
                let end = rest.find('@')?;
                let name = &rest[..end];
                (!name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
                    .then_some(name)
            })
            .collect();
        assert!(left.is_empty(), "a placeholder survived: {left:?}");
    }

    #[test]
    fn the_escape_is_written_so_powershell_cannot_read_it_as_an_index() {
        // "$e[38;..." would be parsed as indexing $e; "${e}[38;..." cannot be.
        let s = script(&Glyphs::pick(cascadia), None);
        assert!(s.contains("${e}["));
        assert!(!s.contains("$e["));
    }

    /// Every truecolor value the script names, as `38;2;r;g;b` or `48;2;r;g;b`.
    fn truecolors(s: &str) -> Vec<u32> {
        let mut out = Vec::new();
        for (i, _) in s.match_indices("8;2;") {
            if !s[..i].ends_with("[3") && !s[..i].ends_with("[4") {
                continue;
            }
            let rest = &s[i + 4..];
            let end = rest.find('m').expect("an unterminated sgr");
            let n: Vec<u32> = rest[..end]
                .split(';')
                .map(|c| c.parse().expect("not a channel"))
                .collect();
            assert_eq!(n.len(), 3, "{:?}", &rest[..end]);
            out.push((n[0] << 16) | (n[1] << 8) | n[2]);
        }
        out
    }

    #[test]
    fn the_chip_names_nothing_but_sentinels() {
        // A real colour baked in here is a chip that cannot follow the theme.
        let s = script(&Glyphs::pick(nerd), None);
        let named = truecolors(&s);
        assert!(!named.is_empty(), "the chip names no colour at all");
        for c in named {
            assert!(config::SENTINELS.contains(&c), "{c:#08x} is not a sentinel");
        }
    }

    #[test]
    fn the_chip_is_the_same_bytes_whatever_the_style() {
        // Inverse is an alias table over these bytes, not a byte stream of its
        // own: a chip in scrollback has to be able to change style.
        let g = Glyphs::pick(nerd);
        assert!(!script(&g, None).contains("${e}[7m"), "reverse survived");
        assert!(!script(&g, None).contains("${e}[39m"), "a default ink");
    }

    #[test]
    fn the_chevron_is_an_indexed_colour() {
        let g = Glyphs::pick(cascadia);
        let s = script(&g, None);
        assert!(
            s.contains(&format!("${{e}}[36m{}", g.chevron)),
            "the chevron is not indexed cyan:\n{s}"
        );
    }

    #[test]
    fn every_fragment_prints_the_sentinel_of_the_role_its_key_names() {
        // The bug: one sentinel on the wrong fragment paints the path in the
        // faded branch ink and makes `path-off` close the branch pill, with
        // every other test still green. `path-off` and `branch-off` are the
        // same bytes, so only these two fragments can be told apart.
        let g = Glyphs::pick(nerd);
        let f = fragments(&g);
        let at = |key: &str| KEYS.iter().position(|k| *k == key).expect("no such key");
        for (key, icon, text) in [
            ("path-on", "[38;2;1;2;242m", "[38;2;1;2;241m"),
            ("branch-on", "[38;2;1;2;244m", "[38;2;1;2;243m"),
        ] {
            let s = &f[at(key)];
            let i = s
                .find(icon)
                .unwrap_or_else(|| panic!("{key}: no {icon} in {s:?}"));
            let t = s
                .find(text)
                .unwrap_or_else(|| panic!("{key}: no {text} in {s:?}"));
            assert!(i < t, "{key}: the text ink comes before the icon in {s:?}");
        }
    }

    #[test]
    fn the_pill_sentinel_is_both_the_cap_ink_and_the_fill() {
        let g = Glyphs::pick(nerd);
        let s = script(&g, None);
        assert!(s.contains(&format!("${{e}}[38;2;1;2;240m{}", g.pill_left)));
        assert!(s.contains("${e}[48;2;1;2;240m"));
        assert!(s.contains(&format!("${{e}}[38;2;1;2;240m{}", g.pill_right)));
    }

    #[test]
    fn the_values_path_goes_in_as_a_literal_a_quote_cannot_escape() {
        let path = std::path::Path::new(r"C:\it's\chip.conf");
        let s = script(&Glyphs::pick(nerd), Some(path));
        assert!(s.contains(r"'C:\it''s\chip.conf'"), "{s}");
        let bare = script(&Glyphs::pick(nerd), None);
        assert!(bare.contains("__footChipFile = ''"), "{bare}");
    }

    #[test]
    fn the_values_file_passes_the_check_the_prompt_makes_of_it() {
        let text = values_file(&Glyphs::pick(nerd));
        assert!(!text.contains("${e}"), "an unresolved escape:\n{text:?}");
        assert!(chip_file_accepted(&text), "rejected:\n{text:?}");
        let found: Vec<&str> = text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.split_once('=').expect("no key").0)
            .collect();
        assert_eq!(found, KEYS);
    }

    const REFUSED: [(&str, &str); 16] = [
        ("delete line", "\u{1b}[5M>"),
        ("erase display", "\u{1b}[2J>"),
        ("cursor position", "\u{1b}[1;1H>"),
        ("device status report", "\u{1b}[6n>"),
        ("hide the cursor", "\u{1b}[?25l>"),
        ("osc", "\u{1b}]0;pwned\u{7}>"),
        ("dcs", "\u{1b}Pq\u{1b}\\>"),
        ("an unterminated csi", "\u{1b}[38;2;1"),
        ("an escape with no bracket", "\u{1b}m>"),
        ("a bare escape", "\u{1b}"),
        ("c1 csi", "\u{9b}2J>"),
        ("c1 osc", "\u{9d}0;pwned\u{9c}>"),
        ("the top of the c1 range", "\u{9f}>"),
        ("the bottom of the c1 range", "\u{80}>"),
        ("the top of the c0 range", "\u{1f}>"),
        ("delete", "\u{7f}>"),
    ];

    const TAKEN: [(&str, &str); 6] = [
        ("an empty sgr", "\u{1b}[m>"),
        ("a chained sgr", "\u{1b}[0;1;38;2;1;2;3m>\u{1b}[0m"),
        ("plain text", "plain>"),
        ("the last printable ascii", "\u{7e}>"),
        ("the first printable above c1", "\u{a0}>"),
        ("a glyph outside ascii", "\u{276f}"),
    ];

    #[test]
    fn the_guard_refuses_everything_but_colour() {
        for (what, value) in REFUSED {
            assert!(!sgr_only(value), "{what}: {value:?} passed");
            assert!(
                !chip_file_accepted(&one_key_file(value)),
                "{what}: {value:?} passed as a file"
            );
        }
        for (what, value) in TAKEN {
            assert!(sgr_only(value), "{what}: {value:?} refused");
            assert!(
                chip_file_accepted(&one_key_file(value)),
                "{what}: {value:?} refused as a file"
            );
        }
    }

    #[test]
    fn a_file_is_refused_unless_it_ends_on_a_newline_and_carries_every_key() {
        assert!(chip_file_accepted(&one_key_file(">")));
        assert!(!chip_file_accepted(
            one_key_file(">").trim_end_matches('\n')
        ));
        assert!(!chip_file_accepted(""));
        assert!(!chip_file_accepted("=novalue\n"));
        assert!(!chip_file_accepted("justtext\n"));
        assert!(!chip_file_accepted(
            &one_key_file(">").replace("chevron", "bogus")
        ));
        let long = format!("{}#{}\n", one_key_file(">"), "x".repeat(4096));
        assert!(!chip_file_accepted(&long));
        // PowerShell's TrimEnd("`r") takes every trailing CR, not just one.
        assert!(chip_file_accepted(
            &one_key_file(">").replace('\n', "\r\r\n")
        ));
        // A PowerShell hashtable looks its keys up case-insensitively.
        assert!(chip_file_accepted(
            &one_key_file(">").replace("chevron", "CHEVRON")
        ));
    }

    fn one_key_file(chevron: &str) -> String {
        let mut out = String::from("# comment\n");
        for key in KEYS {
            out.push_str(key);
            out.push('=');
            if key == "chevron" {
                out.push_str(chevron);
            }
            out.push('\n');
        }
        out
    }

    /// Mirrors `__footChipRead`. A rule that drifts from the PowerShell is a
    /// hole in the guard the suite cannot see.
    fn chip_file_accepted(text: &str) -> bool {
        if text.is_empty() || text.len() > 4096 || !text.ends_with('\n') {
            return false;
        }
        let mut found = Vec::new();
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(j) = line.find('=') else {
                return false;
            };
            if j < 1 || !sgr_only(&line[j + 1..]) {
                return false;
            }
            found.push(&line[..j]);
        }
        KEYS.iter()
            .all(|k| found.iter().any(|f| f.eq_ignore_ascii_case(k)))
    }

    /// Mirrors the regex in `__footChipRead`: `m` lower-case only, and nothing
    /// from C0, DEL or C1.
    fn sgr_only(value: &str) -> bool {
        let mut rest = value;
        while !rest.is_empty() {
            if let Some(tail) = rest.strip_prefix('\u{1b}') {
                let Some(tail) = tail.strip_prefix('[') else {
                    return false;
                };
                let end = tail
                    .find(|c: char| !matches!(c, '0'..='9' | ';'))
                    .unwrap_or(tail.len());
                if !tail[end..].starts_with('m') {
                    return false;
                }
                rest = &tail[end + 1..];
            } else {
                let c = rest.chars().next().unwrap();
                if matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}') {
                    return false;
                }
                rest = &rest[c.len_utf8()..];
            }
        }
        true
    }

    /// The chip as it reaches the screen, `${e}` resolved and a path between
    /// the open and close fragments.
    fn chip_bytes(g: &Glyphs) -> Vec<u8> {
        let f = fragments(g);
        format!("{}~{}", f[0], f[1])
            .replace("${e}", "\u{1b}")
            .into_bytes()
    }

    #[test]
    fn a_chip_already_on_screen_follows_the_style_without_being_reprinted() {
        // The bug: with real colours in the bytes, the chip above keeps the
        // colours of the theme and style it was printed under, forever.
        let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\CascadiaMono.ttf") else {
            return;
        };
        let mut r =
            render::Renderer::new(&bytes, 16.0, &test_colors().palette).expect("no renderer");
        let g = Glyphs::pick(|c| r.has_glyph(c));
        let (cw, ch) = r.cell_size();
        let (w, h) = (cw * 40, ch * 2);
        let mut term = vt_core::Terminal::new(vt_core::TermSize::new(40, 2));
        term.feed(&chip_bytes(&g));

        // The space the open fragment prints right after the cap: no glyph
        // covers it, so the pixel is the pill's own background.
        let inside = |r: &mut render::Renderer| {
            let mut buf = vec![0u32; w * h];
            r.clear(&mut buf);
            r.draw(&term, &mut buf, w, h, render::CursorLook::Hidden);
            buf[(ch / 2) * w + cw + cw / 2]
        };

        // Surface lifts the pill off the background; inverse paints it in the
        // foreground.
        for (style, want) in [
            (ChipStyle::Surface, 0x0032_454e),
            (ChipStyle::Inverse, 0x00dc_ecf5),
        ] {
            r.set_color_aliases(&config::chip_aliases(&test_colors(), style));
            assert_eq!(inside(&mut r), want, "{style:?}");
        }
    }

    #[test]
    fn base64_matches_the_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn the_encoded_command_is_utf16_little_endian() {
        // What PowerShell -EncodedCommand expects; UTF-8 here would be silently
        // decoded as garbage rather than rejected.
        assert_eq!(encoded_command("hi"), base64(&[b'h', 0, b'i', 0]));
    }

    #[test]
    fn the_encoded_command_only_uses_characters_no_shell_has_to_quote() {
        let s = script(&Glyphs::pick(nerd), None);
        let encoded = encoded_command(&s);
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')),
            "the argument would need quoting"
        );
    }
}

#[cfg(test)]
mod diagnostics {
    use super::*;
    use std::path::PathBuf;

    /// Writes the generated script out so it can be run for real:
    ///
    /// ```sh
    /// FOOTWIN_DUMP=/tmp/p.ps1 cargo test -p platform-win dump_the_script -- --ignored
    /// cargo run -p vt-core --example prompt_probe -- /tmp/p.ps1
    /// ```
    ///
    /// `FOOTWIN_VALUES` is the chip file the dumped script will read, and
    /// `FOOTWIN_DUMP_VALUES` a path to write that file's real contents to.
    ///
    /// Rust tests cannot tell whether the PowerShell inside is valid; the probe
    /// runs it under ConPTY and shows what the shell actually prints.
    #[test]
    #[ignore]
    fn dump_the_script() {
        let g = Glyphs::pick(|c| !CASCADIA_LACKS.contains(&c));
        let values = std::env::var("FOOTWIN_VALUES").ok().map(PathBuf::from);
        let out = std::env::var("FOOTWIN_DUMP").expect("set FOOTWIN_DUMP to a path");
        std::fs::write(out, script(&g, values.as_deref())).unwrap();
        if let Ok(out) = std::env::var("FOOTWIN_DUMP_VALUES") {
            std::fs::write(out, values_file(&g)).unwrap();
        }
    }
}
