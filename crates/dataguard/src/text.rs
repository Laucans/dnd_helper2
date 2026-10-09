//! The stored form of a text field: trimmed of Unicode `White_Space` (NBSP
//! included), then NFC. Lengths count Unicode scalar values, and names compare
//! by a lowercased key of that form.

use unicode_normalization::UnicodeNormalization;

/// Trim, then NFC: the form that is measured, compared and stored.
pub fn normalize(s: &str) -> String {
    s.trim().nfc().collect()
}

/// The key two names are compared by: the stored form, lowercased, NFC again
/// (lowercasing can decompose a character).
pub fn name_key(s: &str) -> String {
    normalize(s).to_lowercase().nfc().collect()
}

/// Unicode scalar values, not bytes.
pub fn len(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_unicode_whitespace_including_nbsp() {
        assert_eq!(normalize("\u{00A0} Ysolde\t\u{2003}"), "Ysolde");
        assert_eq!(normalize("\u{00A0}\u{00A0}"), "");
    }

    #[test]
    fn normalizes_to_nfc_and_counts_scalars() {
        let nfd = "E\u{0301}lan";
        assert_eq!(normalize(nfd), "\u{00C9}lan");
        assert_eq!(len(&normalize(nfd)), 4);
        assert_eq!(len("é"), 1);
        assert_eq!("é".len(), 2);
    }

    #[test]
    fn name_key_ignores_case_whitespace_and_form() {
        assert_eq!(name_key("Élan"), name_key("élan"));
        assert_eq!(name_key("  E\u{0301}LAN "), name_key("élan"));
        assert_ne!(name_key("Elan"), name_key("Élan"));
    }
}
