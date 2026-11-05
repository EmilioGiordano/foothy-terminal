use crate::{Blink, ChipStyle, CursorColors, Partial, PartialColors, Variant};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Problem {
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Main,
    Dark,
    Light,
    Other,
}

pub fn parse(text: &str) -> (Partial, Vec<Problem>) {
    let mut out = Partial::default();
    let mut problems = Vec::new();
    // Keys before any header belong to [main] (upstream/foot/config.c:3181).
    let mut section = Section::Main;

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let body = strip_comment(raw).trim();
        if body.is_empty() {
            continue;
        }

        if let Some(rest) = body.strip_prefix('[') {
            match rest.strip_suffix(']') {
                Some(name) => {
                    section = match name.trim() {
                        "main" => Section::Main,
                        "colors-dark" | "colors" => Section::Dark,
                        "colors-light" | "colors2" => Section::Light,
                        _ => Section::Other,
                    }
                }
                None => {
                    // A rejected header must not leave the previous section in
                    // force, or its keys land in the wrong one.
                    section = Section::Other;
                    problems.push(Problem {
                        line,
                        message: "section is missing its closing ']'".into(),
                    });
                }
            }
            continue;
        }

        let Some((key, value)) = body.split_once('=') else {
            problems.push(Problem {
                line,
                message: "no '=' in line".into(),
            });
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            problems.push(Problem {
                line,
                message: "empty key".into(),
            });
            continue;
        }

        let result = match section {
            Section::Main => main_key(&mut out, key, value),
            Section::Dark => colors_key(&mut out, Variant::Dark, key, value),
            Section::Light => colors_key(&mut out, Variant::Light, key, value),
            Section::Other => Ok(()),
        };
        if let Err(message) = result {
            problems.push(Problem { line, message });
        }
    }

    (out, problems)
}

/// A value ends at a `#` preceded by blank space, not at the first `#`.
fn strip_comment(raw: &str) -> &str {
    let line = raw.trim_start();
    if line.starts_with('#') {
        return "";
    }
    let bytes = line.as_bytes();
    for (i, w) in bytes.windows(2).enumerate() {
        if (w[0] == b' ' || w[0] == b'\t') && w[1] == b'#' {
            return &line[..i];
        }
    }
    line
}

fn color(value: &str) -> Result<u32, String> {
    // A leading '#' is not accepted: after a blank the comment rule has already
    // eaten the value, so it would work in one spelling and fail in the other.
    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("'{value}' is not a six-digit RGB colour"));
    }
    u32::from_str_radix(value, 16).map_err(|_| format!("'{value}' is not a six-digit RGB colour"))
}

fn two_colors(value: &str) -> Result<(u32, u32), String> {
    let mut it = value.split_whitespace();
    match (it.next(), it.next(), it.next()) {
        (Some(a), Some(b), None) => Ok((color(a)?, color(b)?)),
        _ => Err(format!("'{value}' is not a pair of colours")),
    }
}

fn boolean(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(format!("'{value}' is not a boolean")),
    }
}

/// `1` means opaque: up to 1 the value is a fraction, above it a 0-255 byte.
fn alpha(value: &str) -> Result<u8, String> {
    let f: f32 = value
        .parse()
        .map_err(|_| format!("'{value}' is not a number"))?;
    if !(0.0..=255.0).contains(&f) {
        return Err(format!("alpha '{value}' is out of range"));
    }
    if f <= 1.0 {
        Ok((f * 255.0).round() as u8)
    } else {
        Ok(f.round() as u8)
    }
}

fn main_key(out: &mut Partial, key: &str, value: &str) -> Result<(), String> {
    let s = &mut out.settings;
    match key {
        "font" => s.font = Some(value.to_string()),
        "font-size" => {
            s.font_size = Some(
                value
                    .parse()
                    .map_err(|_| format!("font-size '{value}' is not a number"))?,
            )
        }
        // The file writes `pad=2x2 center`; the first number is the one we use.
        "pad" => {
            let first = value
                .split_whitespace()
                .next()
                .unwrap_or(value)
                .split(['x', 'X'])
                .next()
                .unwrap_or(value);
            s.pad = Some(
                first
                    .parse()
                    .map_err(|_| format!("pad '{value}' is not a number"))?,
            )
        }
        "alpha" => s.alpha = Some(alpha(value)?),
        "blink" => {
            s.blink = Some(match value.to_ascii_lowercase().as_str() {
                "system" => Blink::System,
                "off" | "no" | "false" | "0" => Blink::Off,
                other => Blink::Ms(
                    other
                        .parse()
                        .map_err(|_| format!("blink '{value}' is not a number of ms"))?,
                ),
            })
        }
        "paste-warning" => s.paste_warning = Some(boolean(value)?),
        "prompt" => s.prompt = Some(boolean(value)?),
        "prompt-style" => {
            s.prompt_style = Some(match value.to_ascii_lowercase().as_str() {
                "surface" => ChipStyle::Surface,
                "inverse" => ChipStyle::Inverse,
                _ => {
                    return Err(format!("prompt-style '{value}' is not surface or inverse"));
                }
            })
        }
        "theme" => s.theme = Some(value.to_string()),
        "initial-color-theme" => {
            s.initial_variant = Some(match value.to_ascii_lowercase().as_str() {
                "dark" | "1" => Variant::Dark,
                "light" | "2" => Variant::Light,
                _ => {
                    return Err(format!(
                        "initial-color-theme '{value}' is not dark or light"
                    ));
                }
            })
        }
        _ => {}
    }
    Ok(())
}

fn colors_key(out: &mut Partial, variant: Variant, key: &str, value: &str) -> Result<(), String> {
    // The colour sections carry alpha too, not only [main].
    if key == "alpha" {
        out.settings.alpha = Some(alpha(value)?);
        return Ok(());
    }

    let c: &mut PartialColors = match variant {
        Variant::Dark => &mut out.dark,
        Variant::Light => &mut out.light,
    };

    if key.as_bytes()[0].is_ascii_digit() {
        let index: usize = key
            .parse()
            .map_err(|_| format!("'{key}' is not a palette index"))?;
        if index >= 256 {
            return Err(format!("palette index {index} is not in range 0-255"));
        }
        c.palette[index] = Some(color(value)?);
        return Ok(());
    }

    match key {
        "foreground" => c.foreground = Some(color(value)?),
        "background" => c.background = Some(color(value)?),
        "selection-foreground" => c.selection_foreground = Some(color(value)?),
        "selection-background" => c.selection_background = Some(color(value)?),
        "cursor" => {
            let (text, cursor) = two_colors(value)?;
            c.cursor = Some(CursorColors { text, cursor })
        }
        _ => {
            if let Some(slot) = indexed(key, "regular", 0).or_else(|| indexed(key, "bright", 8)) {
                c.palette[slot] = Some(color(value)?);
            }
            // An unknown key is not an error: every bundled theme carries
            // foot features we lack.
        }
    }
    Ok(())
}

fn indexed(key: &str, prefix: &str, base: usize) -> Option<usize> {
    let digit = key.strip_prefix(prefix)?;
    let n: usize = digit.parse().ok()?;
    (digit.len() == 1 && n < 8).then_some(base + n)
}
