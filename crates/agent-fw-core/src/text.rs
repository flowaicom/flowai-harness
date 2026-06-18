//! Shared text utilities.

/// Truncate a string at a UTF-8 byte boundary, appending `…` if shortened.
pub fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let boundary = s
        .char_indices()
        .take_while(|(i, _)| *i < max_bytes)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    format!("{}…", &s[..boundary])
}

/// Truncate a string at a character boundary, appending `…` if shortened.
pub fn truncate_utf8_chars(s: &str, max_chars: usize) -> String {
    let mut iter = s.chars();
    let mut out = String::new();

    for _ in 0..max_chars {
        let Some(ch) = iter.next() else {
            return s.to_string();
        };
        out.push(ch);
    }

    if iter.next().is_some() {
        out.push('…');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_bytes_no_truncation() {
        assert_eq!(truncate_utf8_bytes("hello", 10), "hello");
    }

    #[test]
    fn truncate_utf8_bytes_ascii() {
        assert_eq!(truncate_utf8_bytes("hello world", 5), "hello…");
    }

    #[test]
    fn truncate_utf8_bytes_multibyte_boundary() {
        assert_eq!(truncate_utf8_bytes("café señor", 5), "café…");
    }

    #[test]
    fn truncate_utf8_chars_no_truncation() {
        assert_eq!(truncate_utf8_chars("hello", 10), "hello");
    }

    #[test]
    fn truncate_utf8_chars_truncates() {
        assert_eq!(truncate_utf8_chars("abcdef", 3), "abc…");
    }

    #[test]
    fn truncate_utf8_chars_respects_multibyte_chars() {
        assert_eq!(truncate_utf8_chars("👋🌍🎉 hello", 2), "👋🌍…");
    }
}
