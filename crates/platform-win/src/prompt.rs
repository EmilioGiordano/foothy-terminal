//! The prompt every pane's shell is started with.
//!
//! The chip is text PowerShell prints, not chrome we draw. `docs/05` puts the
//! prompt on the shell's side of the line, so what we own here is the script and
//! the glyphs it is allowed to use.

/// Colours the chip is painted with, as `0x00RRGGBB`. Icons carry their own,
/// which is what separates the chip from a plain coloured run of text.
pub struct Palette {
    pub pill: u32,
    pub folder_icon: u32,
    pub path: u32,
    pub branch_icon: u32,
    pub branch: u32,
    pub chevron: u32,
}

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

/// Opens a pill: the left cap drawn in the pill colour over the terminal
/// background, then the fill, the icon in its own colour, then the text colour.
fn pill_open(g: &Glyphs, pill: u32, icon: Option<char>, ink: u32, text: u32) -> String {
    let mut s = format!(
        "${{e}}[0m${{e}}[38;2;{}m{}${{e}}[48;2;{}m ",
        rgb(pill),
        g.pill_left,
        rgb(pill)
    );
    if let Some(icon) = icon {
        s.push_str(&format!("${{e}}[38;2;{}m{icon} ", rgb(ink)));
    }
    s.push_str(&format!("${{e}}[38;2;{}m", rgb(text)));
    s
}

fn pill_close(g: &Glyphs, pill: u32) -> String {
    format!(
        " ${{e}}[0m${{e}}[38;2;{}m{}${{e}}[0m",
        rgb(pill),
        g.pill_right
    )
}

// .NET rather than cmdlets throughout: this runs before every command, and
// Split-Path in particular cannot take -LiteralPath and -Parent together in
// Windows PowerShell 5.1 -- the parameter sets conflict and it throws.
const TEMPLATE: &str = r#"
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

  $out = "@PATH_ON@" + $p + "@PATH_OFF@"
  if ($b) { $out += "@BRANCH_ON@" + $b + "@BRANCH_OFF@" }

  # The chip goes out as ordinary output and only the chevron is returned as the
  # prompt. A prompt PSReadLine believes is one line but that wraps after a
  # resize puts its cursor arithmetic out by a row, and it then reads part of
  # the chip back as typed input. Two cells cannot wrap.
  Write-Host $out
  "@CHEVRON@" + ' '
}
"#;

/// The PowerShell that replaces the stock `PS C:\...>` prompt.
pub fn script(g: &Glyphs, p: &Palette) -> String {
    TEMPLATE
        .replace(
            "@PATH_ON@",
            &pill_open(g, p.pill, g.folder, p.folder_icon, p.path),
        )
        .replace("@PATH_OFF@", &pill_close(g, p.pill))
        .replace(
            "@BRANCH_ON@",
            &pill_open(g, p.pill, g.branch, p.branch_icon, p.branch),
        )
        .replace("@BRANCH_OFF@", &pill_close(g, p.pill))
        .replace(
            "@CHEVRON@",
            &format!("${{e}}[38;2;{}m{}${{e}}[0m", rgb(p.chevron), g.chevron),
        )
        .replace("@ELL@", &g.ellipsis.to_string())
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
mod tests {
    use super::*;

    fn nerd(_: char) -> bool {
        true
    }

    fn cascadia(c: char) -> bool {
        !CASCADIA_LACKS.contains(&c)
    }

    fn palette() -> Palette {
        Palette {
            pill: 0x000e_1418,
            folder_icon: 0x005f_b3a1,
            path: 0x00dc_ecf5,
            branch_icon: 0x009d_7cd8,
            branch: 0x00e8_eef5,
            chevron: 0x005f_b3a1,
        }
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
        let s = script(&Glyphs::pick(cascadia), &palette());
        assert!(!s.contains('@'), "a placeholder survived:\n{s}");
    }

    #[test]
    fn the_escape_is_written_so_powershell_cannot_read_it_as_an_index() {
        // "$e[38;..." would be parsed as indexing $e; "${e}[38;..." cannot be.
        let s = script(&Glyphs::pick(cascadia), &palette());
        assert!(s.contains("${e}[38;2;"));
        assert!(!s.contains("$e[38;2;"));
    }

    #[test]
    fn the_colours_come_out_as_decimal_channels() {
        let s = script(&Glyphs::pick(cascadia), &palette());
        assert!(s.contains("38;2;220;236;245"), "path colour missing");
        assert!(s.contains("48;2;14;20;24"), "pill fill missing");
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
        let s = script(&Glyphs::pick(nerd), &palette());
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

    /// Writes the generated script out so it can be run for real:
    ///
    /// ```sh
    /// FOOTWIN_DUMP=/tmp/p.ps1 cargo test -p platform-win dump_the_script -- --ignored
    /// cargo run -p vt-core --example prompt_probe -- /tmp/p.ps1
    /// ```
    ///
    /// Rust tests cannot tell whether the PowerShell inside is valid; the probe
    /// runs it under ConPTY and shows what the shell actually prints.
    #[test]
    #[ignore]
    fn dump_the_script() {
        let g = Glyphs::pick(|c| !CASCADIA_LACKS.contains(&c));
        let p = Palette {
            pill: 0x000e_1418,
            folder_icon: 0x005f_b3a1,
            path: 0x00dc_ecf5,
            branch_icon: 0x009d_7cd8,
            branch: 0x00e8_eef5,
            chevron: 0x005f_b3a1,
        };
        let out = std::env::var("FOOTWIN_DUMP").expect("set FOOTWIN_DUMP to a path");
        std::fs::write(out, script(&g, &p)).unwrap();
    }
}
