//! Paste safety. Pasted newlines are submitted commands: a paste that carries
//! them can run something the user never read.

/// Bytes above which a paste is worth confirming even without newlines.
pub const LARGE_PASTE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Risk {
    /// Nothing to ask about.
    Safe,
    /// Carries line breaks and the program did not ask for bracketed paste, so
    /// the shell will run each line as it arrives.
    WouldExecute { lines: usize },
    /// Big enough that the user cannot have read all of it.
    Large { bytes: usize },
}

/// `bracketed` is whether the running program enabled bracketed paste, which is
/// how it promises to treat the text as data instead of as typed commands.
pub fn assess(text: &str, bracketed: bool) -> Risk {
    let lines = line_count(text);
    if lines > 1 && !bracketed {
        return Risk::WouldExecute { lines };
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

/// Wraps the text so a program that asked for bracketed paste can tell it apart.
pub fn bracket(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(text.as_bytes());
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
            assess("uno\ndos\ntres", false),
            Risk::WouldExecute { lines: 3 }
        );
    }

    #[test]
    fn bracketed_paste_makes_multiline_safe() {
        // The program promised to treat it as data, so nagging would be noise.
        assert_eq!(assess("uno\ndos\ntres", true), Risk::Safe);
    }

    #[test]
    fn trailing_newline_still_submits() {
        // "rm -rf /\n" runs on arrival: the newline is the Enter.
        assert!(matches!(
            assess("rm -rf /\n", false),
            Risk::WouldExecute { .. }
        ));
    }

    #[test]
    fn crlf_counts_as_one_break() {
        assert_eq!(line_count("uno\r\ndos"), 2);
        assert_eq!(line_count("uno\rdos"), 2);
        assert_eq!(line_count("uno\ndos"), 2);
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
            normalize("uno\r\ndos\ntres\rcuatro"),
            "uno\rdos\rtres\rcuatro"
        );
    }

    #[test]
    fn bracket_wraps_the_payload() {
        assert_eq!(bracket("hi"), b"\x1b[200~hi\x1b[201~");
    }
}
