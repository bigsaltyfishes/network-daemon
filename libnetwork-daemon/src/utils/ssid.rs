//! SSID encoding/decoding utilities
//!
//! wpa_supplicant encodes special characters in SSIDs using escape sequences.
//! This module provides functions to decode and encode these sequences.

use unicode_width::UnicodeWidthStr;

use crate::ensure;

/// Unescape an SSID string from wpa_supplicant format
///
/// wpa_supplicant uses the following escape sequences:
/// - `\xNN` - Hex escape (e.g., `\x00` for null)
/// - `\\` - Literal backslash
/// - `\"` - Literal quote
/// - `\n`, `\r`, `\t` - Newline, carriage return, tab
/// - `\NNN` - Octal escape
///
/// # Arguments
///
/// * `s` - Escaped SSID string
///
/// # Returns
///
/// Unescaped SSID string (UTF-8 decoded)
///
/// # Example
///
/// ```
/// use libnetwork_daemon::utils::unescape_ssid;
///
/// assert_eq!(unescape_ssid(r"Test\x20Network"), "Test Network");
/// assert_eq!(unescape_ssid(r"Hello\\World"), "Hello\\World");
/// ```
pub fn unescape_ssid(s: &str) -> String {
    // First pass: convert escape sequences to bytes
    let mut bytes = Vec::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('x') | Some('X') => {
                    // Hex escape: \xNN
                    chars.next(); // consume 'x'
                    let hex: String = chars.by_ref().take(2).collect();
                    if hex.len() == 2 {
                        if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                            bytes.push(byte);
                            continue;
                        }
                    }
                    // Invalid escape, keep as-is
                    bytes.push(b'\\');
                    bytes.push(b'x');
                    bytes.extend(hex.as_bytes());
                }
                Some('\\') => {
                    chars.next();
                    bytes.push(b'\\');
                }
                Some('"') => {
                    chars.next();
                    bytes.push(b'"');
                }
                Some('n') => {
                    chars.next();
                    bytes.push(b'\n');
                }
                Some('r') => {
                    chars.next();
                    bytes.push(b'\r');
                }
                Some('t') => {
                    chars.next();
                    bytes.push(b'\t');
                }
                Some(c) if c.is_ascii_digit() && *c < '8' => {
                    // Octal escape: \NNN
                    let mut octal = String::new();
                    while octal.len() < 3 {
                        if let Some(&c) = chars.peek() {
                            if c.is_ascii_digit() && c < '8' {
                                octal.push(ensure!(chars.next()));
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    if !octal.is_empty()
                        && let Ok(byte) = u8::from_str_radix(&octal, 8)
                    {
                        bytes.push(byte);
                        continue;
                    }

                    // Invalid escape, keep as-is
                    bytes.push(b'\\');
                    bytes.extend(octal.as_bytes());
                }
                _ => {
                    // Unknown escape, keep backslash
                    bytes.push(b'\\');
                }
            }
        } else {
            // Regular character - encode as UTF-8
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            bytes.extend(encoded.as_bytes());
        }
    }

    // Convert bytes to UTF-8 string, replacing invalid sequences
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Escape an SSID string for wpa_supplicant commands
///
/// # Arguments
///
/// * `s` - Raw SSID string
///
/// # Returns
///
/// Escaped SSID string safe for wpa_supplicant commands
///
/// # Example
///
/// ```
/// use libnetwork_daemon::utils::escape_ssid;
///
/// assert_eq!(escape_ssid("Test Network"), r"Test\x20Network");
/// assert_eq!(escape_ssid("Hello\\World"), r"Hello\\World");
/// ```
pub fn escape_ssid(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);

    for c in s.chars() {
        match c {
            '\\' => result.push_str(r"\\"),
            '"' => result.push_str(r#"\""#),
            '\n' => result.push_str(r"\n"),
            '\r' => result.push_str(r"\r"),
            '\t' => result.push_str(r"\t"),
            c if c.is_ascii_control() || c == ' ' => {
                result.push_str(&format!(r"\x{:02x}", c as u8));
            }
            c => result.push(c),
        }
    }

    result
}

