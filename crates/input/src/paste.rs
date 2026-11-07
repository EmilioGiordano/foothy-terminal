//! Paste safety. Pasted newlines are submitted commands: a paste that carries
//! them can run something the user never read.

/// Bytes above which a paste is worth confirming even without newlines.
pub const LARGE_PASTE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Risk {
    /// Nothing to ask about.
    Safe,
    /// Carries line breaks.
    MultiLine { lines: usize, bracketed: bool },
    /// Big enough that the user cannot have read all of it.
    Large { bytes: usize },
}

pub fn assess(text: &str, bracketed: bool) -> Risk {
    let lines = line_count(text);
    if lines > 1 {
        return Risk::MultiLine { lines, bracketed };
    }
    if text.len() > LARGE_PASTE {
        return Risk::Large { bytes: text.len() };
    }
    Risk::Safe
}

/// Lines the text would submit. A single trailing newline still submits the
/// line before it, so it counts.
pub fn line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .count()
}

/// Line endings normalised to CR, which is what Enter sends.
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\r").replace('\n', "\r")
}

/// ESC and C1 must go: pasted text carrying `ESC[201~` or an eight-bit CSI
/// would close the bracket early and the rest would arrive as typed commands.
pub fn sanitize(text: &str, bracketed: bool) -> String {
    text.chars()
        .filter(|&c| match c {
            '\t' | '\n' | '\r' | '\x0b' | '\x0c' => true,
            '\0' | '\x08' | '\x7f' => bracketed,
            '\x01'..='\x1f' | '\u{80}'..='\u{9f}' => false,
            _ => true,
        })
        .collect()
}

/// Wraps the text so a program that asked for bracketed paste can tell it apart.
pub fn bracket(text: &str) -> Vec<u8> {
    let body = sanitize(text, true);
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_is_safe() {
        assert_eq!(assess("cargo build", false), Risk::Safe);
    }

    #[test]
    fn multiline_without_bracketed_would_execute() {
        assert_eq!(
            assess("one\ntwo\nthree", false),
            Risk::MultiLine {
                lines: 3,
                bracketed: false
            }
        );
    }

    #[test]
    fn bracketed_paste_still_asks_about_line_breaks() {
        assert_eq!(
            assess("one\ntwo\nthree", true),
            Risk::MultiLine {
                lines: 3,
                bracketed: true
            }
        );
    }

    #[test]
    fn trailing_newline_still_submits() {
        // "rm -rf /\n" runs on arrival: the newline is the Enter.
        assert!(matches!(
            assess("rm -rf /\n", false),
            Risk::MultiLine { .. }
        ));
        assert!(matches!(assess("rm -rf /\n", true), Risk::MultiLine { .. }));
    }

    #[test]
    fn crlf_counts_as_one_break() {
        assert_eq!(line_count("one\r\ntwo"), 2);
        assert_eq!(line_count("one\rtwo"), 2);
        assert_eq!(line_count("one\ntwo"), 2);
    }

    #[test]
    fn empty_text_has_no_lines() {
        assert_eq!(line_count(""), 0);
        assert_eq!(assess("", false), Risk::Safe);
    }

    #[test]
    fn large_single_line_is_flagged() {
        let text = "x".repeat(LARGE_PASTE + 1);
        assert_eq!(
            assess(&text, true),
            Risk::Large {
                bytes: LARGE_PASTE + 1
            }
        );
    }

    #[test]
    fn normalize_maps_every_ending_to_cr() {
        assert_eq!(
            normalize("one\r\ntwo\nthree\rfour"),
            "one\rtwo\rthree\rfour"
        );
    }

    #[test]
    fn bracket_wraps_the_payload() {
        assert_eq!(bracket("hi"), b"\x1b[200~hi\x1b[201~");
    }

    fn body(out: &[u8]) -> Vec<u8> {
        assert!(out.starts_with(b"\x1b[200~"));
        assert!(out.ends_with(b"\x1b[201~"));
        out[6..out.len() - 6].to_vec()
    }

    #[test]
    fn the_end_sequence_cannot_survive_the_payload() {
        let payloads = [
            "\x1b[201~rm -rf /",
            "ls\x1b[201~rm -rf /",
            "ls\x1b[201~",
            "\x1b[201~a\x1b[201~b",
            "\x1b[2",
            "01~rm -rf /",
            "\x1b[201\x1b[201~~",
            "\u{9b}201~rm -rf /",
        ];
        for payload in payloads {
            let out = bracket(payload);
            let inner = body(&out);
            assert!(
                !inner.contains(&0x1b),
                "ESC reached the payload for {payload:?}"
            );
            assert!(
                !inner.windows(6).any(|w| w == b"\x1b[201~"),
                "the bracket closes early for {payload:?}"
            );
        }
    }

    #[test]
    fn concatenating_two_sanitized_halves_cannot_reform_the_sequence() {
        let first = sanitize("echo \x1b[20", true);
        let second = sanitize("1~rm -rf /", true);
        let joined = format!("{first}{second}");
        assert!(!joined.contains('\x1b'));
    }

    #[test]
    fn formatting_controls_and_text_survive() {
        assert_eq!(sanitize("a\tb\nc\rd", true), "a\tb\nc\rd");
        assert_eq!(sanitize("áéñ 中文 😀", true), "áéñ 中文 😀");
    }

    #[test]
    fn bracketed_paste_keeps_what_only_the_shell_would_act_on() {
        assert_eq!(sanitize("a\x08b\x7fc\0d", true), "a\x08b\x7fc\0d");
        assert_eq!(sanitize("a\x08b\x7fc\0d", false), "abcd");
    }

    #[test]
    fn other_c0_and_c1_controls_go_either_way() {
        assert_eq!(sanitize("a\x03b\x07c\x18d", true), "abcd");
        assert_eq!(sanitize("a\x03b\x07c\x18d", false), "abcd");
        assert_eq!(sanitize("a\u{9b}b\u{80}c", true), "abc");
    }
}