/// Calculate display width of an SSID
///
/// Takes into account Unicode character widths (e.g., CJK characters
/// typically have width 2).
///
/// # Arguments
///
/// * `s` - SSID string
///
/// # Returns
///
/// Display width in terminal columns
///
/// # Example
///
/// ```
/// use libnetwork_daemon::utils::ssid_display_width;
///
/// assert_eq!(ssid_display_width("Hello"), 5);
/// assert_eq!(ssid_display_width("你好"), 4); // Each CJK char is width 2
/// ```
pub fn ssid_display_width(s: &str) -> usize {
    s.width()
}

/// Truncate a string to fit within a given display width
///
/// If the string is longer than `max_width`, it will be truncated
/// and "..." will be appended.
///
/// # Arguments
///
/// * `s` - String to truncate
/// * `max_width` - Maximum display width
///
/// # Returns
///
/// Truncated string with "..." if needed
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width < 3 {
        return String::new();
    }

    let width = s.width();
    if width <= max_width {
        return s.to_string();
    }

    let mut result = String::new();
    let mut current_width = 0;
    let target_width = max_width - 3; // Leave room for "..."

    for c in s.chars() {
        let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        if current_width + char_width > target_width {
            break;
        }
        result.push(c);
        current_width += char_width;
    }

    result.push_str("...");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unescape_hex() {
        assert_eq!(unescape_ssid(r"Test\x20Network"), "Test Network");
        assert_eq!(unescape_ssid(r"\x48\x65\x6c\x6c\x6f"), "Hello");
        assert_eq!(unescape_ssid(r"A\x00B"), "A\0B");
    }

    #[test]
    fn test_unescape_utf8() {
        // UTF-8 multi-byte characters
        // ～ = U+FF5E = EF BD 9E
        assert_eq!(unescape_ssid(r"Ciallo\xef\xbd\x9e"), "Ciallo～");
        // Full SSID: Ciallo～(∠・ω< )⌒★
        assert_eq!(
            unescape_ssid(
                r"Ciallo\xef\xbd\x9e(\xe2\x88\xa0\xe3\x83\xbb\xcf\x89< )\xe2\x8c\x92\xe2\x98\x85"
            ),
            "Ciallo～(∠・ω< )⌒★"
        );
        // Chinese characters: 中文 = E4 B8 AD E6 96 87
        assert_eq!(unescape_ssid(r"\xe4\xb8\xad\xe6\x96\x87"), "中文");
    }

    #[test]
    fn test_unescape_special() {
        assert_eq!(unescape_ssid(r"Hello\\World"), "Hello\\World");
        assert_eq!(unescape_ssid(r#"Say\"Hello\""#), "Say\"Hello\"");
        assert_eq!(unescape_ssid(r"Line1\nLine2"), "Line1\nLine2");
        assert_eq!(unescape_ssid(r"Tab\there"), "Tab\there");
    }

    #[test]
    fn test_unescape_plain() {
        assert_eq!(unescape_ssid("PlainSSID"), "PlainSSID");
        assert_eq!(unescape_ssid(""), "");
    }

    #[test]
    fn test_escape_basic() {
        assert_eq!(escape_ssid("Hello"), "Hello");
        assert_eq!(escape_ssid(r"Hello\World"), r"Hello\\World");
    }

    #[test]
    fn test_escape_space() {
        assert_eq!(escape_ssid("Test Network"), r"Test\x20Network");
    }

    #[test]
    fn test_escape_special() {
        assert_eq!(escape_ssid("Line\nBreak"), r"Line\nBreak");
        assert_eq!(escape_ssid("Tab\there"), r"Tab\there");
    }

    #[test]
    fn test_roundtrip() {
        let original = "Test Network";
        let escaped = escape_ssid(original);
        let unescaped = unescape_ssid(&escaped);
        assert_eq!(unescaped, original);
    }

    #[test]
    fn test_display_width() {
        assert_eq!(ssid_display_width("Hello"), 5);
        assert_eq!(ssid_display_width(""), 0);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate_to_width("Hello", 10), "Hello");
        assert_eq!(truncate_to_width("Hello World", 8), "Hello...");
        assert_eq!(truncate_to_width("Hi", 10), "Hi");
    }
}
